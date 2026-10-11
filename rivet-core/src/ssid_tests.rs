//! What an SSID must accept, and what a radio could not carry.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

#[test]
fn an_ordinary_name_is_accepted_and_reads_back() {
    let ssid = Ssid::new("an-example-network").expect("a valid name");
    assert_eq!(ssid.as_str(), "an-example-network");
    assert_eq!(ssid.to_string(), "an-example-network");
}

/// **The wildcard is not a network.** A zero-length SSID is what a client sends
/// to ask what is out there; a document asking to join it is asking for
/// something that cannot be satisfied.
#[test]
fn an_empty_name_is_refused() {
    assert!(Ssid::new("").is_err());
}

#[test]
fn exactly_thirty_two_bytes_is_the_longest_accepted() {
    let at_limit = "x".repeat(SSID_MAX_BYTES);
    assert_eq!(at_limit.len(), 32);
    assert!(Ssid::new(&at_limit).is_ok());
    assert!(Ssid::new(&"x".repeat(SSID_MAX_BYTES + 1)).is_err());
}

/// **The limit is OCTETS and not characters**, which is the part a
/// `chars().count()` would get wrong — and it would get it wrong in the
/// direction that produces a frame no radio can send.
#[test]
fn the_limit_counts_bytes_rather_than_characters() {
    // Eleven characters, thirty-three bytes: three octets each.
    let wide = "\u{4f60}".repeat(11);
    assert_eq!(wide.chars().count(), 11);
    assert_eq!(wide.len(), 33);
    assert!(
        Ssid::new(&wide).is_err(),
        "a name of 11 characters and 33 bytes was accepted; 802.11 counts octets"
    );

    // And ten of the same characters — thirty bytes — is fine.
    let fits = "\u{4f60}".repeat(10);
    assert_eq!(fits.len(), 30);
    assert!(Ssid::new(&fits).is_ok());
}

/// Names people actually have. None of these is special to the type and that is
/// the point: an SSID is a name and almost nothing is forbidden in one.
#[test]
fn the_awkward_names_real_networks_have_are_accepted() {
    for name in [
        "My WiFi",                  // a space
        "café",                     // non-ASCII, 5 bytes
        "  leading and trailing  ", // whitespace is part of the name
        "#hashtag",
        "2.4GHz",
        "-",
    ] {
        assert!(Ssid::new(name).is_ok(), "{name:?} was refused");
    }
}

/// **The rule can be reported without the name.** `ADR-020` §4: the document an
/// SSID comes from is the one that must never be echoed carelessly, and a
/// refusal quoting the wrong line of it is one edit away.
#[test]
fn the_rule_is_a_static_string_that_carries_no_input() {
    let refused = Ssid::new("").expect_err("empty");
    let text = refused.to_string();
    assert!(text.contains("32"), "{text}");
    // It reports a LENGTH, never the offending bytes.
    let long = Ssid::new(&"s".repeat(64)).expect_err("too long").to_string();
    assert!(!long.contains("ssssss"), "the refusal echoed its input: {long}");
    assert!(long.contains("64"), "it should say how long it was: {long}");
}
