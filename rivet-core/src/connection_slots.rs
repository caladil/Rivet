//! **How many connections a LAN-facing daemon holds at once, and how many of
//! them one source may take** — the slot half of `cmd/rivet-mcpd`'s governor,
//! shared so every listener applies the same rule (`ADR-035` §12).
//!
//! A listener holds at most [`CONNECTION_SLOTS`] connections, and a single
//! source address at most [`PER_SOURCE_SLOTS`] of them, so [`RESERVED`] are
//! always free for everyone else. That reservation is what lets an operator
//! arriving from another address reach a machine one host is flooding. The
//! per-source share is set high because on a home LAN the busy client is usually
//! one workstation doing several things at once — an install, git, the fleet
//! page, an operator session — and it must not throttle itself; floods are a
//! rate limit's job (`crate::journal::TokenBuckets`), not this cap's. **No
//! source is privileged**: an address vouches for nothing (`ADR-025` §2).
//!
//! [`Slots`] is the bookkeeping; [`Governor`] puts it behind one lock with the
//! per-source rate limit and the collapsed refusal lines, so every listener
//! weighs a connection the same way before it costs a thread or a log line.
//! The clock is the caller's: every decision takes a monotonic `now_ms`.
//!
//! # What it still does not stop, stated plainly
//!
//! A flood from **many** source addresses on the same segment gets a bucket and
//! a share each, up to [`TRACKED_SOURCES`] buckets and the listener's pool. This
//! raises the bar from *one host* to *control of many addresses on the LAN*; it
//! is not a defence against an attacker who owns the segment, and `ADR-025` §2's
//! trust model never claimed to be.

use std::sync::{Arc, Mutex};

use crate::journal::{Admission, TokenBuckets};

/// The most connections a listener holds open at once.
pub const CONNECTION_SLOTS: usize = 16;

/// The most of [`CONNECTION_SLOTS`] a single source address may hold at once.
pub const PER_SOURCE_SLOTS: usize = 12;

/// Slots no single source can take, kept for everyone else.
pub const RESERVED: usize = CONNECTION_SLOTS - PER_SOURCE_SLOTS;

// A reservation that is the whole pool protects nobody, and one that is empty
// lets a single source take every slot.
const _: () = assert!(PER_SOURCE_SLOTS < CONNECTION_SLOTS);
const _: () = assert!(PER_SOURCE_SLOTS > 0);

/// Why a slot was not given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Full {
    /// Every slot is taken, across every source.
    Global,
    /// This source already holds its share.
    PerSource,
}

/// Which source holds how many of a listener's slots.
///
/// Bounded by the pool: an entry exists only while its source holds a slot, so
/// there are never more entries than slots, whatever addresses arrive.
#[derive(Debug, Clone)]
pub struct Slots<K> {
    max: usize,
    per_source: usize,
    held: Vec<(K, usize)>,
    live: usize,
}

impl<K: PartialEq + Copy> Default for Slots<K> {
    fn default() -> Self {
        Self::new(CONNECTION_SLOTS, PER_SOURCE_SLOTS)
    }
}

impl<K: PartialEq + Copy> Slots<K> {
    /// A pool of `max` slots, `per_source` of them at most to one source.
    #[must_use]
    pub const fn new(max: usize, per_source: usize) -> Self {
        Self { max, per_source, held: Vec::new(), live: 0 }
    }

    /// Slots held across every source.
    #[must_use]
    pub const fn live(&self) -> usize {
        self.live
    }

    /// Slots `src` holds.
    #[must_use]
    pub fn held_by(&self, src: K) -> usize {
        self.held.iter().find(|(s, _)| *s == src).map_or(0, |(_, n)| *n)
    }

