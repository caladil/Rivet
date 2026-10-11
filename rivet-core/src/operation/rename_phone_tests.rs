//! `apply rename-phone`'s tests.
//!
//! The fixture is a fleet server: a machine whose `rivet-fleetd` data
//! directory already holds `register/devices/`. The record bytes are a small
//! representative TOML — this subject writes bytes and does not interpret
//! them, so any representative content serves.

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

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A representative phone record — the bytes a workstation would render from
/// `rivetos_register::phones::Phone` with a changed label, and hand over as
/// hex.
fn record(label: &str) -> Vec<u8> {
    format!(
        "identity = \"{IDENTITY}\"\nlabel = \"{label}\"\npublic_key = \"MFkBAgM=\"\nenrolled = \
         \"2000-01-02T03:04:05Z\"\n"
    )
    .into_bytes()
}

/// A fleet server: its `register/devices/` exists, owned by rivet-fleetd.
fn fleet_server() -> FakeFiles {
    let identity = app_identity(&fleetd());
    FakeFiles::new().with(
        &devices_dir(),
        Node::directory().moded(0o700).owned(identity.uid, identity.gid),
    )
}

/// A fleet server that already holds a phone enrolled under `IDENTITY`.
fn fleet_server_with_phone() -> FakeFiles {
    fleet_server().with(&record_path(IDENTITY), Node::file(record("Pixel 8")))
}

fn rename(fake: &mut FakeFiles, words: &[&str]) -> Result<Renamed, OperationError> {
    perform(words, fake)
}

fn confirmed(fake: &mut FakeFiles, identity: &str, bytes: &[u8]) -> Result<Renamed, OperationError> {
    rename(fake, &[SUBJECT, identity, HEX_FLAG, &hex_of(bytes), CONFIRMATION])
}

#[test]
fn the_subject_name_is_rename_phone() {
    assert_eq!(SUBJECT, "rename-phone");
}

#[test]
fn a_phone_record_is_rewritten_owned_by_fleetd_and_private() {
    let mut fake = fleet_server_with_phone();
    let bytes = record("New Label");
    let done = confirmed(&mut fake, IDENTITY, &bytes).expect("renames");
    let path = record_path(IDENTITY);
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
    let mut fake = fleet_server_with_phone();
    let before = fake.nodes[&record_path(IDENTITY)].contents.clone();
    let planned = rename(&mut fake, &[SUBJECT, IDENTITY, HEX_FLAG, &hex_of(&record("New"))])
        .expect_err("a plan");
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let words = planned.plan().expect("a plan").narrative();
    assert!(words.contains("NOTHING IS RESTARTED"), "{words}");
    assert!(words.contains(IDENTITY), "{words}");
    assert!(fake.did.is_empty());
    assert_eq!(fake.nodes[&record_path(IDENTITY)].contents, before);
}

/// **A rename never creates** — the inverse of `enroll-member`'s allowance for
/// a first write.
#[test]
fn a_phone_that_is_not_enrolled_is_refused_rather_than_created() {
    let mut fake = fleet_server();
    let err = confirmed(&mut fake, IDENTITY, &record("New")).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(err.message().contains("NO SUCH PHONE"), "{}", err.message());
    assert!(fake.did.is_empty());
    assert!(!fake.has(&record_path(IDENTITY)));
}

/// **The path is composed, never given.** No identity reaches outside
/// `register/phones/`.
#[test]
fn a_non_identity_is_refused_before_anything_is_composed() {
    for bad in ["../escape", "AAA/x", "SHORT", "not base64 at all, way too short!!"] {
        let mut fake = fleet_server_with_phone();
        let err = confirmed(&mut fake, bad, &record("New")).expect_err("refused");
        assert_eq!(err.code(), ErrorCode::Arguments, "{bad:?}");
        assert!(fake.did.is_empty(), "{bad:?} wrote something");
    }
}

#[test]
fn a_machine_that_is_not_a_fleet_server_is_refused() {
    let mut fake = FakeFiles::new();
    let err = confirmed(&mut fake, IDENTITY, &record("New")).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}

#[test]
fn an_empty_record_is_refused() {
    let mut fake = fleet_server_with_phone();
    let err =
        rename(&mut fake, &[SUBJECT, IDENTITY, HEX_FLAG, "", CONFIRMATION]).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Arguments);
    assert!(fake.did.is_empty());
}

#[test]
fn something_other_than_a_file_at_the_path_is_refused() {
    let mut fake = fleet_server().with(&record_path(IDENTITY), Node::directory());
    let err = confirmed(&mut fake, IDENTITY, &record("New")).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}
