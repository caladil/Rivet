//! **The operator is a certificate that names no host**, proved against rcgen's
//! own encodings and against certificates built by hand where rcgen will not
//! produce the shape.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A leaf signed by a throwaway authority, with `names` as its
/// `subjectAltName` and `common` as its subject.
fn leaf(names: Vec<rcgen::SanType>, common: &str) -> Vec<u8> {
    let issuer_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
    let mut issuer_params = rcgen::CertificateParams::new(Vec::new()).expect("params");
    issuer_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let issuer = rcgen::Issuer::new(issuer_params, issuer_key);

    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
    let mut params = rcgen::CertificateParams::new(Vec::new()).expect("params");
    params.subject_alt_names = names;
    params.distinguished_name = {
        let mut dn = rcgen::DistinguishedName::new();
        dn.push(rcgen::DnType::CommonName, common);
        dn
    };
    params.is_ca = rcgen::IsCa::NoCa;
    params
        .signed_by(&key, &issuer)
        .expect("a leaf")
        .der()
        .to_vec()
}

fn dns(name: &str) -> rcgen::SanType {
    rcgen::SanType::DnsName(name.to_owned().try_into().expect("an IA5 name"))
}

/// **The operator's certificate is the operator**, whatever a person called
/// themselves — including a name shaped like a host, because the subject is
/// not consulted.
#[test]
fn a_leaf_naming_no_host_is_the_operator_whatever_its_subject_says() {
    for person in [
        "Your Name",
        "fleet-owner",
        "operator",
        "your.name",
        "rivet-02.local",
    ] {
        assert_eq!(
            classify_caller(&leaf(Vec::new(), person)),
            Caller::Operator,
            "{person:?} as a subject changed the answer"
        );
    }
}

/// **A machine's front door is a machine**, and the refusal names what it
/// answers to.
#[test]
fn a_leaf_with_dns_names_is_a_machine_and_says_which() {
    let der = leaf(
        vec![dns("rivet-02.local"), dns("rivet-02.my-fleet.home.arpa")],
        "rivet-02.local",
    );
    let caller = classify_caller(&der);
    assert_eq!(
        caller,
        Caller::Machine(vec![
            "rivet-02.local".to_owned(),
            "rivet-02.my-fleet.home.arpa".to_owned()
        ])
    );
    assert!(!caller.is_operator());
    let why = caller.refusal().expect("a machine is refused");
    assert!(why.contains("rivet-02.local"), "{why}");
    assert!(why.contains("names a host"), "{why}");
}

/// An IP address names a host exactly as a DNS name does.
#[test]
fn a_leaf_with_ip_addresses_is_a_machine() {
    let der = leaf(
        vec![
            rcgen::SanType::IpAddress("192.168.50.50".parse().expect("v4")),
            rcgen::SanType::IpAddress("fe80::1".parse().expect("v6")),
        ],
        "a machine",
    );
    assert_eq!(
        classify_caller(&der),
        Caller::Machine(vec!["192.168.50.50".to_owned(), "fe80::1".to_owned()])
    );
}

/// **Any `subjectAltName` is not the operator's shape**, even one that names no
/// host: the rule is the extension's presence, so no entry kind can fall
/// through to "operator".
#[test]
fn a_leaf_with_any_other_kind_of_alternative_name_is_not_the_operator() {
    for names in [
        vec![rcgen::SanType::URI(
            "https://rivet-02.local/"
                .to_owned()
                .try_into()
                .expect("IA5"),
        )],
        vec![rcgen::SanType::Rfc822Name(
            "joel@example.com".to_owned().try_into().expect("IA5"),
        )],
        vec![rcgen::SanType::OtherName((
            vec![1, 3, 6, 1, 4, 1, 311, 20, 2, 3],
            rcgen::OtherNameValue::Utf8String("joel".to_owned()),
        ))],
    ] {
        let caller = classify_caller(&leaf(names.clone(), "Your Name"));
        assert!(
            matches!(caller, Caller::Machine(ref entries) if entries.len() == 1),
            "{names:?} came back {caller:?}"
        );
    }
}

/// Nothing that is not a certificate is the operator, and nothing panics.
#[test]
fn rubbish_is_unreadable_and_never_the_operator() {
    for bytes in [
        b"".as_slice(),
        b"not der".as_slice(),
        &[0x30],
        &[0x30, 0x00],
        &[0x30, 0x82, 0xff, 0xff],
        &[0x30, 0x03, 0x30, 0x01, 0x00],
    ] {
        assert_eq!(classify_caller(bytes), Caller::Unreadable, "{bytes:?}");
    }
}