    /// Whether any source holds a slot.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Give `src` one slot, or say why not. The pool is checked first, so a
    /// full listener says so whoever is asking.
    ///
    /// # Errors
    ///
    /// [`Full::Global`] when every slot is held; [`Full::PerSource`] when `src`
    /// already holds its share.
    pub fn take(&mut self, src: K) -> Result<(), Full> {
        if self.live >= self.max {
            return Err(Full::Global);
        }
        if self.held_by(src) >= self.per_source {
            return Err(Full::PerSource);
        }
        self.live += 1;
        if let Some(entry) = self.held.iter_mut().find(|(s, _)| *s == src) {
            entry.1 += 1;
        } else {
            self.held.push((src, 1));
        }
        Ok(())
    }

    /// `src` gives one slot back. A source holding none changes nothing.
    pub fn give_back(&mut self, src: K) {
        if let Some(index) = self.held.iter().position(|(s, _)| *s == src) {
            self.live = self.live.saturating_sub(1);
            let entry = &mut self.held[index];
            entry.1 -= 1;
            if entry.1 == 0 {
                self.held.swap_remove(index);
            }
        }
    }
}

/// Connections one source may open in a burst before the sustained rate bites.
///
/// **Generous on purpose:** a legitimate workstation running an install plus a
/// fleet sync opens a burst of connections, and the measured bursts were well
/// under this. A flood drains it in a fraction of a second and is then held to
/// [`SOURCE_PER_SECOND`].
pub const SOURCE_BURST: u32 = 32;

/// The sustained connection rate one source may run at, per second.
///
/// High enough that no legitimate use reaches it — each connection does real
/// work — and low enough that the 160-per-second flood measured against the
/// control plane is cut to a trickle.
pub const SOURCE_PER_SECOND: u32 = 8;

/// How many source addresses a bucket is kept for. Bounded, because the map is
/// keyed by something a caller influences and an attacker can spray addresses
/// across the segment; the least recently seen is evicted. The real population
/// on a home LAN is a few hosts.
pub const TRACKED_SOURCES: usize = 64;

/// How often a source over its share gets one line saying so, carrying the count
/// since the last. A line per refused connection would be the flood, filed by
/// the daemon against its own name; one per minute says the same in bounded
/// space (`ADR-022` §11 took the same shape for the ingest socket).
pub const REPORT_EVERY_MS: u64 = 60_000;

const _: () = assert!(SOURCE_BURST > 0);
const _: () = assert!(SOURCE_PER_SECOND > 0);
const _: () = assert!(TRACKED_SOURCES > 0);

/// Why a connection was turned away, for the one collapsed log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Over the per-source connection rate.
    Rate,
    /// This source already holds its share.
    PerSource,
    /// Every slot is taken, across every source.
    Global,
}

impl Reason {
    /// The clause the refusal line prints after the count.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Rate => "over the per-source connection rate",
            Self::PerSource => "already holding this source's share of connections",
            Self::Global => "all connection slots are taken",
        }
    }
}

/// What to do with one arriving connection.
#[derive(Debug)]
#[must_use]
pub enum Outcome<K: Copy + Eq> {
    /// Serve it. The guard releases the slot when the connection ends — every
    /// exit from the thread, including a panic.
    Admit(SlotGuard<K>),
    /// Drop it. Log one line iff `report` is `Some`, naming that many refusals
    /// for this source since the last line; `None` means the sentence is already
    /// written for this source inside the window and repeating it is the flood.
    Refuse { reason: Reason, report: Option<u64> },
}

/// The shared state behind [`Governor`]. Nothing under its lock can block or
/// panic: integer arithmetic and two short vectors.
#[derive(Debug)]
struct Inner<K: Copy + Eq> {
    rate: TokenBuckets<K>,
    /// A second per-source collapse, for handshake FAILURES logged after a
    /// connection the rate admitted fails at the TLS layer. The rate limit
    /// bounds how many a source opens, but each one it admits would still log a
    /// line, which under a sustained flood still rolls the journal. Uses only
    /// [`TokenBuckets::refuse`]; its rate fields are unused.
    fail_log: TokenBuckets<K>,
    slots: Slots<K>,
}

