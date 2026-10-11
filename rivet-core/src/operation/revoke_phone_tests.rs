//! `apply revoke-phone`'s tests — the inverse of `apply rename-phone`'s.
//!
//! The fixture is a fleet server: a machine whose `rivet-fleetd` data
//! directory already holds `register/devices/`. **The one thing every
//! refusal test here has in common** — the same rule `remove_member_tests.rs`
//! states: when a refusal is asserted, the fake is asserted to have removed
//! nothing.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::ErrorCode;
use crate::operation::filesystem::fake::{FakeFiles, Node};

/// A valid phone identity — 43 characters of the base64url alphabet.
const IDENTITY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn fleetd() -> ServiceName {
    ServiceName::new(FLEET_SERVER_APP).expect("rivet-fleetd is a service name")
}

fn devices_dir() -> String {
    format!("{}/{REGISTER_DIR}/{DEVICES_DIR}", data_dir(STATE_MOUNT_POINT, &fleetd()))
}

fn phones_dir() -> String {
    format!("{}/{REGISTER_DIR}/{PHONES_DIR}", data_dir(STATE_MOUNT_POINT, &fleetd()))
}

fn record_path(identity: &str) -> String {
    format!("{}/{identity}{RECORD_SUFFIX}", phones_dir())
}

/// A representative phone record — the same shape `rename-phone` would have
/// written.
const RECORD: &str = "identity = \"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\nlabel = \
                      \"Pixel 8\"\npublic_key = \"MFkBAgM=\"\nenrolled = \
                      \"2000-01-02T03:04:05Z\"\n";

/// A fleet server: its `register/devices/` exists, owned by rivet-fleetd.
fn fleet_server() -> FakeFiles {
    let identity = crate::service::app_identity(&fleetd());
    FakeFiles::new().with(
        &devices_dir(),
        Node::directory().moded(0o700).owned(identity.uid, identity.gid),
    )
}

/// A fleet server that already holds a phone enrolled under `identity`.
fn fleet_server_with_phone(identity: &str) -> FakeFiles {
    fleet_server().with(&record_path(identity), Node::file(RECORD))
}

fn revoke(fake: &mut FakeFiles, words: &[&str]) -> Result<Revoked, OperationError> {
    perform(words, fake)
}

fn confirmed(fake: &mut FakeFiles, identity: &str) -> Result<Revoked, OperationError> {
    revoke(fake, &[SUBJECT, identity, CONFIRMATION])
}

#[test]
fn the_subject_name_is_revoke_phone() {
    assert_eq!(SUBJECT, "revoke-phone");
}

#[test]
fn a_phones_record_is_removed_from_the_register() {
    let mut fake = fleet_server_with_phone(IDENTITY);
    let path = record_path(IDENTITY);
    let done = confirmed(&mut fake, IDENTITY).expect("removes");
    assert!(!fake.has(&path), "the record is still there");
    assert_eq!(done.identity, IDENTITY);
    assert_eq!(done.path, path);
    assert_eq!(done.bytes, RECORD.len() as u64);
    assert_eq!(fake.did, vec![format!("remove-file {path}")]);
    assert!(done.journalled());
}

#[test]
fn without_the_confirmation_it_plans_and_removes_nothing() {
    let mut fake = fleet_server_with_phone(IDENTITY);
    let planned = revoke(&mut fake, &[SUBJECT, IDENTITY]).expect_err("a plan");
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a confirmation refusal carries a plan");
    let words = plan.narrative();
    assert!(words.contains("NOTHING IS RESTARTED"), "{words}");
    assert!(words.contains("CANNOT BE UNDONE"), "{words}");
    assert!(words.contains(IDENTITY), "{words}");

    let expected = format!("rivet apply {SUBJECT} {IDENTITY} confirm");
    assert_eq!(plan.confirm_with(), expected);

    assert!(fake.did.is_empty());
    assert!(fake.has(&record_path(IDENTITY)), "the plan removed the file");

    let tail: Vec<&str> = expected
        .strip_prefix("rivet apply ")
        .expect("confirm_with starts with `rivet apply `")
        .split(' ')
        .collect();
    assert_eq!(tail, [SUBJECT, IDENTITY, CONFIRMATION]);
    let done = revoke(&mut fake, &tail).expect("confirm_with's own command must remove it");
    assert_eq!(done.identity, IDENTITY);
    assert!(!fake.has(&record_path(IDENTITY)), "confirm_with's own command left the record");
}

/// **`is_identity` is `rename-phone`'s own function, reused rather than
/// redefined** — the two subjects can never disagree about what an identity
/// is.
#[test]
fn a_non_identity_is_refused_before_anything_is_composed() {
    for bad in ["../escape", "AAA/x", "SHORT", "not base64 at all, way too short!!"] {
        let mut fake = fleet_server_with_phone(IDENTITY);
        let err = confirmed(&mut fake, bad).expect_err("refused");
        assert_eq!(err.code(), ErrorCode::Arguments, "{bad:?}");
        assert!(fake.did.is_empty(), "{bad:?} removed something");
        assert!(fake.has(&record_path(IDENTITY)), "{bad:?} removed the real phone");
    }
}

#[test]
fn a_machine_that_is_not_a_fleet_server_is_refused() {
    let mut fake = FakeFiles::new();
    let err = confirmed(&mut fake, IDENTITY).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}

/// **The inverse of `rename-phone`'s replaceable check.** Nothing there is a
/// refusal and not a success, exactly as `remove-file` refuses a path that is
/// not there.
#[test]
fn a_phone_that_is_not_enrolled_is_refused_rather_than_reported_as_revoked() {
    let mut fake = fleet_server();
    let err = confirmed(&mut fake, IDENTITY).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(err.message().contains("NO SUCH PHONE"), "{}", err.message());
    assert!(fake.did.is_empty());
}

/// Revoking the same phone twice is refused the second time, the same way a
/// second `remove-file` on a gone path is.
#[test]
fn revoking_the_same_phone_twice_refuses_the_second_time() {
    let mut fake = fleet_server_with_phone(IDENTITY);
    confirmed(&mut fake, IDENTITY).expect("revokes once");
    let err = confirmed(&mut fake, IDENTITY).expect_err("refused the second time");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(err.message().contains("NO SUCH PHONE"), "{}", err.message());
}

/// Something other than a file at the composed path (a directory, say) is
/// refused rather than removed.
#[test]
fn something_other_than_a_file_at_the_path_is_refused() {
    let mut fake = fleet_server().with(&record_path(IDENTITY), Node::directory());
    let err = confirmed(&mut fake, IDENTITY).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
    assert!(fake.has(&record_path(IDENTITY)), "the directory was removed");
}
