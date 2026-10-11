//! **TLS with pinned Ed25519 keys — `ADR-035` §7.2 over `ADR-025` §2's trust
//! model.**
//!
//! `ADR-025` §2 ruled the trust model and this crate implements it without
//! adding to it: *"the fleet holds a map of device to public key; the device
//! holds the fleet's public key. Mutual, symmetric, and about as much machinery
//! as it sounds like."* `ADR-035` §7.2 ruled how a key is presented on the
//! wire — a self-signed certificate, pinned by public key — and `ADR-009` §3.7
//! chose the library.
//!
//! # The certificate is a container and the pin is the authority
//!
//! Every verifier here does the same two things and nothing else:
//!
//! 1. extract the Ed25519 public key from the presented certificate, and
//! 2. refuse unless that key is one it was told to expect.
//!
//! **No chain is walked, no issuer is consulted, no name is matched and no
//! validity window is read.** There is no certificate authority to walk *to*
//! (`ADR-025` §2), so a chain would be a decoration, and this crate refuses one
//! outright rather than ignoring it — a peer that sends intermediates is a peer
//! that has misunderstood the model, and saying so is more useful than
//! silently succeeding.
//!
//! # Expiry is IGNORED, and this crate says so because §7.1 requires it
//!
//! `ADR-035` §7.1 permits either ignoring the validity window on both sides or
//! setting it far enough out that a 1970 clock cannot fall outside it, **and
//! requires the machine to say which it does.** This is the saying:
//!
//! > **Rivet ignores certificate expiry entirely.** [`PinnedPeer`] never reads
//! > `notBefore` or `notAfter`. A certificate a thousand years out of date and
//! > one minted a moment ago are treated identically, and the only question
//! > asked of either is whether the key inside is pinned.
//!
//! That is the honest option rather than the convenient one. `ADR-021`
//! establishes that this hardware has no RTC and boots with a wrong clock, and
//! `ADR-025` §2 names exactly this: *"certificate validity windows are a poor
//! thing to depend on when time is itself unreliable at exactly the moment a
//! device is trying to connect."* A check that consults a clock known to be
//! wrong is a check that fails randomly, and one that *looks* real while doing
//! nothing is the failure mode this project keeps finding.
//!
//! **Revocation is removing a pin**, not expiry and not a CRL — `ADR-025` §2.
//!
//! # What this crate does not do
//!
//! **It never mints a certificate.** `ADR-025` §3 puts generation in the fleet
//! tool — *"the fleet tool generates the pair, writes it into the provisioning
//! file, and retains only the public key"* — and `ADR-019` §2 requires
//! credentials to reach a card by the operator's tooling rather than a network.
//! So minting lives in `cmd/cargo-rivet`, and a card carries a finished
//! certificate it did not make.
//!
//! That is not only a layering preference: minting needs `rcgen`, which brings
//! `time` and seven other crates onto anything that links it. A date library,
//! on a card, to write a validity window this crate forbids anyone from
//! trusting, would be the funniest dependency in the project.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};

/// The length of an Ed25519 seed, public key and pin — all thirty-two bytes.
///
/// The same number as `rivet_crypto::SEED_BYTES`, and deliberately not imported
/// from there: this crate does not depend on `rivet-crypto` and adding a
/// dependency to share a `32` would be worse than writing it twice.
pub const KEY_BYTES: usize = 32;

/// The PKCS#8 v1 prefix for an Ed25519 private key.
///
/// # Why this is a constant and not a call into an encoder
///
/// PKCS#8 for Ed25519 has exactly one shape (RFC 8410 §7): a SEQUENCE holding
/// version 0, the AlgorithmIdentifier for `id-Ed25519` (OID 1.3.101.112, and
/// no parameters — the OID's whole point), and an OCTET STRING wrapping an
/// OCTET STRING of the thirty-two seed bytes. Every length is fixed because
/// the seed's length is fixed, so the encoding is a constant with the seed
/// appended and there is nothing for an ASN.1 encoder to decide.
///
/// Reading these sixteen bytes: `30 2e` SEQUENCE of 46; `02 01 00` INTEGER 0;
/// `30 05 06 03 2b 65 70` the algorithm; `04 22` OCTET STRING of 34, holding
/// `04 20` OCTET STRING of 32 — the seed.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// The DER for an Ed25519 `SubjectPublicKeyInfo`, up to the key itself.
///
/// `30 2a` SEQUENCE of 42; `30 05 06 03 2b 65 70` the same algorithm as above;
/// `03 21 00` BIT STRING of 33 with no unused bits — then the thirty-two byte
/// public key. [`public_key_in_certificate`] looks for this.
const SPKI_ED25519_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// A thirty-two byte Ed25519 public key, as it is pinned.
pub type Pin = [u8; KEY_BYTES];

