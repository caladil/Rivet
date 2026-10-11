//! **Local time, and the one place this port changes a behaviour.**
//!
//! # The timezone
//!
//! The Go controller carries an IANA name in its own config and an embedded
//! copy of the zone database to resolve it (`-tags timetzdata`, because a Rivet
//! has no `/usr/share/zoneinfo`). This program does neither. It asks
//! `localtime_r(3)`, which reads `/etc/localtime`, which the supervisor
//! projects from **the machine's own `timezone` document key** (`ADR-021` §9).
//!
//! That is not a shortcut around a missing crate. It is `ADR-011`: a second
//! copy of the zone would be a second answer to a question the machine already
//! answers, and the two could disagree — a board set to `America/Chicago` and
//! an Aura config still saying `UTC` from whenever it was last written, with
//! nothing on screen to say which one won. `crate::config::migrate` reports and
//! drops the old key rather than leaving it to be edited by somebody who would
//! then watch it do nothing.
//!
//! It is also **the same call the renderer makes** for the clock card it draws
//! itself, so the two halves of Aura cannot disagree about what time it is.
//!
//! # musl already contains the reader
//!
//! Asking the C library is smaller than an RFC 8536 parser and smaller than a
//! date-time crate, and it is what `cmd/rivet-sysd`'s `Kernel::local_time` and
//! `aura-renderer` both do.

/// A broken-down local time, which is all this program ever needs.
///
/// A subset of `struct tm`: the fields the clock card, the schedule and the log
/// lines use, and none of the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    /// 0–23.
    pub hour: u32,
    /// 0–59.
    pub minute: u32,
    /// 0–60; the leap second is what makes the upper bound 60 and not 59.
    /// Nothing displays it — it exists so the schedule can align to `:00`.
    pub second: u32,
    /// 0 is Sunday, matching `tm_wday`.
    pub weekday: u32,
    /// 0 is January, matching `tm_mon`.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    /// Seconds east of UTC, from `tm_gmtoff`.
    ///
    /// Kept so the portal can **show** the machine's zone. Aura no longer owns
    /// a timezone setting — see the module note — so the page states what the
    /// machine decided rather than offering a field that would disagree with
    /// it.
    pub utc_offset_seconds: i64,
    /// The zone's abbreviation, from `tm_zone` — `CDT`, `UTC`, `GMT`.
    ///
    /// **A fixed buffer and not a `String`**, because this type is `Copy` and
    /// is passed around a tick loop; five bytes covers every abbreviation the
    /// zone database uses and a longer one is truncated rather than allocated
    /// for.
    ///
    /// `pub(crate)` rather than public: [`Local::zone_label`] is the way to
    /// read it, and the visibility exists only so that another module's tests
    /// can build a `Local` without going through the C library.
    pub(crate) zone: [u8; 8],
}

/// Full weekday names, indexed by [`Local::weekday`].
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The three-letter forms a schedule's `days` list is written in.
///
/// **Indexed the same way as [`WEEKDAYS`]**, so the two cannot fall out of step
/// with each other — a schedule that fired on the wrong day while the screen
/// showed the right one would be a very quiet bug.
pub const WEEKDAY_ABBREVIATIONS: [&str; 7] =
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Full month names, indexed by [`Local::month`].
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

impl Local {
    /// Now, in the machine's zone.
    ///
    /// # Errors
    ///
    /// A sentence, if the clock cannot be read or the C library refuses to
    /// break it down. Neither has been seen; both are reported rather than
    /// unwrapped, because a panic in the tick loop takes the display with it.
    pub fn now() -> Result<Self, String> {
        // `time(2)` through the same binding the renderer uses. A null argument
        // is the documented way to ask for the return value alone.
        //
        // `i64` and not `libc::time_t`, which `libc` deprecates on musl
        // targets: musl went 64-bit in 1.2.0 and the alias is being removed
        // rather than corrected. The type that matters is the one the ABI has —
        // 64 bits on both targets this builds for — and naming it `i64` says so
        // without leaning on an alias that is on its way out. `aura-renderer`'s
        // `clock.rs` carries the same note for the same call.
        let seconds: i64 = unsafe { libc::time(std::ptr::null_mut()) };
        if seconds == -1 {
            return Err("clock: the system clock could not be read".to_string());
        }
        Self::at(seconds)
    }

