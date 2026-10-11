//! The loop: poll, decide, act, say what happened. No decisions of its own.
//!
//! Everything that *decides* is [`crate::discipline`], which is pure;
//! everything that *touches the world* is [`crate::kernel`] and
//! [`crate::sntp`], which are ports. This module is the wiring, and it is
//! reachable from a test end to end because both of those are faked
//! (`crate::testkernel`).
//!
//! # Why this binary never writes `/state/clock`, and why that is `ADR-011`
//!
//! `rivet_store::clock` owns the stored wall clock and
//! `cmd/rivet-sysd/src/clock.rs` is the one process that writes it —
//! hourly, guarded by a floor it established at boot. It would be easy for this
//! binary to write a corrected time there too, and it deliberately does not:
//! **two writers of one file is two answers to "what does the card say", and
//! `ADR-011` is entirely about not having those.** The supervisor's next hourly
//! write picks up whatever this process corrected the clock to, because it reads
//! the clock rather than remembering it (`AGENTS.md` §5).
//!
//! **The cost of that is real and is worth naming, because it is a finding
//! rather than a detail.** `rivet_store::clock::Origin::Synchronised` still
//! has no producer — and the reason has *changed*. It used to be that nothing in
//! Rivet could synchronise a clock. Now something can, and the stored stamp
//! still says `unsynchronised`, because **the process that synchronises and the
//! process that writes the file are different processes with no channel between
//! them.** Under-claiming is the safe direction (that type's own documentation
//! makes the argument), so the behaviour is correct as it stands; what is not
//! recorded anywhere is which of the two should change. See this work's report.
//!
//! # Where the servers come from
//!
//! Their **shape** is settled in [`crate::args`] — addresses, never names, and
//! that module carries the whole argument. Their **values** are in
//! `cmd/rivet-sysd/src/definitions.rs`, because `ADR-017` §4.1 puts
//! *what a service is* in the definition and there is nowhere else for them to
//! live yet: `ADR-021` §7 says the servers are configuration, and `ADR-007`'s
//! schema has no `[time]` table for them. That gap is stated in the definition
//! and in this work's report; it is not papered over with a default here.
//!
//! # What happens on `SIGTERM`
//!
//! Nothing, on purpose. The supervisor clears a child's signal mask between
//! `fork` and `exec` (measured — `cmd/rivet-sysd/src/kernel.rs`), so
//! `SIGTERM` arrives with its default disposition and terminates this process
//! out of its sleep. There is no clock write to flush and no socket to close
//! gracefully. `ADR-021` §3's *"on clean shutdown"* write is a different
//! question, belongs to a different process, and is **open** — see
//! `cmd/rivet-sysd/src/clock.rs`.

use std::net::SocketAddr;

use crate::args::Options;
use crate::discipline::{Correction, decide};
use crate::journal::{Journal, Severity, Value, rfc3339};
use crate::kernel::Kernel;
use crate::sntp::{QueryFailure, Sntp};

/// How long between polls once a server has answered. About seventeen minutes.
///
/// `1024` seconds is NTP's own `maxpoll` default expressed in the units the rest
/// of this crate uses, and it is chosen for two reasons that are not "because
/// NTP does it":
///
/// * **It has to be shorter than the time the machine takes to drift across
///   [`crate::discipline::NOISE_FLOOR_MS`]**, or the clock spends most of its
///   life outside the floor rather than inside it. At the tens of parts per
///   million an uncompensated crystal shows, a second of drift takes hours.
/// * **It has to be shorter than
///   [`crate::discipline::MIN_STEP_INTERVAL_MS`]**, or the rate limit is a
///   guard that can never fire and the sawtooth §4.2.1 names is unbounded
///   after all. Seventeen minutes against one hour leaves the limiter with
///   something to refuse.
///
/// It is also politeness: one packet every seventeen minutes to a public time
/// server is within what every operator of one asks for.
pub const POLL_INTERVAL_MS: u64 = 1_024_000;

/// How long between polls **before** any server has ever answered.
///
/// `ADR-017` §3.2 puts the obligation on the service: *"a service that needs
/// something not yet ready **retries**"*, and `ADR-021` §7 says so about this
/// one by name. A board whose network comes up thirty seconds after this
/// process starts must not then sit at 1970 for seventeen minutes, so the
/// unsynchronised state polls at NTP's `minpoll` instead — sixty-four seconds.
///
/// The asymmetry is deliberate and is the whole of the convergence model: fast
/// while it is wrong, slow once it is right.
pub const RETRY_INTERVAL_MS: u64 = 64_000;

/// What one poll produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// A server answered and the discipline decided what to do about it.
    Answered(Correction),
    /// A server answered and this machine has no readable wall clock to apply
    /// the offset to.
    ///
    /// A separate variant rather than a [`Correction`] arm, because it is not a
    /// decision the discipline made — it is a machine the discipline was never
    /// asked about. Folding it into `WithinFloor` would put a fabricated
    /// decision in the outcome, which is `AGENTS.md` §5's guess wearing a
    /// success's clothes.
    NoWallClock { offset_ms: i64 },
    /// Every server was tried and none produced a usable sample.
    NoServerAnswered,
}

