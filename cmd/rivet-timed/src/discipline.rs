//! `ADR-021` §4.2.1, as a decision. **This is the module the ruling is about.**
//!
//! > **RULED 2026-08-23: always step. Rivet does not slew.**
//! >
//! > *"What is required with it, because always-stepping without these is a
//! > defect: a **noise floor** — below some offset, do not correct at all; a
//! > **rate limit** — do not correct more often than some interval. Without
//! > both, a flapping or hostile time server has the board stepping back and
//! > forth every poll and producing sawtooth timestamps. Neither number is ruled
//! > here; they are `rivet-timed`'s to choose and to state."*
//!
//! So they are chosen here, they are stated here, and the reasoning is in this
//! file rather than in a commit message — because the next person to change one
//! of them needs the argument, not the value.
//!
//! Nothing in this module performs anything. [`decide`] takes four numbers and
//! returns a [`Correction`], for the reason `rivet_store::clock::Floor` gives
//! about `ADR-021` §3's restore: *setting a clock is the one call in this
//! workspace a test cannot make*, because a test that succeeded would set the
//! clock of the machine running the suite. A decision, by contrast, is
//! exhaustively testable — and it is the part that can be wrong.
//!
//! # Why a threshold is absent, and its absence is the ruling rather than a gap
//!
//! `ADR-021` §4.2 originally described *step at boot, slew while running*, which
//! needs a number saying where "small" ends. §4.2.1 removed the question rather
//! than answering it, and the two reasons are worth keeping next to the code
//! they produced:
//!
//! * **Rivet has exactly one consumer of the wall clock** — the journal's `ts`
//!   field. Ordering is `boot` + `mono` (`ADR-022` §3.2) and every timer in
//!   `ADR-021` §5's table is monotonic, so a step scrambles displayed timestamps
//!   and changes nothing that decides anything.
//! * **`adjtime` adjusts the rate of `CLOCK_MONOTONIC` itself**, which is the
//!   clock those timers read, while `clock_settime` on the wall clock does not
//!   touch it at all. On this system **stepping is the less invasive
//!   operation**, which is the opposite of the usual advice.
//!
//! There is therefore no `adjtime` anywhere in this crate, and no threshold
//! constant. What follows is the two guards that ruling requires instead, and
//! a third that `ADR-021` §3 requires.
//!
//! # No single reading moves the clock
//!
//! A step is taken only when **two different servers agree** to within
//! [`NOISE_FLOOR_MS`]. One reading from one server is unauthenticated, and a
//! clock it stepped would be written to `/state/clock` by the supervisor's
//! next hourly write, where it becomes a floor that only rises. So a single
//! reading, however large, never moves the clock and so never raises the
//! floor. Because Rivet never slews, there is no second route by which one
//! reading could move the clock a little at a time.
//!
//! The tolerance is the noise floor itself, not a new number: two readings
//! closer than the floor are indistinguishable by the measure this module
//! already uses to call an offset worth correcting, and two further apart
//! cannot both be right to within it.

