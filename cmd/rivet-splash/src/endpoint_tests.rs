//! What the endpoint line must say, and what it must never say.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

#[test]
fn the_line_is_the_address_and_the_port() {
    assert_eq!(
        line(Ipv4Addr::new(192, 168, 50, 66), 7777),
        "192.168.50.66:7777"
    );
}

/// A port that is not the default is drawn as it is. The document decides it
/// (`ADR-035` §7.2) and this program is told; inventing 7777 here would make
/// the screen lie about a machine somebody deliberately moved.
#[test]
fn a_port_that_is_not_the_default_is_shown_as_it_is() {
    assert_eq!(line(Ipv4Addr::new(10, 0, 0, 2), 9999), "10.0.0.2:9999");
}

/// **The address discovery runs against the real machine**, so this asserts
/// only what is true of any machine: it either finds something usable or finds
/// nothing, and never something it should have rejected.
#[test]
fn whatever_is_found_is_never_loopback_or_link_local() {
    if let Some(address) = current_address() {
        assert!(
            !address.is_loopback(),
            "loopback was offered as this machine's address: {address}"
        );
        assert!(
            !address.is_link_local(),
            "a link-local address was offered — that is what an interface has when DHCP FAILED, \
             and printing it sends an operator somewhere that will not answer: {address}"
        );
        assert!(!address.is_unspecified(), "0.0.0.0 was offered: {address}");
    }
}

/// Called twice in a row it must answer the same thing. A splash that redrew
/// because the discovery is unstable would flicker for no reason.
#[test]
fn discovery_is_stable_across_two_calls() {
    assert_eq!(current_address(), current_address());
}