/// Everything that can go wrong here, which is less than usual because the
/// verifier asks one question.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The presented certificate carries no Ed25519 `SubjectPublicKeyInfo`.
    ///
    /// A peer using ECDSA or RSA lands here. That is not a downgrade to refuse
    /// politely — `ADR-025` §2 ruled Ed25519 and a certificate holding
    /// anything else cannot be the pinned key, whatever else is true of it.
    NotEd25519,
    /// The key was extracted and is not pinned. **The ordinary refusal**, and
    /// the one that means the trust model is working.
    NotPinned,
    /// The peer sent intermediates. `ADR-035` §7.2: a self-signed leaf is the
    /// whole certificate.
    ChainPresented(usize),
    /// The peer sent no certificate at all where one is required.
    NoCertificate,
    /// Nothing was pinned, so nothing could ever be accepted.
    ///
    /// Refused at construction rather than at handshake time: an empty pin set
    /// is not a permissive configuration, it is a listener nobody can reach,
    /// and discovering that on a headless board is expensive.
    NoPins,
    /// `rustls` refused the identity — a malformed key, or a provider that
    /// cannot sign with it.
    Identity(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEd25519 => f.write_str(
                "the peer's certificate does not carry an Ed25519 public key. ADR-025 §2 ruled \
                 Ed25519 identities, so a certificate holding an ECDSA or RSA key cannot be the \
                 pinned one no matter what else is true of it",
            ),
            Self::NotPinned => f.write_str(
                "the peer's Ed25519 key is not one this machine was told to expect. THIS IS THE \
                 TRUST MODEL WORKING (ADR-025 §2): the pin is the authority and the certificate \
                 is only a container, so a valid, well-formed, perfectly signed certificate is \
                 refused exactly as a forged one is. Add the key to the pins, or connect from a \
                 machine whose key is already there",
            ),
            Self::ChainPresented(n) => write!(
                f,
                "the peer sent {n} intermediate certificate(s) and ADR-035 §7.2 rules that a \
                 self-signed leaf is the whole certificate. There is no authority to chain TO — \
                 ADR-025 §2 has no CA — so a chain is not merely unnecessary here, it means the \
                 peer has understood a different trust model",
            ),
            Self::NoCertificate => f.write_str(
                "the peer presented no certificate. This end requires one: ADR-025 §2's trust is \
                 MUTUAL and symmetric, so a client authenticates the server and the server \
                 authenticates the client",
            ),
            Self::NoPins => f.write_str(
                "no keys were pinned, so no peer could ever be accepted. ADR-025 §2 makes the pin \
                 the whole of the trust model, which means an empty pin set is not a permissive \
                 configuration — it is a listener nobody can reach. This refuses at startup \
                 rather than letting that be discovered against a headless board",
            ),
            Self::Identity(why) => write!(f, "this machine's own TLS identity was refused: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The PKCS#8 DER for an Ed25519 seed.
///
/// The seed is the private key: `ADR-029` §7.1's Ed25519 and the same
/// thirty-two bytes `rivet_crypto::Ed25519Signer::from_seed` takes, so a device
/// identity is a seed exactly as a signing identity is.
#[must_use]
pub fn pkcs8_from_seed(seed: &[u8; KEY_BYTES]) -> Vec<u8> {
    let mut der = Vec::with_capacity(PKCS8_ED25519_PREFIX.len() + KEY_BYTES);
    der.extend_from_slice(&PKCS8_ED25519_PREFIX);
    der.extend_from_slice(seed);
    der
}

/// One DER element: its tag, its content, the bytes it occupies, and what
/// follows it.
///
/// Enough to walk a structure by lengths without decoding anything inside it,
/// which is all [`public_key_in_certificate`] needs.
struct Element<'a> {
    tag: u8,
    /// The content octets, between the length and the next element.
    content: &'a [u8],
    /// Tag, length and content together — the element's own encoding.
    whole: &'a [u8],
    /// Whatever followed this element in the buffer it was read from.
    rest: &'a [u8],
}

/// Read one DER element from the front of `der`.
///
/// **Refuses rather than guesses.** Every malformed shape returns `None`: a
/// truncated buffer, a length that runs past the end, the indefinite-length
/// form (which BER allows and DER forbids), a length field wider than four
/// bytes, and the high-tag-number form. None of those appear in a certificate,
/// and a parser that half-understood one would be deciding what a peer meant.
fn element(der: &[u8]) -> Option<Element<'_>> {
    let (&tag, after_tag) = der.split_first()?;
    if tag & 0x1f == 0x1f {
        return None; // the high-tag-number form
    }
    let (&first, after_first) = after_tag.split_first()?;
    let (length, body) = if first < 0x80 {
        (usize::from(first), after_first)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 {
            return None; // indefinite, or a certificate larger than 4 GiB
        }
        let (bytes, after_length) = after_first.split_at_checked(count)?;
        let mut length = 0usize;
        for b in bytes {
            length = (length << 8) | usize::from(*b);
        }
        (length, after_length)
    };
    let content = body.get(..length)?;
    let rest = body.get(length..)?;
    let whole = der.get(..der.len() - rest.len())?;
    Some(Element { tag, content, whole, rest })
}

