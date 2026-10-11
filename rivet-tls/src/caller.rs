//! **Whose certificate a verified client presented: the operator's, or a
//! machine's.** `.rulings/` R-187 D1.
//!
//! # Why the question has to be asked of the certificate
//!
//! A fleet authority signs two kinds of leaf. Every fleet machine's front door
//! gets one for its own names (`cmd/rivet-provision`'s `issue_from`), and the
//! operator's browser gets one for a person (`Authority::operator`). A verifier
//! over that authority admits both, so "the chain verified" says the caller
//! holds a key the fleet signed and says nothing about which of the two it is.
//! `rivet-fleetd` and `rivet-proxyd` each need the other half of the answer
//! before serving an operator route, and this is the one copy of it.
//!
//! **The rule R-187 D1 settled: the operator is a client certificate that names
//! no host.** A machine's leaf exists to be presented to a browser under the
//! names it answers to, so it always carries them in `subjectAltName`; the
//! operator's identifies a person and carries none. No certificate is reissued
//! for this, and an explicit operator marker follows when browser certificates
//! are next minted (`ADR-052` §10).
//!
//! # What counts as naming a host
//!
//! **Any `subjectAltName` extension at all.** DNS names and IP addresses are
//! what a machine's leaf carries; a URI, an email address, an `otherName` or a
//! directory name is none of those and is still not the operator's shape, so it
//! is not taken for one. The rule is the presence of the extension, not a list
//! of entry kinds, so a kind this code does not know cannot fall through to
//! "operator".
//!
//! **The subject's `commonName` does not count, in either direction.** A machine
//! leaf's subject happens to be its first name and the operator's is whatever a
//! person typed, `Your Name` or `fleet-owner` or, just as legitimately, a
//! dotted name. Nothing on this machine authenticates by the subject
//! (`ADR-028` §4), no verifier in the tree matches a host against it, and
//! [`crate::subject_common_name`] is documented as a label no decision may rest
//! on. Refusing a dotted `commonName` would lock out an operator whose certificate
//! already exists, to guard against a machine leaf with no `subjectAltName` —
//! which no browser or rustls client would accept as that machine anyway, and
//! which `issue_from` refuses to mint.
//!
//! # Conservative when it cannot read
//!
//! **A certificate this cannot walk to its extensions is never the operator.**
//! The walk is [`crate::public_key_in_certificate`]'s: every field is found by
//! position and length, and every shape it does not expect is a refusal rather
//! than a guess. That covers truncation, trailing bytes, a missing `version`
//! (only v3 carries extensions, and `rustls-webpki` admits only v3), unexpected
//! fields after the key, a malformed extension, two `subjectAltName`s, and a
//! `subjectAltName` whose list is empty or holds a tag RFC 5280 does not define.
//!
//! # Why this walks rather than calls a parser
//!
//! The verified chain was already parsed, by `rustls-webpki`, in the handshake.
//! What is left is one extension's presence, and this crate already walks a
//! `TBSCertificate` by field lengths for the key and the subject.
//! `rustls-webpki` exposes a certificate's DNS names but not its IP addresses,
//! so it cannot answer the whole question; `x509-parser` can, and would put
//! `time`, `nom` and `asn1-rs` into every card binary that links this crate,
//! which this crate's own header argues against. The tests hold this walk
//! against rcgen's encodings of every entry kind.

use super::element;

/// What a client certificate that already verified against the fleet authority
/// says about who holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// **The operator**: a leaf that names no host.
    Operator,
    /// **A machine**: a leaf whose `subjectAltName` names something, listed as
    /// the entries read — a DNS name as itself, an IP address in its usual
    /// notation, anything else as its kind.
    Machine(Vec<String>),
    /// A certificate that could not be read far enough to tell. **Never the
    /// operator.**
    Unreadable,
}

impl Caller {
    /// Whether this caller may use an operator route.
    #[must_use]
    pub fn is_operator(&self) -> bool {
        matches!(self, Self::Operator)
    }

