//! `<kind>` — the token that says how the metadata is to be read.
//!
//! `ADR-029` §11 rules **one container for all three artifact kinds**, on
//! `ADR-028` §2's last row: *"the verifier is the most security-sensitive parser
//! on the machine. Three formats means three verifiers, each with a third of the
//! scrutiny."* What differs between an application, an extension and an update
//! payload is *"the **interpretation** of the metadata, not its shape"*.
//!
//! # Why this is not an enum
//!
//! ~~Because two of the three cases are **not the maintainer's to have decided
//! yet**, and an enum would decide them by existing:~~
//!
//! * ~~`ADR-027` §9 item 2 — the application suffix, and by extension its
//!   token.~~
//! * ~~`ADR-027` §9 items 3–4 — whether applications or extensions carry
//!   `<platform>` at all.~~
//!
//! ~~`AGENTS.md` §1 puts those with the maintainer. So [`Kind`] is a validated
//! 16-byte ASCII token with exactly one constant on it — [`Kind::update`],
//! which `ADR-029` §8 *did* rule — and a reader compares against whatever token
//! it was asked about. Adding `Kind::APPLICATION` is a one-line change on the
//! day it is ruled, and until then nothing in this crate has an opinion.~~
//!
//! **That day was 2026-08-24** — the maintainer's 2026-08-24 ruling, written into
//! `ADR-027` §9 items 2–4. One suffix for all three kinds, `<kind>` inside the
//! file saying which, and **no `<platform>` component for an application**,
//! because an application is architecture-shaped rather than firmware-shaped.
//! So there are two constants now, [`Kind::update`] and [`Kind::application`],
//! and the paragraph above is struck rather than deleted because `AGENTS.md` §2
//! asks the record to show its own history.
//!
//! **The type is still not an enum, and the reason survived the ruling.** One
//! of the three kinds — `ADR-016`'s extension — is still unruled, and the
//! stronger reason is `read`'s: a token read out of somebody else's header is
//! text this build may not recognise, and a closed enum would make "a kind I do
//! not implement" indistinguishable from a malformed field. A reader compares
//! against the token it was asked about and refuses everything else with
//! [`PackageError::WrongKind`], which names both.
//!
//! # The suffix is `.rvt` for every kind — `ADR-029` §8
//!
//! ```text
//! rivet-0.2.0-aarch64-rpi4-update.rvt
//! ```
//!
//! One suffix, and the kind token distinguishes. The suffix is a naming
//! convention rather than a fact about the bytes, so it lives here as a constant
//! and nothing in the reader consults it: a payload renamed to `.bin` verifies
//! exactly as well, and a `.rvt` whose header says otherwise is refused exactly
//! as hard.

use crate::package::error::PackageError;
use crate::package::format::{KIND_BYTES, TextFault, read_text};

/// `ADR-029` §8: *"Suffix: `.rvt`, one suffix for all three artifact kinds,
/// with `<kind>` saying which."* Includes the dot, as `ADR-027` §3's grammar
/// writes it.
pub const SUFFIX: &str = ".rvt";

/// Header bytes 160..176 (`format::OFF_KIND`..`format::OFF_BUILD_EPOCH`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kind(String);

impl Kind {
    /// `ADR-029` §8, ruled 2026-08-23. *"`card` and `update` are both words about
    /// what the artifact **does**"* — and `AGENTS.md` §10.2 already fixes
    /// **update payload** as vocabulary.
    ///
    /// A function rather than a `const` because [`Kind`] owns a `String`; the
    /// alternative was a `&'static str` newtype, which cannot hold a token read
    /// out of a header.
    #[must_use]
    pub fn update() -> Self {
        Self("update".to_string())
    }

    /// The maintainer's 2026-08-24 ruling, written into `ADR-027` §9 item 2.
    ///
    /// `ADR-015` §2's word for the artifact, unchanged: *"**Application** …
    /// `/state/app/<name>` … sandboxed … isolated"*. `app` was considered and
    /// refused for the reason `ADR-029` §8 refused `payload` — the tokens are
    /// short because the field is 16 bytes, not because the vocabulary is, and
    /// `AGENTS.md` §10.2 spells the term out everywhere else.
    ///
    /// Eleven bytes of sixteen. What it *means* is
    /// [`crate::package::application`].
    #[must_use]
    pub fn application() -> Self {
        Self("application".to_string())
    }

    /// **Ruled 2026-08-31** —
    /// `docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §4.4's
    /// first bullet: *"`kind = "library"` … Same `.rvt` container, same
    /// signature verification, same trust store — a library is a payload, not a
    /// new distribution channel."*
    ///
    /// The third token, and it arrives on exactly the terms the two above it
    /// did: a module in this directory and a token in a header, **not a second
    /// verifier**. What it *means* is [`crate::package::library`].
    ///
    /// **`ADR-016`'s extension is still unruled and is still not presumed
    /// here.** The paragraph above about this type not being an enum has now
    /// survived two kinds being added rather than one, which is the argument
    /// for it made a second time and by a different case.
    #[must_use]
    pub fn library() -> Self {
        Self("library".to_string())
    }

