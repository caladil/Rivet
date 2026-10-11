//! **Where a meaning becomes a colour.**
//!
//! `SPEC-04` §5.1. An addon sends [`Role::Alarm`]; this file is the only place
//! that decides what red that is. Everything else in the renderer asks a
//! [`Theme`] and never holds a channel value of its own.
//!
//! # The two signal colours are per theme, and that is why roles exist
//!
//! Aura's C++ renderer carries ten palettes and each one ships its own alert
//! and caution, with the reasoning written beside `CardTheme::alert`: contrast
//! is part of a theme, because a red that reads on a black ground is weak on a
//! white one and a theme drawing black text needs a darker red beside it. The
//! two light palettes ship `{186, 26, 26}` and `{150, 95, 0}` where the eight
//! dark ones ship `{255, 69, 58}` and `{255, 197, 0}`.
//!
//! Those exact pairs are carried over below rather than re-chosen, because they
//! were chosen against a real television by somebody looking at it.
//!
//! # NOTHING IN THIS FILE IS DRAWN, since 2026-09-02
//!
//! [`Theme`] was invented for the generic composition in `crate::layout`, which
//! no longer runs. **Aura's own card themes are `CardTheme` in
//! `cpp/src/main.cpp`** and are ported in `crate::widgets::card_theme`; there is
//! no `night` and no `day` in the product, and the two below were chosen here
//! rather than taken from it. What survives and matters is
//! [`BACKGROUND_PALETTES`] — the maintainer's ruled naming, which
//! `crate::background` implements. Kept unreferenced rather than deleted,
//! pending his decision on whether a generic path returns at all.
//!
//! ## That question is answered, and the answer did not bring this file back
//!
//! **The generic path returned on 2026-09-02** — `crate::layout` draws
//! `tertiary` and `quaternary` — and it was rebuilt on `CardTheme` rather than
//! on [`Theme`]. [`Theme::ink`] is now `crate::widgets::role_ink`, which answers
//! the same question against the palette the product actually ships instead of
//! against the two invented below; [`Theme::dimmed`] is the original reading
//! card's `dim`, which `role_ink` applies inline.
//!
//! So `NIGHT` and `DAY` are dead for a firmer reason than before: not *awaiting
//! a decision* but **superseded by one**. [`BACKGROUND_PALETTES`] is the only
//! thing here anybody still needs, and it is documentation rather than code.
//! **Deleting this file is the maintainer's call and nothing else blocks it**;
//! it is left standing rather than removed in the same pass that replaced it,
//! because a diff that both adds a mechanism and deletes its predecessor hides
//! which of the two a regression came from.
#![allow(dead_code)]

use aura_card::Role;

/// A complete set of colours, one per role, plus the two surfaces.
///
/// ~~Seven values and no more.~~ **Six, since 2026-09-02** — `accent` went with
/// `Role::Accent`, which Aura never had. The rule the struck sentence stated is
/// the one that removed it: *a theme that needed an eighth would be a theme
/// that had invented a meaning*, and this file had invented one.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// What an operator calls it.
    pub name: &'static str,
    /// Behind everything.
    pub background: [u8; 3],
    /// The card a slot's contents sit on.
    pub card: [u8; 3],
    /// [`Role::Normal`].
    pub normal: [u8; 3],
    /// [`Role::Muted`].
    pub muted: [u8; 3],
    /// [`Role::Warn`].
    pub warn: [u8; 3],
    /// [`Role::Alarm`].
    pub alarm: [u8; 3],
}

impl Theme {
    /// The colour a role means, in this theme.
    ///
    /// **The only function in the renderer that turns a meaning into pixels.**
    /// A second one would be a second answer, and `ADR-011` is the rule.
    #[must_use]
    pub const fn ink(&self, role: Role) -> [u8; 3] {
        match role {
            Role::Normal => self.normal,
            Role::Muted => self.muted,
            Role::Warn => self.warn,
            Role::Alarm => self.alarm,
        }
    }

    /// Every colour, scaled toward black.
    ///
    /// Aura's brightness command is a percentage that reaches **every** colour
    /// a card draws, so the signal colours dim with the display rather than
    /// blazing at night — the original reading card's `dim` lambda, kept because the
    /// behaviour it produces was the point.
    #[must_use]
    pub fn dimmed(&self, percent: u8) -> Self {
        let scale = |c: [u8; 3]| {
            let f = |v: u8| u8::try_from(u32::from(v) * u32::from(percent) / 100).unwrap_or(v);
            [f(c[0]), f(c[1]), f(c[2])]
        };
        Self {
            name: self.name,
            background: scale(self.background),
            card: scale(self.card),
            normal: scale(self.normal),
            muted: scale(self.muted),
            warn: scale(self.warn),
            alarm: scale(self.alarm),
        }
    }
}