/// The service, as it is held across a run.
pub struct Timed {
    servers: Vec<SocketAddr>,
    poll_interval_ms: u64,
    /// Which server the next poll starts from. Advanced every poll so that a
    /// list of four is four servers rather than one server and three standbys.
    next_server: usize,
    /// `ADR-021` §4.2's *"at boot, step freely"*, as state: `None` means this
    /// run has not stepped, and [`crate::discipline::decide`] exempts that case
    /// from the rate limit.
    last_step_mono_ms: Option<u64>,
    /// Whether this run has reached a settled answer: a server answered and the
    /// discipline either needed no second opinion or had one. An answer left
    /// uncorroborated or disagreed with does not settle it, so a first boot
    /// that reaches one server keeps polling at the retry interval until a
    /// second answers. Only changes the *volume* of the journal and the poll
    /// interval — never a correction.
    settled: bool,
}

impl Timed {
    pub fn new(options: &Options) -> Self {
        Self {
            servers: options.servers.clone(),
            poll_interval_ms: options.poll_interval_ms,
            next_server: 0,
            last_step_mono_ms: None,
            settled: false,
        }
    }

    /// How long to sleep before the next poll.
    pub const fn next_interval_ms(&self) -> u64 {
        if self.settled {
            self.poll_interval_ms
        } else {
            RETRY_INTERVAL_MS
        }
    }

    /// One poll: ask servers in rotation until one answers, then act.
    pub fn poll_once(
        &mut self,
        k: &mut dyn Kernel,
        sntp: &mut dyn Sntp,
        journal: &mut Journal,
    ) -> PollOutcome {
        let corr = journal.next_correlation();

        let Some((server, sample)) = self.take_a_sample(sntp, k, journal, &corr) else {
            return PollOutcome::NoServerAnswered;
        };

        // A reading outside the floor would move the clock, and no single
        // reading may (`ADR-021` §3): ask a different server for a second.
        // A reading inside the floor moves nothing and needs none.
        let second = if sample.offset_ms.unsigned_abs() >= crate::discipline::NOISE_FLOOR_MS {
            self.take_a_second_opinion(sntp, server)
        } else {
            None
        };

        // Read **after** the exchange, because the offset is relative to the
        // clock at the moment of the exchange and this is the closest a reader
        // of the port can stand to it. The residual — the microseconds between
        // `rsntp`'s last `SystemTime::now()` and this call — is four orders of
        // magnitude below `discipline::NOISE_FLOOR_MS` and cannot change any
        // decision.
        let Some(now_wall_ms) = k.wall_ms() else {
            journal.emit(
                k,
                Severity::Error,
                &corr,
                "DEGRADED a server answered and this machine has no readable wall clock, so there \
                 is nothing to apply the offset to and nothing to correct",
                vec![
                    ("server".to_string(), Value::Str(server.to_string())),
                    ("offset_ms".to_string(), Value::Int(sample.offset_ms)),
                ],
            );
            // A sample was obtained, so the network works and the retry interval
            // should relax; there is simply no clock on this machine to fix.
            self.settled = true;
            return PollOutcome::NoWallClock {
                offset_ms: sample.offset_ms,
            };
        };

        let now_mono_ms = k.now_ms();
        let correction = decide(
            now_wall_ms,
            sample.offset_ms,
            second.as_ref().map(|(_, s)| s.offset_ms),
            now_mono_ms,
            self.last_step_mono_ms,
        );

        let mut common = vec![
            ("server".to_string(), Value::Str(server.to_string())),
            ("outcome".to_string(), Value::Str(correction.kind().to_string())),
            // From the DECISION rather than from the sample, so the number in
            // the record is the one the guards were applied to: the first
            // server's reading, or the midpoint once a second server agreed.
            // `server_offset_ms` below is the first server's own reading.
            ("offset_ms".to_string(), Value::Int(correction.offset_ms())),
            ("server_offset_ms".to_string(), Value::Int(sample.offset_ms)),
            (
                "round_trip_ms".to_string(),
                Value::Int(i64::try_from(sample.round_trip_ms).unwrap_or(i64::MAX)),
            ),
            ("stratum".to_string(), Value::Int(i64::from(sample.stratum))),
        ];
        if let Some((other, other_sample)) = &second {
            common.push(("other_server".to_string(), Value::Str(other.to_string())));
            common.push(("other_offset_ms".to_string(), Value::Int(other_sample.offset_ms)));
            common.push((
                "other_round_trip_ms".to_string(),
                Value::Int(i64::try_from(other_sample.round_trip_ms).unwrap_or(i64::MAX)),
            ));
        }

        match correction {
            Correction::Step {
                from_ms,
                to_ms,
                offset_ms,
            } => self.step(k, journal, &corr, common, from_ms, to_ms, offset_ms, now_mono_ms),
            Correction::WithinFloor { .. } => {
                // The steady state, and the only record this service writes
                // when everything is well. `Debug` after the first one, so a
                // healthy board is not writing an `info` every seventeen
                // minutes for ever.
                let first = !self.settled;
                journal.emit(
                    k,
                    if first { Severity::Info } else { Severity::Debug },
                    &corr,
                    "the wall clock agrees with the server to within the noise floor, so nothing \
                     is corrected (ADR-021 §4.2.1)",
                    common,
                );
                if first {
                    self.say_what_the_card_will_still_record(k, journal, &corr);
                }
            }
            Correction::RateLimited {
                next_allowed_in_ms, ..
            } => {
                let mut fields = common;
                fields.push((
                    "next_allowed_in_ms".to_string(),
                    Value::Int(i64::try_from(next_allowed_in_ms).unwrap_or(i64::MAX)),
                ));
                journal.emit(
                    k,
                    Severity::Warn,
                    &corr,
                    "a correction large enough to step was REFUSED by the rate limit, so the wall \
                     clock is unchanged. One of these is a clock that drifted twice in an hour; \
                     several in a row is a server whose answers are moving about, which is what \
                     the limit exists to keep out of the record (ADR-021 §4.2.1)",
                    fields,
                );
            }
            Correction::BeforeTheEpoch { from_ms, .. } => {
                let mut fields = common;
                fields.push(("was".to_string(), Value::Str(rfc3339(from_ms))));
                journal.emit(
                    k,
                    Severity::Warn,
                    &corr,
                    "the server says this machine is FAST by more than the whole time since the \
                     Unix epoch, so the correction has no answer and nothing is done. Either the \
                     reply is not honest or this machine's clock is at the epoch and the offset is \
                     negative, which no correct server produces",
                    fields,
                );
            }
            Correction::Uncorroborated { .. } => {
                journal.emit(
                    k,
                    Severity::Warn,
                    &corr,
                    "a correction large enough to step was WITHHELD because no different server \
                     answered to corroborate it, so the wall clock is unchanged. A single reading \
                     never moves the clock or the stored floor (ADR-021 §3); it is tried again \
                     at the next poll",
                    common,
                );
                return PollOutcome::Answered(correction);
            }
            Correction::Disagreed { .. } => {
                journal.emit(
                    k,
                    Severity::Warn,
                    &corr,
                    "a correction large enough to step was WITHHELD because a second server \
                     disagreed by the noise floor or more, so the wall clock is unchanged. Two \
                     servers must agree before the clock is stepped (ADR-021 §3); it is tried \
                     again at the next poll",
                    common,
                );
                return PollOutcome::Answered(correction);
            }
        }

        self.settled = true;
        PollOutcome::Answered(correction)
    }

