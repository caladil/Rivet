//! What [`crate::judge`] decides about a leaf, over certificates these tests
//! mint themselves (**self-generated**: a counter stands in for
//! `getrandom(2)`, and keys come from `ring`'s generator). Where a fact can be
//! read independently, `x509-parser` and `rustls`'s WebPKI verifier read it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::{LEAF_YEARS, Minted, mint};
use rcgen::{CertificateParams, PKCS_ECDSA_P256_SHA256};
use rivet_core::fleet_root::Request;

fn request(fleet: &str) -> Request {
    Request::new(
        fleet,
        "dev-01",
        &["local".to_owned(), format!("{fleet}.home.arpa")],
    )
    .expect("a legal request")
}

fn counting() -> impl FnMut(usize) -> Result<Vec<u8>, String> {
    let mut next = 7u8;
    move |len| {
        Ok((0..len)
            .map(|_| {
                next = next.wrapping_add(41);
                next
            })
            .collect())
    }
}

fn minted(fleet: &str) -> Minted {
    mint(&request(fleet), &mut counting(), 0).expect("mints")
}

fn root_of(m: &Minted) -> Root {
    Root::from_pem(&m.root.certificate).expect("the root reads")
}

fn issued(m: &Minted, now: u64) -> Pair {
    issue_from_root(&m.root.certificate, &m.root.key, &m.names, &mut counting(), now)
        .expect("issues")
}

/// The first second of a year, for dating a judgement.
fn year_start(year: i64) -> u64 {
    u64::try_from(days_from_civil(year, 1, 1)).expect("after the epoch") * 86_400
}

#[test]
fn the_root_reads_its_own_common_name() {
    assert_eq!(root_of(&minted("dev-fleet")).common_name(), Some("dev-fleet"));
}

/// **A leaf this root issued, with every field, is current** — both the one
/// the first boot mints and one issued later from the resting root.
#[test]
fn a_leaf_this_root_issued_is_current() {
    let m = minted("dev-fleet");
    let root = root_of(&m);
    assert!(matches!(judge(&m.server.certificate, &root, 0), Verdict::Current { .. }));
    let later = issued(&m, 0);
    assert!(matches!(judge(&later.certificate, &root, 0), Verdict::Current { .. }));
}

/// **The leaf issued from the resting root is accepted by the verifier a TLS
/// client runs**, at every name it carries, and carries exactly those names.
#[test]
fn a_leaf_issued_from_the_resting_root_chains_by_webpki() {
    let m = minted("dev-fleet");
    let leaf = issued(&m, 0);
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls_pki_types::CertificateDer::from(m.root.certificate_der.clone()))
        .expect("the root is a certificate");
    let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
        std::sync::Arc::new(roots),
        std::sync::Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .expect("a verifier");
    for name in &m.names {
        use rustls::client::danger::ServerCertVerifier as _;
        verifier
            .verify_server_cert(
                &rustls_pki_types::CertificateDer::from(leaf.certificate_der.clone()),
                &[],
                &rustls_pki_types::ServerName::try_from(name.clone()).expect("a name"),
                &[],
                rustls_pki_types::UnixTime::now(),
            )
            .unwrap_or_else(|e| panic!("{name} refused: {e}"));
    }
    assert_ne!(leaf.key, m.server.key, "a new leaf has a new key");
}

/// **A leaf another fleet's root signed is foreign, and names its issuer** —
/// what is journalled when a workstation's certificate is replaced.
#[test]
fn a_leaf_from_another_root_is_foreign_and_names_its_issuer() {
    let ours = minted("dev-fleet");
    let theirs = minted("workstation");
    assert_eq!(
        judge(&theirs.server.certificate, &root_of(&ours), 0),
        Verdict::Foreign {
            issuer: "CN=workstation".to_owned()
        }
    );
}

/// **The same name with another key is still foreign**: the signature is what
/// is checked, not the issuer's name.
#[test]
fn a_root_with_this_name_and_another_key_is_foreign() {
    let ours = minted("dev-fleet");
    let impostor = minted("dev-fleet");
    assert!(matches!(
        judge(&impostor.server.certificate, &root_of(&ours), 0),
        Verdict::Foreign { .. }
    ));
}

