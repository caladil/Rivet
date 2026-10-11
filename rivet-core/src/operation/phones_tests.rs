//! `get phones`'s tests.
//!
//! The fixture is a fleet server: a machine whose `rivet-fleetd` data
//! directory already holds `register/devices/`, as its supervisor staged at
//! provision time. This subject reads bytes and does not interpret them, so
//! any representative content serves as a phone record.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::ErrorCode;
use crate::operation::filesystem::fake::{FakeFiles, Node};
use crate::service::app_identity;

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

/// A fleet server: its `register/devices/` exists, owned by rivet-fleetd. No
/// `register/phones/` yet — a fleet server that has enrolled no phones.
fn fleet_server() -> FakeFiles {
    let identity = app_identity(&fleetd());
    FakeFiles::new().with(
        &devices_dir(),
        Node::directory().moded(0o700).owned(identity.uid, identity.gid),
    )
}

/// A fleet server whose `register/phones/` directory exists.
fn fleet_server_with_phones_dir() -> FakeFiles {
    let identity = app_identity(&fleetd());
    fleet_server().with(
        &phones_dir(),
        Node::directory().moded(0o700).owned(identity.uid, identity.gid),
    )
}

const IDENTITY_A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const IDENTITY_B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

fn read(fake: &mut FakeFiles) -> Result<Value, OperationError> {
    perform(&[SUBJECT], fake)
}

#[test]
fn the_subject_name_is_phones() {
    assert_eq!(SUBJECT, "phones");
}

#[test]
fn a_machine_that_is_not_a_fleet_server_is_refused() {
    let mut fake = FakeFiles::new();
    let err = read(&mut fake).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
}

#[test]
fn a_fleet_server_with_no_phones_directory_answers_an_empty_list() {
    let mut fake = fleet_server();
    let value = read(&mut fake).expect("an empty list, not a refusal");
    let Value::Object(fields) = value else { panic!("an object") };
    let count = fields.iter().find(|(k, _)| *k == "count").map(|(_, v)| v.clone());
    assert_eq!(count, Some(Value::Integer(0)));
    let phones = fields.iter().find(|(k, _)| *k == "phones").map(|(_, v)| v.clone());
    assert_eq!(phones, Some(Value::List(Vec::new())));
}

#[test]
fn every_enrolled_phone_is_listed_sorted_by_identity_as_hex() {
    let mut fake = fleet_server_with_phones_dir()
        .with(&record_path(IDENTITY_B), Node::file("record-b"))
        .with(&record_path(IDENTITY_A), Node::file("record-a"));
    let value = read(&mut fake).expect("lists");
    let Value::Object(fields) = value else { panic!("an object") };
    let Some(Value::List(phones)) = fields.iter().find(|(k, _)| *k == "phones").map(|(_, v)| v.clone())
    else {
        panic!("a phones list")
    };
    assert_eq!(phones.len(), 2);
    let identities: Vec<String> = phones
        .iter()
        .map(|p| {
            let Value::Object(fields) = p else { panic!("an object") };
            let Some(Value::Str(identity)) =
                fields.iter().find(|(k, _)| *k == "identity").map(|(_, v)| v.clone())
            else {
                panic!("an identity")
            };
            identity
        })
        .collect();
    assert_eq!(identities, vec![IDENTITY_A.to_string(), IDENTITY_B.to_string()], "sorted");

    let record_hex = |p: &Value| {
        let Value::Object(fields) = p else { panic!("an object") };
        let Some(Value::Str(hex)) =
            fields.iter().find(|(k, _)| *k == "record").map(|(_, v)| v.clone())
        else {
            panic!("a record")
        };
        hex
    };
    assert_eq!(record_hex(&phones[0]), hex_of(b"record-a"));
    assert_eq!(record_hex(&phones[1]), hex_of(b"record-b"));
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_second_word_is_refused() {
    let mut fake = fleet_server();
    let err = perform(&[SUBJECT, "extra"], &mut fake).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Arguments);
}

#[test]
fn a_phones_directory_that_is_not_a_directory_is_refused() {
    let mut fake = fleet_server().with(&phones_dir(), Node::file("not a directory"));
    let err = read(&mut fake).expect_err("refused");
    assert_eq!(err.code(), ErrorCode::Unsafe);
}