/// The default. Aura's ice-blue card over a near-black ground —
/// `kCardIce` and `kBoxDarkPalette[0]` from `cpp/src/main.cpp`.
pub const NIGHT: Theme = Theme {
    name: "night",
    background: [0x05, 0x0a, 0x10],
    card: [0x03, 0x05, 0x0e],
    normal: [0xbe, 0xcd, 0xf7],
    muted: [0x5a, 0x66, 0x8c],
    warn: [0xff, 0xc5, 0x00],
    alarm: [0xff, 0x45, 0x3a],
};

/// The light one, and the reason the signal colours are not shared.
///
/// `kSolidWhite` in `cpp/src/main.cpp`: black text on a bright ground, with a
/// deeper red and amber, because *a colour chosen for a black background is not
/// a colour on white — it is a highlighter.*
pub const DAY: Theme = Theme {
    name: "day",
    background: [0xff, 0xff, 0xff],
    card: [0xef, 0xf3, 0xf5],
    normal: [0x00, 0x00, 0x00],
    muted: [0x55, 0x5c, 0x63],
    warn: [0x96, 0x5f, 0x00],
    alarm: [0xba, 0x1a, 0x1a],
};

/// Every theme this build has, in the order an operator would cycle them.
pub const ALL: [Theme; 2] = [NIGHT, DAY];

/// **The background palettes, named descriptively throughout** — `SPEC-04` §9.
///
/// Carried here as the settled naming rather than as code, because this
/// skeleton draws no background yet. Ruled by the maintainer 2026-09-01: the
/// effect is **Digital Rain**, which describes what is on the screen and names
/// nothing anyone owns, and every palette label follows the same test.
///
/// | background | palettes |
/// | :-- | :-- |
/// | Digital Rain | Phosphor Green (default), Ice Blue, Born This Way |
/// | Boxes | Ice Blue (default), Chartreuse, Dark, Born This Way |
/// | Solid | Black (default), White, Navy |
///
/// The two greens are named from their own channel values rather than from each
/// other. Digital Rain's is `{180,255,180}` over `{0,200,50}` over `{0,100,20}`
/// (`ss_matrix.hpp`): no red at all in the trail or the dim, and more blue than
/// red in both — the cool green of a monochrome CRT, which is what the effect
/// is evoking. Boxes' is a ramp through `{10,28,0}`, `{35,80,0}`, `{80,160,5}`
/// to `{145,230,60}` (`ss_boxes.hpp`): red climbing and blue near zero, which
/// is chartreuse, and the file's own comment already calls the next stop that.
///
/// **Born This Way stays.** Its internal key is `pride` and no reference exists
/// anywhere in the source; the label carries a meaning rather than imitating
/// anything.
///
/// **Nothing here is called `matrix` or `construct`.** Aura's portal already
/// says *Digital Rain* everywhere a person can see; the words survive only in
/// `internal/config/config.go`'s stored key, in the C++ file name
/// `ss_matrix.hpp`, in three portal labels reading *Matrix Green*, and in
/// `cpp/src/main.cpp`'s `ConstructGreen`/`ConstructIce` — a half-finished
/// rename that swapped one film reference for another. The rewrite is a new
/// product with no stored configuration to migrate, so it carries none of them
/// and accepts no `matrix_*` alias.
pub const BACKGROUND_PALETTES: [(&str, &[&str]); 3] = [
    (
        "Digital Rain",
        &["Phosphor Green", "Ice Blue", "Born This Way"],
    ),
    (
        "Boxes",
        &["Ice Blue", "Chartreuse", "Dark", "Born This Way"],
    ),
    ("Solid", &["Black", "White", "Navy"]),
];

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **Every role resolves in every theme.** The failure this catches is a
    /// role added to the vocabulary and not to a theme, which would compile
    /// only because `ink` is exhaustive — and would then be caught here rather
    /// than by somebody noticing that `warn` is black.
    #[test]
    fn every_role_has_a_colour_in_every_theme() {
        for theme in ALL {
            let mut seen: Vec<[u8; 3]> = Vec::new();
            for role in Role::ALL {
                let ink = theme.ink(role);
                assert!(
                    !seen.contains(&ink),
                    "{} draws {role:?} the same as something else, so the two \
                     meanings are indistinguishable on screen",
                    theme.name
                );
                seen.push(ink);
            }
        }
    }

    /// A theme's ink must not be its own card colour, which is the one way a
    /// card can be present and invisible.
    #[test]
    fn no_role_is_the_colour_of_the_card_it_is_drawn_on() {
        for theme in ALL {
            for role in Role::ALL {
                assert_ne!(theme.ink(role), theme.card, "{} / {role:?}", theme.name);
            }
        }
    }

    #[test]
    fn dimming_reaches_every_colour_including_the_signals() {
        let dark = NIGHT.dimmed(50);
        assert!(dark.alarm[0] < NIGHT.alarm[0]);
        assert!(dark.warn[1] < NIGHT.warn[1]);
        assert!(dark.normal[2] < NIGHT.normal[2]);
        // 100% is the identity, so an operator who never touches brightness
        // gets the colours the theme was designed with.
        for role in Role::ALL {
            assert_eq!(NIGHT.dimmed(100).ink(role), NIGHT.ink(role));
        }
    }
}