/// **The noise floor: one second.** Below this, nothing is corrected.
///
/// # The upper bound on this number
///
/// `ADR-021` §2 states the requirement in one sentence — *"seconds on a home
/// network, not the microsecond discipline a time server needs"*. A floor of one
/// second sits **inside** that requirement rather than at its edge: whatever
/// this constant leaves uncorrected is by construction smaller than the accuracy
/// the record asks for, so raising it towards five or ten seconds would start
/// spending the requirement itself to buy fewer steps.
///
/// # The lower bound on this number
///
/// It has to be comfortably larger than the error of the measurement that feeds
/// it, or the floor stops filtering noise and starts amplifying it. Rivet
/// takes **one unfiltered sample per server**, and a second server's only when
/// the first lies outside this floor — there is no filter, no
/// clock-discipline loop, no eight-sample register, because `ADR-021` §4 rules
/// *"one UDP round-trip and a clock adjustment"* — and the error of a single
/// SNTP sample is bounded by roughly half the round trip, which on a domestic
/// link to an anycast time server is tens of milliseconds rather than hundreds.
/// One second is an order of magnitude clear of that.
///
/// ~~**This is reasoning, not a measurement**, and the difference matters
/// (`AGENTS.md` §6). Nothing in Rivet has ever exchanged an SNTP packet, so
/// the round trip on the network this will actually run on is unknown~~ —
/// **corrected within the hour it was written, 2026-08-23**, and struck rather
/// than deleted because `AGENTS.md` §7 is about a claim of absence outliving the
/// absence. `rivet-timed --once` was run against all four addresses
/// `cmd/rivet-sysd/src/definitions.rs` ships, from the **development
/// host** under WSL:
///
/// | | Round trip | Stratum |
/// | :-- | --: | --: |
/// | `162.159.200.1` | 20 ms | 3 |
/// | `162.159.200.123` | 25 ms | 3 |
/// | `216.239.35.0` | 54 ms | 1 |
/// | `216.239.35.4` | 48 ms | 1 |
///
/// So the lower bound is now measured rather than assumed: half of the worst
/// round trip is 27 ms, and this constant is about forty times it. **The four
/// also agreed with each other to within 5 ms**, which is the stronger result —
/// four independent servers on two providers' networks corroborating one
/// offset is what says the measurement is not noise.
///
/// **What is still unmeasured is the thing that matters**: none of this was read
/// on a Raspberry Pi, over a board's link, through `bcmgenet`. The development
/// host is a fast wired machine. That is why
/// [`crate::sntp::Sample::round_trip_ms`] is reported on **every** sample: if a
/// board's journal shows round trips approaching this constant, the constant is
/// wrong and the journal is what will say so.
///
/// # What it costs, stated rather than buried
///
/// A Raspberry Pi's oscillator is not disciplined by anything between polls, so
/// it drifts. Drift accumulates until it crosses this floor and is then removed
/// in one step. The floor therefore sets the **size of the routine step**, not
/// whether one happens: a machine that is left running long enough will step by
/// about one second, repeatedly, for ever. That is intended. It is one visible
/// journal record per crossing, and the alternative — a smaller floor — buys a
/// more accurate clock nothing needs at the price of more discontinuities in the
/// one field that suffers from them.
pub const NOISE_FLOOR_MS: u64 = 1_000;

/// **The rate limit: one hour between steps**, and not applied to the first step
/// of a run.
///
/// # What it is defending against
///
/// §4.2.1's own words: *"a flapping or hostile time server has the board
/// stepping back and forth every poll and producing sawtooth timestamps"*. A
/// single flapping server is already kept out by corroboration, since its
/// readings move nothing until a second server agrees; this limit is for what
/// corroboration cannot catch, such as two servers that move together. It does
/// not detect anything, it **bounds how much damage a wrong answer can do to
/// the record**. At one hour, a boot's journal
/// carries at most one discontinuity per hour, each one an explicit record
/// naming the wall clock on both sides of it, so a reader can always reconstruct
/// the timeline. Without the limit the same attacker produces a discontinuity
/// every poll, and there is no reconstruction because there is no stable
/// interval between them.
///
/// # Why an hour rather than something shorter
///
/// It has to be **longer than the poll interval** or it never binds. With
/// [`crate::run::POLL_INTERVAL_MS`] at about seventeen minutes, anything under
/// that is a constant that does nothing, and choosing one would be the kind of
/// guard that looks present and is inert.
///
/// # Why an hour rather than something longer
///
/// The limit must never be what delays a *genuine* correction. Genuine
/// corrections arrive at the rate the machine drifts, and drift at the tens of
/// parts per million an ordinary crystal shows takes hours to cross
/// [`NOISE_FLOOR_MS`]. An hourly cap is therefore slack against real drift and
/// tight against an adversary, which is the only shape a rate limit is allowed
/// to have. *(The drift figure is a general property of uncompensated crystals
/// and is **not** measured on a Raspberry Pi 4 — see this module's report. If a
/// board turns out to drift a second in under an hour, this constant is what
/// starts refusing real corrections, and the `rate limit` records are what would
/// show it.)*
///
/// # The first step of a run is exempt, and that is `ADR-021` §4.2
///
/// *"At boot, step freely. Nothing depends on continuity across a power
/// cycle."* A cold Pi is fifty-six years out; a rate limit that applied to the
/// first correction would leave it fifty-six years out for an hour, which is the
/// whole failure this service exists to fix. [`decide`] takes
/// `last_step_mono_ms: Option<u64>` and `None` means *nothing has been stepped
/// yet in this run*, which is the exemption expressed as a type rather than as a
/// flag somebody can forget to clear.
///
/// # The hole this leaves, named rather than papered over
///
/// **The limiter is process-local, and a restart resets it.** `rivet-timed` is
/// a supervised service; if it exits, `ADR-017` §3.5 restarts it, and the new
/// process's first step is exempt again. So a service that crashed once per
/// minute would step once per minute. What bounds that is §3.5's own backoff and
/// give-up threshold — five attempts and then `Failed` — rather than anything
/// here, and the reason it is not fixed here is that the fix is a *persisted*
/// last-step time, which would be a second writer of `/state` and a card write
/// per step. `crate::run` explains why this binary writes to `/state` not at
/// all.
pub const MIN_STEP_INTERVAL_MS: u64 = 60 * 60 * 1_000;

