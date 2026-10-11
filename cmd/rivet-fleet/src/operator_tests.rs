//! What the operator's identity resolves to, and what it refuses.
//!
//! **Nothing here opens a socket.** Every case is decided before a connection,
//! which is the property `crate::operator` is built for: a refusal that arrives
//! after a payload has been composed is a refusal that cost something.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use super::*;
use crate::register::Interface;

/// 64 hex characters, which is what a control-plane key is.
const REGISTERED: &str = "11111111111111111111111111111111111111111111111111111111111111aa";
const TYPED: &str = "22222222222222222222222222222222222222222222222222222222222222bb";

fn device_at(address: Option<&str>) -> Device {
    let mut device = Device {
        id: "0011223344556677".to_owned(),
        name: "kitchen-pi".to_owned(),
        pin: Some(REGISTERED.to_owned()),
        ..Device::default()
    };
    if let Some(address) = address {
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                address: Some(address.to_owned()),
                ..Interface::default()
            },
        );
    }
    device
}

/// **The register is the ordinary source of a pin**, which is the whole reason
/// `rivet-fleet` needs `--pin` less than `cargo rivet` does: `ADR-034` §3.5 has
/// it hold the public half of every device key already.
#[test]
fn the_pin_comes_from_the_register_when_no_flag_was_given() {
    let operator = Operator {
        identity: Some(PathBuf::from("./id")),
        pin: None,
    };
    assert_eq!(operator.pin_for(&device_at(None)), Some(REGISTERED));
}

/// **And `--pin` overrides it.** A machine whose identity has been replaced has
/// a record that is briefly wrong, and *the record is wrong* has to be
/// survivable without editing the record first.
#[test]
fn a_typed_pin_overrides_the_one_the_register_holds() {
    let operator = Operator {
        identity: Some(PathBuf::from("./id")),
        pin: Some(TYPED.to_owned()),
    };
    assert_eq!(operator.pin_for(&device_at(None)), Some(TYPED));
}

/// A device the register has no key for, and no flag either, has no pin — which
/// is `None` rather than an empty string, because an empty pin would be offered
/// to `parse_pins` and refused three layers further down.
#[test]
fn a_device_with_no_recorded_key_and_no_flag_has_no_pin() {
    let mut device = device_at(None);
    device.pin = None;
    assert_eq!(Operator::default().pin_for(&device), None);
    device.pin = Some(String::new());
    assert_eq!(Operator::default().pin_for(&device), None);
}

/// **The port is named, and it is `rivet-config`'s.** A cold agent found three
/// control-plane ports in three places on 2026-08-29; this composes the address
/// from the record and the port from the crate that owns `[control-plane]`, so
/// it cannot become a fourth spelling.
#[test]
fn the_host_is_the_records_address_and_the_owning_crates_port() {
    let host = control_plane_host(&device_at(Some("192.168.50.66/24"))).expect("an address");
    assert_eq!(
        host,
        format!("192.168.50.66:{}", rivet_config::control_plane::DEFAULT_PORT)
    );
}

/// The record's own refusal comes through unchanged. A device with no address
/// is `crate::member`'s sentence and not a second one written here.
#[test]
fn a_record_with_no_address_refuses_in_the_registers_own_words() {
    let refused = control_plane_host(&device_at(None)).expect_err("no address");
    assert!(refused.contains("carries no address"), "{refused}");
}

/// **No identity is the state a person has to be told about**, because it is
/// the state every fresh run is in and there is no other route to a machine.
#[test]
fn a_run_with_no_identity_refuses_and_says_how_to_get_one() {
    let operator = Operator::default();
    assert!(!operator.holds_identity());
    let refused = operator
        .open(&device_at(Some("192.168.50.66/24")))
        .err()
        .expect("no identity");
    assert!(refused.contains("--identity"), "{refused}");
    assert!(refused.contains("cargo rivet identity new"), "{refused}");
    // And it says why there is no fallback, rather than leaving a reader to
    // go looking for one — ADR-035.
    assert!(refused.contains("ADR-035"), "{refused}");
}

/// **A device with no pin is refused by name**, and the refusal says which
/// command records one. `ADR-025` §2: the pin is the whole of the trust model,
/// so this is not a case that may be defaulted through.
#[test]
fn a_device_with_no_pin_is_refused_before_a_key_is_read() {
    let mut device = device_at(Some("192.168.50.66/24"));
    device.pin = None;
    let operator = Operator {
        identity: Some(PathBuf::from("./no-such-identity-directory")),
        pin: None,
    };
    let refused = operator.open(&device).err().expect("no pin");
    assert!(refused.contains("kitchen-pi"), "{refused}");
    assert!(refused.contains("add identity"), "{refused}");
    // The identity was never read: the pin check comes first, so the sentence
    // is about the pin and not about a missing directory.
    assert!(!refused.contains("no-such-identity-directory"), "{refused}");
}

/// **The endpoint is composed without opening anything**, so the three values
/// the record and the flags resolve can be read before a connection exists.
#[test]
fn the_endpoint_is_composed_from_the_record_and_the_flags() {
    let operator = Operator {
        identity: Some(PathBuf::from("./id")),
        pin: None,
    };
    let endpoint = operator
        .endpoint_for(&device_at(Some("10.0.0.2/24")))
        .expect("an address");
    assert_eq!(
        endpoint.host.as_deref(),
        Some(format!("10.0.0.2:{}", rivet_config::control_plane::DEFAULT_PORT).as_str())
    );
    assert_eq!(endpoint.identity, Some(PathBuf::from("./id")));
    assert_eq!(endpoint.pin.as_deref(), Some(REGISTERED));
}