/// The Ed25519 public key inside a certificate, if it has one.
///
/// # Why this walks the structure, and why SEARCHING for the key was a hole
///
/// A general X.509 parser is a large thing to add — and to keep correct — for
/// one field, and this is still not one: an Ed25519 `SubjectPublicKeyInfo` is a
/// **fixed twelve-byte sequence followed by the key**
/// (`SPKI_ED25519_PREFIX`), because every length in it is determined by the
/// algorithm. So there is nothing to decode. What there IS is a **position**,
/// and this walks to it: past `version`, `serialNumber`, `signature`, `issuer`,
/// `validity` and `subject`, by their own lengths, to the field that comes next.
///
/// **Until 2026-09-03 it searched instead** — the first twelve-byte window
/// anywhere in the DER matching that prefix — under a documented argument that
/// the shortcut *"can only refuse, never admit"*, because the value it returns
/// is compared against a pin. **That argument was exactly inverted, and the
/// audit of 2026-09-03 found it** (`PLAN.md` §123.2 item 1).
///
/// Here is the inversion. `issuer` and `subject` both PRECEDE
/// `subjectPublicKeyInfo` in a `TBSCertificate`, and either can be made to
/// carry arbitrary bytes. rustls verifies the handshake signature against the
/// certificate's **real** `subjectPublicKeyInfo`. So: an attacker mints a
/// self-signed certificate on their own key, plants a pinned public key's
/// forty-four bytes in the subject, and **both checks pass** — this function
/// finds the planted copy and answers "pinned", while rustls verifies a
/// signature the attacker genuinely made with the key they genuinely hold.
/// Board public keys are public by design (`ADR-025` §2) and operator public
/// keys sit in every fleet register, so knowing a pinned key was enough to BE
/// one, in either direction, against `rivet-mcpd` or against `rivet-fleet`.
///
/// The fix is not a stronger check. It is reading **the same field rustls
/// verifies**, so that the two cannot disagree. With that, the original
/// asymmetry is finally true: a planted key is never looked at, so it can
/// neither admit nor refuse.
#[must_use]
pub fn public_key_in_certificate(der: &[u8]) -> Option<Pin> {
    const SEQUENCE: u8 = 0x30;
    // `[0] EXPLICIT`, the context-specific constructed tag `version` is wrapped
    // in. It is optional and defaults to v1, so it may be absent.
    const VERSION: u8 = 0xa0;

    let certificate = element(der)?;
    if certificate.tag != SEQUENCE {
        return None;
    }
    let tbs = element(certificate.content)?;
    if tbs.tag != SEQUENCE {
        return None;
    }

    let mut rest = tbs.content;
    if element(rest)?.tag == VERSION {
        rest = element(rest)?.rest;
    }
    // `serialNumber`, `signature`, `issuer`, `validity`, `subject` — five
    // fields, in the order RFC 5280 §4.1 fixes them, skipped by their lengths
    // and never looked inside. Whatever an attacker writes into the two that
    // hold names goes past unread, which is the entire point.
    for _ in 0..5 {
        rest = element(rest)?.rest;
    }

    let spki = element(rest)?;
    if spki.tag != SEQUENCE {
        return None;
    }
    // The element's own bytes ARE the prefix followed by the key when the
    // algorithm is Ed25519, and are some other shape when it is not — so this
    // one comparison answers both "is this Ed25519" and "which key".
    spki.whole.strip_prefix(&SPKI_ED25519_PREFIX)?.try_into().ok()
}

