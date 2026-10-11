//! **Base64, `RFC 4648` §4** — one format, one implementation, both directions.
//!
//! # Why this module exists, and what it replaces
//!
//! Created 2026-09-01, when `cargo rivet screenshot` became the fourth thing in
//! this repository that needed base64 and the third that would have written it.
//! What was here before was not two implementations of one function — it was
//! **one format whose two halves lived in two unrelated modules, plus a third
//! copy of the alphabet in a test helper**: an encoder in
//! `crate::operation::file` for `get file`'s `content` on a file that is not
//! UTF-8, a decoder somewhere else entirely, and a second encoder in that
//! decoder's tests existing only to round-trip it.
//!
//! Nothing was wrong with any of them and each was carefully argued. The defect
//! is the arrangement: `ADR-011` asks who the authority for a fact is, and *"what
//! does base64 look like"* had three answers that agreed by inspection. The
//! encoder could have gained a line break, or the decoder a URL-safe character,
//! and the only thing that would have noticed is a reader comparing two files
//! that never mention each other.
//!
//! # Why it is written here rather than depended on
//!
//! `ADR-009` §3.4, carried across from `crate::operation::file`'s own note:
//! **`rivet-core` has no third-party dependencies at all** and its `Cargo.toml`
//! says so at the empty table, because `ADR-014` §3 makes the dependency graph
//! the gate rather than a convention. This is sixty lines of table lookup whose
//! output a reader can check by eye against the RFC, and the alternative is a
//! supply chain reaching the crate that is `root`'s login shell.
//!
//! # It is strict, and the strictness is a decision with a cost behind it
//!
//! Every rule below was ruled against a real decoder's source rather than
//! against a memory of the format, and each one is a refusal this build makes on
//! purpose:
//!
//! * **The alphabet is `RFC 4648` §4's** — `A–Z`, `a–z`, `0–9`, `+`, `/`, with
//!   `=` as padding — and nothing else.
//! * **The URL-safe variant's `-` and `_` are a DIFFERENT table and are refused
//!   here.** Accepting both would make this module's answer to *"is this
//!   base64"* wider than the answer given by whatever reads the bytes next,
//!   which is how a value gets vouched for here and rejected there.
//! * **Padding is structural, not decorative.** `=` appears only in the last one
//!   or two positions of the last group; `Zg==Zg==`, `Z===` and `====` are
//!   refused rather than read leniently.
//! * **An empty input is not an empty answer.** Every caller here wants bytes,
//!   and `Some(vec![])` would report *"a value, and it is nothing"* where the
//!   truth is that there was no value at all.
//!
//! **The merge did not loosen any of that**, and the tests below are the ones
//! that were on both sides before it, kept rather than rewritten.

/// The 64 characters, in `RFC 4648` §4's order. The URL-safe table is a
/// different one and deliberately does not appear in this file.
const ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as base64: padded, standard alphabet, no line breaks.
///
/// **No line breaks** because every consumer here is a JSON string or a single
/// field on one line, and `RFC 4648` §3.1 requires a specification that wants
/// wrapping to say so. None of ours does.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let mut quad = [0u8; 4];
        for (i, slot) in quad.iter_mut().enumerate() {
            *slot = ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize];
        }
        // The tail is padded, not truncated: a decoder is entitled to a length
        // that is a multiple of four, and `=` is what says how many bytes of
        // the last group are real.
        match chunk.len() {
            1 => {
                quad[2] = b'=';
                quad[3] = b'=';
            }
            2 => quad[3] = b'=',
            _ => {}
        }
        // Every element of `quad` is either an `ALPHABET` byte or `=`, so this
        // is ASCII by construction; the fallback exists because this crate
        // denies the lint that would let it be assumed.
        out.push_str(core::str::from_utf8(&quad).unwrap_or("...."));
    }
    out
}

/// Decode base64, or refuse.
///
/// `None` for anything this module does not accept — see the strictness rules
/// in the module documentation. **There is no partial answer and no lenient
/// mode**: a wrong decode does not fail, it succeeds, and hands back bytes that
/// look exactly like the right ones.
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, quad) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let mut sextets = [0u8; 4];
        let mut padding = 0usize;
        for (position, &byte) in quad.iter().enumerate() {
            match sextet(byte) {
                Some(value) => {
                    // `A=B` — a padding character with data after it — is not a
                    // shorter encoding of anything.
                    if padding > 0 {
                        return None;
                    }
                    sextets[position] = value;
                }
                // Padding is only ever the last one or two characters of the
                // last group. `==A=` and `A===` are refused.
                None if byte == b'=' && last && position >= 2 => padding += 1,
                None => return None,
            }
        }

        let packed = (u32::from(sextets[0]) << 18)
            | (u32::from(sextets[1]) << 12)
            | (u32::from(sextets[2]) << 6)
            | u32::from(sextets[3]);
        out.push((packed >> 16) as u8);
        if padding < 2 {
            out.push((packed >> 8) as u8);
        }
        if padding < 1 {
            out.push(packed as u8);
        }
    }
    Some(out)
}

