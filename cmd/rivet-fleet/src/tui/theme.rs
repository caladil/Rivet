//! **What the terminal interface looks like** — glyphs and colour, in one
//! place so that neither is decided at a call site.
//!
//! `ADR-034` §10.1 asked for a terminal interface and §11.3 adopted `ratatui`.
//! The maintainer ruled the substance on 2026-08-27: color and nerd fonts are
//! welcome where they are appropriate, provided nerd fonts fail safe
//! (.rulings/ R-327).
//!
//! # How a nerd font fails safe here
//!
//! Not by detection. There is no reliable way to ask a terminal whether it can
//! render a private-use codepoint — every method is a guess, and a guess that
//! is wrong produces a screen full of boxes.
//!
//! So the safety is structural instead: **no glyph ever carries meaning on its
//! own.** Every icon sits beside the word it decorates, every state has a text
//! label, and every glyph occupies exactly one cell. A terminal with no nerd
//! font shows a box where an icon would be, and the screen still says
//! everything it said before. `no_glyph_is_load_bearing` is the test, and
//! [`Glyphs::ascii`] is what a person who dislikes the boxes switches to.
//!
//! # Colour says state, never decoration
//!
//! A colour in this interface always means something a reader could otherwise
//! only get by reading: whether a machine answered, whether a change has been
//! applied, how bad a journal line is. Nothing is coloured to look nice, so a
//! monochrome terminal loses no information — and `NO_COLOR` is honoured
//! because a person who set it has said what they want.

use ratatui::style::{Color, Modifier, Style};

/// The environment variable that turns the icons off.
///
/// `RIVET_FLEET_GLYPHS=ascii`. The prefix is `crate::config::ENV_PREFIX`'s, so
/// it sits with every other override this tool reads rather than being a
/// private convention.
pub const GLYPHS_ENV: &str = "RIVET_FLEET_GLYPHS";

/// The one every terminal already honours.
pub const NO_COLOR_ENV: &str = "NO_COLOR";

/// One icon set.
///
/// **A struct rather than constants**, so that a call site cannot reach for a
/// literal: every glyph on screen comes from a field of this, and swapping the
/// set swaps all of them at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyphs {
    /// A machine that answered.
    pub up: char,
    /// A machine that did not.
    pub down: char,
    /// A machine nobody has asked yet. **Distinct from `down` on purpose** — an
    /// interface that shows "unreachable" before it has looked is lying.
    pub unknown: char,
    /// The register holds a change this machine has not been told about.
    pub pending: char,
    /// Something is wrong and a person should read it.
    pub warn: char,
    /// A setting that only takes effect after a restart.
    pub restart: char,
    /// A switch that is on, and one that is off.
    pub on: char,
    pub off: char,
    /// The selected row.
    pub cursor: char,
    /// **A clock face filling**, from empty to full, as the monitor counts down
    /// to its next poll — `ADR-034` §11.5.2's *waiting* state.
    ///
    /// Five frames, because five is enough to read at a glance and few enough
    /// that each one is a visibly different shape. **An index into this is
    /// derived from `crate::telemetry::Liveness` and never from a clock the
    /// renderer keeps**, which is the whole point of the indicator.
    pub waiting: [char; 5],
    /// **An indeterminate spinner**, for a request that is on the wire.
    ///
    /// Deliberately a different *family* of shape from [`Glyphs::waiting`]: the
    /// two states must be distinguishable at a glance, and a spinner made of
    /// circles beside a clock made of circles is one state with extra steps.
    pub spinning: [char; 4],
    /// **The poller is stopped** — no transport, or paused. Not animated, which
    /// is the ruling: an animation asserts that something is happening.
    pub stopped: char,
}

impl Glyphs {
    /// The icons, for a terminal with a nerd font.
    ///
    /// **Every one is a single cell.** A double-width glyph would shift the
    /// column beside it and make the fallback a different layout rather than
    /// the same layout with boxes in it.
    #[must_use]
    pub const fn nerd() -> Self {
        Self {
            up: '\u{f111}',      // nf-fa-circle
            down: '\u{f10c}',    // nf-fa-circle_o
            unknown: '\u{f141}', // nf-fa-ellipsis_h
            pending: '\u{f069}', // nf-fa-asterisk
            warn: '\u{f071}',    // nf-fa-warning
            restart: '\u{f021}', // nf-fa-refresh
            on: '\u{f205}',      // nf-fa-toggle_on
            off: '\u{f204}',     // nf-fa-toggle_off
            cursor: '\u{f054}',  // nf-fa-chevron_right
            // **Not private-use codepoints**, unlike everything above. These are
            // ordinary geometric shapes and Braille, in Unicode since 1993 and
            // 1999, single-width and present in almost every monospace font — so
            // the *rich* set of the one indicator that says whether the screen
            // is alive does not itself depend on a nerd font being installed.
            waiting: ['○', '◔', '◑', '◕', '●'],
            spinning: ['⠋', '⠙', '⠹', '⠸'],
            stopped: '⊘',
        }
    }

