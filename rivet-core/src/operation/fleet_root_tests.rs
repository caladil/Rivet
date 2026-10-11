//! `get fleet-root`'s tests. The certificate is a hand-composed PEM around
//! three bytes: this subject decodes and fingerprints, and never interprets
//! the certificate, so its content only has to be one PEM block.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::ErrorCode;
use crate::operation::filesystem::fake::{FakeFiles, Node};

const PEM: &str = "-----BEGIN CERTIFICATE-----\nAAEC\n-----END CERTIFICATE-----\n";

fn field(value: &Value, name: &str) -> Option<Value> {
    let Value::Object(fields) = value else { panic!("an object") };
    fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())
}

fn resting(pem: &str) -> FakeFiles {
    FakeFiles::new()
        .with("/state/secrets", Node::directory().moded(0o700))
        .with(&crate::fleet_root::root_certificate_path(), Node::file(pem))
}

/// **What rests is what is answered**, byte for byte, with the fingerprint of
/// exactly its DER — the property a workstation installing it relies on.
#[test]
fn the_resting_certificate_is_answered_verbatim_with_the_digest_of_its_der() {
    let mut fake = resting(PEM);
    fake.digest = Some([0xab; 32]);
    let value = perform(&[SUBJECT], &mut fake).expect("answers");

    assert_eq!(field(&value, "present"), Some(Value::Bool(true)));
    assert_eq!(field(&value, "certificate"), Some(Value::str(PEM)));
    assert_eq!(field(&value, "sha256"), Some(Value::str("ab".repeat(32))));
    assert_eq!(fake.digested, vec![vec![0u8, 1, 2]], "the digest was not over the DER");
    assert_eq!(
        field(&value, "path"),
        Some(Value::str("/state/secrets/fleet-root.crt"))
    );
}

#[test]
fn a_machine_with_no_root_answers_present_false() {
    let mut fake = FakeFiles::new();
    fake.digest = Some([0; 32]);
    let value = perform(&[SUBJECT], &mut fake).expect("an answer, not a refusal");
    assert_eq!(field(&value, "present"), Some(Value::Bool(false)));
    assert_eq!(field(&value, "certificate"), None);
    assert!(fake.digested.is_empty());
}

/// The key beside it is never a path this subject reads: it takes no word.
#[test]
fn a_second_word_is_refused_so_no_other_path_can_be_named() {
    let mut fake = resting(PEM);
    fake.digest = Some([0; 32]);
    let err = perform(&[SUBJECT, "/state/secrets/fleet-root.key"], &mut fake).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Arguments);
    assert!(fake.whole_reads.is_empty(), "something was read: {:?}", fake.whole_reads);
}

#[test]
fn a_certificate_that_will_not_decode_is_refused() {
    for bad in ["not a certificate", "-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n"] {
        let mut fake = resting(bad);
        fake.digest = Some([0; 32]);
        let err = perform(&[SUBJECT], &mut fake).expect_err("refused");
        assert_eq!(err.code(), ErrorCode::MachineRefused, "{bad}");
    }
    let two = format!("{PEM}{PEM}");
    let mut fake = resting(&two);
    fake.digest = Some([0; 32]);
    assert!(perform(&[SUBJECT], &mut fake).is_err(), "two certificates is not one root");
}

/// A machine with no digest refuses rather than answering a certificate with
/// no fingerprint to check it by.
#[test]
fn no_digest_is_a_refusal() {
    let mut fake = resting(PEM);
    let err = perform(&[SUBJECT], &mut fake).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::MachineRefused);
}