/// **Ninety days before its end a leaf is re-issued, and not a day earlier.**
/// Its end is read independently by `x509-parser` to pin what this reader
/// made of the date.
#[test]
fn a_leaf_within_ninety_days_of_its_end_is_near_expiry() {
    let m = minted("dev-fleet");
    let root = root_of(&m);
    let leaf = issued(&m, 0);
    let end = year_start(i64::from(FLOOR_YEAR + LEAF_YEARS));
    let (_, read) = x509_parser::parse_x509_certificate(&leaf.certificate_der).expect("reads");
    assert_eq!(u64::try_from(read.validity().not_after.timestamp()).expect("positive"), end);

    let ninety = 90 * 86_400;
    assert!(matches!(
        judge(&leaf.certificate, &root, end - ninety - 1),
        Verdict::Current { .. }
    ));
    assert_eq!(
        judge(&leaf.certificate, &root, end - ninety),
        Verdict::NearExpiry { not_after: end }
    );
    assert_eq!(
        judge(&leaf.certificate, &root, end + 1),
        Verdict::NearExpiry { not_after: end },
        "an ended leaf is re-issued too"
    );
}

/// **A clock at the epoch is judged as the floor**, as every first boot's is:
/// a leaf issued then is current then.
#[test]
fn a_clock_at_the_epoch_is_judged_at_the_floor() {
    let m = minted("dev-fleet");
    assert!(matches!(
        judge(&issued(&m, 0).certificate, &root_of(&m), 0),
        Verdict::Current { .. }
    ));
}

/// **A leaf this root signed without the standard fields lacks them** — the
/// shape the first boot minted before every leaf carried them. Signed here by
/// `rcgen` with its defaults, which write none of the four.
#[test]
fn a_leaf_without_the_standard_fields_lacks_them() {
    let m = minted("dev-fleet");
    let key = KeyPair::from_pem(&m.root.key).expect("the key reads");
    let issuer = Issuer::from_ca_cert_pem(&m.root.certificate, key).expect("an issuer");
    let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("a key");
    let mut params = CertificateParams::new(m.names.clone()).expect("params");
    params.not_before = rcgen::date_time_ymd(FLOOR_YEAR, 1, 1);
    params.not_after = rcgen::date_time_ymd(FLOOR_YEAR + LEAF_YEARS, 1, 1);
    let bare = params.signed_by(&leaf_key, &issuer).expect("signs");

    assert_eq!(
        judge(&bare.pem(), &root_of(&m), 0),
        Verdict::LacksFields {
            missing: vec![
                "an authority key identifier",
                "a subject key identifier",
                "a key usage",
                "serverAuth in its extended key usage",
                "clientAuth in its extended key usage",
            ]
        }
    );
}

#[test]
fn what_is_not_a_certificate_is_unreadable() {
    let root = root_of(&minted("dev-fleet"));
    assert!(matches!(judge("not a certificate", &root, 0), Verdict::Unreadable(_)));
    let truncated = "-----BEGIN CERTIFICATE-----\nMIIBejCCASCgAwIBAgIU\n-----END CERTIFICATE-----\n";
    assert!(matches!(judge(truncated, &root, 0), Verdict::Unreadable(_)));
}

/// **A key that is not the root's is refused before anything is signed**, so a
/// mismatched pair on the disk cannot produce a leaf that will not chain.
#[test]
fn issuing_refuses_a_key_that_is_not_the_roots() {
    let m = minted("dev-fleet");
    let other = minted("dev-fleet");
    match issue_from_root(&m.root.certificate, &other.root.key, &m.names, &mut counting(), 0) {
        Err(Error::Mint(why)) => assert!(why.contains("not the key"), "{why}"),
        Err(other) => panic!("refused for the wrong reason: {other}"),
        Ok(_) => panic!("a leaf was issued with another root's key"),
    }
}

/// **A random source that refuses issues nothing**, as `ADR-063`'s first boot
/// refuses.
#[test]
fn issuing_refuses_without_randomness() {
    let m = minted("dev-fleet");
    let mut refusing = |_len: usize| -> Result<Vec<u8>, String> { Err("none".to_owned()) };
    assert!(matches!(
        issue_from_root(&m.root.certificate, &m.root.key, &m.names, &mut refusing, 0),
        Err(Error::Randomness(_))
    ));
}

/// Every verdict but `Current` asks for a new leaf.
#[test]
fn only_a_current_leaf_is_kept() {
    assert!(!Verdict::Current { not_after: 1 }.wants_issuance());
    for verdict in [
        Verdict::Missing(String::new()),
        Verdict::Unreadable(String::new()),
        Verdict::Foreign {
            issuer: String::new(),
        },
        Verdict::NearExpiry { not_after: 1 },
        Verdict::LacksFields { missing: vec![] },
    ] {
        assert!(verdict.wants_issuance(), "{verdict:?}");
    }
}

#[test]
fn days_from_civil_agrees_with_known_dates() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    // 2026-01-01T00:00:00Z is 1767225600, as `date -u -d 2026-01-01 +%s` says.
    assert_eq!(year_start(2026), 1_767_225_600);
    assert_eq!(floor_secs(), 1_767_225_600);
}