/// One base64 character's six bits, or `None` for anything that is not one.
const fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **`RFC 4648` §10's own vectors, in both directions.**
    ///
    /// This test existed twice before the merge, once beside each half of the
    /// format, and the two are one test here.
    ///
    /// # What it now has to carry, said out loud
    ///
    /// The decoder's own test module used to hold a **separately written**
    /// encoder to check against, on `AGENTS.md` §4's rule that a round trip
    /// against one's own writer proves nothing about the format. That encoder is
    /// gone with the merge, so the property has to come from somewhere, and it
    /// comes from here: **both directions are pinned against strings the RFC
    /// published**, which neither function wrote and which a shared mistake in
    /// [`ALPHABET`] could not survive. That is the guard the separate encoder
    /// was standing in for.
    #[test]
    fn both_directions_agree_with_rfc_4648s_own_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in vectors {
            assert_eq!(encode(plain.as_bytes()), encoded, "encoding {plain:?}");
            if plain.is_empty() {
                // Refused rather than answered with nothing — see the module
                // documentation.
                assert_eq!(decode(encoded), None);
                continue;
            }
            assert_eq!(
                decode(encoded).as_deref(),
                Some(plain.as_bytes()),
                "decoding {encoded:?}"
            );
        }
    }

    /// **Everything `RFC 4648` §4's table refuses, refused here**, including the
    /// two URL-safe characters. The list is kept verbatim from the decoder's own
    /// tests: a tidy-up that quietly widened the alphabet would make this
    /// module say *"valid base64"* about a string the next reader of those bytes
    /// rejects, which is a disagreement neither side would report.
    #[test]
    fn it_refuses_everything_outside_the_standard_table() {
        for bad in [
            "",         // nothing to decode
            "Zm9vYm",   // not a multiple of four
            "Zm9v*mFy", // outside the alphabet
            "Zm9vYmF-", // the URL-safe alphabet is a different table
            "Zm9vYmF_", // and so is its other character
            "Zg==Zg==", // padding with data after it
            "====",     // padding and nothing else
            "Z===",     // only the last two positions may be padding
            "Zm 9v",    // a space is not base64
        ] {
            assert_eq!(decode(bad), None, "{bad:?} was decoded");
        }
    }

    /// Every byte value encodes, and the padding is right for all three tail
    /// lengths. A table that was wrong at `0x3f`, or an encoder that treated a
    /// byte above `0x7f` as signed, would pass the vectors above and fail here.
    ///
    /// Kept from `crate::operation::file`'s tests.
    #[test]
    fn every_byte_value_encodes_to_a_well_formed_quad() {
        let all: Vec<u8> = (0..=255u8).collect();
        for take in [all.len() - 2, all.len() - 1, all.len()] {
            let out = encode(&all[..take]);
            assert_eq!(out.len() % 4, 0, "not a multiple of four at {take}");
            assert!(
                out.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='),
                "a byte outside the alphabet at {take}"
            );
            let pad = out.bytes().rev().take_while(|b| *b == b'=').count();
            assert_eq!(pad, (3 - take % 3) % 3, "wrong padding at {take}");
        }
    }

    /// **Every byte survives the round trip**, at all three tail lengths.
    ///
    /// New with the merge, and it is what makes the two halves being in one
    /// module worth something rather than merely tidier: before this, nothing in
    /// the repository ever encoded a byte and decoded it back, because the two
    /// functions had no caller in common. `get file` now hands a captured frame
    /// to `cargo rivet screenshot` and that is exactly this round trip, over a
    /// control plane.
    ///
    /// It is not a substitute for the vectors above and does not pretend to be —
    /// a codec with a scrambled alphabet round-trips perfectly. The external
    /// vectors pin the table; this pins the arithmetic.
    #[test]
    fn every_byte_value_round_trips_at_every_tail_length() {
        let all: Vec<u8> = (0..=255u8).collect();
        for take in [all.len() - 2, all.len() - 1, all.len()] {
            let source = &all[..take];
            assert_eq!(
                decode(&encode(source)).as_deref(),
                Some(source),
                "at {take} bytes"
            );
        }
    }

    /// A padded tail decodes to the length the padding says, and not to a
    /// multiple of three with zeroes on the end — which is what an encoder's
    /// zero-fill would leave behind if the decoder ignored the `=`.
    #[test]
    fn padding_decides_the_decoded_length() {
        assert_eq!(decode("Zg==").map(|v| v.len()), Some(1));
        assert_eq!(decode("Zm8=").map(|v| v.len()), Some(2));
        assert_eq!(decode("Zm9v").map(|v| v.len()), Some(3));
    }
}
