//! What [`crate::mint`] guarantees, judged by the verifier a browser's chain
//! check resembles (`rustls`'s WebPKI) and read back by a parser independent
//! of the one that wrote the certificates (`x509-parser`).
//!
//! The randomness in these tests is **self-generated**: a counter the tests
//! own, standing in for `getrandom(2)`. Keys come from `ring`'s generator
//! regardless, as they do on a board.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use rivet_core::fleet_root::Request;

fn request(suffixes: &[&str]) -> Request {
    Request::new(
        "dev-fleet",
        "dev-01",
        &suffixes.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
    )
    .expect("a legal request")
}

/// A source that answers with distinct bytes, as a generator does.
fn counting() -> impl FnMut(usize) -> Result<Vec<u8>, String> {
    let mut next = 1u8;
    move |len| {
        Ok((0..len)
            .map(|_| {
                next = next.wrapping_add(37);
                next
            })
            .collect())
    }
}

fn mint_three() -> Minted {
    let mut random = counting();
    mint(
        &request(&["local", "dev-fleet.home.arpa", "dev-fleet.lan"]),
        &mut random,
        0,
    )
    .expect("mints")
}

fn server_verifier(root_der: &[u8]) -> std::sync::Arc<rustls::client::WebPkiServerVerifier> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls_pki_types::CertificateDer::from(root_der.to_vec()))
        .expect("the root is a certificate");
    rustls::client::WebPkiServerVerifier::builder_with_provider(
        std::sync::Arc::new(roots),
        std::sync::Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .expect("a verifier over one root")
}