/// What [`decide`] concluded. A value, so the whole space is testable without a
/// clock to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Correction {
    /// Move the wall clock. `ADR-021` §4.2.1 — the only correction Rivet has.
    ///
    /// **The three numbers this carries are the ones `ADR-021` §4.2 requires be
    /// made visible** rather than moving the clock silently under running
    /// software, and together with the record's own `mono` and `boot` columns
    /// they are what lets a reader reconstruct the wall time of every *earlier*
    /// record in the same boot: `ts_true ≈ ts_recorded + (to_ms - from_ms)` for
    /// anything stamped before this record's `mono`. One mechanism, two
    /// requirements — `ADR-022` §3.2's *"a boot identifier plus a monotonic
    /// offset makes ordering **always** recoverable, whatever the wall clock
    /// did"* is the other half of the same idea.
    Step {
        /// What the wall clock read immediately before the step.
        from_ms: u64,
        /// What it was set to.
        to_ms: u64,
        /// The server's offset, kept because `to_ms - from_ms` loses its sign
        /// in `u64` arithmetic and a backward step is the interesting one.
        offset_ms: i64,
    },
    /// The offset is inside [`NOISE_FLOOR_MS`]. **The normal steady state**, and
    /// the arm that makes always-stepping safe rather than twitchy.
    WithinFloor { offset_ms: i64 },
    /// A step is warranted and [`MIN_STEP_INTERVAL_MS`] has not elapsed.
    RateLimited {
        offset_ms: i64,
        /// How long until a step would be allowed. Reported so that a reader
        /// meeting several of these in a row can tell a rate limit from a stuck
        /// service.
        next_allowed_in_ms: u64,
    },
    /// A step is warranted by one server and no different server answered to
    /// corroborate it, so nothing moves.
    Uncorroborated { offset_ms: i64 },
    /// A step is warranted by one server and a different server's reading is
    /// [`NOISE_FLOOR_MS`] or more away from it, so nothing moves.
    Disagreed {
        offset_ms: i64,
        /// The second server's reading.
        other_offset_ms: i64,
    },
    /// Applying the offset would put the wall clock before the Unix epoch.
    ///
    /// **Not a hypothetical**: a machine at three seconds past the epoch —
    /// which is what a Raspberry Pi 4 actually came up at (the 1970-clock
    /// finding, 2026-08-23) — that is told it is four seconds fast has
    /// nowhere to go. `AGENTS.md` §5, refuse
    /// rather than guess: saturating at zero would *succeed* and set a clock to
    /// a time no server said.
    ///
    /// **This is not a sanity bound on the value**, and the distinction is
    /// deliberate. It refuses arithmetic that has no answer, not an answer this
    /// code dislikes; a server claiming the year 2200 is accepted and stepped
    /// to, because nothing in the record defines "absurd" and inventing a
    /// definition here would pin every Rivet machine's future to a constant
    /// nobody ruled.
    BeforeTheEpoch { offset_ms: i64, from_ms: u64 },
}

