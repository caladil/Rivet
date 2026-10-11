//! **What this machine can say about itself**, read from `/proc` and `/sys`.
//!
//! # Every number here is formatted with INTEGER arithmetic, deliberately
//!
//! This producer exists to be one of two — `PLAN.md` §112.2 — and the proof it
//! exists for is that **the renderer cannot tell which language wrote the
//! card**. That proof is only as good as the bytes: a card that says `52.4` in
//! Rust and `52.3` in Go has failed it, and would fail it intermittently, at
//! whichever temperature happens to land on a rounding boundary.
//!
//! Rust's `{:.1}` rounds half away from zero and Go's `strconv.FormatFloat`
//! rounds half to even. **So no float is formatted anywhere in this file.**
//! Millidegrees become tenths by `(m + 50) / 100` in integers, and the decimal
//! point is placed by hand. `/proc/loadavg` is already a formatted string and is
//! taken **verbatim** rather than parsed and re-rendered, which is the same
//! decision made twice.
//!
//! `examples/aura/aura-vitals-go/vitals.go` is the other half and carries the
//! same rules in the same order. Both are run against
//! `examples/aura/vitals-fixture` — one machine, frozen — and both assert
//! against its `expected-card.txt`, so a divergence in either fails on both
//! sides rather than being discovered on a wall.
//!
//! # The root is injectable, and that is what makes any of this testable
//!
//! Every read is relative to a root. In production it is `/`; in a test it is a
//! fixture directory. Without that, this module could only be tested on a
//! booted Linux machine with a thermal zone — which is to say, not in CI and not
//! on the maintainer's workstation.

use std::fs;
use std::path::Path;

/// Where the SoC temperature is, under the root.
///
/// **Zone 0 and no search.** A Pi has one thermal zone and it is the SoC; a
/// machine with several would need a rule for which one it means, and inventing
/// that rule for hardware nobody has tested would be inventing a reading. On a
/// machine with no zone 0 — QEMU, notably — the read fails and the card says so
/// with `state: unavailable` rather than showing a number from nowhere.
const THERMAL: &str = "sys/class/thermal/thermal_zone0/temp";
const UPTIME: &str = "proc/uptime";
const MEMINFO: &str = "proc/meminfo";
const LOADAVG: &str = "proc/loadavg";
const HOSTNAME: &str = "proc/sys/kernel/hostname";

/// **A reading this machine could not take.**
///
/// Distinguished from a zero throughout: `None` is *the file was not there or
/// did not parse*, and it reaches the card as `state: unavailable` — which
/// `SPEC-04` §5.4 says is the producer's word and not the renderer's to invent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Vitals {
    /// The machine's name, for the card's caption.
    pub hostname: Option<String>,
    /// SoC temperature in **tenths of a degree Celsius**, as an integer.
    ///
    /// Tenths and not a float: see the module note. The card renders it as
    /// `{tenths / 10}.{tenths % 10}` and so does the Go half.
    pub temperature_tenths: Option<i64>,
    /// Seconds since boot, whole.
    pub uptime_seconds: Option<u64>,
    /// Total and available memory, in kibibytes as `/proc/meminfo` states them.
    pub memory_total_kib: Option<u64>,
    pub memory_available_kib: Option<u64>,
    /// The one-minute load average, **as the kernel formatted it**. A string
    /// and not a number, so there is nothing to re-render and nothing for two
    /// languages to disagree about.
    pub load_1m: Option<String>,
    /// How many tasks exist — the denominator of `/proc/loadavg`'s fourth
    /// field.
    pub tasks: Option<u64>,
}

impl Vitals {
    /// Read everything, from a root.
    ///
    /// **Nothing here fails.** Every field is independently optional, because a
    /// machine missing a thermal zone should still show its uptime, and a card
    /// that refused to exist because one file was absent would be a wall panel
    /// that goes blank on a kernel upgrade.
    #[must_use]
    pub fn read(root: &Path) -> Self {
        let read = |rel: &str| fs::read_to_string(root.join(rel)).ok();
        Self {
            hostname: read(HOSTNAME).and_then(|t| {
                let name = t.trim().to_string();
                (!name.is_empty()).then_some(name)
            }),
            temperature_tenths: read(THERMAL).and_then(|t| parse_millidegrees(&t)),
            uptime_seconds: read(UPTIME).and_then(|t| parse_uptime_seconds(&t)),
            memory_total_kib: read(MEMINFO).and_then(|t| meminfo_kib(&t, "MemTotal:")),
            memory_available_kib: read(MEMINFO).and_then(|t| meminfo_kib(&t, "MemAvailable:")),
            load_1m: read(LOADAVG).and_then(|t| load_field(&t)),
            tasks: read(LOADAVG).and_then(|t| task_total(&t)),
        }
    }

