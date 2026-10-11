//! **A minimal WebAuthn relying party** — `ADR-063` §4, `.rulings/` R-230.
//!
//! Two ceremonies and nothing else:
//!
//! * [`verify_registration`] — a browser created a credential. Attestation
//!   `none` only: this relying party trusts the operator who held the one-time
//!   enrollment code, not an authenticator's maker, so there is no attestation
//!   statement it would know how to use.
//! * [`verify_assertion`] — a browser signed a challenge with a credential
//!   registered earlier.
//!
//! Every check WebAuthn asks of a relying party for these two ceremonies is
//! here, each with its own [`Refusal`]: the ceremony type, the challenge, the
//! origin, `crossOrigin`, the RP ID hash, user presence, user verification, the
//! attested credential, the COSE key's algorithm and parameters, the signature
//! and the signature counter. What is deliberately NOT here is anything with
//! state — issuing a challenge, remembering it, expiring it, storing a
//! credential. That is the caller's, because it is I/O and a clock.
//!
//! Two algorithms: ES256 (COSE `-7`, ECDSA over P-256 with SHA-256, DER
//! signatures) and Ed25519 (COSE `-8`). Anything else is refused by number.
//!
//! **No panic reaches a caller.** The input is bytes a browser — or anybody
//! pretending to be one — sent, and the daemon that calls this aborts on panic.

use base64::Engine as _;

pub mod browser;
pub mod store;

/// Flag bits in authenticator data — WebAuthn's *Authenticator Data*.
const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_AT: u8 = 0x40;
const FLAG_ED: u8 = 0x80;

/// The fixed part of authenticator data: RP ID hash, flags, counter.
const AUTH_DATA_FIXED: usize = 32 + 1 + 4;

/// The longest credential ID accepted — WebAuthn's credential descriptor bounds it at 1023.
pub const MAX_CREDENTIAL_ID: usize = 1023;

/// Who this relying party is: its RP ID and the one origin it serves.
///
/// **One origin, exactly.** A ceremony runs at one of the names the proxy's
/// certificate carries and is checked against `https://<that name>`
/// (`ADR-063` §4); an origin with a port, another scheme or another host is a
/// different page and is refused rather than compared loosely.
#[derive(Debug, Clone, Copy)]
pub struct RelyingParty<'a> {
    /// The RP ID — one of the machine's names, e.g. `dev-01.local`.
    pub id: &'a str,
    /// The origin a ceremony must report, e.g. `https://dev-01.local`.
    pub origin: &'a str,
}

impl<'a> RelyingParty<'a> {
    /// The relying party a machine called `name` is, over HTTPS on 443.
    #[must_use]
    pub fn origin_for(name: &str) -> String {
        format!("https://{name}")
    }
}

/// Whether the UV flag must be set. The proxy always asks [`Self::Required`];
/// the other value exists so the W3C test vectors, several of which carry no
/// UV flag, can prove the rest of a ceremony verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserVerification {
    Required,
    NotRequired,
}

/// A credential's signature algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    /// COSE `-7`: ECDSA over P-256 with SHA-256. The public key is kept as the
    /// 65-byte uncompressed SEC1 point.
    Es256,
    /// COSE `-8`: Ed25519. The public key is its 32 bytes.
    Ed25519,
}

impl Algorithm {
    /// The COSE algorithm identifier.
    #[must_use]
    pub const fn cose(self) -> i64 {
        match self {
            Self::Es256 => -7,
            Self::Ed25519 => -8,
        }
    }

    /// The word a stored credential carries. Part of the file format; never
    /// renamed.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Es256 => "es256",
            Self::Ed25519 => "ed25519",
        }
    }

    /// [`Self::word`]'s inverse.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        [Self::Es256, Self::Ed25519].into_iter().find(|a| a.word() == word)
    }

    fn verifier(self) -> &'static dyn ring::signature::VerificationAlgorithm {
        match self {
            Self::Es256 => &ring::signature::ECDSA_P256_SHA256_ASN1,
            Self::Ed25519 => &ring::signature::ED25519,
        }
    }
}

/// A registered credential: what a relying party keeps to verify assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The credential ID the authenticator chose.
    pub id: Vec<u8>,
    pub algorithm: Algorithm,
    /// See [`Algorithm`] for the encoding of each.
    pub public_key: Vec<u8>,
    /// The signature counter last seen. Zero for an authenticator that keeps
    /// none, which many platform authenticators do.
    pub sign_count: u32,
}

