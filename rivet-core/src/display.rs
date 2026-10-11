//! **When this machine's screen is meant to be on** — `ADR-039` §3.1 and §8.
//!
//! Ruled 2026-09-02: *display hours are the machine's, not the application's.*
//! The maintainer's case is `ADR-039` §1.2 — twenty information displays in a
//! building, off at seven in the evening and on at seven in the morning — and
//! the person who wants that is facilities rather than the author of whatever
//! happens to be drawing.
//!
//! This module is the type that ruling needs, and **it is deliberately the
//! whole of the logic**. `ADR-039` §8.3 puts the evaluation in the platform
//! precisely so that no application carries it: three display vendors carrying
//! three copies of the midnight-wrap arithmetic below is three sets of bugs, in
//! a building nobody is looking at after seven in the evening.
//!
//! # It is a PREDICATE, and that is the whole reason it is cheap
//!
//! `ADR-039` §2 separates *time-varying desired state* from *scheduled
//! actions*, and this is the first kind. [`Hours::is_on`] answers **"should the
//! screen be lit at this moment"** and is asked afresh every convergence pass.
//!
//! Nothing here stores a fire time, and three hard problems disappear with it:
//!
//! | | Why it is not a problem here |
//! | :-- | :-- |
//! | **a missed occurrence** | there are no occurrences. A machine powered off at 19:00 and booted at 21:00 evaluates, finds itself inside the off window, and converges dark. Nothing was missed because nothing was scheduled |
//! | **the clock step** | `ADR-021` §4.2 steps the wall clock at boot, from 1970 to real time. A stored instant computed before that jump is either years overdue or years away (`ADR-039` §5); a predicate re-read after it is simply correct |
//! | **daylight saving** | a repeated hour is evaluated twice and agrees with itself; a missing hour is never asked about. `musl` owns the transition, through the tzfile `ADR-021` §9 projects |
//!
//! # Local time, and it could not have been written a week ago
//!
//! The window is in the machine's **local** time — `ADR-039` §6. An operator
//! who says *"off at seven"* means the clock on the wall, and a UTC schedule is
//! wrong by an hour for half the year in most of the world.
//!
//! That is only expressible because `ADR-021` §9 put the zone in the document
//! on 2026-09-02. Before it there was nothing on the machine to ask, which is
//! the same absence that had `aura-renderer` drawing a wall clock in UTC and
//! labelling it UTC.

use core::fmt;

/// Minutes in a day. The modulus every value here lives under.
const MINUTES_PER_DAY: u16 = 24 * 60;

/// **Where the machine publishes the window for applications to read.**
///
/// `/run/rivet` is Rivet's own runtime namespace — the journal socket, the
/// kiosk socket, the query socket and `reached` all live there — and this is a
/// runtime fact of exactly that kind.
///
/// # Why NOT `/etc`, which the first build of this used
///
/// **Every file in `crate::etc` is there because a C runtime demands that exact
/// path**: musl's `localtime_r` reads `/etc/localtime` and nowhere else, its
/// resolver reads `/etc/resolv.conf`, its `getpwnam` reads `/etc/passwd`,
/// and a login-shell check reads `/etc/shells`. `/etc` on a Rivet is a compatibility surface,
/// not a configuration surface — `ADR-007` §3.6.2 says a file there is a *view*
/// and never an interface, and §3.5 remounts the whole thing read-only.
///
/// Nothing in any C library looks for a display schedule. Putting one in `/etc`
/// would have been Rivet inventing a path in a namespace it does not own, for a
/// reader that is Rivet's own — and the next person to ask which C runtime
/// needs this would have found no answer. The maintainer asked that question
/// on 2026-09-02 (`.rulings/` R-266) and the file moved the same day.
pub const HOURS_PATH: &str = "/run/rivet/display-hours";