    /// Ask every server whose address differs from `first`, in list order
    /// after it, until one answers. `None` when none does, including a list
    /// that names only one address.
    ///
    /// Its failures are not journaled one by one: the poll's own record says
    /// the step was withheld for want of a second server, which is the news.
    fn take_a_second_opinion(
        &self,
        sntp: &mut dyn Sntp,
        first: SocketAddr,
    ) -> Option<(SocketAddr, crate::sntp::Sample)> {
        let count = self.servers.len();
        let at = self.servers.iter().position(|s| *s == first).unwrap_or(0);
        for offset in 1..count {
            let Some(server) = self.servers.get((at + offset) % count).copied() else {
                continue;
            };
            if server == first {
                continue;
            }
            if let Ok(sample) = sntp.query(server) {
                return Some((server, sample));
            }
        }
        None
    }

    /// Ask each server in turn, starting from the rotation point, until one
    /// answers. One summary record when none does.
    fn take_a_sample(
        &mut self,
        sntp: &mut dyn Sntp,
        k: &mut dyn Kernel,
        journal: &mut Journal,
        corr: &str,
    ) -> Option<(SocketAddr, crate::sntp::Sample)> {
        let count = self.servers.len();
        let start = self.next_server;
        self.next_server = if count == 0 { 0 } else { (start + 1) % count };

        let mut failures: Vec<(SocketAddr, QueryFailure)> = Vec::with_capacity(count);
        for offset in 0..count {
            let Some(server) = self.servers.get((start + offset) % count).copied() else {
                continue;
            };
            match sntp.query(server) {
                Ok(sample) => return Some((server, sample)),
                Err(why) => failures.push((server, why)),
            }
        }

        // One record per poll rather than one per server: on a board whose
        // network is not up, every server fails the same way every time, and a
        // record each would be four lines of identical news. The counts keep the
        // distinction that matters — *nothing answered* and *something answered
        // badly* are different faults (`crate::sntp::QueryFailure`).
        let unreachable = failures
            .iter()
            .filter(|(_, f)| matches!(f, QueryFailure::Unreachable { .. }))
            .count();
        let refused = failures.len() - unreachable;
        let first = failures
            .first()
            .map_or_else(String::new, |(s, f)| format!("{s}: {f}"));
        // The countable form of the same thing, so a reader can tally the three
        // kinds without parsing a sentence — `crate::sntp::QueryFailure::kind`.
        let first_kind = failures.first().map_or("none", |(_, f)| f.kind());
        journal.emit(
            k,
            Severity::Warn,
            corr,
            "no time server answered usably, so the wall clock is unchanged and this will be tried \
             again. Normal until the network is up — ADR-017 §3.2 makes a service retry rather \
             than declare a dependency, and ADR-021 §7 says so about this one by name",
            vec![
                (
                    "tried".to_string(),
                    Value::Int(i64::try_from(failures.len()).unwrap_or(i64::MAX)),
                ),
                (
                    "unreachable".to_string(),
                    Value::Int(i64::try_from(unreachable).unwrap_or(i64::MAX)),
                ),
                (
                    "not_usable".to_string(),
                    Value::Int(i64::try_from(refused).unwrap_or(i64::MAX)),
                ),
                ("detail".to_string(), Value::Str(first)),
                ("detail_kind".to_string(), Value::Str(first_kind.to_string())),
            ],
        );
        None
    }

