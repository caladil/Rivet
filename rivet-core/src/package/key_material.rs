//! **32 bytes of key material, from a file a person produced.**
//!
//! `ADR-029` §7.1 rules that a public key is 32 raw bytes with no envelope —
//! *"not a certificate, not a SubjectPublicKeyInfo, not PEM"* — so there is no
//! format here to get wrong before a signature check. What there is instead is
//! a **file**, and a file is produced by whatever tool the person had:
//!
//! ```text
//! head -c 32 /dev/urandom > system.seed        # raw bytes
//! cargo rivet key public system.seed         # 64 hex characters
//! ```
//!
//! Both are 32 bytes of key material and refusing either would send somebody to
//! `xxd` for no reason. So this accepts **exactly two** encodings and nothing
//! else: 32 raw bytes, or 64 hex characters with any surrounding whitespace.
//!
//! # Why this is in `rivet-core` rather than in the tool that first needed it
//!
//! It was `cmd/cargo-rivet`'s, as `read_32`, and `cmd/rivet-image` grew a
//! second reader that accepted **only** raw bytes. Two answers to *what does a
//! key file look like* is `ADR-011` §3 exactly, and the failure it produces is
//! nasty in a specific way: a hex file that one tool signs with and the other
//! refuses to trust, whose refusal reads as a length complaint rather than as
//! the two tools disagreeing.
//!
//! Nothing here does I/O. The caller reads the file; this reads the bytes.
//!
//! **Length is checked exactly.** A 31-byte or 33-byte file is a refusal, never
//! a pad and never a truncation, because quietly adjusting key material to fit
//! is how a key becomes a different key.

use crate::package::format::PUBLIC_KEY_BYTES;

/// How many bytes of key material there are. The same 32 for a public key and
/// for an Ed25519 private seed — `ADR-029` §7.1.
pub const KEY_BYTES: usize = PUBLIC_KEY_BYTES;

/// How many hex characters spell [`KEY_BYTES`].
pub const HEX_CHARS: usize = KEY_BYTES * 2;

/// Why some bytes are not key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMaterialError {
    /// Neither 32 raw bytes nor 64 hex characters.
    WrongLength {
        /// What was actually there, so a refusal can say it.
        bytes: usize,
    },
    /// The right *length* for hex and not hex — which is the interesting
    /// failure, because it is what a truncated paste or a stray character
    /// produces, and a length complaint would send the reader looking in the
    /// wrong place.
    NotHex {
        /// Which character, counting from 0 after whitespace is removed.
        at: usize,
    },
}

impl core::fmt::Display for KeyMaterialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongLength { bytes } => write!(
                f,
                "it is {bytes} bytes; key material is exactly {KEY_BYTES} raw bytes or \
                 {HEX_CHARS} hex characters (ADR-029 §7.1: a key has no envelope to strip)"
            ),
            Self::NotHex { at } => write!(
                f,
                "it is {HEX_CHARS} characters long, so it looks like hex, and character {at} \
                 is not a hex digit"
            ),
        }
    }
}

impl std::error::Error for KeyMaterialError {}

/// Read 32 bytes of key material out of a file's contents.
pub fn parse(bytes: &[u8]) -> Result<[u8; KEY_BYTES], KeyMaterialError> {
    // Raw first, and deliberately: a file of exactly 32 bytes IS a key, whatever
    // those bytes happen to look like. Sniffing for hex first would make a
    // 32-byte key whose bytes are all ASCII hex parse as something else, which
    // is a silent wrong answer rather than a refusal.
    if let Ok(raw) = <[u8; KEY_BYTES]>::try_from(bytes) {
        return Ok(raw);
    }

    let hex: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if hex.len() != HEX_CHARS {
        return Err(KeyMaterialError::WrongLength { bytes: bytes.len() });
    }

    let mut out = [0u8; KEY_BYTES];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = digit(hex[index * 2]).ok_or(KeyMaterialError::NotHex { at: index * 2 })?;
        let low =
            digit(hex[index * 2 + 1]).ok_or(KeyMaterialError::NotHex { at: index * 2 + 1 })?;
        *slot = high * 16 + low;
    }
    Ok(out)
}