/// **A time of day, to the minute, with no date and no zone.**
///
/// Minutes since local midnight, `0..1440`. It is not an instant and cannot be
/// compared to one: `19:00` is a fact about a clock face, and which instant it
/// names depends on a zone and a date that this type deliberately does not
/// carry.
///
/// # To the minute, and no finer
///
/// `ADR-039` §9 rules out sub-second scheduling on the grounds that a schedule
/// cannot be more precise than the clock beneath it, and `ADR-021` §2's target
/// is *"seconds on a home network"*. Seconds in a display schedule would be
/// precision the machine cannot honour and no operator asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    /// What the value has to be, for a refusal that says so rather than
    /// quoting back what somebody typed.
    pub const RULE: &'static str =
        "a time of day as HH:MM on a 24-hour clock, such as 19:00 or 07:00 — the machine's LOCAL \
         time, from the `timezone` key";

    /// Parse `HH:MM`.
    ///
    /// **Strict, and both fields are two digits.** `7:00` is refused. A schema
    /// that accepts several spellings of one value is a schema with several
    /// spellings to keep working, and the cost of being strict is one
    /// diagnostic naming the rule — which is cheaper than the day somebody
    /// discovers `7:00` and `07:00` sort differently somewhere.
    ///
    /// `24:00` is refused as well. It is a real spelling of midnight in some
    /// standards and it would be a second spelling of `00:00` here
    /// (`ADR-011`).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (hh, mm) = text.split_once(':')?;
        if hh.len() != 2 || mm.len() != 2 {
            return None;
        }
        // `str::parse` would accept a leading `+` and unicode digits; this
        // accepts exactly four ASCII digits and nothing else.
        if !hh.bytes().chain(mm.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let hours: u16 = hh.parse().ok()?;
        let minutes: u16 = mm.parse().ok()?;
        if hours > 23 || minutes > 59 {
            return None;
        }
        Some(Self(hours * 60 + minutes))
    }

    /// Build from an hour and a minute, for a caller that already has both.
    #[must_use]
    pub const fn new(hours: u8, minutes: u8) -> Option<Self> {
        if hours > 23 || minutes > 59 {
            return None;
        }
        Some(Self(hours as u16 * 60 + minutes as u16))
    }

    /// Minutes since local midnight.
    #[must_use]
    pub const fn minutes_since_midnight(self) -> u16 {
        self.0
    }

    /// The hour and the minute.
    #[must_use]
    pub const fn hour_and_minute(self) -> (u8, u8) {
        ((self.0 / 60) as u8, (self.0 % 60) as u8)
    }
}

impl fmt::Display for TimeOfDay {
    /// Canonical `HH:MM`, so a document round-trips to one spelling.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m) = self.hour_and_minute();
        write!(f, "{h:02}:{m:02}")
    }
}

/// **The hours a screen is dark, and therefore the hours it is lit.**
///
/// Held as the pair an operator writes — *off at this time, on at that time* —
/// rather than as a lit interval, because that is the sentence the person
/// asking says out loud (`ADR-039` §1.2) and a type that matches the sentence
/// is a type nobody has to invert in their head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hours {
    off: TimeOfDay,
    on: TimeOfDay,
}

impl Hours {
    /// **Both or neither**, and this is the reason.
    ///
    /// `ADR-039` §3.1: a machine told when to switch off and never when to
    /// switch on is a machine somebody has to drive to. The pair is refused as
    /// a pair so that the half-configured state is unrepresentable rather than
    /// merely discouraged — `ADR-007` §3.1's preference, the same one `[kiosk]`
    /// applies to two applications claiming auto-run.
    pub const RULE: &'static str =
        "`off` and `on` together, each a time of day as HH:MM in the machine's local time — a \
         screen told when to go dark and never when to come back is one somebody has to visit";

    /// Build a window.
    ///
    /// # `off` and `on` may not be equal
    ///
    /// It is the one pair with no honest reading. As a zero-length dark window
    /// it says *never off*, which `[display]`'s absence already says more
    /// clearly; as a zero-length lit window it says *never on*, which is a
    /// building of dark screens written as a typo. Refusing costs an operator
    /// one diagnostic and cannot be confused with either.
    #[must_use]
    pub const fn new(off: TimeOfDay, on: TimeOfDay) -> Option<Self> {
        if off.0 == on.0 {
            return None;
        }
        Some(Self { off, on })
    }

    /// When the screen goes dark.
    #[must_use]
    pub const fn off(self) -> TimeOfDay {
        self.off
    }

    /// When it comes back.
    #[must_use]
    pub const fn on(self) -> TimeOfDay {
        self.on
    }

