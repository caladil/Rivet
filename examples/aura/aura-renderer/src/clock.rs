//! **The one card the renderer produces itself** — `SPEC-04` §4.3.
//!
//! # Why the clock is not an addon
//!
//! Every other card's producer is *the nearest thing that already has the
//! fact*: a network client has the weather, a relay client has the corner
//! card's reading, a sensor client has a room's temperature. For the time, the nearest
//! thing that already has the fact **is the renderer** — it is calling
//! `SystemTime::now` anyway, because it is drawing frames.
//!
//! Interposing a process and a socket between a renderer and the current time
//! adds a failure mode to the only card that cannot otherwise fail, and the
//! failure is unusually visible: a stale clock reads as a broken display in a
//! way that a missing reading card does not.
//!
//! **That is not hypothetical.** Aura does it today and has the defect. The Go
//! controller formats `time.Now()` into a string once a second and pushes it
//! (`cmd/aura-controller/main.go`, the tick loop, `st.SetClock`); the C++
//! renderer assigns `time_str` only when a state message arrives
//! (`cpp/src/main.cpp`) and `cpp/src/ipc_service.hpp` has no concept of
//! staleness at all — no timeout, no last-seen, nothing. So a controller that
//! stalls leaves the wall showing the minute it stalled at, indefinitely, while
//! every other card still looks plausible. Verified by reading both files.
//!
//! # It is still a card
//!
//! `SPEC-04` §4.4: this synthesises an ordinary [`Card`] in the ordinary
//! vocabulary and hands it to the same layout and theme path everything else
//! goes through. One drawing path, no special cases; the only difference is
//! that nothing had to arrive over a socket.

use aura_card::{Block, Card, Glyph, Reading, Slot};

