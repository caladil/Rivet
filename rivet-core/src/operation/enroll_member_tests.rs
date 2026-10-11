//! `apply enroll-member`'s tests.
//!
//! The fixture is a fleet server: a machine whose `rivet-fleetd` data directory
//! already holds `register/devices/`, as its supervisor staged at provision
//! time. The record bytes are a small member TOML — this subject writes bytes
//! and does not interpret them, so any representative content serves.

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

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A representative member record — the bytes a workstation would render from a
/// `Device` and hand over as hex.
fn record() -> Vec<u8> {
    b"id = \"1f0532852afc2842\"\nname = \"rivet-dev\"\n".to_vec()
}

/// A fleet server: its `register/devices/` exists, owned by rivet-fleetd.
fn fleet_server() -> FakeFiles {
    let identity = app_identity(&fleetd());
    FakeFiles::new().with(
        &devices_dir(),
        Node::directory().moded(0o700).owned(identity.uid, identity.gid),
    )
}

fn enroll(fake: &mut FakeFiles, words: &[&str]) -> Result<Enrolled, OperationError> {
    perform(words, fake)
}

fn confirmed(fake: &mut FakeFiles, id: &str, bytes: &[u8]) -> Result<Enrolled, OperationError> {
    enroll(fake, &[SUBJECT, id, HEX_FLAG, &hex_of(bytes), CONFIRMATION])
}

#[test]
fn a_member_record_is_written_owned_by_fleetd_and_private() {
    let mut fake = fleet_server();
    let bytes = record();
    let done = confirmed(&mut fake, ID, &bytes).expect("enrolls");
    let path = record_path(ID);
    let node = &fake.nodes[&path];
    let identity = app_identity(&fleetd());
    assert_eq!(node.contents, bytes);
    assert_eq!((node.uid, node.gid), (identity.uid, identity.gid), "owned by rivet-fleetd");
    assert_eq!(node.mode, SECRET_MODE);
    assert_eq!(fake.did, vec![format!("install-secret {path}")]);
    assert!(done.journalled());
}

#[test]
fn without_the_confirmation_it_plans_and_writes_nothing() {
    let mut fake = fleet_server();
    let planned =
        enroll(&mut fake, &[SUBJECT, ID, HEX_FLAG, &hex_of(&record())]).expect_err("a plan");
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let words = planned.plan().expect("a plan").narrative();
    assert!(words.contains("NOTHING IS RESTARTED"), "{words}");
    assert!(words.contains(ID), "{words}");
    assert!(fake.did.is_empty());
    assert!(!fake.nodes.contains_key(&record_path(ID)));
}

#[test]
fn a_re_enroll_replaces_and_reports_the_old_bytes() {
    let old = "an older record for this id";
    let mut fake = fleet_server().with(&record_path(ID), Node::file(old));
    let done = confirmed(&mut fake, ID, b"new").expect("re-enrolls");
    let Value::Object(fields) = done.value() else { panic!("an object") };
    let replaced = fields.iter().find(|(k, _)| *k == "replaced-bytes").map(|(_, v)| v.clone());
    assert_eq!(replaced, Some(Value::Integer(old.len() as i64)));
}

/// **The path is composed, never given.** No id reaches outside
/// `register/devices/`.
#[test]
fn a_non_identifier_id_is_refused_before_anything_is_composed() {
    for bad in ["../escape", "1f0532852afc2842/x", "SHORT", "1F0532852AFC2842", "not-hex-at-all!!"]
    {
        let mut fake = fleet_server();
        let err = confirmed(&mut fake, bad, &record()).expect_err("refused");
        assert_eq!(err.code(), ErrorCode::Arguments, "{bad:?}");
        assert!(fake.did.is_empty(), "{bad:?} wrote something");
    }
}

#[test]
fn a_machine_that_is_not_a_fleet_server_is_refused() {
    // No register/devices/ — a machine the supervisor never staged a register
    // onto, i.e. not a fleet server.
    let mut fake = FakeFiles::new();
    let err = confirmed(&mut fake, ID, &record()).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}

#[test]
fn an_empty_record_is_refused() {
    let mut fake = fleet_server();
    let err = enroll(&mut fake, &[SUBJECT, ID, HEX_FLAG, "", CONFIRMATION]).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Arguments);
    assert!(fake.did.is_empty());
}