/// What a verified assertion reports: the counter to store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asserted {
    pub sign_count: u32,
}

/// **Why a ceremony was refused.** Each is one check; the sentence is
/// [`core::fmt::Display`]'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// `clientDataJSON` is not a JSON object.
    ClientDataNotJson,
    /// `clientDataJSON` lacks a field, or it has the wrong JSON type.
    ClientDataField(&'static str),
    /// `type` is not the ceremony this is.
    WrongType { expected: &'static str, found: String },
    /// The challenge is not the one issued.
    ChallengeMismatch,
    /// The origin is not this machine's.
    OriginMismatch { expected: String, found: String },
    /// The ceremony ran in a frame of another origin.
    CrossOrigin,
    /// The attestation object is not CBOR, or not the shape WebAuthn defines.
    AttestationObject(&'static str),
    /// An attestation format other than `none`.
    AttestationFormat(String),
    /// `none` with a non-empty statement.
    AttestationStatementNotEmpty,
    /// Authenticator data is shorter than its own fields say, or longer.
    AuthenticatorData(&'static str),
    /// The RP ID hash is not SHA-256 of this relying party's ID.
    RpIdHashMismatch,
    /// UP was not set.
    UserNotPresent,
    /// UV was not set and is required.
    UserNotVerified,
    /// A registration whose authenticator data carries no credential.
    NoAttestedCredential,
    /// A credential ID longer than WebAuthn permits, or empty.
    CredentialIdLength(usize),
    /// The public key is not a COSE key this relying party reads.
    PublicKey(&'static str),
    /// A COSE algorithm this relying party does not accept.
    UnsupportedAlgorithm(i128),
    /// The assertion names a different credential than the one supplied.
    CredentialMismatch,
    /// The signature does not verify under the stored key.
    SignatureInvalid,
    /// The signature counter did not increase: WebAuthn's sign of a cloned
    /// authenticator (WebAuthn's *Signature Counter Considerations*).
    CounterRegression { stored: u32, presented: u32 },
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ClientDataNotJson => f.write_str("clientDataJSON is not a JSON object"),
            Self::ClientDataField(name) => {
                write!(f, "clientDataJSON has no string `{name}`, or it is not the right type")
            }
            Self::WrongType { expected, found } => {
                write!(f, "clientDataJSON's type is {found:?} and this ceremony is {expected:?}")
            }
            Self::ChallengeMismatch => f.write_str(
                "the challenge the browser signed is not the one this machine issued",
            ),
            Self::OriginMismatch { expected, found } => write!(
                f,
                "the ceremony ran at origin {found:?} and this machine is {expected:?}"
            ),
            Self::CrossOrigin => f.write_str(
                "the ceremony ran in a frame embedded by another origin (crossOrigin is true)",
            ),
            Self::AttestationObject(why) => write!(f, "the attestation object is unreadable: {why}"),
            Self::AttestationFormat(fmt) => write!(
                f,
                "the attestation format is {fmt:?}; this machine accepts \"none\" only, because \
                 it trusts the operator who held the enrollment code rather than an \
                 authenticator's maker"
            ),
            Self::AttestationStatementNotEmpty => {
                f.write_str("the attestation format is \"none\" and its statement is not empty")
            }
            Self::AuthenticatorData(why) => write!(f, "the authenticator data is unreadable: {why}"),
            Self::RpIdHashMismatch => f.write_str(
                "the authenticator signed for a different relying party (its RP ID hash is not \
                 SHA-256 of this machine's name)",
            ),
            Self::UserNotPresent => f.write_str("the authenticator did not report user presence"),
            Self::UserNotVerified => f.write_str(
                "the authenticator did not verify the user (no PIN, fingerprint or face), and \
                 this machine requires it",
            ),
            Self::NoAttestedCredential => {
                f.write_str("the registration's authenticator data carries no credential")
            }
            Self::CredentialIdLength(n) => {
                write!(f, "the credential ID is {n} bytes; 1 to {MAX_CREDENTIAL_ID} are allowed")
            }
            Self::PublicKey(why) => write!(f, "the credential's public key is unreadable: {why}"),
            Self::UnsupportedAlgorithm(alg) => write!(
                f,
                "the credential uses COSE algorithm {alg}; this machine accepts -7 (ES256) and -8 \
                 (Ed25519)"
            ),
            Self::CredentialMismatch => {
                f.write_str("the assertion names a credential other than the one it was checked against")
            }
            Self::SignatureInvalid => f.write_str("the signature does not verify under the stored key"),
            Self::CounterRegression { stored, presented } => write!(
                f,
                "the signature counter went from {stored} to {presented}; a counter that does not \
                 increase is WebAuthn's sign of a cloned authenticator"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// Base64url without padding — the one spelling WebAuthn uses.
#[must_use]
pub fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// [`b64url`]'s inverse, strict: no padding, no other alphabet.
#[must_use]
pub fn from_b64url(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).ok()
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

/// **The challenge a `clientDataJSON` carries**, decoded — so a relying party
/// holding several outstanding challenges can find which one a ceremony
/// answers before verifying it. It proves nothing: [`verify_registration`] and
/// [`verify_assertion`] check the challenge again, against the one found.
#[must_use]
pub fn challenge_of(client_data_json: &[u8]) -> Option<Vec<u8>> {
    let value: serde_json::Value = serde_json::from_slice(client_data_json).ok()?;
    from_b64url(value.get("challenge")?.as_str()?)
}

/// **`clientDataJSON`'s checks**, shared by both ceremonies — the type, challenge,
/// origin and `crossOrigin` steps of WebAuthn's *Registering a New Credential*
/// and *Verifying an Authentication Assertion*.
fn check_client_data(
    rp: RelyingParty<'_>,
    ceremony: &'static str,
    challenge: &[u8],
    client_data_json: &[u8],
) -> Result<(), Refusal> {
    let value: serde_json::Value =
        serde_json::from_slice(client_data_json).map_err(|_| Refusal::ClientDataNotJson)?;
    let object = value.as_object().ok_or(Refusal::ClientDataNotJson)?;
    let text = |name: &'static str| -> Result<&str, Refusal> {
        object
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or(Refusal::ClientDataField(name))
    };

    let kind = text("type")?;
    if kind != ceremony {
        return Err(Refusal::WrongType {
            expected: ceremony,
            found: kind.to_string(),
        });
    }
    // Decoded and compared as bytes, strictly: a challenge spelled with
    // padding or the other alphabet is not the one issued.
    let presented = from_b64url(text("challenge")?).ok_or(Refusal::ChallengeMismatch)?;
    if presented != challenge {
        return Err(Refusal::ChallengeMismatch);
    }
    let origin = text("origin")?;
    if origin != rp.origin {
        return Err(Refusal::OriginMismatch {
            expected: rp.origin.to_string(),
            found: origin.to_string(),
        });
    }
    match object.get("crossOrigin") {
        None | Some(serde_json::Value::Bool(false)) => {}
        Some(serde_json::Value::Bool(true)) => return Err(Refusal::CrossOrigin),
        Some(_) => return Err(Refusal::ClientDataField("crossOrigin")),
    }
    Ok(())
}

/// Authenticator data, read — WebAuthn's *Authenticator Data*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatorData {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub sign_count: u32,
    /// Present when the AT flag is.
    pub attested: Option<AttestedCredential>,
}

/// The attested credential inside registration authenticator data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedCredential {
    pub aaguid: [u8; 16],
    pub id: Vec<u8>,
    pub algorithm: Algorithm,
    pub public_key: Vec<u8>,
}

/// **Read authenticator data**, refusing anything its own flags do not
/// account for.
///
/// With AT set, the attested credential follows the fixed part; with ED set,
/// one CBOR map of extension outputs follows that. After those, nothing: a
/// byte this function did not account for is refused rather than ignored,
/// because the signature covers it and a reader that skipped it would be
/// verifying bytes it never looked at.
///
/// # Errors
///
/// [`Refusal::AuthenticatorData`], [`Refusal::CredentialIdLength`],
/// [`Refusal::PublicKey`] and [`Refusal::UnsupportedAlgorithm`].
pub fn parse_authenticator_data(bytes: &[u8]) -> Result<AuthenticatorData, Refusal> {
    if bytes.len() < AUTH_DATA_FIXED {
        return Err(Refusal::AuthenticatorData("shorter than its fixed 37 bytes"));
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&bytes[..32]);
    let flags = bytes[32];
    let sign_count = u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]);
    let mut rest = &bytes[AUTH_DATA_FIXED..];

    let attested = if flags & FLAG_AT == 0 {
        None
    } else {
        if rest.len() < 18 {
            return Err(Refusal::AuthenticatorData(
                "the AT flag is set and the attested credential is truncated",
            ));
        }
        let mut aaguid = [0u8; 16];
        aaguid.copy_from_slice(&rest[..16]);
        let length = usize::from(u16::from_be_bytes([rest[16], rest[17]]));
        rest = &rest[18..];
        if length == 0 || length > MAX_CREDENTIAL_ID {
            return Err(Refusal::CredentialIdLength(length));
        }
        if rest.len() < length {
            return Err(Refusal::AuthenticatorData(
                "the credential ID is longer than what follows it",
            ));
        }
        let id = rest[..length].to_vec();
        rest = &rest[length..];
        let key: ciborium::Value = ciborium::from_reader(&mut rest)
            .map_err(|_| Refusal::PublicKey("not one CBOR value"))?;
        let (algorithm, public_key) = cose_key(&key)?;
        Some(AttestedCredential {
            aaguid,
            id,
            algorithm,
            public_key,
        })
    };

    if flags & FLAG_ED != 0 {
        let extensions: ciborium::Value = ciborium::from_reader(&mut rest)
            .map_err(|_| Refusal::AuthenticatorData("the ED flag is set and no CBOR follows"))?;
        if !matches!(extensions, ciborium::Value::Map(_)) {
            return Err(Refusal::AuthenticatorData("the extension outputs are not a map"));
        }
    }
    if !rest.is_empty() {
        return Err(Refusal::AuthenticatorData(
            "bytes follow what its flags account for",
        ));
    }
    Ok(AuthenticatorData {
        rp_id_hash,
        flags,
        sign_count,
        attested,
    })
}

/// An integer CBOR value, or `None`.
fn integer(value: &ciborium::Value) -> Option<i128> {
    match value {
        ciborium::Value::Integer(i) => Some(i128::from(*i)),
        _ => None,
    }
}

/// **A COSE_Key to an algorithm and a public key** — RFC 9053's key types, and only
/// the two shapes this relying party accepts.
fn cose_key(key: &ciborium::Value) -> Result<(Algorithm, Vec<u8>), Refusal> {
    let ciborium::Value::Map(entries) = key else {
        return Err(Refusal::PublicKey("not a map"));
    };
    let field = |label: i128| -> Option<&ciborium::Value> {
        let mut found = entries.iter().filter(|(k, _)| integer(k) == Some(label));
        let first = found.next().map(|(_, v)| v);
        // A label twice is two answers to one question.
        if found.next().is_some() { None } else { first }
    };
    let bytes = |label: i128, what: &'static str| -> Result<&Vec<u8>, Refusal> {
        match field(label) {
            Some(ciborium::Value::Bytes(b)) if b.len() == 32 => Ok(b),
            _ => Err(Refusal::PublicKey(what)),
        }
    };

    let alg = field(3)
        .and_then(integer)
        .ok_or(Refusal::PublicKey("no algorithm (label 3)"))?;
    let kty = field(1)
        .and_then(integer)
        .ok_or(Refusal::PublicKey("no key type (label 1)"))?;
    let crv = field(-1).and_then(integer);
    match alg {
        -7 => {
            if kty != 2 {
                return Err(Refusal::PublicKey("ES256 with a key type other than EC2"));
            }
            if crv != Some(1) {
                return Err(Refusal::PublicKey("ES256 on a curve other than P-256"));
            }
            let x = bytes(-2, "an EC2 key without a 32-byte x")?;
            let y = bytes(-3, "an EC2 key without a 32-byte y")?;
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            Ok((Algorithm::Es256, point))
        }
        -8 => {
            if kty != 1 {
                return Err(Refusal::PublicKey("Ed25519 with a key type other than OKP"));
            }
            if crv != Some(6) {
                return Err(Refusal::PublicKey("EdDSA on a curve other than Ed25519"));
            }
            let x = bytes(-2, "an OKP key without a 32-byte x")?;
            Ok((Algorithm::Ed25519, x.clone()))
        }
        other => Err(Refusal::UnsupportedAlgorithm(other)),
    }
}