    /// **`R-177`, 2026-09-13** — a card image, wrapped as an `.rvt` so it can
    /// travel through the fleet repository (`ADR-052` §7, rewritten by this
    /// change) exactly like every other artifact. `cargo rivet image` emits
    /// this kind, and a key must be entitled to `card` to sign it, which the
    /// trust store grants to `rivet-system` alone (`ADR-028` §5).
    ///
    /// **Unlike every other kind, a `card` payload's one member may be
    /// compressed** — `R-177.1`, zstd, `rivet-core::package::read::COMPRESSION_ZSTD`.
    /// That is the whole reason a card image needs its own token rather than
    /// riding on `update`'s: `ADR-029` §13/R-107 keeps every kind a *board*
    /// installs uncompressed, on purpose, and a card image is the one kind that
    /// is never installed by a board at all — it is decompressed once, on the
    /// workstation, when provisioning writes it. Folding it into `update`
    /// would either compress a kind R-107 forbids compressing or leave `card`
    /// unable to compress at all; a fourth token keeps the two rules from
    /// touching each other.
    ///
    /// Fifth kind at this counting; there have been four `Kind::*()` ceremonies
    /// now and the reason a fifth still needs one is unchanged: a token read out
    /// of somebody else's header is text this build may not recognise.
    #[must_use]
    pub fn card() -> Self {
        Self("card".to_string())
    }

    /// **`ADR-061` §3, `.rulings/` R-222.** An Android APK, held by a fleet
    /// server so `gantry` can hand it to a phone — never installed on a Rivet
    /// machine. What it *means* is [`crate::package::apk`].
    ///
    /// Sixth kind at this counting, and it asks nothing of the verifier, the
    /// builder or the layout — `ADR-029` §11's claim, made a third time.
    #[must_use]
    pub fn apk() -> Self {
        Self("apk".to_string())
    }

    /// Validate a token. Refuses anything the header could not carry back
    /// unchanged, so that a token and the bytes that encode it are the same
    /// thing.
    pub fn parse(text: &str) -> Result<Self, PackageError> {
        if text.len() > KIND_BYTES {
            return Err(PackageError::TextTooLong {
                field: "kind",
                len: text.len(),
                capacity: KIND_BYTES,
            });
        }
        if text.is_empty() {
            return Err(PackageError::MissingText { field: "kind" });
        }
        let mut field = [0u8; KIND_BYTES];
        field[..text.len()].copy_from_slice(text.as_bytes());
        match read_text(&field) {
            Ok(Some(round_tripped)) if round_tripped == text => Ok(Self(text.to_string())),
            Ok(_) => Err(PackageError::MalformedText {
                field: "kind",
                fault: TextFault::Empty,
            }),
            Err(fault) => Err(PackageError::MalformedText {
                field: "kind",
                fault,
            }),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Kind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_five_ruled_tokens_are_update_application_library_card_and_apk() {
        assert_eq!(Kind::update().as_str(), "update");
        assert_eq!(Kind::parse("update"), Ok(Kind::update()));
        assert_eq!(Kind::application().as_str(), "application");
        assert_eq!(Kind::parse("application"), Ok(Kind::application()));
        assert_eq!(Kind::library().as_str(), "library");
        assert_eq!(Kind::parse("library"), Ok(Kind::library()));
        assert_eq!(Kind::card().as_str(), "card");
        assert_eq!(Kind::parse("card"), Ok(Kind::card()));
        assert_eq!(Kind::apk().as_str(), "apk");
        assert_eq!(Kind::parse("apk"), Ok(Kind::apk()));
        assert_ne!(Kind::update(), Kind::application());
        assert_ne!(Kind::application(), Kind::library());
        assert_ne!(Kind::update(), Kind::library());
        assert_ne!(Kind::card(), Kind::update());
        assert_ne!(Kind::card(), Kind::application());
        assert_ne!(Kind::card(), Kind::library());
        assert_ne!(Kind::apk(), Kind::update());
        assert_ne!(Kind::apk(), Kind::application());
        assert_ne!(Kind::apk(), Kind::library());
        assert_ne!(Kind::apk(), Kind::card());
    }

    /// Every token must survive the field it is written into, and
    /// `application` is the longest by far — so the one that would notice first
    /// if [`KIND_BYTES`] ever shrank.
    #[test]
    fn every_token_fits_the_field() {
        assert!(Kind::update().as_str().len() <= KIND_BYTES);
        assert!(Kind::application().as_str().len() <= KIND_BYTES);
        assert!(Kind::library().as_str().len() <= KIND_BYTES);
        assert!(Kind::card().as_str().len() <= KIND_BYTES);
        assert!(Kind::apk().as_str().len() <= KIND_BYTES);
    }

    #[test]
    fn the_suffix_is_one_for_every_kind() {
        assert_eq!(SUFFIX, ".rvt");
        // ADR-029 §8's own example, composed the way ADR-027 §3's grammar does.
        let name = format!("rivet-0.2.0-aarch64-rpi4-{}{SUFFIX}", Kind::update());
        assert_eq!(name, "rivet-0.2.0-aarch64-rpi4-update.rvt");

        // the maintainer's 2026-08-24 ruling: the same suffix, and NO platform component.
        // The absence is the ruling, so it is asserted rather than assumed.
        let app = format!("hello-0.1.0-aarch64-{}{SUFFIX}", Kind::application());
        assert_eq!(app, "hello-0.1.0-aarch64-application.rvt");
        assert!(!app.contains("rpi4"));
    }

    #[test]
    fn a_token_that_would_not_survive_the_header_is_refused() {
        assert!(matches!(
            Kind::parse("0123456789abcdefg"),
            Err(PackageError::TextTooLong { .. })
        ));
        assert!(matches!(
            Kind::parse(""),
            Err(PackageError::MissingText { .. })
        ));
        assert!(matches!(
            Kind::parse("up date"),
            Err(PackageError::MalformedText { .. })
        ));
        assert!(matches!(
            Kind::parse("up\0date"),
            Err(PackageError::MalformedText { .. })
        ));
    }

    /// A sixteen-character token fills the field exactly and needs no NUL, so
    /// the boundary is legal rather than one short.
    #[test]
    fn a_token_may_fill_the_field() {
        assert_eq!(
            Kind::parse("0123456789abcdef").map(|k| k.as_str().len()),
            Ok(16)
        );
    }
}
