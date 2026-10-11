//! **How long each layer of the shutdown waits, and why the numbers are here
//! rather than in the two programs that use them.**
//!
//! Stopping a Rivet is three nested waits:
//!
//! ```text
//!   PID 1  ──SIGTERM──►  rivet-sysd  ──SIGTERM──►  a service
//!     │                      │                         │
//!     │                      │  STOP_GRACE_MS ─────────►│ then SIGKILL
//!     │                      │  KILL_GRACE_MS ─────────►│ then abandon it
//!     │  SUPERVISOR_STOP_GRACE_MS ──►│ then SIGKILL
//! ```
//!
//! **The outer wait must exceed the inner one, and it did not.** `rivet-sysd`
//! gave a service 10 s and then 2 s; `rivet-initd` gave the whole supervisor
//! 5 s. So on any service that used more than five seconds of its own budget,
//! PID 1 killed the supervisor before it had finished — which made `SIGKILL` of
//! `rivet-sysd` the NORMAL path rather than the exception. Every slow shutdown
//! read as a crash in the record, and a service that needed its stop path to
//! flush or unmount did not reliably get it.
//!
//! # Why one module rather than two constants
//!
//! Ruled by the maintainer 2026-09-04 (`.rulings/` R-141), and the ruling has
//! two halves. The value moves — but the *reason it was wrong* is that *two
//! numbers which must be ordered were chosen independently*, in two crates,
//! neither naming the other. Raising one without tying them together fixes
//! today's ordering and leaves the next person free to break it by tuning
//! either.
//!
//! So [`SUPERVISOR_STOP_GRACE_MS`] is **computed** from the service budget. It
//! cannot drift below what it must exceed, and a change to the service numbers
//! carries the supervisor's with it. `ADR-011`: one fact, one place.
//!
//! Both `cmd/rivet-initd` and `cmd/rivet-sysd` depend on `rivet-core`, and
//! neither depends on the other — which is why this is the only place the
//! relationship *can* be stated.

/// How long a service gets after `SIGTERM` before `SIGKILL`.
///
/// `cmd/rivet-sysd`'s `Limits::stop_grace_ms` default.
pub const SERVICE_STOP_GRACE_MS: u64 = 10_000;

/// How long after `SIGKILL` before the supervisor stops waiting for a service
/// and abandons it.
///
/// Safe to abandon because of `ADR-017` §3.1.1: the child reparents to PID 1,
/// which reaps it. `cmd/rivet-sysd`'s `Limits::kill_grace_ms` default.
pub const SERVICE_KILL_GRACE_MS: u64 = 2_000;

/// The headroom the supervisor gets on top of its own worst case.
///
/// It is not zero because the supervisor's budget is not only the waiting: it
/// has its own bookkeeping to finish after the last service is gone — the
/// journal to flush, the exit edge to release. Three seconds is a chosen value
/// rather than a measured one, and saying so is the point: if a shutdown ever
/// turns out to need more, this is the number to raise and the ordering still
/// holds by construction.
pub const SUPERVISOR_MARGIN_MS: u64 = 3_000;

/// How long the supervisor gets to exit after `SIGTERM` before PID 1 kills it.
///
/// **Derived, never chosen.** It is the whole of the worst case a service can
/// impose, plus the supervisor's own margin — so it is impossible for this to
/// be shorter than the wait it contains. That was the defect.
///
/// 15 s as the constants stand.
pub const SUPERVISOR_STOP_GRACE_MS: u64 =
    SERVICE_STOP_GRACE_MS + SERVICE_KILL_GRACE_MS + SUPERVISOR_MARGIN_MS;

// ---------------------------------------------------------------------------
// The ordering, checked by the COMPILER
// ---------------------------------------------------------------------------
//
// **A test would have been the weaker gate.** `AGENTS.md` §3 ranks the three:
// convention erodes, review is a rule somebody has to apply, and a build that
// refuses is the only one that cannot be skipped. Every value here is a `const`,
// so the relationship between them can be settled at compile time — and a
// runtime test asserting over constants is the same check reached an hour later,
// which clippy is right to call `assertions_on_constants`.
//
// A tree that breaks the ordering does not build. That is the property the
// audit of 2026-09-03 wanted and `.rulings/` R-141 asked for.

/// PID 1 must not kill the supervisor while the supervisor is still inside a
/// wait it is entitled to.
///
/// This is the 2026-09-03 defect as a property. When it failed, `SIGKILL` of
/// `rivet-sysd` was the ordinary path for any slow service.
const _PID_ONE_WAITS_LONGEST: () = assert!(
    SUPERVISOR_STOP_GRACE_MS > SERVICE_STOP_GRACE_MS + SERVICE_KILL_GRACE_MS,
    "PID 1's grace must exceed the whole of what one service can take, or the supervisor is \
     killed mid-shutdown and every slow stop reads as a crash"
);

/// And the margin is real rather than nominal, so the two are not equal by a
/// rounding accident.
const _THE_MARGIN_IS_REAL: () = assert!(
    SUPERVISOR_MARGIN_MS > 0,
    "the supervisor needs room of its own after the last service is gone"
);