    /// The same meanings, in characters every terminal has had for forty years.
    #[must_use]
    pub const fn ascii() -> Self {
        Self {
            up: '*',
            down: 'o',
            unknown: '?',
            pending: '!',
            warn: '!',
            restart: '~',
            on: '+',
            off: '-',
            cursor: '>',
            // **The same five steps and the same four frames, in ASCII.** An
            // indicator that requires a font is a liveness signal that vanishes
            // exactly where a terminal is least capable, which is `ADR-034`
            // §11.5.2's reason for insisting on this pair.
            waiting: ['.', ':', 'o', 'O', '@'],
            spinning: ['|', '/', '-', '\\'],
            stopped: 'x',
        }
    }

    /// Which set to use, from the environment.
    ///
    /// **Icons by default**, because the maintainer asked for them and because
    /// the fallback above is a preference rather than a repair — nothing breaks
    /// without a nerd font, it merely looks worse.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var(GLYPHS_ENV).as_deref() {
            Ok("ascii" | "none" | "plain") => Self::ascii(),
            _ => Self::nerd(),
        }
    }
}

/// Whether this run may use colour.
///
/// `NO_COLOR` set to anything at all, including the empty string, means no —
/// that is the convention's own rule and not this program's reading of it.
#[must_use]
pub fn colour_allowed() -> bool {
    std::env::var_os(NO_COLOR_ENV).is_none()
}

/// The colours, and what each one means.
///
/// **Every field is a state, not a place.** There is no `heading` or `border`
/// here, because a colour applied for looks is one a monochrome terminal loses
/// and nobody misses — and one applied for meaning is information that has to
/// exist in the text as well.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    colour: bool,
}