/// The RP ID hash, UP and UV — shared by both ceremonies.
fn check_flags(
    rp: RelyingParty<'_>,
    uv: UserVerification,
    data: &AuthenticatorData,
) -> Result<(), Refusal> {
    if data.rp_id_hash != sha256(rp.id.as_bytes()) {
        return Err(Refusal::RpIdHashMismatch);
    }
    if data.flags & FLAG_UP == 0 {
        return Err(Refusal::UserNotPresent);
    }
    if uv == UserVerification::Required && data.flags & FLAG_UV == 0 {
        return Err(Refusal::UserNotVerified);
    }
    Ok(())
}

/// **The registration ceremony's checks** — WebAuthn's *Registering a New
/// Credential*, attestation `none`.
///
/// `challenge` is the one the caller issued for this ceremony; whether it was
/// still live and not used before is the caller's to know.
///
/// # Errors
///
/// A [`Refusal`] naming the first check that failed.
pub fn verify_registration(
    rp: RelyingParty<'_>,
    uv: UserVerification,
    challenge: &[u8],
    client_data_json: &[u8],
    attestation_object: &[u8],
) -> Result<Credential, Refusal> {
    check_client_data(rp, "webauthn.create", challenge, client_data_json)?;

    let mut rest = attestation_object;
    let object: ciborium::Value = ciborium::from_reader(&mut rest)
        .map_err(|_| Refusal::AttestationObject("not CBOR"))?;
    if !rest.is_empty() {
        return Err(Refusal::AttestationObject("bytes follow the CBOR map"));
    }
    let ciborium::Value::Map(entries) = object else {
        return Err(Refusal::AttestationObject("not a map"));
    };
    let field = |name: &str| -> Option<&ciborium::Value> {
        let mut found = entries
            .iter()
            .filter(|(k, _)| matches!(k, ciborium::Value::Text(t) if t == name));
        let first = found.next().map(|(_, v)| v);
        if found.next().is_some() { None } else { first }
    };
    let Some(ciborium::Value::Text(fmt)) = field("fmt") else {
        return Err(Refusal::AttestationObject("no text `fmt`"));
    };
    if fmt != "none" {
        return Err(Refusal::AttestationFormat(fmt.clone()));
    }
    match field("attStmt") {
        Some(ciborium::Value::Map(statement)) if statement.is_empty() => {}
        Some(ciborium::Value::Map(_)) => return Err(Refusal::AttestationStatementNotEmpty),
        _ => return Err(Refusal::AttestationObject("no map `attStmt`")),
    }
    let Some(ciborium::Value::Bytes(auth_data)) = field("authData") else {
        return Err(Refusal::AttestationObject("no byte string `authData`"));
    };

    let data = parse_authenticator_data(auth_data)?;
    check_flags(rp, uv, &data)?;
    let attested = data.attested.ok_or(Refusal::NoAttestedCredential)?;
    Ok(Credential {
        id: attested.id,
        algorithm: attested.algorithm,
        public_key: attested.public_key,
        sign_count: data.sign_count,
    })
}

