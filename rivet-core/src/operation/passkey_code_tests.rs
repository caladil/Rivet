//! `apply passkey-code`'s tests.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::ErrorCode;
use crate::operation::filesystem::fake::{FakeFiles, Node};

/// Spelled out rather than composed, so this file is a second opinion about
/// the path rather than a copy of the code's arithmetic.
const DATA: &str = "/state/app/rivet-proxyd/data";
const FILE: &str = "/state/app/rivet-proxyd/data/passkey-enrolment";

fn proxy_machine() -> FakeFiles {
    let id = app_identity(&ServiceName::new("rivet-proxyd").expect("a name"));
    let mut fake = FakeFiles::new()
        .with("/state/app", Node::directory())
        .with("/state/app/rivet-proxyd", Node::directory())
        .with(DATA, Node::directory().moded(0o700).owned(id.uid, id.gid));
    fake.random = Some(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    fake
}

fn text(value: &Value, key: &str) -> String {
    let Value::Object(fields) = value else {
        panic!("an object")
    };
    match fields.iter().find(|(k, _)| *k == key) {
        Some((_, Value::Str(s))) => s.clone(),
        other => panic!("{key}: {other:?}"),
    }
}

#[test]
fn a_confirmed_code_is_written_for_the_proxy_and_returned_once() {
    let mut fake = proxy_machine();
    let issued = perform(&[SUBJECT, CONFIRMATION], &mut fake).expect("issued");
    assert_eq!(issued.code(), "0123456789");

    let node = &fake.nodes[FILE];
    assert_eq!(node.contents, b"0123456789");
    assert_eq!(node.mode, 0o600);
    let id = app_identity(&ServiceName::new("rivet-proxyd").expect("a name"));
    assert_eq!((node.uid, node.gid), (id.uid, id.gid));
    assert_eq!(fake.did, vec![format!("install-secret {FILE}")]);

    let value = issued.value();
    assert_eq!(text(&value, "code"), "01234-56789");
    assert_eq!(text(&value, "path"), FILE);
    let next = text(&value, "next");
    assert!(next.contains("https://<this machine's name>/sign-in/enroll "), "{next}");
    assert!(!next.contains("/.rivet/"), "{next}");
    assert!(next.contains("remove-file"), "{next}");
}

#[test]
fn without_the_confirmation_no_code_is_drawn_and_the_plan_shows_none() {
    let mut fake = proxy_machine();
    let planned = perform(&[SUBJECT], &mut fake).expect_err("a plan");
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    assert!(!fake.has(FILE));
    let words = planned.plan().expect("a plan").narrative();
    assert!(!words.contains("0123456789"), "{words}");
    assert!(words.contains("rivet apply passkey-code confirm"), "{words}");
    assert!(fake.did.is_empty());
}

#[test]
fn a_second_code_replaces_the_first() {
    let mut fake = proxy_machine();
    perform(&[SUBJECT, CONFIRMATION], &mut fake).expect("first");
    fake.random = Some(vec![31; 10]);
    let second = perform(&[SUBJECT, CONFIRMATION], &mut fake).expect("second");
    assert_eq!(fake.nodes[FILE].contents, b"ZZZZZZZZZZ");
    let Value::Object(fields) = second.value() else {
        panic!("an object")
    };
    assert!(fields.iter().any(|(k, v)| *k == "replaced" && *v == Value::Bool(true)));
}

#[test]
fn a_machine_without_the_proxy_is_refused_and_nothing_is_written() {
    let mut fake = FakeFiles::new().with("/state/app", Node::directory());
    fake.random = Some(vec![0; 10]);
    let refused = perform(&[SUBJECT, CONFIRMATION], &mut fake).expect_err("no proxy");
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("rivet-proxyd"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

#[test]
fn a_link_at_the_code_path_is_refused_rather_than_written_through() {
    let mut fake = proxy_machine().with(FILE, Node::symlink("/state/config/machine.toml"));
    let refused = perform(&[SUBJECT, CONFIRMATION], &mut fake).expect_err("a link");
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}

#[test]
fn no_random_source_or_the_wrong_length_writes_nothing() {
    let mut fake = proxy_machine();
    fake.random = None;
    let refused = perform(&[SUBJECT, CONFIRMATION], &mut fake).expect_err("no source");
    assert_eq!(refused.code(), ErrorCode::MachineRefused);
    assert!(!fake.has(FILE));

    let mut fake = proxy_machine();
    fake.random = Some(vec![1; 3]);
    let refused = perform(&[SUBJECT, CONFIRMATION], &mut fake).expect_err("short");
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(!fake.has(FILE));
}

#[test]
fn a_word_this_subject_does_not_take_is_refused() {
    let mut fake = proxy_machine();
    let refused = perform(&[SUBJECT, "extra", CONFIRMATION], &mut fake).expect_err("extra");
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(fake.did.is_empty());
}