    /// **Should the screen be lit at this local time?**
    ///
    /// The one question this module exists to answer, and the one call an
    /// application makes (`ADR-039` §8.3).
    ///
    /// # The midnight wrap, which is the normal case and not the edge case
    ///
    /// The window an operator actually writes — `off = "19:00"`, `on = "07:00"`
    /// — **crosses midnight**, so the dark hours are not a single ascending
    /// range. Both orderings are ordinary and neither is a special case:
    ///
    /// | | dark when | example |
    /// | :-- | :-- | :-- |
    /// | `off < on` | `off <= t < on` | `off = "02:00"`, `on = "07:00"` — dark in the small hours only |
    /// | `off > on` | `t >= off` **or** `t < on` | `off = "19:00"`, `on = "07:00"` — dark overnight |
    ///
    /// **Half-open at both ends**, so the minute named by `on` is lit and the
    /// minute named by `off` is dark. Every minute of the day belongs to
    /// exactly one of the two states, which is the property
    /// `every_minute_of_the_day_is_either_lit_or_dark_and_never_both` asserts
    /// by walking all 1440 of them.
    #[must_use]
    pub const fn is_on(self, now: TimeOfDay) -> bool {
        let dark = if self.off.0 < self.on.0 {
            now.0 >= self.off.0 && now.0 < self.on.0
        } else {
            now.0 >= self.off.0 || now.0 < self.on.0
        };
        !dark
    }

    /// How many minutes until the state changes, from `now`.
    ///
    /// **For a caller deciding when to look again**, so a convergence loop that
    /// has nothing else to do can sleep instead of spinning. Never zero and
    /// never more than a day: at the exact minute of a transition the answer is
    /// the time to the *next* one.
    ///
    /// It is a duration and not an instant, deliberately — `ADR-021` §5, and
    /// `ADR-039` §5 rule 2. A caller adds it to its own monotonic clock; this
    /// module names no absolute time and therefore cannot be wrong about one
    /// across a step.
    #[must_use]
    pub const fn minutes_until_change(self, now: TimeOfDay) -> u16 {
        let next = if self.is_on(now) { self.off.0 } else { self.on.0 };
        let delta = (next + MINUTES_PER_DAY - now.0) % MINUTES_PER_DAY;
        if delta == 0 { MINUTES_PER_DAY } else { delta }
    }
}

