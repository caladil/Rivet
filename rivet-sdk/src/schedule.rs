//! **Whether the screen should be lit right now** — `ADR-039` §8.3.
//!
//! # Why an application asks rather than decides
//!
//! The hours a building's screens are dark belong to the **machine** and not to
//! whatever happens to be drawing (`ADR-039` §1.2). The case that settled it is
//! twenty information displays in one building, off at seven in the evening: the
//! person who wants that is facilities, and a policy owned by the application is
//! lost the moment somebody swaps the wayfinding app for a menu board, absent
//! while the application is restarting, and absent again whenever the machine
//! has fallen back to the launcher.
//!
//! # But the application is the only thing that can ACT on it
//!
//! A kiosk application holds a DRM lease over the CRTC **and its connector**,
//! and a lease is exclusive — the lessor cannot modify leased objects while it
//! is held (`ADR-039` §8.1). So the operating system cannot blank a screen an
//! application is drawing on, which is exactly when a building's screens need
//! switching off.
//!
//! **That is the whole shape**: the machine owns the policy, this module is the
//! one implementation of reading it, and the application performs the switch on
//! the objects it was handed. Three display vendors carrying three copies of
//! [`Hours::is_on`]'s midnight arithmetic would be three sets of bugs, in a
//! building nobody is looking at after seven in the evening.
//!
//! # Absent means LIT, and that direction is not arbitrary
//!
//! A machine with no `[display]` table has no schedule, and every Rivet before
//! 2026-09-02 was in that state. Every failure here resolves the same way — an
//! unreadable file, a malformed line, a clock that cannot be read — because the
//! two mistakes are not equal: **a screen that stays lit when it should not is
//! visible to somebody, and a building dark at nine in the morning is a phone
//! call.**
//!
//! # What this module does NOT do
//!
//! **It does not switch anything off.** It answers a question. What an
//! application does with `false` is the application's: stop compositing, blank
//! the framebuffer, or set DPMS on the connector it holds. `ADR-039` §8.2 is
//! why the machine additionally sends HDMI-CEC standby where that is
//! configured — a blanked panel commonly keeps its backlight lit, and the
//! television is what the electricity meter notices.
//!
//! # Re-declared, not imported — `ADR-036` §8.3
//!
//! An application is a foreign program and must not link the operating system,
//! so the path and the arithmetic below restate `rivet_core::etc::DISPLAY_HOURS`
//! and `rivet_core::display` rather than importing them.
//! `tests/agreement.rs` is what buys the right to restate: it asserts this
//! module and `rivet_core` agree on the path, on the parse, and on the predicate
//! at every minute of the day.

use std::fs;

/// **Where the machine publishes the window** — `rivet_core::display::HOURS_PATH`.
///
/// Written by the supervisor from the document's `[display]` table. An
/// application cannot read the configuration document, and `/run/rivet` is the
/// runtime surface it can — the journal socket, the kiosk socket and the query
/// socket are all there.
///
/// **`/run/rivet` and NOT `/etc`**, which the first build of this got wrong.
/// `/etc` on a Rivet is a compatibility surface for C runtimes that demand an
/// exact path — musl reads `/etc/localtime` and nowhere else — and no C runtime
/// asks for a display schedule. `cmd/rivet-sysd/src/runtime.rs` carries the
/// rule; the maintainer asked for it on 2026-09-02 and the file moved the same
/// day.
///
/// **Absent is the ordinary case**, not an error: it means the operator wrote
/// no `[display]` table.
pub const HOURS_PATH: &str = "/run/rivet/display-hours";

/// A time of day, to the minute, in the machine's **local** time.
///
/// Minutes since local midnight, `0..1440`. See the module note for why the
/// window is local and not UTC: an operator who says *"off at seven"* means the
/// clock on the wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(u16);

impl TimeOfDay {
    /// Parse `HH:MM`, strictly — two digits, a colon, two digits.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (hh, mm) = text.split_once(':')?;
        if hh.len() != 2 || mm.len() != 2 {
            return None;
        }
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

    /// Build from an hour and a minute.
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

impl std::fmt::Display for TimeOfDay {
    /// Canonical `HH:MM` - the same spelling the machine wrote into the file,
    /// so an application echoing the window into its own journal produces the
    /// text an operator can match against the document.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (h, m) = self.hour_and_minute();
        write!(f, "{h:02}:{m:02}")
    }
}

