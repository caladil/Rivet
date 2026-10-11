//! Reading which boot this is, and how long it has been going — `ADR-035`
//! phase 4.
//!
//! **`/proc/uptime` and `/proc/sys/kernel/random/boot_id`, and nothing else.**
//! `crate::journal`'s note carries why that belongs here rather than behind
//! `BootMedium`: it needs no `mount(2)`, no device tree and no pid 1.
//!
//! # The module is `uptime` and the subject is `boot`
//!
//! Deliberately, because `crate::boot` already exists and is the composition
//! root's boot half — the A/B card and pid 1. This module is neither of those
//! things and naming it `boot` would have put two unrelated meanings of the word
//! in one crate, which `AGENTS.md` §10.2 is a whole section against.
//!
//! # No wall-clock time is read here, and that is the point
//!
//! `ADR-021`: a Raspberry Pi has **no real-time clock**. Every boot starts at
//! the epoch until something corrects it, and when something does, the wall
//! clock jumps. `/proc/uptime` comes off the monotonic counter and is therefore
//! true whether or not the clock was ever set — which is why it is the fact this
//! module returns and a timestamp is not.

use rivet_core::operation::machine::{BootRead, MachineError};

/// Seconds since boot, and seconds idle. Two decimal figures, space separated.
const UPTIME: &str = "/proc/uptime";

/// This boot's identifier — a random UUID the kernel mints once per boot. It is
/// the same value `rivet_core::journal::Record`'s `boot` field carries.
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";

/// Which boot this is and how long it has been going.
///
/// # Errors
///
/// If `/proc/uptime` could not be read or does not parse. **A missing boot id is
/// not an error** — it is reported as unmeasured, because a duration is still a
/// useful answer without a name for the boot it belongs to.
pub fn read() -> Result<BootRead, MachineError> {
    let text = std::fs::read_to_string(UPTIME)
        .map_err(|e| MachineError::Refused(format!("{UPTIME} could not be read: {e}")))?;

    // **The uptime is the one that must parse.** Answering `0` would say this
    // machine booted a moment ago, which is a different claim from *the file was
    // not what was expected* and a far more misleading one.
    let (uptime_seconds, idle_seconds) = seconds(&text).ok_or_else(|| {
        MachineError::Refused(format!(
            "{UPTIME} holds {text:?}, which is not the two figures this expected, so nothing was \
             measured about how long this machine has been up"
        ))
    })?;

    let (boot_id, boot_id_unmeasured) = match std::fs::read_to_string(BOOT_ID) {
        // Trimmed and checked for emptiness: an empty string is not an
        // identifier, and reporting one would make the journal comparison
        // succeed against nothing.
        Ok(raw) if !raw.trim().is_empty() => (Some(raw.trim().to_string()), None),
        Ok(_) => (
            None,
            Some(format!("{BOOT_ID} is there and is empty, so it is not an identifier")),
        ),
        Err(e) => (None, Some(format!("{BOOT_ID} could not be read: {e}"))),
    };

    Ok(BootRead {
        boot_id,
        boot_id_unmeasured,
        uptime_seconds,
        idle_seconds,
        source: UPTIME.to_string(),
    })
}

/// `"12345.67 98765.43"` as whole seconds up and whole seconds idle.
///
/// **Truncated rather than rounded, and the fraction is dropped here.**
/// `rivet_core::value::Value` has no floating-point variant (`ADR-006` §4), and
/// hundredths of a second of uptime answer no question anyone asks — unlike
/// `get memory`'s pressure, where the fraction is the whole signal.
///
/// The idle figure is `None` rather than zero when it is absent, because zero
/// idle seconds is a real and very different reading.
fn seconds(text: &str) -> Option<(u64, Option<u64>)> {
    let mut figures = text.split_whitespace();
    let up = whole(figures.next()?)?;
    Some((up, figures.next().and_then(whole)))
}

/// One `12345.67` as `12345`.
fn whole(figure: &str) -> Option<u64> {
    figure.split_once('.').map_or(figure, |(w, _)| w).parse().ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The two figures, truncated to whole seconds.
    #[test]
    fn the_two_figures_become_whole_seconds() {
        assert_eq!(seconds("13415.19 52340.94\n"), Some((13415, Some(52340))));
    }

    /// **A machine with no idle figure reports none, not zero.** Zero idle
    /// seconds is a real reading — a machine that has been busy since it booted
    /// — and it must not be confused with a kernel that stated nothing.
    #[test]
    fn a_missing_idle_figure_is_absent_rather_than_zero() {
        assert_eq!(seconds("13415.19\n"), Some((13415, None)));
        assert_eq!(seconds("13415.19 0.00\n"), Some((13415, Some(0))));
    }

    /// **A file that is not two figures yields nothing**, so that `read`
    /// refuses rather than reporting an uptime of zero — which would say this
    /// machine booted a moment ago.
    #[test]
    fn a_file_that_is_not_an_uptime_is_refused_rather_than_read_as_zero() {
        for not_an_uptime in ["", "\n", "nonsense", "up 3 days"] {
            assert_eq!(seconds(not_an_uptime), None, "{not_an_uptime:?} parsed");
        }
    }

    /// An integer with no fraction is still an uptime — the kernel's format is
    /// documented with two decimals, but a reader that required them would fail
    /// on a kernel that stopped writing them.
    #[test]
    fn a_figure_with_no_fraction_still_parses() {
        assert_eq!(seconds("13415 52340"), Some((13415, Some(52340))));
    }

    /// **The read either answers or refuses, and it never reports a boot that
    /// began a moment ago.** This runs on whatever host the suite is on — Linux
    /// answers, Windows refuses — and asserts the property that holds on both.
    #[test]
    fn a_reading_on_this_host_is_either_an_answer_or_a_named_refusal() {
        match read() {
            Ok(reading) => {
                assert_eq!(reading.source, UPTIME);
                assert_eq!(
                    reading.boot_id.is_none(),
                    reading.boot_id_unmeasured.is_some(),
                    "exactly one of the boot id and its absence-reason is present"
                );
            }
            Err(refusal) => assert!(refusal.why().contains(UPTIME), "{}", refusal.why()),
        }
    }
}