    /// Why this caller is not the operator, in words for a refusal and a log
    /// line — or `None` for the operator.
    #[must_use]
    pub fn refusal(&self) -> Option<String> {
        match self {
            Self::Operator => None,
            Self::Machine(names) => Some(format!(
                "this client certificate names a host ({}), so it belongs to a machine and not \
                 to the operator. Operator routes accept only a certificate that names no host \
                 (.rulings/ R-187): present the operator's browser certificate",
                names.join(", ")
            )),
            Self::Unreadable => Some(
                "this client certificate could not be read far enough to tell whether it names \
                 a host, so it is not taken for the operator's (.rulings/ R-187)"
                    .to_owned(),
            ),
        }
    }
}

/// `2.5.29.17`, `id-ce-subjectAltName`, as DER content bytes.
const SUBJECT_ALT_NAME: [u8; 3] = [0x55, 0x1d, 0x11];

/// **Classify a verified client certificate's leaf.** See the module
/// documentation for the rule and for why the subject is not consulted.
#[must_use]
pub fn classify_caller(leaf: &[u8]) -> Caller {
    match subject_alt_names(leaf) {
        Some(None) => Caller::Operator,
        Some(Some(names)) => Caller::Machine(names.iter().map(|&(tag, bytes)| render(tag, bytes)).collect()),
        None => Caller::Unreadable,
    }
}

/// **The DNS names a certificate's `subjectAltName` carries, in order** —
/// `None` for a certificate this walk refuses, an empty list for one with no
/// DNS entry or no `subjectAltName` at all.
///
/// For a machine reading its OWN certificate, which is why nothing here is an
/// authentication decision: `rivet-proxyd` takes the first entry as the name
/// the applications behind it are served under when no setting names one
/// (`ADR-060`). The walk is [`classify_caller`]'s, so a shape one refuses the
/// other refuses too.
#[must_use]
pub fn dns_names(der: &[u8]) -> Option<Vec<String>> {
    /// `[2] IMPLICIT IA5String`, `dNSName`.
    const DNS_NAME: u8 = 0x82;
    let entries = subject_alt_names(der)?.unwrap_or_default();
    Some(
        entries
            .into_iter()
            .filter(|&(tag, _)| tag == DNS_NAME)
            .filter_map(|(_, bytes)| core::str::from_utf8(bytes).ok().map(str::to_owned))
            .collect(),
    )
}

/// One `GeneralName`: its tag and its content octets.
type Entry<'a> = (u8, &'a [u8]);

