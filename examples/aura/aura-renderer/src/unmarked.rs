//! **When to say that `secondary` holds a reading this build draws no mark
//! for**, and when not to.
//!
//! `SPEC-04` §7 asks for a mark beside a reading whose state is not `ok`, and
//! the weather panel draws none (see the call site in `main.rs` for why). That
//! is worth one journal line — but the condition is a STATE of the slot, not an
//! event. A producer that re-sends the same `unavailable` reading, or any other
//! slot changing, says nothing new about it, and the journal is a rolling
//! buffer that a line repeated per card pushes everything else out of.
//!
//! So this journals a change of state: once when the slot starts holding an
//! unmarked reading (or holds one of a different state), and once when it stops.
//! A once-a-minute roll-up with a count was the other shape on offer, and is
//! the wrong one here: it would still write a line a minute for a condition
//! that has not changed, and the count it carries measures how often a producer
//! re-sends, which says nothing about the panel.

/// What changed, for the caller to journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The slot now holds a reading in this state, which the panel draws no
    /// mark for.
    Arrived(&'static str),
    /// The slot no longer holds an unmarked reading; this was the last state
    /// said.
    Cleared(&'static str),
}

/// The state last said, so the next observation can tell a change from a
/// repeat.
#[derive(Debug, Default)]
pub struct Watch {
    said: Option<&'static str>,
}

impl Watch {
    /// Observe the slot: `Some(state)` when it holds an unmarked reading,
    /// `None` when it holds none (an `ok` reading, or no card at all).
    ///
    /// Returns a [`Change`] only when this differs from what was last said.
    pub fn observe(&mut self, now: Option<&'static str>) -> Option<Change> {
        if now == self.said {
            return None;
        }
        match (std::mem::replace(&mut self.said, now), now) {
            (_, Some(state)) => Some(Change::Arrived(state)),
            (Some(was), None) => Some(Change::Cleared(was)),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The defect: a stream of identical `unavailable` readings journalled
    /// one line per card. It says once.
    #[test]
    fn a_stream_of_identical_unavailable_readings_is_said_once() {
        let mut watch = Watch::default();
        let said: Vec<Change> = (0..1000)
            .filter_map(|_| watch.observe(Some("unavailable")))
            .collect();
        assert_eq!(said, vec![Change::Arrived("unavailable")]);
    }

    /// Recovery is said once, and a second outage is said again.
    #[test]
    fn a_recovery_is_said_once_and_a_second_outage_is_said_again() {
        let mut watch = Watch::default();
        assert_eq!(watch.observe(Some("unavailable")), Some(Change::Arrived("unavailable")));
        assert_eq!(watch.observe(None), Some(Change::Cleared("unavailable")));
        assert_eq!(watch.observe(None), None);
        assert_eq!(watch.observe(Some("unavailable")), Some(Change::Arrived("unavailable")));
    }

    /// A different unmarked state is a change, and is said.
    #[test]
    fn a_reading_moving_from_stale_to_unavailable_is_said() {
        let mut watch = Watch::default();
        assert_eq!(watch.observe(Some("stale")), Some(Change::Arrived("stale")));
        assert_eq!(watch.observe(Some("stale")), None);
        assert_eq!(watch.observe(Some("unavailable")), Some(Change::Arrived("unavailable")));
    }

    /// A slot that never held an unmarked reading says nothing at all.
    #[test]
    fn a_slot_that_was_always_fine_says_nothing() {
        let mut watch = Watch::default();
        assert!((0..10).all(|_| watch.observe(None).is_none()));
    }
}