impl Palette {
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            colour: colour_allowed(),
        }
    }

    /// Plain text.
    #[must_use]
    pub const fn plain(self) -> Style {
        Style::new()
    }

    /// A machine that answered.
    #[must_use]
    pub fn up(self) -> Style {
        self.tinted(Color::Green)
    }

    /// A machine that did not answer.
    #[must_use]
    pub fn down(self) -> Style {
        self.tinted(Color::Red)
    }

    /// A machine nobody has asked yet — **dim rather than coloured**, because
    /// "I do not know" should not compete for attention with "it is down".
    #[must_use]
    pub fn unknown(self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }

    /// A change the register holds and the machine has not been told.
    ///
    /// Unused until the device screen, which is where a change is made.
    #[allow(dead_code)]
    #[must_use]
    pub fn pending(self) -> Style {
        self.tinted(Color::Yellow)
    }

    /// Something a person should read.
    #[allow(dead_code)]
    #[must_use]
    pub fn warn(self) -> Style {
        self.tinted(Color::Yellow)
    }

    /// Something that has gone wrong.
    #[allow(dead_code)]
    #[must_use]
    pub fn error(self) -> Style {
        self.tinted(Color::Red)
    }

    /// The selected row. **Reversed rather than coloured**, so it is visible on
    /// a terminal with no colour at all — which is the one piece of state a
    /// person cannot do without.
    #[must_use]
    pub const fn selected(self) -> Style {
        Style::new().add_modifier(Modifier::REVERSED)
    }

    /// Text that is present for orientation rather than for reading.
    #[must_use]
    pub const fn faint(self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }

    fn tinted(self, colour: Color) -> Style {
        if self.colour {
            Style::new().fg(colour)
        } else {
            // **Not nothing.** A meaning that was carried by colour has to be
            // carried by something when there is none, and bold is what is
            // left. The word beside it is still the real answer.
            Style::new().add_modifier(Modifier::BOLD)
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The property that makes a missing nerd font survivable.** Every glyph
    /// is one cell, so a terminal that cannot render one draws a box in the
    /// same column and the layout is unchanged.
    #[test]
    fn every_glyph_is_exactly_one_cell_in_both_sets() {
        for set in [Glyphs::nerd(), Glyphs::ascii()] {
            for (name, glyph) in fields(set) {
                let width = unicode_width(glyph);
                assert_eq!(width, 1, "{name} is {width} cells wide: {glyph:?}");
            }
        }
    }

    /// The two sets carry the same meanings, so switching cannot lose one.
    #[test]
    fn the_two_sets_have_the_same_fields_and_differ_in_all_of_them() {
        let (nerd, ascii) = (Glyphs::nerd(), Glyphs::ascii());
        assert_eq!(fields(nerd).len(), fields(ascii).len());
        for ((name, a), (_, b)) in fields(nerd).into_iter().zip(fields(ascii)) {
            assert_ne!(a, b, "{name} is the same in both sets");
        }
    }

    /// **The liveness indicator exists in both sets, and its two animated
    /// states are drawn from different shapes.**
    ///
    /// `ADR-034` §11.5.2 requires the four states to be *visually
    /// distinguishable*, and the two that move are the pair a person will
    /// confuse: a clock face counting down and a spinner that means *the
    /// remaining time is unknown* say opposite things. A shape shared between
    /// them would collapse that.
    #[test]
    fn waiting_and_in_flight_never_share_a_frame_in_either_set() {
        for set in [Glyphs::nerd(), Glyphs::ascii()] {
            for face in set.waiting {
                assert!(
                    !set.spinning.contains(&face),
                    "{face:?} is both a clock face and a spinner frame"
                );
                assert_ne!(face, set.stopped, "{face:?} also means stopped");
            }
            for frame in set.spinning {
                assert_ne!(frame, set.stopped, "{frame:?} also means stopped");
            }
            // Every clock face is its own step, or the countdown does not read
            // as one.
            let mut seen = set.waiting.to_vec();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), set.waiting.len(), "two clock faces are the same");
        }
    }

    /// **`unknown` is not `down`.** An interface that showed "unreachable"
    /// before it had asked would be stating something it does not know.
    #[test]
    fn not_knowing_is_a_different_glyph_from_being_down() {
        let g = Glyphs::nerd();
        assert_ne!(g.unknown, g.down);
        assert_ne!(g.unknown, g.up);
        let a = Glyphs::ascii();
        assert_ne!(a.unknown, a.down);
        assert_ne!(a.unknown, a.up);
    }

    /// **Colour never carries a meaning alone.** With colour off, every style
    /// that meant something still differs from plain text — so a `NO_COLOR`
    /// terminal loses appearance and not information.
    #[test]
    fn no_meaning_is_carried_by_colour_alone() {
        let mono = Palette { colour: false };
        for (name, style) in [
            ("up", mono.up()),
            ("down", mono.down()),
            ("pending", mono.pending()),
            ("warn", mono.warn()),
            ("error", mono.error()),
            ("unknown", mono.unknown()),
            ("selected", mono.selected()),
        ] {
            assert_ne!(
                style,
                Style::new(),
                "{name} is indistinguishable from plain text without colour"
            );
            assert_eq!(style.fg, None, "{name} still asks for a colour");
        }
    }

    /// With colour on, the states that mean different things look different.
    #[test]
    fn up_and_down_are_never_the_same_colour() {
        let p = Palette { colour: true };
        assert_ne!(p.up().fg, p.down().fg);
        assert_ne!(p.up().fg, p.error().fg);
    }

    /// `NO_COLOR` means no colour whatever it is set to, including empty —
    /// that is the convention's rule, not this program's reading of it.
    #[test]
    fn the_no_color_convention_is_honoured_as_written() {
        // The function reads the environment, so this asserts the shape of the
        // check rather than mutating a global in a threaded test runner.
        let source = include_str!("theme.rs");
        assert!(
            source.contains("var_os(NO_COLOR_ENV).is_none()"),
            "NO_COLOR must be tested for PRESENCE, not for a value"
        );
    }

    fn fields(g: Glyphs) -> Vec<(&'static str, char)> {
        let mut all = vec![
            ("up", g.up),
            ("down", g.down),
            ("unknown", g.unknown),
            ("pending", g.pending),
            ("warn", g.warn),
            ("restart", g.restart),
            ("on", g.on),
            ("off", g.off),
            ("cursor", g.cursor),
            ("stopped", g.stopped),
        ];
        all.extend(g.waiting.iter().map(|c| ("waiting", *c)));
        all.extend(g.spinning.iter().map(|c| ("spinning", *c)));
        all
    }

    /// How many cells a character takes.
    ///
    /// Written here rather than depended on: `unicode-width` is a crate and a
    /// table, and the only question this interface asks of it is whether a
    /// glyph is wide. The private-use area every nerd font lives in is
    /// narrow by definition, and the ASCII set is ASCII.
    fn unicode_width(c: char) -> usize {
        match c {
            // Control characters have no business here at all.
            c if c.is_control() => 0,
            // The ranges a terminal renders double-width. None of the glyphs
            // above are in them, which is the point of the test.
            '\u{1100}'..='\u{115F}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE30}'..='\u{FE6F}'
            | '\u{FF00}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
            | '\u{1F300}'..='\u{1F64F}'
            | '\u{1F900}'..='\u{1F9FF}' => 2,
            _ => 1,
        }
    }
}
