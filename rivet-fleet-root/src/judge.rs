//! **Whether a certificate an application holds is one this machine's root
//! issued, and is still good** — `ADR-063` §4.
//!
//! A fleet-root machine re-issues a leaf that is missing, that its own root did
//! not sign, that is near its end, or that lacks the fields a chain engine
//! needs. Each of those is a question about a certificate already on the disk,
//! so this module reads one back. It reads only the handful of fields those
//! questions need, with a DER walk written here, and checks the signature with
//! `ring`, which every card already carries: no X.509 library reaches a card
//! for it.
//!
//! [`Root`] is read from the root's certificate alone, which is public. Judging
//! a leaf never needs the root's private key, so the process that judges never
//! holds it; only the process that signs does ([`issue_from_root`]).

use rcgen::{
    DistinguishedName, DnType, Issuer, KeyIdMethod, KeyPair, KeyUsagePurpose,
};

use crate::{Error, FLOOR_YEAR, Pair};

/// A leaf whose end is closer than this is re-issued: ninety days.
pub const RENEW_WITHIN_SECS: u64 = 90 * 86_400;

/// What [`judge`] made of a leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Signed by this root, carrying every standard field, and not near its end.
    Current {
        /// When it ends, in seconds since the epoch.
        not_after: u64,
    },
    /// There is no leaf, or no part of one, where there should be. [`judge`]
    /// never answers this; it is the caller's word for a file that is not
    /// there, so every reason a leaf is replaced reads alike.
    Missing(String),
    /// Not a certificate this reader can read.
    Unreadable(String),
    /// Signed by something other than this root. `issuer` is the common name the
    /// leaf names as its issuer, or a description when it names none.
    Foreign {
        /// Who the leaf says signed it.
        issuer: String,
    },
    /// Signed by this root, and ending within [`RENEW_WITHIN_SECS`] of now, or
    /// already ended.
    NearExpiry {
        /// When it ends, in seconds since the epoch.
        not_after: u64,
    },
    /// Signed by this root, in date, and missing fields a chain engine needs.
    LacksFields {
        /// The fields that are absent or wrong, in words.
        missing: Vec<&'static str>,
    },
}

impl Verdict {
    /// Whether this leaf should be replaced.
    #[must_use]
    pub fn wants_issuance(&self) -> bool {
        !matches!(self, Self::Current { .. })
    }

    /// One phrase for a journal record: why a leaf is being replaced, or that it
    /// is not.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Current { .. } => "it is current".to_owned(),
            Self::Missing(what) => format!("{what} is missing"),
            Self::Unreadable(why) => format!("it does not read as a certificate: {why}"),
            Self::Foreign { issuer } => {
                format!("it was issued by {issuer}, not by this machine's fleet root")
            }
            Self::NearExpiry { not_after } => {
                format!("it ends at {not_after} (seconds since the epoch), within ninety days")
            }
            Self::LacksFields { missing } => format!("it lacks {}", missing.join(", ")),
        }
    }
}

/// A fleet's root, read from its certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    subject: Vec<u8>,
    common_name: Option<String>,
    public_key: Vec<u8>,
    subject_key_id: Option<Vec<u8>>,
}

impl Root {
    /// Read a root from its PEM certificate.
    ///
    /// # Errors
    ///
    /// A sentence saying why it does not read, or why it is not a P-256 key.
    pub fn from_pem(pem: &str) -> Result<Self, String> {
        let der = crate::der_of_pem(pem).ok_or_else(|| "the root is not a PEM certificate".to_owned())?;
        let parsed = Parsed::read(&der)?;
        if parsed.key_algorithm != Some(EcKey::P256) {
            return Err("the root's key is not an ECDSA P-256 key".to_owned());
        }
        Ok(Self {
            common_name: common_name(&parsed.subject),
            subject: parsed.subject,
            public_key: parsed.public_key,
            subject_key_id: parsed.subject_key_id,
        })
    }

    /// The root's common name — the fleet.
    #[must_use]
    pub fn common_name(&self) -> Option<&str> {
        self.common_name.as_deref()
    }
}