/// The leaf's `subjectAltName` entries: `Some(None)` for a certificate with no
/// such extension, `Some(Some(entries))` for one with it, `None` for anything
/// this walk refuses.
fn subject_alt_names(der: &[u8]) -> Option<Option<Vec<Entry<'_>>>> {
    const SEQUENCE: u8 = 0x30;
    const INTEGER: u8 = 0x02;
    const BOOLEAN: u8 = 0x01;
    const OID: u8 = 0x06;
    const OCTET_STRING: u8 = 0x04;
    /// `[0] EXPLICIT Version`.
    const VERSION: u8 = 0xa0;
    /// `[1] IMPLICIT issuerUniqueID` and `[2] IMPLICIT subjectUniqueID`, in
    /// either encoding a writer might choose.
    const UNIQUE_IDS: [u8; 4] = [0x81, 0xa1, 0x82, 0xa2];
    /// `[3] EXPLICIT Extensions`.
    const EXTENSIONS: u8 = 0xa3;

    let certificate = element(der)?;
    if certificate.tag != SEQUENCE || !certificate.rest.is_empty() {
        return None;
    }
    let tbs = element(certificate.content)?;
    if tbs.tag != SEQUENCE {
        return None;
    }

    // **Version 3, or nothing.** Extensions exist only in v3, so a certificate
    // without the explicit version cannot carry a `subjectAltName` — and reading
    // that absence as "names no host" would admit a shape the handshake's own
    // verifier refuses. Refused instead.
    let version = element(tbs.content)?;
    if version.tag != VERSION || !element(version.content)?.rest.is_empty() {
        return None;
    }
    let number = element(version.content)?;
    if number.tag != INTEGER || number.content != [0x02] {
        return None;
    }

    // `serialNumber`, `signature`, `issuer`, `validity`, `subject`,
    // `subjectPublicKeyInfo`: six fields in the order RFC 5280 §4.1 fixes, each
    // stepped over by its length and never looked inside.
    let mut rest = version.rest;
    for _ in 0..6 {
        rest = element(rest)?.rest;
    }
    while let Some(field) = element(rest) {
        if !UNIQUE_IDS.contains(&field.tag) {
            break;
        }
        rest = field.rest;
    }
    if rest.is_empty() {
        return Some(None);
    }

    let wrapper = element(rest)?;
    if wrapper.tag != EXTENSIONS || !wrapper.rest.is_empty() {
        return None;
    }
    let list = element(wrapper.content)?;
    if list.tag != SEQUENCE || !list.rest.is_empty() {
        return None;
    }

    let mut found: Option<Vec<(u8, &[u8])>> = None;
    let mut extensions = list.content;
    while !extensions.is_empty() {
        let extension = element(extensions)?;
        extensions = extension.rest;
        if extension.tag != SEQUENCE {
            return None;
        }
        let oid = element(extension.content)?;
        if oid.tag != OID {
            return None;
        }
        let mut after = element(oid.rest)?;
        if after.tag == BOOLEAN {
            after = element(after.rest)?;
        }
        if after.tag != OCTET_STRING || !after.rest.is_empty() {
            return None;
        }
        if oid.content != SUBJECT_ALT_NAME {
            continue;
        }
        // Two of one extension is forbidden by RFC 5280 §4.2 and refused by
        // the handshake's verifier; which one a reader believed would be a
        // guess.
        if found.is_some() {
            return None;
        }
        found = Some(general_names(after.content)?);
    }
    Some(found)
}

/// A `GeneralNames` sequence, each entry as its tag and content octets. `None`
/// for an empty list, a tag RFC 5280 §4.2.1.6 does not define, or anything that
/// does not walk.
fn general_names(der: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    const SEQUENCE: u8 = 0x30;
    /// Every tag `GeneralName` defines, and no other.
    const DEFINED: [u8; 9] = [0xa0, 0x81, 0x82, 0xa3, 0xa4, 0xa5, 0x86, 0x87, 0x88];

    let names = element(der)?;
    if names.tag != SEQUENCE || !names.rest.is_empty() || names.content.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut rest = names.content;
    while !rest.is_empty() {
        let name = element(rest)?;
        rest = name.rest;
        if !DEFINED.contains(&name.tag) {
            return None;
        }
        out.push((name.tag, name.content));
    }
    Some(out)
}

/// One `GeneralName` entry rendered for a person.
fn render(tag: u8, content: &[u8]) -> String {
    match tag {
        0xa0 => "an otherName entry".to_owned(),
        0x81 => format!("email {}", String::from_utf8_lossy(content)),
        0x82 => String::from_utf8_lossy(content).into_owned(),
        0xa3 => "an x400Address entry".to_owned(),
        0xa4 => "a directoryName entry".to_owned(),
        0xa5 => "an ediPartyName entry".to_owned(),
        0x86 => format!("URI {}", String::from_utf8_lossy(content)),
        0x87 => address(content),
        _ => "a registeredID entry".to_owned(),
    }
}

/// An `iPAddress` entry in its usual notation. A length that is neither four
/// nor sixteen is still an address entry, and is still not the operator's.
fn address(bytes: &[u8]) -> String {
    if let Ok(v4) = <[u8; 4]>::try_from(bytes) {
        return std::net::Ipv4Addr::from(v4).to_string();
    }
    if let Ok(v6) = <[u8; 16]>::try_from(bytes) {
        return std::net::Ipv6Addr::from(v6).to_string();
    }
    format!("an IP address entry of {} bytes", bytes.len())
}

#[cfg(test)]
#[path = "caller_tests.rs"]
mod tests;
