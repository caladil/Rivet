//! **Which certificates belong to the authority being reset — decided by its
//! KEY, and never by its name.**
//!
//! # The incident this module is the answer to
//!
//! On 2026-09-07 a new authority was minted carrying the **same subject name**
//! as the one it replaced: both `CN=home-fleet`. A browser selects a client
//! certificate by matching the issuer **name**, so every certificate either
//! authority had ever issued became eligible at once — the maintainer was
//! offered four, none of which could authenticate
//! (`Fleet-Bring-Up-Findings.md` §3.1).
//!
//! **So a reset that matched on the issuer's NAME would be wrong in both
//! directions at once.** It would delete certificates belonging to an authority
//! the operator meant to keep, and — for the same reason — it could never tell
//! the operator which of two identically-named roots it was about to remove.
//!
//! # What is matched instead
//!
//! **The signature.** A certificate belongs to this authority if and only if
//! this authority's public key verifies it. That is not a heuristic and there is
//! nothing to tune: two authorities that share a subject name do not share a
//! key, and no certificate signed by one verifies under the other.
//!
//! The authority key identifier (`ADR`-free X.509 §4.2.1.1) would be the cheap
//! version of the same idea and is deliberately **not** used as the test: it is
//! a hint copied out of the issuer at minting time, it is optional, and anything
//! that can write a certificate can write any value into it. It is reported as
//! corroboration and it decides nothing.
//!
//! # The third answer, which is the one that makes a report useful
//!
//! A certificate that carries this authority's subject name and does **not**
//! verify under its key is neither ours nor unrelated — it is the other
//! `home-fleet`, and saying so by name is what turns *"you have four
//! certificates"* into *"three of these are from an authority whose key is not
//! here"*. [`Verdict::SameNameOtherKey`] is that answer, and it is never
//! removed by default.

use x509_parser::prelude::*;

/// What one certificate in a store is, with respect to the authority being
/// reset.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Verdict {
    /// **This is the authority's own root**, byte for byte.
    TheRoot,
    /// **Signed by the authority's key**, which is the whole test.
    Issued,
    /// Carries the authority's subject name and was **not** signed by its key.
    ///
    /// A different authority wearing the same name — the 2026-09-07 state. Left
    /// alone unless the operator asks for retired authorities too, and named in
    /// the report either way.
    SameNameOtherKey,
    /// Nothing to do with this authority. The Apple device certificate and the
    /// `trust_<guid>` entries that live in every operator's Personal store are
    /// this, and they are why the answer has to be *unrelated* rather than
    /// *not obviously ours*.
    Unrelated,
    /// **The certificate could not be read, so no claim is made about it.**
    ///
    /// `AGENTS.md` §7: a probe that cannot tell an error from a true negative
    /// must report which one it had. An unreadable certificate is reported and
    /// left in place.
    Unreadable(String),
}

/// Rule on one certificate.
///
/// `candidate` and `root` are both DER.
#[must_use]
pub fn verdict(candidate: &[u8], root: &[u8]) -> Verdict {
    let (Ok((_, mine)), Ok((_, theirs))) = (
        X509Certificate::from_der(root),
        X509Certificate::from_der(candidate),
    ) else {
        return Verdict::Unreadable(
            "this certificate is not one x509-parser can read".to_owned(),
        );
    };

    // **The root is recognised by its bytes, not by its signature.** A
    // self-signed root does verify under its own key, so the signature test
    // below would also catch it — but a reset that reported the authority's own
    // root as *"something it issued"* would be telling the operator something
    // subtly untrue about the most important line in the list.
    if candidate == root {
        return Verdict::TheRoot;
    }

    // **The test.** Everything else in this module is presentation.
    if theirs.verify_signature(Some(mine.public_key())).is_ok() {
        return Verdict::Issued;
    }

    // Not ours. Is it the other authority of the same name?
    if theirs.issuer().as_raw() == mine.subject().as_raw() {
        return Verdict::SameNameOtherKey;
    }
    // And the retired ROOT itself carries the name in its subject rather than
    // its issuer, because it signed itself.
    if theirs.subject().as_raw() == mine.subject().as_raw() {
        return Verdict::SameNameOtherKey;
    }
    Verdict::Unrelated
}

/// Whether `candidate` is a certificate authority that signed itself.
///
/// **Used only to describe a [`Verdict::SameNameOtherKey`]** — a retired root
/// and a leaf that retired root issued are both "the other `home-fleet`", and
/// an operator deciding whether to clear them out is better served by being
/// told which is which.
#[must_use]
pub fn is_self_signed_authority(candidate: &[u8]) -> bool {
    let Ok((_, cert)) = X509Certificate::from_der(candidate) else {
        return false;
    };
    cert.is_ca()
        && cert.issuer().as_raw() == cert.subject().as_raw()
        && cert.verify_signature(None).is_ok()
}

