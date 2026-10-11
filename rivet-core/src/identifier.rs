//! **An opaque, stable identifier** — sixteen lowercase hexadecimal characters.
//!
//! `ADR-025` §4.1 ruled the principle for a fleet member:
//!
//! > A stable opaque identifier is authoritative for a logical device. The
//! > name is a mutable label.
//!
//! **The principle is not about devices.** It is about any thing whose *name*
//! an operator will want to change without the change meaning *a different
//! thing*. A device is the first such thing Rivet had; a repository is the
//! second, ruled 2026-08-26 on the maintainer's account of having had to rename
//! a repository more than once (`.rulings/` R-357), which is `ADR-025` §4.1
//! arriving from the other direction.
//!
//! # Why this is in the core and not in the register
//!
//! It was `cmd/rivet-fleet`'s `register` module until 2026-08-26, and moved on
//! the day the second consumer appeared — `rivet-config`, which has to *refuse*
//! an identifier a document carries, and cannot depend on a binary crate.
//!
//! **One grammar and two subjects, not two grammars.** A device id and a
//! repository id are the same sixteen characters and the same rule; two
//! spellings of that rule would eventually differ, and the first sign of it
//! would be a register that accepted an id a document refused.
//!
//! # What it deliberately is not
//!
//! **Not a secret, and not unguessable.** `ADR-025` §4.1 asks for *stable* and
//! *opaque*, and nothing anywhere treats knowing an identifier as authority. An
//! id appears in a document on a card and in a directory on a disk. What it
//! must not do is collide.

/// How long an identifier is, in characters.
///
/// Sixteen lowercase hexadecimal characters — **64 bits.** `ADR-025` §4.1
/// requires that it be stable and opaque and says nothing about its length; 64
/// bits is chosen so that a fleet that grows by three orders of magnitude
/// beyond `PLAN.md` §9's list still has no realistic chance of a collision, and
/// so that the string is short enough for a human to compare two of them by
/// eye.
pub const CHARS: usize = 16;

/// The rule, in the words a refusal uses.
///
/// **Published rather than described**, so that the sentence an operator reads
/// and the check the code performs cannot drift — `the_published_rule_matches`
/// is what holds them together.
pub const RULE: &str = "an identifier is exactly 16 lowercase hexadecimal characters. It is minted \
                        rather than chosen, and it is opaque: it is not a secret and knowing one \
                        is not authority (ADR-025 4.1)";

/// Whether `text` is an identifier this system would mint.
///
/// Used to refuse a hand-written one **before** it is filed, rather than after
/// something references it. Uppercase is refused rather than folded: two
/// spellings of one identifier is the exact failure the identifier exists to
/// prevent.
#[must_use]
pub fn is_identifier(text: &str) -> bool {
    text.len() == CHARS && text.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Mint an identifier from entropy the caller supplies.
///
/// **The randomness is the caller's**, which is what makes this testable at
/// all: `ADR-025` §4.1 requires the identifier be stable and opaque, and both
/// properties are about the *string*, not about where the bytes came from.
#[must_use]
pub fn from_entropy(entropy: &[u8]) -> String {
    let mut out = String::with_capacity(CHARS);
    for byte in entropy.iter().take(CHARS / 2) {
        out.push_str(&format!("{byte:02x}"));
    }
    // A short slice is padded rather than producing a short identifier, because
    // a caller that handed over too little entropy has made a mistake and a
    // silently shorter id would be filed and referenced before anybody noticed.
    while out.len() < CHARS {
        out.push('0');
    }
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn what_is_minted_is_what_is_accepted() {
        let id = from_entropy(&[0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04]);
        assert_eq!(id, "deadbeef01020304");
        assert!(is_identifier(&id));
    }

    /// Too little entropy is padded rather than shortened — a short id would be
    /// filed and referenced before anybody noticed.
    #[test]
    fn too_little_entropy_is_padded_rather_than_shortened() {
        let id = from_entropy(&[0xaa]);
        assert_eq!(id.len(), CHARS);
        assert!(is_identifier(&id));
        assert!(id.starts_with("aa"));
    }

    /// **Uppercase is refused rather than folded.** Two spellings of one
    /// identifier is the exact failure the identifier exists to prevent.
    #[test]
    fn a_second_spelling_of_one_identifier_is_refused() {
        assert!(is_identifier("deadbeef01020304"));
        assert!(!is_identifier("DEADBEEF01020304"));
        assert!(!is_identifier("DeadBeef01020304"));
    }

    #[test]
    fn anything_that_is_not_sixteen_hexadecimal_characters_is_refused() {
        for bad in [
            "",
            "deadbeef",                  // too short
            "deadbeef010203040",         // too long
            "deadbeef0102030g",          // not hexadecimal
            "deadbeef-0102030",          // a separator
            " deadbeef01020304",
        ] {
            assert!(!is_identifier(bad), "{bad:?}");
        }
    }

    /// The published rule describes what is enforced, rather than what somebody
    /// remembered it as.
    #[test]
    fn the_published_rule_matches_what_is_checked() {
        assert!(RULE.contains(&CHARS.to_string()));
        assert!(RULE.contains("lowercase"));
        assert!(RULE.contains("hexadecimal"));
    }
}