impl Correction {
    /// A short, stable word for the journal's `outcome` field.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Step { .. } => "stepped",
            Self::WithinFloor { .. } => "within-floor",
            Self::RateLimited { .. } => "rate-limited",
            Self::Uncorroborated { .. } => "uncorroborated",
            Self::Disagreed { .. } => "disagreed",
            Self::BeforeTheEpoch { .. } => "before-the-epoch",
        }
    }

    /// What the clock must be set to, or `None` when it must be left alone.
    ///
    /// `None` for every arm but [`Correction::Step`], which is the point: each
    /// guard is an arm that answers `None` for an offset large enough to have
    /// been a step.
    ///
    /// **Reached only from the tests**, and kept for that: it is what
    /// `nothing_but_a_step_moves_the_clock_and_a_step_moves_it_by_the_offset`
    /// asserts over the whole grid, which is the strongest statement this module
    /// makes. `crate::run` destructures [`Correction::Step`] instead, because it
    /// needs all three of its fields.
    #[allow(dead_code)]
    pub const fn set_to(self) -> Option<u64> {
        match self {
            Self::Step { to_ms, .. } => Some(to_ms),
            Self::WithinFloor { .. }
            | Self::RateLimited { .. }
            | Self::Uncorroborated { .. }
            | Self::Disagreed { .. }
            | Self::BeforeTheEpoch { .. } => None,
        }
    }

    /// The offset the guards were applied to: the first server's reading, or,
    /// once a second server has agreed with it, the midpoint of the two.
    pub const fn offset_ms(self) -> i64 {
        match self {
            Self::Step { offset_ms, .. }
            | Self::WithinFloor { offset_ms }
            | Self::RateLimited { offset_ms, .. }
            | Self::Uncorroborated { offset_ms }
            | Self::Disagreed { offset_ms, .. }
            | Self::BeforeTheEpoch { offset_ms, .. } => offset_ms,
        }
    }
}

/// Whether two readings agree: they are less than [`NOISE_FLOOR_MS`] apart,
/// the floor's own exclusive boundary.
pub fn agree(a_ms: i64, b_ms: i64) -> bool {
    (i128::from(a_ms) - i128::from(b_ms)).unsigned_abs() < u128::from(NOISE_FLOOR_MS)
}

/// The midpoint of two readings, rounded toward negative infinity. Neither
/// server chooses the number alone.
fn midpoint(a_ms: i64, b_ms: i64) -> i64 {
    // The mean of two `i64`s always fits an `i64`; the `i128` only keeps the
    // sum from overflowing on its way there.
    let mean = (i128::from(a_ms) + i128::from(b_ms)).div_euclid(2);
    i64::try_from(mean).unwrap_or(a_ms)
}