/// **Weighs every arriving connection before it costs a thread or a log line**:
/// the per-source rate first, then the pool and the source's share.
///
/// Cloneable: each clone shares the one state, so the accept loop and every
/// [`SlotGuard`] it hands a connection thread refer to the same counts.
#[derive(Debug, Clone)]
pub struct Governor<K: Copy + Eq>(Arc<Mutex<Inner<K>>>);

impl<K: Copy + Eq> Governor<K> {
    /// A governor over `max` slots, at most `per_source` of them to one source,
    /// with the shared per-source rate limit.
    #[must_use]
    pub fn new(max: usize, per_source: usize) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            rate: TokenBuckets::new(SOURCE_BURST, SOURCE_PER_SECOND, TRACKED_SOURCES, REPORT_EVERY_MS),
            fail_log: TokenBuckets::new(1, 1, TRACKED_SOURCES, REPORT_EVERY_MS),
            slots: Slots::new(max, per_source),
        })))
    }

    /// A poisoned lock means a thread panicked holding it. Nothing under this
    /// lock can panic, so the state is recovered rather than taking the
    /// listener down with it.
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner<K>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Decide what to do with a connection from `src` at monotonic `now_ms`.
    ///
    /// The rate is weighed first, so a flood never reaches the slot bookkeeping.
    /// A refusal for the pool or the share is fed back through the same
    /// per-source bucket ([`TokenBuckets::refuse`]) so it collapses onto the same
    /// one-line-per-window cadence.
    pub fn arrive(&self, src: K, now_ms: u64) -> Outcome<K> {
        let mut inner = self.inner();
        match inner.rate.admit(src, now_ms) {
            Admission::Accept => {}
            Admission::Report { dropped } => {
                return Outcome::Refuse { reason: Reason::Rate, report: Some(dropped) };
            }
            Admission::Drop => return Outcome::Refuse { reason: Reason::Rate, report: None },
        }
        if let Err(full) = inner.slots.take(src) {
            let reason = match full {
                Full::Global => Reason::Global,
                Full::PerSource => Reason::PerSource,
            };
            let report = match inner.rate.refuse(src, now_ms) {
                Admission::Report { dropped } => Some(dropped),
                _ => None,
            };
            return Outcome::Refuse { reason, report };
        }
        Outcome::Admit(SlotGuard { inner: Arc::clone(&self.0), src })
    }

    /// A connection from `src` failed its handshake. `Some(n)` when it is time
    /// to log — the first failure from a source and then at most one line per
    /// [`REPORT_EVERY_MS`], `n` being the failures since the last line.
    pub fn note_handshake_failure(&self, src: K, now_ms: u64) -> Option<u64> {
        match self.inner().fail_log.refuse(src, now_ms) {
            Admission::Report { dropped } => Some(dropped),
            _ => None,
        }
    }

    /// Slots held right now, across every source.
    #[must_use]
    pub fn live(&self) -> usize {
        self.inner().slots.live()
    }

    /// Whether no source holds a slot.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.inner().slots.is_empty()
    }
}

/// Holds one slot for the life of a connection and releases it on drop — a guard
/// rather than a decrement at the end of the thread, because there are several
/// ways out of that thread and only a destructor runs on all of them.
#[derive(Debug)]
pub struct SlotGuard<K: Copy + Eq> {
    inner: Arc<Mutex<Inner<K>>>,
    src: K,
}

impl<K: Copy + Eq> Drop for SlotGuard<K> {
    fn drop(&mut self) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slots
            .give_back(self.src);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn admit(g: &Governor<u8>, src: u8, now: u64) -> SlotGuard<u8> {
        match g.arrive(src, now) {
            Outcome::Admit(guard) => guard,
            Outcome::Refuse { reason, .. } => panic!("refused with {reason:?}"),
        }
    }