/// The `commonName` out of a certificate's **subject**, for a person to read.
///
/// # It is a LABEL and never an authorisation
///
/// This says what a caller called itself in a certificate that something else
/// already verified. **Nothing here checks anything**, and no decision may rest
/// on the answer: `ADR-028` §4.5's distinction between attribution and
/// authority, in a function. `rivet-proxyd` puts it on the index page so an
/// operator can see which certificate their browser sent; that is the whole of
/// what it is for.
///
/// # It walks, for [`public_key_in_certificate`]'s reason
///
/// Not because a name can be planted to fool this — nothing rests on it — but
/// because a certificate has two `commonName`s, one in the `issuer` and one in
/// the `subject`, and a search finds whichever comes first. That is the issuer,
/// which on this fleet is `home-fleet` and would put the AUTHORITY's name
/// where the caller's belongs. So it skips to `subject` by length, exactly as
/// the key reader skips past it.
///
/// # What it accepts
///
/// `PrintableString` and `UTF8String`, which are what `rcgen` writes and what
/// the web PKI uses. A `commonName` in any other string type answers `None`
/// rather than being decoded half-correctly, and a subject with no
/// `commonName` at all — which is legal — answers `None` too.
#[must_use]
pub fn subject_common_name(der: &[u8]) -> Option<String> {
    const SEQUENCE: u8 = 0x30;
    const SET: u8 = 0x31;
    const OID: u8 = 0x06;
    const PRINTABLE_STRING: u8 = 0x13;
    const UTF8_STRING: u8 = 0x0c;
    const VERSION: u8 = 0xa0;
    /// `2.5.4.3`, `id-at-commonName`, as DER content bytes.
    const COMMON_NAME: [u8; 3] = [0x55, 0x04, 0x03];

    let certificate = element(der)?;
    if certificate.tag != SEQUENCE {
        return None;
    }
    let tbs = element(certificate.content)?;
    if tbs.tag != SEQUENCE {
        return None;
    }

    let mut rest = tbs.content;
    if element(rest)?.tag == VERSION {
        rest = element(rest)?.rest;
    }
    // `serialNumber`, `signature`, `issuer`, `validity` — four fields, and the
    // fifth is the one wanted. The issuer is stepped over rather than searched
    // past, which is the whole reason this is a walk.
    for _ in 0..4 {
        rest = element(rest)?.rest;
    }
    let subject = element(rest)?;
    if subject.tag != SEQUENCE {
        return None;
    }

    // A Name is a SEQUENCE of RelativeDistinguishedName, each a SET of
    // AttributeTypeAndValue, each a SEQUENCE of an OID and a value. The last
    // `commonName` wins, which is the convention every reader uses for the
    // multi-valued case.
    let mut found: Option<String> = None;
    let mut rdns = subject.content;
    while let Some(rdn) = element(rdns) {
        rdns = rdn.rest;
        if rdn.tag != SET {
            continue;
        }
        let mut attributes = rdn.content;
        while let Some(attribute) = element(attributes) {
            attributes = attribute.rest;
            if attribute.tag != SEQUENCE {
                continue;
            }
            let Some(oid) = element(attribute.content) else {
                continue;
            };
            if oid.tag != OID || oid.content != COMMON_NAME {
                continue;
            }
            let Some(value) = element(oid.rest) else {
                continue;
            };
            if value.tag != PRINTABLE_STRING && value.tag != UTF8_STRING {
                continue;
            }
            // Lossy: this is a display string and a certificate that carried
            // invalid UTF-8 in a name should show as mangled rather than as
            // absent, which would read as *no certificate*.
            found = Some(String::from_utf8_lossy(value.content).into_owned());
        }
    }
    found
}