/// One hex digit, either case.
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Bytes as lower-case hex. The encoding [`parse`] reads back, and the one a
/// public key is written down in.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(digit_char(byte >> 4)));
        out.push(char::from(digit_char(byte & 0x0f)));
    }
    out
}

const fn digit_char(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + nibble - 10
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// RFC 8032 §7.1's first test vector's public key, so the bytes in this
    /// module came from somewhere other than this project — `AGENTS.md` §4.
    const KEY: [u8; KEY_BYTES] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];

    #[test]
    fn thirty_two_raw_bytes_are_the_key_they_are() {
        assert_eq!(parse(&KEY), Ok(KEY));
    }

    #[test]
    fn sixty_four_hex_characters_are_the_same_key() {
        let hex = to_hex(&KEY);
        assert_eq!(hex.len(), HEX_CHARS);
        assert_eq!(parse(hex.as_bytes()), Ok(KEY));
    }

    /// A key pasted out of a terminal arrives with a newline, and often with
    /// indentation. Neither is a reason to refuse it.
    #[test]
    fn whitespace_around_and_inside_the_hex_is_not_part_of_it() {
        for decorated in [
            format!("{}\n", to_hex(&KEY)),
            format!("  {}  ", to_hex(&KEY)),
            format!("{}\r\n", to_hex(&KEY)),
            to_hex(&KEY)
                .as_bytes()
                .chunks(8)
                .map(|c| String::from_utf8_lossy(c).into_owned())
                .collect::<Vec<_>>()
                .join("\n"),
        ] {
            assert_eq!(parse(decorated.as_bytes()), Ok(KEY), "{decorated:?}");
        }
    }

    /// Upper case is what half the tools in the world print.
    #[test]
    fn hex_is_read_in_either_case_and_written_in_one() {
        assert_eq!(parse(to_hex(&KEY).to_uppercase().as_bytes()), Ok(KEY));
        assert_eq!(to_hex(&KEY), to_hex(&KEY).to_lowercase());
    }

    /// **A key is never padded and never truncated.** One byte short is a
    /// different key, and a tool that quietly fixed it would produce an image
    /// that trusts something nobody holds.
    #[test]
    fn a_file_of_nearly_the_right_length_is_refused_rather_than_adjusted() {
        for length in [0, 1, 31, 33, 63, 65, 128] {
            let bytes = vec![0x41u8; length];
            assert_eq!(
                parse(&bytes),
                Err(KeyMaterialError::WrongLength { bytes: length }),
                "{length} bytes was accepted"
            );
        }
    }

    /// The length is right and the content is not, which is what a bad paste
    /// looks like — so the refusal says *character 12 is not a hex digit*
    /// rather than complaining about a length that is correct.
    #[test]
    fn hex_length_with_a_bad_character_says_which_character() {
        let mut hex = to_hex(&KEY).into_bytes();
        hex[12] = b'z';
        assert_eq!(parse(&hex), Err(KeyMaterialError::NotHex { at: 12 }));
    }

    /// Raw wins over hex at 32 bytes, and this is the case that proves it: 32
    /// ASCII hex characters are a *key*, not half a hex encoding.
    #[test]
    fn thirty_two_bytes_that_look_like_hex_are_still_thirty_two_raw_bytes() {
        let looks_like_hex = b"0123456789abcdef0123456789abcdef";
        assert_eq!(parse(looks_like_hex), Ok(*looks_like_hex));
    }

    /// The refusals are read by a person at the moment a build failed, so they
    /// carry the numbers rather than a shrug — and neither ships with the
    /// collapsed indentation `package::error` now has a guard for.
    #[test]
    fn every_refusal_names_a_number_and_carries_no_run_of_spaces() {
        for error in [
            KeyMaterialError::WrongLength { bytes: 31 },
            KeyMaterialError::NotHex { at: 12 },
        ] {
            let message = error.to_string();
            assert!(!message.is_empty());
            assert!(!message.contains("  "), "{message}");
            assert!(
                message.contains("32") || message.contains("64"),
                "{message}"
            );
        }
    }
}