/// **Judge one leaf against this machine's root**, at `now` (seconds since the
/// epoch). A clock that reads earlier than [`FLOOR_YEAR`] is judged as if it
/// read the floor, as [`crate::mint`] dates a certificate.
#[must_use]
pub fn judge(leaf_pem: &str, root: &Root, now: u64) -> Verdict {
    let Some(der) = crate::der_of_pem(leaf_pem) else {
        return Verdict::Unreadable("no PEM certificate was found".to_owned());
    };
    let leaf = match Parsed::read(&der) {
        Ok(leaf) => leaf,
        Err(why) => return Verdict::Unreadable(why),
    };

    if !chains(&leaf, root) {
        return Verdict::Foreign {
            issuer: common_name(&leaf.issuer).map_or_else(
                || "an issuer with no common name".to_owned(),
                |name| format!("CN={name}"),
            ),
        };
    }

    let now = now.max(floor_secs());
    if leaf.not_after <= now.saturating_add(RENEW_WITHIN_SECS) {
        return Verdict::NearExpiry {
            not_after: leaf.not_after,
        };
    }

    let mut missing = Vec::new();
    match (&leaf.authority_key_id, &root.subject_key_id) {
        (None, _) => missing.push("an authority key identifier"),
        (Some(theirs), Some(ours)) if theirs != ours => {
            missing.push("an authority key identifier naming this root");
        }
        _ => {}
    }
    if !leaf.subject_key_id_present {
        missing.push("a subject key identifier");
    }
    if !leaf.key_usage_present {
        missing.push("a key usage");
    }
    if !leaf.server_auth {
        missing.push("serverAuth in its extended key usage");
    }
    if !leaf.client_auth {
        missing.push("clientAuth in its extended key usage");
    }
    if missing.is_empty() {
        Verdict::Current {
            not_after: leaf.not_after,
        }
    } else {
        Verdict::LacksFields { missing }
    }
}

/// **Issue a leaf from a root that rests on this machine** — its certificate
/// and its private key, both PEM — through [`crate::issue_leaf`], and check the
/// result with [`judge`] before handing it back.
///
/// The issuer is built from what the root's certificate says about itself: its
/// subject's common name, and its subject key identifier, which the leaf's
/// authority key identifier must repeat. A leaf that would not judge
/// [`Verdict::Current`] against the same root is refused rather than returned.
///
/// # Errors
///
/// [`Error::Mint`] when the root or its key does not read, when `rcgen`
/// refuses, or when the leaf does not judge current; [`Error::Randomness`] as
/// [`crate::issue_leaf`] refuses.
pub fn issue_from_root(
    root_certificate_pem: &str,
    root_key_pem: &str,
    names: &[String],
    random: &mut dyn FnMut(usize) -> Result<Vec<u8>, String>,
    now: u64,
) -> Result<Pair, Error> {
    let root = Root::from_pem(root_certificate_pem).map_err(Error::Mint)?;
    let Some(fleet) = root.common_name() else {
        return Err(Error::Mint("the root's subject carries no common name".to_owned()));
    };
    let key = KeyPair::from_pem(root_key_pem)
        .map_err(|e| Error::Mint(format!("the root's key does not read: {e}")))?;
    if key.public_key_raw() != root.public_key.as_slice() {
        return Err(Error::Mint(
            "the root's key is not the key its certificate names".to_owned(),
        ));
    }

    let mut params = rcgen::CertificateParams::default();
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, fleet);
    params.distinguished_name = subject;
    if let Some(id) = &root.subject_key_id {
        params.key_identifier_method = KeyIdMethod::PreSpecified(id.clone());
    }
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let issuer = Issuer::new(params, key);

    let leaf = crate::issue_leaf(&issuer, names, random, now)?;
    match judge(&leaf.certificate, &root, now) {
        Verdict::Current { .. } => Ok(leaf),
        other => Err(Error::Mint(format!(
            "the leaf just issued would not be accepted as this root's: {}",
            other.reason()
        ))),
    }
}

/// The first second of [`FLOOR_YEAR`].
fn floor_secs() -> u64 {
    u64::try_from(days_from_civil(i64::from(FLOOR_YEAR), 1, 1)).unwrap_or(0) * 86_400
}

/// Whether `leaf` names `root` as its issuer and `root`'s key signed it.
fn chains(leaf: &Parsed, root: &Root) -> bool {
    if leaf.issuer != root.subject || !leaf.signed_ecdsa_sha256 {
        return false;
    }
    ring::signature::UnparsedPublicKey::new(
        &ring::signature::ECDSA_P256_SHA256_ASN1,
        &root.public_key,
    )
    .verify(&leaf.tbs, &leaf.signature)
    .is_ok()
}

// ---- the DER walk ----------------------------------------------------------

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;
const BOOLEAN: u8 = 0x01;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
const VERSION: u8 = 0xa0;
const EXTENSIONS: u8 = 0xa3;
const KEY_IDENTIFIER: u8 = 0x80;

const OID_COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_SUBJECT_KEY_ID: &[u8] = &[0x55, 0x1d, 0x0e];
const OID_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const OID_AUTHORITY_KEY_ID: &[u8] = &[0x55, 0x1d, 0x23];
const OID_EXTENDED_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
const OID_SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
const OID_CLIENT_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EcKey {
    P256,
}

