//! **When the compositor's thread is held too long, the journal says so.**
//!
//! Everything this program does happens on one thread, which also flips the
//! screen and hands clients their buffers back. A pass of its loop that runs
//! long is a screen that stops moving for that long, and nothing else on the
//! board would record it: a client drawing at 30 fps just finds it has no
//! buffer, and a person watching sees a hitch.
//!
//! A pass normally takes a few milliseconds of work plus a wait of at most the
//! loop's 16 ms timeout, so [`STALL`] is far enough above that to mean
//! something held the thread. Each stall is logged with how long each part of
//! the pass took, which names what held it — at most one line per
//! [`QUIET_FOR`], with a count of the ones not logged, so a thread that stalls
//! on every pass cannot fill the journal.

use std::time::{Duration, Instant};

/// A pass of the loop that takes this long held the screen still.
pub const STALL: Duration = Duration::from_millis(100);

/// At most one stall line in this long.
pub const QUIET_FOR: Duration = Duration::from_secs(5);

/// Whether a stall is logged now, and how many were not since the last line.
#[derive(Debug, Default)]
pub struct StallLog {
    last_logged: Option<Instant>,
    unlogged: u32,
}

impl StallLog {
    /// A stall happened at `now`. `Some(n)` means log it, with `n` stalls
    /// since the last line that were not; `None` means stay quiet and count
    /// it.
    pub fn note(&mut self, now: Instant) -> Option<u32> {
        match self.last_logged {
            Some(last) if now.saturating_duration_since(last) < QUIET_FOR => {
                self.unlogged = self.unlogged.saturating_add(1);
                None
            }
            _ => {
                self.last_logged = Some(now);
                Some(std::mem::take(&mut self.unlogged))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The first stall is always logged, with nothing unlogged before it.
    #[test]
    fn the_first_stall_is_logged() {
        let mut log = StallLog::default();
        assert_eq!(log.note(Instant::now()), Some(0));
    }

    /// Stalls inside the quiet period are counted, not logged, and the count
    /// arrives with the next line after it.
    #[test]
    fn stalls_inside_the_quiet_period_are_counted_and_reported_with_the_next_line() {
        let mut log = StallLog::default();
        let start = Instant::now();
        assert_eq!(log.note(start), Some(0));
        for n in 1..=3 {
            assert_eq!(log.note(start + Duration::from_secs(n)), None);
        }
        assert_eq!(log.note(start + QUIET_FOR), Some(3));
        assert_eq!(log.note(start + QUIET_FOR + Duration::from_secs(1)), None);
    }

    /// A stall after a long quiet is logged at once, with nothing unlogged.
    #[test]
    fn a_stall_after_a_long_quiet_is_logged_with_nothing_unlogged() {
        let mut log = StallLog::default();
        let start = Instant::now();
        assert_eq!(log.note(start), Some(0));
        assert_eq!(log.note(start + Duration::from_secs(60)), Some(0));
    }
}
