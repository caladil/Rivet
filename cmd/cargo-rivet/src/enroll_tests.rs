//! What this verb sends, what it renders, and what it will and will not
//! accept back — the three pure parts of [`super`], asserted with no board.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use super::*;

fn a_device() -> Device {
    Device {
        id: "0011223344556677".to_owned(),
        name: "kitchen-pi".to_owned(),
        role: Some("member".to_owned()),
        ..Device::default()
    }
}

// ---- `enroll_words` --------------------------------------------------------

/// The exact word vector this verb sends, with every one of the three
/// endpoint flags present — `crate::secrets::run`'s shape, one file instead
/// of three.
#[test]
fn enroll_words_is_the_frozen_wire_contract_exactly() {
    let endpoint = Endpoint {
        host: Some("board:7777".to_owned()),
        identity: Some(PathBuf::from("./id")),
        pin: Some("ab".to_owned()),
    };
    let words = enroll_words(&endpoint, "0011223344556677", "deadbeef");
    assert_eq!(
        words,
        [
            "--host",
            "board:7777",
            "--identity",
            "./id",
            "--pin",
            "ab",
            "--",
            "apply",
            "enroll-member",
            "0011223344556677",
            "--hex",
            "deadbeef",
            "confirm",
        ]
    );
}

/// An endpoint flag that was not given is not sent as an empty word — the
/// same rule `crate::secrets::run` follows, and the reason is the same: a
/// missing `--identity` must read as missing, not as `--identity ""`.
#[test]
fn an_endpoint_flag_that_is_absent_is_not_sent_at_all() {
    let endpoint = Endpoint::default();
    let words = enroll_words(&endpoint, "0011223344556677", "deadbeef");
    assert_eq!(
        words,
        ["--", "apply", "enroll-member", "0011223344556677", "--hex", "deadbeef", "confirm"]
    );
}

// ---- `render_device` → hex → `read_device` round trip ---------------------

/// **The property the whole module depends on**: what this renders and
/// hex-encodes is exactly what `rivetos_register::read_device` parses back —
/// the same record, field for field.
#[test]
fn a_rendered_device_hex_round_trips_through_read_device() {
    let mut written = a_device();
    written.location = Some("bench".to_owned());
    written.pin = Some("c0ffee".repeat(11)[..64].to_owned());
    written.listen = Some("0.0.0.0:7777".to_owned());
    written.trusts.push("k1".to_owned());

    let text = render_device(&written).expect("a plain record renders");
    let hex = rivet_core::package::key_material::to_hex(text.as_bytes());

    // The trip this verb's wire contract actually makes: hex out, hex back.
    let decoded = rivet_core::secret::bytes_from_hex(&hex).expect("this end's own hex reads back");
    let decoded_text = String::from_utf8(decoded).expect("a rendered record is UTF-8");
    assert_eq!(decoded_text, text);

    let path = PathBuf::from("/reg/devices/0011223344556677.toml");
    let read = rivetos_register::read_device(&path, &decoded_text)
        .expect("what this module sends is what read_device parses back");
    assert_eq!(read, written);
}

/// A device with none of the optional fields still round-trips — the empty
/// case, which is the one most likely to be reached first by a real member.
#[test]
fn a_minimal_device_hex_round_trips_too() {
    let written = a_device();
    let text = render_device(&written).expect("renders");
    let hex = rivet_core::package::key_material::to_hex(text.as_bytes());
    let decoded = rivet_core::secret::bytes_from_hex(&hex).expect("reads back");
    let decoded_text = String::from_utf8(decoded).expect("UTF-8");
    let path = PathBuf::from("/reg/devices/0011223344556677.toml");
    let read = rivetos_register::read_device(&path, &decoded_text).expect("parses back");
    assert_eq!(read, written);
}

// ---- `read_answer` ----------------------------------------------------------

fn success_answer(id: &str, bytes: usize) -> String {
    format!(r#"{{"id":"{id}","bytes":{bytes}}}"#)
}

/// The success case: the answer names this device and the byte count the
/// machine decoded matches what was sent.
#[test]
fn a_success_answer_naming_the_device_and_the_right_byte_count_is_accepted() {
    let report = read_answer(&success_answer("0011223344556677", 42), "0011223344556677", 42)
        .expect("a matching answer is accepted");
    assert!(report.contains("0011223344556677"), "{report}");
    assert!(report.contains("42"), "{report}");
}

/// **The refusal this checker exists for**: an answer about a different
/// device is never read as success about this one, whatever it says.
#[test]
fn an_answer_naming_a_different_device_is_refused() {
    let refused = read_answer(&success_answer("8899aabbccddeeff", 42), "0011223344556677", 42)
        .expect_err("an answer about another device");
    assert!(refused.contains("0011223344556677"), "{refused}");
    assert!(refused.contains("8899aabbccddeeff"), "{refused}");
}

/// **The byte-integrity check.** A truncated record would enroll "cleanly" and
/// fail later at a moment that says nothing about why — this is the earlier,
/// honest failure.
#[test]
fn a_byte_count_that_does_not_match_what_was_sent_is_refused() {
    let refused = read_answer(&success_answer("0011223344556677", 40), "0011223344556677", 42)
        .expect_err("a short answer");
    assert!(refused.contains("42"), "{refused}");
    assert!(refused.contains("40"), "{refused}");
    assert!(refused.contains("NOT the record"), "{refused}");
}

/// An answer with no `id` field at all cannot be told apart from an answer
/// about something else, so it is refused rather than assumed to be about the
/// device that was asked about.
#[test]
fn an_answer_with_no_id_field_is_refused() {
    let refused = read_answer(r#"{"bytes":42}"#, "0011223344556677", 42)
        .expect_err("no id in the answer");
    assert!(refused.contains("names no id"), "{refused}");
}

/// An answer with no `bytes` field says nothing about whether the record
/// arrived whole, so it is refused rather than trusted.
#[test]
fn an_answer_with_no_bytes_field_is_refused() {
    let refused = read_answer(r#"{"id":"0011223344556677"}"#, "0011223344556677", 42)
        .expect_err("no byte count in the answer");
    assert!(refused.contains("no byte count"), "{refused}");
}

/// Text that is not JSON at all is refused with what it actually said, rather
/// than a generic parse failure.
#[test]
fn an_answer_that_is_not_json_is_refused_and_quotes_what_it_said() {
    let refused = read_answer("machine refused: no such thing", "0011223344556677", 42)
        .expect_err("not JSON");
    assert!(refused.contains("machine refused: no such thing"), "{refused}");
}