/// The fields of one certificate the questions above need.
struct Parsed {
    /// The `TBSCertificate`, tag and length included: what the signature covers.
    tbs: Vec<u8>,
    signed_ecdsa_sha256: bool,
    signature: Vec<u8>,
    /// The issuer `Name`, tag and length included, compared by bytes.
    issuer: Vec<u8>,
    /// The subject `Name`, likewise.
    subject: Vec<u8>,
    not_after: u64,
    key_algorithm: Option<EcKey>,
    /// The subject public key's bit string, without its unused-bits octet.
    public_key: Vec<u8>,
    subject_key_id: Option<Vec<u8>>,
    subject_key_id_present: bool,
    authority_key_id: Option<Vec<u8>>,
    key_usage_present: bool,
    server_auth: bool,
    client_auth: bool,
}

/// One tag-length-value: its tag, its content, and the whole of it.
struct Tlv<'a> {
    tag: u8,
    content: &'a [u8],
    whole: &'a [u8],
}

/// Take the next TLV off the front of `input`.
fn next<'a>(input: &mut &'a [u8]) -> Result<Tlv<'a>, String> {
    let bytes = *input;
    let (&tag, rest) = bytes.split_first().ok_or("the DER ends where an element was expected")?;
    let (&first, rest) = rest.split_first().ok_or("the DER ends inside a length")?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return Err("the DER carries a length this reader does not accept".to_owned());
        }
        let len = rest[..count].iter().fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        (len, &rest[count..])
    };
    if rest.len() < len {
        return Err("the DER is shorter than an element says it is".to_owned());
    }
    let header = bytes.len() - rest.len();
    let tlv = Tlv {
        tag,
        content: &rest[..len],
        whole: &bytes[..header + len],
    };
    *input = &rest[len..];
    Ok(tlv)
}

/// [`next`], refusing any tag but `tag`.
fn expect<'a>(input: &mut &'a [u8], tag: u8, what: &str) -> Result<Tlv<'a>, String> {
    let tlv = next(input)?;
    if tlv.tag == tag {
        Ok(tlv)
    } else {
        Err(format!("{what} has tag {:#04x} where {tag:#04x} was expected", tlv.tag))
    }
}

impl Parsed {
    fn read(der: &[u8]) -> Result<Self, String> {
        let mut outer = der;
        let certificate = expect(&mut outer, SEQUENCE, "the certificate")?;
        let mut body = certificate.content;
        let tbs = expect(&mut body, SEQUENCE, "the signed part")?;
        let algorithm = expect(&mut body, SEQUENCE, "the signature algorithm")?;
        let signature = expect(&mut body, BIT_STRING, "the signature")?;
        let signature = match signature.content.split_first() {
            Some((0, bits)) => bits.to_vec(),
            _ => return Err("the signature is not a whole number of octets".to_owned()),
        };
        let mut algorithm = algorithm.content;
        let signed_ecdsa_sha256 = expect(&mut algorithm, OID, "the signature algorithm")?.content
            == OID_ECDSA_SHA256;

        let mut fields = tbs.content;
        let mut first = next(&mut fields)?;
        if first.tag == VERSION {
            first = next(&mut fields)?;
        }
        if first.tag != INTEGER {
            return Err("the serial number is missing".to_owned());
        }
        expect(&mut fields, SEQUENCE, "the inner signature algorithm")?;
        let issuer = expect(&mut fields, SEQUENCE, "the issuer")?.whole.to_vec();
        let mut validity = expect(&mut fields, SEQUENCE, "the validity")?.content;
        let _not_before = next(&mut validity)?;
        let not_after = time(&next(&mut validity)?)?;
        let subject = expect(&mut fields, SEQUENCE, "the subject")?.whole.to_vec();
        let mut key_info = expect(&mut fields, SEQUENCE, "the public key")?.content;
        let mut key_algorithm = expect(&mut key_info, SEQUENCE, "the key algorithm")?.content;
        let kind = expect(&mut key_algorithm, OID, "the key algorithm")?.content;
        let curve = next(&mut key_algorithm).ok();
        let key_algorithm = (kind == OID_EC_PUBLIC_KEY
            && curve.is_some_and(|c| c.tag == OID && c.content == OID_P256))
        .then_some(EcKey::P256);
        let public_key = match expect(&mut key_info, BIT_STRING, "the public key")?
            .content
            .split_first()
        {
            Some((0, bits)) => bits.to_vec(),
            _ => return Err("the public key is not a whole number of octets".to_owned()),
        };

        let mut parsed = Self {
            tbs: tbs.whole.to_vec(),
            signed_ecdsa_sha256,
            signature,
            issuer,
            subject,
            not_after,
            key_algorithm,
            public_key,
            subject_key_id: None,
            subject_key_id_present: false,
            authority_key_id: None,
            key_usage_present: false,
            server_auth: false,
            client_auth: false,
        };
        while !fields.is_empty() {
            let element = next(&mut fields)?;
            if element.tag == EXTENSIONS {
                let mut wrapped = element.content;
                let mut list = expect(&mut wrapped, SEQUENCE, "the extensions")?.content;
                while !list.is_empty() {
                    parsed.extension(expect(&mut list, SEQUENCE, "an extension")?.content)?;
                }
            }
        }
        Ok(parsed)
    }