    /// The temperature as the card shows it — `"52.4"`, or `None`.
    #[must_use]
    pub fn temperature_text(&self) -> Option<String> {
        self.temperature_tenths.map(format_tenths)
    }

    /// The uptime as the card shows it — `"3d 4h"`, `"4h 12m"`, `"12m"`.
    #[must_use]
    pub fn uptime_text(&self) -> Option<String> {
        self.uptime_seconds.map(format_uptime)
    }

    /// Memory as the card shows it — `"412 MiB free of 924"`.
    ///
    /// **Free of total, in that order**, because the number a person is looking
    /// for is the headroom. Mebibytes by integer division, which truncates: a
    /// machine reporting 411 MiB when it has 411.9 is understating its headroom,
    /// and of the two directions that is the one that never surprises anybody.
    #[must_use]
    pub fn memory_text(&self) -> Option<String> {
        let (available, total) = (self.memory_available_kib?, self.memory_total_kib?);
        Some(format!(
            "{} MiB free of {}",
            available / 1024,
            total / 1024
        ))
    }
}

/// `"52412\n"` — millidegrees — into tenths, rounded half up.
///
/// `(m + 50) / 100` in integers. See the module note for why this is not
/// `m as f64 / 1000.0`.
///
/// **Negative temperatures are refused rather than mishandled.** A thermal zone
/// below zero is a machine outdoors in winter, which is a real case and not one
/// anybody has tested here — and `(m + 50) / 100` truncates toward zero in Rust
/// and in Go, so a negative would round the wrong way in both by the same
/// mistake. Refusing shows `unavailable`, which is honest.
#[must_use]
pub fn parse_millidegrees(text: &str) -> Option<i64> {
    let milli: i64 = text.trim().parse().ok()?;
    if milli < 0 {
        return None;
    }
    Some((milli + 50) / 100)
}

