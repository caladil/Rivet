//! `mode` — **whether this machine drives a screen**, as a real document key.
//! Added 2026-09-13, `docs/PLAN-Feature-Set-2026-09-12.md` §5.2 item 2
//! (`.rulings/` R-162, Wave 3a).
//!
//! # What this key means, and the one thing it deliberately does not do
//!
//! `cmd/rivet-provision`'s `Mode` question (`headless`/`kiosk`) decides two
//! things today: whether the menu asks `Auto-run` at all, and whether a
//! `[kiosk]` table is written — empty if the operator has not decided which
//! application takes the screen (`SPEC-05` §5: *"blank is a kiosk machine
//! that has not decided yet, which is a working machine"*). Neither of those
//! is a fact this key can assert on its own: `[kiosk]`'s own absence already
//! means *no application takes the screen* (`ADR-004` §6), so a document
//! that carries `mode = "kiosk"` with no `[kiosk]` table, or `mode =
//! "headless"` with a populated one, is not a state this crate refuses.
//!
//! **This is a recorded intent, not an enforced one, and that is a decision
//! named rather than an oversight.** `ADR-007` §3's rule for a document
//! authored anywhere applies here exactly as it does to an `auto-run` naming
//! an application nothing has installed: the cost of disagreement is a
//! visible one (an operator, or `cmd/rivet-fleet`'s portal, can compare
//! `mode` against `[kiosk]` and say so) rather than a refusal invented for
//! this key alone.
//!
//! **Resolved 2026-09-13, `.rulings/` R-164: `mode` is the switch, `[kiosk]`
//! is only its configuration.** A mismatch is tolerated and made visible,
//! never refused — the maintainer's own words go further than a plain
//! tolerate-and-warn: `[kiosk]`'s presence is not a second assertion
//! competing with `mode`, it is data `mode` reads when it says `kiosk`. A
//! headless machine carrying a `[kiosk]` table it does not use is not a
//! contradiction. So this crate still adds no refusal and no tolerance rule
//! for either combination — see `ADR-007` §7.
//!
//! **`.rulings/` R-165, minutes later: several modes' tables may coexist, and
//! `mode` alone picks which one the machine acts on.** A `[kiosk]` table (and,
//! one day, a `[desktop]` table) may each be fully configured at once; nothing
//! here treats a table's presence as that mode being active, and switching
//! `mode` is changing one key, never writing or clearing a table. This crate
//! already had nothing else to do to honour that — it never inspected
//! `[kiosk]`'s presence in the first place.
//!
//! So `mode` composes nothing else in this crate today. Its value is for a
//! reader — a generated interview (plan §5.2 item 3), a portal, an
//! operator — to act on; see the module documentation for the JSON Schema
//! this becomes.

/// Whether this machine drives a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineMode {
    /// A machine reached over the network. `cmd/rivet-provision`'s default.
    Headless,
    /// A machine that drives a display — `ADR-004` §6, `[kiosk]`.
    Kiosk,
}

impl MachineMode {
    /// The two words, in the order `cmd/rivet-provision`'s `Mode` question
    /// offers them.
    pub const ALL: &'static [&'static str] = &["headless", "kiosk"];

    /// What a value must be — quoted by the parser's own refusal.
    pub const RULE: &'static str = "one of \"headless\" or \"kiosk\"";

    /// Parse the word an operator wrote.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "headless" => Some(Self::Headless),
            "kiosk" => Some(Self::Kiosk),
            _ => None,
        }
    }

    /// The word this renders back.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Kiosk => "kiosk",
        }
    }
}

impl core::fmt::Display for MachineMode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_mode_is_exactly_one_of_two_words() {
        assert_eq!(MachineMode::parse("headless"), Some(MachineMode::Headless));
        assert_eq!(MachineMode::parse("kiosk"), Some(MachineMode::Kiosk));
        assert_eq!(MachineMode::parse("Kiosk"), None);
        assert_eq!(MachineMode::parse(""), None);
    }

    #[test]
    fn every_mode_round_trips_through_its_own_spelling() {
        for mode in [MachineMode::Headless, MachineMode::Kiosk] {
            assert_eq!(MachineMode::parse(mode.as_str()), Some(mode), "{mode} does not round-trip");
        }
    }
}