/// Checks a peer's certificate against a set of pinned Ed25519 keys, and does
/// nothing else at all.
///
/// One type implements **both** [`ServerCertVerifier`] and
/// [`ClientCertVerifier`], because `ADR-025` §2's trust is symmetric and two
/// types would be two places to change one rule (`ADR-011`).
///
/// **The pin set can be replaced while it is in use** — [`PinnedPeer::replace`],
/// `ADR-063` §3. Every handshake reads the set as it stands at that moment, so a
/// replacement governs the next handshake and touches no session already made.
#[derive(Debug)]
pub struct PinnedPeer {
    pins: RwLock<Vec<Pin>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinnedPeer {
    /// Refuses an empty pin set — see [`Error::NoPins`].
    pub fn new(
        pins: Vec<Pin>,
        provider: Arc<rustls::crypto::CryptoProvider>,
    ) -> Result<Self, Error> {
        if pins.is_empty() {
            return Err(Error::NoPins);
        }
        Ok(Self { pins: RwLock::new(pins), provider })
    }

    /// The set, read. A poisoned lock still holds a whole `Vec` — the only
    /// writer is [`PinnedPeer::replace`], which assigns and cannot leave half
    /// of one — so it is read through rather than refused.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Vec<Pin>> {
        self.pins.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The one question this type asks.
    fn check(&self, end_entity: &CertificateDer<'_>, chain: usize) -> Result<(), Error> {
        if chain != 0 {
            return Err(Error::ChainPresented(chain));
        }
        let key = public_key_in_certificate(end_entity.as_ref()).ok_or(Error::NotEd25519)?;
        if self.read().contains(&key) {
            Ok(())
        } else {
            Err(Error::NotPinned)
        }
    }

    /// How many keys this will accept. For a machine reporting its own state.
    #[must_use]
    pub fn pinned(&self) -> usize {
        self.read().len()
    }

    /// The keys this accepts now, in the order they were given.
    #[must_use]
    pub fn pins(&self) -> Vec<Pin> {
        self.read().clone()
    }

    /// **Replace the whole set**, for every handshake from now on.
    ///
    /// # Errors
    ///
    /// [`Error::NoPins`] for an empty set, and the set in force is unchanged —
    /// the same refusal [`PinnedPeer::new`] makes, for the same reason.
    pub fn replace(&self, pins: Vec<Pin>) -> Result<(), Error> {
        if pins.is_empty() {
            return Err(Error::NoPins);
        }
        *self.pins.write().unwrap_or_else(PoisonError::into_inner) = pins;
        Ok(())
    }
}

fn refuse(e: Error) -> rustls::Error {
    rustls::Error::General(e.to_string())
}

/// **TLS 1.2 is not spoken here** — ruled 2026-09-04, `.rulings/` R-140.
///
/// `rustls` requires this hook on both verifier traits whether or not the
/// `tls12` feature is compiled in, so the method cannot simply be deleted. It
/// refuses, loudly, rather than being left to call a function that is no longer
/// there — and a refusal is the honest answer: reaching it means a peer
/// negotiated a version this build does not intend to serve.
///
/// # What the check found before the feature came out
///
/// Measured against the board on 2026-09-04, not assumed. Every consumer of
/// this crate negotiates **1.3** when offered both — and an `openssl s_client
/// -tls1_2` was nevertheless **accepted** by `rivet-fleetd` on 443 and by
/// `rivet-mcpd` on 7777, failing afterwards on the client certificate rather
/// than on the version. So the 1.2 path was live and reachable by an
/// unauthenticated caller, which is the surface the ruling removes.
///
/// The two outside clients the ruling required checking first: a browser
/// against the fleet page, which negotiates 1.3 and has since 2020; and the
/// phone that pairs with Reliquary, which **does not depend on this crate at
/// all** — `cargo tree -p reliquary-rivet` names no `rivetos-tls`, so nothing
/// here can affect it.
///
/// # Errors
///
/// Always. The arguments are named for the trait and unused.
fn no_tls12(
    _message: &[u8],
    _cert: &CertificateDer<'_>,
    _dss: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, rustls::Error> {
    Err(rustls::Error::General(
        "this build speaks TLS 1.3 only (.rulings/ R-140); a 1.2 handshake signature reached \
         a verifier that no longer has a 1.2 implementation to call"
            .to_string(),
    ))
}

/// Refuse a handshake signature that is not Ed25519.
///
/// **Defence in depth and not the load-bearing check.** The load-bearing one is
/// [`public_key_in_certificate`]: the certificate carries an Ed25519 key or it
/// is refused, and rustls verifies the signature against that same key, so a
/// peer claiming an ECDSA scheme over an Ed25519 key fails anyway. This makes
/// the intent explicit at the one place a future edit could widen it by
/// accident, and it costs a comparison.
fn ed25519_only(dss: &DigitallySignedStruct) -> Result<(), rustls::Error> {
    if dss.scheme == SignatureScheme::ED25519 {
        Ok(())
    } else {
        Err(refuse(Error::NotEd25519))
    }
}

impl ServerCertVerifier for PinnedPeer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // `_server_name` is ignored deliberately: the pin identifies the peer,
        // and a name would be a second authority for the same question
        // (ADR-011). `_now` is ignored because expiry is — see this module.
        self.check(end_entity, intermediates.len())
            .map(|()| ServerCertVerified::assertion())
            .map_err(refuse)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        no_tls12(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        ed25519_only(dss)?;
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    /// **Ed25519 and nothing else** — `ADR-025` §2 ruled the algorithm, and
    /// `PinnedPeer::check` already refuses a certificate carrying any other
    /// kind of key. Advertising the provider's full list said this end would
    /// consider a signature it was always going to reject, one field later.
    /// Narrowing it moves the refusal earlier and makes it say why.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

impl ClientCertVerifier for PinnedPeer {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        // Nothing to hint. A hint names certificate authorities a client could
        // chain to, and ADR-025 §2 has none.
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.check(end_entity, intermediates.len())
            .map(|()| ClientCertVerified::assertion())
            .map_err(refuse)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        no_tls12(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        ed25519_only(dss)?;
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    /// **Ed25519 and nothing else** — `ADR-025` §2 ruled the algorithm, and
    /// `PinnedPeer::check` already refuses a certificate carrying any other
    /// kind of key. Advertising the provider's full list said this end would
    /// consider a signature it was always going to reject, one field later.
    /// Narrowing it moves the refusal earlier and makes it say why.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// This machine's own end of the connection: the certificate it presents and
/// the seed it proves with.
pub struct Identity {
    /// The self-signed leaf, DER. Minted by host tooling — see this module.
    pub certificate: Vec<u8>,
    /// The Ed25519 seed whose public half is inside that certificate.
    pub seed: [u8; KEY_BYTES],
}

impl fmt::Debug for Identity {
    /// **The seed never reaches a log.** `Debug` on a struct holding a private
    /// key is one derive away from printing it, so this is written by hand.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("certificate", &format_args!("{} bytes", self.certificate.len()))
            .field("seed", &"<not shown>")
            .finish()
    }
}

impl Identity {
    /// The public key this identity presents, read back out of its own
    /// certificate.
    ///
    /// **Read from the certificate rather than derived from the seed** — there
    /// is no `agrees()` method here to compare it against the seed; `rustls`
    /// itself already catches a mismatched pair (see below).
    #[must_use]
    pub fn public_key(&self) -> Option<Pin> {
        public_key_in_certificate(&self.certificate)
    }

    // **There is no `agrees()` here, and that is deliberate.** An earlier
    // draft had one, to catch a card provisioned with a certificate from one
    // keypair and a seed from another. `rustls` already does it, properly,
    // and said so the first time the tests ran a mismatched pair:
    //
    // ```text
    // keys may not be consistent: KeyMismatch
    // ```
    //
    // `with_single_cert` and `with_client_auth_cert` both check that the
    // certificate's public key matches the private key before they will serve
    // it, so [`server_config`] and [`client_config`] already refuse the
    // mismatch — as [`Error::Identity`], carrying that message. A method here
    // could only have been weaker, because this crate has no Ed25519
    // implementation of its own to derive a public key with, and a check that
    // is weaker than the one already running is `ADR-011`'s second authority.

    fn private_key_der(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8_from_seed(&self.seed)))
    }
}

/// A server that presents `identity` and accepts only peers holding `pins`.
///
/// **Client authentication is required, not optional.** `ADR-035`'s
/// `rivet-mcpd` hands a caller the stdin and stdout of a program that installs
/// payloads and reboots the machine; a server-authenticated-only TLS session
/// would prove to the *caller* which board it reached and prove nothing to the
/// board about who called. That is the arrangement `cmd/rivet-mcpd` already
/// refuses to bind a routable address for.
pub fn server_config(
    identity: &Identity,
    pins: Vec<Pin>,
    provider: Arc<rustls::crypto::CryptoProvider>,
) -> Result<ServerConfig, Error> {
    let verifier = Arc::new(PinnedPeer::new(pins, provider.clone())?);
    server_config_verified_by(identity, verifier, provider)
}

/// A server whose pins are replaced while it runs: `verifier` stays with the
/// caller, and [`PinnedPeer::replace`] on it governs the next handshake.
///
/// **Session resumption is off, and that is what makes a removal real.** A
/// resumed TLS 1.3 session does not present a certificate again: the server
/// restores the peer's certificate from its session store and never asks the
/// verifier. With resumption on, a key removed from the pins would still be
/// admitted by any client holding a ticket from before the removal. Every
/// connection here is a full handshake, which costs one Ed25519 signature and
/// one verification — nothing, next to the program each connection starts.
///
/// # Errors
///
/// As [`server_config`], less [`Error::NoPins`], which `verifier` already
/// refused when it was made.
pub fn live_server_config(
    identity: &Identity,
    verifier: Arc<PinnedPeer>,
    provider: Arc<rustls::crypto::CryptoProvider>,
) -> Result<ServerConfig, Error> {
    let mut config = server_config_verified_by(identity, verifier, provider)?;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    Ok(config)
}

fn server_config_verified_by(
    identity: &Identity,
    verifier: Arc<PinnedPeer>,
    provider: Arc<rustls::crypto::CryptoProvider>,
) -> Result<ServerConfig, Error> {
    ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Identity(e.to_string()))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![CertificateDer::from(identity.certificate.clone())],
            identity.private_key_der(),
        )
        .map_err(|e| Error::Identity(e.to_string()))
}

