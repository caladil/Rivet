//! Per-source admission for the accept loop — `ADR-035` §12.
//!
//! # The two findings this answers
//!
//! A Rivet is reached over the control plane and nothing else (`ADR-035`), so
//! that is also what a hostile host on the LAN can reach. Two defects followed
//! from an accept loop with **one global cap of [`MAX_CONNECTIONS`] and no
//! per-source bound**:
//!
//! 1. **Lockout.** A connection was accounted, logged and given a thread before
//!    the TLS handshake, so keyless connections (refused at the handshake, no
//!    key needed) churning the global slots crowded out authenticated callers.
//!    A flood from one host made the board unmanageable while it ran.
//! 2. **Journal churn.** Every refusal wrote two journal lines, turning the
//!    rolling ~8 MiB journal (`ADR-022` §8) over in minutes and erasing history.
//!
//! # What this does about them
//!
//! Every connection is weighed **before a thread is spawned or a line is
//! logged**, keyed by the peer's IP address, by `rivet_core::connection_slots`'
//! [`Governor`] — the same one `rivet-proxyd` uses:
//!
//! * a per-source **rate limit** cuts a flood to a trickle, and because a
//!   refusal collapses into one line per [`REPORT_EVERY_MS`], stops the journal
//!   churn;
//! * a per-source **concurrency cap** of [`PER_SOURCE_SLOTS`] of
//!   [`MAX_CONNECTIONS`] — so one host that opens connections and holds them
//!   (rate alone does not stop that: opening costs a token, holding costs none)
//!   can never take the last slots. [`RESERVED`] are always kept for everyone
//!   else, which is what guarantees an operator from another address can still
//!   reach a board that something is flooding.
//!
//! **No source is privileged.** The journal exempts uid 0 because root can write
//! the file directly; there is no equivalent here — an IP address vouches for
//! nothing (`ADR-025` §2).

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Instant;

pub use rivet_core::connection_slots::REPORT_EVERY_MS;

/// The most connections the daemon holds open at once.
///
/// **Sixteen, a floor on usability rather than a guess at load.** One operator
/// runs one `rivet mcp` session; the fleet page holds one per device it shows; a
/// git clone is one more. Sixteen is several times any of that and small enough
/// that the thread and descriptor cost is bounded at something a Pi can hold.
pub const MAX_CONNECTIONS: usize = rivet_core::connection_slots::CONNECTION_SLOTS;

/// The most of [`MAX_CONNECTIONS`] a single source address may hold at once:
/// twelve of sixteen, for the reasons `rivet_core::connection_slots` gives.
pub const PER_SOURCE_SLOTS: usize = rivet_core::connection_slots::PER_SOURCE_SLOTS;

/// Slots no single source can take, kept for everyone else.
pub const RESERVED: usize = rivet_core::connection_slots::RESERVED;

/// The governor this daemon's accept loop weighs every connection with.
pub type Governor = rivet_core::connection_slots::Governor<IpAddr>;

/// What the governor decided about one connection.
pub type Outcome = rivet_core::connection_slots::Outcome<IpAddr>;

/// A governor over this daemon's [`MAX_CONNECTIONS`] and [`PER_SOURCE_SLOTS`].
#[must_use]
pub fn governor() -> Governor {
    Governor::new(MAX_CONNECTIONS, PER_SOURCE_SLOTS)
}

/// Milliseconds on `CLOCK_MONOTONIC` since the first call.
///
/// **Monotonic, never the wall clock.** `ADR-021` §3 corrects a Pi's clock
/// backwards at boot, which a wall-clock bucket would read as a free refill or a
/// minute-long stall. `Instant` is monotonic.
pub fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::connection_slots::Reason;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 168, 50, last))
    }

    /// **The reservation holds on this daemon's numbers**: one source cannot
    /// take the last slots, and an operator from another address still gets in.
    #[test]
    fn one_source_is_capped_and_another_still_gets_in() {
        let g = governor();
        let mut held = Vec::new();
        for _ in 0..PER_SOURCE_SLOTS {
            match g.arrive(ip(10), 1) {
                Outcome::Admit(guard) => held.push(guard),
                Outcome::Refuse { reason, .. } => panic!("refused with {reason:?}"),
            }
        }
        assert!(matches!(g.arrive(ip(10), 1), Outcome::Refuse { reason: Reason::PerSource, .. }));
        for _ in 0..RESERVED {
            match g.arrive(ip(99), 1) {
                Outcome::Admit(guard) => held.push(guard),
                Outcome::Refuse { reason, .. } => panic!("a reserved slot refused with {reason:?}"),
            }
        }
        assert!(matches!(g.arrive(ip(100), 1), Outcome::Refuse { reason: Reason::Global, .. }));
        drop(held);
        assert_eq!(g.live(), 0, "a slot leaked");
    }

    /// Every source address is weighed — there is no privileged one.
    #[test]
    fn the_zero_address_is_not_privileged() {
        let g = governor();
        let zero = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
        for _ in 0..PER_SOURCE_SLOTS {
            if let Outcome::Admit(guard) = g.arrive(zero, 1) {
                std::mem::forget(guard);
            }
        }
        assert!(matches!(g.arrive(zero, 1), Outcome::Refuse { .. }));
    }
}