/// Whether `candidate` was signed by `issuer_der`'s key.
///
/// The same test [`verdict`] makes, exposed for the retired-authority sweep:
/// once a retired root has been identified, its leaves are found by asking this
/// about every certificate in every store.
#[must_use]
pub fn signed_by(candidate: &[u8], issuer_der: &[u8]) -> bool {
    let (Ok((_, issuer)), Ok((_, cert))) = (
        X509Certificate::from_der(issuer_der),
        X509Certificate::from_der(candidate),
    ) else {
        return false;
    };
    cert.verify_signature(Some(issuer.public_key())).is_ok()
}

/// A certificate's subject, rendered the way a person reads it.
#[must_use]
pub fn subject_of(der: &[u8]) -> String {
    X509Certificate::from_der(der)
        .map_or_else(|_| "(unreadable)".to_owned(), |(_, c)| c.subject().to_string())
}

/// A certificate's issuer, rendered the way a person reads it.
#[must_use]
pub fn issuer_of(der: &[u8]) -> String {
    X509Certificate::from_der(der)
        .map_or_else(|_| "(unreadable)".to_owned(), |(_, c)| c.issuer().to_string())
}

/// The DER behind a PEM certificate.
///
/// # Errors
///
/// A sentence naming what was expected, with the path left to the caller —
/// which is the one that knows it.
pub fn der_of_pem(pem: &str) -> Result<Vec<u8>, String> {
    let (_, block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|e| format!("this is not a PEM certificate: {e}"))?;
    // Parsed as well as decoded: a file that base64-decodes to something that is
    // not a certificate must fail HERE, where the message can name the file,
    // rather than in a matcher that would quietly rule every store certificate
    // unrelated and report a clean store.
    X509Certificate::from_der(&block.contents)
        .map_err(|e| format!("this PEM does not hold a certificate: {e}"))?;
    Ok(block.contents)
}