/// A client that presents `identity` and accepts only servers holding `pins`.
pub fn client_config(
    identity: &Identity,
    pins: Vec<Pin>,
    provider: Arc<rustls::crypto::CryptoProvider>,
) -> Result<ClientConfig, Error> {
    let verifier = Arc::new(PinnedPeer::new(pins, provider.clone())?);
    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Identity(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(
            vec![CertificateDer::from(identity.certificate.clone())],
            identity.private_key_der(),
        )
        .map_err(|e| Error::Identity(e.to_string()))
}

/// The provider this crate uses, named once.
#[must_use]
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[cfg(test)]
mod tests;

// **Whose certificate a verified client presented** — `.rulings/` R-187 D1.
// A module of its own because it answers a different question from everything
// above: those pin a machine's key, and this tells a fleet authority's two kinds
// of leaf apart. It walks with [`element`], so it lives beside it.
mod caller;
pub use caller::{Caller, classify_caller, dns_names};

/// The file holding a device's Ed25519 seed — thirty-two raw bytes, mode 0600.
///
/// **Named here rather than in either consumer**, because `cmd/cargo-rivet`
/// writes this file and `cmd/rivet-mcpd` reads it, and a name spelled in two
/// crates is two names (`ADR-011`). `ADR-025` §2.3 puts it under
/// `/state/secrets`; this constant is the leaf and the directory is the
/// caller's, because a *path* is a statement about a machine and a *file name*
/// is a statement about a format.
pub const SEED_FILE: &str = "device.seed";

/// The file holding a device's self-signed certificate — DER, not PEM, and not
/// secret.
///
/// DER because it is what `rustls` takes and what this crate searches; PEM
/// would be base64 wrapping the same bytes plus a parser to remove it.
pub const CERTIFICATE_FILE: &str = "device.crt";

/// The third file of the same triple: the Ed25519 public keys a machine's
/// control plane will accept, one hex key per line.
///
/// **Not a secret.** `ADR-025` §2 calls the fleet's public key *"ordinary
/// configuration — public by definition, and what matters for it is integrity,
/// not confidentiality"*.
///
/// It is here beside the other two because they travel together — `cargo rivet
/// identity new` writes the first two and `cargo rivet setup` writes this one
/// into the same directory, and `cmd/rivet-image`'s `--identity DIR` takes
/// *"all three or none"*. `cmd/rivet-image` and `cmd/rivet-initd` each spell it
/// for themselves and neither depends on this crate; this is the spelling for
/// anything that does, rather than a fourth invention (`ADR-011`).
pub const PEERS_FILE: &str = "peers";

/// Read the thirty-two byte seed and the certificate from one directory.
///
/// **The refusals name the file and the fix**, because the machine that fails
/// this is headless and the message is the only thing its operator gets.
///
/// # Errors
///
/// If either file is missing or unreadable, or the seed is not exactly
/// [`KEY_BYTES`] bytes long.
/// **A sentence about the PATH FORM, when a Windows open fails on a path that
/// is not one.**
///
/// # The failure, which cost real time on 2026-08-29
///
/// `--identity /c/Users/someone/.rivet/identity/operator` on Windows produced:
///
/// ```text
/// cannot read this machine's TLS seed at
/// /c/Users/someone/.rivet/identity/operator\device.seed: The system cannot
/// find the path specified. (os error 3)
/// ```
///
/// Read that path: **half MSYS, half Windows.** A Unix-shell-on-Windows prints
/// `$HOME` as `/c/Users/…`, the operator pastes it, and `Path::join` appends a
/// backslash to a form Windows has never understood. The message then described
/// a **missing file**, which is not what was wrong — the file is there and the
/// path form is not one this platform resolves. That is the confidently-wrong
/// shape `ADR-012` §7.4 is about, and an operator debugs the wrong thing.
///
/// # It adds a clause and never changes the verdict
///
/// The read failed and it still fails; this only says something the machine can
/// see. It fires on the **shape** — a leading `/` then a single letter then `/`
/// — and only where that shape cannot be a real directory, which is why it is
/// `#[cfg(windows)]`: on Linux `/c/…` is an ordinary path and a machine that
/// second-guessed it would be wrong about a real one.
#[cfg(windows)]
fn unix_shaped_hint(directory: &std::path::Path) -> String {
    let text = directory.to_string_lossy();
    let bytes = text.as_bytes();
    let msys_shaped = bytes.len() > 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b'/';
    if !msys_shaped {
        return String::new();
    }
    format!(
        ". AND THAT PATH IS NOT A WINDOWS PATH: `{}` is the form a Unix shell on Windows \
         prints, and this program resolves it through the platform, which has never \
         understood it — so the failure above is about the FORM and not about a missing \
         file. Give it as `{}:\\{}` — `cygpath -w` converts one",
        text,
        &text[1..2].to_uppercase(),
        text[3..].replace('/', "\\")
    )
}

/// See the `windows` definition; every other platform resolves `/c/…` itself and
/// has nothing to add.
#[cfg(not(windows))]
fn unix_shaped_hint(_directory: &std::path::Path) -> String {
    String::new()
}

pub fn read_identity(directory: &std::path::Path) -> Result<Identity, String> {
    let seed_at = directory.join(SEED_FILE);
    let cert_at = directory.join(CERTIFICATE_FILE);

    let raw = std::fs::read(&seed_at).map_err(|e| {
        format!(
            "cannot read this machine's TLS seed at {}: {e}. ADR-025 §3 has the fleet tool \
             generate the pair and write it to the card, so a machine without one was never \
             provisioned rather than having lost something — `cargo rivet identity new` mints \
             it{}",
            seed_at.display(),
            unix_shaped_hint(directory)
        )
    })?;
    let seed: [u8; KEY_BYTES] = raw.as_slice().try_into().map_err(|_| {
        format!(
            "this machine's TLS seed at {} is {} bytes and an Ed25519 seed is exactly {}. A \
             truncated or text-wrapped copy is the usual cause — the file holds RAW bytes, not \
             hex and not base64",
            seed_at.display(),
            raw.len(),
            KEY_BYTES
        )
    })?;

    let certificate = std::fs::read(&cert_at).map_err(|e| {
        format!(
            "cannot read this machine's TLS certificate at {}: {e}. It is minted beside the seed \
             and the two must come from one `cargo rivet identity new` — rustls refuses a \
             certificate whose key is not the seed's, with `KeyMismatch`",
            cert_at.display()
        )
    })?;

    Ok(Identity { certificate, seed })
}

/// Parse a pin file: one hex-encoded Ed25519 public key per line.
///
/// Blank lines are skipped and `#` starts a comment, so an operator can write
/// down whose key each one is — which matters more here than usual, because a
/// pin file is a list of who may control the machine and a bare column of hex
/// is unreviewable.
///
/// # Errors
///
/// If a line is not [`KEY_BYTES`] bytes of hex, naming the line number.
pub fn parse_pins(text: &str) -> Result<Vec<Pin>, String> {
    let mut pins = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bytes = decode_hex(line)
            .ok_or_else(|| format!("line {}: {line:?} is not hexadecimal", n + 1))?;
        let pin: Pin = bytes.as_slice().try_into().map_err(|_| {
            format!(
                "line {}: an Ed25519 public key is {KEY_BYTES} bytes and this is {}",
                n + 1,
                bytes.len()
            )
        })?;
        pins.push(pin);
    }
    Ok(pins)
}

/// Hex, as a pin file writes it. Lower or upper case, no separators.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Hex, as a pin file reads it — for tooling that has to print a key.
#[must_use]
pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
