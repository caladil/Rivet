//! `apply remove-member`'s tests — the inverse of `apply enroll-member`'s.
//!
//! The fixture is a fleet server: a machine whose `rivet-fleetd` data directory
//! already holds `register/devices/`, as its supervisor staged at provision
//! time. **The one thing every refusal test here has in common** — the same
//! rule `remove_tests.rs` states for `remove-file`: when a refusal is
//! asserted, the fake is asserted to have removed nothing.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::ErrorCode;
use crate::operation::filesystem::fake::{FakeFiles, Node};

/// A valid register identifier — sixteen lowercase hex characters.
const ID: &str = "1f0532852afc2842";

fn fleetd() -> ServiceName {
    ServiceName::new(FLEET_SERVER_APP).expect("rivet-fleetd is a service name")
}

/// `/state/app/rivet-fleetd/data/register/devices`, composed the way `perform`
/// composes it.
fn devices_dir() -> String {
    format!("{}/{REGISTER_DIR}/{DEVICES_DIR}", data_dir(STATE_MOUNT_POINT, &fleetd()))
}

fn record_path(id: &str) -> String {
    format!("{}/{id}{RECORD_SUFFIX}", devices_dir())
}

/// A representative member record — the same shape `enroll-member` would have
/// written.
const RECORD: &str = "id = \"1f0532852afc2842\"\nname = \"rivet-dev\"\n";

/// A fleet server: its `register/devices/` exists, owned by rivet-fleetd.
fn fleet_server() -> FakeFiles {
    let identity = crate::service::app_identity(&fleetd());
    FakeFiles::new().with(
        &devices_dir(),
        Node::directory().moded(0o700).owned(identity.uid, identity.gid),
    )
}

/// A fleet server that already holds a member's record.
fn fleet_server_with_member(id: &str) -> FakeFiles {
    fleet_server().with(&record_path(id), Node::file(RECORD))
}

fn remove(fake: &mut FakeFiles, words: &[&str]) -> Result<Removed, OperationError> {
    perform(words, fake)
}

fn confirmed(fake: &mut FakeFiles, id: &str) -> Result<Removed, OperationError> {
    remove(fake, &[SUBJECT, id, CONFIRMATION])
}

#[test]
fn the_subject_name_is_remove_member() {
    assert_eq!(SUBJECT, "remove-member");
}

#[test]
fn a_members_record_is_removed_from_the_register() {
    let mut fake = fleet_server_with_member(ID);
    let path = record_path(ID);
    let done = confirmed(&mut fake, ID).expect("removes");
    assert!(!fake.has(&path), "the record is still there");
    assert_eq!(done.id, ID);
    assert_eq!(done.path, path);
    assert_eq!(done.bytes, RECORD.len() as u64);
    assert_eq!(fake.did, vec![format!("remove-file {path}")]);
    assert!(done.journalled());
}

#[test]
fn without_the_confirmation_it_plans_and_removes_nothing() {
    let mut fake = fleet_server_with_member(ID);
    let planned = remove(&mut fake, &[SUBJECT, ID]).expect_err("a plan");
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a confirmation refusal carries a plan");
    let words = plan.narrative();
    assert!(words.contains("NOTHING IS RESTARTED"), "{words}");
    assert!(words.contains("CANNOT BE UNDONE"), "{words}");
    assert!(words.contains(ID), "{words}");

    // **The plan names the exact command that confirms it** — both the
    // accessor a caller reads, and the same string under `confirm-with` in
    // the structured payload the refusal carries.
    let expected = format!("rivet apply {SUBJECT} {ID} confirm");
    assert_eq!(plan.confirm_with(), expected);
    match plan.value() {
        Value::Object(fields) => {
            let confirm_with = fields.iter().find(|(k, _)| *k == "confirm-with").map(|(_, v)| v);
            assert_eq!(confirm_with, Some(&Value::str(expected.clone())));
        }
        other => panic!("plan.value() is not an object: {other:?}"),
    }

    assert!(fake.did.is_empty());
    assert!(fake.has(&record_path(ID)), "the plan removed the file");

    // **And it is not just a string**: running the exact words after `rivet
    // apply` reaches the same member and actually removes it.
    let tail: Vec<&str> = expected
        .strip_prefix("rivet apply ")
        .expect("confirm_with starts with `rivet apply `")
        .split(' ')
        .collect();
    assert_eq!(tail, [SUBJECT, ID, CONFIRMATION]);
    let done = remove(&mut fake, &tail).expect("confirm_with's own command must remove it");
    assert_eq!(done.id, ID);
    assert!(!fake.has(&record_path(ID)), "confirm_with's own command left the record in place");
}

/// **`is_identifier` is `enroll-member`'s own function, reused rather than
/// redefined** — the two subjects can never disagree about what an id is.
#[test]
fn a_non_identifier_id_is_refused_before_anything_is_composed() {
    for bad in ["../escape", "1f0532852afc2842/x", "SHORT", "1F0532852AFC2842", "not-hex-at-all!!"]
    {
        let mut fake = fleet_server_with_member(ID);
        let err = confirmed(&mut fake, bad).expect_err("refused");
        assert_eq!(err.code(), ErrorCode::Arguments, "{bad:?}");
        assert!(fake.did.is_empty(), "{bad:?} removed something");
        assert!(fake.has(&record_path(ID)), "{bad:?} removed the real member");
    }
}

#[test]
fn a_machine_that_is_not_a_fleet_server_is_refused() {
    // No register/devices/ — a machine the supervisor never staged a register
    // onto, i.e. not a fleet server.
    let mut fake = FakeFiles::new();
    let err = confirmed(&mut fake, ID).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}

/// **The inverse of `enroll-member`'s replaceable check.** Nothing there is a
/// refusal and not a success, exactly as `remove-file` refuses a path that is
/// not there.
#[test]
fn a_member_that_is_not_enrolled_is_refused_rather_than_reported_as_removed() {
    let mut fake = fleet_server();
    let err = confirmed(&mut fake, ID).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(err.message().contains("NO SUCH MEMBER"), "{}", err.message());
    assert!(err.message().contains("different facts"), "{}", err.message());
    assert!(fake.did.is_empty());
}

/// Removing the same member twice is refused the second time, the same way a
/// second `remove-file` on a gone path is.
#[test]
fn removing_the_same_member_twice_refuses_the_second_time() {
    let mut fake = fleet_server_with_member(ID);
    confirmed(&mut fake, ID).expect("removes once");
    let err = confirmed(&mut fake, ID).expect_err("refused the second time");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(err.message().contains("NO SUCH MEMBER"), "{}", err.message());
}

/// Something other than a file at the composed path (a directory, say) is
/// refused rather than removed — `enroll-member`'s `check_replaceable` guards
/// the same shape on the write side.
#[test]
fn something_other_than_a_file_at_the_path_is_refused() {
    let mut fake = fleet_server().with(&record_path(ID), Node::directory());
    let err = confirmed(&mut fake, ID).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
    assert!(fake.has(&record_path(ID)), "the directory was removed");
}
