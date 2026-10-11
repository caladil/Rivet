//! What a stored key must be, and what a missing one must say.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A PMK-shaped value. `AGENTS.md` §8.3 — a fixture, not a key to anything.
const NOT_A_REAL_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn wlan0() -> InterfaceName {
    InterfaceName::new("wlan0").expect("a valid name")
}

#[test]
fn the_path_is_per_interface_and_under_the_secret_store() {
    let path = path_of(&wlan0());
    assert_eq!(path, "/state/secrets/wlan0.psk");
    assert!(path.starts_with(rivet_core::state::SECRETS));
}

/// Two interfaces do not share a key — `eth0` wired while `wlan0` holds one is
/// the ordinary machine.
#[test]
fn two_interfaces_have_two_paths() {
    let other = InterfaceName::new("wlan1").expect("valid");
    assert_ne!(path_of(&wlan0()), path_of(&other));
}

#[test]
fn a_well_formed_key_is_read() {
    let key = read(Some(format!("{NOT_A_REAL_KEY}\n")), "/p").expect("a valid key");
    assert_eq!(key.expose(), NOT_A_REAL_KEY);
}

/// **A key that is read cannot be printed by accident** — the audit of
/// 2026-09-03's LOW list, *"PSK returned as a plain `String`"*.
///
/// `ADR-020` §11.2.3 threw away the envelope and the key provider and kept the
/// half it called the highest-value: secrecy is a property of the TYPE. This
/// module returned the one credential on a Rivet that the ADR is actually about
/// as a bare `String`, so a `format!` anywhere on `rivet-netd`'s wireless path
/// would have put a pairwise master key in the journal — and §4's point is that
/// this failure happens **by accident**.
///
/// `Secret` has no `Display` at all, so the only thing left to check is that
/// the `Debug` a diagnostic reaches for redacts.
#[test]
fn a_key_that_was_read_does_not_appear_in_a_diagnostic() {
    let key = read(Some(NOT_A_REAL_KEY.to_string()), "/p").expect("a valid key");
    let printed = format!("{key:?}");
    assert!(
        !printed.contains(NOT_A_REAL_KEY),
        "the key reached a Debug format: {printed}"
    );
    // And a fragment of it, so this cannot pass on a formatting difference.
    assert!(!printed.contains("00112233"), "{printed}");
    // Exposing it is still how a caller reaches the wire form.
    assert_eq!(key.expose(), NOT_A_REAL_KEY);
}

/// A person pasting from `wpa_passphrase` may paste upper case. It reads, and
/// is normalised — the supplicant takes either, but one form out means one
/// thing to compare.
#[test]
fn upper_case_reads_and_lower_case_is_what_comes_out() {
    let key = read(Some(NOT_A_REAL_KEY.to_ascii_uppercase()), "/p").expect("valid");
    assert_eq!(key.expose(), NOT_A_REAL_KEY);
}

/// **The most likely mistake is pasting the passphrase**, and it must not look
/// like a wrong password — that sends an operator to the router for a problem
/// on the card.
#[test]
fn a_passphrase_where_a_key_belongs_is_refused_and_says_so() {
    let refused = read(Some("correct-horse-battery-staple".to_string()), "/p")
        .expect_err("a passphrase is not a key");
    let said = refused.to_string();
    assert!(said.contains("A passphrase is not a key"), "{said}");
    assert!(said.contains("derived"), "{said}");
}

/// **The refusal reports a LENGTH and never the contents.** `ADR-020` §4: this
/// is a file whose whole purpose is to hold a credential.
#[test]
fn a_refusal_never_quotes_the_file() {
    let refused = read(Some("deadbeef".repeat(9)), "/p").expect_err("72 characters");
    let said = refused.to_string();
    assert!(!said.contains("deadbeef"), "the refusal quoted the file: {said}");
    assert!(said.contains("72"), "{said}");
}

#[test]
fn something_that_is_the_right_length_and_not_hex_is_refused() {
    let refused = read(Some("z".repeat(PMK_HEX_LENGTH)), "/p").expect_err("not hex");
    assert!(matches!(refused, PskError::NotAPmk { .. }), "{refused:?}");
}

/// **An absent key explains the split**, because the reason it is absent is
/// that the document deliberately cannot hold it.
#[test]
fn an_absent_key_explains_why_it_is_not_in_the_document() {
    let refused = read(None, "/state/secrets/wlan0.psk").expect_err("absent");
    let said = refused.to_string();
    assert!(said.contains("/state/secrets/wlan0.psk"), "{said}");
    assert!(said.contains("ADR-007 §7.3"), "{said}");
    assert!(said.contains("Nothing was joined") || said.contains("nothing was joined"), "{said}");
}
