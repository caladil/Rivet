//! `[display]` — **the hours this machine's screen is meant to be lit.**
//!
//! `ADR-039` §3.1 and §8, ruled 2026-09-02. The type is
//! `rivet_core::display::Hours`; this module is the table around it — the keys
//! it accepts and the sentence a refusal quotes.
//!
//! # Why it is its own table and not part of `[kiosk]`
//!
//! `[kiosk]` names *which application holds the display*. This says *whether
//! there should be a picture at all*, and the two are independent in both
//! directions:
//!
//! * a machine with no `[kiosk]` still has a screen — the launcher is drawing
//!   on it, and a building's launcher menus should go dark at seven with
//!   everything else
//! * a machine with a `[kiosk]` and no `[display]` runs its application around
//!   the clock, which is what every Rivet does today
//!
//! Folding them together would make the schedule an attribute of the
//! application, which is the exact ownership `ADR-039` §1.2 argues against: the
//! person who wants the building dark is facilities, and they should not lose
//! the policy because somebody swapped the wayfinding app for a menu board.
//!
//! # The table is optional; a table that exists carries both keys
//!
//! `ADR-039` §3.1. A screen told when to go dark and never when to come back is
//! a screen somebody has to drive to, so *both or neither* is the rule — and it
//! is expressed the way `[control-plane]` expresses its one key rather than the
//! way `[splash]` expresses its two. `[splash]`'s keys are independent switches
//! and either alone is a complete statement; these two are one statement in two
//! halves.
//!
//! So **an empty `[display]` is refused** rather than read as silence: the way
//! to say nothing is to omit the table. `off == on` is refused as well, because
//! a window of no width has no honest reading
//! (`rivet_core::display::Hours::new`).
//!
//! That leaves the schema saying exactly what the parser enforces, with no
//! condition to publish in prose — which is `ADR-007` §3.1's preference for
//! states that cannot be spelled over states that are caught.

/// `[display]` — the table's shape, for the parser and for its diagnostics.
///
/// There is no struct here on purpose: the value is
/// `rivet_core::display::Hours`, which already carries the arithmetic and the
/// rule, and a wrapper holding one field would be a second name for one thing
/// (`ADR-011`).
pub struct Display;

impl Display {
    /// Every key this table accepts. An unknown one is refused by name, on
    /// `crate::splash`'s rule: a misspelled setting that parses is a setting
    /// that silently does nothing — and a misspelled `off` is a building that
    /// stays lit all night with a document that says it should not.
    pub const DECLARED_KEYS: &'static [&'static str] = &["off", "on"];

    /// What the table is, for a refusal about the wrong kind of thing.
    pub const EXPECTED: &'static str = "a table of the hours the screen is lit";

    /// What is wrong when one of the pair is missing.
    ///
    /// Spelled here rather than at the call site because it is the diagnostic
    /// an operator is most likely to see: writing `off` and forgetting `on` is
    /// the natural mistake, and it is the one with the worst consequence.
    pub const PAIR_RULE: &'static str =
        "`[display]` needs both `off` and `on`, or neither — a screen told when to go dark and \
         never when to come back is one somebody has to visit";
}