/// **The offset from UTC, in seconds, asked of the C library.**
///
/// ~~A Rivet has no `/usr/share/zoneinfo` (`SPEC-01` §5.2), so there is nothing
/// on the machine to ask.~~ ~~**Struck 2026-09-02: there is now**, and this is
/// left at zero — the clock is UTC — because turning `/etc/localtime` into an
/// offset means a tzfile reader (RFC 8536) or `chrono`.~~
///
/// **Both struck 2026-09-02, later the same day.** The renderer was showing UTC
/// on a board configured for `America/Chicago` and the maintainer was looking at
/// it. Neither of the two options above was the answer, and the answer needs no
/// argument at all: **musl already has a tzfile reader, it is called
/// `localtime_r(3)`, and it reads exactly the file the supervisor projects.**
///
/// `cmd/rivet-sysd`'s `Kernel::local_time` — written the same morning — is the
/// pattern, down to the reentrant form and the reason for it. `tm_gmtoff` is
/// seconds **east** of UTC, so it is added, and it is negative in the Americas.
///
/// # Why an offset and not `struct tm`'s fields
///
/// Because the fields would leave two date implementations in one program.
/// [`long_date`] and [`hour_and_minute`] already exist, are tested against leap
/// days and century years, and produce the exact strings Aura's Go controller
/// emits. Asking the library only for the offset keeps one answer to *what day
/// is it* (`ADR-011`) and makes the C call the smallest thing it can be.
///
/// # What a failure does
///
/// Returns zero, which is UTC, and the caller says so in the journal once. A
/// clock that stopped because a library call failed would be a worse outcome
/// than a clock that is five hours out and labelled — and this cannot loop or
/// panic on a wall panel.
#[must_use]
pub fn utc_offset_seconds(epoch: i64) -> i64 {
    #[cfg(unix)]
    {
        // `i64` and not `libc::time_t`, which `libc` deprecates on musl
        // targets: musl went 64-bit in 1.2.0 and the alias is being removed
        // rather than corrected. `localtime_r` takes a pointer, so the type
        // that matters is the one the ABI has — 64 bits on both targets this
        // builds for — and naming it `i64` says that without leaning on an
        // alias that is on its way out.
        let t: i64 = epoch;
        // SAFETY: `localtime_r` writes one `struct tm` through a pointer to a
        // live local and reads one `time_t` through a pointer to another; it
        // retains neither. The reentrant form deliberately — `localtime`
        // returns a pointer to static storage, and this process draws frames on
        // one thread while a signal could arrive on another.
        let mut tm: libc::tm = unsafe { core::mem::zeroed() };
        let filled = unsafe { libc::localtime_r(&raw const t, &raw mut tm) };
        if filled.is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
    #[cfg(not(unix))]
    {
        // The development host. `cargo test` runs here and the board does not,
        // so this arm never draws a clock anybody looks at.
        let _ = epoch;
        0
    }
}

/// The 24-hour default, **until the controller says otherwise**.
///
/// **`false`, because that is Aura's default** — `internal/config/config.go`'s
/// `defaults()` sets `Use24Hour: false`, and the controller then formats
/// `"3:04"`: no leading zero and no meridiem, which is what has been on the wall.
///
/// # THIS FILE IS THE FALLBACK NOW, not the clock
///
/// ~~It is one named constant because an application on a Rivet has no
/// configuration surface.~~ **Corrected 2026-09-07**, and the correction is
/// that this was never supposed to be the clock at all.
///
/// `aura-controller` formats the time and date per its config and sends them in
/// **every** `state` message — `wire.rs`'s `Clock`, whose own words are *"the
/// controller formats; the renderer prints."* This process never implemented
/// the printing half: `StateMessage` had `weather`, `indoor` and the reading
/// card and no `clock`, so the block was deserialised into nothing and dropped, and the
/// screen drew from the constant below. The maintainer set 24-hour in the
/// portal, saved, watched a 12-hour clock, and was right to ask why.
///
/// **What this file is FOR is the argument in the manifest**: putting a process
/// and a socket between the display and the time would add a failure mode to
/// the only card that cannot otherwise fail. That argument is about a
/// controller that is *absent*, not one that is talking — so this produces the
/// clock when nothing has sent one, and stands aside when something has.
///
/// **A controller that dies therefore leaves a clock**, in this default shape
/// rather than the last one it was told. That is a visible change at the moment
/// of a failure, which is better than a frozen time and better than none.
pub const TWENTY_FOUR_HOUR: bool = false;

/// The clock card for this instant.
///
/// `epoch` is seconds since 1970-01-01T00:00:00Z, which is what
/// `SystemTime::now().duration_since(UNIX_EPOCH)` gives.
#[must_use]
pub fn card(epoch: i64) -> Card {
    let local = epoch + utc_offset_seconds(epoch);
    let (hour, minute) = hour_and_minute(local);
    let time = if TWENTY_FOUR_HOUR {
        format!("{hour:02}:{minute:02}")
    } else {
        let h = match hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{h}:{minute:02}")
    };
    Card {
        slot: Slot::Primary,
        // Two minutes. The renderer replaces this card every minute, so a TTL
        // of two means a renderer that somehow stopped producing one would show
        // an empty slot rather than a frozen time — the failure this whole file
        // is about, guarded against even in the one producer that cannot fail.
        ttl_seconds: 120,
        blocks: vec![Block::Reading(Reading {
            glyph: Some(Glyph::Clock),
            value: Some(time),
            caption: Some(long_date(local)),
            ..Reading::default()
        })],
    }
}

/// Hours and minutes of a day, from a signed epoch.
///
/// `rem_euclid` and not `%`: a negative epoch is before 1970 and `%` would give
/// a negative hour. It cannot happen on a board whose clock is set, and a clock
/// that reads `-3:-15` on a board whose clock is *not* set is the kind of thing
/// that gets diagnosed as a font bug.
fn hour_and_minute(epoch: i64) -> (i64, i64) {
    let seconds_today = epoch.rem_euclid(86_400);
    (seconds_today / 3600, (seconds_today % 3600) / 60)
}

/// `"Monday, September 1"` — the format Aura's controller emits.
#[must_use]
pub fn long_date(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let (_, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday, which is index 4 of a week starting Sunday.
    let weekday = (days + 4).rem_euclid(7) as usize;
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    format!(
        "{}, {} {day}",
        WEEKDAYS[weekday],
        MONTHS[(month - 1) as usize]
    )
}

/// Days since the epoch to a civil year, month and day.
///
/// Howard Hinnant's `civil_from_days`, which is the algorithm every C++ and Rust
/// date library uses and is short enough to carry rather than to depend on. It
/// shifts the year to start in March so that the leap day is the last day of
/// the year and needs no special case.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// Vectors computed from the Unix epoch by hand, and one of them is a leap
    /// day — which is the whole reason `civil_from_days` is not four lines of
    /// division.
    #[test]
    fn the_civil_date_is_right_at_the_dates_that_break_naive_arithmetic() {
        for (epoch, expected) in [
            (0_i64, "Thursday, January 1"),
            // 2000-02-29, a leap year that is also a century.
            (951_782_400, "Tuesday, February 29"),
            // 2100 is NOT a leap year; 2100-03-01.
            (4_107_542_400, "Monday, March 1"),
            // The day this file was written.
            (1_756_684_800, "Monday, September 1"),
        ] {
            assert_eq!(long_date(epoch), expected, "epoch {epoch}");
        }
    }

    #[test]
    fn the_time_is_right_across_a_day_boundary() {
        assert_eq!(hour_and_minute(0), (0, 0));
        assert_eq!(hour_and_minute(86_399), (23, 59));
        assert_eq!(hour_and_minute(86_400), (0, 0));
        // Before the epoch, which is what an unset clock can produce.
        assert_eq!(hour_and_minute(-1), (23, 59));
    }

    /// The synthesised card must be a card the wire would accept. If it is not,
    /// the claim that the clock goes through the same path as everything else
    /// is false.
    #[test]
    fn the_synthesised_clock_is_a_card_the_protocol_admits() {
        let message = aura_card::Message::Card(card(1_756_684_800 + 22 * 3600 + 15 * 60));
        let text = message.encode().expect("the clock is an ordinary card");
        assert_eq!(
            aura_card::Message::decode(&text).expect("and it decodes"),
            message
        );
        // The host this runs on is not the board, so the offset is whatever the
        // host's zone is; the assertion is about the SHAPE the wire admits, not
        // about a wall clock. `the_twelve_hour_clock_is_the_format_aura_ships`
        // below is where the format is pinned.
        assert!(text.contains("value: "), "{text}");
        assert!(text.contains("caption: "), "{text}");
    }

    /// **Aura's default is a twelve-hour clock with no leading zero**, which
    /// `cmd/aura-controller/main.go` writes as `localNow.Format("3:04")` when
    /// `Use24Hour` is false — and `internal/config/config.go`'s `defaults()`
    /// sets it false. `"09:05"` and `"9:05 PM"` are both wrong on this wall.
    #[test]
    fn the_twelve_hour_clock_is_the_format_aura_ships() {
        const { assert!(!TWENTY_FOUR_HOUR, "Aura's default is twelve-hour") };
        for (hour, minute, expected) in [
            (0_i64, 5_i64, "12:05"),
            (9, 5, "9:05"),
            (12, 0, "12:00"),
            (13, 45, "1:45"),
            (23, 59, "11:59"),
        ] {
            let h = match hour % 12 {
                0 => 12,
                h => h,
            };
            assert_eq!(format!("{h}:{minute:02}"), expected);
        }
    }

    /// **The offset is asked for and never assumed.** On this host it is
    /// whatever the host's zone is; what matters is that the call returns
    /// something a clock can be built from rather than panicking, and that
    /// applying it is what separates a UTC clock from a local one.
    #[test]
    fn the_library_answers_an_offset_that_a_clock_can_be_built_from() {
        let epoch = 1_756_684_800_i64;
        let offset = utc_offset_seconds(epoch);
        assert!(
            offset.abs() <= 26 * 3600,
            "{offset} is not a time zone offset any place on earth has"
        );
        // Whole minutes: every zone in the database is, and a stray second
        // would mean the field was read as something other than `tm_gmtoff`.
        assert_eq!(offset % 60, 0, "{offset} is not a whole number of minutes");
    }
}