impl fmt::Display for Hours {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "off {}, on {}", self.off, self.on)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn t(text: &str) -> TimeOfDay {
        TimeOfDay::parse(text).expect("a valid time")
    }

    fn overnight() -> Hours {
        Hours::new(t("19:00"), t("07:00")).expect("a valid window")
    }

    #[test]
    fn a_time_of_day_parses_only_as_two_digits_and_two_digits() {
        assert_eq!(t("00:00").minutes_since_midnight(), 0);
        assert_eq!(t("07:00").minutes_since_midnight(), 420);
        assert_eq!(t("19:30").minutes_since_midnight(), 19 * 60 + 30);
        assert_eq!(t("23:59").minutes_since_midnight(), 1439);
        for bad in [
            "7:00",      // one-digit hour — one value, one spelling
            "07:0",      // one-digit minute
            "24:00",     // a second spelling of 00:00
            "23:60",     // not a minute
            "25:00",     // not an hour
            "0700",      // no separator
            "07:00:00",  // seconds are not in scope, ADR-039 §9
            "+7:00",     // `str::parse` would take the sign
            "",
            ":",
            "ab:cd",
            " 07:00",
        ] {
            assert!(
                TimeOfDay::parse(bad).is_none(),
                "{bad:?} should be refused rather than guessed at"
            );
        }
    }

    /// A parsed time renders back to exactly one spelling, so a document that
    /// is read and written does not drift.
    #[test]
    fn a_time_of_day_round_trips_through_its_canonical_spelling() {
        for text in ["00:00", "07:00", "12:34", "19:00", "23:59"] {
            assert_eq!(t(text).to_string(), text);
            assert_eq!(TimeOfDay::parse(&t(text).to_string()), Some(t(text)));
        }
    }

    /// **The case the maintainer actually described** — `ADR-039` §1.2.
    #[test]
    fn the_building_is_dark_overnight_and_lit_through_the_working_day() {
        let hours = overnight();
        // Lit through the day.
        for lit in ["07:00", "07:01", "12:00", "18:00", "18:59"] {
            assert!(hours.is_on(t(lit)), "{lit} should be lit");
        }
        // Dark overnight, across midnight.
        for dark in ["19:00", "19:01", "23:59", "00:00", "03:00", "06:59"] {
            assert!(!hours.is_on(t(dark)), "{dark} should be dark");
        }
    }

    /// The other ordering is ordinary too, and is not a special case.
    #[test]
    fn a_window_that_does_not_cross_midnight_works_the_same_way() {
        let small_hours = Hours::new(t("02:00"), t("07:00")).unwrap();
        for dark in ["02:00", "04:30", "06:59"] {
            assert!(!small_hours.is_on(t(dark)), "{dark} should be dark");
        }
        for lit in ["07:00", "12:00", "19:00", "23:59", "00:00", "01:59"] {
            assert!(small_hours.is_on(t(lit)), "{lit} should be lit");
        }
    }

    /// **Every minute belongs to exactly one state.** The property that says
    /// the two branches of `is_on` tile the day rather than overlapping at the
    /// boundary or leaving a gap at midnight — which is the bug a hand-written
    /// wrap check makes, in the one hour nobody is watching.
    #[test]
    fn every_minute_of_the_day_is_either_lit_or_dark_and_never_both() {
        for (off, on) in [
            ("19:00", "07:00"), // crosses midnight
            ("02:00", "07:00"), // does not
            ("00:00", "12:00"), // starts at midnight
            ("12:00", "00:00"), // ends at midnight
            ("23:59", "00:00"), // one minute dark
            ("00:00", "23:59"), // one minute lit
        ] {
            let hours = Hours::new(t(off), t(on)).expect("a valid window");
            let mut lit = 0u16;
            for minute in 0..MINUTES_PER_DAY {
                let now = TimeOfDay(minute);
                if hours.is_on(now) {
                    lit += 1;
                }
            }
            // Both states occur, so no window is secretly always-on or
            // always-off — which is what refusing `off == on` buys.
            assert!(lit > 0, "{off}/{on}: never lit");
            assert!(lit < MINUTES_PER_DAY, "{off}/{on}: never dark");
            // The boundaries are half-open in the stated direction.
            assert!(!hours.is_on(t(off)), "{off}/{on}: the off minute must be dark");
            assert!(hours.is_on(t(on)), "{off}/{on}: the on minute must be lit");
        }
    }

    /// `off == on` has no honest reading and is refused rather than guessed.
    #[test]
    fn a_window_with_no_width_is_refused() {
        assert!(Hours::new(t("19:00"), t("19:00")).is_none());
        assert!(Hours::new(t("00:00"), t("00:00")).is_none());
    }

    /// The countdown never returns zero and never exceeds a day, so a caller
    /// sleeping on it always wakes and never oversleeps a transition.
    #[test]
    fn the_time_to_the_next_change_is_a_duration_and_lands_on_the_change() {
        let hours = overnight();
        assert_eq!(hours.minutes_until_change(t("18:00")), 60);
        assert_eq!(hours.minutes_until_change(t("06:00")), 60);
        // At the exact minute of a change, the answer is the NEXT change:
        // 19:00 is dark, so the next event is 07:00 — twelve hours away.
        assert_eq!(hours.minutes_until_change(t("19:00")), 12 * 60);
        assert_eq!(hours.minutes_until_change(t("07:00")), 12 * 60);

        for (off, on) in [("19:00", "07:00"), ("02:00", "07:00"), ("00:00", "12:00")] {
            let hours = Hours::new(t(off), t(on)).unwrap();
            for minute in 0..MINUTES_PER_DAY {
                let now = TimeOfDay(minute);
                let delta = hours.minutes_until_change(now);
                assert!(delta > 0 && delta <= MINUTES_PER_DAY, "{off}/{on} at {now}");
                // The state is unchanged right up to the transition, and
                // different at it.
                let just_before = TimeOfDay((minute + delta - 1) % MINUTES_PER_DAY);
                let at = TimeOfDay((minute + delta) % MINUTES_PER_DAY);
                assert_eq!(
                    hours.is_on(now),
                    hours.is_on(just_before),
                    "{off}/{on}: the state changed before the countdown said it would, at {now}"
                );
                assert_ne!(
                    hours.is_on(now),
                    hours.is_on(at),
                    "{off}/{on}: the state did not change when the countdown ran out, at {now}"
                );
            }
        }
    }
}
