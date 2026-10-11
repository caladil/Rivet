//! The derivation, against vectors nobody in this project computed.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// **Known answers, and the point is that the answers came from elsewhere.** A
/// derivation that agrees with these joins a real access point; one that does
/// not, does not, and no amount of internal consistency would reveal it.
///
/// The first two are IEEE 802.11i's own published vectors, quoted in every WPA
/// implementation's test suite. The third is **cross-checked against a second
/// implementation** rather than quoted: Python's `hashlib.pbkdf2_hmac`, which is
/// OpenSSL, produces it for the same inputs.
///
/// That distinction is recorded because it was earned. A third value was first
/// written here from memory, and it was wrong — the test failed, and the failure
/// could have been the code. Two independent implementations agreeing is what
/// settled it, and a "known answer" somebody remembered is not one.
///
/// `AGENTS.md` §8.3 — published fixtures, not anybody's network.
#[test]
fn the_ieee_test_vectors_are_reproduced() {
    for (passphrase, ssid, expected) in [
        (
            "password",
            "IEEE",
            "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e",
        ),
        (
            "ThisIsAPassword",
            "ThisIsASSID",
            "0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af",
        ),
        (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
            "4fd16ee24bd1d8f9e7ebd86cbd802d0b3acfd23cb08de414da4e1690e474b857",
        ),
    ] {
        // The third vector's passphrase is 64 bytes, which is outside 802.11's
        // range for a passphrase — the standard publishes it anyway as a
        // derivation vector. Skip the length rule for it by deriving directly.
        let got = if passphrase.len() > PASSPHRASE_MAX_BYTES {
            let mut pmk = [0u8; PMK_BYTES];
            pbkdf2::pbkdf2_hmac::<sha1::Sha1>(
                passphrase.as_bytes(),
                ssid.as_bytes(),
                ITERATIONS,
                &mut pmk,
            );
            pmk
        } else {
            derive(ssid, passphrase).expect("a valid passphrase")
        };
        assert_eq!(
            to_hex(&got),
            expected,
            "the derivation disagrees with IEEE's own vector for {ssid:?}"
        );
    }
}

/// **The SSID is the salt**, which is the whole reason a derivation needs both.
/// One passphrase on two networks must not give one key.
#[test]
fn the_same_passphrase_on_two_networks_gives_two_keys() {
    let a = derive("network-one", "a-passphrase").expect("valid");
    let b = derive("network-two", "a-passphrase").expect("valid");
    assert_ne!(a, b, "the SSID was not used as the salt");
}

#[test]
fn the_derivation_is_deterministic() {
    assert_eq!(
        derive("a-network", "a-passphrase").expect("valid"),
        derive("a-network", "a-passphrase").expect("valid")
    );
}

/// 802.11's own range. **63 and not 64**: a 64-character value is how a PMK is
/// written, and the standard leaves the shorter range to passphrases so the two
/// cannot be confused.
#[test]
fn the_passphrase_length_rule_is_the_standards() {
    assert!(derive("n", &"x".repeat(PASSPHRASE_MIN_BYTES)).is_ok());
    assert!(derive("n", &"x".repeat(PASSPHRASE_MAX_BYTES)).is_ok());
    assert!(derive("n", &"x".repeat(PASSPHRASE_MIN_BYTES - 1)).is_err());
    assert!(
        derive("n", &"x".repeat(64)).is_err(),
        "64 bytes is a PMK, not a passphrase"
    );
}

/// **The refusal reports a LENGTH and never the passphrase.** This is the one
/// error in the project whose careless form would print a credential
/// (`ADR-020` §4).
#[test]
fn a_refusal_never_carries_the_passphrase() {
    // A fixture, not a real secret — `AGENTS.md` §8.3.
    let too_short = "hunter2";
    let refused = derive("a-network", too_short).expect_err("7 bytes");
    let said = refused.to_string();
    assert!(!said.contains("hunter"), "the refusal echoed the passphrase: {said}");
    assert!(said.contains('7'), "it should say how long it was: {said}");
}

/// The hex form is what `SET_NETWORK <id> psk <hex>` takes — lowercase, 64
/// characters, unquoted. A quoted value would be a PASSPHRASE the supplicant
/// derives from again, producing a different key and failing to associate.
#[test]
fn the_hex_form_is_what_the_supplicant_takes() {
    let pmk = derive("a-network", "a-passphrase").expect("valid");
    let hex = to_hex(&pmk);
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

/// A passphrase with non-ASCII bytes derives rather than panicking. 802.11
/// says nothing about the encoding; the bytes are what is hashed.
#[test]
fn a_passphrase_with_non_ascii_derives() {
    let pass = "pässwörd-ünïcode";
    assert!(pass.len() > PASSPHRASE_MIN_BYTES);
    assert!(derive("a-network", pass).is_ok());
}
