//! What `ADR-020`'s surviving half must do, and the accident it exists to
//! prevent asserted rather than described.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A passphrase-shaped value. `AGENTS.md` §8.3 — a fixture, not a real secret.
const NOT_A_REAL_SECRET: &str = "correct-horse-battery-staple";

/// **The failure this type exists to prevent**, and it is the one that actually
/// happens: a value reaches a log, a plan or an error message because somebody
/// formatted the struct that held it.
#[test]
fn debug_redacts_and_never_carries_the_value() {
    let secret = Secret::new(NOT_A_REAL_SECRET.to_string());
    let shown = format!("{secret:?}");
    assert_eq!(shown, "<secret>");
    assert!(
        !shown.contains("horse"),
        "the value reached a Debug rendering: {shown}"
    );
}

/// And it survives being nested, which is how it will actually be formatted —
/// nobody writes `{secret:?}`, they write `{whole_struct:?}`.
#[test]
fn a_secret_inside_something_else_is_still_redacted() {
    #[derive(Debug)]
    #[allow(dead_code, reason = "read only through Debug, which is the point")]
    struct Wireless {
        ssid: String,
        psk: Secret,
    }
    let shown = format!(
        "{:?}",
        Wireless {
            ssid: "an-ssid".to_string(),
            psk: Secret::new(NOT_A_REAL_SECRET.to_string()),
        }
    );
    assert!(shown.contains("an-ssid"), "{shown}");
    assert!(!shown.contains("horse"), "the value leaked: {shown}");
}

/// The one way to the bytes, and it is named so that reaching for it is
/// visible to a reviewer.
#[test]
fn expose_is_the_only_route_and_it_returns_the_value() {
    let secret = Secret::new(NOT_A_REAL_SECRET.to_string());
    assert_eq!(secret.expose(), NOT_A_REAL_SECRET);
}

/// **A round trip, which is the property the store depends on.**
#[test]
fn a_secret_survives_being_stored_and_read_back() {
    let secret = Secret::new(NOT_A_REAL_SECRET.to_string());
    let stored = secret.to_stored();
    let back = Secret::from_stored(&stored).expect("what we wrote must read back");
    assert_eq!(back.expose(), NOT_A_REAL_SECRET);
}

/// **The point of the hex, stated as a test**: the stored form does not contain
/// the value, so grepping a card for a known password does not hit it.
#[test]
fn the_stored_form_does_not_contain_the_plain_value() {
    let stored = Secret::new(NOT_A_REAL_SECRET.to_string()).to_stored();
    assert!(!stored.contains("horse"), "{stored}");
    assert!(
        stored.chars().all(|c| c.is_ascii_hexdigit()),
        "the stored form must be hex and nothing else: {stored}"
    );
}

/// It is ENCODING and never encryption, and this is what that means: anybody
/// with the file gets the value back. The test exists so nobody later mistakes
/// the hex for protection.
#[test]
fn the_stored_form_is_reversible_by_anybody_which_is_the_whole_claim() {
    let stored = Secret::new("hunter2".to_string()).to_stored();
    assert_eq!(stored, "68756e74657232");
    assert_eq!(
        Secret::from_stored("68756e74657232").expect("valid").expose(),
        "hunter2"
    );
}

/// An empty secret is a value, not an absence. Refusing it here would put a
/// policy decision in an encoder.
#[test]
fn an_empty_secret_round_trips() {
    let stored = Secret::new(String::new()).to_stored();
    assert_eq!(stored, "");
    assert_eq!(Secret::from_stored("").expect("valid").expose(), "");
}

/// Non-ASCII must survive, because a passphrase can hold any character a person
/// can type.
#[test]
fn a_passphrase_with_non_ascii_survives_the_round_trip() {
    let value = "pässwörd-with-ünïcode-ß";
    let stored = Secret::new(value.to_string()).to_stored();
    assert_eq!(
        Secret::from_stored(&stored).expect("valid").expose(),
        value
    );
}

/// **Refuse rather than guess**, `AGENTS.md` §5. A damaged file must not
/// produce a secret, because a wrong credential presented as a right one is
/// worse than no credential.
#[test]
fn a_file_that_is_not_hex_is_refused_rather_than_repaired() {
    assert_eq!(Secret::from_stored("abc").unwrap_err(), StoredError::NotHex);
    assert_eq!(
        Secret::from_stored("zz").unwrap_err(),
        StoredError::NotHex,
        "a non-hex character must be refused"
    );
    assert_eq!(
        Secret::from_stored("6875 6e74").unwrap_err(),
        StoredError::NotHex,
        "internal whitespace is not hex"
    );
}

/// Hex that decodes to bytes which are not text. Nothing this build writes
/// produces that, so it means the file came from somewhere else.
#[test]
fn hex_that_is_not_text_is_refused_by_name() {
    assert_eq!(
        Secret::from_stored("ff").unwrap_err(),
        StoredError::NotText
    );
}

/// Surrounding whitespace is trimmed, because a file an operator touched with
/// an editor gains a trailing newline and refusing that would be refusing the
/// repair break-glass exists for.
#[test]
fn a_trailing_newline_does_not_break_a_stored_secret() {
    assert_eq!(
        Secret::from_stored("  68756e74657232\n").expect("valid").expose(),
        "hunter2"
    );
}

/// Upper case reads, and is never written — one input, one output, which is
/// what canonical has to mean.
#[test]
fn upper_case_hex_reads_and_lower_case_is_what_is_produced() {
    assert_eq!(
        Secret::from_stored("68756E74657232").expect("valid").expose(),
        "hunter2"
    );
    assert_eq!(Secret::new("hunter2".to_string()).to_stored(), "68756e74657232");
}

/// **One authority for where a secret lives** — `ADR-011`, and the directory
/// was a literal in five places before this.
#[test]
fn the_path_is_composed_from_the_one_constant() {
    assert_eq!(path_of("wireless-psk"), "/state/secrets/wireless-psk");
    assert!(path_of("x").starts_with(crate::state::SECRETS));
}