/// Whether a server presenting `leaf` is accepted at `name`, now.
fn accepted(root_der: &[u8], leaf_der: &[u8], name: &str) -> Result<(), String> {
    use rustls::client::danger::ServerCertVerifier as _;
    let server_name = rustls_pki_types::ServerName::try_from(name.to_owned()).expect("a name");
    server_verifier(root_der)
        .verify_server_cert(
            &rustls_pki_types::CertificateDer::from(leaf_der.to_vec()),
            &[],
            &server_name,
            &[],
            rustls_pki_types::UnixTime::now(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn permitted(der: &[u8]) -> Vec<String> {
    let (_, certificate) = x509_parser::parse_x509_certificate(der).expect("a certificate");
    let constraints = certificate
        .name_constraints()
        .expect("readable")
        .expect("the root carries a name constraint");
    constraints
        .value
        .permitted_subtrees
        .iter()
        .flatten()
        .map(|subtree| match subtree.base {
            x509_parser::extensions::GeneralName::DNSName(d) => d.to_owned(),
            ref other => format!("{other:?}"),
        })
        .collect()
}

fn alternative_names(der: &[u8]) -> Vec<String> {
    let (_, certificate) = x509_parser::parse_x509_certificate(der).expect("a certificate");
    certificate
        .subject_alternative_name()
        .expect("readable")
        .expect("the server carries names")
        .value
        .general_names
        .iter()
        .map(|name| match name {
            x509_parser::extensions::GeneralName::DNSName(d) => (*d).to_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// **The constraint is exactly the fleet's names**, as the request gave them
/// and in its order — no more, and in particular not an empty one.
#[test]
fn the_roots_constraint_is_exactly_the_requested_suffixes() {
    let minted = mint_three();
    assert_eq!(
        permitted(&minted.root.certificate_der),
        vec!["local", "dev-fleet.home.arpa", "dev-fleet.lan"]
    );

    let mut random = counting();
    let two = mint(&request(&["local", "dev-fleet.home.arpa"]), &mut random, 0).expect("mints");
    assert_eq!(permitted(&two.root.certificate_der), vec!["local", "dev-fleet.home.arpa"]);
}

/// **The server's certificate chains to the root at every name it carries**,
/// by the verifier a TLS client runs, and carries exactly those names.
#[test]
fn the_server_chains_to_the_root_and_carries_every_name() {
    let minted = mint_three();
    assert_eq!(
        minted.names,
        vec!["dev-01.local", "dev-01.dev-fleet.home.arpa", "dev-01.dev-fleet.lan"]
    );
    assert_eq!(alternative_names(&minted.server.certificate_der), minted.names);
    for name in &minted.names {
        accepted(&minted.root.certificate_der, &minted.server.certificate_der, name)
            .unwrap_or_else(|why| panic!("{name} was refused under its own root: {why}"));
    }
    // And not at a name it does not carry.
    assert!(
        accepted(&minted.root.certificate_der, &minted.server.certificate_der, "dev-02.local")
            .is_err()
    );
}

/// **A leaked root forges only this fleet's names.** A certificate the root's
/// own key signs for a public name is refused by the constraint, which is the
/// whole reason the constraint exists.
#[test]
fn the_root_cannot_vouch_for_a_name_outside_the_fleet() {
    let minted = mint_three();
    let key = KeyPair::from_pem(&minted.root.key).expect("the root key reads back");
    let issuer = Issuer::from_ca_cert_pem(&minted.root.certificate, key).expect("an issuer");
    let forged_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("a key");
    for outside in ["bank.example.com", "dev-01.their-fleet.home.arpa", "dev-01.lan"] {
        let mut params = CertificateParams::new(vec![outside.to_owned()]).expect("params");
        params.not_before = rcgen::date_time_ymd(FLOOR_YEAR, 1, 1);
        params.not_after = rcgen::date_time_ymd(FLOOR_YEAR + 1, 1, 1);
        let forged = params.signed_by(&forged_key, &issuer).expect("signs");
        assert!(
            accepted(&minted.root.certificate_der, forged.der(), outside).is_err(),
            "the root vouched for {outside}, outside its constraint"
        );
    }
}

/// **A random source that refuses mints nothing** — the default of every
/// port that has no generator behind it.
#[test]
fn minting_refuses_without_randomness() {
    let mut refusing = |len: usize| -> Result<Vec<u8>, String> {
        Err(format!("no random source; {len} bytes could not be drawn"))
    };
    match mint(&request(&["local"]), &mut refusing, 0) {
        Err(Error::Randomness(why)) => assert!(why.contains("no random source"), "{why}"),
        Err(other) => panic!("refused for the wrong reason: {other}"),
        Ok(_) => panic!("a root was minted with no random source"),
    }
}

/// **A stub filling a buffer is refused as a generator would never answer.**
#[test]
fn a_source_answering_one_repeated_byte_is_refused() {
    for byte in [0x00u8, 0xff, 0x42] {
        let mut stub = move |len: usize| -> Result<Vec<u8>, String> { Ok(vec![byte; len]) };
        assert!(
            matches!(mint(&request(&["local"]), &mut stub, 0), Err(Error::Randomness(_))),
            "{byte:#04x}"
        );
    }
    let mut short = |_len: usize| -> Result<Vec<u8>, String> { Ok(vec![1, 2, 3]) };
    assert!(matches!(mint(&request(&["local"]), &mut short, 0), Err(Error::Randomness(_))));
}

/// **At the epoch the certificates are valid from the floor and valid now**,
/// which is the first boot of every Raspberry Pi.
#[test]
fn a_mint_at_the_epoch_is_valid_today_and_not_expired() {
    let minted = mint_three();
    let (_, root) =
        x509_parser::parse_x509_certificate(&minted.root.certificate_der).expect("a certificate");
    assert_eq!(root.validity().not_before.to_datetime().year(), FLOOR_YEAR);
    assert_eq!(root.validity().not_after.to_datetime().year(), FLOOR_YEAR + ROOT_YEARS);
    // `accepted` checks validity at the real time, so the chain test above is
    // also the proof the window covers today.
    accepted(&minted.root.certificate_der, &minted.server.certificate_der, "dev-01.local")
        .expect("valid now");
}

/// Seconds since the epoch at `hour`:00 UTC on a calendar day.
fn at(year: i64, month: i64, day: i64, hour: u64) -> u64 {
    u64::try_from(crate::judge::days_from_civil(year, month, day)).expect("after the epoch") * 86_400
        + hour * 3_600
}

/// `(year, month, day, hour)`, UTC.
type Instant = (i32, u8, u8, u8);

/// A certificate's window as `x509-parser` reads it — a parser independent of
/// the writer — at each end.
fn window(der: &[u8]) -> (Instant, Instant) {
    let (_, cert) = x509_parser::parse_x509_certificate(der).expect("a certificate");
    let read = |t: x509_parser::time::ASN1Time| {
        let t = t.to_datetime();
        (t.year(), u8::from(t.month()), t.day(), t.hour())
    };
    (read(cert.validity().not_before), read(cert.validity().not_after))
}

/// **A leaf issued late in a year lives five years from the day it was
/// issued**, not from the first of that year: its window starts at the
/// issuing day's midnight and ends on the same day five years on.
#[test]
fn a_leaf_issued_in_december_lives_five_years_from_that_day() {
    let minted = mint_three();
    let key = KeyPair::from_pem(&minted.root.key).expect("the root key reads back");
    let issuer = Issuer::from_ca_cert_pem(&minted.root.certificate, key).expect("an issuer");
    let names = vec!["dev-01.local".to_owned()];
    let leaf = issue_leaf(&issuer, &names, &mut counting(), at(2031, 12, 20, 15)).expect("issues");
    assert_eq!(
        window(&leaf.certificate_der),
        ((2031, 12, 20, 0), (2031 + LEAF_YEARS, 12, 20, 0))
    );
}

/// **The root, and the server leaf minted with it, are dated the same way**:
/// from the minting day, ten and five years on.
#[test]
fn a_root_minted_in_december_lives_ten_years_from_that_day() {
    let minted = mint(&request(&["local"]), &mut counting(), at(2031, 12, 20, 15)).expect("mints");
    assert_eq!(
        window(&minted.root.certificate_der),
        ((2031, 12, 20, 0), (2031 + ROOT_YEARS, 12, 20, 0))
    );
    assert_eq!(
        window(&minted.server.certificate_der),
        ((2031, 12, 20, 0), (2031 + LEAF_YEARS, 12, 20, 0))
    );
}

/// **A window starting on 29 February ends on 1 March** of a year that has
/// no 29 February — never earlier than the full term.
#[test]
fn a_window_starting_on_a_leap_day_ends_on_the_first_of_march() {
    let minted = mint(&request(&["local"]), &mut counting(), at(2028, 2, 29, 9)).expect("mints");
    assert_eq!(
        window(&minted.root.certificate_der),
        ((2028, 2, 29, 0), (2038, 3, 1, 0))
    );
    assert_eq!(
        window(&minted.server.certificate_der),
        ((2028, 2, 29, 0), (2033, 3, 1, 0))
    );
}

/// **A clock earlier than the floor still dates from the floor's first
/// day** — a board whose clock nothing has set yet issues a certificate that
/// is valid once its clock is right.
#[test]
fn a_clock_before_the_floor_dates_from_the_floor() {
    let minted = mint(&request(&["local"]), &mut counting(), at(2025, 11, 3, 12)).expect("mints");
    let floor = (FLOOR_YEAR, 1, 1, 0);
    assert_eq!(
        window(&minted.root.certificate_der),
        (floor, (FLOOR_YEAR + ROOT_YEARS, 1, 1, 0))
    );
    assert_eq!(
        window(&minted.server.certificate_der),
        (floor, (FLOOR_YEAR + LEAF_YEARS, 1, 1, 0))
    );
}

/// **A clock no certificate can be dated from is refused, never a panic** —
/// PID 1 links this, and a panic there is a kernel panic.
#[test]
fn a_clock_beyond_any_certificate_date_is_refused() {
    assert!(matches!(
        mint(&request(&["local"]), &mut counting(), u64::MAX),
        Err(Error::Mint(_))
    ));
}

/// Two mints are two roots. Deterministic output would mean every fleet
/// shared one.
#[test]
fn every_mint_is_its_own_key_and_serial() {
    let a = mint_three();
    let b = mint_three();
    assert_ne!(a.root.key, b.root.key);
    assert_ne!(a.server.key, b.server.key);
    assert_ne!(a.root.key, a.server.key, "the server must not reuse the root's key");
    assert_ne!(fingerprint(&a.root.certificate_der), fingerprint(&b.root.certificate_der));
}

#[test]
fn the_root_is_an_authority_and_the_server_is_not() {
    let minted = mint_three();
    let (_, root) =
        x509_parser::parse_x509_certificate(&minted.root.certificate_der).expect("a certificate");
    let (_, server) =
        x509_parser::parse_x509_certificate(&minted.server.certificate_der).expect("a certificate");
    assert!(root.is_ca());
    assert!(!server.is_ca());
    assert_eq!(
        root.subject().iter_common_name().next().and_then(|c| c.as_str().ok()),
        Some("dev-fleet")
    );
}

/// The standard fields a leaf is judged on by a chain engine that builds the
/// path itself, read back with `x509-parser`.
#[derive(Debug, PartialEq, Eq)]
struct LeafFields {
    authority_key_id: Option<Vec<u8>>,
    subject_key_id: Option<Vec<u8>>,
    /// `(critical, digitalSignature, keyCertSign)`.
    key_usage: Option<(bool, bool, bool)>,
    /// `(serverAuth, clientAuth)`.
    extended_key_usage: Option<(bool, bool)>,
}

fn leaf_fields(der: &[u8]) -> LeafFields {
    use x509_parser::extensions::ParsedExtension;
    let (_, certificate) = x509_parser::parse_x509_certificate(der).expect("a certificate");
    let mut fields = LeafFields {
        authority_key_id: None,
        subject_key_id: None,
        key_usage: None,
        extended_key_usage: None,
    };
    for extension in certificate.extensions() {
        match extension.parsed_extension() {
            ParsedExtension::AuthorityKeyIdentifier(aki) => {
                fields.authority_key_id = aki.key_identifier.as_ref().map(|k| k.0.to_vec());
            }
            ParsedExtension::SubjectKeyIdentifier(ski) => {
                fields.subject_key_id = Some(ski.0.to_vec());
            }
            ParsedExtension::KeyUsage(ku) => {
                fields.key_usage =
                    Some((extension.critical, ku.digital_signature(), ku.key_cert_sign()));
            }
            ParsedExtension::ExtendedKeyUsage(eku) => {
                fields.extended_key_usage = Some((eku.server_auth, eku.client_auth));
            }
            _ => {}
        }
    }
    fields
}

fn subject_key_id(der: &[u8]) -> Vec<u8> {
    leaf_fields(der)
        .subject_key_id
        .expect("the certificate carries a subject key identifier")
}

/// **The server's certificate carries every field a chain engine needs to
/// build its path without being handed the root**: an authority key identifier
/// equal to the root's subject key identifier, its own subject key identifier,
/// a critical key usage of digitalSignature alone, and an extended key usage of
/// serverAuth and clientAuth — clientAuth because `rivet-proxyd` presents this
/// same file to the applications behind it as its client certificate.
#[test]
fn the_server_carries_the_standard_leaf_fields() {
    let minted = mint_three();
    let root_ski = subject_key_id(&minted.root.certificate_der);
    let fields = leaf_fields(&minted.server.certificate_der);
    assert_eq!(fields.authority_key_id.as_ref(), Some(&root_ski), "{fields:?}");
    let own = fields.subject_key_id.clone().expect("the server has its own identifier");
    assert_ne!(own, root_ski, "the server's identifier must be its own key's");
    assert_eq!(fields.key_usage, Some((true, true, false)), "{fields:?}");
    assert_eq!(fields.extended_key_usage, Some((true, true)), "{fields:?}");
}

/// **And with every field present the server still chains to its root**, as a
/// server and as a client, by the verifiers `rustls` builds — the ones the
/// proxy and the applications behind it run.
#[test]
fn the_server_with_its_fields_verifies_as_server_and_as_client() {
    let minted = mint_three();
    accepted(&minted.root.certificate_der, &minted.server.certificate_der, "dev-01.local")
        .expect("accepted as a server");
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls_pki_types::CertificateDer::from(minted.root.certificate_der.clone()))
        .expect("the root is a certificate");
    let clients = rustls::server::WebPkiClientVerifier::builder_with_provider(
        std::sync::Arc::new(roots),
        std::sync::Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .expect("a verifier over one root");
    clients
        .verify_client_cert(
            &rustls_pki_types::CertificateDer::from(minted.server.certificate_der.clone()),
            &[],
            rustls_pki_types::UnixTime::now(),
        )
        .expect("accepted as a client");
}

/// **A leaf issued later from a resting root has the same shape**: the root
/// read back from its PEM, the way a machine holding one would, signs a leaf
/// that carries the fields and chains. An empty name list is refused.
#[test]
fn a_leaf_issued_from_a_resting_root_has_the_same_shape() {
    let minted = mint_three();
    let key = KeyPair::from_pem(&minted.root.key).expect("the root key reads back");
    let issuer = Issuer::from_ca_cert_pem(&minted.root.certificate, key).expect("an issuer");
    let names = vec!["dev-01.local".to_owned()];
    let mut random = counting();
    let leaf = issue_leaf(&issuer, &names, &mut random, 0).expect("issues");
    let fields = leaf_fields(&leaf.certificate_der);
    assert_eq!(
        fields.authority_key_id,
        Some(subject_key_id(&minted.root.certificate_der)),
        "{fields:?}"
    );
    assert!(fields.subject_key_id.is_some(), "{fields:?}");
    assert_eq!(fields.key_usage, Some((true, true, false)), "{fields:?}");
    assert_eq!(fields.extended_key_usage, Some((true, true)), "{fields:?}");
    accepted(&minted.root.certificate_der, &leaf.certificate_der, "dev-01.local")
        .expect("chains to the root");

    assert!(matches!(issue_leaf(&issuer, &[], &mut random, 0), Err(Error::Mint(_))));
    let mut refusing = |_len: usize| -> Result<Vec<u8>, String> { Err("none".to_owned()) };
    assert!(matches!(
        issue_leaf(&issuer, &names, &mut refusing, 0),
        Err(Error::Randomness(_))
    ));
}

#[test]
fn the_pem_reads_back_to_the_der_it_was_printed_from() {
    let minted = mint_three();
    assert_eq!(der_of_pem(&minted.root.certificate), Some(minted.root.certificate_der.clone()));
    assert_eq!(der_of_pem("not a certificate"), None);
}

/// The fingerprint is SHA-256 over the DER, as a trust store prints it.
/// Known vector: SHA-256 of the empty string.
#[test]
fn the_fingerprint_is_sha256_of_the_der() {
    assert_eq!(
        fingerprint(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}
