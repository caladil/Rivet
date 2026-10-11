//! **The marks a reading carries** — trend arrows and connection marks.
//!
//! `SPEC-04` §5.5, §5.6 and §7 rule 4. Built 2026-09-02, and the thing worth
//! saying first is that **it needed no decision.**
//!
//! §11 (5) listed *"whether Aura carries its own asset or the image's icon font
//! grows"* as an open question for months. It was not one:
//! `Aura/assets/MaterialSymbols-Aura.ttf` already existed, already subset to
//! exactly these seven codepoints, and `cpp/src/main.cpp:209` already loaded
//! it. The renderer already shipped three faces the same way. Asking the
//! maintainer what font to use, when his own repository had answered it, is
//! what turned a task into a question.
//!
//! # Seven glyphs, and they are the C++'s own numbers
//!
//! Copied from `cpp/src/widgets.hpp:615-621` rather than looked up. A Material
//! Symbols codepoint is not derivable from a name, and a wrong one does not
//! fail — it draws **a different picture**, which is the `605` failure
//! `crate::layout`'s note is about.
//!
//! # Why the face is 2 KB
//!
//! Because it is already a subset. The full Material Symbols is megabytes; this
//! carries seven outlines and nothing else, the same way `AuraRain.ttf`
//! carries the 105 codepoints Digital Rain draws from.
//!
//! # A missing file costs the marks and nothing else
//!
//! The C++'s own rule, kept: *"a missing file costs the marks and nothing else,
//! so the card still shows readings on a box where the asset never landed."*
//! [`Icons::load`] returns `None` and every caller draws the reading without
//! its mark — which is `SPEC-04` §7 rule 5's lesser failure, and the one this
//! renderer has been taking all along for want of the face.

use aura_card::{State, Trend};

/// `LoadFontEx(..., 96, ...)` — the atlas size the C++ builds this face at.
///
/// **Not a taste and not a guess.** `crate::typeface` reproduces raylib's
/// metrics, and every one of them is computed at `Font::baseSize` and then
/// scaled; declaring the wrong one measures wrongly at every other size. 96 is
/// what `main.cpp:209` passes.
pub const BASE_SIZE: f32 = 96.0;

/// The file, shipped as an `ADR-015` §3 package asset beside the three type
/// faces.
pub const FONT: &str = "MaterialSymbols-Aura.ttf";

/// `kMarkSize = kValueSize * 0.58` and `kArrowSize = kValueSize * 0.56`, as
/// fractions — the caller multiplies by whatever its own value size is, so the
/// ratio survives a composition the C++ never had.
pub const MARK_FRACTION: f32 = 0.58;
pub const ARROW_FRACTION: f32 = 0.56;

/// `kGapMark` — the connection mark to the reading beside it.
pub const MARK_GAP: f32 = 24.0;

/// **The mark for a reading that cannot be fully believed** — `SPEC-04` §7
/// rule 4.
///
/// `None` for [`State::Ok`], which is the ordinary case and carries no mark at
/// all. `stale` takes the same question mark as `connecting`: both mean *this
/// is not current*, and the C++ has one glyph for that.
///
/// **The alert is red because something is wrong; the question is amber because
/// nothing is, yet** — the original renderer's reading card says so in its own
/// comment, and it is the reason these
/// return a role rather than a colour. `SPEC-04` §5.1: no colour crosses the
/// wire and none is chosen here.
#[must_use]
pub const fn mark(state: State) -> Option<(char, aura_card::Role)> {
    match state {
        State::Ok => None,
        // `android_wifi_3_bar_question` — 0xEF17.
        State::Stale | State::Connecting => Some(('\u{EF17}', aura_card::Role::Warn)),
        // `android_wifi_3_bar_alert` — 0xEF1B.
        State::Unavailable => Some(('\u{EF1B}', aura_card::Role::Alarm)),
    }
}

/// **The arrow for a trend** — `SPEC-04` §5.5.
///
/// The five codepoints are `widgets.hpp:617-621`, and the mapping is the one
/// `aura_card::Trend`'s own documentation already states glyph by glyph.
#[must_use]
pub const fn arrow(trend: Trend) -> char {
    match trend {
        // `arrow_downward`
        Trend::FallingFast => '\u{E5DB}',
        // `south_east`
        Trend::Falling => '\u{F1E4}',
        // `arrow_forward`
        Trend::Flat => '\u{E5C8}',
        // `north_east`
        Trend::Rising => '\u{F1E1}',
        // `arrow_upward`
        Trend::RisingFast => '\u{E5D8}',
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **Every trend has its own arrow**, and no two share one. Five readings
    /// that all drew the same picture would be five readings a person cannot
    /// tell apart, which is worse than no arrow at all.
    #[test]
    fn the_five_trends_map_to_five_different_glyphs() {
        let mut seen = Vec::new();
        for trend in Trend::ALL {
            let glyph = arrow(trend);
            assert!(!seen.contains(&glyph), "{trend:?} draws the same glyph as another trend");
            seen.push(glyph);
        }
        assert_eq!(seen.len(), 5);
    }

    /// `ok` carries no mark; every other state does, and *wrong* is a different
    /// glyph from *not yet*.
    #[test]
    fn only_a_reading_that_cannot_be_believed_carries_a_mark() {
        assert_eq!(mark(State::Ok), None, "an ordinary reading must not be decorated");
        let stale = mark(State::Stale).expect("stale carries a mark");
        let connecting = mark(State::Connecting).expect("connecting carries a mark");
        let unavailable = mark(State::Unavailable).expect("unavailable carries a mark");
        // `stale` and `connecting` share a glyph deliberately: both mean *this
        // is not current*, and the C++ has one mark for that.
        assert_eq!(stale, connecting);
        assert_ne!(
            unavailable, connecting,
            "gone-for-good must not look like not-yet — that is the distinction \
             SPEC-04 §5.4 exists to carry"
        );
        assert_eq!(unavailable.1, aura_card::Role::Alarm, "wrong is the fault colour");
        assert_eq!(connecting.1, aura_card::Role::Warn, "a cold start is not a fault");
    }

    /// **The face actually contains all seven**, which is the one thing a
    /// codepoint table cannot assert about itself.
    ///
    /// A glyph missing from a subset does not fail: `fontdue` rasterises
    /// nothing and the mark silently disappears, or — in the C++'s atlas — the
    /// FIRST glyph is drawn instead, which is how a reading of `6.5` reached
    /// the wall as `605`. This is the test that says the file matches the table.
    #[test]
    fn the_shipped_face_carries_every_glyph_this_module_names() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/MaterialSymbols-Aura.ttf");
        let face = crate::typeface::Typeface::load(path, BASE_SIZE)
            .expect("the icon face this package ships should load");
        let mut wanted: Vec<char> = Trend::ALL.into_iter().map(arrow).collect();
        for state in State::ALL {
            if let Some((glyph, _)) = mark(state)
                && !wanted.contains(&glyph)
            {
                wanted.push(glyph);
            }
        }
        assert_eq!(wanted.len(), 7, "seven glyphs, as the C++ subsets");
        for glyph in wanted {
            let ink = face.ink(glyph, 96.0);
            assert!(
                ink.width() > 0.0 && ink.height() > 0.0,
                "U+{:04X} rasterises to nothing — it is not in the subset, and a mark that \
                 is absent is the failure SPEC-04 §7 rule 5 tolerates only when the whole \
                 FACE is missing",
                glyph as u32
            );
        }
    }
}