    fn extension(&mut self, mut body: &[u8]) -> Result<(), String> {
        let id = expect(&mut body, OID, "an extension's identifier")?.content;
        let mut value = next(&mut body)?;
        if value.tag == BOOLEAN {
            value = next(&mut body)?;
        }
        if value.tag != OCTET_STRING {
            return Err("an extension's value is not an octet string".to_owned());
        }
        let mut inner = value.content;
        if id == OID_SUBJECT_KEY_ID {
            self.subject_key_id_present = true;
            self.subject_key_id = Some(expect(&mut inner, OCTET_STRING, "the subject key identifier")?.content.to_vec());
        } else if id == OID_AUTHORITY_KEY_ID {
            let mut parts = expect(&mut inner, SEQUENCE, "the authority key identifier")?.content;
            while !parts.is_empty() {
                let part = next(&mut parts)?;
                if part.tag == KEY_IDENTIFIER {
                    self.authority_key_id = Some(part.content.to_vec());
                }
            }
        } else if id == OID_KEY_USAGE {
            self.key_usage_present = true;
        } else if id == OID_EXTENDED_KEY_USAGE {
            let mut purposes = expect(&mut inner, SEQUENCE, "the extended key usage")?.content;
            while !purposes.is_empty() {
                let purpose = expect(&mut purposes, OID, "a key purpose")?.content;
                self.server_auth |= purpose == OID_SERVER_AUTH;
                self.client_auth |= purpose == OID_CLIENT_AUTH;
            }
        }
        Ok(())
    }
}

/// The common name in a `Name`, tag and length included, if it carries one.
fn common_name(name: &[u8]) -> Option<String> {
    let mut outer = name;
    let mut sets = expect(&mut outer, SEQUENCE, "a name").ok()?.content;
    while !sets.is_empty() {
        let mut set = expect(&mut sets, SET, "a name's part").ok()?.content;
        while !set.is_empty() {
            let mut pair = expect(&mut set, SEQUENCE, "a name's attribute").ok()?.content;
            let id = expect(&mut pair, OID, "an attribute's type").ok()?.content;
            let value = next(&mut pair).ok()?;
            if id == OID_COMMON_NAME {
                return Some(String::from_utf8_lossy(value.content).into_owned());
            }
        }
    }
    None
}

/// A `UTCTime` or `GeneralizedTime` ending in `Z`, as seconds since the epoch.
fn time(tlv: &Tlv<'_>) -> Result<u64, String> {
    let text = core::str::from_utf8(tlv.content).map_err(|_| "a time is not text".to_owned())?;
    let digits = text
        .strip_suffix('Z')
        .ok_or_else(|| format!("the time {text:?} is not in UTC"))?;
    let full = match (tlv.tag, digits.len()) {
        (UTC_TIME, 12) => {
            let yy: i64 = number(&digits[..2])?;
            format!("{}{}", if yy >= 50 { 1900 + yy } else { 2000 + yy }, &digits[2..])
        }
        (GENERALIZED_TIME, 14) => digits.to_owned(),
        _ => return Err(format!("the time {text:?} has a form this reader does not accept")),
    };
    let year = number(&full[0..4])?;
    let month = number(&full[4..6])?;
    let day = number(&full[6..8])?;
    let hour = number(&full[8..10])?;
    let minute = number(&full[10..12])?;
    let second = number(&full[12..14])?;
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(secs).map_err(|_| format!("the time {text:?} is before the epoch"))
}

fn number(digits: &str) -> Result<i64, String> {
    if digits.bytes().all(|b| b.is_ascii_digit()) {
        digits.parse().map_err(|_| format!("{digits:?} is not a number"))
    } else {
        Err(format!("{digits:?} is not a number"))
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date — Howard Hinnant's
/// `days_from_civil`.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
#[path = "judge_tests.rs"]
mod tests;