/// The hours the screen is dark, and therefore the hours it is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hours {
    off: TimeOfDay,
    on: TimeOfDay,
}

impl Hours {
    /// Build a window. `off == on` has no honest reading and is refused.
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
    /// The window an operator actually writes crosses midnight — `off 19:00`,
    /// `on 07:00` — so the dark hours are not a single ascending range. Both
    /// orderings are ordinary:
    ///
    /// | | dark when |
    /// | :-- | :-- |
    /// | `off < on` | `off <= t < on` |
    /// | `off > on` | `t >= off` **or** `t < on` |
    ///
    /// Half-open at both ends, so the minute named by `on` is lit and the
    /// minute named by `off` is dark, and every minute of the day belongs to
    /// exactly one state.
    #[must_use]
    pub const fn is_on(self, now: TimeOfDay) -> bool {
        let dark = if self.off.0 < self.on.0 {
            now.0 >= self.off.0 && now.0 < self.on.0
        } else {
            now.0 >= self.off.0 || now.0 < self.on.0
        };
        !dark
    }

    /// Parse the projected file's contents.
    ///
    /// One line of two `HH:MM` fields, `off` first. `#` comments and blank
    /// lines are skipped — the file carries `ADR-007` §3.6.1's provenance
    /// header, which exists so that a person who runs `cat` on it learns what
    /// it is and what to change instead.
    #[must_use]
    pub fn parse(contents: &str) -> Option<Self> {
        let line = contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))?;
        let (off, on) = line.split_once(char::is_whitespace)?;
        Self::new(TimeOfDay::parse(off.trim())?, TimeOfDay::parse(on.trim())?)
    }
}

/// **The machine's display window, if it has one.**
///
/// `None` is the ordinary case and means *no schedule*: either the operator
/// wrote no `[display]` table, or the file could not be read or made sense of.
/// All three collapse deliberately — see the module note on why every failure
/// here resolves to *lit*.
#[must_use]
pub fn hours() -> Option<Hours> {
    Hours::parse(&fs::read_to_string(HOURS_PATH).ok()?)
}

/// **What time it is locally, to the minute.**
///
/// `localtime_r(3)` — musl's own tzfile reader, reading the `/etc/localtime`
/// the supervisor projects from the document's `timezone` key (`ADR-021` §9).
/// Provided here so an application does not have to reach for `libc` itself,
/// and so there is one answer to *what time is it here* rather than one per
/// application.
///
/// `None` when the clock cannot be read at all, which the caller treats as
/// *lit* along with every other failure. **On a non-Unix host that is the
/// permanent answer** — see `broken_down_local`.
#[must_use]
pub fn local_now() -> Option<TimeOfDay> {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let t = i64::try_from(epoch).ok()?;
    let (hour, minute) = broken_down_local(t)?;
    TimeOfDay::new(hour, minute)
}

/// A `time_t` broken down into the local hour and minute, or `None`.
///
/// **The one platform call in this module**, split out on 2026-09-05 so that the
/// other three hundred lines — the parser, the midnight arithmetic and every
/// refusal — compile and are tested on the development host. `localtime_r(3)` is
/// POSIX and Windows has no such function; before the split, declaring this
/// module unconditionally broke `cargo check --workspace` on the maintainer's own
/// machine for every workspace that depended on this crate.
#[cfg(unix)]
fn broken_down_local(t: i64) -> Option<(u8, u8)> {
    // SAFETY: `localtime_r` writes one `struct tm` through a pointer to a live
    // local and reads one `time_t` through a pointer to another; it retains
    // neither. The reentrant form deliberately, so a second thread calling
    // `localtime` in the same program cannot overwrite the result under us.
    //
    // Lifted for this expression only, on `crate::listen`'s pattern rather than
    // `crate::display`'s: this is one C call whose contract fits in the
    // paragraph above, not a module of steps that only make sense together.
    #[allow(unsafe_code)]
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(std::ptr::addr_of!(t).cast(), &raw mut tm).is_null() {
            return None;
        }
        tm
    };
    Some((u8::try_from(tm.tm_hour).ok()?, u8::try_from(tm.tm_min).ok()?))
}