/// **No prefix of a certificate is the operator, and neither is a certificate
/// with bytes after it.** A walk that stopped early would read a machine's leaf
/// cut before its extensions as a leaf with none.
#[test]
fn a_truncated_or_padded_certificate_is_never_the_operator() {
    for der in [
        leaf(vec![dns("rivet-02.local")], "rivet-02.local"),
        leaf(Vec::new(), "Your Name"),
    ] {
        for cut in 0..der.len() {
            assert_eq!(
                classify_caller(&der[..cut]),
                Caller::Unreadable,
                "a {cut}-byte prefix"
            );
        }
        let mut padded = der.clone();
        padded.push(0x00);
        assert_eq!(
            classify_caller(&padded),
            Caller::Unreadable,
            "a trailing byte"
        );
    }
}

// ---------------------------------------------------------------------------
// Shapes rcgen will not write, built by hand. The walk never checks a
// signature, so a `TBSCertificate` with placeholder fields is enough.
// ---------------------------------------------------------------------------

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(u8::try_from(n).expect("short"));
    } else if n <= 0xff {
        out.extend([0x81, u8::try_from(n).expect("one byte")]);
    } else {
        out.extend([
            0x82,
            u8::try_from(n >> 8).expect("hi"),
            u8::try_from(n & 0xff).expect("lo"),
        ]);
    }
    out.extend_from_slice(content);
    out
}

/// A `Certificate` whose `TBSCertificate` holds `version` (if any), six
/// placeholder fields, and then `tail`.
fn built(version: bool, tail: &[u8]) -> Vec<u8> {
    let mut tbs = Vec::new();
    if version {
        tbs.extend(der(0xa0, &der(0x02, &[0x02])));
    }
    tbs.extend(der(0x02, &[0x01])); // serialNumber
    tbs.extend(der(0x30, &der(0x06, &[0x2b, 0x65, 0x70]))); // signature
    tbs.extend(der(0x30, &[])); // issuer
    tbs.extend(der(0x30, &[])); // validity
    tbs.extend(der(0x30, &[])); // subject
    tbs.extend(der(0x30, &der(0x30, &der(0x06, &[0x2b, 0x65, 0x70])))); // spki
    tbs.extend_from_slice(tail);
    let mut body = der(0x30, &tbs);
    body.extend(der(0x30, &der(0x06, &[0x2b, 0x65, 0x70]))); // signatureAlgorithm
    body.extend(der(0x03, &[0x00])); // signature
    der(0x30, &body)
}

/// `[3] { SEQUENCE { extension… } }`.
fn extensions(each: &[Vec<u8>]) -> Vec<u8> {
    der(0xa3, &der(0x30, &each.concat()))
}

/// One `Extension`, with `critical` written out when it is given.
fn extension(oid: &[u8], critical: Option<bool>, value: &[u8]) -> Vec<u8> {
    let mut content = der(0x06, oid);
    if let Some(flag) = critical {
        content.extend(der(0x01, &[if flag { 0xff } else { 0x00 }]));
    }
    content.extend(der(0x04, value));
    der(0x30, &content)
}

/// A `subjectAltName` extension holding `entries`, already encoded.
fn san(critical: Option<bool>, entries: &[Vec<u8>]) -> Vec<u8> {
    extension(&SUBJECT_ALT_NAME, critical, &der(0x30, &entries.concat()))
}

/// `2.5.29.19`, basicConstraints — any extension that is not the one asked.
const BASIC_CONSTRAINTS: [u8; 3] = [0x55, 0x1d, 0x13];

/// The hand builder produces what the rcgen tests already established: no
/// extensions is the operator, a DNS name is a machine. So the refusals below
/// are about the shape each one varies, not about the builder.
#[test]
fn the_hand_built_certificates_agree_with_the_minted_ones() {
    assert_eq!(classify_caller(&built(true, &[])), Caller::Operator);
    assert_eq!(
        classify_caller(&built(
            true,
            &extensions(&[extension(&BASIC_CONSTRAINTS, None, &der(0x30, &[]))])
        )),
        Caller::Operator
    );
    assert_eq!(
        classify_caller(&built(
            true,
            &extensions(&[san(Some(true), &[der(0x82, b"rivet-02.local")])])
        )),
        Caller::Machine(vec!["rivet-02.local".to_owned()])
    );
}

