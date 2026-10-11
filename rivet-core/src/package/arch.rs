//! **What this machine executes, as its image recorded it** — the `<arch>`
//! token an artifact's header is compared against.
//!
//! `ADR-027` §3: `<arch>` answers *"will this binary execute"*, and the
//! vocabulary is `aarch64`, `armv6` and `x86_64` —
//! [`crate::artifact::ARCH_TOKENS`].
//!
//! # Why a file in `/system` and not the compiler
//!
//! **The compiler speaks a different vocabulary.** `std::env::consts::ARCH` is
//! `aarch64` on a Pi 4 and **`arm`** on a Pi Zero W, where `ADR-027` §3 says
//! `armv6`. A machine that compared artifacts against the compiler's word would
//! refuse every application on an `armv6` board, and the refusal would read as
//! the artifact's fault. A table from the compiler's word to the record's would
//! be a guess about which of several `arm` targets built this binary — `arm` is
//! armv6 and armv7 alike — and `rivet_remote::name::arch_for` already refuses to
//! make that guess from a triple for exactly that reason.
//!
//! **The image knows**, because the image build chose the target: the card
//! build compiles every binary for one triple and knows which `ADR-027` token
//! that triple is. So `cmd/rivet-image` writes the token here, beside the trust
//! store and for the trust store's reasons (`ADR-028` §5): a read-only file
//! in `/system`, replaced only wholesale by a verified update, and a fact a
//! machine can be asked for rather than a constant nobody can read.
//!
//! # Who reads it
//!
//! **Both points at which an artifact is admitted** — `cmd/rivet-mcpd`'s store
//! door and `cmd/rivet-deploy` — through
//! [`crate::package::verify::architecture`], so the comparison has one
//! implementation (`PLAN-Feature-Set-2026-09-12` §3, `ADR-052` §7).
//!
//! # A machine with no record admits nothing
//!
//! Refused rather than defaulted, for [`crate::package::verify::trust_set`]'s
//! reason: a record that could not be read is not a record that says
//! `aarch64`, and treating it as one is how a wrong-architecture application
//! gets installed on the one machine whose image forgot to say. Every image
//! built since 2026-09-12 carries one, and the binaries that read it arrive in
//! the same `/system` as the file — so a machine whose readers are new enough
//! to ask is a machine whose image is new enough to answer.

use crate::artifact::ARCH_TOKENS;

/// Where the image records this machine's `<arch>`.
///
/// Under `/system` beside [`crate::package::trust_store::PATH`], for that
/// constant's reasons, and declared in this crate because `cmd/rivet-image`
/// writes it while `cmd/rivet-deploy` and `cmd/rivet-mcpd` read it — three
/// binaries that cannot see one another (`ADR-011` §3).
pub const PATH: &str = "/system/arch";

/// The mode the image writes it with: read by everyone, written by nobody.
pub const MODE: u32 = 0o444;

/// Why a machine's architecture record yields no token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchRefusal {
    /// The file could not be read.
    Unreadable(String),
    /// It was read and is not one of `ADR-027` §3's tokens.
    Unknown(String),
}

impl core::fmt::Display for ArchRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(
                f,
                "cannot read this machine's architecture record at {PATH}: {why}. The image \
                 writes it (cmd/rivet-image --arch); without it this machine cannot say which \
                 artifacts it can execute, and admits none rather than guessing"
            ),
            Self::Unknown(found) => write!(
                f,
                "this machine's architecture record at {PATH} says {found:?}, which is not one \
                 of ADR-027 §3's tokens ({}). It is refused rather than trusted: a record this \
                 build cannot read is not a record that names an architecture",
                ARCH_TOKENS.join(", ")
            ),
        }
    }
}

/// The file's bytes for `token`, or `None` if `token` is not in
/// [`ARCH_TOKENS`].
///
/// One trailing newline, so the file reads cleanly with `cat` — and [`read`]
/// accepts exactly that shape and nothing looser.
#[must_use]
pub fn render(token: &str) -> Option<Vec<u8>> {
    ARCH_TOKENS
        .contains(&token)
        .then(|| format!("{token}\n").into_bytes())
}

/// The token, from the bytes of [`PATH`] **as read at the moment of use**.
///
/// # Errors
///
/// [`ArchRefusal::Unreadable`] when the read failed, [`ArchRefusal::Unknown`]
/// when the bytes are not a known token followed by at most one newline.
pub fn read<E: core::fmt::Display>(stored: Result<Vec<u8>, E>) -> Result<String, ArchRefusal> {
    let bytes = stored.map_err(|e| ArchRefusal::Unreadable(e.to_string()))?;
    let text = String::from_utf8_lossy(&bytes);
    let token = text.strip_suffix('\n').unwrap_or(&text);
    if ARCH_TOKENS.contains(&token) {
        Ok(token.to_string())
    } else {
        Err(ArchRefusal::Unknown(text.into_owned()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn every_token_round_trips_through_the_file() {
        for token in ARCH_TOKENS {
            let bytes = render(token).expect("a known token renders");
            assert_eq!(read::<&str>(Ok(bytes)), Ok((*token).to_string()));
        }
    }

    /// **The compiler's word is not a token**, which is the whole reason this
    /// file exists: `arm` is what `std::env::consts::ARCH` says on an `armv6`
    /// board, and a record saying it is refused rather than read as `armv6`.
    #[test]
    fn the_compilers_word_for_an_armv6_board_is_not_a_token() {
        assert_eq!(render("arm"), None);
        assert!(matches!(
            read::<&str>(Ok(b"arm\n".to_vec())),
            Err(ArchRefusal::Unknown(_))
        ));
    }

    #[test]
    fn an_unreadable_record_is_refused_and_says_where_it_looked() {
        let refusal = read(Err("No such file or directory")).expect_err("absent");
        assert!(matches!(refusal, ArchRefusal::Unreadable(_)));
        assert!(refusal.to_string().contains(PATH), "{refusal}");
    }

    /// Exactly one newline, and nothing looser: a record with padding or a
    /// second line is one something other than the image wrote.
    #[test]
    fn a_record_with_anything_but_one_trailing_newline_is_refused() {
        for bytes in [&b"aarch64\n\n"[..], b" aarch64\n", b"aarch64 ", b"", b"\n"] {
            assert!(
                read::<&str>(Ok(bytes.to_vec())).is_err(),
                "{:?} was accepted",
                String::from_utf8_lossy(bytes)
            );
        }
        assert_eq!(read::<&str>(Ok(b"aarch64".to_vec())), Ok("aarch64".to_string()));
    }
}
