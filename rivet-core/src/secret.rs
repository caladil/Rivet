//! **Secrets at rest** — `ADR-020`, ruled Accepted 2026-08-22, ruled *build it*
//! 2026-08-29 (§11), and built 2026-08-30.
//!
//! # What this is, after the rulings narrowed it
//!
//! `ADR-020` as written is four mechanisms: a redacting type (§4), a store
//! separate from the document (§5), envelope encryption behind a `KeyProvider`
//! port (§6), and a board-bound key provider (§7).
//!
//! **§11.2.3 removed the middle two on 2026-08-30**, and the reasoning is worth
//! carrying here rather than leaving a reader to find it: the maintainer asked
//! how the closest relative solves this, a real Raspberry Pi OS box was
//! measured, and it does not — a wireless passphrase lives in a `0600`
//! root-owned file, in plain text, with no `psk-flags`, no encryption at rest
//! and no TPM. The card is the whole boundary.
//!
//! The maintainer's conclusion (`.rulings/` R-363) was that obfuscation is
//! enough: it stops the casual attacker, nothing can stop a determined one who
//! holds the board, and so the design does not go further.
//!
//! So there is **no envelope and no `KeyProvider`**. §6 and §7 describe a design
//! that was considered and not built, and §11.2.3 says so in the ADR.
//!
//! # What survives, and it is the half §4 called the highest-value
//!
//! * **Secrecy is a property of the TYPE** (§4). [`Secret`] redacts in `Debug`
//!   and has no `Display` at all, so a secret cannot reach a log, a plan, an
//!   `export` or an error message by being formatted — which is the failure
//!   that actually happens, and it happens by accident.
//! * **The store is separate** (§5). The document says which secrets exist; the
//!   values live under [`crate::state::SECRETS`], root-owned. `rivet export`
//!   therefore reconstitutes a machine except its secrets, which is what makes
//!   an export safe to commit.
//! * **The value is hex** (§11.2.3.1). Not protection — an encoding, and it is
//!   never to be called anything else. What it buys is the accidental case: a
//!   card grepped for a known password does not hit, and a document pasted into
//!   a bug report does not leak a credential in the clear.
//!
//! # `secrecy` and `zeroize` rather than a hand-rolled wrapper
//!
//! `ADR-020` §4 named both, on `ADR-009`'s ladder: *"existing crates do the
//! work … rung 1"*. They are what [`Secret`] is built from — `zeroize` clears
//! the bytes when the value drops, which a newtype around `String` would not.

use secrecy::{ExposeSecret, SecretBox};

/// A secret value, which redacts itself.
///
/// **There is no `Display`, deliberately.** `Debug` redacting is not enough on
/// its own: `format!("{value}")` would still print it, and that is the shape
/// the accident takes. The only way to reach the bytes is
/// [`Secret::expose`], which is a name somebody has to type and a reviewer can
/// search for.
pub struct Secret(SecretBox<String>);

impl Secret {
    /// Take ownership of a secret value.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(SecretBox::new(Box::new(value)))
    }

    /// **The one way to the bytes**, named so that reaching for it is visible.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// The value as it is stored on the machine — `ADR-020` §11.2.3.1's hex.
    ///
    /// **Encoding, not encryption**, and this method's name is the only place
    /// that has to be right about it. It stops the casual and accidental case
    /// and nothing else, which is the level §11.2.3 ruled.
    #[must_use]
    pub fn to_stored(&self) -> String {
        self.expose().bytes().fold(String::new(), |mut out, byte| {
            use core::fmt::Write as _;
            // Writing to a String cannot fail; the result is dropped rather
            // than unwrapped so this cannot become a panic in a supervisor.
            let _ = write!(out, "{byte:02x}");
            out
        })
    }

    /// Read a value back from its stored form.
    ///
    /// **Refuses rather than guessing.** Odd length, a non-hex character, or
    /// bytes that are not UTF-8 all mean the file is not what this wrote, and
    /// a secret reconstructed from a damaged file would be a wrong credential
    /// presented as a right one.
    pub fn from_stored(text: &str) -> Result<Self, StoredError> {
        String::from_utf8(bytes_from_hex(text)?)
            .map(Self::new)
            .map_err(|_| StoredError::NotText)
    }
}

/// Hexadecimal characters back to the bytes they encode.
///
/// **Extracted from [`Secret::from_stored`] 2026-09-01**, unchanged, so that
/// `apply install-secret` decodes hex with the decoder this project already
/// has and already tests rather than a fourth copy of one. That subject's
/// content may be **any** bytes — an Ed25519 seed is not text — so the UTF-8
/// step stays in `from_stored`, which is the caller that needs it.
///
/// Leading and trailing whitespace is trimmed and nothing internal is: a value
/// that arrived with a newline on it is the ordinary case, and a value with a
/// space in the middle is a value somebody mangled.
///
/// # Errors
///
/// [`StoredError::NotHex`] for an odd length or a character that is not a
/// hexadecimal digit. **Never a partial decode** — half a credential is a wrong
/// credential presented as a right one.
pub fn bytes_from_hex(text: &str) -> Result<Vec<u8>, StoredError> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err(StoredError::NotHex);
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks(2) {
        let hi = pair.first().copied().and_then(hex_digit).ok_or(StoredError::NotHex)?;
        let lo = pair.get(1).copied().and_then(hex_digit).ok_or(StoredError::NotHex)?;
        bytes.push(hi << 4 | lo);
    }
    Ok(bytes)
}

/// Why a stored secret could not be read back.
#[derive(Debug, PartialEq, Eq)]
pub enum StoredError {
    /// The file is not an even number of hexadecimal characters.
    NotHex,
    /// It decoded, and the bytes are not text.
    NotText,
}

impl core::fmt::Display for StoredError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotHex => f.write_str(
                "a stored secret must be an even number of hexadecimal characters and this is \
                 not — ADR-020 §11.2.3.1. It is hex so that a card grepped for a known password \
                 does not hit, which is ENCODING and not encryption",
            ),
            Self::NotText => f.write_str(
                "a stored secret decoded from hex and the bytes are not text. Nothing this build \
                 writes produces that, so the file was written by something else or is damaged",
            ),
        }
    }
}

/// One hexadecimal character's value.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        // **Upper case is accepted on the way in and never produced.**
        // Postel's rule applied narrowly: a person retyping a value should not
        // be refused over case, and `to_stored` still has exactly one output
        // for one input, which is what a canonical form has to mean.
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl core::fmt::Debug for Secret {
    /// **`<secret>` and nothing else** — `ADR-020` §4.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("<secret>")
    }
}

/// Where one named secret lives — `ADR-020` §5.
///
/// **Composed here and nowhere else**, `ADR-011`: the directory was a string
/// literal in five places before 2026-08-30, which is how two of them come to
/// disagree.
#[must_use]
pub fn path_of(name: &str) -> String {
    format!("{}/{name}", crate::state::SECRETS)
}

#[cfg(test)]
#[path = "secret_tests.rs"]
mod tests;