    /// Break down a specific instant. Split out so the formatters are testable
    /// on a machine whose clock is not the board's.
    ///
    /// # Errors
    ///
    /// A sentence, if `localtime_r` refuses the instant.
    pub fn at(seconds: i64) -> Result<Self, String> {
        // SAFETY: `localtime_r` writes one `struct tm` through a pointer to a
        // live local and reads one `time_t` through a pointer to another; it
        // retains neither. The reentrant form deliberately — `localtime`
        // returns a pointer to static storage, and this is called from the tick
        // loop and, later, from the schedule thread.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let written = unsafe { libc::localtime_r(&raw const seconds, &raw mut tm) };
        if written.is_null() {
            return Err(format!("clock: localtime_r refused the instant {seconds}"));
        }
        // SAFETY: `tm_zone` points at storage the C library owns for the life
        // of the process — musl returns a pointer into the loaded tzfile or a
        // static literal, and never into `tm`. It is copied out here rather
        // than kept, so nothing borrows it.
        let mut zone = [0u8; 8];
        if !tm.tm_zone.is_null() {
            let bytes = unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }.to_bytes();
            let take = bytes.len().min(zone.len());
            zone[..take].copy_from_slice(&bytes[..take]);
        }

        Ok(Self {
            hour: tm.tm_hour.clamp(0, 23) as u32,
            minute: tm.tm_min.clamp(0, 59) as u32,
            second: tm.tm_sec.clamp(0, 60) as u32,
            weekday: tm.tm_wday.clamp(0, 6) as u32,
            month: tm.tm_mon.clamp(0, 11) as u32,
            day: tm.tm_mday.clamp(1, 31) as u32,
            utc_offset_seconds: tm.tm_gmtoff as i64,
            zone,
        })
    }

    /// The machine's zone, as a person reads it — `CDT (UTC−05:00)`.
    ///
    /// **What the portal shows where the Go portal had a timezone picker.** The
    /// machine's `timezone` key is what decides this, so the page states the
    /// consequence and names the command that changes it.
    ///
    /// The IANA name is deliberately absent: `localtime_r` does not report one,
    /// and inferring it from an abbreviation is guesswork — `CST` alone is
    /// three different zones.
    pub fn zone_label(&self) -> String {
        let end = self.zone.iter().position(|b| *b == 0).unwrap_or(self.zone.len());
        let name = std::str::from_utf8(&self.zone[..end]).unwrap_or("");
        let total = self.utc_offset_seconds;
        // U+2212 MINUS SIGN, not a hyphen: this is a number, and the page is
        // read rather than parsed.
        let sign = if total < 0 { '\u{2212}' } else { '+' };
        let hours = total.abs() / 3600;
        let minutes = (total.abs() % 3600) / 60;
        if name.is_empty() {
            return format!("UTC{sign}{hours:02}:{minutes:02}");
        }
        format!("{name} (UTC{sign}{hours:02}:{minutes:02})")
    }

    /// The clock card's time, in the format the config asks for.
    ///
    /// `15:04` and `3:04` in Go's vocabulary: 24-hour zero-padded, or 12-hour
    /// with no leading zero **and no meridiem**. The absent AM/PM is the Go
    /// program's choice and is reproduced rather than corrected — the renderer
    /// lays the string out at a fixed width and a suffix would overflow it.
    pub fn time_string(&self, use_24_hour: bool) -> String {
        if use_24_hour {
            return format!("{:02}:{:02}", self.hour, self.minute);
        }
        let twelve = match self.hour % 12 {
            0 => 12,
            other => other,
        };
        format!("{twelve}:{:02}", self.minute)
    }

    /// The clock card's date — `"Weekday, Month D"`, Go's `Monday, January 2`.
    pub fn date_string(&self) -> String {
        format!(
            "{}, {} {}",
            WEEKDAYS[self.weekday as usize % 7],
            MONTHS[self.month as usize % 12],
            self.day
        )
    }

    /// `HH:MM`, which is how a scheduled event states its time.
    ///
    /// Always 24-hour, whatever the clock card is showing: a schedule is
    /// written once and read by a machine, and `7:00` would be ambiguous in a
    /// way `19:00` is not.
    pub fn schedule_time(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// The three-letter weekday a schedule's `days` list is written in.
    pub fn weekday_abbreviation(&self) -> &'static str {
        WEEKDAY_ABBREVIATIONS[self.weekday as usize % 7]
    }

    /// How many seconds until the next `:00`.
    ///
    /// The schedule evaluates on minute boundaries, so it sleeps this long
    /// first and then a minute at a time — an event set for `19:00` fires at
    /// `19:00` and not at whatever offset into the minute this process happened
    /// to start at. **Never zero**: a whole minute is the right wait at `:00`,
    /// and returning zero would spin.
    pub const fn seconds_to_next_minute(&self) -> u64 {
        60 - (self.second as u64 % 60)
    }
}

#[cfg(test)]
mod tests;