    /// **The governor's reservation holds**, and a finished connection gives
    /// its slot back to the pool and to its source.
    #[test]
    fn the_governor_caps_a_source_keeps_the_reservation_and_releases() {
        let g = Governor::new(CONNECTION_SLOTS, PER_SOURCE_SLOTS);
        let mut held: Vec<_> = (0..PER_SOURCE_SLOTS).map(|_| admit(&g, 1, 1)).collect();
        assert!(matches!(g.arrive(1, 1), Outcome::Refuse { reason: Reason::PerSource, .. }));
        for _ in 0..RESERVED {
            held.push(admit(&g, 2, 1));
        }
        assert!(matches!(g.arrive(3, 1), Outcome::Refuse { reason: Reason::Global, .. }));
        drop(held);
        assert_eq!(g.live(), 0, "a slot leaked");
        assert!(g.is_idle(), "a source entry leaked");
    }

    /// **A fast flood is cut by the rate before it reaches a slot**, and its
    /// refusals collapse to one report per window.
    #[test]
    fn a_rate_flood_is_bounded_and_reported_once_per_window() {
        let g = Governor::new(64, PER_SOURCE_SLOTS);
        let (mut admitted, mut reports) = (0, 0);
        for _ in 0..(SOURCE_BURST * 10) {
            match g.arrive(7u8, 1) {
                Outcome::Admit(guard) => {
                    admitted += 1;
                    std::mem::forget(guard);
                }
                Outcome::Refuse { report: Some(_), .. } => reports += 1,
                Outcome::Refuse { report: None, .. } => {}
            }
        }
        assert!(admitted <= PER_SOURCE_SLOTS, "admitted {admitted}");
        assert_eq!(reports, 1, "a flood produced {reports} log lines, not one");
    }

    /// Handshake failures collapse per source: the first at once, then one line
    /// per window carrying the count.
    #[test]
    fn handshake_failures_collapse_to_one_line_per_window() {
        let g: Governor<u8> = Governor::new(CONNECTION_SLOTS, PER_SOURCE_SLOTS);
        assert_eq!(g.note_handshake_failure(1, 1), Some(1));
        for _ in 0..500 {
            assert_eq!(g.note_handshake_failure(1, 1), None);
        }
        assert_eq!(g.note_handshake_failure(1, 1 + REPORT_EVERY_MS), Some(501));
        assert_eq!(g.note_handshake_failure(2, 1), Some(1));
    }

    /// **The reservation holds**: one source stops at its share, another still
    /// gets every reserved slot, and then the pool is full for anyone.
    #[test]
    fn one_source_is_capped_and_another_still_gets_in() {
        let mut slots: Slots<u8> = Slots::default();
        for _ in 0..PER_SOURCE_SLOTS {
            slots.take(1).expect("within the share");
        }
        assert_eq!(slots.take(1), Err(Full::PerSource));
        for _ in 0..RESERVED {
            slots.take(2).expect("a reserved slot");
        }
        assert_eq!(slots.take(3), Err(Full::Global));
        assert_eq!(slots.live(), CONNECTION_SLOTS);
    }

    /// A slot given back is free again, to the pool and to its source, and a
    /// pool given everything back holds no entries.
    #[test]
    fn a_slot_given_back_is_taken_again_and_nothing_leaks() {
        let mut slots: Slots<u8> = Slots::default();
        for _ in 0..PER_SOURCE_SLOTS {
            slots.take(1).expect("within the share");
        }
        slots.give_back(1);
        slots.take(1).expect("the slot given back");
        assert_eq!(slots.take(1), Err(Full::PerSource));
        for _ in 0..PER_SOURCE_SLOTS {
            slots.give_back(1);
        }
        assert_eq!(slots.live(), 0);
        assert!(slots.is_empty());
        // A source with nothing to give back takes nothing from the count.
        slots.give_back(9);
        assert_eq!(slots.live(), 0);
    }

    /// The ruled numbers: twelve of sixteen per source.
    #[test]
    fn a_source_may_hold_twelve_of_sixteen() {
        assert_eq!((PER_SOURCE_SLOTS, CONNECTION_SLOTS, RESERVED), (12, 16, 4));
    }
}