/// **There is no local time here, and saying so is the honest answer.**
///
/// `AGENTS.md` §3: *"Stubs refuse; they never fake success."* This one refuses.
/// The clock a Rivet application means by *local* is the one `localtime_r(3)`
/// reads out of the `/etc/localtime` the supervisor projects from the document's
/// `timezone` key (`ADR-021` §9). A development host has neither, so answering
/// out of the host's own zone would be a different machine's opinion presented as
/// this one's — and the whole point of [`hours`] is that the window belongs to
/// the machine.
///
/// `None` is already this function's *cannot be read* value and every caller
/// resolves it to **lit**, which is the direction the module note argues for at
/// length. So a Windows author gets a [`screen_is_lit`] that answers `true`
/// unconditionally, which is exactly what an unscheduled machine answers.
#[cfg(not(unix))]
fn broken_down_local(_t: i64) -> Option<(u8, u8)> {
    None
}

/// **The one call an application makes.**
///
/// ```no_run
/// if rivet_sdk::schedule::screen_is_lit() {
///     // draw
/// } else {
///     // blank, and consider DPMS on the connector you were handed
/// }
/// ```
///
/// True when there is no schedule, when the schedule says lit, and whenever
/// anything at all could not be determined — see the module note.
///
/// **Cheap enough to call every frame**, but it does read a file: at thirty
/// frames a second that is thirty `open`/`read`/`close` a second on a tmpfs,
/// which is not free on a Pi that is also compositing. An application drawing
/// continuously should hold the [`hours`] it read and call [`Hours::is_on`]
/// with [`local_now`], re-reading the file only when it has reason to.
#[must_use]
pub fn screen_is_lit() -> bool {
    let Some(hours) = hours() else {
        return true;
    };
    local_now().is_none_or(|now| hours.is_on(now))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn t(text: &str) -> TimeOfDay {
        TimeOfDay::parse(text).expect("a valid time")
    }

    #[test]
    fn the_file_the_supervisor_writes_parses_back_to_the_window() {
        let projected = "# Generated by Rivet from the `[display]` table of the configuration \
                         document. Do not edit.\n\
                         # /etc is a read-only tmpfs, rendered afresh on every boot.\n\
                         19:00 07:00\n";
        let hours = Hours::parse(projected).expect("the projected file should parse");
        assert_eq!(hours.off(), t("19:00"));
        assert_eq!(hours.on(), t("07:00"));
    }

    /// The building case, from the other side of the wire.
    #[test]
    fn the_building_is_dark_overnight_and_lit_through_the_working_day() {
        let hours = Hours::parse("19:00 07:00").unwrap();
        for lit in ["07:00", "12:00", "18:59"] {
            assert!(hours.is_on(t(lit)), "{lit} should be lit");
        }
        for dark in ["19:00", "23:59", "00:00", "06:59"] {
            assert!(!hours.is_on(t(dark)), "{dark} should be dark");
        }
    }

    /// **Every failure resolves to LIT.** The direction this module is most
    /// likely to be wrong in, so it is the one asserted hardest: a building dark
    /// at nine in the morning is the outcome nothing may cause by accident.
    #[test]
    fn nothing_malformed_can_produce_a_dark_screen() {
        for junk in [
            "",
            "\n\n",
            "# only a header\n",
            "19:00",             // one field
            "19:00 7:00",        // a spelling the schema refuses
            "19:00 25:00",       // not a time
            "19:00 19:00",       // no width
            "off=19:00 on=07:00",// a format nobody writes
            "garbage",
            "\u{0}\u{1}",
        ] {
            assert!(
                Hours::parse(junk).is_none(),
                "{junk:?} parsed into a window, and a window switches screens off"
            );
        }
    }

    /// A window whose file is absent is not a window, and `screen_is_lit`
    /// treats that as lit. Asserted through `hours()` rather than by reading
    /// `HOURS_PATH`, because a host running this test has no `/etc` of Rivet's.
    #[test]
    fn a_machine_with_no_projected_file_has_no_schedule() {
        assert!(
            std::path::Path::new(HOURS_PATH).exists() || hours().is_none(),
            "no file, so there must be no schedule"
        );
    }

    #[test]
    fn local_time_is_a_time_of_day_or_nothing_at_all() {
        if let Some(now) = local_now() {
            assert!(now.minutes_since_midnight() < 24 * 60);
        }
    }
}