/// Tenths into `"52.4"`.
#[must_use]
pub fn format_tenths(tenths: i64) -> String {
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// `/proc/uptime`'s first field, whole seconds.
///
/// **The fraction is dropped without parsing it.** `"1234.56 5678.90"` is split
/// at the space and then at the dot, and only the integer part is read — so
/// there is no float parse to disagree about, and a kernel that one day writes
/// more decimals changes nothing.
#[must_use]
pub fn parse_uptime_seconds(text: &str) -> Option<u64> {
    let first = text.trim().split(' ').next()?;
    let whole = first.split('.').next()?;
    whole.parse().ok()
}

/// `"3d 4h"`, `"4h 12m"`, `"12m"`.
///
/// **Two units at most, and the smaller one is dropped once the larger is
/// days.** A wall panel is read from across a room; `3d 4h 17m 42s` is a
/// number nobody is going to finish.
#[must_use]
pub fn format_uptime(seconds: u64) -> String {
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        format!("{days}d {}h", hours % 24)
    } else if hours > 0 {
        format!("{hours}h {}m", minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

/// A `/proc/meminfo` line's kibibytes — `"MemTotal:        946684 kB"`.
#[must_use]
pub fn meminfo_kib(text: &str, key: &str) -> Option<u64> {
    let line = text.lines().find(|line| line.starts_with(key))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// `/proc/loadavg`'s first field, **verbatim**.
///
/// Validated as a shape rather than parsed into a number: it must be digits, a
/// dot and digits, so a kernel that wrote something surprising is refused
/// instead of drawn. But the string that reaches the card is the kernel's own
/// bytes, which is what makes this field impossible for two languages to
/// disagree about.
#[must_use]
pub fn load_field(text: &str) -> Option<String> {
    let field = text.trim().split(' ').next()?;
    let (whole, fraction) = field.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(whole) && digits(fraction)).then(|| field.to_string())
}

/// `/proc/loadavg`'s fourth field is `running/total`; this is the total.
#[must_use]
pub fn task_total(text: &str) -> Option<u64> {
    let field = text.trim().split(' ').nth(3)?;
    field.split_once('/')?.1.parse().ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn millidegrees_become_tenths_rounded_half_up() {
        assert_eq!(parse_millidegrees("52412\n"), Some(524));
        // The boundary the two languages would disagree about if either used a
        // float: 52.45 -> 524 or 525 depending on the rounding mode.
        assert_eq!(parse_millidegrees("52450"), Some(525));
        assert_eq!(parse_millidegrees("52350"), Some(524));
        assert_eq!(parse_millidegrees("0"), Some(0));
        assert_eq!(parse_millidegrees("-1000"), None, "below zero is untested here");
        assert_eq!(parse_millidegrees("warm"), None);
        assert_eq!(parse_millidegrees(""), None);
    }

    #[test]
    fn a_temperature_renders_with_one_decimal_and_no_float() {
        assert_eq!(format_tenths(524), "52.4");
        assert_eq!(format_tenths(600), "60.0");
        assert_eq!(format_tenths(5), "0.5");
    }

    #[test]
    fn uptime_reads_the_whole_seconds_and_ignores_the_fraction() {
        assert_eq!(parse_uptime_seconds("1234.56 5678.90\n"), Some(1234));
        assert_eq!(parse_uptime_seconds("0.00 0.00"), Some(0));
        // More decimals than today's kernel writes changes nothing.
        assert_eq!(parse_uptime_seconds("99.123456789 1"), Some(99));
        assert_eq!(parse_uptime_seconds("nonsense"), None);
    }

    #[test]
    fn uptime_shows_two_units_at_most() {
        assert_eq!(format_uptime(0), "0m");
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(60), "1m");
        assert_eq!(format_uptime(59 * 60), "59m");
        assert_eq!(format_uptime(60 * 60), "1h 0m");
        assert_eq!(format_uptime(4 * 3600 + 12 * 60), "4h 12m");
        assert_eq!(format_uptime(23 * 3600 + 59 * 60), "23h 59m");
        assert_eq!(format_uptime(24 * 3600), "1d 0h");
        assert_eq!(format_uptime(3 * 86400 + 4 * 3600 + 17 * 60), "3d 4h");
    }

    #[test]
    fn meminfo_finds_the_field_it_was_asked_for() {
        let text = "MemTotal:         946684 kB\n\
                    MemFree:          123456 kB\n\
                    MemAvailable:     422948 kB\n";
        assert_eq!(meminfo_kib(text, "MemTotal:"), Some(946_684));
        assert_eq!(meminfo_kib(text, "MemAvailable:"), Some(422_948));
        assert_eq!(meminfo_kib(text, "Swap:"), None);
        // `MemFree` must not answer for `MemAvailable` by prefix accident.
        assert_ne!(meminfo_kib(text, "MemFree:"), meminfo_kib(text, "MemAvailable:"));
    }

    #[test]
    fn the_load_average_is_taken_verbatim_and_validated_as_a_shape() {
        assert_eq!(load_field("0.14 0.10 0.09 1/128 4242\n"), Some("0.14".into()));
        assert_eq!(load_field("12.00 1.00 1.00 1/1 1"), Some("12.00".into()));
        for bad in ["", "x.yz 1 1 1/1 1", "014 1 1 1/1 1", ".5 1 1 1/1 1", "1. 1 1 1/1 1"] {
            assert_eq!(load_field(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_task_total_is_the_denominator_of_the_fourth_field() {
        assert_eq!(task_total("0.14 0.10 0.09 1/128 4242\n"), Some(128));
        assert_eq!(task_total("0.00 0.00 0.00 2/3 9"), Some(3));
        assert_eq!(task_total("0.00 0.00 0.00 broken 9"), None);
        assert_eq!(task_total("0.00 0.00 0.00"), None);
    }

    #[test]
    fn memory_is_shown_as_headroom_first() {
        let v = Vitals {
            memory_total_kib: Some(946_684),
            memory_available_kib: Some(422_948),
            ..Vitals::default()
        };
        assert_eq!(v.memory_text().as_deref(), Some("413 MiB free of 924"));
        // One half missing is no answer, not a half answer.
        let half = Vitals {
            memory_total_kib: Some(946_684),
            ..Vitals::default()
        };
        assert_eq!(half.memory_text(), None);
    }

    /// **A machine with none of these files still produces a `Vitals`.** The
    /// failure this prevents is a wall panel that goes blank because a kernel
    /// upgrade moved a path — every field is independently optional.
    #[test]
    fn a_root_with_nothing_in_it_reads_as_every_field_absent() {
        let v = Vitals::read(Path::new("/nonexistent-root-for-this-test"));
        assert_eq!(v, Vitals::default());
        assert_eq!(v.temperature_text(), None);
        assert_eq!(v.uptime_text(), None);
        assert_eq!(v.memory_text(), None);
    }
}