/// **A certificate with no version is refused, not read as naming nothing.**
/// Only v3 carries extensions; a v1 leaf is a shape the handshake refuses.
#[test]
fn a_certificate_without_a_version_is_unreadable() {
    assert_eq!(classify_caller(&built(false, &[])), Caller::Unreadable);
    let mut tbs_v2 = built(true, &[]);
    // `02 01 02` is the version INTEGER's encoding; make it v2's `01`.
    let at = tbs_v2
        .windows(5)
        .position(|w| w == [0xa0, 0x03, 0x02, 0x01, 0x02])
        .expect("a version");
    tbs_v2[at + 4] = 0x01;
    assert_eq!(classify_caller(&tbs_v2), Caller::Unreadable);
}

/// Two `subjectAltName` extensions, an empty one, and one holding a tag RFC
/// 5280 does not define are all refused.
#[test]
fn a_malformed_alternative_name_is_unreadable() {
    let one = san(None, &[der(0x82, b"rivet-02.local")]);
    for (what, tail) in [
        ("two of them", extensions(&[one.clone(), one.clone()])),
        ("an empty list", extensions(&[san(None, &[])])),
        (
            "an undefined tag",
            extensions(&[san(None, &[der(0x89, b"x")])]),
        ),
        (
            "a value that is not a SEQUENCE",
            extensions(&[extension(&SUBJECT_ALT_NAME, None, &der(0x04, b"x"))]),
        ),
        ("an extension with a field after its value", {
            let mut content = der(0x06, &SUBJECT_ALT_NAME);
            content.extend(der(0x04, &der(0x30, &der(0x82, b"a.local"))));
            content.extend(der(0x05, &[]));
            extensions(&[der(0x30, &content)])
        }),
        ("an empty extensions wrapper", der(0xa3, &[])),
        (
            "a field after the extensions",
            [extensions(std::slice::from_ref(&one)), der(0x05, &[])].concat(),
        ),
        (
            "something that is not the extensions after the key",
            der(0xa4, &[]),
        ),
    ] {
        assert_eq!(
            classify_caller(&built(true, &tail)),
            Caller::Unreadable,
            "{what}"
        );
    }
}

/// **A malformed extension that is not `subjectAltName` still stops the walk**,
/// wherever it sits — a machine's names after it must not be skipped by a reader
/// that gave up on the extension before them.
#[test]
fn a_malformed_extension_before_the_names_is_unreadable() {
    let broken = der(0x30, &der(0x06, &BASIC_CONSTRAINTS)); // no value
    let tail = extensions(&[broken, san(None, &[der(0x82, b"rivet-02.local")])]);
    assert_eq!(classify_caller(&built(true, &tail)), Caller::Unreadable);
}

/// The unique identifiers RFC 5280 permits between the key and the extensions
/// are stepped over, so a names-bearing leaf that carries one is still a
/// machine.
#[test]
fn unique_identifiers_do_not_hide_the_extensions() {
    let tail = [
        der(0x81, &[0x00, 0x01]),
        extensions(&[san(None, &[der(0x87, &[10, 0, 0, 1])])]),
    ]
    .concat();
    assert_eq!(
        classify_caller(&built(true, &tail)),
        Caller::Machine(vec!["10.0.0.1".to_owned()])
    );
}

/// The operator is told nothing, and an unreadable certificate is told why it
/// was not taken for one.
#[test]
fn only_a_caller_that_is_not_the_operator_has_a_refusal() {
    assert_eq!(Caller::Operator.refusal(), None);
    assert!(Caller::Operator.is_operator());
    let why = Caller::Unreadable.refusal().expect("refused");
    assert!(why.contains("could not be read"), "{why}");
    assert!(!Caller::Unreadable.is_operator());
}

/// **A machine's own DNS names, in the order the certificate lists them, and
/// nothing that is not a DNS name** — what `rivet-proxyd` takes its default
/// upstream name from (`ADR-060`).
#[test]
fn the_dns_names_come_back_in_order_and_other_kinds_are_left_out() {
    let der = leaf(
        vec![
            rcgen::SanType::IpAddress("10.0.0.1".parse().expect("v4")),
            dns("rivet-02.local"),
            dns("rivet-02.my-fleet.home.arpa"),
        ],
        "rivet-02.local",
    );
    assert_eq!(
        dns_names(&der),
        Some(vec![
            "rivet-02.local".to_owned(),
            "rivet-02.my-fleet.home.arpa".to_owned()
        ])
    );
}

/// No `subjectAltName` is an empty list, and a certificate the walk refuses is
/// `None` — never an empty list that reads as "no names".
#[test]
fn no_names_is_empty_and_an_unreadable_certificate_is_none() {
    assert_eq!(dns_names(&leaf(Vec::new(), "fleet-owner")), Some(Vec::new()));
    assert_eq!(dns_names(&[0x30, 0x01]), None);
}