/// The DER behind a certificate FILE, whichever of the two shapes it is in.
///
/// **PEM first, then DER**, because that is the order the two are met: every
/// certificate this project writes is PEM, and a certificate exported from the
/// Windows store by a person clicking through the wizard is usually DER with a
/// `.cer` name. Guessing from the extension would be a third fact to get wrong.
///
/// # Errors
///
/// A sentence saying it was tried as both, so a file that is neither is not
/// reported as *"bad PEM"* to somebody holding a `.cer`.
pub fn der_of_file(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if let Ok(text) = std::str::from_utf8(bytes)
        && text.contains("-----BEGIN")
    {
        return der_of_pem(text);
    }
    X509Certificate::from_der(bytes)
        .map_err(|e| format!("this is neither a PEM nor a DER certificate: {e}"))?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
        KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
    };

    /// An authority and its DER, minted the way `rivet-provision` mints one.
    struct Fixture {
        params: CertificateParams,
        key: KeyPair,
        der: Vec<u8>,
        /// **The same certificate**, and not a second minting of the same
        /// parameters: `self_signed` called twice is two certificates, and a
        /// test that compared one with the other would be asserting something
        /// this module does not claim.
        pem: String,
    }

    fn named(common: &str) -> DistinguishedName {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common);
        dn
    }

    fn authority(name: &str) -> Fixture {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("a key");
        let mut params = CertificateParams::default();
        params.distinguished_name = named(name);
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let root = params.clone().self_signed(&key).expect("a root");
        let der = root.der().to_vec();
        let pem = root.pem();
        Fixture {
            params,
            key,
            der,
            pem,
        }
    }

    fn leaf(from: &Fixture, common: &str) -> Vec<u8> {
        let issuer = Issuer::new(from.params.clone(), &from.key);
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("a key");
        let mut params = CertificateParams::new(Vec::new()).expect("params");
        params.distinguished_name = named(common);
        params.is_ca = IsCa::NoCa;
        params
            .signed_by(&key, &issuer)
            .expect("a leaf")
            .der()
            .to_vec()
    }

    /// **THE TEST THIS WHOLE MODULE EXISTS FOR.**
    ///
    /// Two authorities, one name. The exact 2026-09-07 state, and the exact
    /// reason matching on the issuer's name is not sufficient: a matcher that
    /// compared names would rule all four of these certificates ours and delete
    /// the ones belonging to the authority the operator is keeping.
    #[test]
    fn two_authorities_with_one_name_are_told_apart_by_their_keys() {
        let mine = authority("home-fleet");
        let theirs = authority("home-fleet");
        let my_leaf = leaf(&mine, "Your Name");
        let their_leaf = leaf(&theirs, "Your Name");

        assert_eq!(verdict(&mine.der, &mine.der), Verdict::TheRoot);
        assert_eq!(verdict(&my_leaf, &mine.der), Verdict::Issued);

        assert_eq!(
            verdict(&theirs.der, &mine.der),
            Verdict::SameNameOtherKey,
            "the OTHER authority's root was claimed by name"
        );
        assert_eq!(
            verdict(&their_leaf, &mine.der),
            Verdict::SameNameOtherKey,
            "a leaf of the other authority of the same name was claimed by name"
        );

        // And the same, seen from the other side: the verdicts are symmetric,
        // so neither authority can be the privileged one by accident.
        assert_eq!(verdict(&their_leaf, &theirs.der), Verdict::Issued);
        assert_eq!(verdict(&my_leaf, &theirs.der), Verdict::SameNameOtherKey);
    }

    /// **A certificate that is nobody's business is left alone.**
    ///
    /// The operator's real Personal store holds an Apple iPhone device
    /// certificate and several `trust_<guid>` entries belonging to other
    /// software. Deleting one of those is unrecoverable, so *unrelated* has to
    /// be a verdict this function reaches rather than a case it falls into.
    #[test]
    fn somebody_elses_certificate_is_unrelated() {
        let mine = authority("home-fleet");
        let apple = authority("Apple iPhone Device CA");
        let device = leaf(&apple, "76D8BA47-AEBB-4E36-B7A6-8E28BF2AFA9D");
        let self_signed = authority("trust_df3eb374-0b5f-4c1f-9479-d97b3f9227a7");

        assert_eq!(verdict(&device, &mine.der), Verdict::Unrelated);
        assert_eq!(verdict(&apple.der, &mine.der), Verdict::Unrelated);
        assert_eq!(verdict(&self_signed.der, &mine.der), Verdict::Unrelated);
    }

    /// A retired authority is recognisable as an authority, and a leaf is not —
    /// which is what lets the report say *"a retired root"* rather than
    /// *"something with the same name"*.
    #[test]
    fn a_root_is_told_from_a_leaf() {
        let a = authority("home-fleet");
        assert!(is_self_signed_authority(&a.der));
        assert!(!is_self_signed_authority(&leaf(&a, "Your Name")));
    }

    /// The sweep that clears a retired authority's leaves asks about the key,
    /// exactly as the ordinary path does.
    #[test]
    fn the_retired_sweep_matches_on_the_key_too() {
        let retired = authority("home-fleet");
        let current = authority("home-fleet");
        let old = leaf(&retired, "Your Name");
        let new = leaf(&current, "Your Name");

        assert!(signed_by(&old, &retired.der));
        assert!(!signed_by(&new, &retired.der));
        assert!(!signed_by(&retired.der, &current.der));
    }

    /// **Rubbish is reported as rubbish, not as an answer.** A store that hands
    /// back something unparseable must not be silently ruled unrelated, because
    /// *unrelated* is the verdict that leaves a certificate in place while
    /// claiming the store was examined.
    #[test]
    fn an_unreadable_certificate_says_so() {
        let mine = authority("home-fleet");
        assert!(matches!(
            verdict(&[0x30, 0x00], &mine.der),
            Verdict::Unreadable(_)
        ));
        assert!(matches!(
            verdict(&mine.der, &[]),
            Verdict::Unreadable(_)
        ));
    }

    /// PEM in, DER out — and a file that is not a certificate is refused with a
    /// sentence rather than accepted as an empty authority.
    #[test]
    fn a_root_is_read_out_of_its_pem() {
        let a = authority("home-fleet");
        assert_eq!(der_of_pem(&a.pem).expect("reads"), a.der);
        assert!(der_of_pem("not a certificate at all").is_err());
        assert!(
            der_of_pem("-----BEGIN CERTIFICATE-----\nZm9v\n-----END CERTIFICATE-----\n").is_err(),
            "a PEM block that decodes to something other than a certificate was accepted"
        );
    }

    /// **Both shapes a certificate arrives in.** A file exported from the
    /// Windows store by hand is DER with a `.cer` name; everything this project
    /// writes is PEM. `--verdict` takes either, so the operator asking *"would
    /// this one go?"* does not first have to work out which they are holding.
    #[test]
    fn a_certificate_file_is_read_as_pem_or_as_der() {
        let a = authority("home-fleet");
        assert_eq!(der_of_file(a.pem.as_bytes()).expect("PEM"), a.der);
        assert_eq!(der_of_file(&a.der).expect("DER"), a.der);
        assert!(der_of_file(b"neither of those things").is_err());
    }
}