/// `ADR-021` §4.2.1 in one function: step, unless one of the two guards says
/// not to.
///
/// The order of the guards is load-bearing and is the cheaper-first order:
/// **the noise floor is checked before the rate limit**, so an offset that was
/// never going to be corrected does not consume the rate limiter's attention or
/// produce a record implying a correction was withheld. A reader meeting a
/// `rate-limited` record therefore knows a real step was refused, which is the
/// only way that record is worth writing.
///
/// * `now_wall_ms` — what this machine's wall clock reads, from
///   [`crate::kernel::Kernel::wall_ms`], read **after** the exchange so that the
///   offset applies to it.
/// * `offset_ms` — the first server's [`crate::sntp::Sample::offset_ms`].
/// * `second_ms` — a **different** server's reading, or `None` when no second
///   server answered. The caller asks for one only when the first reading lies
///   outside the floor.
/// * `now_mono_ms` — the monotonic clock, which is what both intervals in this
///   crate are measured on (`ADR-021` §5). Using the wall clock here would be
///   self-inflicted: this function's own output moves it.
/// * `last_step_mono_ms` — `None` until this run has stepped once, which is
///   `ADR-021` §4.2's *"at boot, step freely"*.
///
/// After the floor and the rate limit comes corroboration: no second reading
/// is [`Correction::Uncorroborated`], one [`NOISE_FLOOR_MS`] or more away is
/// [`Correction::Disagreed`], and two that [`agree`] are replaced by their
/// midpoint, which then meets the floor again and the epoch check. So **a
/// single reading never reaches [`Correction::Step`]**.
pub fn decide(
    now_wall_ms: u64,
    offset_ms: i64,
    second_ms: Option<i64>,
    now_mono_ms: u64,
    last_step_mono_ms: Option<u64>,
) -> Correction {
    if offset_ms.unsigned_abs() < NOISE_FLOOR_MS {
        return Correction::WithinFloor { offset_ms };
    }

    if let Some(last) = last_step_mono_ms {
        // `saturating_sub` rather than a subtraction: the monotonic clock does
        // not go backwards, but `Kernel::now_ms` answers 0 when
        // `clock_gettime` fails, and a guard that underflowed there would
        // silently permit every step it exists to refuse.
        let since = now_mono_ms.saturating_sub(last);
        if since < MIN_STEP_INTERVAL_MS {
            return Correction::RateLimited {
                offset_ms,
                next_allowed_in_ms: MIN_STEP_INTERVAL_MS - since,
            };
        }
    }

    let Some(other_offset_ms) = second_ms else {
        return Correction::Uncorroborated { offset_ms };
    };
    if !agree(offset_ms, other_offset_ms) {
        return Correction::Disagreed {
            offset_ms,
            other_offset_ms,
        };
    }
    let offset_ms = midpoint(offset_ms, other_offset_ms);
    if offset_ms.unsigned_abs() < NOISE_FLOOR_MS {
        return Correction::WithinFloor { offset_ms };
    }

    let Some(to_ms) = now_wall_ms.checked_add_signed(offset_ms) else {
        // Either before the epoch, or past what a `u64` of milliseconds holds —
        // which is about 584 million years and cannot be reached from a wall
        // clock plus an offset this crate accepted. So the reachable half is the
        // one the variant is named for.
        return Correction::BeforeTheEpoch {
            offset_ms,
            from_ms: now_wall_ms,
        };
    };

    Correction::Step {
        from_ms: now_wall_ms,
        to_ms,
        offset_ms,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A plausible wall time, hand-composed and arbitrary. `AGENTS.md` §8.3
    /// keeps the dates of real events out of tracked files.
    const A_PLAUSIBLE_MS: u64 = 1_700_000_000_000;

    /// What a Raspberry Pi 4 actually came up at — the 1970-clock finding, 2026-08-23 records the
    /// stamp `1970-01-01T00:00:03` on every record the first board wrote.
    const WHAT_A_COLD_PI_BELIEVES_MS: u64 = 3_000;

    // -- the boot case -------------------------------------------------------

    /// **The whole point of the service.** A board at the epoch is told it is
    /// fifty-six years behind and moves, in one step, with nothing rate-limiting
    /// it — `ADR-021` §4.2, *"at boot, step freely"*.
    #[test]
    fn a_cold_board_steps_fifty_six_years_forward_on_its_first_sample() {
        let offset = i64::try_from(A_PLAUSIBLE_MS - WHAT_A_COLD_PI_BELIEVES_MS)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let c = decide(WHAT_A_COLD_PI_BELIEVES_MS, offset, Some(offset), 12_000, None);
        assert_eq!(
            c,
            Correction::Step {
                from_ms: WHAT_A_COLD_PI_BELIEVES_MS,
                to_ms: A_PLAUSIBLE_MS,
                offset_ms: offset,
            }
        );
        assert_eq!(c.set_to(), Some(A_PLAUSIBLE_MS));
    }

    /// ...and the record it produces carries **both** wall times, which is what
    /// lets a reader recover the true time of every earlier record in the boot.
    #[test]
    fn a_step_reports_the_clock_on_both_sides_of_itself() {
        let Correction::Step { from_ms, to_ms, .. } =
            decide(WHAT_A_COLD_PI_BELIEVES_MS, 1_699_999_997_000, Some(1_699_999_997_000), 12_000, None)
        else {
            unreachable!("a fifty-six year offset did not step")
        };
        assert_eq!(from_ms, WHAT_A_COLD_PI_BELIEVES_MS);
        assert_eq!(to_ms, A_PLAUSIBLE_MS);
        // The correction a reader would apply to an earlier record.
        assert_eq!(to_ms - from_ms, 1_699_999_997_000);
    }

    // -- the noise floor -----------------------------------------------------

    /// The steady state: a clock that is close enough is left exactly alone, in
    /// both directions.
    #[test]
    fn an_offset_inside_the_floor_corrects_nothing_either_way() {
        for offset in [0, 1, 500, 999, -1, -500, -999] {
            let c = decide(A_PLAUSIBLE_MS, offset, Some(offset), 0, None);
            assert_eq!(c, Correction::WithinFloor { offset_ms: offset }, "{offset}");
            assert_eq!(c.set_to(), None, "{offset} moved the clock");
        }
    }

    /// The boundary, stated exactly, so that a change to the constant has to
    /// change a test rather than sliding past one.
    #[test]
    fn the_floor_is_exclusive_and_one_second_exactly_is_a_step() {
        assert_eq!(NOISE_FLOOR_MS, 1_000);
        assert_eq!(
            decide(A_PLAUSIBLE_MS, 999, Some(999), 0, None),
            Correction::WithinFloor { offset_ms: 999 }
        );
        assert!(matches!(
            decide(A_PLAUSIBLE_MS, 1_000, Some(1_000), 0, None),
            Correction::Step { .. }
        ));
        assert!(matches!(
            decide(A_PLAUSIBLE_MS, -1_000, Some(-1_000), 0, None),
            Correction::Step { .. }
        ));
    }

    /// **The assumption the floor rests on, as an assertion.** The floor filters
    /// measurement noise only while it is far larger than the measurement error,
    /// which for one unfiltered SNTP sample is bounded by about half the round
    /// trip.
    ///
    /// **The number below is an observation, not a guess** (`AGENTS.md` §4 —
    /// label fixtures by provenance). `rivet-timed --once` against all four
    /// shipped addresses from the development host, 2026-08-23, gave round trips
    /// of 20, 25, 48 and 54 ms. The constant here is `200`, which is four times
    /// the worst of them, so it stays a pessimistic stand-in for a board's link
    /// rather than becoming a restatement of the fastest network Rivet will
    /// ever be on.
    ///
    /// If a board ever reports round trips near [`NOISE_FLOOR_MS`], this test is
    /// where the reasoning is written down and the constant is what is wrong.
    #[test]
    fn the_noise_floor_is_far_above_a_plausible_round_trip() {
        let worst_measured_ms = 54_u64;
        let a_pessimistic_round_trip_ms = 200_u64;
        assert!(
            a_pessimistic_round_trip_ms > worst_measured_ms,
            "the pessimistic figure has stopped being pessimistic"
        );
        assert!(
            NOISE_FLOOR_MS >= a_pessimistic_round_trip_ms * 5,
            "a floor of {NOISE_FLOOR_MS} ms does not clear half of a {a_pessimistic_round_trip_ms} \
             ms round trip by enough to be filtering noise rather than tracking it"
        );
    }

    // -- the rate limit ------------------------------------------------------

    /// The flapping server, which is the failure §4.2.1 names. Two large and
    /// opposite offsets a poll apart produce **one** step, not two.
    #[test]
    fn a_server_that_flaps_moves_the_clock_once_and_then_is_refused() {
        let first = decide(A_PLAUSIBLE_MS, 5_000, Some(5_000), 100_000, None);
        let Correction::Step { to_ms, .. } = first else {
            unreachable!("a five second offset did not step")
        };
        assert_eq!(to_ms, A_PLAUSIBLE_MS + 5_000);

        // Seventeen minutes later the same server says the opposite.
        let second = decide(to_ms, -5_000, Some(-5_000), 100_000 + 1_024_000, Some(100_000));
        assert_eq!(second.set_to(), None, "the sawtooth was not refused");
        assert!(matches!(second, Correction::RateLimited { .. }), "{second:?}");
    }

    /// The boundary of the limit, and that it is measured from the last **step**
    /// rather than from the last poll.
    #[test]
    fn a_step_is_allowed_exactly_one_hour_after_the_previous_one() {
        assert_eq!(MIN_STEP_INTERVAL_MS, 3_600_000);
        let last = 500_000_u64;
        assert!(matches!(
            decide(A_PLAUSIBLE_MS, 5_000, Some(5_000), last + MIN_STEP_INTERVAL_MS - 1, Some(last)),
            Correction::RateLimited { .. }
        ));
        assert!(matches!(
            decide(A_PLAUSIBLE_MS, 5_000, Some(5_000), last + MIN_STEP_INTERVAL_MS, Some(last)),
            Correction::Step { .. }
        ));
    }

    #[test]
    fn a_refusal_says_how_long_is_left_so_a_reader_can_tell_it_from_a_stuck_service() {
        let last = 0_u64;
        let c = decide(A_PLAUSIBLE_MS, 60_000, Some(60_000), 600_000, Some(last));
        assert_eq!(
            c,
            Correction::RateLimited {
                offset_ms: 60_000,
                next_allowed_in_ms: MIN_STEP_INTERVAL_MS - 600_000,
            }
        );
    }

    /// The rate limit never sees an offset the floor already dismissed, so a
    /// quiet machine's records never claim a correction was withheld.
    #[test]
    fn the_floor_is_checked_before_the_rate_limit() {
        let c = decide(A_PLAUSIBLE_MS, 10, Some(10), 1_000, Some(999));
        assert_eq!(c, Correction::WithinFloor { offset_ms: 10 });
    }

    /// A monotonic clock this crate could not read answers zero
    /// (`Kernel::now_ms`), and the guard must not underflow into permitting
    /// every step it exists to refuse.
    #[test]
    fn a_monotonic_clock_that_reads_backwards_still_refuses_rather_than_permits() {
        let c = decide(A_PLAUSIBLE_MS, 5_000, Some(5_000), 0, Some(1_000_000));
        assert!(matches!(c, Correction::RateLimited { .. }), "{c:?}");
    }

    // -- corroboration -------------------------------------------------------

    /// **A single reading never steps the clock**, however large, including on
    /// a cold board's first poll where the rate limit is exempt.
    #[test]
    fn a_single_reading_never_steps_the_clock() {
        for offset in [1_000_i64, -1_000, 5_000, 1_699_999_997_000] {
            let c = decide(WHAT_A_COLD_PI_BELIEVES_MS, offset, None, 12_000, None);
            assert_eq!(c, Correction::Uncorroborated { offset_ms: offset });
            assert_eq!(c.set_to(), None);
        }
    }

    /// The tolerance is the noise floor, with the floor's own exclusive
    /// boundary: 999 ms apart agree, 1000 ms apart do not.
    #[test]
    fn two_readings_agree_only_when_closer_than_the_floor() {
        assert!(agree(5_000, 4_001));
        assert!(agree(-5_000, -5_999));
        assert!(!agree(5_000, 4_000));
        assert!(!agree(5_000, -5_000));
        assert!(!agree(i64::MAX, i64::MIN), "the difference must not overflow");
        assert_eq!(
            decide(A_PLAUSIBLE_MS, 5_000, Some(4_000), 0, None),
            Correction::Disagreed {
                offset_ms: 5_000,
                other_offset_ms: 4_000
            }
        );
    }

    /// Two agreeing readings step the clock by their midpoint, so neither
    /// server picks the number alone.
    #[test]
    fn two_agreeing_readings_step_by_their_midpoint() {
        assert_eq!(
            decide(A_PLAUSIBLE_MS, 5_000, Some(4_002), 0, None),
            Correction::Step {
                from_ms: A_PLAUSIBLE_MS,
                to_ms: A_PLAUSIBLE_MS + 4_501,
                offset_ms: 4_501,
            }
        );
    }

    /// A midpoint that falls back inside the floor corrects nothing: the two
    /// servers together say the clock is close enough.
    #[test]
    fn a_midpoint_inside_the_floor_corrects_nothing() {
        assert_eq!(
            decide(A_PLAUSIBLE_MS, 1_000, Some(990), 0, None),
            Correction::WithinFloor { offset_ms: 995 }
        );
    }

    /// The rate limit is reported before corroboration is asked about, so a
    /// refusal record means a corroborated step was refused or would have been
    /// asked for, never that a second server was missing.
    #[test]
    fn the_rate_limit_is_checked_before_corroboration() {
        assert!(matches!(
            decide(A_PLAUSIBLE_MS, 5_000, None, 1_000, Some(0)),
            Correction::RateLimited { .. }
        ));
    }

    // -- the arithmetic ------------------------------------------------------

    /// A cold board told it is *fast* has nowhere to go, and says so rather than
    /// saturating at the epoch.
    #[test]
    fn an_offset_that_would_land_before_the_epoch_is_refused_rather_than_clamped() {
        let c = decide(WHAT_A_COLD_PI_BELIEVES_MS, -4_000, Some(-4_000), 0, None);
        assert_eq!(
            c,
            Correction::BeforeTheEpoch {
                offset_ms: -4_000,
                from_ms: WHAT_A_COLD_PI_BELIEVES_MS,
            }
        );
        assert_eq!(c.set_to(), None);
    }

    /// **No sanity bound is applied to the value**, and this test exists so that
    /// adding one silently is impossible. A server claiming a century from now
    /// is stepped to: nothing in the record defines "absurd", and the ADR that
    /// would have to is `ADR-021`.
    #[test]
    fn a_time_far_in_the_future_is_stepped_to_because_nothing_defines_absurd() {
        let a_century_ms = 100 * 365 * 24 * 60 * 60 * 1_000_i64;
        let c = decide(A_PLAUSIBLE_MS, a_century_ms, Some(a_century_ms), 0, None);
        assert!(matches!(c, Correction::Step { .. }), "{c:?}");
    }

    /// Every arm renders a distinct, countable word for the journal.
    #[test]
    fn every_outcome_has_its_own_word() {
        let words = [
            decide(A_PLAUSIBLE_MS, 0, Some(0), 0, None).kind(),
            decide(A_PLAUSIBLE_MS, 5_000, Some(5_000), 0, None).kind(),
            decide(A_PLAUSIBLE_MS, 5_000, Some(5_000), 0, Some(0)).kind(),
            decide(3_000, -4_000, Some(-4_000), 0, None).kind(),
            decide(A_PLAUSIBLE_MS, 5_000, None, 0, None).kind(),
            decide(A_PLAUSIBLE_MS, 5_000, Some(0), 0, None).kind(),
        ];
        let mut sorted = words;
        sorted.sort_unstable();
        let before = sorted.len();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), before, "{words:?}");
    }

    /// The property, over a grid rather than at points: **no arm other than
    /// `Step` ever moves the clock**, and every `Step` moves it by exactly the
    /// offset the server reported.
    #[test]
    fn nothing_but_a_step_moves_the_clock_and_a_step_moves_it_by_the_offset() {
        let walls = [0, 3_000, 1_000_000, A_PLAUSIBLE_MS];
        let offsets = [-3_600_000_i64, -1_001, -1_000, -999, 0, 999, 1_000, 1_001, 3_600_000];
        let monos = [0_u64, 1, MIN_STEP_INTERVAL_MS - 1, MIN_STEP_INTERVAL_MS];
        for wall in walls {
            for offset in offsets {
                for mono in monos {
                    for last in [None, Some(0_u64)] {
                        let c = decide(wall, offset, Some(offset), mono, last);
                        assert_eq!(c.offset_ms(), offset);
                        match c.set_to() {
                            None => assert!(
                                !matches!(c, Correction::Step { .. }),
                                "wall={wall} offset={offset} gave {c:?}"
                            ),
                            Some(to) => {
                                assert_eq!(
                                    i128::from(to) - i128::from(wall),
                                    i128::from(offset),
                                    "wall={wall} offset={offset} mono={mono} last={last:?}"
                                );
                                assert!(offset.unsigned_abs() >= NOISE_FLOOR_MS);
                            }
                        }
                    }
                }
            }
        }
    }
}