    /// Move the clock, then say so — **in that order**, so that the record's own
    /// `ts` column carries the corrected time and its `mono` column carries the
    /// instant the discontinuity happened.
    ///
    /// That ordering is the whole mechanism `ADR-021` §4.2 asks for and it does
    /// two jobs at once. The step is *visible* rather than silent; and because
    /// the record names the wall clock on both sides of itself and the monotonic
    /// instant between them, **every earlier record of the same boot can have
    /// its true wall time reconstructed** — add `moved_ms` to any `ts` stamped
    /// at a lower `mono` within the same `boot`. `ADR-022` §3.2 is the other
    /// half of that: `boot` + `mono` make ordering recoverable whatever the wall
    /// clock did.
    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        k: &mut dyn Kernel,
        journal: &mut Journal,
        corr: &str,
        common: Vec<(String, Value)>,
        from_ms: u64,
        to_ms: u64,
        offset_ms: i64,
        now_mono_ms: u64,
    ) {
        if let Err(e) = k.set_wall_ms(to_ms) {
            let mut fields = common;
            fields.push(("wanted".to_string(), Value::Str(rfc3339(to_ms))));
            fields.push(("errno".to_string(), Value::Int(i64::from(e.0))));
            fields.push(("detail".to_string(), Value::Str(e.to_string())));
            journal.emit(
                k,
                Severity::Error,
                corr,
                "DEGRADED the wall clock could not be set, so this machine keeps the time it had \
                 and every record it writes is stamped from it. Setting the clock needs \
                 CAP_SYS_TIME, so EPERM here means this process is not running with it",
                fields,
            );
            // **The rate limit is not armed by a step that did not happen.**
            // Arming it here would leave a machine that lacks CAP_SYS_TIME
            // refusing its own next attempt for an hour, and the machine would
            // then be wrong *and* quiet about it.
            return;
        }

        self.last_step_mono_ms = Some(now_mono_ms);

        let mut fields = common;
        fields.push(("was".to_string(), Value::Str(rfc3339(from_ms))));
        fields.push(("now".to_string(), Value::Str(rfc3339(to_ms))));
        fields.push(("moved_ms".to_string(), Value::Int(offset_ms)));
        fields.push((
            "reconstruct".to_string(),
            Value::Str(
                "add moved_ms to the ts of any record of this boot stamped at a lower mono"
                    .to_string(),
            ),
        ));
        journal.emit(
            k,
            Severity::Info,
            corr,
            "THE WALL CLOCK IS STEPPED. Rivet always steps and never slews (ADR-021 §4.2.1), so \
             this is a discontinuity in ts and in nothing else: ordering is boot+mono and every \
             timer is monotonic (ADR-022 §3.2, ADR-021 §5)",
            fields,
        );
    }

    /// Said once, on the first answer of a run: the clock has now been checked
    /// against something outside this machine, and `/state/clock` will go on
    /// saying otherwise.
    ///
    /// It is here rather than left implicit because the opposite claim is
    /// written into the supervisor's boot records and into
    /// `rivet_store::clock::Origin`, and a reader who finds both without this
    /// one would conclude the correction never happened.
    fn say_what_the_card_will_still_record(
        &self,
        k: &mut dyn Kernel,
        journal: &mut Journal,
        corr: &str,
    ) {
        journal.emit(
            k,
            Severity::Info,
            corr,
            "this machine's wall clock has now been checked against an external time source. The \
             stored clock on /state will still be written as `unsynchronised`: rivet-sysd \
             is its only writer (ADR-011) and cannot observe this process, so it under-claims \
             rather than asserting a provenance it did not see",
            vec![],
        );
    }

    /// Poll, sleep, repeat.
    ///
    /// `max_polls` is `None` for the service — which is the loop that never
    /// returns — and `Some(1)` for `--once`. **It is a count rather than a
    /// boolean** so that the unbounded loop itself is reachable from a test:
    /// with `Some(4)` a suite exercises the sleeping, the rotation and the
    /// interval relaxing in microseconds, and a `bool` would have left the only
    /// loop in this binary untested. `AGENTS.md` §4 — a suite that cannot be run
    /// rots, and one that cannot reach the code is worse.
    ///
    /// The sleep is on the port, so a seventeen-minute interval costs a test
    /// nothing. On a board it is `nanosleep(2)`, which `SIGTERM` interrupts —
    /// see this module's opening note.
    pub fn run(
        &mut self,
        k: &mut dyn Kernel,
        sntp: &mut dyn Sntp,
        journal: &mut Journal,
        max_polls: Option<u64>,
    ) -> PollOutcome {
        let mut polls = 0_u64;
        loop {
            let outcome = self.poll_once(k, sntp, journal);
            polls = polls.saturating_add(1);
            if max_polls.is_some_and(|max| polls >= max) {
                return outcome;
            }
            k.sleep_ms(self.next_interval_ms());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::args::Options;
    use crate::kernel::{ConsoleForm, Errno};
    use crate::sntp::Sample;
    use crate::testkernel::{FakeKernel, FakeSntp};

    const A_PLAUSIBLE_MS: u64 = 1_700_000_000_000;
    const WHAT_A_COLD_PI_BELIEVES_MS: u64 = 3_000;

    fn server(n: u8) -> SocketAddr {
        SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, n)),
            123,
        )
    }

    fn options(servers: &[SocketAddr]) -> Options {
        Options {
            servers: servers.to_vec(),
            poll_interval_ms: POLL_INTERVAL_MS,
            once: true,
            console: ConsoleForm::Jsonl,
        }
    }

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::ACTOR.to_string())
    }

    fn sample(offset_ms: i64) -> Sample {
        Sample {
            offset_ms,
            round_trip_ms: 24,
            stratum: 2,
        }
    }

    fn said(k: &FakeKernel, needle: &str) -> bool {
        k.records.iter().any(|r| r.msg.contains(needle))
    }

    // -- the whole thing, from the outside -----------------------------------

    /// **The change this crate exists for.** A board at the epoch takes one
    /// sample and believes the right year.
    #[test]
    fn a_cold_board_that_reaches_a_server_ends_the_poll_with_the_right_time() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        let offset = i64::try_from(A_PLAUSIBLE_MS - WHAT_A_COLD_PI_BELIEVES_MS).unwrap();
        let mut s = FakeSntp::always(Ok(sample(offset)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1), server(2)]));

        let outcome = t.poll_once(&mut k, &mut s, &mut j);

        assert!(matches!(outcome, PollOutcome::Answered(Correction::Step { .. })));
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS]);
        assert_eq!(k.wall_ms, Some(A_PLAUSIBLE_MS));
    }

    /// The step record is the thing `ADR-021` §4.2 requires, and it carries all
    /// three of the values a reader needs: the wall clock before, the wall clock
    /// after, and — as the record's own column — the monotonic instant.
    #[test]
    fn the_step_record_carries_both_wall_times_and_the_monotonic_instant() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.mono_ms = 12_500;
        let offset = i64::try_from(A_PLAUSIBLE_MS - WHAT_A_COLD_PI_BELIEVES_MS).unwrap();
        let mut s = FakeSntp::always(Ok(sample(offset)));
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);

        let step = k
            .records
            .iter()
            .find(|r| r.msg.contains("THE WALL CLOCK IS STEPPED"))
            .expect("no step record");
        assert_eq!(step.sev, Severity::Info);
        let field = |name: &str| {
            step.fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            field("was"),
            Some(Value::Str(rfc3339(WHAT_A_COLD_PI_BELIEVES_MS)))
        );
        assert_eq!(field("now"), Some(Value::Str(rfc3339(A_PLAUSIBLE_MS))));
        assert_eq!(field("moved_ms"), Some(Value::Int(offset)));
        assert!((step.mono - 12.5).abs() < 1e-9, "{}", step.mono);
        // ...and the record is stamped from the CORRECTED clock, which is what
        // proves the step happened before the record rather than after it.
        assert_eq!(step.ts.as_deref(), Some(rfc3339(A_PLAUSIBLE_MS).as_str()));
    }

    /// A reader with only the journal can put an earlier record back where it
    /// belongs. This is the reconstruction the record promises, performed.
    #[test]
    fn an_earlier_records_true_wall_time_is_recoverable_from_the_step_record() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        let mut j = journal();
        // A record written before the correction, stamped from the wrong clock.
        let corr = j.next_correlation();
        j.emit(&mut k, Severity::Info, &corr, "something earlier", vec![]);
        let earlier = k.records[0].clone();

        let offset = i64::try_from(A_PLAUSIBLE_MS - WHAT_A_COLD_PI_BELIEVES_MS).unwrap();
        let mut s = FakeSntp::always(Ok(sample(offset)));
        k.mono_ms = 30_000;
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);

        let step = k
            .records
            .iter()
            .find(|r| r.msg.contains("STEPPED"))
            .expect("no step record");
        let Some((_, Value::Int(moved))) =
            step.fields.iter().find(|(n, _)| n == "moved_ms").cloned()
        else {
            unreachable!("the step record carries no moved_ms")
        };
        assert!(step.mono > earlier.mono, "the step must be the later record");
        // The reconstruction, done the way the record's own `reconstruct` field
        // describes it.
        let recovered = WHAT_A_COLD_PI_BELIEVES_MS
            .checked_add_signed(moved)
            .expect("the correction does not apply");
        assert_eq!(earlier.ts.as_deref(), Some(rfc3339(WHAT_A_COLD_PI_BELIEVES_MS).as_str()));
        assert_eq!(rfc3339(recovered), rfc3339(A_PLAUSIBLE_MS));
    }

    // -- corroboration: two servers must agree ------------------------------

    /// **A single reading never moves the clock.** A first boot that reaches one
    /// server keeps its time, says why, and goes on polling at the fast retry
    /// interval until a second server answers.
    #[test]
    fn a_single_reachable_server_never_moves_the_clock() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        let offset = i64::try_from(A_PLAUSIBLE_MS - WHAT_A_COLD_PI_BELIEVES_MS).unwrap();
        let mut s = FakeSntp::always(Ok(sample(offset)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1)]));

        t.poll_once(&mut k, &mut s, &mut j);

        assert!(k.attempted_wall_sets.is_empty(), "one server moved the clock");
        assert_eq!(t.next_interval_ms(), RETRY_INTERVAL_MS);
        assert!(said(&k, "WITHHELD"), "{:?}", k.records);
    }

    /// The same, with a second server listed and not answering.
    #[test]
    fn a_second_server_that_does_not_answer_leaves_the_clock_alone() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![
            Ok(sample(5_000)),
            Err(QueryFailure::Unreachable {
                detail: "no route".to_string(),
            }),
        ]);
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);
        assert!(k.attempted_wall_sets.is_empty(), "an uncorroborated reading moved the clock");
        assert_eq!(s.asked, vec![server(1), server(2)]);
    }

    /// **Two servers that disagree by the noise floor or more move nothing.**
    #[test]
    fn two_servers_that_disagree_by_the_floor_leave_the_clock_alone() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![Ok(sample(5_000)), Ok(sample(4_000))]);
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);
        assert!(k.attempted_wall_sets.is_empty(), "a disagreement moved the clock");
        assert!(said(&k, "disagreed"), "{:?}", k.records);
    }

    /// Two servers that agree inside the floor step the clock by the midpoint of
    /// their readings, and the step record names both.
    #[test]
    fn two_servers_that_agree_step_the_clock_and_the_record_names_both() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![Ok(sample(5_000)), Ok(sample(4_002))]);
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS + 4_501]);
        let step = k
            .records
            .iter()
            .find(|r| r.msg.contains("THE WALL CLOCK IS STEPPED"))
            .expect("no step record");
        let field = |n: &str| step.fields.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        assert_eq!(field("server"), Some(Value::Str(server(1).to_string())));
        assert_eq!(field("server_offset_ms"), Some(Value::Int(5_000)));
        assert_eq!(field("other_server"), Some(Value::Str(server(2).to_string())));
        assert_eq!(field("other_offset_ms"), Some(Value::Int(4_002)));
    }

    /// "Two different servers" means two addresses: a list naming one address
    /// twice is one server, and never corroborates itself.
    #[test]
    fn one_address_listed_twice_never_corroborates_itself() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(5_000)));
        let mut j = journal();
        Timed::new(&options(&[server(1), server(1)])).poll_once(&mut k, &mut s, &mut j);
        assert!(k.attempted_wall_sets.is_empty(), "one address corroborated itself");
        assert_eq!(s.asked, vec![server(1)]);
    }

    /// A reading inside the floor needs no second opinion: it moves nothing, so
    /// the steady state costs one packet a poll.
    #[test]
    fn a_reading_inside_the_floor_asks_no_second_server() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(120)));
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);
        assert_eq!(s.asked, vec![server(1)]);
    }

    // -- the two guards, through the loop ------------------------------------

    #[test]
    fn a_clock_that_is_close_enough_is_left_alone_and_the_record_says_so() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(120)));
        let mut j = journal();
        Timed::new(&options(&[server(1)])).poll_once(&mut k, &mut s, &mut j);
        assert!(k.wall_sets.is_empty(), "a clock inside the floor was moved");
        assert!(said(&k, "within the noise floor"), "{:?}", k.records);
    }

    /// **The sawtooth, refused.** Servers that agree on +5 s and then on −5 s
    /// move the clock once; the second answer is rate-limited and the record
    /// says so at `warn`.
    #[test]
    fn a_flapping_server_moves_the_clock_once_and_the_refusal_is_loud() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![
            Ok(sample(5_000)),
            Ok(sample(5_000)),
            Ok(sample(-5_000)),
        ]);
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1), server(2)]));

        t.poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS + 5_000]);

        k.mono_ms += POLL_INTERVAL_MS;
        t.poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets.len(), 1, "the sawtooth reached the clock");
        let refusal = k
            .records
            .iter()
            .find(|r| r.msg.contains("REFUSED by the rate limit"))
            .expect("no refusal record");
        assert_eq!(refusal.sev, Severity::Warn);
    }

    /// ...and an hour later the same offset is applied, so the limit delays a
    /// genuine correction rather than losing it.
    #[test]
    fn a_correction_refused_by_the_limit_is_taken_once_the_hour_has_passed() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(5_000)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1), server(2)]));

        t.poll_once(&mut k, &mut s, &mut j);
        k.mono_ms += crate::discipline::MIN_STEP_INTERVAL_MS - 1;
        t.poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets.len(), 1);
        k.mono_ms += 1;
        t.poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets.len(), 2);
    }

    // -- failures ------------------------------------------------------------

    /// The ordinary state of a board whose network is not up yet. One record per
    /// poll, not one per server, and it says the retry is expected.
    #[test]
    fn a_board_with_no_network_says_so_once_per_poll_and_carries_on() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        let mut s = FakeSntp::always(Err(QueryFailure::Unreachable {
            detail: "network is unreachable".to_string(),
        }));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1), server(2), server(3)]));

        assert_eq!(t.poll_once(&mut k, &mut s, &mut j), PollOutcome::NoServerAnswered);
        assert_eq!(s.asked.len(), 3, "not every server was tried");
        let summary: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg.contains("no time server answered"))
            .collect();
        assert_eq!(summary.len(), 1, "{:?}", k.records);
        assert_eq!(summary[0].sev, Severity::Warn);
        assert!(k.wall_sets.is_empty());
    }

    /// The two failures an operator would act on differently are counted apart:
    /// *nothing answered* and *something answered badly*.
    #[test]
    fn the_summary_distinguishes_an_unreachable_server_from_an_unusable_reply() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![
            Err(QueryFailure::Unreachable {
                detail: "no route".to_string(),
            }),
            Err(QueryFailure::Refused {
                detail: "kiss of death".to_string(),
            }),
        ]);
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2)])).poll_once(&mut k, &mut s, &mut j);
        let summary = k
            .records
            .iter()
            .find(|r| r.msg.contains("no time server answered"))
            .expect("no summary");
        let field = |n: &str| summary.fields.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        assert_eq!(field("unreachable"), Some(Value::Int(1)));
        assert_eq!(field("not_usable"), Some(Value::Int(1)));
    }

    /// The first server being down does not stop the second answering, and the
    /// second opinion is asked of the server after the one that answered.
    #[test]
    fn the_first_server_that_answers_is_the_one_used() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![
            Err(QueryFailure::Unreachable {
                detail: "no route".to_string(),
            }),
            Ok(sample(5_000)),
        ]);
        let mut j = journal();
        Timed::new(&options(&[server(1), server(2), server(3)])).poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS + 5_000]);
        assert_eq!(s.asked, vec![server(1), server(2), server(3)]);
    }

    /// A list of four is four servers, not one server and three standbys — the
    /// rotation moves the starting point every poll.
    #[test]
    fn the_starting_server_rotates_so_one_server_does_not_carry_the_list() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(0)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1), server(2), server(3)]));
        for _ in 0..4 {
            t.poll_once(&mut k, &mut s, &mut j);
        }
        assert_eq!(
            s.asked,
            vec![server(1), server(2), server(3), server(1)],
            "the rotation did not advance"
        );
    }

    /// A refused `clock_settime` is loud, and — the part that is easy to get
    /// wrong — it does **not** arm the rate limit, so a machine that gains
    /// `CAP_SYS_TIME` later is not locked out for an hour by an attempt that
    /// never moved anything.
    #[test]
    fn a_refused_clock_set_is_loud_and_does_not_arm_the_rate_limit() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        k.set_wall_results = Some(Err(Errno::EPERM));
        let mut s = FakeSntp::always(Ok(sample(5_000)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1), server(2)]));

        t.poll_once(&mut k, &mut s, &mut j);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("DEGRADED")),
            "{:?}",
            k.records
        );
        assert!(k.records.iter().any(|r| r.msg.contains("CAP_SYS_TIME")));
        assert_eq!(k.wall_ms, Some(A_PLAUSIBLE_MS), "the clock moved anyway");

        // The very next poll tries again rather than reporting a rate limit.
        k.set_wall_results = None;
        k.mono_ms += 1_000;
        t.poll_once(&mut k, &mut s, &mut j);
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS + 5_000]);
    }

    #[test]
    fn a_machine_with_no_readable_wall_clock_says_so_and_does_not_set_one() {
        let mut k = FakeKernel::new();
        k.wall_ms = None;
        let mut s = FakeSntp::always(Ok(sample(5_000)));
        let mut j = journal();
        Timed::new(&options(&[server(1)])).poll_once(&mut k, &mut s, &mut j);
        assert!(k.wall_sets.is_empty());
        assert!(said(&k, "no readable wall clock"), "{:?}", k.records);
    }

    // -- the intervals -------------------------------------------------------

    /// Fast while it is wrong, slow once it is right — `ADR-017` §3.2's
    /// convergence obligation, as a number.
    #[test]
    fn the_poll_interval_is_short_until_a_server_answers_and_long_afterwards() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1)]));
        assert_eq!(t.next_interval_ms(), RETRY_INTERVAL_MS);

        let mut down = FakeSntp::always(Err(QueryFailure::Unreachable {
            detail: "no route".to_string(),
        }));
        t.poll_once(&mut k, &mut down, &mut j);
        assert_eq!(
            t.next_interval_ms(),
            RETRY_INTERVAL_MS,
            "a failed poll must not relax the retry"
        );

        let mut up = FakeSntp::always(Ok(sample(0)));
        t.poll_once(&mut k, &mut up, &mut j);
        assert_eq!(t.next_interval_ms(), POLL_INTERVAL_MS);
    }

    /// The rate limit only means something while it is longer than the poll
    /// interval. A change to either constant that made the guard inert has to
    /// fail here.
    #[test]
    fn the_rate_limit_is_longer_than_the_poll_interval_or_it_never_binds() {
        // `const` blocks, so a change that made the guard inert fails to
        // **compile** rather than failing a test somebody might be tempted to
        // adjust. `cmd/rivet-sysd/src/kernel.rs` pins its `signalfd`
        // stride the same way, for the same reason.
        const {
            assert!(
                crate::discipline::MIN_STEP_INTERVAL_MS > POLL_INTERVAL_MS,
                "a rate limit no longer than the poll interval can never refuse anything"
            );
        }
        const {
            assert!(
                RETRY_INTERVAL_MS < POLL_INTERVAL_MS,
                "the unsynchronised retry must be the faster of the two, or a board whose network \
                 is late sits at 1970 for longer than a healthy one waits between polls"
            );
        }
    }

    /// `run` without `--once` sleeps for the interval it reports rather than
    /// spinning — which on a board is the difference between a poll every
    /// seventeen minutes and a denial-of-service against a public time server.
    #[test]
    fn the_loop_sleeps_for_the_interval_between_polls() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(0)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1)]));
        t.run(&mut k, &mut s, &mut j, Some(4));
        assert_eq!(s.asked.len(), 4);
        assert_eq!(
            k.sleeps,
            vec![POLL_INTERVAL_MS; 3],
            "the loop must sleep between polls and not after the last one"
        );
    }

    /// A board whose network is not up sleeps the *retry* interval, so the same
    /// loop converges quickly and then backs off.
    #[test]
    fn the_loop_uses_the_retry_interval_until_something_answers() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::scripted(vec![
            Err(QueryFailure::Unreachable {
                detail: "no route".to_string(),
            }),
            Err(QueryFailure::Unreachable {
                detail: "no route".to_string(),
            }),
            Ok(sample(0)),
            Ok(sample(0)),
        ]);
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1)]));
        t.run(&mut k, &mut s, &mut j, Some(4));
        assert_eq!(
            k.sleeps,
            vec![RETRY_INTERVAL_MS, RETRY_INTERVAL_MS, POLL_INTERVAL_MS]
        );
    }

    /// `--once` takes exactly one sample and returns, which is what makes a
    /// hand-run on a board possible (`AGENTS.md` §6).
    #[test]
    fn once_polls_exactly_once_and_never_sleeps() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(0)));
        let mut j = journal();
        let outcome = Timed::new(&options(&[server(1)])).run(&mut k, &mut s, &mut j, Some(1));
        assert!(matches!(outcome, PollOutcome::Answered(_)));
        assert_eq!(s.asked.len(), 1);
        assert!(k.sleeps.is_empty());
    }

    // -- what this service does NOT do ---------------------------------------

    /// **It never touches `/state`.** The stored clock has one writer
    /// (`ADR-011`), and it is the supervisor. The `Kernel` port this crate
    /// exposes has no way to write a file at all, which makes the rule
    /// structural rather than a promise — this test states the rule so that
    /// adding one is a deliberate act.
    #[test]
    fn nothing_here_can_write_a_file_at_all() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(5_000)));
        let mut j = journal();
        Timed::new(&options(&[server(1)])).poll_once(&mut k, &mut s, &mut j);
        assert!(
            k.reads.is_empty(),
            "a poll read the filesystem: {:?}",
            k.reads
        );
    }

    /// ...and it says out loud, once, that the card will go on recording
    /// `unsynchronised` — because the supervisor asserts the opposite at boot
    /// and a reader meeting only that would conclude nothing ever corrected the
    /// clock.
    #[test]
    fn the_first_good_poll_says_the_card_will_still_record_unsynchronised() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut s = FakeSntp::always(Ok(sample(0)));
        let mut j = journal();
        let mut t = Timed::new(&options(&[server(1)]));
        t.poll_once(&mut k, &mut s, &mut j);
        t.poll_once(&mut k, &mut s, &mut j);
        let saying: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg.contains("will still be written as `unsynchronised`"))
            .collect();
        assert_eq!(saying.len(), 1, "{:?}", k.records);
    }
}