/// **The authentication ceremony's checks** — WebAuthn's *Verifying an
/// Authentication Assertion*.
///
/// `credential_id` is the ID the browser said it used; `credential` is what
/// the caller looked up by it. On success the caller stores
/// [`Asserted::sign_count`].
///
/// **The counter rule** (WebAuthn's *Signature Counter Considerations*): when either the stored or the presented
/// counter is non-zero, the presented one must be greater. Two zeros is an
/// authenticator that keeps no counter, which is ordinary.
///
/// # Errors
///
/// A [`Refusal`] naming the first check that failed.
#[allow(clippy::too_many_arguments)]
pub fn verify_assertion(
    rp: RelyingParty<'_>,
    uv: UserVerification,
    challenge: &[u8],
    credential: &Credential,
    credential_id: &[u8],
    client_data_json: &[u8],
    authenticator_data: &[u8],
    signature: &[u8],
) -> Result<Asserted, Refusal> {
    if credential_id != credential.id.as_slice() {
        return Err(Refusal::CredentialMismatch);
    }
    check_client_data(rp, "webauthn.get", challenge, client_data_json)?;
    let data = parse_authenticator_data(authenticator_data)?;
    check_flags(rp, uv, &data)?;

    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(&sha256(client_data_json));
    ring::signature::UnparsedPublicKey::new(credential.algorithm.verifier(), &credential.public_key)
        .verify(&signed, signature)
        .map_err(|_| Refusal::SignatureInvalid)?;

    let (stored, presented) = (credential.sign_count, data.sign_count);
    if (stored != 0 || presented != 0) && presented <= stored {
        return Err(Refusal::CounterRegression { stored, presented });
    }
    Ok(Asserted {
        sign_count: presented,
    })
}

#[cfg(test)]
mod authenticator;
#[cfg(test)]
mod captured;
#[cfg(test)]
mod captured_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod w3c_tests;
