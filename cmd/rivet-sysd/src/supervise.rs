//! Convergence: `ADR-017` §3.
//!
//! Rivet does **not** order services by a declared dependency graph. Every
//! service starts together, a service that needs something not yet ready
//! retries, and readiness is declared rather than assumed. The consequence for
//! this module is that there is no start *sequence* anywhere in it — there is a
//! desired state, an observed state, and a pass that moves one toward the
//! other, run again whenever anything changes or a deadline expires.
//!
//! That is deliberately the same shape as `apply` (`ADR-015` §7), which is the
//! ADR's own argument for it: the system converges rather than being sequenced,
//! which is one idea rather than two.
//!
//! ## The decision is separated from the doing
//!
//! [`decide`] is a pure function of (phase, definition, state, limits, clock).
//! It performs no I/O, touches no [`Kernel`], and returns an [`Action`]. Every
//! interesting branch of this module — a backoff expiring, a give-up threshold
//! being reached, a child that will not die inside its grace period, a service
//! that never declares readiness — is reachable by calling it with three
//! arguments, which is why they are all tested and none of them needs a
//! process, a signal or a clock that really ticks.
//!
//! ## One service set, and since 2026-08-24 it may grow
//!
//! `ADR-017` §3. The set a [`Supervisor`] converges toward was fixed at the
//! moment it was built until 2026-08-24, when [`Supervisor::adopt`] was added so
//! that the maintainer's 2026-08-24 install-request ruling's install channel could exist at all. It is the
//! **only** way the set changes after `new`, it has exactly one caller
//! ([`crate::deploy_request`]), and it is deliberately not a general
//! service-management surface — see its own documentation, and §3.2.2 for what
//! was rejected.
//!
//! ## What is deliberately absent
//!
//! * **No dependency declarations, and no ordering.** §3.2.
//! * **No write to `enabled` from an operation.** `ADR-017` §6: `enabled` is
//!   configuration and the document's alone. [`Supervisor::adopt`] does not
//!   touch it, and neither do the three operations that cross the control
//!   surface — [`Supervisor::restart_service`] and [`Supervisor::hold_service`]
//!   set [`Managed::asked`] instead, which lives in memory and is gone at the
//!   next boot.

use crate::journal::{Journal, Record, Severity, Value};
use crate::kernel::{
    Errno, ExitStatus, Hierarchy, Kernel, Observation, Sig, Stream, Terminating,
};
use crate::applications::{render_bound, render_sockets};
use crate::verify;
use crate::service::{DefError, Limits, Origin, Readiness, ServiceDef};

/// The actor recorded on the supervisor's own records (`ADR-022` §5,
/// `ADR-012` §4). Nothing here is triggered by a human or an agent yet; when an
/// operation arrives through an adapter it will carry its own.
///
/// **`rivet_core`'s constant, for [`SELF_NAME`]'s reason**: `rivet_core`'s
/// install report tells a record this process produced from one an ingest
/// sender forged by checking this exact value, and a rename here that did not
/// reach that check would make the forgery invisible again.
pub const ACTOR: &str = rivet_core::journal::SUPERVISOR_ACTOR;

/// The service name used for the supervisor's own records.
///
/// **`rivet_core`'s constant since 2026-09-04**, and not a second spelling. The
/// ingest path has to exclude this one name — `rivet_core::journal` argues why
/// at [`rivet_core::journal::SUPERVISOR_SVC`] — and an exclusion held in a
/// different crate from the name it excludes is one rename away from being
/// silently wrong, with nothing failing until somebody reads a forged column.
pub const SELF_NAME: &str = rivet_core::journal::SUPERVISOR_SVC;

/// What the supervisor as a whole is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The normal state: converge, for ever. Reported as `running` (`ADR-032`
    /// §8), because the loop never settles and an operator reading
    /// `converging` takes it to mean the machine is not there yet.
    Converging,
    /// `SIGTERM` arrived. Bring the children down, then finish.
    ShuttingDown,
    /// Everything is down. `run` may return — and only here.
    Finished,
}

/// **Which rest state a [`State::Draining`] settles into once the cgroup
/// confirms nothing is left** — S-1's remainder, `ADR-017` §3.
///
/// A drain begun after an ordinary stop, or after killing a leftover
/// [`Supervisor::start`] found at the top of this supervisor's own lifetime,
/// settles at [`State::Idle`] — the shape [`Supervisor::begin_drain`] had
/// before this type existed. One begun after this service already reached
/// [`State::Done`] or [`State::Failed`] must settle back to that same
/// outcome: `Draining` is a wait for a fact about the CGROUP, not a verdict
/// on the service, and losing which verdict was already reached would report
/// a oneshot that finished, or a service that gave up, as merely `idle` —
/// silently inviting the very next convergence pass to start it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainRest {
    Idle,
    Done,
    Failed,
}

impl DrainRest {
    /// The state a drain with this rest settles into once the cgroup is
    /// confirmed empty.
    const fn state(self) -> State {
        match self {
            Self::Idle => State::Idle,
            Self::Done => State::Done,
            Self::Failed => State::Failed,
        }
    }
}

/// Where one service actually is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Not running, and nothing pending. A service that is not `enabled` rests
    /// here, and so does one that exited under `never`.
    Idle,
    /// Running, but **not** as a child of this process.
    ///
    /// `ADR-017` §3: when the supervisor dies its children reparent to PID
    /// 1 and keep serving, and the restarted supervisor observes them rather
    /// than remembering them. It cannot `waitpid` for them — they are not its
    /// children — so it re-asks the cgroup instead (§3.1.3).
    Adopted { pids: Vec<i32>, next_poll_ms: u64 },
    /// Running as a child of this process.
    Started {
        pid: i32,
        since_ms: u64,
        /// `ADR-017` §3 — distinct from having been started.
        ready: bool,
        /// Whether the readiness deadline has already been reported, so it is
        /// said once rather than on every convergence pass.
        ready_reported: bool,
    },
    /// Exited under a policy that does not want it back.
    ///
    /// **Distinct from [`State::Idle`], and the distinction is load-bearing.**
    /// Convergence has no memory: an `Idle` service that is `enabled` gets
    /// started, which is the correct answer for one that has never run and the
    /// wrong one for a `oneshot` that already finished. Without this state a
    /// `never` policy would restart on the very next pass — which is the
    /// opposite of what it says.
    Done,
    /// Exited, and waiting out `ADR-017` §3's backoff before trying again.
    Backoff { until_ms: u64 },
    /// `ADR-017` §3's give-up threshold was reached. The supervisor stops
    /// trying, and the machine keeps working.
    Failed,
    /// `SIGTERM` sent, waiting for the exit.
    Stopping {
        pid: i32,
        deadline_ms: u64,
        /// Whether the escalation to `SIGKILL` has already happened.
        killed: bool,
    },
    /// **Waiting for this service's CGROUP — not merely the pid this
    /// supervisor knew about — to empty, after a stop edge OR after a
    /// `Done`/`Failed` exit** — S-1's fix and its remainder, `ADR-017` §3.
    ///
    /// The cgroup is `ADR-017` §3's own authority for what is running.
    /// `Stopping`'s `SIGTERM` (or a natural exit, or `stop_adopted`'s own
    /// signal) accounts for exactly one pid; a helper that pid forked, and
    /// that ignores `SIGTERM`, is invisible to all of them. This is what
    /// catches it: every path that used to settle straight to a rest state
    /// after a stop, a completed `oneshot`, or a give-up now asks the cgroup
    /// first, through [`Supervisor::begin_drain`], and only a cgroup that
    /// still answers with members reaches this state at all — the ordinary
    /// case costs one [`Kernel::observe`] and no delay.
    Draining {
        /// When to escalate to a harder kill if the cgroup has not emptied —
        /// `Limits::stop_grace_ms`, the same grace a `Stopping` service's own
        /// `SIGTERM` gets.
        deadline_ms: u64,
        /// Whether the harder kill ([`Kernel::cgroup_kill`], or a manual
        /// `SIGKILL` sweep of whatever [`Kernel::observe`] still lists) has
        /// already run.
        killed: bool,
        /// When to re-ask the cgroup whether it has emptied —
        /// `Limits::adopt_poll_ms`, the same cadence [`State::Adopted`]
        /// already polls at: both questions are "ask the cgroup again."
        next_poll_ms: u64,
        /// Which state to settle into once the cgroup is confirmed empty —
        /// [`DrainRest`].
        then: DrainRest,
    },
    /// **Enabled, but the per-machine material its manifest declares is not
    /// on this machine yet** — `ADR-055` §5.
    ///
    /// Reached from [`Supervisor::start`], before a spawn is ever attempted,
    /// by [`crate::material::missing`] naming at least one file. Distinct
    /// from [`State::Failed`] and from `ADR-017` §3's "not a new state"
    /// precedent for a program the image cannot run — see that section's own
    /// account of both: that case is permanent on a read-only `/system` and
    /// retrying changes nothing, where this one is about writable
    /// `/state/app/<name>/data` — an operator running `cargo rivet secrets`
    /// or `rivet apply install-secret` makes it true on the very next check,
    /// with no reload and no restart. A service held here is never spawned,
    /// so it never counts toward `ADR-017` §3's give-up threshold either.
    WaitingForMaterial {
        /// Whether the record naming what is missing has already been
        /// journalled, so a service held here for days says it once — the
        /// same shape as [`Managed::cgroup_warned`] and `Started`'s own
        /// `ready_reported`.
        announced: bool,
        /// When this service is looked at again — `Limits::material_poll_ms`.
        /// Nothing on this machine wakes the supervisor when `apply
        /// install-secret` writes a file (`rivet-core`'s `install_secret`
        /// module says so in its own answer), so this is a poll, not a push,
        /// on the same footing as `Adopted`'s own `next_poll_ms`.
        next_check_ms: u64,
    },
}

/// One service and everything the supervisor knows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Managed {
    pub def: ServiceDef,
    pub state: State,
    /// **An uninstall is waiting on this service to stop.** `Some(purge)` once
    /// `rivet_core::uninstall`'s request has been taken up; the `bool` is
    /// whether the application's data goes too.
    ///
    /// # Why a flag rather than a state
    ///
    /// [`State`] is what the *service* is doing and it is already the thing
    /// `decide` matches on. An uninstall is not a state a service is in — it is
    /// a thing waiting to happen *after* whatever state it is in becomes a
    /// stopped one — and adding `State::Uninstalling` would have meant a new arm
    /// in every match and a new way for the stop machinery to be wrong.
    ///
    /// So the uninstall sets `def.enabled = false` and lets `ADR-017` §3's
    /// existing stop run: `Started` → `Stopping` → the deadline → `SIGKILL` if
    /// it will not go. This flag is only the note that says what to do when it
    /// arrives.
    pub pending_uninstall: Option<bool>,
    /// Consecutive failures, forgiven once the service has been up for
    /// [`Limits::success_window_ms`].
    pub attempts: u32,
    /// **Why this service is not running**, in the supervisor's own words.
    ///
    /// `PLAN.md` §104.22. `get status` used to answer `state: idle` and nothing
    /// else, so an application refused for a device collision, one whose signed
    /// header did not verify, and one that is simply not enabled all read
    /// identically — and the difference lived only in `get log … all`, behind a
    /// 200-record default that hides the start of a boot.
    ///
    /// **Set where the refusal is decided and cleared when it stops applying.**
    /// A reason that outlived its cause would be worse than none: an operator
    /// reading `started` beside *"REFUSED another service already declares this
    /// device class"* would trust the sentence over the state.
    pub reason: Option<String>,
    /// **Whether this service is [`State::Failed`] because its `connects`
    /// could not be resolved** — the named provider is not enabled, an endpoint the
    /// provider does not offer, or a contested one (`ADR-015` §5).
    ///
    /// That refusal is a statement about the set of ENABLED definitions, not
    /// about this service, so it stops being true when that set changes — a
    /// reload that switches the provider on, or an install that adds it.
    /// [`Supervisor::converge`] looks again at every service carrying this
    /// mark and starts the ones the current set now satisfies. Every other
    /// way of reaching `Failed` (a give-up after crashes, a refused signature,
    /// no Landlock) stays terminal, which is why this is a mark of its own
    /// rather than a reading of `Failed`.
    pub refused_connects: bool,
    /// `ADR-022` §5's correlation identifier, re-minted for each attempt so
    /// that one service's start, readiness, exit and backoff records are one
    /// chain and the *next* attempt is a different one.
    pub corr: String,
    /// Whether the "this service cannot be observed" warning has been said.
    /// Once is information; every convergence pass is noise that hides the rest.
    ///
    /// **Set only on the dangerous branch** (`ADR-017` §3). A first boot
    /// whose cgroup is merely absent must not consume the latch, or a hierarchy
    /// that goes away later would be the one failure this never reports.
    pub cgroup_warned: bool,
    /// **Whether this service may still legitimately become [`State::Adopted`]
    /// if its cgroup answers non-empty at the next [`Supervisor::start`]** —
    /// S-1's fix, `ADR-017` §3.1.1.
    ///
    /// Adoption exists for a SUPERVISOR restart: a child that outlived the
    /// old process is legitimately still the service, and the cgroup is the
    /// only record of that. `true` only until this service's very first
    /// `start()` in this supervisor's own lifetime — set from
    /// [`Supervisor::new`] or [`Supervisor::adopt`], whichever began this
    /// service's life here. After that first `start()`, a non-empty cgroup
    /// means something THIS supervisor's own earlier stop failed to clean
    /// up, not a prior process it never knew about — leftovers to kill, not
    /// a service to adopt.
    pub adoption_eligible: bool,
    /// **What an operator has asked of this service's running state** — a
    /// restart, a stop that is held, or nothing. `ADR-017` §6.
    ///
    /// A restart is taken up once and cleared by the pass that sees it. **A
    /// stop is not cleared by any pass**: it stays until
    /// [`Supervisor::hold_service`] releases it, the service is uninstalled, or
    /// this process ends. Nothing writes it to disk, so a reboot or a restart of
    /// the supervisor returns the service to what the document says.
    ///
    /// # Why a field and not a state, which is [`Self::pending_uninstall`]'s
    /// question with a different answer
    ///
    /// An uninstall is a *flag* because it is a thing waiting to happen after
    /// whatever state the service is in becomes a stopped one. A restart is not
    /// waiting for anything — it is an **input to the decision**, which is why
    /// it is a parameter of [`decide`] rather than something [`Supervisor`]
    /// acts on itself. Keeping it out of `decide` would have meant a second
    /// place that decides what happens to a service, and `decide`'s own
    /// documentation is that it is *the whole* of that.
    ///
    /// # And it is not `enabled = false`, which was the cheaper shape
    ///
    /// `begin_uninstall` sets `enabled = false` and lets the ordinary stop run.
    /// A restart may **never** do that: `enabled` is what `get status` reports,
    /// so a service that read `enabled: false` for even one pass would be the
    /// machine holding a second notion of what it should be running — which is
    /// exactly the property `ADR-017` §6 exists to make unconstructable.
    ///
    /// A restart is cleared by the convergence pass that sees it, so a request
    /// is taken up exactly once.
    pub asked: Asked,

    /// **What this service last answered, and what it said with it** —
    /// `ADR-045` §3, 2026-09-08.
    ///
    /// `None` means **no successful answer is on record**, which `get status`
    /// reports as `unknown` (§4.2). It is `None` rather than
    /// `Some(State::Unknown)` for the reason `Reported` has three variants and
    /// `State` four: `unknown` is what an observer concludes, so storing it as
    /// something the service *said* would be storing an observation as a
    /// report.
    ///
    /// **Cleared on every start**, so a restarted service does not inherit the
    /// opinion the previous process had of itself.
    pub health: Option<(rivet_core::health::Reported, Option<String>)>,

    /// **When that answer arrived** — monotonic milliseconds, `ADR-057` §3.
    ///
    /// `Some` exactly when [`Self::health`] is, so a reader cannot be given a
    /// word without being able to say how old it is. The supervisor asks every
    /// thirty seconds (`ADR-045` §3), which makes a health word the one fact a
    /// dashboard is most likely to draw as live when it is half a minute old —
    /// so the facts call carries this as an **age**, computed at the moment it
    /// answers.
    ///
    /// **Not derivable from [`Self::next_health_ms`]**, which is why it is
    /// stored. A `HEALTH=` hint moves that field to *now* (§3.1.3), so
    /// subtracting it from the interval would report an answer as fresh
    /// because a *question* was recent.
    pub health_at_ms: Option<u64>,

    /// **When the supervisor next asks** — monotonic milliseconds.
    ///
    /// The cadence is the supervisor's, which is the whole of `ADR-045`
    /// §3.1.1: an application that chose when to speak would have set the
    /// fleet's monitoring resolution, and one that reported at startup and then
    /// went quiet would have declared itself permanently healthy.
    ///
    /// **`HEALTH=` on `NOTIFY_SOCKET` moves this to now and nothing else**
    /// (§3.1.3) — an *ask me now*, so a data-integrity failure at 03:00 is seen
    /// before the next interval rather than up to thirty seconds later.
    pub next_health_ms: u64,

    /// **Whether the last question went unanswered**, and why, in the
    /// supervisor's own words.
    ///
    /// Kept beside [`Self::health`] rather than folded into it, because they
    /// are different facts: a service can have answered `degraded` a minute ago
    /// and be silent now, and a reader deciding what to do next needs both. The
    /// state `get status` reports for that service is `unknown` — the newer
    /// fact wins — and this sentence is what says how it got there.
    pub health_silent: Option<String>,
}

impl Managed {
    /// **Clear a restart once a pass has seen it, and leave a hold alone.**
    ///
    /// The one place a convergence pass touches [`Self::asked`]. A restart is a
    /// request taken up exactly once; a hold is a standing instruction, and a
    /// pass that cleared it would have the service running again one pass
    /// after it was stopped — a stop the supervisor immediately undoes.
    pub fn take_up_restart(&mut self) {
        if self.asked == Asked::Restart {
            self.asked = Asked::Nothing;
        }
    }
}

/// What an operator has asked of one service. [`Managed::asked`], `ADR-017` §6.
///
/// A named type rather than a `bool` because it is a parameter of [`decide`],
/// and `decide(phase, def, state, true, limits, now)` at twenty call sites
/// would be twenty places a reader has to look up what the `true` meant.
///
/// **One field for both, because they exclude each other.** A restart of a
/// held service is refused ([`Supervisor::restart_service`]), and taking a hold
/// replaces a restart that has not been taken up yet — the operator's later
/// word wins, and the hold is the stronger of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// The ordinary case: converge by the document alone.
    Nothing,
    /// **Discard this service's current state and converge again.** It never
    /// changes what the service *should* be doing — the guard in
    /// [`Supervisor::restart_service`] has already established that the
    /// document enables it.
    Restart,
    /// **Stop it, and do not start it again while this holds.** Taken and
    /// released by [`Supervisor::hold_service`] (`.rulings/` R-183). A crash
    /// restart, a backoff expiring and a reload all leave it stopped; the hold
    /// is released by `service-start`, by an uninstall removing the service, or
    /// by this process ending.
    Stop,
}

/// What [`decide`] concluded. Every variant is an intent, not a syscall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Bring it up: observe first (`ADR-017` §3), then spawn if it is not
    /// already running.
    Start,
    /// `SIGTERM` a child of ours.
    Stop,
    /// The grace period expired.
    Kill,
    /// `SIGKILL` did not work inside its grace period either. Stop waiting.
    ///
    /// Safe precisely because of `ADR-017` §3: an abandoned process
    /// reparents to PID 1, which reaps it. Blocking shutdown for ever would be
    /// worse than letting go.
    Abandon,
    /// `SIGTERM` processes that are running but are not our children.
    StopAdopted,
    /// Re-ask the cgroup whether an adopted service is still running.
    PollAdoption,
    /// Re-ask the cgroup whether a [`State::Draining`] service has emptied
    /// yet — S-1's fix.
    PollDrain,
    /// A [`State::Draining`] service's grace period expired without the
    /// cgroup emptying: kill it harder ([`Kernel::cgroup_kill`], or a manual
    /// `SIGKILL` of whatever is still there).
    EscalateDrain,
    /// A [`State::Draining`] service's cgroup would not empty even after the
    /// harder kill. Stop waiting and say so — `ADR-017` §3's [`Action::Abandon`],
    /// for a cgroup rather than a single pid.
    AbandonDrain,
    /// A `notify` service has not declared readiness inside its deadline. Say
    /// so; do not kill it.
    ReportNotReady,
    /// Nothing to converge toward: drop a pending backoff or a failure and
    /// come to rest.
    Cancel,
}

impl Action {
    /// Whether this action **gives something back** — a socket, a device node,
    /// the screen — rather than taking something.
    ///
    /// # Why the convergence pass needs to know
    ///
    /// `PLAN.md` §123, Wave 2.7. One pass used to decide and perform each
    /// service in turn, in the order they are stored, which put the release of
    /// one service's resources *after* the acquisition of another's whenever
    /// the names sorted that way. Two applications that both declare
    /// `devices = ["display"]`, with the incoming one sorting first: `Start(B)`
    /// ran, then `Stop(A)` ran, and `crate::devices::revoke` chowned
    /// `/dev/input/*` and `/dev/fb0` back to root **after B had already been
    /// spawned and given them**. B then has a screen it cannot open, and
    /// nothing anywhere says why.
    ///
    /// So a pass does every release first and every acquisition second, and
    /// this is the line that sorts them.
    #[must_use]
    pub const fn releases(&self) -> bool {
        matches!(
            self,
            Self::Stop
                | Self::StopAdopted
                | Self::Kill
                | Self::Abandon
                | Self::Cancel
                | Self::EscalateDrain
                | Self::AbandonDrain
        )
    }
}

/// The whole of the supervisor's decision-making, as a pure function.
///
/// No `Kernel`, no clock of its own, no I/O. Give it a phase, a definition, a
/// state, ~~and a time~~ **whether a restart was asked for, and a time**, and it
/// says what should happen.
///
/// # The fourth input, added 2026-08-29 — `ADR-017` §6
///
/// **A restart cannot be expressed as a function of the other three**, which is
/// why it is a parameter here rather than something the caller does around this
/// function. A `Started` service that the document enables is exactly the case
/// where this returns `Nothing`, and a restart is a request to stop it anyway —
/// so any shape that left `decide` alone would have needed a *second* place
/// that decides what happens to a service, and the whole value of this function
/// is that there is one.
///
/// **What it may never do is contradict `def.enabled`.** Every arm below either
/// stops a service that the document says should be running — which the next
/// pass then starts again — or starts one it says should be running. Nothing
/// here can start a disabled service, and the test
/// `a_restart_can_never_produce_a_running_service_the_document_disables`
/// walks every state to say so.
pub fn decide(
    phase: Phase,
    def: &ServiceDef,
    state: &State,
    asked: Asked,
    limits: &Limits,
    now_ms: u64,
) -> Action {
    // Shutdown overrides `enabled`: on the way down, everything comes down.
    if phase != Phase::Converging {
        return match state {
            State::Started { .. } => Action::Stop,
            State::Adopted { .. } => Action::StopAdopted,
            State::Stopping {
                deadline_ms,
                killed,
                ..
            } => stopping_action(*deadline_ms, *killed, now_ms),
            State::Draining {
                deadline_ms,
                killed,
                next_poll_ms,
                ..
            } => draining_action(*deadline_ms, *killed, *next_poll_ms, now_ms),
            State::Backoff { .. } | State::Failed | State::Done | State::WaitingForMaterial { .. } => {
                Action::Cancel
            }
            State::Idle => Action::Nothing,
        };
    }

    // **`ADR-017` §6's converge-only restart**, and it is above the
    // ordinary arms rather than folded into them because it answers a different
    // question: not *where should this service be* — the answer to that is
    // unchanged and is `def.enabled` — but *may it keep the state it is in on
    // the way there*.
    //
    // **`def.enabled` is deliberately not re-checked here.**
    // `Supervisor::restart_service` refuses a disabled service before this flag
    // can be set, and a second check would be a second answer to the one
    // question `ADR-011` says has one. What makes that safe is the shape: every
    // arm below is either a stop that the very next pass undoes because the
    // document enables it, or a start of something the document enables. A
    // disabled service reaching here would be started by `Start` — which is why
    // the guard is a guard and is tested as one.
    // **A held stop — `.rulings/` R-183.** Above the ordinary arms for the
    // restart's reason: `def.enabled` still says where the service should be,
    // and the hold says it may not go there while it lasts. Every arm is a
    // stop or a rest, so a held service is never started from any state — and
    // a service that exits while held rests at `Idle` rather than waiting out a
    // backoff that would start it.
    if asked == Asked::Stop {
        return match state {
            State::Started { .. } => Action::Stop,
            State::Adopted { .. } => Action::StopAdopted,
            State::Stopping {
                deadline_ms,
                killed,
                ..
            } => stopping_action(*deadline_ms, *killed, now_ms),
            State::Draining {
                deadline_ms,
                killed,
                next_poll_ms,
                ..
            } => draining_action(*deadline_ms, *killed, *next_poll_ms, now_ms),
            State::Backoff { .. } | State::Failed | State::Done | State::WaitingForMaterial { .. } => {
                Action::Cancel
            }
            State::Idle => Action::Nothing,
        };
    }

    if asked == Asked::Restart {
        return match state {
            // Running: signal it. The reap sets `Idle`, and the next pass
            // starts it because the document says it should be running — the
            // restart is two ordinary convergences, not a new mechanism.
            State::Started { .. } => Action::Stop,
            State::Adopted { .. } => Action::StopAdopted,
            // Not running: start it **now**, rather than clearing the latch and
            // leaving it `Idle` for a later pass. `Cancel` was the obvious
            // spelling and is wrong: `next_timeout_ms` returns nothing for an
            // idle service, so a supervisor with nothing else pending would
            // sleep indefinitely with the restart taken and not performed.
            // Discarding the backoff and the give-up latch is what *discard
            // current state* means.
            State::Idle
            | State::Backoff { .. }
            | State::Failed
            | State::Done
            | State::WaitingForMaterial { .. } => Action::Start,
            // Already going down, and it comes back on its own. Falling through
            // to the deadline is right: a restart must not shorten or extend
            // `ADR-017` §3's grace period.
            State::Stopping {
                deadline_ms,
                killed,
                ..
            } => stopping_action(*deadline_ms, *killed, now_ms),
            // The same, for the cgroup-drain phase: a restart does not skip
            // waiting for it to empty either.
            State::Draining {
                deadline_ms,
                killed,
                next_poll_ms,
                ..
            } => draining_action(*deadline_ms, *killed, *next_poll_ms, now_ms),
        };
    }

    match state {
        State::Idle => {
            if def.enabled {
                Action::Start
            } else {
                Action::Nothing
            }
        }

        State::Adopted { next_poll_ms, .. } => {
            if def.enabled {
                if now_ms >= *next_poll_ms {
                    Action::PollAdoption
                } else {
                    Action::Nothing
                }
            } else {
                Action::StopAdopted
            }
        }

        State::Started {
            since_ms,
            ready,
            ready_reported,
            ..
        } => {
            if !def.enabled {
                return Action::Stop;
            }
            // Readiness is reported, never inferred. A service that is slow is
            // not a service that has failed — ADR-017 §3's model is that
            // things retry until their preconditions hold — so this says so
            // and changes nothing else.
            let overdue = now_ms.saturating_sub(*since_ms) >= limits.ready_timeout_ms;
            if def.readiness == Readiness::Notify && !*ready && !*ready_reported && overdue {
                Action::ReportNotReady
            } else {
                Action::Nothing
            }
        }

        State::Backoff { until_ms } => {
            if !def.enabled {
                Action::Cancel
            } else if now_ms >= *until_ms {
                Action::Start
            } else {
                Action::Nothing
            }
        }

        // `ADR-055` §5: a poll, not a push — see `State::WaitingForMaterial`'s
        // own documentation for why nothing else wakes this pass. `Start`
        // re-runs `Supervisor::start`, which re-checks and either spawns or
        // holds again; it is never a crash to retry, so this never touches
        // `attempts`.
        State::WaitingForMaterial { next_check_ms, .. } => {
            if !def.enabled {
                Action::Cancel
            } else if now_ms >= *next_check_ms {
                Action::Start
            } else {
                Action::Nothing
            }
        }

        // ADR-017 §3: after the threshold the supervisor stops trying. Doing
        // nothing here is the entire point — the alternative writes to the SD
        // card until the card dies.
        //
        // Disabling a failed service clears it, so `enabled = false`, apply,
        // `enabled = true`, apply is the way back without a restart operation.
        // Disabling a service that has run to completion returns it to `Idle`,
        // so `enabled = false`, apply, `enabled = true`, apply runs a oneshot
        // again — the only way there is, since `ADR-017` §6 has no manual
        // start either.
        State::Failed | State::Done => {
            if def.enabled {
                Action::Nothing
            } else {
                Action::Cancel
            }
        }

        State::Stopping {
            deadline_ms,
            killed,
            ..
        } => stopping_action(*deadline_ms, *killed, now_ms),

        State::Draining {
            deadline_ms,
            killed,
            next_poll_ms,
            ..
        } => draining_action(*deadline_ms, *killed, *next_poll_ms, now_ms),
    }
}

/// [`State::Draining`]'s footing exactly, for [`stopping_action`]'s own
/// reason: the deadline decides whether to escalate or abandon, and the poll
/// cadence decides when to ask again while still inside it. Checked in that
/// order, so a deadline that has also passed its poll time escalates or
/// abandons rather than merely asking once more.
fn draining_action(deadline_ms: u64, killed: bool, next_poll_ms: u64, now_ms: u64) -> Action {
    if now_ms >= deadline_ms {
        if killed { Action::AbandonDrain } else { Action::EscalateDrain }
    } else if now_ms >= next_poll_ms {
        Action::PollDrain
    } else {
        Action::Nothing
    }
}

/// **The supervisor's own word for what a service is doing**, and the one place
/// it is spelled.
///
/// `AGENTS.md` §5 — report what the system said. It reaches an operator through
/// `get status` and through `apply service-restart`'s answer, and a second
/// spelling in either would be a second answer to *what is this service doing*.
///
/// **Adopted is its own word and not `started`.** A service this supervisor did
/// not spawn is one it cannot `waitpid` for (`ADR-017` §3), and an operator
/// debugging a restarted supervisor needs to know which kind they are looking
/// at.
pub const fn state_word(state: &State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Adopted { .. } => "adopted",
        State::Started { .. } => "started",
        State::Done => "done",
        State::Backoff { .. } => "backoff",
        State::Failed => "failed",
        State::Stopping { .. } => "stopping",
        State::Draining { .. } => "draining",
        State::WaitingForMaterial { .. } => "waiting-for-material",
    }
}

fn stopping_action(deadline_ms: u64, killed: bool, now_ms: u64) -> Action {
    if now_ms < deadline_ms {
        Action::Nothing
    } else if killed {
        Action::Abandon
    } else {
        Action::Kill
    }
}

/// Why a run-time instance was not created. [`Supervisor::adopt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdoptError {
    /// The definition is not one this supervisor could run. Refused rather than
    /// repaired, for [`Supervisor::new`]'s reason.
    Definition(DefError),
    /// A service of that name is in the set and is not finished.
    Busy { name: String },
}

impl core::fmt::Display for AdoptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Definition(e) => write!(f, "{e}"),
            Self::Busy { name } => write!(
                f,
                "{name} is already in this supervisor's service set and has not finished"
            ),
        }
    }
}

/// The supervisor.
pub struct Supervisor {
    services: Vec<Managed>,

    /// **Which display application holds the screen**, or `None` for the
    /// launcher's menu. `PLAN.md` §104.40.
    ///
    /// Runtime state and deliberately not the document's: the document says
    /// what is OFFERED (`enabled`), and this says which of them is running.
    /// Writing a selection into the document would make every button press on a
    /// remote control a configuration change.
    display_selection: Option<String>,

    /// Every application the document offered a screen to, by name.
    ///
    /// Recorded once, from `enabled` as the document wrote it, before the gate
    /// below narrows it. This is the set a selection selects from, and the
    /// thing whose loss `crate::document::apply_auto_run` used to cause.
    display_offered: Vec<String>,
    /// **What this machine is for**, from the document — `PLAN.md` §109.3.
    ///
    /// Document-derived and held here for the same reason `ServiceDef::enabled`
    /// is: `get status` is the one answer that crosses the query socket, and a
    /// caller asking what a machine is doing is asking this at the same moment.
    ///
    /// **It starts as `unknown` and not `headless`**, so a path that never sets
    /// it says *nobody told me* instead of asserting that a kiosk is a headless
    /// box (`AGENTS.md` §3).
    kiosk: rivet_core::operation::status::KioskMode,
    /// **Which services the document names for the launcher and the
    /// compositor** — [`crate::document::DisplayAuthority`]. The image's own
    /// until [`Self::set_display_authority`] is called, which `crate::main`
    /// does once at boot, beside the kiosk socket it is read with.
    display_authority: crate::document::DisplayAuthority,
    limits: Limits,
    journal: Journal,
    phase: Phase,

    /// **How much of the journal any one uid may consume** — `ADR-022` §11,
    /// and `PLAN.md` §11.2's option (c), built 2026-09-04.
    ///
    /// It lives on the supervisor rather than inside the journal because the
    /// journal writes every source — the kernel, a captured pipe, this
    /// process's own records — and none of those is a uid that connected to a
    /// socket. The limit is about **this one door**.
    ingest_rate: rivet_core::journal::IngestRate,

    /// **This machine's name and the slot it is running from** — `ADR-057` §4,
    /// the two facts the facts call carries that are fixed for the life of the
    /// boot.
    ///
    /// Held here for `Self::kiosk`'s reason exactly: both are already known by
    /// the time the first application starts, both are read once by
    /// `crate::main` — the name from `crate::naming`, the slot from the
    /// kernel's mount table — and a supervisor that went and read either of
    /// them when asked would be a supervisor an application can make do
    /// filesystem work. `.rulings/` R-195 D5's invariant is why that matters
    /// more here than it would anywhere else.
    ///
    /// **`None` is an answer and not a hole.** A machine off a card has no
    /// slot; a machine nobody named has no hostname; and a supervisor that
    /// has not been told yet reports neither rather than guessing.
    machine_name: Option<String>,
    /// See [`Self::machine_name`].
    machine_slot: Option<String>,

    /// **When the last convergence pass ran** — monotonic milliseconds,
    /// `ADR-057` §3.
    ///
    /// The age every fact in the facts call except the three boot-fixed
    /// strings is reported against. A supervisor that is **still answering and
    /// has stopped turning** is a state this number makes drawable, and it is
    /// not the same thing as silence: the answer arrives, and says the view in
    /// it is five hours old.
    converged_at_ms: u64,

    /// **What `rivet-mdnsd` was last known to announce from** — `crate::mdns`.
    /// Handed over at boot and at every reload by
    /// [`Self::follow_mdns_inputs`], which restarts the responder when it
    /// changes.
    mdns_inputs: Option<crate::mdns::Inputs>,

    /// **The DHCP leases `rivet-netd` was last known to apply from** —
    /// `crate::leases`. Handed over at boot and at every reload by
    /// [`Self::follow_leases`], which runs `rivet-netd` again when one changes.
    leases: Option<crate::leases::Leases>,

    /// **The firewall policy last handed to the kernel** — `ADR-023` §9,
    /// `crate::runtime::reconcile_firewall`. Not the kernel's state, which is
    /// read back at every apply: a record of this process's own last request,
    /// so an install that changes no proxied port does not rebuild the table,
    /// and a policy the kernel refused is not retried on every event.
    firewall_attempted: Option<rivet_adapter_net::firewall::Policy>,
}

impl Supervisor {
    /// Build a supervisor over a set of definitions.
    ///
    /// Refuses rather than repairs: a definition that cannot be run is a fact
    /// about the image or the document, and starting the *other* services while
    /// silently dropping this one would produce a machine whose state matches
    /// nothing anyone wrote (`AGENTS.md` §3).
    /// **Which applications may be given the screen**, by name.
    ///
    /// Read once from `enabled` as the document wrote it. `PLAN.md` §104.40.
    fn offered_displays(services: &[Managed]) -> Vec<String> {
        services
            .iter()
            .filter(|m| {
                m.def.enabled
                    // **Either declaration, and both are live at once.**
                    //
                    // `devices = ["display"]` is the LEASE's way of saying it
                    // and `connects = [":wayland"]` is the SURFACE's. A card
                    // written before `ADR-058` runs applications that say the
                    // first and no compositor to say the second to; a card
                    // written after runs applications that say the second and
                    // are granted no DRM node to make the first true. Both
                    // machines have to offer a menu, so both spellings count.
                    //
                    // They are not mutually exclusive on purpose: an
                    // application mid-conversion can carry both and is offered
                    // once, because this is a filter and not a sum.
                    && (m.def
                        .devices
                        .contains(&rivet_core::package::application::Device::Display)
                        || Self::draws_through_the_compositor(&m.def))
                    // **The launcher is not one of its own choices.**
                    //
                    // It declares a display like everything it offers, so
                    // without this the first selection would disable and stop
                    // the very process that made it — and the machine would
                    // have no way back to a menu.
                    //
                    // Identified by the DECLARATION and not by a name:
                    // `connects = ["rivet-sysd:kiosk"]` is what makes an
                    // application the thing that controls the kiosk, only one
                    // may hold it (`crate::main`), and a name would be a second
                    // spelling of the same fact.
                    && !Self::drives_the_screen(&m.def)
            })
            .map(|m| m.def.name.clone())
            .collect()
    }

    /// **Read the offered set again**, after the document has been applied.
    ///
    /// `PLAN.md` §106.5. The constructor reads it once, in the one moment
    /// `enabled` still means *offered* rather than *running* — and until
    /// 2026-09-01 that was the only time it was ever read.
    ///
    /// **So a display application enabled after boot could never be selected.**
    /// The document said yes, [`Self::set_enabled`] recorded it, and
    /// [`Self::apply_display_gate`] immediately narrowed it back to *not
    /// selected* — with nothing having captured the answer in between. The
    /// launcher offered it, the supervisor refused it, and the refusal named a
    /// set computed before the operator had said anything.
    ///
    /// The caller must invoke this **between** applying the document and the
    /// gate, which is the same order the constructor uses. Doing it after the
    /// gate would read the narrowed answer and make the set collapse to the
    /// selection — which is the bug one door along.
    pub fn refresh_display_offered(&mut self) {
        self.display_offered = Self::offered_displays(&self.services);
        self.apply_display_gate();
    }

    /// **Which side of the display arbitration this service is on** —
    /// `rivet_core::service::DisplayRole`, `ADR-036` §3 as amended 2026-09-04
    /// (`.rulings/` R-138).
    ///
    /// With no compositor enabled, the arbiter is the named launcher, which
    /// declares `connects = ["rivet-sysd:kiosk"]` — the same declaration and
    /// the same name `crate::main` gives the kiosk socket by. **One spelling of
    /// one fact**: the program that is given the socket is the program that is
    /// given the card, and a machine cannot end up with one without the other.
    ///
    /// **The declaration asks and the document decides.** A role is given
    /// only to the service the machine document names for it
    /// ([`crate::document::DisplayAuthority`], the image's own launcher and
    /// compositor by default) and only when that service's manifest declares
    /// it. Any other service making the same declaration is a lessee, so an
    /// application cannot take the screen, or take it away from the program
    /// meant to hold it, by declaring a role — and since one name holds each
    /// role, a declaration can never put two programs in a position to take
    /// DRM master from each other, which is the arbitration `ADR-015` §6 point
    /// 3 exists to hold.
    fn display_role(&self, def: &ServiceDef) -> rivet_core::service::DisplayRole {
        // **The named compositor, ENABLED, takes the screen**, and a disabled
        // one changes nothing — which is what lets a card carry
        // `rivet-displayd` and go on behaving exactly as the card before it
        // until a document turns it on. `ADR-050` §2's tier 2, applied to the
        // arbitration.
        let compositor_on = self
            .services
            .iter()
            .any(|s| s.def.enabled && self.is_the_compositor(&s.def));
        let arbiter = if compositor_on {
            self.is_the_compositor(def)
        } else {
            self.is_the_launcher(def)
        };
        if arbiter {
            rivet_core::service::DisplayRole::Arbiter
        } else {
            rivet_core::service::DisplayRole::Lessee
        }
    }

    /// Whether `def` is the compositor the document names AND declares the
    /// compositor's endpoint.
    fn is_the_compositor(&self, def: &ServiceDef) -> bool {
        def.name == self.display_authority.compositor && Self::provides_the_compositor(def)
    }

    /// Whether `def` is the launcher the document names AND declares the
    /// kiosk endpoint.
    fn is_the_launcher(&self, def: &ServiceDef) -> bool {
        def.name == self.display_authority.launcher && Self::controls_the_kiosk(def)
    }

    /// Whether this definition is the one that drives the kiosk.
    fn controls_the_kiosk(def: &ServiceDef) -> bool {
        def.connects.iter().any(|e| {
            e.owner() == rivet_core::kiosk::PROVIDER && e.name() == rivet_core::kiosk::ENDPOINT
        })
    }

    /// Whether this definition **provides the compositor** — `ADR-058` §4.
    ///
    /// By the declaration and never by a name, exactly as
    /// [`Self::controls_the_kiosk`] is: a service that offers
    /// `provides = ["socket:wayland"]` is offering to hand other programs a
    /// surface, and it cannot do that without the screen.
    fn provides_the_compositor(def: &ServiceDef) -> bool {
        def.provides
            .iter()
            .any(|e| e.name() == rivet_core::compositor::ENDPOINT)
    }

    /// Whether this definition **draws through the compositor** — the other
    /// side of [`Self::provides_the_compositor`], and what makes a program a
    /// kiosk application since `ADR-058`.
    ///
    /// **`devices = ["display"]` used to be the whole answer**, and for a
    /// leased screen it was the right one: a program that was going to
    /// modeset a DRM node had to ask for the node. `ADR-058` took the node
    /// away — a client is handed a surface and opens nothing — so
    /// `aura-renderer` now declares `devices = []`, and under the old test it
    /// stopped being a screen application at all. The launcher's menu went
    /// empty on `rivet-06f5c` on 2026-09-20 and said *"no application on this
    /// machine offers a screen"* while `aura-renderer` was started and
    /// drawing.
    ///
    /// **The suite agreed with itself and could not have caught it.** Every
    /// kiosk definition in these tests is built by `kiosk_def(…,
    /// vec![Device::Display])`, so the fixtures declared the one thing the
    /// conversion removed. `docs/PLAN.md` §180.2 is the pattern.
    ///
    /// By the declaration, like every other rule here: `connects` naming the
    /// compositor's endpoint is a program saying it needs a surface, and a
    /// surface is a screen.
    fn draws_through_the_compositor(def: &ServiceDef) -> bool {
        def.connects
            .iter()
            .any(|e| e.name() == rivet_core::compositor::ENDPOINT)
    }

    /// **Which service is the one that may take the screen**, and the answer
    /// moved on 2026-09-20.
    ///
    /// `.rulings/` R-138 made it *the program that connects to the kiosk
    /// endpoint* — the launcher — because the launcher held DRM master and
    /// leased the display out. `ADR-058` §4 split that program in two and
    /// gave the screen to the half that may not fail, so the rule has to move
    /// with it: **the arbiter is the compositor.**
    ///
    /// # The kiosk clause is a MIGRATION and not a second mode
    ///
    /// **"Mode" means something else here, and it is worth not confusing.**
    /// The maintainer, 2026-09-20: `KIOSK` decides whether a machine is
    /// `headless`, `kiosk` or, one day, `desktop`, and that is how the
    /// compositor fits in (.rulings/ R-312). That is
    /// `rivet_core::operation::status::KioskMode` — **what this machine is
    /// for** — and it is a property of the machine, not of a service. What
    /// this function decides is much smaller: given that a machine has a
    /// screen, which single program is allowed to drive it.
    ///
    /// The two meet at the mode's third value, and the maintainer gave the
    /// whole mapping in one line (.rulings/ R-312): **`headless` runs no
    /// displayd, `kiosk` runs the launcher, `desktop` runs COSMIC.**
    ///
    /// Read that against this function and it is the argument for the rule.
    /// `cosmic-comp` is a compositor in its own right — built on Smithay, like
    /// `rivet-displayd`, which is why `ADR-058` §5 chose Smithay — so a
    /// desktop machine does not add a session *on top of* `rivet-displayd`, it
    /// runs **a different provider of the same endpoint.** Because the arbiter
    /// is *whoever provides `socket:wayland`* and not *the program named
    /// rivet-displayd*, that machine needs no change here at all: one enabled
    /// provider, and it takes the screen.
    ///
    /// A rule written against the name would have made adopting COSMIC a
    /// supervisor change. This one makes it a document change.
    ///
    /// **The mode is not "which compositor", and the maintainer's next two
    /// values are why** — he could one day see a mobile mode too, and then even
    /// a `container` mode (.rulings/ R-312). A phone shell is a fourth provider
    /// of this one endpoint and costs this function nothing; a machine that
    /// runs containers has **no** provider, exactly like `headless`, and
    /// differs from it in things that have nothing to do with a screen. So the
    /// mode stays *what this machine is for*, and the compositor is one
    /// consequence of it among several — which is the only reading under which
    /// `container` and `desktop` are values of the same field.
    ///
    /// A machine with no compositor — every card written before this one, and
    /// any card whose document leaves `rivet-displayd` disabled — still has a
    /// launcher that takes DRM master, and it must go on working. So the old
    /// rule applies exactly when the new one finds nobody, and it stops
    /// applying by itself the moment a compositor is enabled. Nothing has to
    /// be switched over and no document has to be rewritten in step with an
    /// image.
    ///
    /// **It is not "either, whichever".** If a compositor exists it is the
    /// arbiter and the launcher is not, even though the launcher still
    /// declares the kiosk endpoint — because it still selects applications,
    /// which is the other half of `ADR-058`'s split. A rule that let both
    /// claim would put two programs on one DRM device, which is the
    /// arbitration `ADR-015` §6 point 3 exists to hold.
    fn drives_the_screen(def: &ServiceDef) -> bool {
        // NOTE: this cannot see the other definitions, so the migration clause
        // is resolved by the two CALLERS above, which both count claimants
        // across the whole set. Written this way so that "is this the
        // compositor" stays a property of one definition.
        Self::provides_the_compositor(def) || Self::controls_the_kiosk(def)
    }

    /// Apply the display gate: of everything OFFERED, only the selected one is
    /// enabled.
    ///
    /// **The same rule `crate::document::apply_auto_run` used to apply once**,
    /// moved here so it can be applied again. `enabled` on an offered
    /// application is now derived state — the document's answer narrowed by the
    /// selection — and this is the one place that derives it.
    ///
    /// Nothing outside the offered set is touched. A daemon is not on a screen
    /// and its `enabled` is the operator's alone.
    fn apply_display_gate(&mut self) {
        let selected = self.display_selection.clone();
        for managed in &mut self.services {
            // Only the offered set is derived; the launcher is not in it, so its
            // `enabled` stays exactly as the operator wrote it.
            if !self.display_offered.iter().any(|n| n == &managed.def.name) {
                continue;
            }
            managed.def.enabled = selected.as_deref() == Some(managed.def.name.as_str());
        }
    }

    /// Give the screen to `name`, or say why not.
    ///
    /// `rivet_core::kiosk`'s `select`. The convergence that acts on it runs at
    /// the bottom of `crate::main`'s loop, by the same rules every service is
    /// always subject to — `ADR-017` §6 still has no manual start, and this is
    /// not one: it changes what the machine is converging TOWARD.
    pub fn select_display(&mut self, name: &str) -> Result<String, String> {
        if !self.display_offered.iter().any(|n| n == name) {
            return Err(format!(
                "`{name}` is not offered a screen on this machine. An application is offered \
                 when the document enables it AND its manifest declares devices = \
                 [\"display\"] (ADR-015 §6). Offered here: {}",
                if self.display_offered.is_empty() {
                    "nothing".to_string()
                } else {
                    self.display_offered.join(", ")
                }
            ));
        }
        self.display_selection = Some(name.to_owned());
        self.apply_display_gate();
        Ok(format!("{name} holds the screen"))
    }

    /// Take the screen back from whatever holds it.
    ///
    /// # It closes the display channel, and 2026-09-05 is why
    ///
    /// **A display descriptor outlived the selection that produced it, and was
    /// delivered to an application forty seconds later.** `ADR-042` §8 is the
    /// account; the shape is that `crate::main`'s `grant_display` writes the
    /// lease into a `SOCK_SEQPACKET` pair **before** the spawn, deliberately, so
    /// that it is already buffered when the child first reads. That is correct
    /// while the spawn follows on the same convergence pass. It is not correct
    /// when the application is **not started** — one that has given up, or is in
    /// backoff — because the message then sits in the socket buffer while the
    /// launcher, seeing nothing running, revokes the lease.
    ///
    /// What the application eventually read was a revoked lease: a descriptor
    /// whose ioctls all answer and which describes a display with no connectors
    /// at all. On `rivet-01` it reported *"no connected output with a mode — is
    /// a cable in?"* and was read as a confinement regression.
    ///
    /// So the descriptor goes with the selection. [`Kernel::close_display`] is
    /// the same call [`Self::release`] makes when a service stops — one verb for
    /// one fact (`ADR-011`) — and `grant_display` re-opens the channel on every
    /// `select`, so nothing is lost by closing it here.
    ///
    /// **It does not close the channel of a service that still holds the
    /// screen and keeps running**, because there is no such case: clearing the
    /// selection is what [`Self::apply_display_gate`] then uses to stop it.
    pub fn return_display(&mut self, k: &mut dyn Kernel) -> String {
        let was = self.display_selection.take();
        if let Some(name) = was.as_deref() {
            k.close_display(name);
        }
        self.apply_display_gate();
        match was {
            Some(name) => format!("{name} was stopped and nothing holds the screen"),
            None => "nothing held the screen".to_string(),
        }
    }

    /// What holds the screen, for a report.
    #[must_use]
    pub fn display_selection(&self) -> Option<&str> {
        self.display_selection.as_deref()
    }

    /// **Tell this supervisor what the document said this machine is for.**
    ///
    /// `crate::main` calls it once at boot and again on every reload, from
    /// `crate::document::kiosk_mode`. A *setter* rather than a constructor
    /// argument for one reason and it is not convenience: **the document is
    /// re-read on `SIGHUP`**, so the answer has to be replaceable, and a fact
    /// that could be set only at construction would go stale the first time an
    /// operator wrote `[kiosk]` into the document of a running machine — which
    /// is precisely the change somebody makes over the control plane and then
    /// checks with `get status`.
    pub fn set_kiosk_mode(&mut self, kiosk: rivet_core::operation::status::KioskMode) {
        self.kiosk = kiosk;
    }

    /// **Tell this supervisor which services the document names for the two
    /// display roles.**
    ///
    /// Called once, at boot, and deliberately not on a reload: the kiosk socket
    /// is given to the named launcher when it is bound at boot, and a reload
    /// that moved the role without moving the socket would leave the two
    /// answering differently. The schema says these keys take effect on the
    /// next restart for that reason.
    pub fn set_display_authority(&mut self, authority: crate::document::DisplayAuthority) {
        self.display_authority = authority;
    }

    pub fn new(
        defs: Vec<ServiceDef>,
        limits: Limits,
        mut journal: Journal,
    ) -> Result<Self, DefError> {
        let mut services = Vec::with_capacity(defs.len());
        for def in defs {
            def.validate()?;
            if services
                .iter()
                .any(|m: &Managed| m.def.name == def.name)
            {
                return Err(DefError::Duplicate { name: def.name });
            }
            let corr = journal.next_correlation();
            services.push(Managed {
                def,
                state: State::Idle,
                attempts: 0,
            reason: None,
                refused_connects: false,
                corr,
                cgroup_warned: false,
                // Every service at boot is a first `start()` for this
                // process — `Managed::adoption_eligible`'s own doc.
                adoption_eligible: true,
                pending_uninstall: None,
                asked: Asked::Nothing,
                // **Nothing has been asked yet**, which `get status` reports as
                // `unknown` — `ADR-045` §4, and the honest answer for a
                // service that has not started.
                health: None,
                health_at_ms: None,
                next_health_ms: 0,
                health_silent: None,
            });
        }
        // **The offered set is read before the gate narrows it** — `PLAN.md`
        // §104.40. `enabled` at this moment is the document's own answer, which
        // is what "offered" means; a moment later it is the document's answer
        // narrowed by the selection, which is what "running" means.
        let display_offered = Self::offered_displays(&services);
        let mut sup = Self {
            services,
            display_selection: None,
            display_offered,
            // **Not `headless`.** The document is read by `crate::document` and
            // handed here by `crate::main`; until it is, this process has not
            // been told and says so. See [`Self::set_kiosk_mode`].
            kiosk: rivet_core::operation::status::KioskMode::unknown(
                "this supervisor has not been told what the document says — the desired state is \
                 read by the process that builds it, and it has not handed the answer over yet",
            ),
            display_authority: crate::document::DisplayAuthority::default(),
            limits,
            journal,
            machine_name: None,
            machine_slot: None,
            converged_at_ms: 0,
            mdns_inputs: None,
            leases: None,
            firewall_attempted: None,
            ingest_rate: rivet_core::journal::IngestRate::new(),
            phase: Phase::Converging,
        };
        // Nothing is selected at construction, so the gate disables every
        // offered application. `crate::main` sets the document's `[kiosk]
        // auto-run` immediately after, which is what makes a machine with one
        // configured come up running it.
        sup.apply_display_gate();
        Ok(sup)
    }

    /// **Create a service instance at run time** — `ADR-017` §3, ruled
    /// 2026-08-24.
    ///
    /// Until this existed the set a supervisor converged toward was whatever
    /// [`Self::new`] was given, and `cmd/rivet/src/deploy.rs` recorded the
    /// consequence: *a request written anywhere the supervisor could see it
    /// would not be noticed until the next boot*. The maintainer's 2026-08-24
    /// install-request ruling named this as the one new idea it needs.
    ///
    /// # What it is NOT, and this is the trap the ruling named
    ///
    /// **It does not touch `enabled`.** `ADR-017` §6 makes `enabled` the
    /// operator's entire control surface, and the tempting shape — compile the
    /// installer into [`crate::definitions::system_services`] disabled, and have
    /// a request flip the flag — would put a *second* writer on the one key the
    /// document owns, so `rivet status` and the document would disagree about
    /// a service the operator never mentioned. This creates an **instance** that
    /// no document names and no document can name, precisely so that §6 is left
    /// alone. The instance's own `enabled` is `true` because a definition built
    /// on request is a definition of a thing that should run; nothing ever reads
    /// it from anywhere but the definition itself.
    ///
    /// **It is not a `start` operation.** `ADR-017` §6 has no manual start and
    /// still does not: this adds a definition, and the next [`Self::converge`]
    /// decides — by exactly the rules every other service is subject to —
    /// whether to spawn it. A caller cannot start anything and cannot choose a
    /// state.
    ///
    /// # Replacing rather than duplicating
    ///
    /// A name already in the set is not a second entry: a duplicate would mean
    /// two `ServiceDef`s sharing one cgroup and one notify socket, which is what
    /// [`Self::new`] refuses at boot for the same reason.
    ///
    /// * an instance that has **run to completion** ([`State::Done`]) or given
    ///   up ([`State::Failed`]) is replaced, with fresh attempts and a fresh
    ///   correlation identifier — a second install is a second thing, not a
    ///   restart of the first;
    /// * anything else is [`AdoptError::Busy`]. **The one that matters is a
    ///   running install**: replacing its definition while a 640 MiB write is in
    ///   flight would leave the supervisor holding a path that is not the one
    ///   the running process was given, and the next exit would be attributed to
    ///   the wrong payload.
    /// * [`State::Idle`] is `Busy` too, and that is the conservative direction
    ///   rather than an oversight. It is the state every compiled-in definition
    ///   rests in when it is disabled, so replacing one would let this method
    ///   overwrite a system service — and for the one caller there is, `Idle`
    ///   only exists inside the single pass between the adoption and the
    ///   convergence that starts it.
    pub fn adopt(&mut self, def: ServiceDef) -> Result<(), AdoptError> {
        def.validate().map_err(AdoptError::Definition)?;
        let corr = self.journal.next_correlation();
        let fresh = Managed {
            def,
            state: State::Idle,
            attempts: 0,
            reason: None,
            refused_connects: false,
            corr,
            cgroup_warned: false,
            // A name never seen before is a first `start()`, same as at
            // boot. The `Some(at)` arm below overrides this: a REPLACED
            // entry shares its cgroup with whatever ran under this name
            // before, in this same supervisor lifetime, so it is not.
            adoption_eligible: true,
            pending_uninstall: None,
            asked: Asked::Nothing,
            health: None,
            health_at_ms: None,
            next_health_ms: 0,
            health_silent: None,
        };
        match self.index_of(&fresh.def.name) {
            Some(at) if matches!(self.services[at].state, State::Done | State::Failed) => {
                let mut fresh = fresh;
                // **Not a fresh name to this supervisor** — `Managed::adoption_eligible`'s
                // own doc. `Done` and `Failed` are reached only after at
                // least one `start()`, so a non-empty cgroup under this name
                // is a leftover to kill, never a process to adopt.
                fresh.adoption_eligible = false;
                self.services[at] = fresh;
            }
            Some(_) => {
                return Err(AdoptError::Busy {
                    name: fresh.def.name,
                });
            }
            None => self.services.push(fresh),
        }
        Ok(())
    }

    /// Give an **application** the document's answer about whether it runs.
    ///
    /// Returns `true` when the answer changed, so the caller can say so once
    /// rather than on every pass.
    ///
    /// # Why this exists, and why it is applications only
    ///
    /// `enabled` is the operator's whole control surface (§6) and the document
    /// is where they use it — but `crate::main` reads the document once, before
    /// this supervisor exists, so an answer written afterwards reached nothing
    /// until `crate::installed` was built. That module re-reads the document at
    /// the one moment an install finishes and calls this.
    ///
    /// **`Origin::Image` is refused rather than filtered**, and it is the
    /// important half. Re-applying the document to `rivet-mcpd` on an unrelated
    /// install would let a badly edited document take the control plane away
    /// from a machine mid-session, and `ADR-018` §8's argument is exactly that a
    /// machine which can be reached is one somebody can repair. A system service
    /// keeps the
    /// answer it booted with until the machine reboots and `crate::main` reads
    /// the document again with the console still attached.
    ///
    /// **It does not start or stop anything.** It sets a field; the next
    /// [`Self::converge`] decides, by the same rules every service is subject
    /// to. A stop an operator holds ([`Self::hold_service`]) is not released by
    /// this — `enabled` and the hold are separate facts (`ADR-017` §6).
    pub fn set_application_enabled(&mut self, name: &str, enabled: bool) -> bool {
        let Some(at) = self.index_of(name) else {
            return false;
        };
        let managed = &mut self.services[at];
        if managed.def.origin != Origin::Application || managed.def.enabled == enabled {
            return false;
        }
        managed.def.enabled = enabled;
        true
    }

    /// Take **everything a freshly read manifest decides** for an application
    /// this supervisor already knows. Returns `true` when anything changed.
    ///
    /// # The defect this closes — Shared Rivets, 2026-08-31
    ///
    /// A definition is built once, when discovery reads a manifest, and
    /// `crate::installed::reconsider` has taken exactly one field out of a
    /// re-reading since it was written: `enabled`. Everything else stayed as it
    /// was at the first reading.
    ///
    /// **That was harmless until a manifest field became something the
    /// supervisor acts on at spawn.** `restart`, `ready` and `state` are all
    /// reported as unenforced (`Manifest::unenforced`), the program path and the
    /// identity are derived from the *name* and so cannot go stale, and the
    /// memory request is applied from the document's side. The declared closure
    /// is the first manifest-derived value that reaches `execve`.
    ///
    /// It was found by doing it: `greeter` was reinstalled declaring
    /// `libgreet-2.0.0-…`, `current` pointed at the new manifest, the board
    /// answered `library = ["libgreet-2.0.0-…"]` — and `/proc/<pid>/environ`
    /// answered `LD_LIBRARY_PATH=/state/lib/libgreet-1.0.0-…/lib`. The
    /// application resolved a library its own manifest no longer named.
    ///
    /// # What this deliberately does NOT do
    ///
    /// **It does not restart anything.** A process that is already running keeps
    /// the environment it was `execve`'d with, because an environment cannot be
    /// changed from outside a running process — and stopping it to apply one
    /// would be `ADR-017` §6's decision, which is the operator's and is
    /// `PLAN.md` §19.4's open question. What this buys is that **whenever** the
    /// service next starts — a crash restart, a `service-restart`, the next boot
    /// — it starts against the manifest that is on the card rather than one that
    /// was there at some earlier moment.
    ///
    /// `crate::installed` says so in the record rather than leaving a reader to
    /// notice the version in `current` and the version in `/proc` disagree.
    /// # It took only the PROGRAM until 2026-08-31, and that was too narrow
    ///
    /// The first version of this function replaced `program` alone, because the
    /// declared closure was the field that had gone stale. **`listen` then went
    /// stale in exactly the same way, four hours later**: `aura-controller` was
    /// reinstalled declaring `listen = ["unix:kiosk.sock"]`, the board's copy of
    /// the manifest said so, and the supervisor opened no socket — because the
    /// definition it held had been built from the previous install's manifest,
    /// which declared none.
    ///
    /// So the rule is not *"take the program"*. It is **take everything the
    /// manifest decides**, which is `applications::definition`'s output minus
    /// the two fields that are not the manifest's:
    ///
    /// | Field | Whose | Taken here? |
    /// | :-- | :-- | :-- |
    /// | `program`, `restart`, `readiness`, `listen` | the manifest, inside the signature (`ADR-015` §4) | **yes** |
    /// | `memory` | both — the manifest requests, the document caps, the lower wins (`ADR-015` §3) | **yes**, and `crate::document` re-applies the cap on the same pass |
    /// | `enabled` | the document, alone (`ADR-015` §3) | no — [`Self::set_application_enabled`] |
    /// | `origin`, `volume`, `name` | the filesystem | no |
    pub fn take_application_definition(&mut self, name: &str, fresh: &ServiceDef) -> bool {
        let Some(at) = self.index_of(name) else {
            return false;
        };
        let managed = &mut self.services[at];
        if managed.def.origin != Origin::Application {
            return false;
        }
        // **The WHOLE definition, minus the one field that is not the
        // manifest's.** Corrected 2026-09-01, `PLAN.md` §105.7.
        //
        // This enumerated the fields it copied, and that list was wrong three
        // times. It began as `program` alone; `listen` was added four hours
        // later *"in the same shape, because the first fix took one field
        // instead of the rule"*; and on 2026-09-01 `connects` was found missing
        // the same way — a consumer's manifest asked for an endpoint, the
        // supervisor took the new program and the new `listen` from that very
        // manifest, and kept an empty `connects` from the old one. The
        // application was told nothing was granted, on a machine where it
        // plainly was.
        //
        // **An enumerated list is a promise to remember**, and every field
        // added to a manifest since has had to be remembered here. Taking the
        // whole thing inverts that: a new field is carried by default, and
        // anything that must NOT be is an explicit exception with a reason.
        //
        // `enabled` is the only exception, and `ADR-015` §3 is the reason:
        // whether a service runs is the **document's** answer and never the
        // manifest's. Every other field on `ServiceDef` is derived from the
        // manifest and the place it was found, both of which `fresh` already
        // has.
        let candidate = ServiceDef {
            enabled: managed.def.enabled,
            ..fresh.clone()
        };
        if managed.def == candidate {
            return false;
        }
        managed.def = candidate;
        true
    }

    /// Give **any** service the document's answer about whether it runs, for a
    /// `SIGHUP` reload. Returns `true` when the answer changed.
    ///
    /// # Why this is not [`Self::set_application_enabled`] with the filter moved
    ///
    /// The two have different callers and therefore different policies, and
    /// collapsing them would lose the reason each exists.
    ///
    /// [`Self::set_application_enabled`] is reached from an **install edge** —
    /// an event that is about one application and says nothing about whether the
    /// operator meant the rest of their document to be taken. Refusing
    /// `Origin::Image` there is right, and stays.
    ///
    /// A reload is the operator saying *apply my document now*, explicitly, so
    /// `Origin` is not the question. The question is whether stopping this
    /// service takes away the means of undoing that — and
    /// [`crate::definitions::REACHABILITY_SET`] is that answer.
    /// `crate::document::Occasion::may_stop` applies it **before** this is
    /// called, so a withheld service never reaches here and the journal note
    /// that says so is written where the document is read.
    ///
    /// **It does not start or stop anything**, exactly as its sibling does not:
    /// it sets a field and the next [`Self::converge`] decides. `ADR-032` §3's
    /// signal is not a way to start or stop one service, and it leaves a held
    /// stop in place.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        let Some(at) = self.index_of(name) else {
            return false;
        };
        let managed = &mut self.services[at];
        if managed.def.enabled == enabled {
            return false;
        }
        managed.def.enabled = enabled;
        true
    }

    /// **`ADR-017` §6's converge-only service control**, built 2026-08-29.
    /// The receiving half of `rivet apply service-restart <name> confirm`.
    ///
    /// `apply` is whether this is the confirmed act (`true`) or `ADR-006`
    /// §6.4's `plan` (`false`). **The two run the identical guard**, which is
    /// the property that matters: a plan that used a different rule from the
    /// apply would be a plan an operator could confirm and be refused, and this
    /// surface's whole claim is that its refusals are legible in advance.
    ///
    /// # The three refusals, and why they are the whole of it
    ///
    /// `ADR-017` §6 rules that this surface may only ask the supervisor to
    /// **converge**. There is no `stop` to refuse — the request grammar cannot
    /// spell one (`rivet_core::operation::service_restart::Request`) — so what
    /// is left to refuse is everything that would be a *start* of something the
    /// machine should not be running:
    ///
    /// | | |
    /// | :-- | :-- |
    /// | no such service | there is nothing to converge toward. It carries the list, because the next move is to type one of them |
    /// | the document disables it | **§6.3.2's central refusal.** Starting it would make the machine run what its own desired state says it should not, and that state would vanish at the next boot with nothing to say why. The sentence names `apply edit` and `apply reload` |
    /// | the machine is shutting down | every service is coming down and none is going back up, so a restart taken now is a stop with no start after it |
    ///
    /// # What it does NOT do, and this is the ordering that makes the answer
    /// reach the caller
    ///
    /// **It signals nothing.** It sets [`Managed::asked`] and returns; the
    /// next [`Self::converge`] decides, by exactly the rules every service is
    /// always subject to. That is `crate::reload`'s posture and it buys
    /// something extra here: `crate::main` writes the answer to the socket
    /// **before** it converges, so on a machine where the service being
    /// restarted is the one carrying the request — the control plane — the reply
    /// is written before the `SIGTERM`. It is still a race, because the reply
    /// then has to travel through that process, and
    /// `rivet_core::operation::service_restart::caveat` says so rather than
    /// pretending otherwise.
    ///
    /// # The attempt counter is reset, and that is a decision
    ///
    /// A service that has reached `ADR-017` §3's give-up threshold would
    /// otherwise be restarted once, fail, and be `Failed` again immediately —
    /// which is not what *discard current state* means. The counter already
    /// means *consecutive failures since something went right*: it is forgiven
    /// by `Limits::success_window_ms` for the same reason. An operator asking
    /// for a restart is the other thing that goes right, so it is forgiven here
    /// too, and the old value is journalled rather than lost.
    pub fn restart_service(
        &mut self,
        k: &mut dyn Kernel,
        name: &str,
        apply: bool,
    ) -> Result<rivet_core::operation::Restarting, rivet_core::operation::ServiceRestartRefusal> {
        use rivet_core::operation::{Doing, Restarting, ServiceRestartRefusal as Refusal};

        let Some(at) = self.index_of(name) else {
            return Err(Refusal::UnknownService {
                service: name.to_string(),
                known: self.services.iter().map(|m| m.def.name.clone()).collect(),
            });
        };
        if self.phase != Phase::Converging {
            return Err(Refusal::ShuttingDown {
                service: name.to_string(),
            });
        }
        if !self.services[at].def.enabled {
            return Err(Refusal::Disabled {
                service: name.to_string(),
            });
        }
        // **A held service is not restarted**, or a restart would release the
        // hold as a side effect and start what the operator stopped.
        if self.services[at].asked == Asked::Stop {
            return Err(Refusal::Held {
                service: name.to_string(),
            });
        }

        let was = state_word(&self.services[at].state);
        let doing = match self.services[at].state {
            State::Started { .. } | State::Adopted { .. } => Doing::Stop,
            State::Idle
            | State::Backoff { .. }
            | State::Failed
            | State::Done
            | State::WaitingForMaterial { .. } => Doing::Start,
            State::Stopping { .. } | State::Draining { .. } => Doing::Nothing,
        };
        let answer = Restarting {
            service: name.to_string(),
            was: was.to_string(),
            doing,
            // **The supervisor is the authority for this and this crate holds
            // the list.** `rivet-core` composes the sentence and never names a
            // service (`ADR-014` §3), so what crosses the socket is the answer
            // rather than the membership.
            reachability: crate::definitions::keeps_the_machine_reachable(name),
        };
        if !apply {
            // **A plan records nothing.** `ADR-017` §3's argument about a
            // console that cries wolf: a request that changed nothing is not an
            // event, and journalling every dry run would put a card write behind
            // a question.
            return Ok(answer);
        }

        let attempts_before = self.services[at].attempts;
        self.services[at].attempts = 0;
        self.services[at].asked = Asked::Restart;
        let corr = self.services[at].corr.clone();
        self.journal.emit(
            k,
            Severity::Info,
            name,
            &corr,
            "restart requested over the supervisor's control surface; the next convergence pass \
             performs it (ADR-017 §6). Nothing about what this machine SHOULD be running has \
             changed — `enabled` is configuration and this is an operation (ADR-017 §6)",
            vec![
                ("was".to_string(), Value::Str(was.to_string())),
                (
                    "doing".to_string(),
                    Value::Str(doing.as_str().to_string()),
                ),
                (
                    "attempts_forgiven".to_string(),
                    Value::Int(i64::from(attempts_before)),
                ),
            ],
        );
        Ok(answer)
    }

    /// **Sets `rivet-proxyd`'s STORED `listen`, and nothing else** — `ADR-046`
    /// §7, `ADR-061` §7, `.rulings/` R-221.
    ///
    /// It never opens or closes a socket. [`Self::start`] reads
    /// `def.listen` fresh at every (re)start — including one a restart
    /// drives — so the next start is what makes a change here live; this
    /// method only changes what that next start will see. `runtime`'s
    /// `reconcile_proxy_listen` is the caller, right after
    /// `runtime::publish_routes` derives whether the table wants port 80.
    ///
    /// **Narrow on purpose.** It refuses every name but `rivet-proxyd`'s, and
    /// answers `false` without changing anything: a general "change any
    /// service's sockets at runtime" primitive would be a far bigger door
    /// than the one problem this exists for.
    ///
    /// Returns whether the stored value actually changed, so the caller
    /// journals only a real change.
    pub fn set_proxy_listen(
        &mut self,
        name: &str,
        listen: Vec<rivet_core::package::application::Socket>,
    ) -> bool {
        if name != rivet_core::service::PROXYD_SERVICE_NAME {
            return false;
        }
        let Some(at) = self.index_of(name) else {
            return false;
        };
        if self.services[at].def.listen == listen {
            return false;
        }
        self.services[at].def.listen = listen;
        true
    }

    /// **Record `policy` as the firewall this process is about to ask for**,
    /// answering whether it differs from the last one — `ADR-023` §9. The
    /// caller applies only on `true`.
    pub fn take_firewall_change(&mut self, policy: &rivet_adapter_net::firewall::Policy) -> bool {
        if self.firewall_attempted.as_ref() == Some(policy) {
            return false;
        }
        self.firewall_attempted = Some(policy.clone());
        true
    }

    /// **Whether `rivet-proxyd` is presently in a state a change to
    /// [`Self::set_proxy_listen`] would not reach without a restart** —
    /// `Started` or `Adopted`, [`Self::restart_service`]'s own reading of
    /// "running" (`Doing::Stop` there is this same pair). `Idle` and every
    /// other state have not opened a listener yet, so the next ordinary start
    /// already picks up whatever is stored — nothing to warn about.
    #[must_use]
    pub fn service_is_running(&self, name: &str) -> bool {
        self.index_of(name).is_some_and(|at| {
            matches!(
                self.services[at].state,
                State::Started { .. } | State::Adopted { .. }
            )
        })
    }

    /// **Stop one application and hold it stopped, or release that hold.**
    /// The receiving half of `rivet apply service-stop <name> confirm` and
    /// `rivet apply service-start <name> confirm` — `ADR-017` §6, `.rulings/`
    /// R-183.
    ///
    /// `apply` is `false` for `ADR-006` §6's `plan`, and the two run the
    /// identical guard, so a plan an operator confirms is never refused on a
    /// rule the plan did not apply.
    ///
    /// # The refusals, in the order they are checked
    ///
    /// | | |
    /// | :-- | :-- |
    /// | a name the supervisor does not know | carries the list |
    /// | the machine is shutting down | every service is already coming down |
    /// | **a service that keeps the machine reachable** | stopping `rivet-mcpd` or `rivet-netd` cuts the operator off with the hold in place, and nothing could release it but a power cycle. Checked before the origin, so the sentence says the reason that matters |
    /// | a service from the image | this pauses applications; the image's services are the document's |
    /// | a stop of a service nothing enables | there is nothing running to stop |
    /// | a stop of a held service, a start of one that is not | neither reports success for having done nothing |
    ///
    /// # What it does NOT do
    ///
    /// **It signals nothing and removes nothing.** It sets [`Managed::asked`]
    /// and returns; the next [`Self::converge`] stops the service by the stop
    /// every service is subject to (`ADR-017` §3). It never reaches
    /// [`Self::remove_application`], so `/state/app/<name>` is untouched.
    ///
    /// **It never writes `enabled`.** `get status` keeps reporting what the
    /// document says, and the hold is reported as the service's `reason`.
    pub fn hold_service(
        &mut self,
        k: &mut dyn Kernel,
        kind: rivet_core::operation::ServiceHoldKind,
        name: &str,
        apply: bool,
    ) -> Result<rivet_core::operation::ServiceHold, rivet_core::operation::ServiceHoldRefusal> {
        use rivet_core::operation::service_hold::Doing;
        use rivet_core::operation::{
            ServiceHold as Hold, ServiceHoldKind as Kind, ServiceHoldRefusal as Refusal,
        };

        let Some(at) = self.index_of(name) else {
            return Err(Refusal::UnknownService {
                kind,
                service: name.to_string(),
                known: self.services.iter().map(|m| m.def.name.clone()).collect(),
            });
        };
        if self.phase != Phase::Converging {
            return Err(Refusal::ShuttingDown {
                kind,
                service: name.to_string(),
            });
        }
        let held = self.services[at].asked == Asked::Stop;
        let m = &self.services[at];
        let was = state_word(&m.state);
        let doing = match kind {
            Kind::Stop => {
                if crate::definitions::keeps_the_machine_reachable(name) {
                    return Err(Refusal::CarriesTheWayIn {
                        service: name.to_string(),
                    });
                }
                if m.def.origin != Origin::Application {
                    return Err(Refusal::NotAnApplication {
                        service: name.to_string(),
                    });
                }
                if held {
                    return Err(Refusal::AlreadyStopped {
                        service: name.to_string(),
                    });
                }
                if !m.def.enabled {
                    return Err(Refusal::NotEnabled {
                        service: name.to_string(),
                    });
                }
                match m.state {
                    State::Started { .. } | State::Adopted { .. } => Doing::Stop,
                    State::Stopping { .. } | State::Draining { .. } => Doing::Nothing,
                    State::Idle
                    | State::Backoff { .. }
                    | State::Failed
                    | State::Done
                    | State::WaitingForMaterial { .. } => Doing::Hold,
                }
            }
            Kind::Start => {
                if !held {
                    return Err(Refusal::NotStopped {
                        service: name.to_string(),
                    });
                }
                if m.def.enabled {
                    Doing::Start
                } else {
                    Doing::Nothing
                }
            }
        };
        let answer = Hold {
            kind,
            service: name.to_string(),
            was: was.to_string(),
            doing,
        };
        if !apply {
            // A plan records nothing — `restart_service`'s reason.
            return Ok(answer);
        }

        let corr = self.services[at].corr.clone();
        let message = match kind {
            Kind::Stop => {
                self.services[at].asked = Asked::Stop;
                "stop requested over the supervisor's control surface; the next convergence pass \
                 stops it, and it is NOT started again until `apply service-start` or the next \
                 boot (ADR-017 §6). `enabled` is unchanged and nothing under /state/app is touched"
            }
            Kind::Start => {
                self.services[at].asked = Asked::Nothing;
                // The attempt counter is forgiven for `restart_service`'s
                // reason: an operator asking for it to run is something going
                // right.
                self.services[at].attempts = 0;
                "the stop is released over the supervisor's control surface; the service converges \
                 by the document alone from the next pass (ADR-017 §6)"
            }
        };
        self.journal.emit(
            k,
            Severity::Info,
            name,
            &corr,
            message,
            vec![
                ("was".to_string(), Value::Str(was.to_string())),
                (
                    "doing".to_string(),
                    Value::Str(doing.as_str().to_string()),
                ),
            ],
        );
        Ok(answer)
    }

    /// **The reason `get status` gives for a held service**, while it is not
    /// running. One sentence, here, so the status and the journal cannot
    /// describe the same hold two ways.
    const HELD_REASON: &'static str = "stopped by `apply service-stop`, and held stopped until \
                                       `apply service-start` or the next boot; the document still \
                                       enables it";

    /// **This supervisor's own state, as `ADR-032` §3's query socket answers
    /// it.** Built 2026-08-26.
    ///
    /// The mapping from this crate's vocabulary onto
    /// `rivet_core::operation::status` is here and nowhere else: `ADR-014` §3
    /// forbids the core naming anything in a binary crate, so the domain has its
    /// own [`rivet_core::operation::ServiceStatus`] and this is the one place
    /// the two meet.
    ///
    /// **The state word is this crate's own, lowercased and not translated.**
    /// `AGENTS.md` §5 — report what the system said: a second vocabulary for
    /// what a service is doing would be a second answer, and an operator reading
    /// `backoff` in a status and `backoff` in the journal is reading one word.
    /// **The two facts this supervisor reads once and then knows** — the
    /// machine's name and the slot it booted from, handed over by
    /// `crate::main` rather than read here.
    ///
    /// `crate::naming` settles the name and the kernel's mount table names the
    /// slot, both before the first application starts and both in the process
    /// that already had to read them. Told rather than derived (`ADR-011`):
    /// this holds what it was given and computes nothing, so there is no second
    /// reader of either.
    pub fn set_machine_identity(&mut self, hostname: Option<String>, slot: Option<String>) {
        self.machine_name = hostname;
        self.machine_slot = slot;
    }

    /// **The machine's name again, as a reload settled it.** `crate::reload`
    /// runs `crate::naming` over the re-read document and hands the answer over
    /// here, so a renamed machine answers with its new name without a reboot.
    /// The slot is not the document's and does not change on a running machine,
    /// so it is left as boot set it.
    pub fn set_machine_name(&mut self, hostname: Option<String>) {
        self.machine_name = hostname;
    }

    /// **`rivet-mdnsd`'s inputs as they are now** — `crate::mdns`,
    /// `ADR-017` §3. The first call records them; each later one
    /// compares, and asks the next convergence to restart the responder when
    /// what it would announce has changed under it.
    ///
    /// A running responder is restarted when
    /// [`crate::mdns::Inputs::restart_needed_for`] says so; one that gave up
    /// (`Failed`, or backing off) is started again on any change, because
    /// what it gave up on may be what changed. One the document disables, or
    /// an operator holds stopped, is left alone: this follows the document
    /// and the operator, and never overrides either.
    pub fn follow_mdns_inputs(&mut self, k: &mut dyn Kernel, now: crate::mdns::Inputs) {
        let Some(was) = self.mdns_inputs.replace(now.clone()) else {
            return;
        };
        if was == now {
            return;
        }
        let name = crate::definitions::MDNSD_NAME;
        let Some(at) = self.index_of(name) else {
            return;
        };
        if !self.services[at].def.enabled || self.services[at].asked == Asked::Stop {
            return;
        }
        let wanted = match self.services[at].state {
            State::Started { .. } | State::Adopted { .. } => was.restart_needed_for(&now),
            State::Failed | State::Backoff { .. } => true,
            _ => false,
        };
        if !wanted {
            return;
        }
        let was_state = state_word(&self.services[at].state);
        self.services[at].attempts = 0;
        self.services[at].asked = Asked::Restart;
        let corr = self.services[at].corr.clone();
        let render = |i: &crate::mdns::Inputs| {
            let addresses: Vec<String> = i
                .addressed
                .iter()
                .map(|(iface, a)| format!("{iface}={a}"))
                .collect();
            format!(
                "name {} address {}",
                i.hostname.as_deref().unwrap_or("(none)"),
                if addresses.is_empty() {
                    "(none)".to_string()
                } else {
                    addresses.join(",")
                }
            )
        };
        self.journal.emit(
            k,
            Severity::Info,
            name,
            &corr,
            "the machine's name or address changed, so the mDNS responder is restarted to \
             announce what is true now; it reads both once, at its start",
            vec![
                ("was".to_string(), Value::Str(was_state.to_string())),
                ("before".to_string(), Value::Str(render(&was))),
                ("now".to_string(), Value::Str(render(&now))),
            ],
        );
    }

    /// `rivet-mdnsd`'s inputs as [`Self::follow_mdns_inputs`] last recorded
    /// them, or `None` before the first record. What
    /// `crate::runtime::deliver_mdns_projection` hands the responder.
    #[must_use]
    pub fn mdns_inputs(&self) -> Option<&crate::mdns::Inputs> {
        self.mdns_inputs.as_ref()
    }

    /// **The DHCP leases as they are now** — `crate::leases`, `ADR-004` §8.
    /// The first call records them; each later one compares, and runs
    /// `rivet-netd` again when a lease is new, different or gone.
    ///
    /// `rivet-netd` applies a lease once and exits, and the client that holds
    /// the lease is a service of its own; this is what carries a first lease,
    /// a renewal that changed anything, and a lease withdrawn by `nak` or
    /// `leasefail`, from the hook's file to the interface. The interface then
    /// carries exactly its lease's address, or none.
    pub fn follow_leases(&mut self, k: &mut dyn Kernel, now: crate::leases::Leases) {
        let Some(was) = self.leases.replace(now.clone()) else {
            return;
        };
        let changed = now.changed_since(&was);
        if changed.is_empty() {
            return;
        }
        self.rerun_netd(
            k,
            "a DHCP lease was recorded, changed or withdrawn, so rivet-netd is run again to make \
             the interface carry exactly its lease's address; it reads each lease once, at its \
             start, and is the one program that sets an interface's address (ADR-004 §8)",
            vec![("interfaces".to_string(), Value::Str(changed.join(",")))],
        );
    }

    /// **Ask the next convergence to run `rivet-netd` again**, for a reason
    /// the caller names.
    ///
    /// Whatever state it is in: one that ran to completion or gave up is
    /// started, and one still running is stopped and started, because it may
    /// already have read what changed. One the document disables or an
    /// operator holds stopped is left alone, as [`Self::follow_mdns_inputs`]
    /// leaves the responder.
    fn rerun_netd(&mut self, k: &mut dyn Kernel, why: &str, fields: Vec<(String, Value)>) {
        let name = crate::definitions::NETD_NAME;
        let Some(at) = self.index_of(name) else {
            return;
        };
        if !self.services[at].def.enabled || self.services[at].asked == Asked::Stop {
            return;
        }
        // `Idle` starts on this pass anyway, and a stop already under way
        // comes back by itself.
        if matches!(
            self.services[at].state,
            State::Idle | State::Stopping { .. } | State::Draining { .. }
        ) {
            return;
        }
        let was_state = state_word(&self.services[at].state);
        self.services[at].attempts = 0;
        self.services[at].asked = Asked::Restart;
        let corr = self.services[at].corr.clone();
        let mut fields = fields;
        fields.push(("was".to_string(), Value::Str(was_state.to_string())));
        self.journal.emit(k, Severity::Info, name, &corr, why, fields);
    }

    /// **A supplicant just started: if `rivet-netd` has already run, run it
    /// again.** The network a supplicant joins is told to it over its control
    /// socket and lives only in its memory, so a supplicant the supervisor
    /// restarted knows no network until `rivet-netd` tells it again. At boot
    /// the supplicant starts in the same pass as `rivet-netd`, which is then
    /// still running and reaches it by itself.
    fn follow_supplicant_start(&mut self, k: &mut dyn Kernel, started: &str) {
        let Some(at) = self.index_of(crate::definitions::NETD_NAME) else {
            return;
        };
        if !matches!(
            self.services[at].state,
            State::Done | State::Failed | State::Backoff { .. }
        ) {
            return;
        }
        self.rerun_netd(
            k,
            "a wireless supplicant was started after rivet-netd had run, so rivet-netd is run \
             again to tell it its network; a supplicant knows only what it is told over its \
             control socket (ADR-004 §4)",
            vec![("supplicant".to_string(), Value::Str(started.to_string()))],
        );
    }

    /// The machine's name as it was handed over by
    /// [`Self::set_machine_identity`], or `None` before then or on a machine
    /// `crate::naming` could not name.
    #[must_use]
    pub fn machine_name(&self) -> Option<&str> {
        self.machine_name.as_deref()
    }

    /// **What this machine tells an application about itself** — `ADR-057`,
    /// `.rulings/` R-199 D4.
    ///
    /// # It reads [`Self::status`] and adds nothing of its own
    ///
    /// Every live field here comes out of the same fold `get status` renders,
    /// so a dashboard on this machine's own screen and an operator asking over
    /// the control plane are reading **one derivation** of what is running.
    /// `ADR-011`: the alternative is two answers to one question, and the one
    /// most likely to disagree is `machine`, whose ordering clause
    /// (`unknown` above `degraded`) is exactly what a second implementation
    /// gets wrong.
    ///
    /// # Nothing here makes this supervisor go and look
    ///
    /// No file is opened, no directory walked and no service asked. That is
    /// `ADR-057` §4's rule and it is the reason the fact set is the shape it
    /// is: this socket is reachable by every application on the machine, and a
    /// question that cost a `statvfs` or a `sysfs` read would be a way for one
    /// of them to put I/O in the supervisor's loop — which is
    /// `.rulings/` R-195 D5's invariant, and the failure the screen this feeds
    /// exists to report.
    ///
    /// The one call out is [`Kernel::now_ms`], the monotonic clock, which is
    /// what turns two stored instants into the ages that cross the wire.
    pub fn facts(&self, k: &mut dyn Kernel) -> rivet_core::facts::Facts {
        let now = k.now_ms();
        let status = self.status();
        rivet_core::facts::Facts {
            protocol: rivet_core::facts::PROTOCOL,
            hostname: self.machine_name.clone(),
            version: status.version.clone(),
            slot: self.machine_slot.clone(),
            phase: status.phase.clone(),
            machine: status.machine,
            mode: status.kiosk.mode.clone(),
            view_age_ms: now.saturating_sub(self.converged_at_ms),
            services: status
                .services
                .iter()
                .map(|s| rivet_core::facts::ServiceFact {
                    name: s.name.clone(),
                    enabled: s.enabled,
                    state: s.state.clone(),
                    // **Narrowed to `u32` here rather than in the domain.** A
                    // pid is never negative; `ServiceStatus` carries an `i32`
                    // because that is what `waitpid` deals in, and a facts
                    // answer that rendered `-1` would be rendering a value no
                    // process has.
                    pid: s.pid.and_then(|p| u32::try_from(p).ok()),
                    restarts: s.restarts,
                    ready: s.ready,
                    health: s.health,
                    // **The age, computed now**, so nothing absolute crosses
                    // and no clock has to be agreed between two processes.
                    //
                    // **Found by name and not by position.** `status()` is a
                    // map over this same list so the two are in step today,
                    // and a `zip` would go on compiling and start pairing a
                    // service with another service's age the moment either
                    // side filtered anything. An age drawn against the wrong
                    // service is worse than no age at all, because it looks
                    // right.
                    health_age_ms: self
                        .services
                        .iter()
                        .find(|m| m.def.name == s.name)
                        .and_then(|m| m.health_at_ms)
                        .map(|at| now.saturating_sub(at)),
                })
                .collect(),
            // **Rendering is what bounds an answer**, not this: `Facts::render`
            // stops on a service boundary and counts what it dropped, because
            // the bound is a property of the datagram and not of the machine.
            truncated: 0,
        }
    }

    pub fn status(&self) -> rivet_core::operation::Status {
        rivet_core::operation::Status {
            phase: match self.phase {
                Phase::Converging => "running",
                Phase::ShuttingDown => "shutting-down",
                Phase::Finished => "finished",
            }
            .to_string(),
            // **The image's own version, read once from `crate::etc::VERSION`**
            // — always `Some` on THIS build. `R33`, `.rulings/` R-163.
            version: Some(crate::etc::VERSION.to_string()),
            // **Carried through, not recomputed.** `crate::document::kiosk_mode`
            // is the one reader of the `[kiosk]` table on this side, and a
            // second derivation here would be a second answer to what this
            // machine is for.
            kiosk: self.kiosk.clone(),
            // **The arbiter, and only while it is RUNNING.** The role itself is
            // a declaration — `connects = ["rivet-sysd:kiosk"]`, read by
            // `Self::display_role` — but a declaration is not a screen: an
            // arbiter that is `idle` holds no card, and reporting it would put
            // the badge back among the things a reader has to discount.
            //
            // `display_role` answers `Arbiter` for at most one service by
            // construction (the document names one service per role), so
            // `find` cannot be hiding a second.
            display_master: self
                .services
                .iter()
                .find(|m| {
                    matches!(
                        self.display_role(&m.def),
                        rivet_core::service::DisplayRole::Arbiter
                    ) && state_word(&m.state) == "started"
                })
                .map(|m| m.def.name.clone()),
            // **Carried, not derived.** This is the arbiter's own answer,
            // recorded when it called `select_display`, and it is the only fact
            // on this machine that says what is drawing. `kiosk.auto_run` is
            // beside it and is a different question — see `OUTPUTS`.
            display_leased: self.display_selection.clone(),
            // **`ADR-045` §4's fold, computed HERE and carried** — not left
            // for the CLI, the portal and the fleet server to each work out.
            // The rule is normative, it lives in one function, and the clause
            // most likely to be got wrong (`unknown` above `degraded`) is
            // exactly the one three readers would have had to reimplement.
            machine: rivet_core::health::machine(
                self.services
                    .iter()
                    .map(|m| (m.def.critical, self.health_state(m))),
            ),
            services: self
                .services
                .iter()
                .map(|m| rivet_core::operation::ServiceStatus {
                    name: m.def.name.clone(),
                    // **What the OPERATOR wrote, not what the display gate left
                    // behind** — corrected 2026-09-02, `PLAN.md` §112.4.
                    //
                    // `apply_display_gate` overwrites `def.enabled` for every
                    // offered display application, setting it to *is this the
                    // one that was selected*. That is right for convergence,
                    // which uses the field to decide what to start — and wrong
                    // for a status, which an operator reads beside the document
                    // they wrote.
                    //
                    // The symptom on the maintainer's board: the document said
                    // `[services.kiosk-demo] enabled = true` and `get status`
                    // answered `enabled: false`, with nothing to explain the
                    // contradiction. `§112.4` carried it as *a runtime/document
                    // disagreement nobody has chased* until it was.
                    //
                    // **Being offered IS the document having enabled it**, so
                    // the operator's answer is recoverable rather than lost:
                    // `select_display`'s own refusal spells the rule — *an
                    // application is offered when the document enables it AND
                    // its manifest declares `devices = ["display"]`*.
                    //
                    // Nothing is added to the frame to say the rest, because
                    // the frame already says it: an offered application that
                    // was not selected reads `enabled: true, state: idle`,
                    // and `kiosk.auto_run` beside it names the one that won.
                    // That is the 2026-08-31 ruling exactly — *enabled means on
                    // the launcher's list, and the launcher can have one
                    // auto-run.*
                    enabled: m.def.enabled
                        || self.display_offered.iter().any(|n| n == &m.def.name),
                    state: state_word(&m.state).to_string(),
                    // **The first pid of an adopted set**, and the set may hold
                    // more. A status that reported one number for several
                    // processes would be wrong in the direction that looks
                    // right; `rivet get log` is where the whole set appears, and
                    // `ADR-032` §3 item 1 keeps this answer to what the socket
                    // is for rather than growing it a field at a time.
                    pid: match &m.state {
                        State::Started { pid, .. } | State::Stopping { pid, .. } => Some(*pid),
                        State::Adopted { pids, .. } => pids.first().copied(),
                        _ => None,
                    },
                    restarts: m.attempts,
                    ready: matches!(m.state, State::Started { ready: true, .. }),
                    // **Only when it is not running.** A reason beside a
                    // `started` service is a sentence about the past, and a
                    // reader would take it for the present.
                    // **A held stop is the reason, over any older one** — the
                    // hold is why it is not running now, and `enabled: true`
                    // beside `idle` needs it said (`.rulings/` R-183).
                    reason: match m.state {
                        State::Started { .. } | State::Adopted { .. } => None,
                        _ if m.asked == Asked::Stop => Some(Self::HELD_REASON.to_string()),
                        _ => m.reason.clone(),
                    },
                    health: self.health_state(m),
                    // **The caption belongs to the state that came with it**,
                    // so a silent service loses both together rather than
                    // keeping a sentence that describes an opinion nothing
                    // holds any more. `ADR-045` §5 makes the caption part of
                    // the state rather than a log beside it, and this is that
                    // rule at the one place the two could come apart.
                    caption: match (&m.health, &m.health_silent) {
                        (Some((_, caption)), None) => caption.clone(),
                        _ => None,
                    },
                    critical: m.def.critical,
                })
                .collect(),
        }
    }

    /// **What one service's health IS, as an observer sees it** — `ADR-045`
    /// §4.2's `unknown`, produced here and nowhere else.
    ///
    /// Three inputs and one word:
    ///
    /// * a question that went unanswered wins, however good the last answer
    ///   was. **That is the reversal's whole point** — a wedged application's
    ///   last words were `healthy`, and reporting them would be the telling
    ///   design wearing the polling design's clothes;
    /// * otherwise the last answer, if there is one;
    /// * otherwise `unknown`: nothing has been asked yet, or the service speaks
    ///   no health at all, and both are *it has not said*.
    ///
    /// **A stopped service is `unknown` rather than `down`.** `down` is a thing
    /// a service says about itself; that a service is not running is already in
    /// `state`, `pid` and `reason`, and answering `down` here would be the
    /// supervisor putting a word in a mouth that is not there — and would make
    /// a deliberately disabled service drag a machine's answer with it.
    fn health_state(&self, m: &Managed) -> rivet_core::health::State {
        use rivet_core::health::State;
        if m.health_silent.is_some() {
            return State::Unknown;
        }
        m.health
            .as_ref()
            .map_or(State::Unknown, |(reported, _)| State::from(*reported))
    }

    // The four accessors below are the read side of `rivet status`
    // (`ADR-017` §3, `ADR-006` §6). Nothing calls them outside the tests
    // because no adapter exists to ask — there is no CLI crate and no API —
    // and `ADR-014` §3 says an adapter is where that question gets asked from,
    // not something to grow here.
    #[allow(dead_code)]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    #[allow(dead_code)]
    pub fn services(&self) -> &[Managed] {
        &self.services
    }

    #[allow(dead_code)]
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// **How an application is doing** — `PLAN.md` §108.2, for the kiosk
    /// socket's `health` verb.
    ///
    /// `None` when this machine has no such service, which is a different
    /// answer from "it is not running" and must stay one: a locked kiosk naming
    /// something that is not installed is a mistake in the document, and a
    /// launcher told `stopped` would wait for ever for a thing that will never
    /// start.
    ///
    /// **The count is the supervisor's own**, not a second tally kept
    /// elsewhere. That is the whole reason this method exists rather than the
    /// launcher watching processes.
    pub fn health_of(&self, name: &str) -> Option<(bool, u32, Option<u32>)> {
        self.services.iter().find(|m| m.def.name == name).map(|m| {
            (
                matches!(m.state, State::Started { .. } | State::Adopted { .. }),
                m.attempts,
                // `u32` on the wire because a pid is positive; a value that
                // will not convert is reported as none rather than as a number
                // nobody could act on.
                match &m.state {
                    State::Started { pid, .. } | State::Stopping { pid, .. } => Some(*pid),
                    State::Adopted { pids, .. } => pids.first().copied(),
                    _ => None,
                }
                .and_then(|p| u32::try_from(p).ok()),
            )
        })
    }

    /// **What is on the menu, and what this machine is for** — the kiosk
    /// socket's `offer` verb, `ADR-042` §8.2, 2026-09-05.
    ///
    /// Both halves of the answer are already here and were already the
    /// authority. `self.kiosk` is `crate::document::kiosk_mode`'s reading of
    /// the `[kiosk]` table, set at boot and re-set on every `SIGHUP`;
    /// `self.display_offered` is the set [`Self::select_display`] validates
    /// against. **That is the point of the verb rather than a convenience:**
    /// until 2026-09-05 `cmd/rivet-launcher` computed its own version of both
    /// by reading the document and walking `/state/app`, so the menu and the
    /// supervisor could disagree — and once `ADR-042` confined the launcher it
    /// could read neither, so the menu was empty and `auto-run` was never
    /// honoured.
    ///
    /// **The mode word is carried through unchanged** (`AGENTS.md` §5),
    /// including `unknown`. A supervisor that has not been told what the
    /// document says must not answer `headless`: `headless` is a reading and
    /// `unknown` is the absence of one, and a launcher that could not tell them
    /// apart would draw a menu on a machine that might be locked.
    #[must_use]
    pub fn kiosk_offer(&self) -> (String, bool, Option<String>, Vec<String>) {
        (
            self.kiosk.mode.clone(),
            self.kiosk.locked,
            self.kiosk.auto_run.clone(),
            self.display_offered.clone(),
        )
    }

    #[allow(dead_code)]
    pub fn state_of(&self, name: &str) -> Option<&State> {
        self.services
            .iter()
            .find(|m| m.def.name == name)
            .map(|m| &m.state)
    }

    /// Whether `run` may return. Only ever true after a shutdown completed.
    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    /// The journal, for a phase that runs beside supervision rather than inside
    /// it.
    ///
    /// One caller: `crate::etc`, which projects `/etc` before the first
    /// convergence (`ADR-017` §3.1.8). It needs to record what it did — a
    /// `/etc` that failed to mount and said nothing would be the defect
    /// `ADR-017` §8 item 2 rules against — and it is not a service, so it has
    /// no `ServiceDef` to speak through.
    pub fn journal_mut(&mut self) -> &mut Journal {
        &mut self.journal
    }

    /// Say something in the supervisor's own voice.
    pub fn note(&mut self, k: &mut dyn Kernel, sev: Severity, msg: &str, fields: Vec<(String, Value)>) {
        let corr = self.journal.next_correlation();
        self.journal
            .emit(k, sev, SELF_NAME, &corr, msg, fields);
    }

    /// How long the supervisor may sleep before it must look again.
    ///
    /// `None` means "nothing is pending, wait for an event" — which is what a
    /// supervisor with nothing to do must do. **Returning `Some(0)` when
    /// nothing is pending would be a busy loop**, and `ADR-017` §3.5's whole
    /// concern is a supervisor that burns a machine down by repeating itself.
    pub fn next_timeout_ms(&self, now_ms: u64) -> Option<u64> {
        let mut soonest: Option<u64> = None;
        let mut consider = |deadline: u64| {
            let remaining = deadline.saturating_sub(now_ms);
            soonest = Some(soonest.map_or(remaining, |s: u64| s.min(remaining)));
        };
        for m in &self.services {
            match &m.state {
                State::Backoff { until_ms } if m.def.enabled => consider(*until_ms),
                State::Stopping { deadline_ms, .. } => consider(*deadline_ms),
                // **Both**, not the sooner one picked here: a draining
                // service is woken at whichever of its own deadline and its
                // own poll comes first, and `consider` already reduces to
                // the minimum across every service — reducing here too would
                // be a second, redundant answer to the same question.
                State::Draining {
                    deadline_ms,
                    next_poll_ms,
                    ..
                } => {
                    consider(*deadline_ms);
                    consider(*next_poll_ms);
                }
                State::Adopted { next_poll_ms, .. } if m.def.enabled => consider(*next_poll_ms),
                State::WaitingForMaterial { next_check_ms, .. } if m.def.enabled => {
                    consider(*next_check_ms);
                }
                State::Started {
                    since_ms,
                    ready,
                    ready_reported,
                    ..
                } if m.def.readiness == Readiness::Notify && !*ready && !*ready_reported => {
                    consider(since_ms.saturating_add(self.limits.ready_timeout_ms));
                }
                _ => {}
            }
            // **The next health question** — `ADR-045` §3. Without this a
            // supervisor with nothing else pending sleeps indefinitely and the
            // poll happens only when something unrelated wakes it, which on a
            // quiet machine is never. That is the shape the reversal exists to
            // avoid: a cadence the supervisor owns and does not keep.
            if m.def.origin == Origin::Application && matches!(m.state, State::Started { .. }) {
                consider(m.next_health_ms);
            }
        }
        soonest
    }

    /// `SIGTERM` arrived. `ADR-017` §3.1.5 item 6 puts shutdown in PID 1; this
    /// is the supervisor's half — bring the children down and let PID 1 get on
    /// with it.
    pub fn begin_shutdown(&mut self, k: &mut dyn Kernel, how: Terminating) {
        if self.phase != Phase::Converging {
            // Said even so, and at `Info`. A second signal is ordinary -- an
            // impatient operator, or pid 1 escalating -- and a journal that
            // recorded the first and not the second would leave a reader
            // unable to tell a machine that ignored a signal from one that was
            // never sent a second.
            self.note(
                k,
                Severity::Info,
                "another shutdown request arrived; this supervisor is already stopping services",
                vec![("shutdown".to_string(), Value::Str(how.as_str().to_string()))],
            );
            return;
        }
        self.phase = Phase::ShuttingDown;
        // **`ADR-022` §12 item 7**: the reason, and whether a reboot or a
        // power-off. Both fields, because "shutdown requested" alone cannot
        // distinguish a clean restart from a power cut -- and §12.3 makes that
        // distinction the whole reason this record exists, since ADR-018 §5.1's
        // rollback path is entered by exactly one of them.
        self.note(
            k,
            Severity::Info,
            "shutdown requested; stopping services",
            vec![
                ("shutdown".to_string(), Value::Str(how.as_str().to_string())),
                (
                    // What this process was TOLD, never what it inferred.
                    // `AGENTS.md` §5, and here it is load-bearing: pid 1 stops
                    // this process with SIGTERM for a trial restart too, so the
                    // signal is the honest answer and the outcome is not this
                    // process's to claim.
                    "reason".to_string(),
                    Value::Str(format!("pid 1 sent {} (ADR-017 §3.1.10)", how.signal())),
                ),
            ],
        );
    }

    /// A service declared itself ready (`ADR-017` §3.3, §3.4).
    pub fn note_ready(&mut self, k: &mut dyn Kernel, service: &str) {
        let Some(index) = self.index_of(service) else {
            self.note(
                k,
                Severity::Warn,
                "readiness notification for an unknown service",
                vec![("svc_name".to_string(), Value::Str(service.to_string()))],
            );
            return;
        };
        let m = &mut self.services[index];
        if let State::Started { ready, since_ms, .. } = &mut m.state {
            if *ready {
                return;
            }
            *ready = true;
            let took = k.now_ms().saturating_sub(*since_ms);
            let (name, corr) = (m.def.name.clone(), m.corr.clone());
            self.journal.emit(
                k,
                Severity::Info,
                &name,
                &corr,
                "ready",
                vec![("after_ms".to_string(), Value::Int(as_i64(took)))],
            );
        } else {
            let (name, corr) = (m.def.name.clone(), m.corr.clone());
            // **S-3's remainder.** `note_output` and `note_status` are rate
            // limited by the service's own uid; this Warn, fired for every
            // `READY=1` a service the supervisor is not running sends, was
            // left unbounded and could evict the journal exactly as they
            // could.
            if self.suppress_if_over_limit(k, index, &name, &corr) {
                return;
            }
            self.journal.emit(
                k,
                Severity::Warn,
                &name,
                &corr,
                "readiness declared by a service the supervisor is not running",
                vec![],
            );
        }
    }

    /// A service supplied a human status string over the same socket.
    pub fn note_status(&mut self, k: &mut dyn Kernel, service: &str, text: &str) {
        let Some(index) = self.index_of(service) else {
            return;
        };
        let (name, corr) = (
            self.services[index].def.name.clone(),
            self.services[index].corr.clone(),
        );
        if self.suppress_if_over_limit(k, index, &name, &corr) {
            return;
        }
        self.journal.emit(
            k,
            Severity::Info,
            &name,
            &corr,
            "status",
            vec![("status".to_string(), Value::Str(text.to_string()))],
        );
    }

    /// **A service answered the health question** — `ADR-045` §3.
    ///
    /// Two of §3.1.1's three outcomes arrive here: *it answered*, and *it
    /// answered badly*. Both are facts and neither is silence, so a badly
    /// formed answer is journalled and leaves the service `unknown` rather
    /// than being ignored — an application whose word the supervisor cannot
    /// read is an application the operator needs told about.
    pub fn note_answered(&mut self, k: &mut dyn Kernel, service: &str, line: &str) {
        let Some(index) = self.index_of(service) else {
            return;
        };
        let (name, corr) = (
            self.services[index].def.name.clone(),
            self.services[index].corr.clone(),
        );
        // The answer is `HEALTH=<word>[ <caption>]`; anything else is a program
        // answering a question it did not understand.
        let Some(value) = line.trim_end().strip_prefix(&format!("{}=", rivet_core::health::KEY))
        else {
            self.journal.emit(
                k,
                Severity::Warn,
                &name,
                &corr,
                "this service answered the health question with something that is not a health \
                 line, so nothing is known about it (ADR-045 §3). An answer is \
                 `HEALTH=<healthy|degraded|down>[ <sentence>]` on the descriptor handed under \
                 LISTEN_FDNAMES entry `rivet-health`",
                vec![(
                    "answer".to_string(),
                    Value::Str(line.chars().take(120).collect::<String>()),
                )],
            );
            return;
        };
        match rivet_core::health::parse(value) {
            Ok((reported, caption)) => {
                let previous = self.services[index]
                    .health
                    .as_ref()
                    .map(|(r, _)| r.word().to_string());
                self.services[index].health = Some((reported, caption.clone()));
                // **Stamped with the answer, not with the question** —
                // `ADR-057` §3. The two fields are set together so that a word
                // can never reach a screen without an age beside it.
                self.services[index].health_at_ms = Some(k.now_ms());
                // **Cleared, because a service that has answered is no longer
                // silent.** Leaving it would keep the machine `unknown` on the
                // strength of a question that has since been answered.
                self.services[index].health_silent = None;
                // **Only when the answer CHANGED.** `ADR-045` §2: health is
                // state and a notification is an event, and journalling every
                // poll of every service would put a card write behind a
                // question asked twice a minute (`ADR-022` §8's cap is the
                // thing that would absorb it).
                if previous.as_deref() != Some(reported.word()) {
                    self.journal.emit(
                        k,
                        Severity::Info,
                        &name,
                        &corr,
                        "health",
                        vec![
                            (
                                "health".to_string(),
                                Value::Str(reported.word().to_string()),
                            ),
                            (
                                "caption".to_string(),
                                Value::Str(caption.unwrap_or_default()),
                            ),
                        ],
                    );
                }
            }
            Err(why) => {
                self.journal.emit(
                    k,
                    Severity::Warn,
                    &name,
                    &corr,
                    &why.to_string(),
                    vec![],
                );
            }
        }
    }

    /// **A service did not answer** — `ADR-045` §3's third outcome, and the
    /// one the reversal exists for.
    ///
    /// It leaves the service `unknown` (§4.2), which is different from
    /// `healthy` and different from `down`. **The previous answer is
    /// deliberately not kept as the reported state**: a wedged application's
    /// last words were `healthy`, and reporting them is precisely the failure
    /// the telling design could not avoid.
    pub fn note_unanswered(&mut self, k: &mut dyn Kernel, service: &str, why: &str) {
        let Some(index) = self.index_of(service) else {
            return;
        };
        let (name, corr) = (
            self.services[index].def.name.clone(),
            self.services[index].corr.clone(),
        );
        // **Said once per transition into silence**, for `note_answered`'s
        // reason: a service that has been wedged for an hour must not write a
        // record every thirty seconds about it.
        let first = self.services[index].health_silent.is_none();
        self.services[index].health_silent = Some(why.to_string());
        if first {
            self.journal.emit(
                k,
                Severity::Warn,
                &name,
                &corr,
                "this service did not answer the health question, so it is UNKNOWN rather than \
                 healthy (ADR-045 §3). Its process is still there — a crashed one would show \
                 in `state` and the restart count — which is the wedged-but-alive case this poll \
                 exists to catch",
                vec![
                    ("detail".to_string(), Value::Str(why.to_string())),
                    (
                        "last_answer".to_string(),
                        Value::Str(
                            self.services[index]
                                .health
                                .as_ref()
                                .map_or_else(|| "none".to_string(), |(r, _)| r.word().to_string()),
                        ),
                    ),
                ],
            );
        }
    }

    /// **A service asked to be polled now** — `HEALTH=` on `NOTIFY_SOCKET`,
    /// `ADR-045` §3.
    ///
    /// It moves the next question forward and does nothing else. The push
    /// carries no state and could not be believed if it did: an application
    /// that chose what its own health record said would have the power §3.1.1
    /// takes away, arriving through the side door.
    pub fn note_health_hint(&mut self, k: &mut dyn Kernel, service: &str) {
        let Some(index) = self.index_of(service) else {
            return;
        };
        self.services[index].next_health_ms = k.now_ms();
    }

    /// **Ask every running service how it is, when its interval is up** —
    /// `ADR-045` §3, *"the supervisor decides how often to ask"*.
    ///
    /// Called at the end of every convergence pass, after the starts and stops,
    /// so a service started this pass is asked on the next one rather than
    /// before its first instruction has run.
    ///
    /// **A failed ask is an answer.** `Kernel::ask_health` returning an errno
    /// means the socket could not be reached at all, which is *this service is
    /// not answering* — recorded through [`Self::note_unanswered`] rather than
    /// retried, because retrying in a tight loop is what the ADR forbids and
    /// the next interval is when it will be asked again.
    fn ask_health_where_due(&mut self, k: &mut dyn Kernel) {
        let now = k.now_ms();
        let due: Vec<String> = self
            .services
            .iter()
            .filter(|m| {
                m.def.origin == Origin::Application
                    && matches!(m.state, State::Started { .. })
                    && now >= m.next_health_ms
            })
            .map(|m| m.def.name.clone())
            .collect();
        for name in due {
            let Some(index) = self.index_of(&name) else {
                continue;
            };
            // **Set before the ask, not after.** An ask that fails still costs
            // an interval; without this the failing path would be asked again
            // on the very next pass, which on a busy machine is a tight loop
            // against a socket that is not there.
            self.services[index].next_health_ms = now + rivet_core::health::INTERVAL_MS;
            if let Err(e) = k.ask_health(&name) {
                self.note_unanswered(k, &name, &e.to_string());
            }
        }
    }

    /// Whether a captured line or a `STATUS=` update from the service at
    /// `index` exceeds that service's uid's share of the journal — and, when
    /// it does, files the same "N dropped" report the ingest socket's own
    /// limit files.
    ///
    /// **Returns `true` when the caller must stop and emit nothing.**
    /// `ADR-022` §8 caps the journal at a rolling size, and §11's per-uid
    /// limit bounds only what arrives over the ingest socket — a service's
    /// own captured stdout, stderr and `STATUS=` line were unbounded, so an
    /// application printing quickly could turn the whole journal over,
    /// supervisor's and kernel's records included. This is the same
    /// [`rivet_core::journal::IngestRate`] the ingest path uses, keyed the
    /// same way: by uid.
    ///
    /// **A root-run image service is unaffected.** Its identity is `None`
    /// (uid 0), and `IngestRate::admit` already exempts uid 0 unconditionally
    /// — the same exemption the ingest socket relies on, for the same reason:
    /// every image service shares that one uid, and a shared bucket would
    /// make them starve each other rather than bound any one of them.
    fn suppress_if_over_limit(
        &mut self,
        k: &mut dyn Kernel,
        index: usize,
        name: &str,
        corr: &str,
    ) -> bool {
        let uid = self.services[index]
            .def
            .program
            .identity
            .map_or(0, |identity| identity.uid);
        match self.ingest_rate.admit(uid, k.now_ms()) {
            rivet_core::journal::Admission::Accept => false,
            rivet_core::journal::Admission::Report { dropped } => {
                self.note_captured_dropped(k, name, corr, uid, dropped);
                true
            }
            rivet_core::journal::Admission::Drop => true,
        }
    }

    /// The report [`Supervisor::suppress_if_over_limit`] files once per
    /// [`rivet_core::journal::INGEST_REPORT_EVERY_MS`] window — the same
    /// shape [`Supervisor::note_ingest_dropped`] files for the ingest socket,
    /// attributed to the service whose own output was suppressed rather than
    /// to the supervisor.
    fn note_captured_dropped(
        &mut self,
        k: &mut dyn Kernel,
        name: &str,
        corr: &str,
        uid: u32,
        dropped: u64,
    ) {
        self.journal.emit(
            k,
            Severity::Warn,
            name,
            corr,
            "this service filed captured output or status faster than its uid's share of the \
             journal allows, so this many were DROPPED and their contents are not recoverable. \
             The journal is rolling and capped (ADR-022 section 8), so the alternative is that \
             this service evicts everything else on the machine. Records from other services are \
             unaffected",
            vec![
                (
                    "dropped".to_string(),
                    Value::Int(i64::try_from(dropped).unwrap_or(i64::MAX)),
                ),
                (
                    "per_second".to_string(),
                    Value::Int(i64::from(rivet_core::journal::INGEST_PER_SECOND)),
                ),
                (
                    "burst".to_string(),
                    Value::Int(i64::from(rivet_core::journal::INGEST_BURST)),
                ),
                ("uid".to_string(), Value::Int(i64::from(uid))),
            ],
        );
    }

    /// **A reclaim helper has reported** — `Kernel::reclaim_in_child`.
    ///
    /// Scratch is quiet on success, for `Self::empty_scratch`'s reason: it
    /// happens at every spawn and every exit. A removal's helper — an
    /// uninstall, a superseded library, a destroyed repository, `crate::reclaim`
    /// — is said at `Info`, because it is the moment the space a removal freed
    /// actually comes back, and removals are rare. A failure is a `Warn`, said
    /// once per helper: what was set aside is still on the card under its aside
    /// name, invisible to everything, which is a disk filling slowly rather than
    /// anything broken, and the next helper in that directory, or the next
    /// start, tries again.
    pub fn note_reclaimed(
        &mut self,
        k: &mut dyn Kernel,
        service: &str,
        dir: &str,
        prefix: &str,
        failure: Option<&str>,
    ) {
        let corr = match self.index_of(service) {
            Some(index) => self.services[index].corr.clone(),
            None => self.journal.next_correlation(),
        };
        if prefix == crate::reclaim::prefix("") {
            let path = ("path".to_string(), Value::Str(dir.to_string()));
            match failure {
                None => self.journal.emit(
                    k,
                    Severity::Info,
                    service,
                    &corr,
                    "what a removal set aside has been deleted, off the supervisor's loop, so the \
                     space it held is free again",
                    vec![path],
                ),
                Some(detail) => self.journal.emit(
                    k,
                    Severity::Warn,
                    service,
                    &corr,
                    "what a removal set aside could not all be deleted, so some of it is still on \
                     the card under a reclaim name in this directory. It is invisible to \
                     everything on the machine and costs only its space; the next removal here, \
                     or the next start, tries again",
                    vec![path, ("detail".to_string(), Value::Str(detail.to_string()))],
                ),
            }
            return;
        }
        match failure {
            None => self.journal.emit(
                k,
                Severity::Debug,
                service,
                &corr,
                "the application's old scratch contents were deleted, off the supervisor's loop",
                vec![],
            ),
            Some(detail) => self.journal.emit(
                k,
                Severity::Warn,
                service,
                &corr,
                "the application's old scratch contents could not all be deleted, so they are \
                 still on the card beside its scratch directory under a reclaim name. Nothing \
                 is broken by this on its own, and the next spawn or exit tries again",
                vec![("detail".to_string(), Value::Str(detail.to_string()))],
            ),
        }
    }

    /// One line a service wrote — `ADR-022` §4's second capture path.
    ///
    /// > **Text, wrapped.** `wpa_supplicant` and every other mechanism Rivet
    /// > did not write emit lines. The journal captures their standard output
    /// > and error and wraps each line as a record with the line as `msg`.
    ///
    /// **The severity is `Info` whichever stream it came from**, and that is a
    /// refusal to guess rather than an omission. A program that writes
    /// *everything* to stderr including its banner is ordinary, so mapping
    /// stderr to `warn` would make an ordinary boot read as a fault; and Rivet
    /// has no way to know what a mechanism means by a line. The stream is a
    /// field and the reader decides — `AGENTS.md` §5.
    ///
    /// **`svc` is the service's name, not the supervisor's**, which is what
    /// makes `rivet get log | where svc == wpa_supplicant` select the
    /// mechanism's own words alongside the supervisor's records about it. The
    /// correlation
    /// identifier is the service's current one, so a line ties back to the
    /// start that produced it (`ADR-022` §5).
    ///
    /// A line from a service this supervisor does not know is **dropped**,
    /// because there is no correlation identifier to give it and a record
    /// attributed to nothing is worse than one that is missing. It cannot
    /// happen through [`Kernel::wait`], whose captures are created by
    /// [`Kernel::spawn`], and the arm exists because the port does not say so.
    pub fn note_output(
        &mut self,
        k: &mut dyn Kernel,
        service: &str,
        stream: Stream,
        text: &str,
        truncated: bool,
    ) {
        let Some(index) = self.index_of(service) else {
            return;
        };
        let (name, corr) = (
            self.services[index].def.name.clone(),
            self.services[index].corr.clone(),
        );
        if self.suppress_if_over_limit(k, index, &name, &corr) {
            return;
        }
        let mut fields = vec![(
            "stream".to_string(),
            Value::Str(stream.as_str().to_string()),
        )];
        if truncated {
            fields.push((
                "truncated".to_string(),
                Value::Int(i64::try_from(crate::kernel::MAX_CAPTURED_LINE).unwrap_or(i64::MAX)),
            ));
        }

        // **`ADR-022` §4's FIRST path, arriving through the second one.**
        //
        // §4 has two: Rivet's own services emit records natively, and C
        // mechanisms emit lines that get wrapped. Both arrive here as bytes on
        // a pipe, because §4's "natively" has no channel yet — so a line from
        // `cmd/rivet-timed`, which emits proper §3 records, would be wrapped
        // as the `msg` of *another* record and its severity would become
        // `Info`. `rivet get log | where sev == error` would then miss a time
        // client that failed, which is a functional loss rather than an
        // untidiness.
        //
        // So a line that **parses as a record** is forwarded as one. This is
        // what the reader `ADR-022` §9 needed was for, used from the other
        // direction.
        //
        // **`svc` is overridden with the service the supervisor actually
        // started**, and `boot`, `mono`, `ts` and `actor` are the journal's as
        // they are for every other record. A service cannot therefore write a
        // record claiming to be a different service, or to belong to another
        // boot — which matters because the thing on the other end of the pipe
        // is a program, and `ADR-011`'s "one authority" for who said something
        // is the supervisor rather than the speaker. What the service keeps is
        // what only it knows: the severity, the message and the fields.
        if let Ok(record) = rivet_core::journal::Record::from_jsonl(text) {
            fields.push((
                "wrapped".to_string(),
                Value::Str("record".to_string()),
            ));
            // The child's own correlation identifier and monotonic reading,
            // kept as fields rather than dropped: they are the only place its
            // internal causal chain is written down, and the supervisor's are
            // about the *start*, not about what the service was doing.
            fields.push(("emitted_corr".to_string(), Value::Str(record.corr.clone())));
            fields.push(("emitted_mono".to_string(), Value::Float(record.mono)));
            for (n, v) in record.fields {
                if !Record::RESERVED_FIELD_NAMES.contains(&n.as_str())
                    && !fields.iter().any(|(existing, _)| *existing == n)
                {
                    fields.push((n, v));
                }
            }
            self.journal
                .emit(k, record.sev, &name, &corr, &record.msg, fields);
            return;
        }

        self.journal
            .emit(k, Severity::Info, &name, &corr, text, fields);
    }

    /// One record from `/dev/kmsg` — `ADR-022` §4.1's third source.
    ///
    /// > the firmware-loading failures that `ADR-004` §8 warns will dominate
    /// > Stage 2 — `brcmfmac` not finding its `clm_blob`, NVRAM under the wrong
    /// > board-specific name — appear only in kernel messages.
    ///
    /// **`svc` is `kernel`, not the supervisor's.** The supervisor is the
    /// transport; a record saying it reported a PCI error would attribute the
    /// kernel's words to the wrong speaker, and
    /// `rivet get log | where svc == kernel` is the query this exists for.
    ///
    /// **The correlation identifier is minted per record**, and that is
    /// deliberate rather than lazy: `ADR-022` §5 ties together the records of
    /// *one causal chain*, and a kernel message is not part of any chain
    /// Rivet started. Giving them all one identifier would say they were.
    ///
    /// A record that will not parse is journalled as a `Warn` **carrying the
    /// raw line**, never dropped: `AGENTS.md` §5, and a kernel whose format
    /// this build does not know is exactly when its messages matter most.
    pub fn note_kernel_message(&mut self, k: &mut dyn Kernel, record: &str) {
        let corr = self.journal.next_correlation();
        match crate::kmsg::parse(record) {
            Ok(message) => {
                self.journal.emit(
                    k,
                    message.severity(),
                    crate::kmsg::SERVICE,
                    &corr,
                    &message.text,
                    message.fields(),
                );
            }
            Err(fault) => {
                self.journal.emit(
                    k,
                    Severity::Warn,
                    crate::kmsg::SERVICE,
                    &corr,
                    "a /dev/kmsg record could not be read, so it is recorded as the kernel \
                     wrote it rather than dropped (ADR-022 §4.1)",
                    vec![
                        ("detail".to_string(), Value::Str(fault.to_string())),
                        (
                            "raw".to_string(),
                            Value::Str(record.trim_end().to_string()),
                        ),
                    ],
                );
            }
        }
    }

    /// One thing off the journal ingest socket — `ADR-022` §11's third source.
    ///
    /// **This is the whole of "the CLI is not a second writer".** `ADR-022` §7
    /// gives the journal one writer and it is still this process; what arrived
    /// here is a record somebody else *composed*, and the difference is that
    /// every column that says where a record sits in the machine's history —
    /// `boot`, `mono`, `ts`, `corr` — is stamped by [`Journal::emit_as`] out of
    /// this process's own clocks, exactly as it is for every other record.
    ///
    /// **What the sender keeps and what it cannot claim** is
    /// `rivet_core::journal::ingest`'s decision, not this method's, and that
    /// function's documentation carries it. The one thing decided here is the
    /// correlation identifier: **minted per record**, for
    /// [`Supervisor::note_kernel_message`]'s reason — `ADR-022` §5 ties together
    /// the records of one causal chain, and an ingested record belongs to a
    /// chain this process did not start. The sender's own identifier survives as
    /// `emitted_corr`, which is what makes its chain recoverable.
    pub fn note_ingest(
        &mut self,
        k: &mut dyn Kernel,
        peer: rivet_core::journal::Peer,
        what: &crate::kernel::Ingest,
    ) {
        use crate::kernel::Ingest;

        let corr = self.journal.next_correlation();
        let (held, why) = match what {
            Ingest::Line(line) => {
                // **The rate limit, and it is a rate limit rather than a
                // permission check** — `rivet_core::journal::IngestRate` says
                // in as many words what it does not stop. A uid over its rate
                // loses the record; it is not disconnected, not refused at
                // `connect(2)`, and not treated as hostile. The socket stays
                // `0666` and the ruling that made it so (`ADR-022` §11.1) is
                // untouched: what is bounded is how much of §8's rolling
                // storage one uid may turn over.
                match self.ingest_rate.admit(peer.uid, k.now_ms()) {
                    rivet_core::journal::Admission::Accept => {}
                    rivet_core::journal::Admission::Report { dropped } => {
                        self.note_ingest_dropped(k, peer, &corr, dropped);
                        return;
                    }
                    // Already said for this uid inside the reporting window.
                    // Saying it again is the flood, filed by the supervisor
                    // against its own name.
                    rivet_core::journal::Admission::Drop => return,
                }
                let composed = rivet_core::journal::ingest(line, &peer);
                self.journal.emit_as(
                    k,
                    composed.sev,
                    &composed.svc,
                    &corr,
                    &composed.actor,
                    &composed.msg,
                    composed.fields,
                );
                return;
            }
            Ingest::TooLarge { held } => (
                *held,
                "a process sent the journal a line longer than the cap without a newline, so \
                 the connection was closed and the bytes were DROPPED. A prefix of a record is \
                 not a record (ADR-022 §11.5). The sender's own stderr is where it went instead",
            ),
            Ingest::Unfinished { held } => (
                *held,
                "a process opened a journal ingest connection, wrote part of a line and did not \
                 finish it inside the deadline, so the connection was closed and those bytes \
                 were DROPPED (ADR-022 §11.5)",
            ),
        };
        // **A refusal is a record**, and it is the supervisor's own — the peer
        // did not compose it, so it carries this process's actor and service
        // rather than `uid:N`. What is the peer's is stated as fields.
        self.journal.emit(
            k,
            Severity::Warn,
            SELF_NAME,
            &corr,
            why,
            vec![
                ("held".to_string(), Value::Int(i64::try_from(held).unwrap_or(i64::MAX))),
                ("ingest_uid".to_string(), Value::Int(i64::from(peer.uid))),
                ("ingest_gid".to_string(), Value::Int(i64::from(peer.gid))),
                ("ingest_pid".to_string(), Value::Int(i64::from(peer.pid))),
            ],
        );
    }

    /// **A uid went over its share of the journal, and the journal says so.**
    ///
    /// The supervisor's own record — its actor and its service name — because
    /// the peer did not compose it and the thing being reported is a decision
    /// this process took. What is the peer's is stated as fields, exactly as it
    /// is for the two refusals above.
    ///
    /// **This is the sentence that keeps the mechanism honest.** A journal that
    /// silently discarded records would be answering *"there is nothing about
    /// that in the log"* falsely, which `ADR-022` §11.5 names as the one thing
    /// this system must not do — so the drop is bounded, counted, and written
    /// down. What cannot be written down is the *content* of what was dropped;
    /// that is the price of the bound and the message says so.
    fn note_ingest_dropped(
        &mut self,
        k: &mut dyn Kernel,
        peer: rivet_core::journal::Peer,
        corr: &str,
        dropped: u64,
    ) {
        self.journal.emit(
            k,
            Severity::Warn,
            SELF_NAME,
            corr,
            "a process filed journal records faster than one uid's share of the journal allows, \
             so this many were DROPPED and their contents are not recoverable. The journal is \
             rolling and capped (ADR-022 section 8), so the alternative is that this sender \
             evicts everything else on the machine. Records from other uids are unaffected",
            vec![
                (
                    "dropped".to_string(),
                    Value::Int(i64::try_from(dropped).unwrap_or(i64::MAX)),
                ),
                (
                    "per_second".to_string(),
                    Value::Int(i64::from(rivet_core::journal::INGEST_PER_SECOND)),
                ),
                (
                    "burst".to_string(),
                    Value::Int(i64::from(rivet_core::journal::INGEST_BURST)),
                ),
                ("ingest_uid".to_string(), Value::Int(i64::from(peer.uid))),
                ("ingest_gid".to_string(), Value::Int(i64::from(peer.gid))),
                ("ingest_pid".to_string(), Value::Int(i64::from(peer.pid))),
            ],
        );
    }

    /// Reap every child that has exited and account for it.
    ///
    /// Called on every wakeup rather than only on `SIGCHLD`: signals coalesce,
    /// and in the degraded mode where `signalfd(2)` was refused there is no
    /// `SIGCHLD` to react to at all.
    pub fn reap(&mut self, k: &mut dyn Kernel) {
        let now = k.now_ms();
        for (pid, status) in k.reap_ready() {
            self.note_exit(k, pid, status, now);
        }
    }

    /// Empty an application's scratch directory — `rivet_core::service::tmp_dir`.
    ///
    /// Called at exactly two MOMENTS, which is not the same as two call
    /// sites — S-1's remainder. The exit moment is reached through
    /// `Self::settle_scratch`, from every place a drain can settle this
    /// service into its rest state.
    ///
    /// # Why the lifecycle edges and NOT a timer
    ///
    /// A periodic sweep of a temporary directory has to decide whether a file
    /// is in use by a live process, which it cannot know — that is `tmpreaper`,
    /// and getting it wrong deletes a running operation's scratch file
    /// mid-write. **The supervisor knows for certain that nothing is using this
    /// directory at exactly two moments**: immediately before it spawns the
    /// child, and once this service's CGROUP is confirmed empty after an
    /// exit — not merely once the main pid is reaped, which is a weaker fact:
    /// a helper the main pid forked can survive its own reap and go on using
    /// the directory. `Self::settle_scratch` is reached only once the cgroup
    /// itself answers empty.
    ///
    /// `ADR-039` §2 is the other half of the argument. It separates a predicate
    /// over the clock from an event that fires, and this is neither: the moment
    /// it should happen is a lifecycle transition already observed here, so it
    /// needs none of a scheduled action's missed-occurrence or overlap
    /// semantics. **A scheduler would add hazards to buy nothing.**
    ///
    /// The exit moment is the one that reclaims the space; the spawn moment
    /// exists for the boot after a power cut, where no exit was ever seen.
    ///
    /// # What is NOT here
    ///
    /// **A size cap.** If an application ever leaks scratch while it is up,
    /// deleting under it is the wrong answer and a quota is the right one: a
    /// cap refuses the write and the application sees the error. Nothing has
    /// measured how large an application's scratch gets, so nothing is capped.
    ///
    /// # It is quiet on success
    ///
    /// A `Debug` record, because this runs at every spawn and every exit and an
    /// `Info` at each would bury the records around it — the journal already
    /// costs a reader the 200-record default. A failure is a `Warn`: it leaves
    /// `TMPDIR` naming a directory with somebody's leftovers in it, which is
    /// untidy rather than broken, and is worth saying exactly once.
    fn empty_scratch(&mut self, k: &mut dyn Kernel, index: usize, when: &str) {
        let Some(volume) = self.services[index].def.volume.clone() else {
            return;
        };
        // A service with no identity is not an application: built-in services
        // run as root and have no confinement and no scratch directory.
        let Some(id) = self.services[index].def.program.identity else {
            return;
        };
        let raw = self.services[index].def.name.clone();
        let Some(name) = rivet_core::service::ServiceName::new(&raw) else {
            return;
        };
        let corr = self.services[index].corr.clone();
        let path = rivet_core::service::tmp_dir(&volume, &name);

        // **Rename aside and remake, then delete in a helper** — `ADR-017` §3.
        // Not a tree walk that decides what to spare: nothing in here is worth
        // keeping. And not a delete in this loop: deleting costs time in
        // proportion to what the application left, while a rename is one
        // syscall and frees the name at once. `Kernel::reclaim_in_child`
        // deletes the aside directory off the loop.
        //
        // **`ENOENT` is success here, and that is a real distinction rather
        // than a shrug.** A directory that is not there is already empty: the
        // first spawn after an install, and every spawn on a machine where the
        // last exit tidied up. Treating it as a failure would put a `Warn` in
        // the journal at the start of every well-behaved application.
        let mut trouble: Option<String> = None;
        let (parent, leaf) = path.rsplit_once('/').unwrap_or((".", path.as_str()));
        let prefix = crate::reclaim::prefix(leaf);
        let moved = match crate::reclaim::rename_aside(k, &path, leaf) {
            Ok(_) => true,
            Err(e) if e == Errno::ENOENT => false,
            Err(e) => {
                trouble = Some(e.to_string());
                false
            }
        };
        if trouble.is_some() {
            // fall through to the report
        } else if let Err(e) = k.mkdir_mode(&path, rivet_core::service::TMP_DIR_MODE) {
            trouble = Some(e.to_string());
        } else if let Err(e) = k.chown(&path, id.uid, id.gid) {
            trouble = Some(e.to_string());
        }
        // Every entry carrying the prefix goes, not only this one, so an aside
        // directory a power cut left undeleted is reclaimed now. The helper's
        // own outcome arrives later, as `Event::Reclaimed`.
        if moved
            && let Err(e) = k.reclaim_in_child(&raw, parent, &prefix)
            && trouble.is_none()
        {
            trouble = Some(format!(
                "the old contents were set aside and the helper to delete them could not \
                 start: {e}"
            ));
        }

        match trouble {
            None => self.journal.emit(
                k,
                Severity::Debug,
                &raw,
                &corr,
                "the application's scratch directory is empty",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("when".to_string(), Value::Str(when.to_string())),
                ],
            ),
            Some(detail) => self.journal.emit(
                k,
                Severity::Warn,
                &raw,
                &corr,
                "the application's scratch directory could not be emptied, so TMPDIR names a \
                 directory holding whatever the last run left. Nothing is broken by this on its \
                 own; a directory that never empties is a disk filling up slowly",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("when".to_string(), Value::Str(when.to_string())),
                    ("detail".to_string(), Value::Str(detail)),
                ],
            ),
        }
    }

    fn note_exit(&mut self, k: &mut dyn Kernel, pid: i32, status: ExitStatus, now_ms: u64) {
        let Some(index) = self.services.iter().position(|m| match m.state {
            State::Started { pid: p, .. } | State::Stopping { pid: p, .. } => p == pid,
            _ => false,
        }) else {
            // A child this supervisor spawned and then lost track of. It is
            // still reaped — that is what `reap_ready` did — but it is worth
            // saying, because it means an accounting bug here, not there.
            self.note(
                k,
                Severity::Warn,
                "reaped a child the supervisor was not tracking",
                vec![
                    ("pid".to_string(), Value::Int(i64::from(pid))),
                    ("status".to_string(), Value::Str(status.to_string())),
                ],
            );
            return;
        };

        // **The exit edge is one of the two moments the scratch directory can
        // be emptied safely — but only once the CGROUP confirms nothing is
        // left, not merely because the main pid was reaped** — S-1's
        // remainder. A helper the main pid forked survives its own reap;
        // whichever drain this function starts below is what catches it, and
        // emptying unconditionally here would delete scratch out from under
        // a helper still using it.
        let name = self.services[index].def.name.clone();
        if matches!(
            k.observe(&name),
            Observation::Empty | Observation::Unavailable(_)
        ) {
            self.empty_scratch(k, index, "exit");
        }

        let m = &mut self.services[index];
        let corr = m.corr.clone();
        let was_stopping = matches!(m.state, State::Stopping { .. });
        let uptime = match m.state {
            State::Started { since_ms, .. } => now_ms.saturating_sub(since_ms),
            _ => 0,
        };

        // A service that stayed up for the success window has earned a clean
        // slate. Without this the failure count is cumulative over the
        // machine's whole uptime and a service that crashes once a month is
        // eventually marked permanently failed.
        if !was_stopping && uptime >= self.limits.success_window_ms && m.attempts > 0 {
            m.attempts = 0;
        }

        if was_stopping {
            self.journal.emit(
                k,
                Severity::Info,
                &name,
                &corr,
                "stopped",
                vec![("status".to_string(), Value::Str(status.to_string()))],
            );
            // **S-1's fix.** `Self::stop` signalled only this one pid;
            // `Self::begin_drain` asks the CGROUP before this service is
            // allowed to rest at `State::Idle`, which is what catches a
            // helper this exit never accounted for.
            self.begin_drain(k, index, now_ms, DrainRest::Idle);
            return;
        }

        let policy = m.def.restart;
        let severity = if status.is_success() {
            Severity::Info
        } else {
            Severity::Warn
        };
        // `Done` rather than `Idle`: convergence has no memory, and an `Idle`
        // enabled service is started on the next pass.
        m.state = if policy.wants_restart(status) {
            State::Idle
        } else {
            State::Done
        };
        self.journal.emit(
            k,
            severity,
            &name,
            &corr,
            "exited",
            vec![
                ("status".to_string(), Value::Str(status.to_string())),
                ("uptime_ms".to_string(), Value::Int(as_i64(uptime))),
                ("restart".to_string(), Value::Str(policy.as_str().to_string())),
            ],
        );

        if !policy.wants_restart(status) {
            // **The exit edge, and the release that was never on it** —
            // `PLAN.md` §123, Wave 2.3. This service is `Done`: it will not be
            // started again by anything short of an operator changing the
            // document, so what it held is not held for it any more. A
            // `oneshot` that finished used to leave its listening socket open
            // in this supervisor for the rest of the boot.
            self.release_finally(k, index);
            // **S-1's remainder.** A `oneshot`'s helper survives its own
            // reap exactly as a stopped service's can, and until now `Done`
            // was reached with no drain at all — the helper kept running,
            // under devices and sockets `release_finally` had just given
            // back, with the record reading `done`.
            self.begin_drain(k, index, now_ms, DrainRest::Done);
            return;
        }
        self.schedule_retry(k, index, now_ms);
    }

    /// Common to "it exited and should come back" and "it would not even
    /// start". Both are failures to have the service running, both must back
    /// off, and both must eventually give up — a spawn that fails every time
    /// would otherwise be an unthrottled loop, which is exactly the hazard
    /// `ADR-017` §3.5 names.
    fn schedule_retry(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let m = &mut self.services[index];
        m.attempts = m.attempts.saturating_add(1);
        let attempts = m.attempts;
        let name = m.def.name.clone();
        let corr = m.corr.clone();

        if attempts >= self.limits.give_up_after {
            m.state = State::Failed;
            let consequence = m.def.consequence.clone();
            self.journal.emit(
                k,
                Severity::Error,
                &name,
                &corr,
                "giving up; the service is marked failed and will not be restarted",
                vec![
                    ("attempts".to_string(), Value::Int(i64::from(attempts))),
                    (
                        "threshold".to_string(),
                        Value::Int(i64::from(self.limits.give_up_after)),
                    ),
                    // The reader of this line needs to know whether to care,
                    // and "giving up on rivet-netd" does not say. `ADR-017`
                    // §3.1.6's `consequence_of` mechanism, applied to a
                    // service rather than to a mount.
                    ("consequence".to_string(), Value::Str(consequence)),
                ],
            );
            // **Giving up is a terminal state, so what it held goes back** —
            // `PLAN.md` §123, Wave 2.3. This is the case the audit named:
            // a display owner that crashes is restarted five times and then
            // marked failed, and until now its uid kept `/dev/fb0` and the card
            // node while the splash never returned. A dead screen with nothing
            // saying why.
            self.release_finally(k, index);
            // **S-1's remainder.** A service that gives up can have a helper
            // outlive it exactly as a stopped or completed one can, and
            // until now `Failed` was reached with no drain at all.
            self.begin_drain(k, index, now_ms, DrainRest::Failed);
            return;
        }

        let delay = self.limits.backoff_ms(attempts);
        m.state = State::Backoff {
            until_ms: now_ms.saturating_add(delay),
        };
        self.journal.emit(
            k,
            Severity::Info,
            &name,
            &corr,
            "will retry after backoff",
            vec![
                ("attempt".to_string(), Value::Int(i64::from(attempts))),
                ("delay_ms".to_string(), Value::Int(as_i64(delay))),
            ],
        );
    }

    /// One convergence pass over every service.
    ///
    /// **Two sweeps, releases before acquisitions** — `PLAN.md` §123, Wave 2.7,
    /// and [`Action::releases`] carries the argument. It was one sweep, and a
    /// service whose name sorted earlier could be started and then have its
    /// device nodes taken away by the stop of the service it was replacing.
    ///
    /// **The second sweep decides again rather than reusing the first sweep's
    /// answers.** Deciding everything up front and then acting would be acting
    /// on a state that the first sweep has since changed — `start` can fail,
    /// give up, and clear the display selection, which reaches `enabled` on
    /// every offered application through `apply_display_gate`. `decide` is a
    /// pure function over state that is already in hand, so asking it twice
    /// costs nothing and cannot be stale.
    pub fn converge(&mut self, k: &mut dyn Kernel) {
        let now = k.now_ms();
        // **Stamped at the top of the pass and not the bottom**, so the age the
        // facts call reports is the age of the *reading* rather than of the
        // work that followed it — a pass that spent four seconds starting
        // something must not report a view four seconds fresher than it is.
        self.converged_at_ms = now;

        if self.phase == Phase::Converging {
            self.retry_refused_connects(k);
        }

        // Sweep one: everything that gives something back.
        for index in 0..self.services.len() {
            let action = {
                let m = &self.services[index];
                decide(self.phase, &m.def, &m.state, m.asked, &self.limits, now)
            };
            if action.releases() {
                self.perform(k, index, action, now);
                self.services[index].take_up_restart();
            }
        }

        // Sweep two: everything else.
        for index in 0..self.services.len() {
            let action = {
                let m = &self.services[index];
                decide(self.phase, &m.def, &m.state, m.asked, &self.limits, now)
            };
            if !action.releases() {
                self.perform(k, index, action, now);
            }
            // **Cleared by the pass that saw it, unconditionally** — so a
            // request is taken up exactly once and a service cannot be caught in
            // a restart loop by a flag nothing resets. It is right even when the
            // action was `Nothing` (a service inside `ADR-017` §3.5's stop
            // deadline): that service is already going where a restart would put
            // it and comes back because the document enables it, which is what
            // `Doing::Nothing` told the operator before they confirmed.
            //
            // **A held stop is NOT cleared here**, and that is the whole of
            // what makes it a hold: see `Managed::take_up_restart`.
            self.services[index].take_up_restart();
        }

        // **After the actions, not before.** A service asked to stop in the loop
        // above may already be down by the time it returns, and finishing in the
        // same pass is one less convergence an operator waits through. A service
        // still stopping is simply not ready yet and is looked at again next
        // pass — which is why this is a sweep rather than a callback.
        self.finish_uninstalls(k);

        // **The health poll, last** — `ADR-045` §3. After the starts and
        // stops so that a service started this pass is asked on the next one
        // rather than before its first instruction has run, and after the
        // uninstalls so nothing is asked of a service that has just gone.
        //
        // **Only while converging.** On the way down every service is being
        // stopped and *it did not answer* about a process that has been sent
        // SIGTERM is a true statement nobody needs.
        if self.phase == Phase::Converging {
            self.ask_health_where_due(k);
        }

        if self.phase == Phase::ShuttingDown && self.everything_is_down() {
            self.phase = Phase::Finished;
            self.note(k, Severity::Info, "all services stopped; exiting", vec![]);
        }
    }

    /// **Return to `Idle` every service refused only for its `connects`, once
    /// the enabled set satisfies it** — so the pass that follows starts it, as
    /// a boot that found the same set would have.
    ///
    /// The refusal in [`Self::start`] is final for the set it was decided
    /// against, and is not retried on a timer: nothing about the provider
    /// changes by waiting. What changes it is the operator — a reload enabling
    /// the provider, an install adding it — so this runs at the top of every
    /// pass, and costs nothing unless a service carries
    /// [`Managed::refused_connects`].
    ///
    /// A service whose `connects` still do not resolve is left `Failed` with
    /// its reason, and nothing is journalled for it: the refusal has already
    /// been said, and a pass is not news.
    fn retry_refused_connects(&mut self, k: &mut dyn Kernel) {
        if !self
            .services
            .iter()
            .any(|m| m.refused_connects && m.def.enabled && m.state == State::Failed)
        {
            return;
        }
        let mut all: Vec<(String, Vec<_>, Vec<_>)> = self
            .services
            .iter()
            .filter(|s| s.def.enabled)
            .map(|s| {
                (
                    s.def.name.clone(),
                    s.def.provides.clone(),
                    s.def.connects.clone(),
                )
            })
            .collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        let (_, refusals) = crate::endpoints::resolve(&all);
        let still_refused = |name: &str| {
            refusals.iter().any(|r| match r {
                crate::endpoints::Refusal::NoSuchProvider { consumer, .. }
                | crate::endpoints::Refusal::NotProvided { consumer, .. }
                | crate::endpoints::Refusal::Contested { consumer, .. } => consumer == name,
            })
        };
        for index in 0..self.services.len() {
            let m = &self.services[index];
            if !(m.refused_connects && m.def.enabled && m.state == State::Failed)
                || still_refused(&m.def.name)
            {
                continue;
            }
            let name = m.def.name.clone();
            let corr = m.corr.clone();
            let was = m.reason.clone().unwrap_or_default();
            let managed = &mut self.services[index];
            managed.state = State::Idle;
            managed.reason = None;
            managed.refused_connects = false;
            managed.attempts = 0;
            self.journal.emit(
                k,
                Severity::Info,
                &name,
                &corr,
                "the endpoints this application connects to now resolve, so the refusal that \
                 kept it from starting no longer applies and this pass starts it (ADR-015 §5.3)",
                vec![("was".to_string(), Value::Str(was))],
            );
        }
    }

    /// **Take up an uninstall.** `rivet_core::uninstall`, ruled 2026-08-25.
    ///
    /// Marks the service and disables it; the removal happens once the existing
    /// stop machinery has brought it down. An application that is not a service
    /// this supervisor knows — never discovered, or already gone — is removed
    /// straight away, because there is nothing to wait for.
    pub fn begin_uninstall(&mut self, k: &mut dyn Kernel, name: &str, purge: bool) {
        let Some(service) = rivet_core::service::ServiceName::new(name) else {
            self.note(
                k,
                Severity::Error,
                &format!(
                    "REFUSED an uninstall named {name:?}, which is not a usable application \
                     name, so nothing was removed"
                ),
                vec![("application".to_string(), Value::Str(name.to_string()))],
            );
            return;
        };

        match self.index_of(name) {
            // **A service from the image is not uninstallable, and refusing it
            // here is the only place that can be checked.** `rivet-core` sees
            // a name and cannot know what runs under it; this process knows the
            // origin. `rivet-mcpd` and `rivet-netd` live on read-only `/system`
            // (`ADR-018` §3) so the removal would fail anyway — but it would
            // fail *after* the service had been disabled and stopped, which is
            // a machine with no network and no way in at all, produced by a
            // command that then reported it could not finish.
            Some(at) if self.services[at].def.origin != Origin::Application => {
                self.note(
                    k,
                    Severity::Error,
                    "REFUSED an uninstall of a service that is part of the image, not an \
                     installed application. It is on read-only /system (ADR-018 §3) and there \
                     is nothing under /state/app to remove; nothing was stopped and nothing \
                     was removed",
                    vec![
                        ("application".to_string(), Value::Str(name.to_string())),
                        (
                            "origin".to_string(),
                            Value::Str(self.services[at].def.origin.as_str().to_string()),
                        ),
                    ],
                );
            }
            Some(at) => {
                self.services[at].pending_uninstall = Some(purge);
                // The stop itself is `decide`'s, through `enabled` — see
                // `Managed::pending_uninstall` for why this is not a new state.
                self.services[at].def.enabled = false;
                let running = !matches!(
                    self.services[at].state,
                    State::Idle | State::Failed | State::Done
                );
                self.note(
                    k,
                    Severity::Info,
                    if running {
                        "uninstall accepted; the application is being stopped first and will be \
                         removed when it is down (ADR-017 §3.5's deadline applies, and a SIGKILL \
                         if it will not go)"
                    } else {
                        "uninstall accepted; the application is not running, so it is removed now"
                    },
                    vec![
                        ("application".to_string(), Value::Str(name.to_string())),
                        ("purge".to_string(), Value::Bool(purge)),
                    ],
                );
                self.finish_uninstalls(k);
            }
            None => {
                self.note(
                    k,
                    Severity::Info,
                    "uninstall accepted for an application this supervisor does not run, so \
                     there is nothing to stop first",
                    vec![
                        ("application".to_string(), Value::Str(name.to_string())),
                        ("purge".to_string(), Value::Bool(purge)),
                    ],
                );
                // **The built-in volume, because there is no definition to
                // ask.** This branch is the one case where the volume cannot
                // come from the definition — the supervisor does not run this
                // application, so it has none. A machine with more than one
                // volume will have to search the attached set for the
                // application's directory and refuse if two hold it
                // (`ADR-031` §11 item 3); today there is one volume and this is
                // that search.
                self.remove_application(
                    k,
                    &service,
                    rivet_core::service::BUILT_IN_VOLUME,
                    purge,
                );
            }
        }
    }

    /// Remove every marked service that has reached a stopped state.
    fn finish_uninstalls(&mut self, k: &mut dyn Kernel) {
        let ready: Vec<(usize, bool)> = self
            .services
            .iter()
            .enumerate()
            .filter_map(|(at, m)| match (m.pending_uninstall, &m.state) {
                (Some(purge), State::Idle | State::Failed | State::Done) => Some((at, purge)),
                _ => None,
            })
            .collect();

        // Removed back to front so the earlier indices stay valid.
        let removing = !ready.is_empty();
        for (at, purge) in ready.into_iter().rev() {
            let name = self.services[at].def.name.clone();
            let Some(service) = rivet_core::service::ServiceName::new(&name) else {
                // Unreachable: a `ServiceDef` cannot hold a name that is not
                // one. Left as a refusal rather than a panic — this crate denies
                // `panic` and a supervisor that aborts is PID 1 restarting it.
                self.services[at].pending_uninstall = None;
                continue;
            };
            let Some(volume) = self.services[at].def.volume.clone() else {
                // Also unreachable, and refused for the sharper reason: the
                // guard above this loop already declines to uninstall an
                // `Origin::Image` service, so anything reaching here has a
                // volume. Guessing `/state` for one that somehow does not would
                // be a removal pointed at a path nothing derived.
                self.services[at].pending_uninstall = None;
                continue;
            };
            self.remove_application(k, &service, &volume, purge);
            self.services.remove(at);
        }
        if removing {
            // **The routing table follows the uninstall** — `ADR-046` §5. A
            // removed application's route leaves the table here rather than at
            // the next install or boot. The proxy reads the table at its start
            // (`ADR-046` §7), so a running proxy serves the old row until it is
            // restarted; the table on disk is what that restart reads, and a
            // stale one would put the route back. `reconcile_proxy_listen`
            // journals the restart command when the removed route was the only
            // plain one, exactly as it does after an install.
            let applications = crate::runtime::installed_applications(self.services());
            let plain = crate::runtime::publish_routes(k, &mut self.journal, &applications);
            crate::runtime::reconcile_proxy_listen(k, self, plain);
            // **The firewall follows it too** — `ADR-023` §9. A removed proxied
            // application's port leaves the policy here rather than at the next
            // install or boot.
            crate::runtime::reconcile_firewall(k, self);
            // **And the backup surface** — `ADR-051`, derived from the same
            // manifests as the routing table. A removed application's surface
            // leaves the table here, so a backup client is never sent to an
            // application that is gone.
            crate::runtime::publish_backups(k, &mut self.journal, &applications);
        }
    }

    /// The removal itself, **by renaming aside** — `ADR-017` §3,
    /// `crate::reclaim`. What is removed is gone from every listing when this
    /// returns and is reported; a forked helper deletes it afterwards.
    fn remove_application(
        &mut self,
        k: &mut dyn Kernel,
        name: &rivet_core::service::ServiceName,
        // The volume the definition said it was on (`ADR-031` §8 item 1), not a
        // constant: removing an application from the wrong volume would leave
        // the real one installed and report success.
        volume: &str,
        purge: bool,
    ) {
        let app_dir = rivet_core::service::app_dir(volume, name);
        let data_dir = rivet_core::service::data_dir(volume, name);

        let mut removed = 0usize;
        let mut trouble: Vec<String> = Vec::new();

        if purge {
            // **Everything, in one rename.** Nothing under the application's
            // directory is being kept, so the whole of it — versions, `current`
            // and data together — is renamed aside under the application root
            // at once. There is no moment at which the data is gone and the
            // versions are not, or the reverse.
            match crate::reclaim::set_aside(k, name.as_str(), &app_dir) {
                Ok(done) => {
                    removed += 1;
                    if let Some(e) = done.unforked {
                        trouble.push(format!(
                            "{app_dir} is removed, and the helper to delete what was set aside \
                             could not start ({e}); the next start deletes it"
                        ));
                    }
                }
                Err(e) => trouble.push(format!("{app_dir}: {e}")),
            }
        } else {
            // **`current` by name, and not by listing.** `Kernel::list_dir`
            // answers with **directories only** — deliberately, for
            // `crate::applications`, where following a link would let one into
            // `/system` present itself as an installed application. `current`
            // is a symlink (`ADR-015` §7), so a remover that enumerated would
            // never see it.
            //
            // **A board found that**, on 2026-08-25: the first real uninstall
            // reported success and left `current` dangling. The fake had
            // listed it, because it was more permissive than the kernel — which
            // is the failure mode a fake has, and the fake is corrected too.
            //
            // The tree's shape is known rather than discovered: `ADR-015` §7
            // puts exactly three kinds of thing here — version directories,
            // `current`, and `data` — so naming the link is not a guess.
            //
            // `crate::applications::remove_versions` is that walk, shared with
            // the migration that retires an application the image now defines
            // (`ADR-060`), so the two cannot disagree about what "keep the data"
            // leaves behind.
            (removed, trouble) = crate::applications::remove_versions(k, name.as_str(), &app_dir);
            // The application's own directory stays: it holds `data`, which is
            // the whole point of not purging.
        }

        let kept = if purge {
            Value::Bool(false)
        } else {
            Value::Str(data_dir)
        };
        let severity = if trouble.is_empty() {
            Severity::Info
        } else {
            Severity::Warn
        };
        self.note(
            k,
            severity,
            if trouble.is_empty() {
                "the application is removed"
            } else {
                "the application is PARTLY removed; some of it is still on /state and is named \
                 below"
            },
            vec![
                ("application".to_string(), Value::Str(name.to_string())),
                ("removed".to_string(), Value::Int(removed as i64)),
                ("keeps".to_string(), kept),
                (
                    "trouble".to_string(),
                    Value::Str(trouble.join("; ")),
                ),
            ],
        );
    }

    fn everything_is_down(&self) -> bool {
        self.services.iter().all(|m| {
            matches!(
                m.state,
                State::Idle | State::Failed | State::Done | State::WaitingForMaterial { .. }
            )
        })
    }

    fn perform(&mut self, k: &mut dyn Kernel, index: usize, action: Action, now_ms: u64) {
        match action {
            Action::Nothing => {}
            Action::Start => self.start(k, index, now_ms),
            Action::Stop => self.stop(k, index, now_ms),
            Action::Kill => self.escalate(k, index, now_ms),
            Action::Abandon => self.abandon(k, index, now_ms),
            Action::StopAdopted => self.stop_adopted(k, index, now_ms),
            Action::PollAdoption => self.poll_adoption(k, index, now_ms),
            Action::PollDrain => self.poll_drain(k, index, now_ms),
            Action::EscalateDrain => self.escalate_drain(k, index, now_ms),
            Action::AbandonDrain => self.abandon_drain(k, index),
            Action::ReportNotReady => self.report_not_ready(k, index, now_ms),
            Action::Cancel => self.services[index].state = State::Idle,
        }
    }

    /// Bring a service up — observing before spawning, per `ADR-017` §3.1.2.
    fn start(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let name = self.services[index].def.name.clone();
        let corr = self.journal.next_correlation();
        self.services[index].corr = corr.clone();

        // **S-1's fix.** Adoption is for a SUPERVISOR restart, so it is only
        // ever legitimate on the very FIRST `start()` a service gets from
        // this process — `Managed::adoption_eligible`'s own doc. Read and
        // consumed here, before the observation, so every path out of this
        // function (adopt, kill leftovers, or spawn cleanly) counts as that
        // first `start()` equally.
        let may_adopt = self.services[index].adoption_eligible;
        self.services[index].adoption_eligible = false;

        // §3.1.3: the cgroup is the authority for what is running. The
        // supervisor may have been restarted while this service kept serving.
        match k.observe(&name) {
            Observation::Members(pids) => {
                if may_adopt {
                    self.services[index].state = State::Adopted {
                        pids: pids.clone(),
                        next_poll_ms: now_ms.saturating_add(self.limits.adopt_poll_ms),
                    };
                    self.journal.emit(
                        k,
                        Severity::Info,
                        &name,
                        &corr,
                        "already running; adopted rather than started",
                        vec![("pids".to_string(), Value::Str(join_pids(&pids)))],
                    );
                    return;
                }
                // **Not a supervisor restart — this same process already ran
                // this service once.** A non-empty cgroup here is leftovers
                // an earlier stop failed to clean up, not a process to trust
                // (S-1's fix): kill them and wait for the cgroup to empty,
                // through the same `State::Draining` a stop edge uses. The
                // NEXT `start()` — reached once draining settles this
                // service back to `Idle` — observes an empty cgroup and
                // spawns normally.
                self.journal.emit(
                    k,
                    Severity::Warn,
                    &name,
                    &corr,
                    "a process was already in this service's cgroup, within this supervisor's \
                     own lifetime — not a restart this process could have inherited from, so it \
                     is leftovers an earlier stop did not clean up. Killing it rather than \
                     adopting it (S-1's fix, ADR-017 §3.1.1)",
                    vec![("pids".to_string(), Value::Str(join_pids(&pids)))],
                );
                self.begin_drain(k, index, now_ms, DrainRest::Idle);
                return;
            }
            Observation::Empty => {}
            Observation::Unavailable(e) => self.report_unobservable(k, index, &corr, e),
        }

        // **`ADR-055` §5: an enabled application missing its declared
        // material is HELD, not spawned and crash-looped.** After the
        // adoption check, for that check's own reason — an already-running
        // process is left exactly as it is, whatever its data directory
        // holds today — and before every other precondition, so a service
        // that will not run for this reason never reaches a check that
        // assumes it might.
        let missing = crate::material::missing(k, &self.services[index].def);
        if !missing.is_empty() {
            let already_announced = matches!(
                self.services[index].state,
                State::WaitingForMaterial { announced: true, .. }
            );
            let issued_here = crate::issuance::issues_here(k);
            let text = crate::material::describe(&name, &missing, issued_here);
            if !already_announced {
                self.journal.emit(
                    k,
                    Severity::Warn,
                    &name,
                    &corr,
                    &text,
                    vec![(
                        "missing".to_string(),
                        Value::Str(
                            missing
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    )],
                );
            }
            self.services[index].reason = Some(text);
            self.services[index].state = State::WaitingForMaterial {
                announced: true,
                next_check_ms: now_ms.saturating_add(self.limits.material_poll_ms),
            };
            return;
        }

        // **`ADR-015` §4.2: verify the signature, at every launch.** Ruled
        // 2026-08-25, and the argument was revocation rather than tampering:
        // `ADR-028` §5.5.4 makes revocation an image update, which replaces the
        // trust store — and without a check here, dropping a key stopped future
        // installs and did nothing about what that key had already put on the
        // machine. `crate::verify` carries the whole account.
        //
        // **Applications only, and that is not an exemption for the image's own
        // services.** A system service's program is on `/system`, which
        // `ADR-018` §3 makes read-only and which an A/B update replaces whole
        // and verified; there is no moment at which its bytes can differ from
        // the ones that were installed. An application's are on writable
        // `/state`. The check exists where the mutability is.
        //
        // **After the adoption check and before the notify socket**, so a
        // refusal leaves nothing open behind it: an adopted service is already
        // running and this supervisor did not start it, which is the shape
        // `ADR-017` §3.1.2 exists to avoid.
        if self.services[index].def.origin == Origin::Application {
            let name = self.services[index].def.name.clone();
            // **The version directory is derived from the program path**, not
            // rebuilt from the name. They must be the same tree — checking a
            // header beside one binary and exec'ing another would be a check
            // that proves nothing — and deriving it once is how that is
            // guaranteed rather than remembered. `.../current/bin/<name>` has
            // its last two components stripped.
            let program_path = self.services[index].def.program.path.clone();
            let version_dir = program_path
                .rsplit_once('/')
                .and_then(|(dir, _)| dir.rsplit_once('/'))
                .map(|(dir, _)| dir.to_string());
            let Some(version_dir) = version_dir else {
                self.journal.emit(
                    k,
                    Severity::Error,
                    &name,
                    &corr,
                    "REFUSED TO LAUNCH: this application's program path has no version \
                     directory above it, so there is nowhere for ADR-015 §4.2's signed \
                     header to be",
                    vec![("path".to_string(), Value::Str(program_path))],
                );
                self.services[index].state = State::Failed;
                self.services[index].reason = Some(
                    "REFUSED TO LAUNCH: no version directory above the program, so there is \
                     nowhere for ADR-015 §4.2's signed header to be"
                        .to_string(),
                );
                return;
            };
            if let Err(refusal) = verify::program(k, &version_dir, &program_path) {
                self.journal.emit(
                    k,
                    Severity::Error,
                    &name,
                    &corr,
                    &refusal.describe(),
                    vec![("fault".to_string(), Value::Str(refusal.word().to_string()))],
                );
                // `Failed`, not a restart loop: every refusal here is about
                // bytes on a disk, and spawning again changes none of them.
                // `ADR-017` §3.5's backoff is for a program that ran and
                // failed; this one was never run.
                self.services[index].state = State::Failed;
                // **The same sentence the journal got.** `AGENTS.md` §5: one
                // fact, one wording. A status that paraphrased would be a
                // second account of the same refusal.
                self.services[index].reason = Some(refusal.describe());
                return;
            }
        }

        // Readiness is declared over NOTIFY_SOCKET (ADR-017 §3.4). The socket
        // is created before the spawn, so a service that notifies immediately
        // cannot beat the supervisor to it.
        let mut program = self.services[index].def.program.clone();
        if self.services[index].def.readiness == Readiness::Notify {
            // The identity that will WRITE to it — `None` for a root service.
            // Taken from the program that is about to be spawned rather than
            // re-derived from the name, so the socket cannot end up owned by an
            // identity the child does not run as.
            let owner = self.services[index].def.program.identity;
            match k.open_notify(&name, owner) {
                Ok(path) => program.env.push(("NOTIFY_SOCKET".to_string(), path)),
                Err(e) => {
                    self.journal.emit(
                        k,
                        Severity::Warn,
                        &name,
                        &corr,
                        "no notification socket, so this service cannot declare readiness",
                        vec![("detail".to_string(), Value::Str(e.to_string()))],
                    );
                }
            }
        }

        // **The listening sockets, opened before the spawn** — `ADR-015` §5.2,
        // `ADR-017` §3.4, built 2026-08-31.
        //
        // A failure here is the service's, not the machine's: it is
        // overwhelmingly a port already in use, which is a fact about this
        // machine that a restart may resolve and that nothing else here should
        // be brought down for (`ADR-017` §3.1.4). So the service is left to the
        // ordinary failure path with the errno named, rather than being started
        // without the descriptors it declared — which would be the silent
        // wrong answer, a program blocking for ever on a socket nobody opened.
        let declared = self.services[index].def.listen.clone();
        // **The answering channel is opened for an APPLICATION and for nothing
        // else** — `ADR-045` §3's *"every application gets one"*, and
        // `SPEC-03` is the contract with an application.
        //
        // An image service is deliberately outside it. It runs as root, the
        // supervisor already holds its pid and cgroup, and none of the three in
        // the image speaks this protocol — so binding one would hand every
        // system service a socket nothing accepts on and then journal a warning
        // per boot saying so, which is a report about a machine that is fine.
        // **What it costs, said rather than discovered:** a `critical` image
        // service is permanently `unknown` and would drag `ADR-045` §4's fold
        // with it. `critical` defaults to false, so reaching that takes an
        // operator writing the key against a service that cannot answer it.
        let answers_health = self.services[index].def.origin == Origin::Application;
        if !declared.is_empty() || answers_health {
            let owner = self.services[index].def.program.identity;
            let proxied = self.services[index].def.proxied;
            match k.open_listeners(&name, &declared, proxied, owner) {
                Ok(mut names) => {
                    // **Where each port was bound, said at the bind** —
                    // `ADR-046` §7, `.rulings/` R-188. `127.0.0.1` is a proxied
                    // application's ordinary port, reachable only on this
                    // machine; an operator asking why the LAN cannot reach one
                    // reads the answer here rather than in a manifest.
                    let bound = render_bound(&declared, proxied);
                    if !bound.is_empty() {
                        self.journal.emit(
                            k,
                            Severity::Info,
                            &name,
                            &corr,
                            "this service's tcp and udp sockets are bound. 127.0.0.1 is reachable \
                             only on this machine, which is where a proxied application's ports \
                             listen unless its signed manifest declares one :direct; 0.0.0.0 is \
                             reachable from the network (ADR-046 §7)",
                            vec![
                                ("bound".to_string(), Value::Str(bound)),
                                (
                                    "listen".to_string(),
                                    Value::Str(render_sockets(&declared)),
                                ),
                            ],
                        );
                    }
                    // **Appended after the manifest's own**, so the descriptor
                    // indices a manifest implies do not move — `Kernel::open_health`.
                    if answers_health {
                        match k.open_health(&name) {
                            Ok(Some(token)) => names.push(token),
                            Ok(None) => {}
                            Err(e) => {
                                self.journal.emit(
                                    k,
                                    Severity::Error,
                                    &name,
                                    &corr,
                                    "this machine could not bind this application's health \
                                     channel, so it was NOT started. Every application is handed \
                                     one under the reserved LISTEN_FDNAMES entry whether or not \
                                     its manifest asks (ADR-045 §3), and starting it with a \
                                     name for a socket that was never bound would hand it a \
                                     descriptor that is not there",
                                    vec![("detail".to_string(), Value::Str(e.to_string()))],
                                );
                                self.services[index].state = State::Failed;
                                k.close_listeners(&name);
                                return;
                            }
                        }
                    }
                    // `LISTEN_FDS` and `LISTEN_PID` are the child's own, for the
                    // reason `Kernel::spawn` gives. `LISTEN_FDNAMES` is not — it
                    // depends on the manifest and not on the pid, so it travels
                    // the ordinary way.
                    if !names.is_empty() {
                        program
                            .env
                            .push(("LISTEN_FDNAMES".to_string(), names.join(":")));
                    }
                }
                Err(e) => {
                    self.journal.emit(
                        k,
                        Severity::Error,
                        &name,
                        &corr,
                        "this service declares listening sockets and this machine could not \
                         open them, so it was NOT started. An application cannot bind these \
                         itself — it runs as an unprivileged uid and, for a unix socket, the \
                         path is not its to choose (ADR-015 §5.2, §5.3). A port already in use \
                         is the usual cause and is a fact about this machine rather than about \
                         the artifact",
                        vec![
                            ("detail".to_string(), Value::Str(e.to_string())),
                            (
                                "listen".to_string(),
                                Value::Str(render_sockets(&declared)),
                            ),
                        ],
                    );
                    self.services[index].state = State::Failed;
                    return;
                }
            }
        }

        // **A provider re-grants its endpoints every time it binds them** —
        // `ADR-015` §5.3, fixed 2026-09-01. (A citation here named `PLAN.md`
        // a PLAN section that was never written; removed 2026-09-01 by the
        // audit in `§107.5`, rather than left pointing at nothing.)
        //
        // The grant is a chown of the socket FILE, and `open_listeners` above
        // has just unlinked the old file and bound a new one, root-owned `0600`
        // like any listener. Granting only when the CONSUMER starts was
        // therefore a grant that did not survive the provider restarting: the
        // consumer keeps running, its reconnect loop finds a socket its uid
        // cannot open, and nothing reports anything because neither service
        // failed.
        //
        // Found on a board. Restarting `aura-controller` to pick up a new build
        // left `aura-renderer` alive, drawing its last frame, and permanently
        // unable to reconnect — a frozen screen with two healthy services.
        if !self.services[index].def.provides.is_empty() {
            let mut all: Vec<(String, Vec<_>, Vec<_>)> = self
                .services
                .iter()
                .filter(|s| s.def.enabled)
                .map(|s| {
                    (
                        s.def.name.clone(),
                        s.def.provides.clone(),
                        s.def.connects.clone(),
                    )
                })
                .collect();
            all.sort_by(|a, b| a.0.cmp(&b.0));
            let (grants, _) = crate::endpoints::resolve(&all);

            for grant in grants.iter().filter(|g| g.provider == name) {
                // **A shared endpoint is root-owned and group-reachable**, not
                // chowned to a consumer — `.rulings/` R-206. One socket, many
                // consumers, and the group is what says which.
                //
                // Applied once per grant rather than once per endpoint:
                // setting the same owner and mode twice is idempotent and
                // costs a syscall, where tracking which grants share a path
                // would be a second answer to a question the path already
                // answers.
                if grant.shared {
                    if let Err(e) = k.chown(&grant.path, 0, grant.gid()) {
                        self.journal.emit(
                            k,
                            Severity::Warn,
                            &name,
                            &corr,
                            "this service provides a SHARED endpoint whose group could not be \
                             set, so no consumer can connect until it is rebound. A shared \
                             socket is root:<endpoint gid> at 0660 and the GROUP is the grant \
                             (.rulings/ R-206)",
                            vec![
                                ("path".to_string(), Value::Str(grant.path.clone())),
                                ("gid".to_string(), Value::Int(i64::from(grant.gid()))),
                                ("detail".to_string(), Value::Str(e.to_string())),
                            ],
                        );
                    } else if let Err(e) = k.chmod(&grant.path, 0o660) {
                        self.journal.emit(
                            k,
                            Severity::Warn,
                            &name,
                            &corr,
                            "this service provides a SHARED endpoint whose mode could not be \
                             set. It is root-owned at 0600, so its group cannot connect",
                            vec![
                                ("path".to_string(), Value::Str(grant.path.clone())),
                                ("detail".to_string(), Value::Str(e.to_string())),
                            ],
                        );
                    }
                    continue;
                }
                // The consumer's identity, taken from ITS definition rather
                // than re-derived from the name — `crate::applications` already
                // owns that derivation and two of them could disagree.
                let Some(identity) = self
                    .services
                    .iter()
                    .find(|s| s.def.name == grant.consumer)
                    .and_then(|s| s.def.program.identity)
                else {
                    continue;
                };
                if let Err(e) = k.chown(&grant.path, identity.uid, identity.gid) {
                    self.journal.emit(
                        k,
                        Severity::Warn,
                        &name,
                        &corr,
                        "this service provides an endpoint that could not be re-granted to its \
                         consumer, so that consumer cannot connect until IT restarts. The \
                         socket was just rebound and is root-owned again, which is what a \
                         grant undoes (ADR-015 §5.3)",
                        vec![
                            ("consumer".to_string(), Value::Str(grant.consumer.clone())),
                            ("path".to_string(), Value::Str(grant.path.clone())),
                            ("detail".to_string(), Value::Str(e.to_string())),
                        ],
                    );
                }
            }
        }

        // **The declared device capability, granted before the spawn** —
        // `ADR-015` §6.2, kiosk mode, 2026-08-31.
        //
        // Before, on `open_notify`'s terms: the node changes hands while the
        // service is not running, so there is no window in which a child could
        // reach a device it does not yet own — or, worse, start, find the node
        // unopenable, and give up before the grant landed.
        //
        // **An image service running as root never reaches this**: root
        // already reaches every node (§6.3), so there is nothing to grant. One
        // with an identity — the compositor's `display` — is granted exactly
        // as an application is.
        let declared_devices = self.services[index].def.devices.clone();
        // **The lessee's display grant is the render node and nothing else** —
        // `ADR-036` §3.1 as amended 2026-09-04 (`.rulings/` R-138). See
        // `Self::display_role` for how the two sides are told apart.
        let role = self.display_role(&self.services[index].def);
        // **The same list, twice used, walked once** — `ADR-011`. The chown
        // below expands the declared classes against the machine; `ADR-042`'s
        // ruleset needs exactly those paths, and expanding them a second time
        // would be a second answer to *"which nodes did this application get"*.
        let mut device_nodes: Vec<String> = Vec::new();
        if !declared_devices.is_empty()
            && let Some(identity) = self.services[index].def.program.identity
        {
            let granted = crate::devices::grant(
                k,
                &mut self.journal,
                &name,
                &corr,
                &declared_devices,
                role,
                identity,
            );
            self.journal.emit(
                k,
                Severity::Info,
                &name,
                &corr,
                "ENFORCED this application declared device classes and the supervisor has given \
                 the nodes to its own uid for as long as it runs (ADR-015 §6.2), except a render \
                 node, which is shared: root:render 0660, with the render group added to this \
                 service's groups. A LESSEE is \
                 given the render node only: it draws through the lease it is handed, and the \
                 card node and /dev/fb0 are doors around that lease (ADR-036 §3.1). Undeclared \
                 may not draw and needs nothing built: an application runs as its own uid and a \
                 devtmpfs node is 0600 root:root, so the kernel already refuses. This is NOT \
                 isolation — anything running as uid 0 reaches these whatever a manifest says, \
                 and there is no namespace hiding them from an application that declared nothing",
                vec![
                    (
                        "devices".to_string(),
                        Value::Str(
                            declared_devices
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ),
                    (
                        "display_role".to_string(),
                        Value::Str(
                            match role {
                                rivet_core::service::DisplayRole::Arbiter => {
                                    "arbiter: it is the compositor the machine document names, or \
                                     with no compositor enabled the named launcher, so it opens \
                                     the card node and holds DRM master"
                                }
                                rivet_core::service::DisplayRole::Lessee => {
                                    "lessee: it is handed a lease on RIVET_DISPLAY and opens no \
                                     display node"
                                }
                            }
                            .to_string(),
                        ),
                    ),
                    ("granted".to_string(), Value::Str(granted.join(", "))),
                ],
            );
            // **A render node is reached through the render group** — it is
            // shared, not chowned (`crate::devices::groups_for`), so the
            // group is what this service was granted.
            for gid in crate::devices::groups_for(&granted) {
                if !program.groups.contains(&gid) {
                    program.groups.push(gid);
                }
            }
            device_nodes = granted;
        }

        // **The Landlock ruleset** — `ADR-042`, ruled and built 2026-09-04.
        //
        // Here rather than in `crate::applications` because two of the three
        // inputs are facts about the running machine: which device nodes the
        // declared classes expanded to, and what Landlock this kernel offers.
        // The policy itself came from the manifest and is already on the
        // program.
        if let Some(policy) = program.confinement.as_mut() {
            for node in &device_nodes {
                policy.allow(
                    node.clone(),
                    rivet_core::confinement::device_access(),
                    rivet_core::confinement::Presence::Optional,
                    "a device node the declared class expanded to on this machine",
                );
            }

            // **Asked of the kernel at every start** (`AGENTS.md` §5), because
            // an A/B update replaces the kernel under a running `/state` and a
            // value read once at boot would answer for the wrong one.
            let (abi, errno) = k.landlock_abi();
            if abi < crate::landlock::MINIMUM_ABI {
                let text = format!(
                    "REFUSED TO LAUNCH: this kernel offers no Landlock (errno {errno}), so \
                     ADR-015 §5.1's filesystem restriction cannot be applied and this \
                     application would run able to read and write every path its uid can \
                     reach. ADR-042 §5 refuses rather than starting it: a Rivet ships its \
                     kernel and its image together (ADR-018 §5.2), so a machine without \
                     Landlock is a broken build of Rivet's own kernel and not a \
                     configuration an operator chose"
                );
                self.journal.emit(
                    k,
                    Severity::Error,
                    &name,
                    &corr,
                    &text,
                    vec![
                        ("landlock_abi".to_string(), Value::Int(i64::from(abi))),
                        ("errno".to_string(), Value::Int(i64::from(errno))),
                    ],
                );
                self.services[index].state = State::Failed;
                self.services[index].reason = Some(text);
                // **S-10's fix.** The device grant just above already ran, so
                // without this the app's uid keeps every node it was handed —
                // `/dev/input/*`, a render node, and for an arbiter `card*`
                // and `fb0` — for the rest of the boot, with nothing running
                // to use them (`devices.rs`'s own "permission held by
                // nothing" is exactly this).
                self.release(k, index);
                return;
            }

            let unhandled = crate::landlock::unhandled_words(abi);
            self.journal.emit(
                k,
                Severity::Info,
                &name,
                &corr,
                "ENFORCED this application is confined to the paths its manifest declares, by a \
                 Landlock ruleset the supervisor builds and the child applies between fork and \
                 exec (ADR-015 §5.1, ADR-042, built 2026-09-04). Everything not listed below is \
                 refused with EACCES, INCLUDING paths its uid could otherwise reach: /tmp, \
                 /system, and another application's installed files. RIVET_APP_DATA is writable \
                 and NOT executable. The ruleset is inherited by the execve and cannot be \
                 removed by anything the program does. This is NOT a mount namespace: the paths \
                 are still THERE and anything running as uid 0 still reaches them",
                vec![
                    ("landlock_abi".to_string(), Value::Int(i64::from(abi))),
                    (
                        "granted".to_string(),
                        Value::Str(
                            policy
                                .grants
                                .iter()
                                .map(|g| format!("{} ({})", g.path, g.access.words().join("+")))
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ),
                    // **Named, not counted.** A kernel too old for a right does
                    // not enforce it anywhere, which widens the sandbox in a
                    // way an operator can only reason about if they are told
                    // which right it was (ADR-042 §5).
                    (
                        "not_enforceable_on_this_kernel".to_string(),
                        Value::Str(if unhandled.is_empty() {
                            "none: this kernel handles every right the policy uses".to_string()
                        } else {
                            unhandled.join(", ")
                        }),
                    ),
                ],
            );
        }

        // **The granted endpoints, chowned before the spawn** — `ADR-015`
        // §5.3, built 2026-08-31.
        //
        // `crate::endpoints` carries why this is a chown and not the ADR's bind
        // mount, and what it costs. The order is the device grant's and for the
        // device grant's reason: the file changes hands while the consumer is
        // not running, so there is no window in which it could find the socket
        // unopenable and give up before the grant landed.
        if !self.services[index].def.connects.is_empty()
            && let Some(identity) = self.services[index].def.program.identity
        {
            // Every ENABLED definition, sorted by name, so the refusals and
            // the claimants they name read the same on every boot.
            let mut all: Vec<(String, Vec<_>, Vec<_>)> = self
                .services
                .iter()
                .filter(|s| s.def.enabled)
                .map(|s| {
                    (
                        s.def.name.clone(),
                        s.def.provides.clone(),
                        s.def.connects.clone(),
                    )
                })
                .collect();
            all.sort_by(|a, b| a.0.cmp(&b.0));
            let (grants, refusals) = crate::endpoints::resolve(&all);

            // A refusal that names this service is FINAL FOR THE ENABLED SET
            // it was decided against — the provider is absent, or does not
            // offer it, or another application claims it too. None of those is
            // fixed by trying again on a timer, so it is reported as the reason
            // the service is not running rather than swallowed into a restart
            // loop. It is fixed by the set changing, and
            // `Self::retry_refused_connects` looks again on every pass.
            let mine: Vec<&crate::endpoints::Refusal> = refusals
                .iter()
                .filter(|r| match r {
                    crate::endpoints::Refusal::NoSuchProvider { consumer, .. }
                    | crate::endpoints::Refusal::NotProvided { consumer, .. }
                    | crate::endpoints::Refusal::Contested { consumer, .. } => {
                        consumer == &name
                    }
                })
                .collect();
            if let Some(first) = mine.first() {
                let text = first.to_string();
                self.journal.emit(
                    k,
                    Severity::Error,
                    &name,
                    &corr,
                    "this application declares connects and the connection was refused, so it was \
                     NOT started. Starting it anyway would produce a program failing at a socket \
                     nobody granted it, which is the same silent wrong answer a missing listener \
                     would be (ADR-015 §5.3)",
                    vec![("detail".to_string(), Value::Str(text.clone()))],
                );
                self.services[index].reason = Some(text);
                self.services[index].state = State::Failed;
                self.services[index].refused_connects = true;
                // S-10's fix — see the same call after the Landlock refusal
                // above: the device grant already ran, and nothing releasing
                // it here would leave the app's uid owning nodes nothing is
                // using.
                self.release(k, index);
                return;
            }

            // The grant itself. `ENOENT` is the ordinary case and not a defect:
            // nothing orders a consumer after its provider, so the socket may
            // simply not be bound yet. The restart backoff (`ADR-017` §3.5) is
            // the mechanism for waiting and it already exists.
            for grant in grants.iter().filter(|g| g.consumer == name) {
                // **A shared endpoint is granted by GROUP, not by chown** —
                // `.rulings/` R-206. The socket is root-owned at `0660` and
                // this consumer is about to be spawned with its gid, so there
                // is nothing to chown here and chowning it would take the
                // socket away from every other consumer.
                //
                // The provider set the owner and mode when it bound; if it has
                // not bound yet the group below reaches a socket that does not
                // exist, and the connect fails with `ENOENT` in the
                // application — the same ordinary case the chown path treats
                // as a wait rather than a fault.
                if grant.shared {
                    if !program.groups.contains(&grant.gid()) {
                        program.groups.push(grant.gid());
                    }
                    continue;
                }
                if let Err(e) = k.chown(&grant.path, identity.uid, identity.gid) {
                    let text = format!(
                        "{}:{} could not be granted: {e} at {}",
                        grant.provider, grant.endpoint, grant.path
                    );
                    // **A-1's fix.** `ENOENT` means the providing service has
                    // not bound its socket yet — nothing orders a consumer
                    // after its provider, and services converge in name
                    // order, so on a first boot any consumer that sorts
                    // before its provider hits this every time. That is the
                    // ordinary case the comment above the loop already names,
                    // and the restart backoff it already cites is the
                    // mechanism for waiting — so this is transient rather
                    // than a resolution refusal. A refusal that is not
                    // `ENOENT` — the socket exists and the chown itself was
                    // refused — stays permanent.
                    if e == Errno::ENOENT {
                        self.journal.emit(
                            k,
                            Severity::Warn,
                            &name,
                            &corr,
                            "the endpoint this application connects to could not be handed to \
                             its uid yet, so it was NOT started this attempt. The providing \
                             service has not bound its socket yet — nothing orders one before \
                             the other",
                            vec![("detail".to_string(), Value::Str(text.clone()))],
                        );
                        self.services[index].reason = Some(text);
                        // S-10's fix, same reason as the two refusals above.
                        self.release(k, index);
                        self.schedule_retry(k, index, now_ms);
                        return;
                    }
                    self.journal.emit(
                        k,
                        Severity::Error,
                        &name,
                        &corr,
                        "the endpoint this application connects to could not be handed to its \
                         uid, so it was NOT started",
                        vec![("detail".to_string(), Value::Str(text.clone()))],
                    );
                    self.services[index].reason = Some(text);
                    self.services[index].state = State::Failed;
                    // S-10's fix, same reason as the two refusals above.
                    self.release(k, index);
                    return;
                }
            }

            if let Some(value) = crate::endpoints::env_for(&name, &grants) {
                self.journal.emit(
                    k,
                    Severity::Info,
                    &name,
                    &corr,
                    "ENFORCED this application declared connects and the supervisor has given each \
                     granted socket FILE to its own uid for as long as it runs (ADR-015 §5.3). One \
                     consumer per endpoint: the grant is a chown, and an endpoint more than one \
                     application asks for is refused to all of them, by name. This is NOT the ADR's bind mount — \
                     there are no mount namespaces here — so anything running as uid 0 reaches \
                     these sockets whatever a manifest says",
                    vec![(
                        rivet_core::service::CONNECT_ENV.to_string(),
                        Value::Str(value.clone()),
                    )],
                );
                program
                    .env
                    .push((rivet_core::service::CONNECT_ENV.to_string(), value));
            }
        }

        // **The other safe moment.** The exit edge above catches the ordinary
        // case; this one catches a machine that lost power mid-operation, where
        // no exit was ever observed and the debris outlived the boot.
        self.empty_scratch(k, index, "spawn");

        match k.spawn(&name, &program) {
            Ok(pid) => {
                self.services[index].state = State::Started {
                    pid,
                    since_ms: now_ms,
                    ready: false,
                    ready_reported: false,
                };
                // **Cleared on a successful spawn.** A reason that outlived its
                // cause is worse than none: an operator reading a refusal
                // beside a running service would believe the sentence over the
                // state. PLAN.md §104.22.
                self.services[index].reason = None;
                self.services[index].refused_connects = false;
                // **Not asked in the pass that started it, and not made to
                // wait a whole interval either** — `ADR-045` §3.
                //
                // A question sent before the child has reached its accept loop
                // is a question that times out, and a machine whose every
                // application was reported silent for its first two seconds
                // would be a machine reporting its own start-up as a fault.
                // One `PATIENCE_MS` is the smallest delay that is more than a
                // race and less than a policy: the first real answer lands a
                // few seconds into a service's life rather than half a minute.
                self.services[index].next_health_ms =
                    now_ms.saturating_add(rivet_core::health::PATIENCE_MS);
                // **A new process gets a fresh opinion.** Clearing on the
                // release edge is not enough and the difference is a real path:
                // a service with `restart = "always"` that crashes goes through
                // `schedule_retry`, which releases nothing, so without this
                // line the answer the DEAD process gave would be reported about
                // the live one — the wedged case's defect arriving by another
                // road.
                self.services[index].health = None;

                self.services[index].health_at_ms = None;
                self.services[index].health_silent = None;
                self.journal.emit(
                    k,
                    Severity::Info,
                    &name,
                    &corr,
                    "started",
                    vec![
                        ("pid".to_string(), Value::Int(i64::from(pid))),
                        (
                            "program".to_string(),
                            Value::Str(program.path.clone()),
                        ),
                        (
                            "attempt".to_string(),
                            Value::Int(i64::from(self.services[index].attempts + 1)),
                        ),
                    ],
                );
                if name.starts_with(rivet_core::service::SUPPLICANT_SERVICE_PREFIX) {
                    self.follow_supplicant_start(k, &name);
                }
                // Placing the child in its cgroup is what makes the observation
                // above authoritative next time. Failing is survivable and must
                // be said, because it degrades a different thing than it looks
                // like: the service is fine, the supervisor's memory is not.
                if let Err(e) = k.place_in_cgroup(&name, pid) {
                    self.journal.emit(
                        k,
                        Severity::Warn,
                        &name,
                        &corr,
                        "started, but could not be placed in its cgroup; a later supervisor \
                         cannot observe it",
                        vec![("detail".to_string(), Value::Str(e.to_string()))],
                    );
                }
                self.apply_cap(k, index, &name, &corr);
                self.apply_cpu_cap(k, index, &name, &corr);
            }
            Err(e) if e.is_a_program_this_image_cannot_run() => {
                // **Refuse once, loudly, rather than four times slowly.**
                //
                // `ADR-017` §3.2's retry model is about a precondition that is
                // not ready *yet* — a socket nothing is listening on. A program
                // that is not on the filesystem is a different fact: `/system`
                // is read-only `erofs` (`ADR-018` §3), so it cannot appear
                // while the machine runs, and the kernel has already given its
                // verdict. Backing off over it would spend four sleeps and five
                // journal lines to reach the answer the first attempt returned,
                // and would bury the one line that says what the machine has
                // lost under four that do not.
                //
                // This is `State::Failed` by the same route §3.5 reaches it —
                // the supervisor stops trying and the machine keeps working —
                // and `enabled = false`, apply, `enabled = true`, apply is
                // still the way back, exactly as for a give-up.
                self.services[index].state = State::Failed;
                let consequence = self.services[index].def.consequence.clone();
                let program = program.path.clone();
                // **Which sentence is true depends on the origin**, and saying
                // the wrong one is worse than saying nothing. `ADR-018` §3's
                // read-only `/system` is what makes the refusal permanent for a
                // system service; an application's binary is on **writable
                // `/state`**, so that citation is simply false about it — and
                // it is the sentence an operator would act on.
                //
                // Corrected 2026-08-24, when applications began running under a
                // uid of their own (`ADR-015` §5.1) and `EACCES` on an
                // application stopped being a curiosity: the program is now
                // resolved by an unprivileged identity, so **every directory
                // from `/` down must be searchable by it**, and a mode nobody
                // thought about anywhere on that path produces exactly this.
                // The first QEMU boot of the change hit it — a `/` of 0700 —
                // and the old message sent the reader to `/system`.
                //
                // The retry *policy* is deliberately unchanged and is a
                // question for the maintainer: `PLAN.md` §0.11 has it. An
                // application's binary really can appear while the machine
                // runs, so `State::Failed` without a retry is arguably wrong
                // for this origin — but changing it is a change to `ADR-017`
                // §3.5's shape, not a message fix, and this is a message fix.
                let why = match self.services[index].def.origin {
                    Origin::Image => {
                        "REFUSED this image cannot run the program this service names, so it is \
                         marked failed without retrying. ADR-018 §3 makes /system read-only, so \
                         no later attempt can find what this one did not"
                    }
                    Origin::Application => {
                        "REFUSED this machine cannot execute the application's binary, so it is \
                         marked failed without retrying. It is on writable /state, so unlike a \
                         system service this MAY become runnable later and nothing here will \
                         notice. With EACCES the usual cause is not the binary: the program is \
                         resolved by the application's OWN uid (ADR-015 §5.1), so every \
                         directory from / down to it must be searchable by that identity"
                    }
                };
                self.journal.emit(
                    k,
                    Severity::Error,
                    &name,
                    &corr,
                    why,
                    vec![
                        ("program".to_string(), Value::Str(program)),
                        (
                            "uid".to_string(),
                            Value::Int(
                                self.services[index]
                                    .def
                                    .program
                                    .identity
                                    .map_or(0, |i| i64::from(i.uid)),
                            ),
                        ),
                        ("errno".to_string(), Value::Int(i64::from(e.0))),
                        ("detail".to_string(), Value::Str(e.to_string())),
                        // What the machine has lost, not merely which call
                        // failed. `ADR-017` §3.1.6's `consequence_of` mechanism.
                        ("consequence".to_string(), Value::Str(consequence)),
                    ],
                );
                // S-10's fix, same reason as the three refusals above: the
                // device grant already ran and this arm never spawns, so
                // nothing will hold what it was given.
                self.release(k, index);
            }
            Err(e) => {
                self.journal.emit(
                    k,
                    Severity::Error,
                    &name,
                    &corr,
                    "could not be started",
                    vec![("detail".to_string(), Value::Str(e.to_string()))],
                );
                self.services[index].state = State::Idle;
                // A spawn that fails for a reason that could pass — the machine
                // was momentarily out of process slots or memory — must back
                // off and give up exactly as a crash loop does, or it becomes
                // an unthrottled loop. The permanent cases are the arm above.
                self.schedule_retry(k, index, now_ms);
            }
        }
    }

    /// An observation that failed — and **which** of the two failures it was.
    ///
    /// `ADR-017` §3.1.3.1, ruled 2026-08-23 after a Raspberry Pi 4 printed the
    /// old single warning once per service on a boot where nothing was wrong.
    ///
    /// # The two facts `ENOENT` used to hide
    ///
    /// | The machine | The observation | What it means |
    /// | :-- | :-- | :-- |
    /// | cgroup2 is mounted | this service's cgroup is `ENOENT` | it has **not run on this boot**. There is nothing to adopt (§3.1.1) and no hazard. |
    /// | cgroup2 is not mounted | everything under it is `ENOENT` | there is **no authority** for what is running (§3.1.3), and a restarted supervisor will start a second copy of a live service. |
    ///
    /// One errno, two states, opposite consequences. So the errno is not what
    /// decides: [`Kernel::cgroup_hierarchy`] is asked, and it reads the cgroup2
    /// root's own `cgroup.controllers` — a file the kernel creates in every
    /// cgroup2 directory and nowhere else.
    ///
    /// # The benign case records nothing, and that is the ruling
    ///
    /// Three reasons, in the order they decided it:
    ///
    /// 1. **The very next record says it already.** A service whose cgroup is
    ///    absent is one that is about to be spawned, and `started` carries the
    ///    pid. Saying "it was not running" immediately before "it is now
    ///    running as pid N" is one fact written twice, which `ADR-011` is
    ///    about in the small.
    /// 2. **[`Observation::Empty`] is already silent**, and it means the same
    ///    thing to the decision: not running, start it. Recording one and not
    ///    the other would make the console depend on *how* the supervisor
    ///    learned something rather than on what it learned.
    /// 3. **It fires once per service on every boot for ever.** That is how a
    ///    warning stops being read on the boot where it means something — which
    ///    is not a hypothetical here, since it is exactly what the board's
    ///    console looked like.
    ///
    /// The machine-wide fact is not lost by this: `crate::run` states it once
    /// at start-up, before any service is converged, so a boot log still
    /// answers *was the authority available* — and answers it once rather than
    /// once per service.
    ///
    /// # The latch stays per-service, deliberately
    ///
    /// A hierarchy that cannot be read is one machine-wide fact, but its
    /// *consequence* is per-service: this service, named, may end up
    /// double-started. So each service says it once. Making the latch
    /// supervisor-wide would report a hazard for one service and stay silent
    /// about the rest, which is the failure mode the constraint on this change
    /// named explicitly.
    fn report_unobservable(&mut self, k: &mut dyn Kernel, index: usize, corr: &str, e: Errno) {
        // Asked here rather than remembered from start-up: `AGENTS.md` §5 says
        // the kernel is the authority for system state and its verdicts are
        // never cached. It is one read on a pseudo-filesystem, on a path that
        // has already failed.
        let hierarchy = k.cgroup_hierarchy();

        if hierarchy == Hierarchy::Present && e == Errno::ENOENT {
            // A first boot. Nothing to adopt, nothing to warn about, and the
            // latch is deliberately NOT consumed — see the field's own note.
            return;
        }

        if self.services[index].cgroup_warned {
            return;
        }
        self.services[index].cgroup_warned = true;

        let name = self.services[index].def.name.clone();
        let (msg, hierarchy_errno) = match hierarchy {
            Hierarchy::Unreachable(he) => (
                "there is no readable cgroup2 hierarchy, so whether this service is already \
                 running cannot be established; starting it anyway, which starts a second copy \
                 if a previous supervisor left one running (ADR-017 §3.1.3)",
                Some(he),
            ),
            // The hierarchy is fine and this one service's cgroup is not. Rarer
            // and stranger than the case above, and just as blinding for this
            // service, so it is said in its own words rather than folded in.
            Hierarchy::Present => (
                "the cgroup2 hierarchy is readable but this service's own cgroup is not, so \
                 whether it is already running cannot be established; starting it anyway, which \
                 starts a second copy if a previous supervisor left one running",
                None,
            ),
        };

        let mut fields = vec![
            ("errno".to_string(), Value::Int(i64::from(e.0))),
            ("detail".to_string(), Value::Str(e.to_string())),
            (
                "hierarchy".to_string(),
                Value::Str(crate::kernel::CGROUP_CONTROLLERS.to_string()),
            ),
        ];
        if let Some(he) = hierarchy_errno {
            // What the kernel said about the *hierarchy*, kept apart from what
            // it said about the service. They are routinely the same number and
            // never the same fact (`AGENTS.md` §5).
            fields.push((
                "hierarchy_errno".to_string(),
                Value::Int(i64::from(he.0)),
            ));
            fields.push((
                "hierarchy_detail".to_string(),
                Value::Str(he.to_string()),
            ));
        }

        self.journal
            .emit(k, Severity::Warn, &name, corr, msg, fields);
    }

    /// Write the effective memory cap into the service's cgroup, or say why not.
    ///
    /// **The operator's half of the 2026-08-24 ruling, at the moment it becomes
    /// a fact about the machine** (`ADR-007` §7.2).
    /// `crate::document` computed the number — the lower of what the manifest
    /// requested and what the document capped; this writes it.
    ///
    /// # Why after the spawn rather than before it
    ///
    /// A service's cgroup is created by [`Kernel::place_in_cgroup`], which needs
    /// a pid. Capping before there is anything to cap would mean creating the
    /// cgroup twice from two places, and `ADR-017` §3.1.3 makes that directory
    /// the authority for what is running — two creators is two answers about
    /// whether a service exists.
    ///
    /// **The window this leaves is real and is small**: between `execve` and
    /// this write, the service is uncapped. Closing it needs the cgroup to exist
    /// before the fork and the child to be placed in it by `clone3`'s
    /// `CLONE_INTO_CGROUP`, which is a change to how this supervisor spawns
    /// rather than to how it caps. It is named in `PLAN.md` §0.4.3 rather than
    /// left to be discovered by somebody reading a memory graph.
    ///
    /// # A refusal is said out loud, every time
    ///
    /// `ADR-024` §8.1 found the `memory` controller **absent on the board** and
    /// §8.2 found the same kernel offering it under an emulator, so whether this
    /// works is a fact about the running machine that nothing here can assume.
    /// A cap the kernel refused is a cap that is not enforced, and an operator
    /// who wrote one and was not told would believe their machine protected.
    /// That is `AGENTS.md` §3's faked success in its most expensive form, so the
    /// refusal is `Severity::Error` and names the errno.
    fn apply_cap(&mut self, k: &mut dyn Kernel, index: usize, name: &str, corr: &str) {
        let effective = self.services[index].def.memory;
        let Some(limit) = effective.limit else {
            return;
        };
        match k.apply_memory_limit(name, limit.bytes()) {
            Ok(()) => self.journal.emit(
                k,
                Severity::Info,
                name,
                corr,
                "capped",
                vec![
                    ("memory".to_string(), Value::Str(limit.to_string())),
                    (
                        "memory_bytes".to_string(),
                        Value::Int(i64::try_from(limit.bytes()).unwrap_or(i64::MAX)),
                    ),
                    (
                        "memory_source".to_string(),
                        Value::Str(effective.source.as_str().to_string()),
                    ),
                ],
            ),
            Err(e) => self.journal.emit(
                k,
                Severity::Error,
                name,
                corr,
                "NOT ENFORCED the kernel refused this service's memory cap, so it is running \
                 with no limit at all. ADR-024 §8.1 records the cgroup memory controller as \
                 absent on the Raspberry Pi, which is the likeliest cause; a cap reported and \
                 not enforced would be worse than no cap, so it is reported as absent",
                vec![
                    ("memory".to_string(), Value::Str(limit.to_string())),
                    (
                        "memory_source".to_string(),
                        Value::Str(effective.source.as_str().to_string()),
                    ),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            ),
        }
    }

    /// [`Self::apply_cap`]'s footing exactly, for CPU — `ADR-007` §7 and
    /// `ADR-015` §3's 2026-09-09 amendment. See [`Self::apply_cap`]'s own
    /// documentation for why this runs after the spawn and why a refusal is
    /// always said out loud; this function does not repeat the argument.
    ///
    /// # One cause is known, and the rest are not
    /// `ADR-024` §8's board read the `cpu` controller present before the
    /// `memory` fix landed — see `Kernel::apply_cpu_limit`'s own
    /// documentation — so a refusal is reported without naming a likely
    /// cause, rather than repeating a diagnosis this build has no evidence
    /// for. **One exception**: a quota below
    /// [`rivet_core::service::CpuLimit::KERNEL_MIN_QUOTA_US`] is refused by
    /// the kernel for a reason this build knows for certain — no board
    /// reading is needed to say that a quota under one millisecond is
    /// refused at any period — so that one refusal names its cause.
    fn apply_cpu_cap(&mut self, k: &mut dyn Kernel, index: usize, name: &str, corr: &str) {
        let effective = self.services[index].def.cpu;
        let Some(limit) = effective.limit else {
            return;
        };
        match k.apply_cpu_limit(name, limit.quota_us()) {
            Ok(()) => self.journal.emit(
                k,
                Severity::Info,
                name,
                corr,
                "capped",
                vec![
                    ("cpu".to_string(), Value::Str(limit.to_string())),
                    (
                        "cpu_quota_us".to_string(),
                        Value::Int(i64::try_from(limit.quota_us()).unwrap_or(i64::MAX)),
                    ),
                    (
                        "cpu_source".to_string(),
                        Value::Str(effective.source.as_str().to_string()),
                    ),
                ],
            ),
            Err(e) => {
                let quota_us = limit.quota_us();
                let msg = if quota_us < rivet_core::service::CpuLimit::KERNEL_MIN_QUOTA_US {
                    format!(
                        "NOT ENFORCED the requested cpu cap is a quota of {quota_us} \
                         microsecond(s) against the {}-microsecond period, below the kernel's \
                         {}-microsecond minimum quota at any period, so it is running with no \
                         limit at all. A cap reported and not enforced would be worse than no \
                         cap, so it is reported as absent",
                        rivet_core::service::CpuLimit::PERIOD_US,
                        rivet_core::service::CpuLimit::KERNEL_MIN_QUOTA_US,
                    )
                } else {
                    "NOT ENFORCED the kernel refused this service's cpu cap, so it is running \
                     with no limit at all. A cap reported and not enforced would be worse than \
                     no cap, so it is reported as absent"
                        .to_string()
                };
                self.journal.emit(
                    k,
                    Severity::Error,
                    name,
                    corr,
                    &msg,
                    vec![
                        ("cpu".to_string(), Value::Str(limit.to_string())),
                        (
                            "cpu_source".to_string(),
                            Value::Str(effective.source.as_str().to_string()),
                        ),
                        ("detail".to_string(), Value::Str(e.to_string())),
                    ],
                );
            }
        }
    }

    fn stop(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let State::Started { pid, .. } = self.services[index].state else {
            return;
        };
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        self.services[index].state = State::Stopping {
            pid,
            deadline_ms: now_ms.saturating_add(self.limits.stop_grace_ms),
            killed: false,
        };
        match k.signal_process(pid, Sig::Term) {
            Ok(()) => self.journal.emit(
                k,
                Severity::Info,
                &name,
                &corr,
                "stopping",
                vec![("pid".to_string(), Value::Int(i64::from(pid)))],
            ),
            Err(e) => {
                // The process may simply have exited between the reap and here.
                // Either way the deadline still runs, and the reap that follows
                // will settle it.
                self.journal.emit(
                    k,
                    Severity::Warn,
                    &name,
                    &corr,
                    "could not be signalled",
                    vec![
                        ("pid".to_string(), Value::Int(i64::from(pid))),
                        ("detail".to_string(), Value::Str(e.to_string())),
                    ],
                );
            }
        }
        self.release(k, index);
    }

    /// **Everything a service holds while it is running, given back.**
    ///
    /// # Why this is a function
    ///
    /// It was written twice — in [`Self::stop`] and in [`Self::stop_adopted`] —
    /// and **not at all on the third edge**, which is a service that exits by
    /// itself. The audit of 2026-09-03 found that (`PLAN.md` §123, Wave 2.3):
    /// `note_exit` released nothing, so a `oneshot` that finished left its port
    /// listening in this supervisor for the life of the boot, and a display
    /// owner that crashed kept its uid on `/dev/fb0` and the card node while
    /// the splash never came back. `ADR-015` §6.2 point 4 — *"the splash
    /// returns when the owner exits"* — was true only when the supervisor had
    /// done the stopping.
    ///
    /// Three copies of a four-step sequence is how the third one came to be
    /// missing, so now there is one.
    fn release(&mut self, k: &mut dyn Kernel, index: usize) {
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();

        k.close_notify(&name);
        // Beside the notify socket and on its terms: a service that is not
        // running holds no descriptors and leaves no unix socket file behind.
        // `Kernel::open_listeners` re-opens at the next start.
        k.close_listeners(&name);
        // **Any outstanding health question, and the opinion it produced** —
        // `ADR-045` §3. The socket FILE goes with the listeners above,
        // because that is what it is; what goes here is the supervisor's own
        // end of a question in flight, plus the last answer.
        //
        // **Both are cleared, and clearing the answer is the load-bearing
        // half.** A service that stopped and comes back is a new process, and
        // an inherited `healthy` from the run before would be the supervisor
        // reporting a dead process's opinion of a live one. `unknown` until it
        // says otherwise is `ADR-045` §4 exactly.
        k.close_health(&name);
        self.services[index].health = None;

        self.services[index].health_at_ms = None;
        self.services[index].health_silent = None;
        self.services[index].next_health_ms = 0;
        // **The display channel**, which had no caller anywhere in the tree
        // until this function existed. A socket whose far end belonged to a
        // process that is gone is a write that succeeds against nobody.
        k.close_display(&name);
        // And the device nodes go back to root:root — `ADR-015` §6.2 makes the
        // grant per boot, and this makes it per RUN, which is stricter and is
        // what stops a stopped application keeping the screen. See
        // `crate::devices` for why that is more than tidiness.
        let held = self.services[index].def.devices.clone();
        // The SAME role the grant used, derived from the same definition rather
        // than remembered — `crate::devices::revoke` says why a mismatch would
        // leave a node owned by a stopped program.
        let role = self.display_role(&self.services[index].def);
        if !held.is_empty() {
            crate::devices::revoke(k, &mut self.journal, &name, &corr, &held, role);
        }
        // **Point 4: the splash returns.** After the revoke, so the node is
        // root-owned again before the supervisor writes it.
        //
        // **Unless the arbiter is up, in which case the screen is already
        // somebody's** — the audit of 2026-09-03's LOW list, *"splash run by
        // root on every stop edge even while a DRM master holds the screen"*.
        //
        // `cmd/rivet-splash` writes `/dev/fb0`, and `PLAN.md` §104.33.4 is the
        // reading that settles what that does here: **DRM master suspends the
        // framebuffer console**, which is why the launcher takes master instead
        // of writing `fb0` itself. So on a machine with a launcher running,
        // every lessee that stops used to spawn a root process that drew
        // nothing — and then this supervisor journalled *"the splash was drawn
        // again"*, which is a report of a state the machine is not in
        // (`AGENTS.md` §5, `ADR-034` §5.3).
        //
        // The launcher redraws its own menu on the very next frame, so nothing
        // is lost by not running it. What is gained is that the record is true.
        if crate::splash::owns_display(&held) {
            match self.arbiter_holding_the_screen(index) {
                Some(arbiter) => self.journal.emit(
                    k,
                    Severity::Info,
                    &name,
                    &corr,
                    "the display's owner stopped and the splash was NOT restored, because \
                     the arbiter is running and holds DRM master — which suspends the \
                     framebuffer console, so a splash drawn now would not reach the screen \
                     and saying it had would be untrue. The arbiter draws the next frame",
                    vec![("arbiter".to_string(), Value::Str(arbiter))],
                ),
                None => crate::splash::restore(k, &mut self.journal, &name, &corr),
            }
        }
    }

    /// **The display arbiter, if one is alive** — [`Self::display_role`]'s
    /// `Arbiter`, narrowed to a service with a process behind it.
    ///
    /// `except` is the index being released, which must not count as its own
    /// reason: the launcher is itself a service, and a launcher that stopped
    /// would otherwise report that a launcher holds the screen.
    ///
    /// **`Stopping` counts as holding it.** A `SIGTERM` has been sent and the
    /// process has not gone yet; it still has the card open and still has
    /// master, so this is about what the kernel believes rather than about what
    /// the supervisor has asked for.
    fn arbiter_holding_the_screen(&self, except: usize) -> Option<String> {
        self.services.iter().enumerate().find_map(|(i, managed)| {
            (i != except
                // **`drives_the_screen`, not `controls_the_kiosk`** —
                // `ADR-058` §4. Under a compositor the program holding the
                // DRM device is `rivet-displayd`, and a check that still asked
                // for the kiosk connector would report that nothing holds the
                // screen while the compositor was composing on it.
                && Self::drives_the_screen(&managed.def)
                && matches!(
                    managed.state,
                    State::Started { .. } | State::Adopted { .. } | State::Stopping { .. }
                ))
            .then(|| managed.def.name.clone())
        })
    }

    /// The same, plus: this machine stops saying that a program which is not
    /// running and will not be restarted holds the screen.
    ///
    /// **Only on a TERMINAL state** — `Done`, or `Failed` after the retries are
    /// exhausted. A service that is about to be restarted keeps its selection,
    /// because it is coming back and clearing it would take the screen away
    /// from an application that crashed once and would flicker the splash on
    /// every restart.
    fn release_finally(&mut self, k: &mut dyn Kernel, index: usize) {
        self.release(k, index);
        let name = self.services[index].def.name.clone();
        if self.display_selection.as_deref() == Some(name.as_str()) {
            self.display_selection = None;
            self.apply_display_gate();
            self.journal.emit(
                k,
                Severity::Warn,
                &name,
                &self.services[index].corr.clone(),
                "held the screen and is not running; the selection is cleared and the screen is \
                 the launcher's again",
                Vec::new(),
            );
        }
    }

    fn escalate(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let State::Stopping { pid, .. } = self.services[index].state else {
            return;
        };
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        self.services[index].state = State::Stopping {
            pid,
            deadline_ms: now_ms.saturating_add(self.limits.kill_grace_ms),
            killed: true,
        };
        let outcome = k.signal_process(pid, Sig::Kill);
        self.journal.emit(
            k,
            Severity::Warn,
            &name,
            &corr,
            "did not exit within its grace period; sending SIGKILL",
            vec![
                ("pid".to_string(), Value::Int(i64::from(pid))),
                (
                    "grace_ms".to_string(),
                    Value::Int(as_i64(self.limits.stop_grace_ms)),
                ),
                (
                    "signalled".to_string(),
                    Value::Bool(outcome.is_ok()),
                ),
            ],
        );
    }

    fn abandon(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let State::Stopping { pid, .. } = self.services[index].state else {
            return;
        };
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        self.journal.emit(
            k,
            Severity::Error,
            &name,
            &corr,
            "did not exit after SIGKILL; abandoning it. It reparents to PID 1, which reaps it \
             (ADR-017 §3.1.1)",
            vec![("pid".to_string(), Value::Int(i64::from(pid)))],
        );
        // **S-1's fix.** The main pid is given up on, not confirmed dead, so
        // this is exactly the moment a helper it forked — one this
        // supervisor never had a pid for at all — would otherwise be left
        // running unnoticed. `Self::begin_drain` asks the cgroup, which does
        // not depend on `waitpid` the way the line above does.
        self.begin_drain(k, index, now_ms, DrainRest::Idle);
    }

    /// Signal processes that belong to a service but are not our children.
    ///
    /// They cannot be waited for — `waitpid` only answers about children — so
    /// this signals, says exactly that, and comes to rest. The cgroup is the
    /// place to look afterwards, and PID 1 is what reaps them.
    fn stop_adopted(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let State::Adopted { ref pids, .. } = self.services[index].state else {
            return;
        };
        let pids = pids.clone();
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        let mut signalled = 0i64;
        for pid in &pids {
            if k.signal_process(*pid, Sig::Term).is_ok() {
                signalled += 1;
            }
        }
        self.release(k, index);
        self.journal.emit(
            k,
            Severity::Warn,
            &name,
            &corr,
            "signalled processes this supervisor did not spawn; their exit cannot be observed \
             from here",
            vec![
                ("pids".to_string(), Value::Str(join_pids(&pids))),
                ("signalled".to_string(), Value::Int(signalled)),
            ],
        );
        // **S-1's fix — the same `Stopping` deadline and `SIGKILL` escalation
        // a started service's own stop gets**, rather than settling straight
        // to `Idle` on the strength of a `SIGTERM` this supervisor cannot
        // confirm was even delivered to a process it does not own.
        self.begin_drain(k, index, now_ms, DrainRest::Idle);
    }

    /// **After a stop edge, or a `Done`/`Failed` exit, confirm the CGROUP is
    /// empty before this service rests** — S-1's fix and its remainder,
    /// `ADR-017` §3.
    ///
    /// `Self::stop`'s `SIGTERM`, a natural exit, and `Self::stop_adopted`'s
    /// own signal each account for the pid or pids this supervisor already
    /// knew about. A helper one of them forked, and that ignores `SIGTERM`,
    /// is invisible to all three — this is what catches it, on the same
    /// footing `Self::start`'s own adoption check already asks the cgroup on.
    ///
    /// The common case — nothing left — costs one [`Kernel::observe`] and no
    /// delay: this service settles straight to `then`'s rest state
    /// ([`DrainRest::state`]). Only a cgroup that still answers with members
    /// is signalled and enters [`State::Draining`], carrying `then` forward
    /// so the wait does not lose which rest state it is waiting to reach.
    ///
    /// **Everything this service held was already given back** by whichever
    /// caller reached this — `Self::release` or `Self::release_finally`, on
    /// the exit edge, run before this is ever called. What is left here is
    /// only the PROCESS.
    ///
    /// **Blind mode is unchanged.** [`Observation::Unavailable`] — no cgroup
    /// hierarchy to ask — settles the same way [`Observation::Empty`] does,
    /// exactly as every other cgroup read in this module already treats it
    /// (`ADR-017` §3.1.3.1): there is no authority here to say otherwise, and
    /// this is not the latch `Self::start`'s own check owns.
    fn begin_drain(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64, then: DrainRest) {
        let name = self.services[index].def.name.clone();
        match k.observe(&name) {
            Observation::Empty | Observation::Unavailable(_) => {
                self.services[index].state = then.state();
                self.settle_scratch(k, index);
            }
            Observation::Members(pids) => {
                let corr = self.services[index].corr.clone();
                for pid in &pids {
                    let _ = k.signal_process(*pid, Sig::Term);
                }
                self.journal.emit(
                    k,
                    Severity::Warn,
                    &name,
                    &corr,
                    "the cgroup still held a process after this service's own pid was accounted \
                     for, so it was signalled directly rather than left running under this \
                     service's granted devices and sockets (S-1's fix, ADR-017 §3)",
                    vec![("pids".to_string(), Value::Str(join_pids(&pids)))],
                );
                self.services[index].state = State::Draining {
                    deadline_ms: now_ms.saturating_add(self.limits.stop_grace_ms),
                    killed: false,
                    next_poll_ms: now_ms.saturating_add(self.limits.adopt_poll_ms),
                    then,
                };
            }
        }
    }

    /// Re-ask the cgroup whether a [`State::Draining`] service has emptied
    /// yet.
    fn poll_drain(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let State::Draining {
            deadline_ms,
            killed,
            then,
            ..
        } = self.services[index].state
        else {
            return;
        };
        let name = self.services[index].def.name.clone();
        match k.observe(&name) {
            Observation::Empty | Observation::Unavailable(_) => {
                let corr = self.services[index].corr.clone();
                self.services[index].state = then.state();
                self.journal.emit(
                    k,
                    Severity::Info,
                    &name,
                    &corr,
                    "the cgroup emptied; stopped",
                    vec![],
                );
                self.settle_scratch(k, index);
            }
            Observation::Members(_) => {
                self.services[index].state = State::Draining {
                    deadline_ms,
                    killed,
                    next_poll_ms: now_ms.saturating_add(self.limits.adopt_poll_ms),
                    then,
                };
            }
        }
    }

    /// A [`State::Draining`] service's grace period expired without the
    /// cgroup emptying: kill it harder.
    ///
    /// [`Kernel::cgroup_kill`] is preferred — one atomic `SIGKILL` of
    /// everything the cgroup holds, including anything still forking. Where
    /// it is unavailable (a kernel older than 5.14), this falls back to
    /// signalling whatever [`Kernel::observe`] lists right now, by hand.
    fn escalate_drain(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let State::Draining { then, .. } = self.services[index].state else {
            return;
        };
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        let hard = k.cgroup_kill(&name).is_ok();
        if !hard && let Observation::Members(pids) = k.observe(&name) {
            for pid in &pids {
                let _ = k.signal_process(*pid, Sig::Kill);
            }
        }
        self.services[index].state = State::Draining {
            deadline_ms: now_ms.saturating_add(self.limits.kill_grace_ms),
            killed: true,
            next_poll_ms: now_ms.saturating_add(self.limits.adopt_poll_ms),
            then,
        };
        self.journal.emit(
            k,
            Severity::Warn,
            &name,
            &corr,
            "did not empty within its grace period; killing the cgroup",
            vec![("cgroup_kill".to_string(), Value::Bool(hard))],
        );
    }

    /// A [`State::Draining`] service's cgroup would not empty even after the
    /// harder kill. Stop waiting and say so — `Self::abandon`'s own shape,
    /// for a cgroup rather than a single pid. Safe for the same reason: what
    /// is still there reparents to PID 1, which reaps it eventually; it is
    /// simply no longer this supervisor's to wait on.
    fn abandon_drain(&mut self, k: &mut dyn Kernel, index: usize) {
        let State::Draining { then, .. } = self.services[index].state else {
            return;
        };
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        self.services[index].state = then.state();
        self.journal.emit(
            k,
            Severity::Error,
            &name,
            &corr,
            "the cgroup did not empty even after being killed; giving up and treating the \
             service as stopped",
            vec![],
        );
        self.settle_scratch(k, index);
    }

    /// **The cgroup-drain remainder of `Self::empty_scratch`'s "exactly two
    /// moments" — S-1's remainder.** Called at every point a drain settles
    /// this service into its rest state, so a helper the main pid's exit
    /// never accounted for cannot still be using the scratch directory when
    /// it is emptied.
    ///
    /// `Self::empty_scratch`'s own doc used to claim the supervisor "knows
    /// for certain that nothing is using this directory" immediately after
    /// reaping the main pid — the same false premise S-1 found in
    /// `Self::release`'s devices and sockets. Reaping the main pid is not
    /// that guarantee; the cgroup answering empty is. This is the moment
    /// that is actually true, whichever of `Self::begin_drain`,
    /// `Self::poll_drain` or `Self::abandon_drain` reached it.
    fn settle_scratch(&mut self, k: &mut dyn Kernel, index: usize) {
        self.empty_scratch(k, index, "exit");
    }

    /// Re-ask the cgroup about an adopted service (`ADR-017` §3.1.3).
    fn poll_adoption(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        match k.observe(&name) {
            Observation::Members(pids) => {
                self.services[index].state = State::Adopted {
                    pids,
                    next_poll_ms: now_ms.saturating_add(self.limits.adopt_poll_ms),
                };
            }
            Observation::Empty => {
                // It went away while we were not its parent, so there is no
                // exit status to read and none is needed: convergence only asks
                // whether it is running. ADR-017 §3.1.2.
                self.services[index].state = State::Idle;
                self.journal.emit(
                    k,
                    Severity::Info,
                    &name,
                    &corr,
                    "adopted processes are gone; converging again",
                    vec![],
                );
            }
            Observation::Unavailable(e) => {
                self.services[index].state = State::Idle;
                self.journal.emit(
                    k,
                    Severity::Warn,
                    &name,
                    &corr,
                    "the cgroup of an adopted service became unreadable; treating it as not \
                     running",
                    vec![("detail".to_string(), Value::Str(e.to_string()))],
                );
            }
        }
    }

    fn report_not_ready(&mut self, k: &mut dyn Kernel, index: usize, now_ms: u64) {
        let name = self.services[index].def.name.clone();
        let corr = self.services[index].corr.clone();
        let since = match self.services[index].state {
            State::Started { since_ms, .. } => since_ms,
            _ => now_ms,
        };
        if let State::Started {
            ready_reported: ref mut reported,
            ..
        } = self.services[index].state
        {
            *reported = true;
        }
        self.journal.emit(
            k,
            Severity::Warn,
            &name,
            &corr,
            "running, but has not declared readiness; it is reported as running and not as ready",
            vec![(
                "after_ms".to_string(),
                Value::Int(as_i64(now_ms.saturating_sub(since))),
            )],
        );
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.services.iter().position(|m| m.def.name == name)
    }
}

fn join_pids(pids: &[i32]) -> String {
    pids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Journal fields are typed and `Value::Int` is an `i64`. A duration in
/// milliseconds cannot exceed it in any life this machine will have, but the
/// conversion is still done rather than assumed.
fn as_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test. See the identical note in
    // `crate::kernel`'s test module for why this arrived a day late: the
    // capture tests below are the first here to `panic!`, this crate denies
    // `panic` rather than warning on it, and the check that was supposed to
    // catch that grepped for `^warning` and could not see an error at all.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::Journal;
    use crate::kernel::{Errno, Program};
    use crate::service::RestartPolicy;
    use rivet_core::package::application::{Device, Socket};
    use crate::testkernel::{FakeKernel, Script};

    fn def(name: &str) -> ServiceDef {
        ServiceDef {
            name: name.to_string(),
            program: Program {
                path: "/bin/svc".to_string(),
                args: Vec::new(),
                env: Vec::new(),
                identity: None,
                confinement: None,
                groups: Vec::new(),
                capabilities: Vec::new(),
            },
            enabled: true,
            restart: RestartPolicy::Always,
            readiness: Readiness::OnStart,
            consequence: "the fixture would lose whatever this stands for".to_string(),
            ..crate::service::IMAGE_DEFINED
        }
    }

    /// `def`, as an APPLICATION. An application definition needs an identity of
    /// its own — `Supervisor::new` refuses one without, because it would run as
    /// root out of writable `/state` (`ADR-015` §5.1) — so the two cannot be one
    /// fixture with a field flipped.
    fn application_def(name: &str) -> ServiceDef {
        let service_name = rivet_core::service::ServiceName::new(name)
            .unwrap_or_else(|| unreachable!("{name:?} is not a service name"));
        let mut d = def(name);
        d.origin = Origin::Application;
        d.program.identity = Some(rivet_core::service::app_identity(&service_name));
        // **And a confinement, because a real one always has one** —
        // `ADR-042`. `crate::applications::definition` puts a policy on every
        // application it discovers, so a fixture without one would exercise a
        // start path no machine takes and would report every test about
        // applications as passing on a build that had stopped confining them.
        d.program.confinement = Some(rivet_core::confinement::policy(
            rivet_core::service::BUILT_IN_VOLUME,
            &service_name,
            &rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            ),
            None,
            &[],
        ));
        d
    }

    fn journal() -> Journal {
        Journal::new("test-boot".to_string(), 1, ACTOR.to_string())
    }

    fn supervisor(defs: Vec<ServiceDef>) -> Supervisor {
        match Supervisor::new(defs, Limits::default(), journal()) {
            Ok(s) => s,
            Err(e) => unreachable!("fixture refused: {e}"),
        }
    }

    /// A supervisor with `name` holding the screen.
    ///
    /// **A display application does not run merely by being enabled** since
    /// `PLAN.md` §104.40: `enabled` offers it a screen and a SELECTION gives it
    /// one. Every test about a display application therefore has to say which
    /// one is showing, and saying so here is the point — a fixture that started
    /// one anyway would be testing a rule the machine no longer has.
    fn showing(defs: Vec<ServiceDef>, name: &str) -> Supervisor {
        let mut s = supervisor(defs);
        if let Err(why) = s.select_display(name) {
            unreachable!("fixture could not show {name}: {why}");
        }
        s
    }

    /// The fake reports an empty cgroup by default so that `start` reaches the
    /// spawn; `Unavailable` is the *real* machine's answer today and has its
    /// own tests.
    fn kernel() -> FakeKernel {
        let mut k = FakeKernel::new();
        k.observations = Script::always(Observation::Empty);
        k
    }

    // ----------------------------------------------------------------------
    // decide(): the pure half. Every branch, no I/O.
    // ----------------------------------------------------------------------

    #[test]
    fn an_enabled_idle_service_is_started() {
        let l = Limits::default();
        assert_eq!(
            decide(Phase::Converging, &def("a"), &State::Idle, Asked::Nothing, &l, 0),
            Action::Start
        );
    }

    #[test]
    fn a_disabled_idle_service_is_left_alone() {
        let l = Limits::default();
        let mut d = def("a");
        d.enabled = false;
        assert_eq!(
            decide(Phase::Converging, &d, &State::Idle, Asked::Nothing, &l, 0),
            Action::Nothing
        );
    }

    /// `ADR-017` §6 — `enabled = false` is the only way a service comes down,
    /// and it must actually bring it down.
    #[test]
    fn disabling_a_running_service_stops_it() {
        let l = Limits::default();
        let mut d = def("a");
        d.enabled = false;
        let running = State::Started {
            pid: 7,
            since_ms: 0,
            ready: true,
            ready_reported: false,
        };
        assert_eq!(decide(Phase::Converging, &d, &running, Asked::Nothing, &l, 0), Action::Stop);
    }

    #[test]
    fn a_backoff_that_has_not_expired_does_nothing() {
        let l = Limits::default();
        let waiting = State::Backoff { until_ms: 5_000 };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &waiting, Asked::Nothing, &l, 4_999),
            Action::Nothing
        );
    }

    #[test]
    fn a_backoff_that_has_expired_starts_the_service() {
        let l = Limits::default();
        let waiting = State::Backoff { until_ms: 5_000 };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &waiting, Asked::Nothing, &l, 5_000),
            Action::Start
        );
    }

    /// The single most important "does not spin" property: a service the
    /// supervisor has given up on produces no action, for ever.
    #[test]
    fn a_failed_service_is_left_alone_rather_than_retried() {
        let l = Limits::default();
        for now in [0, 1_000, 86_400_000, u64::MAX] {
            assert_eq!(
                decide(Phase::Converging, &def("a"), &State::Failed, Asked::Nothing, &l, now),
                Action::Nothing,
                "at {now}"
            );
        }
    }

    /// Without this there is no way back from a give-up short of rebooting,
    /// because `ADR-017` §6 has no manual start either.
    #[test]
    fn disabling_a_failed_service_clears_it() {
        let l = Limits::default();
        let mut d = def("a");
        d.enabled = false;
        assert_eq!(
            decide(Phase::Converging, &d, &State::Failed, Asked::Nothing, &l, 0),
            Action::Cancel
        );
    }

    #[test]
    fn a_notify_service_that_is_not_ready_yet_is_left_alone_until_the_deadline() {
        let l = Limits::default();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let started = State::Started {
            pid: 1,
            since_ms: 0,
            ready: false,
            ready_reported: false,
        };
        assert_eq!(
            decide(Phase::Converging, &d, &started, Asked::Nothing, &l, l.ready_timeout_ms - 1),
            Action::Nothing
        );
        assert_eq!(
            decide(Phase::Converging, &d, &started, Asked::Nothing, &l, l.ready_timeout_ms),
            Action::ReportNotReady
        );
    }

    /// A slow service is not a failed service. `ADR-017` §3.2's model is retry
    /// until preconditions hold, so the deadline produces a *record*, never a
    /// kill.
    #[test]
    fn an_overdue_readiness_is_reported_once_and_never_kills_anything() {
        let l = Limits::default();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let reported = State::Started {
            pid: 1,
            since_ms: 0,
            ready: false,
            ready_reported: true,
        };
        assert_eq!(
            decide(Phase::Converging, &d, &reported, Asked::Nothing, &l, u64::MAX / 2),
            Action::Nothing
        );
    }

    #[test]
    fn a_service_that_does_not_use_notify_has_no_readiness_deadline() {
        let l = Limits::default();
        let started = State::Started {
            pid: 1,
            since_ms: 0,
            ready: false,
            ready_reported: false,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &started, Asked::Nothing, &l, u64::MAX / 2),
            Action::Nothing
        );
    }

    #[test]
    fn a_ready_service_needs_nothing_further() {
        let l = Limits::default();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let ready = State::Started {
            pid: 1,
            since_ms: 0,
            ready: true,
            ready_reported: false,
        };
        assert_eq!(
            decide(Phase::Converging, &d, &ready, Asked::Nothing, &l, u64::MAX / 2),
            Action::Nothing
        );
    }

    #[test]
    fn a_stop_escalates_to_kill_only_after_the_grace_period() {
        let l = Limits::default();
        let stopping = State::Stopping {
            pid: 3,
            deadline_ms: 1_000,
            killed: false,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &stopping, Asked::Nothing, &l, 999),
            Action::Nothing
        );
        assert_eq!(
            decide(Phase::Converging, &def("a"), &stopping, Asked::Nothing, &l, 1_000),
            Action::Kill
        );
    }

    #[test]
    fn a_kill_that_did_not_work_is_abandoned_rather_than_waited_on_for_ever() {
        let l = Limits::default();
        let killed = State::Stopping {
            pid: 3,
            deadline_ms: 1_000,
            killed: true,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &killed, Asked::Nothing, &l, 1_000),
            Action::Abandon
        );
    }

    #[test]
    fn shutdown_stops_a_running_service_however_it_is_configured() {
        let l = Limits::default();
        let running = State::Started {
            pid: 4,
            since_ms: 0,
            ready: true,
            ready_reported: false,
        };
        assert_eq!(
            decide(Phase::ShuttingDown, &def("a"), &running, Asked::Nothing, &l, 0),
            Action::Stop
        );
    }

    /// A pending restart during shutdown must be dropped, or the supervisor
    /// starts a service on the way down and never finishes.
    #[test]
    fn shutdown_cancels_a_pending_backoff_rather_than_starting_the_service() {
        let l = Limits::default();
        let waiting = State::Backoff { until_ms: 0 };
        assert_eq!(
            decide(Phase::ShuttingDown, &def("a"), &waiting, Asked::Nothing, &l, 10_000),
            Action::Cancel
        );
    }

    #[test]
    fn shutdown_leaves_an_idle_service_alone_so_the_pass_can_finish() {
        let l = Limits::default();
        assert_eq!(
            decide(Phase::ShuttingDown, &def("a"), &State::Idle, Asked::Nothing, &l, 0),
            Action::Nothing
        );
    }

    #[test]
    fn shutdown_signals_adopted_processes_too() {
        let l = Limits::default();
        let adopted = State::Adopted {
            pids: vec![11],
            next_poll_ms: u64::MAX,
        };
        assert_eq!(
            decide(Phase::ShuttingDown, &def("a"), &adopted, Asked::Nothing, &l, 0),
            Action::StopAdopted
        );
    }

    #[test]
    fn an_adopted_service_is_re_observed_when_its_poll_falls_due() {
        let l = Limits::default();
        let adopted = State::Adopted {
            pids: vec![11],
            next_poll_ms: 1_000,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &adopted, Asked::Nothing, &l, 999),
            Action::Nothing
        );
        assert_eq!(
            decide(Phase::Converging, &def("a"), &adopted, Asked::Nothing, &l, 1_000),
            Action::PollAdoption
        );
    }

    // ----------------------------------------------------------------------
    // Construction
    // ----------------------------------------------------------------------

    #[test]
    fn an_invalid_definition_is_refused_rather_than_skipped() {
        let mut bad = def("a");
        bad.program.path = "relative".to_string();
        let err = Supervisor::new(vec![bad], Limits::default(), journal());
        assert!(matches!(err, Err(DefError::ProgramNotAbsolute { .. })));
    }

    /// Two definitions with one name would collide in the cgroup path and the
    /// notify socket path, and the second would silently win.
    #[test]
    fn a_duplicate_service_name_is_refused() {
        let err = Supervisor::new(vec![def("a"), def("a")], Limits::default(), journal());
        assert!(matches!(err, Err(DefError::Duplicate { .. })));
    }

    #[test]
    fn every_service_starts_idle_with_no_failures_recorded() {
        let s = supervisor(vec![def("a"), def("b")]);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert_eq!(s.state_of("b"), Some(&State::Idle));
        assert!(s.services().iter().all(|m| m.attempts == 0));
        assert_eq!(s.phase(), Phase::Converging);
        assert!(!s.is_finished());
    }

    // ----------------------------------------------------------------------
    // Requirement 1: nothing to do is not a reason to exit
    // ----------------------------------------------------------------------

    /// PID 1 restarts this process with backoff, so a supervisor that exits
    /// because it has nothing to supervise is an infinite loop dressed as a
    /// design.
    #[test]
    fn a_supervisor_with_no_services_does_not_finish() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        for _ in 0..10 {
            s.reap(&mut k);
            s.converge(&mut k);
        }
        assert!(!s.is_finished());
        assert_eq!(s.phase(), Phase::Converging);
    }

    /// And it must not *spin* either. No deadlines means the wait blocks on the
    /// signal descriptor rather than returning immediately.
    #[test]
    fn a_supervisor_with_nothing_pending_waits_indefinitely_rather_than_spinning() {
        let s = supervisor(vec![]);
        assert_eq!(s.next_timeout_ms(0), None);
    }

    #[test]
    fn a_supervisor_whose_services_have_all_failed_still_waits_indefinitely() {
        let mut s = supervisor(vec![def("a")]);
        s.services[0].state = State::Failed;
        assert_eq!(s.next_timeout_ms(1_000), None);
    }

    #[test]
    fn a_pending_backoff_sets_a_deadline_and_it_shrinks_as_time_passes() {
        let mut s = supervisor(vec![def("a")]);
        s.services[0].state = State::Backoff { until_ms: 5_000 };
        assert_eq!(s.next_timeout_ms(0), Some(5_000));
        assert_eq!(s.next_timeout_ms(4_000), Some(1_000));
        assert_eq!(s.next_timeout_ms(9_000), Some(0));
    }

    #[test]
    fn the_soonest_of_several_deadlines_is_the_one_that_is_waited_for() {
        let mut s = supervisor(vec![def("a"), def("b")]);
        s.services[0].state = State::Backoff { until_ms: 5_000 };
        s.services[1].state = State::Stopping {
            pid: 2,
            deadline_ms: 1_200,
            killed: false,
        };
        assert_eq!(s.next_timeout_ms(200), Some(1_000));
    }

    #[test]
    fn a_disabled_service_in_backoff_contributes_no_deadline() {
        let mut s = supervisor(vec![def("a")]);
        s.services[0].def.enabled = false;
        s.services[0].state = State::Backoff { until_ms: 5_000 };
        assert_eq!(s.next_timeout_ms(0), None);
    }

    // ----------------------------------------------------------------------
    // Requirement 2: start, notice the exit, back off, give up
    // ----------------------------------------------------------------------

    #[test]
    fn converging_starts_an_enabled_service_and_records_it() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);

        assert_eq!(k.spawned.len(), 1);
        assert_eq!(k.spawned[0].0, "a");
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));
        assert!(
            k.records.iter().any(|r| r.msg == "started" && r.svc == "a"),
            "{:?}",
            k.records
        );
    }

    #[test]
    fn a_started_service_is_placed_in_its_cgroup_so_a_later_supervisor_can_see_it() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert_eq!(k.placed, vec![("a".to_string(), 1_000)]);
    }

    // -- the memory cap, ruled by the maintainer 2026-08-24 ----------

    /// A definition carrying an effective cap, as `crate::document` leaves one.
    fn capped(name: &str, text: &str) -> ServiceDef {
        let limit = rivet_core::service::MemoryLimit::parse(text).expect("a valid fixture");
        ServiceDef {
            memory: rivet_core::service::EffectiveMemory {
                limit: Some(limit),
                source: rivet_core::service::MemorySource::Document,
            },
            ..def(name)
        }
    }

    /// **The operator's half of the ruling reaching the kernel.** Without this
    /// the cap is a number in a journal record.
    #[test]
    fn a_capped_service_has_its_limit_written_to_its_cgroup_at_the_start() {
        let mut k = kernel();
        let mut s = supervisor(vec![capped("a", "64M")]);
        s.converge(&mut k);
        assert_eq!(k.capped, vec![("a".to_string(), 64 * 1024 * 1024)]);
        assert!(
            k.records.iter().any(|r| r.msg == "capped" && r.svc == "a"),
            "{:?}",
            k.records
        );
    }

    /// A service nobody capped must not be capped at anything — least of all
    /// zero, which would be a service that may allocate nothing.
    #[test]
    fn a_service_with_no_cap_has_nothing_written_at_all() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(k.capped.is_empty(), "{:?}", k.capped);
        assert!(!k.records.iter().any(|r| r.msg == "capped"));
    }

    /// **`ADR-024` §8.1's board.** The controller is absent there and present
    /// under an emulator, so a refusal is a real outcome and not an edge case —
    /// and a cap reported and not enforced is worse than no cap.
    #[test]
    fn a_cap_the_kernel_refuses_is_reported_as_not_enforced_rather_than_assumed() {
        let mut k = kernel();
        k.memory_results = Script::always(Err(Errno::ENODEV));
        let mut s = supervisor(vec![capped("a", "64M")]);
        s.converge(&mut k);

        assert!(k.capped.is_empty(), "a refused cap was recorded as applied");
        let said = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("NOT ENFORCED") && r.svc == "a")
            .unwrap_or_else(|| panic!("a refused cap went unreported: {:?}", k.records));
        assert_eq!(said.sev, Severity::Error);
        // And the service is running regardless: an uncapped service is a
        // service, and refusing to start it would be a broken machine rather
        // than a broken limit.
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));
    }

    /// The cap is re-applied on a restart, because the cgroup a service is
    /// capped in is created by its start. A supervisor that capped only the
    /// first start would leave every restarted service unlimited.
    #[test]
    fn a_restarted_service_is_capped_again() {
        let mut k = kernel();
        let mut s = supervisor(vec![capped("a", "64M")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the service should be running")
        };
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        k.mono_ms += 60_000;
        s.converge(&mut k);
        assert_eq!(k.capped.len(), 2, "{:?}", k.capped);
        assert!(k.capped.iter().all(|(name, bytes)| name == "a" && *bytes == 64 * 1024 * 1024));
    }

    // -- the cpu cap, on the memory cap's footing exactly (ADR-007 §7 and
    // ADR-015 §3's 2026-09-09 amendment) -----------------------------------

    /// [`capped`]'s footing, for CPU.
    fn capped_cpu(name: &str, text: &str) -> ServiceDef {
        let limit = rivet_core::service::CpuLimit::parse(text).expect("a valid fixture");
        ServiceDef {
            cpu: rivet_core::service::EffectiveCpu {
                limit: Some(limit),
                source: rivet_core::service::CpuSource::Document,
            },
            ..def(name)
        }
    }

    /// [`a_capped_service_has_its_limit_written_to_its_cgroup_at_the_start`]'s
    /// footing, for CPU. `0.5` cores at
    /// `rivet_core::service::CpuLimit::PERIOD_US` (100000 µs) is a quota of
    /// 50000 µs — the worked arithmetic from `CpuLimit`'s own documentation.
    #[test]
    fn a_cpu_capped_service_has_its_quota_written_to_its_cgroup_at_the_start() {
        let mut k = kernel();
        let mut s = supervisor(vec![capped_cpu("a", "0.5")]);
        s.converge(&mut k);
        assert_eq!(k.capped_cpu, vec![("a".to_string(), 50_000)]);
        assert!(
            k.records.iter().any(|r| r.msg == "capped" && r.svc == "a"),
            "{:?}",
            k.records
        );
    }

    /// [`a_service_with_no_cap_has_nothing_written_at_all`]'s footing, for
    /// CPU — and the half of this change that matters most: an application
    /// with no `cpu` declared must be unaffected, never falling back to some
    /// invented default.
    #[test]
    fn a_service_with_no_cpu_cap_has_nothing_written_at_all() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(k.capped_cpu.is_empty(), "{:?}", k.capped_cpu);
        // And the ordinary memory-cap assertion still holds beside it: adding
        // the cpu cap must not have disturbed the sibling it mirrors.
        assert!(k.capped.is_empty(), "{:?}", k.capped);
    }

    /// [`a_cap_the_kernel_refuses_is_reported_as_not_enforced_rather_than_
    /// assumed`]'s footing, for CPU.
    #[test]
    fn a_cpu_cap_the_kernel_refuses_is_reported_as_not_enforced_rather_than_assumed() {
        let mut k = kernel();
        k.cpu_results = Script::always(Err(Errno::ENODEV));
        let mut s = supervisor(vec![capped_cpu("a", "0.5")]);
        s.converge(&mut k);

        assert!(
            k.capped_cpu.is_empty(),
            "a refused cap was recorded as applied"
        );
        let said = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("NOT ENFORCED") && r.svc == "a" && r.msg.contains("cpu"))
            .unwrap_or_else(|| panic!("a refused cpu cap went unreported: {:?}", k.records));
        assert_eq!(said.sev, Severity::Error);
        // And the service is running regardless — refusing to start it would
        // be a broken machine rather than a broken limit.
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));
    }

    /// **A quota below the kernel's own minimum is refused for a reason this
    /// build knows for certain**, and the record says so — unlike the
    /// unqualified refusal above, whose cause is not known.
    #[test]
    fn a_refused_cpu_quota_below_the_kernels_minimum_names_that_minimum() {
        let mut k = kernel();
        k.cpu_results = Script::always(Err(Errno::ENODEV));
        // `0.001` cores is a quota of 100 microseconds at the fixed
        // 100000-microsecond period — below the 1000-microsecond minimum.
        let mut s = supervisor(vec![capped_cpu("a", "0.001")]);
        s.converge(&mut k);

        assert!(
            k.capped_cpu.is_empty(),
            "a refused cap was recorded as applied"
        );
        let said = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("NOT ENFORCED") && r.svc == "a")
            .unwrap_or_else(|| panic!("a refused cpu cap went unreported: {:?}", k.records));
        assert_eq!(said.sev, Severity::Error);
        let quota_us = rivet_core::service::CpuLimit::parse("0.001")
            .expect("a valid fixture")
            .quota_us();
        assert!(
            said.msg.contains(&quota_us.to_string()),
            "the refusal does not name the requested quota, {quota_us}: {}",
            said.msg
        );
        assert!(
            said.msg.contains(
                &rivet_core::service::CpuLimit::KERNEL_MIN_QUOTA_US.to_string()
            ),
            "the refusal does not name the kernel's minimum: {}",
            said.msg
        );
    }

    /// **A refused quota the kernel COULD have kept is reported exactly as it
    /// always was** — pinned character for character, so a future change to
    /// the minimum-quota wording above cannot silently widen to cover this
    /// case too.
    #[test]
    fn a_refused_cpu_quota_at_or_above_the_minimum_keeps_todays_wording() {
        let mut k = kernel();
        k.cpu_results = Script::always(Err(Errno::ENODEV));
        let mut s = supervisor(vec![capped_cpu("a", "0.5")]);
        s.converge(&mut k);

        let said = k
            .records
            .iter()
            .find(|r| r.svc == "a" && r.sev == Severity::Error)
            .unwrap_or_else(|| panic!("a refused cpu cap went unreported: {:?}", k.records));
        assert_eq!(
            said.msg,
            "NOT ENFORCED the kernel refused this service's cpu cap, so it is running with no \
             limit at all. A cap reported and not enforced would be worse than no cap, so it is \
             reported as absent"
        );
    }

    /// [`a_restarted_service_is_capped_again`]'s footing, for CPU: the cgroup
    /// a service is capped in is created by its start, so a supervisor that
    /// capped only the first start would leave every restarted service
    /// unlimited.
    #[test]
    fn a_restarted_service_is_cpu_capped_again() {
        let mut k = kernel();
        let mut s = supervisor(vec![capped_cpu("a", "0.5")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the service should be running")
        };
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        k.mono_ms += 60_000;
        s.converge(&mut k);
        assert_eq!(k.capped_cpu.len(), 2, "{:?}", k.capped_cpu);
        assert!(
            k.capped_cpu
                .iter()
                .all(|(name, quota_us)| name == "a" && *quota_us == 50_000)
        );
    }

    #[test]
    fn converging_twice_does_not_start_a_second_copy() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.converge(&mut k);
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1);
    }

    #[test]
    fn an_exit_is_noticed_and_the_service_comes_back_after_a_backoff() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the service should be running");
        };

        k.exits.push((pid, ExitStatus::Exited(1)));
        k.mono_ms = 100;
        s.reap(&mut k);

        assert_eq!(
            s.state_of("a"),
            Some(&State::Backoff { until_ms: 100 + 250 })
        );
        assert!(k.records.iter().any(|r| r.msg == "exited"));
        assert!(k.records.iter().any(|r| r.msg == "will retry after backoff"));
    }

    #[test]
    fn the_service_is_not_restarted_before_its_backoff_expires() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);

        k.mono_ms = 249;
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1, "restarted early");

        k.mono_ms = 250;
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 2);
    }

    #[test]
    fn the_backoff_doubles_across_consecutive_failures() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        let mut delays = Vec::new();
        for _ in 0..4 {
            s.converge(&mut k);
            let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
                break;
            };
            k.exits.push((pid, ExitStatus::Exited(1)));
            s.reap(&mut k);
            if let Some(State::Backoff { until_ms }) = s.state_of("a") {
                delays.push(until_ms - k.mono_ms);
                k.mono_ms = *until_ms;
            }
        }
        assert_eq!(delays, vec![250, 500, 1_000, 2_000]);
    }

    /// `ADR-017` §3.5: after the threshold the supervisor stops trying, marks
    /// the service failed and reports it. On a Pi the alternative writes to the
    /// SD card until the card dies.
    #[test]
    fn a_hopeless_service_is_given_up_on_rather_than_restarted_for_ever() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        for _ in 0..20 {
            s.converge(&mut k);
            if let Some(State::Started { pid, .. }) = s.state_of("a").cloned() {
                k.exits.push((pid, ExitStatus::Exited(1)));
                s.reap(&mut k);
            }
            if let Some(State::Backoff { until_ms }) = s.state_of("a") {
                k.mono_ms = *until_ms;
            }
        }
        assert_eq!(s.state_of("a"), Some(&State::Failed));
        // give_up_after = 5 means five starts, then no more.
        assert_eq!(k.spawned.len(), 5);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("giving up")),
            "the give-up must be reported, not silent"
        );
    }

    /// Without a success window the failure count is cumulative over the
    /// machine's whole uptime, so a service that crashes once a month is
    /// eventually marked permanently failed — a supervisor giving up on a
    /// working service.
    #[test]
    fn staying_up_past_the_success_window_forgives_earlier_failures() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        // Fail four times: one more would give up.
        for _ in 0..4 {
            s.converge(&mut k);
            let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
                unreachable!()
            };
            k.exits.push((pid, ExitStatus::Exited(1)));
            s.reap(&mut k);
            if let Some(State::Backoff { until_ms }) = s.state_of("a") {
                k.mono_ms = *until_ms;
            }
        }
        assert_eq!(s.services()[0].attempts, 4);

        // Now it runs for longer than the success window before dying.
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.mono_ms += Limits::default().success_window_ms;
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);

        assert_eq!(s.services()[0].attempts, 1, "the slate was not wiped");
        assert!(matches!(s.state_of("a"), Some(State::Backoff { .. })));
    }

    #[test]
    fn a_short_life_does_not_forgive_anything() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.mono_ms += Limits::default().success_window_ms - 1;
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        assert_eq!(s.services()[0].attempts, 1);

        let Some(State::Backoff { until_ms }) = s.state_of("a") else {
            unreachable!("a failure must schedule a retry")
        };
        k.mono_ms = *until_ms;
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        assert_eq!(s.services()[0].attempts, 2, "the count was reset");
    }

    #[test]
    fn a_clean_exit_under_on_failure_is_left_alone() {
        let mut k = kernel();
        let mut d = def("a");
        d.restart = RestartPolicy::OnFailure;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert_eq!(s.state_of("a"), Some(&State::Done));
        s.converge(&mut k);
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1, "a clean oneshot was restarted");
    }

    #[test]
    fn a_crash_under_on_failure_is_restarted() {
        let mut k = kernel();
        let mut d = def("a");
        d.restart = RestartPolicy::OnFailure;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Signalled(11)));
        s.reap(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Backoff { .. })));
    }

    #[test]
    fn a_never_policy_leaves_even_a_crash_alone() {
        let mut k = kernel();
        let mut d = def("a");
        d.restart = RestartPolicy::Never;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Signalled(11)));
        s.reap(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Done));
        // The state that makes `never` mean never. `Idle` plus `enabled` means
        // Start, so reaching `Idle` here would restart it on the very next
        // pass, which is the opposite of what the policy says.
        for _ in 0..5 {
            s.converge(&mut k);
        }
        assert_eq!(k.spawned.len(), 1);
    }

    /// `enabled = false`, apply, `enabled = true`, apply is the only way to run
    /// a completed oneshot again — `ADR-017` §6 has no manual start.
    #[test]
    fn disabling_a_completed_service_lets_it_run_again_when_re_enabled() {
        let mut k = kernel();
        let mut d = def("a");
        d.restart = RestartPolicy::Never;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        s.services[0].def.enabled = false;
        s.converge(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        s.services[0].def.enabled = true;
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 2);
    }

    /// A spawn that keeps failing for a reason that *could* pass — the machine
    /// was momentarily out of process slots — must throttle and eventually give
    /// up exactly as a crash loop does, or the supervisor becomes an
    /// unthrottled loop.
    ///
    /// ~~This test used `ENOENT`~~ and was changed 2026-08-23, because a
    /// missing binary stopped being a case that backs off: `/system` is
    /// read-only, so the kernel's answer cannot change and the supervisor now
    /// refuses on the first attempt. The two behaviours are tested separately
    /// below, and the distinction is
    /// `Errno::is_a_program_this_image_cannot_run`.
    #[test]
    fn a_spawn_that_keeps_failing_transiently_backs_off_and_gives_up() {
        let mut k = kernel();
        k.spawn_results = Script::always(Err(Errno::EAGAIN));
        let mut s = supervisor(vec![def("a")]);
        for _ in 0..20 {
            s.converge(&mut k);
            if let Some(State::Backoff { until_ms }) = s.state_of("a") {
                k.mono_ms = *until_ms;
            }
        }
        assert_eq!(s.state_of("a"), Some(&State::Failed));
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg == "could not be started")
        );
    }

    /// The other side of the same fork, and the one that matters on a card
    /// carrying a definition whose binary is not in the image. One attempt, one
    /// record, `Failed`, and nothing written to the SD card four more times to
    /// reach the same answer.
    #[test]
    fn a_program_this_image_cannot_run_is_refused_on_the_first_attempt() {
        for errno in [Errno::ENOENT, Errno::ENOTDIR, Errno::EACCES, Errno::ENOEXEC] {
            let mut k = kernel();
            k.spawn_results = Script::always(Err(errno));
            let mut s = supervisor(vec![def("a")]);
            for _ in 0..5 {
                s.converge(&mut k);
                k.mono_ms += 60_000;
            }
            assert_eq!(s.state_of("a"), Some(&State::Failed), "{errno}");
            let refusals = k
                .records
                .iter()
                .filter(|r| r.msg.starts_with("REFUSED this image cannot run"))
                .count();
            assert_eq!(refusals, 1, "{errno} was reported {refusals} times");
            assert!(
                !k.records.iter().any(|r| r.msg == "will retry after backoff"),
                "{errno} must not schedule a retry"
            );
        }
    }

    /// The refusal has to name the program and what the machine lost, or it is
    /// an error a human cannot act on — `ADR-017` §3.1.6's `consequence_of`
    /// applied to a service.
    #[test]
    fn the_refusal_names_the_program_and_what_its_absence_costs() {
        let mut k = kernel();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(record) = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("REFUSED this image cannot run"))
        else {
            unreachable!("nothing was refused")
        };
        let rendered = format!("{:?}", record.fields);
        assert!(rendered.contains("/bin/svc"), "{rendered}");
        assert!(rendered.contains("would lose whatever this stands for"), "{rendered}");
    }

    /// `EPERM` is deliberately outside the permanent set: a seccomp filter or a
    /// container policy can produce it and can change.
    #[test]
    fn a_refused_permission_still_retries_because_a_policy_can_change() {
        let mut k = kernel();
        k.spawn_results = Script::always(Err(Errno::EPERM));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Backoff { .. })));
    }

    // ------------------------------------------------------------------
    // `ADR-055` §5: an enabled application missing its declared material is
    // HELD `waiting-for-material` rather than spawned and crash-looped.
    // ------------------------------------------------------------------

    /// An application ready to pass every OTHER precondition — signed,
    /// identified, on a volume — that declares `fleet-tls` and has none of it.
    fn material_def(name: &str) -> ServiceDef {
        let mut d = application_def(name);
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        d.material = vec![rivet_core::package::application::Material::FleetTls];
        d
    }

    /// Put a correctly signed program where `d.program.path` says, so a test
    /// about material is not also, silently, a test about verification.
    fn signed(k: &mut FakeKernel, d: &ServiceDef) {
        let version_dir = d
            .program
            .path
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .expect("a version directory");
        crate::verify::fixture::install_signed(k, &d.name, &version_dir, &d.program.path);
    }

    /// The three files `Material::FleetTls` names, in `d`'s data directory.
    fn data_dir_of(d: &ServiceDef) -> String {
        rivet_core::service::data_dir(
            d.volume.as_deref().expect("a fixture volume"),
            &rivet_core::service::ServiceName::new(&d.name).expect("a valid fixture name"),
        )
    }

    #[test]
    fn an_enabled_application_missing_its_material_is_held_not_spawned() {
        let mut k = kernel();
        let d = material_def("a");
        signed(&mut k, &d);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(k.spawned.is_empty(), "{:?}", k.spawned);
        assert_eq!(
            s.state_of("a").map(state_word),
            Some("waiting-for-material"),
            "{:?}",
            s.state_of("a")
        );
    }

    #[test]
    fn it_journals_the_hold_exactly_once_across_several_passes() {
        let mut k = kernel();
        let d = material_def("a");
        signed(&mut k, &d);
        let mut s = supervisor(vec![d]);
        for _ in 0..5 {
            s.converge(&mut k);
            k.mono_ms += Limits::default().material_poll_ms;
        }
        let holds = k
            .records
            .iter()
            .filter(|r| r.msg.starts_with("HELD waiting-for-material"))
            .count();
        assert_eq!(holds, 1, "{holds} records: {:?}", k.records);
        assert!(
            matches!(s.state_of("a"), Some(State::WaitingForMaterial { .. })),
            "{:?}",
            s.state_of("a")
        );
    }

    #[test]
    fn once_the_three_files_arrive_the_next_check_spawns_it() {
        let mut k = kernel();
        let d = material_def("a");
        signed(&mut k, &d);
        let data = data_dir_of(&d);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(k.spawned.is_empty(), "{:?}", k.spawned);

        for file in ["tls-cert", "tls-key", "tls-root"] {
            k.exists_mut()
                .insert(format!("{data}/{file}"), Script::always(Ok(true)));
        }
        // No reload and no restart — advancing the clock to the next poll is
        // the whole of what makes the next pass look again.
        k.mono_ms += Limits::default().material_poll_ms;
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1, "{:?}", k.spawned);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "{:?}",
            s.state_of("a")
        );
    }

    #[test]
    fn waiting_for_material_never_counts_toward_the_give_up_threshold() {
        let mut k = kernel();
        let d = material_def("a");
        signed(&mut k, &d);
        let mut s = supervisor(vec![d]);
        for _ in 0..(Limits::default().give_up_after as usize + 5) {
            s.converge(&mut k);
            k.mono_ms += Limits::default().material_poll_ms;
        }
        assert!(k.spawned.is_empty(), "{:?}", k.spawned);
        assert_eq!(
            s.state_of("a").map(state_word),
            Some("waiting-for-material"),
            "a service never spawned must not reach Failed by way of the give-up threshold: {:?}",
            s.state_of("a")
        );
    }

    #[test]
    fn an_application_that_declares_no_material_starts_exactly_as_before() {
        let mut k = kernel();
        let mut d = application_def("a");
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        signed(&mut k, &d);
        assert!(d.material.is_empty(), "the fixture must declare nothing");
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "{:?}",
            s.state_of("a")
        );
    }

    #[test]
    fn one_services_failure_does_not_stop_another_from_running() {
        let mut k = kernel();
        k.spawn_results = Script::sequence(vec![Err(Errno::EAGAIN)], Ok(4_242));
        let mut s = supervisor(vec![def("broken"), def("fine")]);
        s.converge(&mut k);
        assert!(matches!(s.state_of("broken"), Some(State::Backoff { .. })));
        assert!(matches!(s.state_of("fine"), Some(State::Started { .. })));
    }

    /// **The run-once shape, driven end to end through the engine.**
    ///
    /// `ADR-017` §3.5's `on-failure` plus `State::Done` is what expresses "runs
    /// once, exits 0, stays exited", and this is the test that says so: a
    /// service that converged and exited cleanly is not started again on the
    /// next pass, however many passes there are.
    #[test]
    fn an_on_failure_service_that_exits_cleanly_is_not_started_again() {
        let mut k = kernel();
        let mut oneshot = def("converger");
        oneshot.restart = RestartPolicy::OnFailure;
        let mut s = supervisor(vec![oneshot]);

        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1);
        k.exits.push((crate::testkernel::FIRST_FAKE_PID, ExitStatus::Exited(0)));
        s.reap(&mut k);
        assert_eq!(s.state_of("converger"), Some(&State::Done));

        for _ in 0..10 {
            k.mono_ms += 60_000;
            s.converge(&mut k);
        }
        assert_eq!(
            k.spawned.len(),
            1,
            "a converge that succeeded must not be run again on every pass"
        );
        assert_eq!(s.state_of("converger"), Some(&State::Done));
    }

    /// And the other half of the same policy: a converge that *failed* is a
    /// precondition that was not ready, which `ADR-017` §3.2 says retries.
    #[test]
    fn an_on_failure_service_that_exits_non_zero_is_started_again() {
        let mut k = kernel();
        let mut oneshot = def("converger");
        oneshot.restart = RestartPolicy::OnFailure;
        let mut s = supervisor(vec![oneshot]);

        s.converge(&mut k);
        k.exits.push((crate::testkernel::FIRST_FAKE_PID, ExitStatus::Exited(1)));
        s.reap(&mut k);
        let Some(State::Backoff { until_ms }) = s.state_of("converger") else {
            unreachable!("a failed converge must be retried, not left Done")
        };
        k.mono_ms = *until_ms;
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 2);
    }

    /// Convergence, not sequencing: services start together and none waits for
    /// another (`ADR-017` §3.2).
    #[test]
    fn every_service_starts_in_the_same_pass_with_no_ordering_between_them() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a"), def("b"), def("c")]);
        s.converge(&mut k);
        let names: Vec<_> = k.spawned.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert!(
            s.services()
                .iter()
                .all(|m| matches!(m.state, State::Started { .. }))
        );
    }

    #[test]
    fn reaping_a_pid_the_supervisor_does_not_know_is_reported_not_ignored() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        k.exits.push((9_999, ExitStatus::Exited(0)));
        s.reap(&mut k);
        assert!(
            k.records
                .iter()
                .any(|r| r.msg == "reaped a child the supervisor was not tracking")
        );
    }

    // ----------------------------------------------------------------------
    // Requirement 3: SIGTERM stops children and the supervisor exits
    // ----------------------------------------------------------------------

    #[test]
    fn shutdown_terms_every_running_child_and_then_finishes() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a"), def("b")]);
        s.converge(&mut k);
        let pids: Vec<i32> = s
            .services()
            .iter()
            .filter_map(|m| match m.state {
                State::Started { pid, .. } => Some(pid),
                _ => None,
            })
            .collect();
        assert_eq!(pids.len(), 2);

        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);
        assert_eq!(
            k.signals,
            vec![(pids[0], Sig::Term), (pids[1], Sig::Term)]
        );
        // **`ADR-022` §12 item 7**: the shutdown record says WHICH and why.
        // Until 2026-08-26 `Event::Terminate` collapsed SIGTERM and SIGINT, so
        // this field was unwritable rather than merely unwritten — and a journal
        // that cannot tell a clean restart from a power-off cannot answer the
        // question §12.3 says this record exists for, since ADR-018 §5.1's
        // rollback path is entered by exactly one of them.
        let started = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("shutdown requested"))
            .unwrap_or_else(|| panic!("no shutdown record: {:?}", k.records));
        assert_eq!(
            started
                .fields
                .iter()
                .find(|(name, _)| name == "shutdown")
                .map(|(_, v)| v.clone()),
            Some(Value::Str("power-off".to_string())),
            "{:?}",
            started.fields
        );
        assert!(
            started.fields.iter().any(|(name, v)| name == "reason"
                && matches!(v, Value::Str(text) if text.contains("SIGTERM"))),
            "the record does not say what it was told: {:?}",
            started.fields
        );
        assert!(!s.is_finished(), "finished before the children exited");

        for pid in &pids {
            k.exits.push((*pid, ExitStatus::Signalled(15)));
        }
        s.reap(&mut k);
        s.converge(&mut k);
        assert!(s.is_finished());
        assert_eq!(s.phase(), Phase::Finished);
    }

    #[test]
    fn shutdown_with_nothing_running_finishes_immediately() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);
        assert!(s.is_finished());
    }

    #[test]
    fn a_child_that_ignores_sigterm_is_killed_after_the_grace_period() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);

        k.mono_ms = Limits::default().stop_grace_ms;
        s.converge(&mut k);
        assert_eq!(k.signals, vec![(pid, Sig::Term), (pid, Sig::Kill)]);
        assert!(!s.is_finished());
    }

    /// A supervisor that waited for ever would block PID 1's shutdown. It lets
    /// go instead, which is safe because the process reparents to PID 1 —
    /// `ADR-017` §3.1.1.
    #[test]
    fn a_child_that_survives_sigkill_is_abandoned_so_shutdown_can_finish() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);

        let l = Limits::default();
        k.mono_ms = l.stop_grace_ms;
        s.converge(&mut k);
        k.mono_ms += l.kill_grace_ms;
        s.converge(&mut k);

        assert!(s.is_finished());
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("did not exit after SIGKILL"))
        );
    }

    #[test]
    fn a_second_sigterm_during_shutdown_changes_nothing() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);
        let signals_after_first = k.signals.len();
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);
        assert_eq!(k.signals.len(), signals_after_first);
    }

    #[test]
    fn a_service_in_backoff_when_shutdown_arrives_is_not_started() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        k.mono_ms = 100_000;
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1);
        assert!(s.is_finished());
    }

    #[test]
    fn stopping_a_service_removes_its_notification_socket() {
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert_eq!(k.notify_open, vec!["a".to_string()]);
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);
        assert!(k.notify_open.is_empty());
    }

    /// The exit of a service the supervisor asked to stop is expected, and must
    /// not be treated as a failure that earns a restart.
    #[test]
    fn an_expected_exit_during_shutdown_does_not_count_as_a_failure() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        s.begin_shutdown(&mut k, Terminating::PowerOff);
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        assert_eq!(s.services()[0].attempts, 0);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
    }

    /// Disabling a service is the only stop there is (`ADR-017` §6), and once
    /// it has stopped it must stay stopped rather than being restarted by the
    /// exit accounting.
    #[test]
    fn a_disabled_service_that_is_stopped_stays_stopped() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        s.services[0].def.enabled = false;
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        s.converge(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert_eq!(k.spawned.len(), 1);
    }

    // ----------------------------------------------------------------------
    // Uninstall — rivet_core::uninstall, ruled 2026-08-25
    // ----------------------------------------------------------------------

    /// An installed tree on the fake, as the installer would have left it.
    ///
    /// **`current` is registered as NOT a directory**, which is what it is —
    /// `ADR-015` §7 makes it a symlink — and what `Kernel::list_dir` therefore
    /// never reports. A fixture that left it looking like a directory is what
    /// let the first uninstall on a board leave a dangling link (2026-08-25).
    fn installed(k: &mut FakeKernel, name: &str) {
        let app = format!("/state/app/{name}");
        k.dirs_mut()
            .insert("/state/app".to_string(), vec![name.to_string()]);
        k.dirs_mut().insert(
            app.clone(),
            vec![
                "0.1.0".to_string(),
                "current".to_string(),
                "data".to_string(),
            ],
        );
        k.dirs_mut().insert(format!("{app}/0.1.0"), vec![]);
        k.dirs_mut().insert(format!("{app}/data"), vec![]);
        k.not_a_directory.insert(format!("{app}/current"));
        // The link exists as far as `remove_file` is concerned. `files_mut` is
        // the public way to say a path is there; `live` is the fake's own.
        k.files_mut().insert(
            format!("{app}/current"),
            crate::testkernel::Script::always(Ok("0.1.0".to_string())),
        );
    }

    /// **The ruling of 2026-08-25, as a test.** A plain uninstall removes the
    /// versions and leaves the data.
    #[test]
    fn an_uninstall_removes_the_versions_and_keeps_the_data() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![]);
        s.begin_uninstall(&mut k, "a", false);

        assert!(
            k.removed_paths().iter().any(|p| p == "/state/app/a/0.1.0"),
            "the version was not removed: {:?}",
            k.removed_paths()
        );
        assert!(
            !k.removed_paths().iter().any(|p| p == "/state/app/a/data"),
            "the DATA was removed and the ruling says it must not be: {:?}",
            k.removed_paths()
        );
        // **The `current` link goes too, and a board is why this is asserted.**
        // It is a symlink, `Kernel::list_dir` reports directories only, and the
        // first uninstall on real hardware enumerated the directory, removed
        // everything it was shown, reported success, and left the link
        // dangling. It is removed BY NAME now, not by listing.
        assert!(
            k.removed.iter().any(|p| p == "/state/app/a/current"),
            "the `current` link was not removed: {:?}",
            k.removed
        );
    }

    /// `application_def`, plus the two things an application needs before the
    /// supervisor will actually spawn it: a **volume** to compose its paths
    /// from (`ADR-031` §8 item 1, and the supervisor refuses to guess one) and
    /// a **signed header** for `verify::program`, which runs immediately before
    /// every application spawn (`ADR-015` §4.2).
    ///
    /// Without the header the service reaches `Failed` with *"no signed header
    /// at /package-header"* and never gets near the spawn — which is the right
    /// behaviour and makes a fixture that omits it silently test nothing.
    fn app_on_volume(k: &mut FakeKernel, name: &str) -> ServiceDef {
        let mut d = application_def(name);
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        crate::verify::fixture::install_signed(k, name, "", &d.program.path);
        d
    }

    /// **The scratch directory is emptied at BOTH edges**, and the two are not
    /// the same edge for the same reason.
    ///
    /// The exit is what reclaims the space — a crash leaves debris and holding
    /// it until the application next happens to run is how a disk fills up.
    /// The spawn is for the boot after a power cut, where no exit was ever
    /// observed and nothing on the exit path could have run.
    #[test]
    fn an_applications_scratch_is_emptied_at_the_spawn_and_again_at_the_exit() {
        let mut k = kernel();
        installed(&mut k, "a");
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        let mut s = supervisor(vec![app_on_volume(&mut k, "a")]);

        s.converge(&mut k);
        assert_eq!(
            scratch_set_aside(&k),
            1,
            "the scratch directory was not emptied before the spawn: renames={:?} state={:?}",
            k.renames,
            s.state_of("a"),
        );

        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the service should be running")
        };
        // The run fills it again.
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert_eq!(
            scratch_set_aside(&k),
            2,
            "the exit edge left the scratch directory holding whatever the run put there, so \
             the space is not reclaimed until the application next starts: {:?}",
            k.renames
        );

        // **And the data directory is untouched by all of it.** This is the
        // assertion that makes the sibling layout worth having: a bug in the
        // emptying reaches scratch and cannot reach the history.
        assert!(
            !k.removed_paths().iter().any(|p| p == "/state/app/a/data"),
            "emptying scratch removed the application's data: {:?}",
            k.removed_paths()
        );
        assert!(
            !k.reclaims.iter().any(|(_, _, prefix)| !prefix.starts_with("tmp.")),
            "a reclaim was aimed at something other than scratch: {:?}",
            k.reclaims
        );
    }

    /// How many times `a`'s scratch directory was renamed aside — which is how
    /// the loop empties it, `ADR-017` §3. Counts the attempts, as the fake
    /// records a rename before it answers.
    fn scratch_set_aside(k: &FakeKernel) -> usize {
        k.renames
            .iter()
            .filter(|(from, to)| {
                from == "/state/app/a/tmp" && to.starts_with("/state/app/a/tmp.reclaim-")
            })
            .count()
    }

    /// **The loop deletes no scratch itself.** It renames the directory
    /// aside, makes a fresh one, and hands the old one to a helper —
    /// `ADR-017` §3, because deleting costs time in proportion to what an
    /// application left and the loop is single threaded.
    #[test]
    fn scratch_is_renamed_aside_and_deleted_by_a_helper_not_by_the_loop() {
        let mut k = kernel();
        installed(&mut k, "a");
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        let mut s = supervisor(vec![app_on_volume(&mut k, "a")]);

        s.converge(&mut k);

        let aside = k
            .renames
            .iter()
            .find(|(from, _)| from == "/state/app/a/tmp")
            .map(|(_, to)| to.clone())
            .unwrap_or_else(|| panic!("scratch was not renamed aside: {:?}", k.renames));
        assert!(
            aside.starts_with("/state/app/a/tmp.reclaim-"),
            "set aside somewhere a helper is not told to look: {aside}"
        );
        assert_eq!(
            k.reclaims,
            [(
                "a".to_string(),
                "/state/app/a".to_string(),
                "tmp.reclaim-".to_string()
            )],
            "the set-aside directory was not handed to a helper"
        );
        assert!(
            k.mkdir_mode_calls
                .iter()
                .any(|(p, _)| p == "/state/app/a/tmp"),
            "no fresh scratch directory was made for the spawn"
        );
        assert!(
            k.spawned.iter().any(|(n, _)| n == "a"),
            "the application did not start"
        );
    }

    /// **Two set aside in one millisecond do not collide.** An exit and the
    /// restart's spawn can land in one pass on one clock reading, while the
    /// first aside directory is still being deleted.
    #[test]
    fn every_aside_name_is_its_own() {
        let mut k = kernel();
        installed(&mut k, "a");
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        let mut s = supervisor(vec![app_on_volume(&mut k, "a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the service should be running")
        };
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        let asides: Vec<&String> = k
            .renames
            .iter()
            .filter(|(from, _)| from == "/state/app/a/tmp")
            .map(|(_, to)| to)
            .collect();
        assert_eq!(asides.len(), 2, "{:?}", k.renames);
        assert_ne!(asides[0], asides[1], "two aside directories were given one name");
    }

    /// **A helper's failure is said once, as a warning; its success is quiet.**
    #[test]
    fn a_reclaim_that_failed_is_a_warning_and_one_that_succeeded_is_not() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![app_on_volume(&mut k, "a")]);
        s.note_reclaimed(&mut k, "a", "/state/app/a", "tmp.reclaim-", None);
        assert!(
            !k.records.iter().any(|r| r.sev >= Severity::Info),
            "a scratch helper that deleted everything was more than a debug line"
        );
        s.note_reclaimed(
            &mut k,
            "a",
            "/state/app/a",
            "tmp.reclaim-",
            Some("Permission denied (os error 13)"),
        );
        let warned: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.sev == Severity::Warn && r.msg.contains("scratch"))
            .collect();
        assert_eq!(warned.len(), 1, "{:?}", k.records);
        assert_eq!(warned[0].svc, "a");
    }

    /// **A removal's helper is said at `Info` when it is done**, because that
    /// is when the space comes back and removals are rare; its failure is one
    /// warning naming the directory.
    #[test]
    fn a_removals_helper_is_reported_at_info_and_its_failure_as_a_warning() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        s.note_reclaimed(&mut k, "a", "/state/app", ".reclaim-", None);
        let said: Vec<_> = k.records.iter().filter(|r| r.sev == Severity::Info).collect();
        assert_eq!(said.len(), 1, "{:?}", k.records);
        assert!(said[0].msg.contains("removal"), "{:?}", said[0]);
        s.note_reclaimed(&mut k, "a", "/state/app", ".reclaim-", Some("EIO"));
        let warned: Vec<_> = k.records.iter().filter(|r| r.sev == Severity::Warn).collect();
        assert_eq!(warned.len(), 1, "{:?}", k.records);
        assert!(
            warned[0]
                .fields
                .iter()
                .any(|(key, v)| key == "path" && *v == Value::Str("/state/app".to_string())),
            "{:?}",
            warned[0]
        );
    }

    /// **A directory that is not there is already empty, and says nothing.**
    ///
    /// A rename answers `ENOENT` for a missing path, and for an uninstall
    /// "removed nothing" is one of the two answers that matter. For emptying
    /// it is success — the first spawn after
    /// an install, and every spawn after an exit that tidied up. Reporting it
    /// would put a warning in the journal at the start of every well-behaved
    /// application, which is how a journal stops being read.
    #[test]
    fn a_scratch_directory_that_is_not_there_is_not_a_complaint() {
        let mut k = kernel();
        installed(&mut k, "a");
        // Deliberately NOT creating `/state/app/a/tmp`.
        let mut s = supervisor(vec![app_on_volume(&mut k, "a")]);
        s.converge(&mut k);

        assert!(
            !k.records
                .iter()
                .any(|r| r.sev == Severity::Warn && r.msg.contains("scratch")),
            "a missing scratch directory was reported as trouble: {:?}",
            k.records
                .iter()
                .filter(|r| r.sev == Severity::Warn)
                .collect::<Vec<_>>()
        );
        assert!(
            k.spawned.iter().any(|(n, _)| n == "a"),
            "the application did not start"
        );
    }

    /// A built-in service has no identity and no scratch directory, and the
    /// supervisor must not invent one at `/state/app/<name>/tmp` for it.
    #[test]
    fn a_built_in_service_gets_no_scratch_directory() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(
            !k.renames.iter().any(|(from, _)| from.contains("/tmp")) && k.reclaims.is_empty(),
            "{:?} {:?}",
            k.renames,
            k.reclaims
        );
    }

    /// And `purge` removes it.
    #[test]
    fn purge_removes_the_data_too() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![]);
        s.begin_uninstall(&mut k, "a", true);

        assert_eq!(
            k.list_dir("/state/app/a/data"),
            Err(Errno::ENOENT),
            "purge kept the data: {:?}",
            k.removed_paths()
        );
    }

    /// **An uninstall answers at the rename, and a helper deletes** —
    /// `ADR-017` §3. The loop deletes nothing itself: each version is renamed
    /// aside inside the application's directory, so it is gone from every
    /// listing when the removal is reported, and a helper is sent to delete it.
    #[test]
    fn an_uninstall_renames_the_versions_aside_and_a_helper_deletes_them() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![]);
        s.begin_uninstall(&mut k, "a", false);

        assert!(
            k.renames
                .iter()
                .any(|(from, to)| from == "/state/app/a/0.1.0"
                    && to.starts_with("/state/app/a/.reclaim-")),
            "the version was not renamed aside: {:?}",
            k.renames
        );
        assert!(
            k.reclaims
                .contains(&("a".to_string(), "/state/app/a".to_string(), ".reclaim-".to_string())),
            "no helper was sent: {:?}",
            k.reclaims
        );
        assert_eq!(k.list_dir("/state/app/a"), Ok(vec!["data".to_string()]));
        assert!(
            k.records.iter().any(|r| r.msg == "the application is removed"),
            "{:?}",
            k.records
        );
    }

    /// **A purge renames the whole application aside in one step**, so its
    /// versions and its data go together and nothing is left half removed.
    #[test]
    fn a_purge_renames_the_whole_application_aside_in_one_step() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![]);
        s.begin_uninstall(&mut k, "a", true);

        assert_eq!(k.renames.len(), 1, "{:?}", k.renames);
        assert_eq!(k.renames[0].0, "/state/app/a");
        assert!(k.renames[0].1.starts_with("/state/app/.reclaim-"), "{:?}", k.renames);
        assert_eq!(
            k.reclaims,
            [("a".to_string(), "/state/app".to_string(), ".reclaim-".to_string())]
        );
        assert_eq!(k.list_dir("/state/app"), Ok(vec![]));
    }

    /// **A running application is stopped before anything is removed.** The
    /// whole reason this is the supervisor's and not the installer's — removing
    /// the tree from under a live process is the failure being avoided.
    #[test]
    fn a_running_application_is_stopped_first_and_removed_only_when_it_is_down() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut d = def("a");
        d.origin = Origin::Application;
        // An application is on a volume, and `finish_uninstalls` refuses to
        // remove one whose volume it cannot name (`ADR-031` §8 item 1).
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        // `ServiceDef::validate` refuses an application with no identity —
        // `ADR-015` §5.1 made that structural on 2026-08-24 — so the fixture
        // has to carry one.
        d.program.identity = Some(rivet_core::service::AppIdentity {
            uid: 27_751,
            gid: 27_751,
        });
        // An application must pass launch verification to start at all, so the
        // fixture that satisfies it is the one `crate::verify` uses.
        let version_dir = d
            .program
            .path
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .expect("a version directory");
        crate::verify::fixture::install_signed(&mut k, "a", &version_dir, &d.program.path);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "the fixture must be running for this test to mean anything: {:?}",
            s.state_of("a")
        );

        s.begin_uninstall(&mut k, "a", false);
        // **The scratch directory is excluded, and nothing else is.** Since
        // 2026-09-07 the supervisor empties `tmp` at every spawn, so
        // `removed_paths()` is no longer empty on a machine that started
        // anything — but this test is about the VERSION and DATA trees, which
        // are the ones that must not go while a process is still using them.
        let while_up: Vec<_> = k
            .removed_paths()
            .into_iter()
            .filter(|p| !p.ends_with("/tmp"))
            .collect();
        assert!(
            while_up.is_empty(),
            "it removed the tree while the service was still up: {while_up:?}"
        );

        // The stop, through the machinery that already existed: the service is
        // disabled, `decide` moves it to `Stopping`, and the exit brings it
        // down. Nothing here is new — that is the point of the design.
        let pid = match s.state_of("a") {
            Some(State::Started { pid, .. } | State::Stopping { pid, .. }) => *pid,
            other => unreachable!("{other:?}"),
        };
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        s.converge(&mut k);

        assert!(
            k.removed_paths().iter().any(|p| p == "/state/app/a/0.1.0"),
            "it never removed the tree after the service came down: {:?}",
            k.removed_paths()
        );
        assert!(
            s.state_of("a").is_none(),
            "the service is still in the table after being uninstalled"
        );
    }

    /// An application this supervisor does not run is removed straight away —
    /// there is nothing to wait for. `growfix` on the board was exactly this.
    #[test]
    fn an_application_that_is_not_a_service_is_removed_immediately() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![]);
        s.begin_uninstall(&mut k, "a", false);
        assert!(
            k.removed_paths().iter().any(|p| p == "/state/app/a/0.1.0"),
            "{:?}",
            k.removed_paths()
        );
    }

    /// **A service from the image cannot be uninstalled.** Refused before
    /// anything is stopped — the failure being prevented is a machine that has
    /// disabled and killed its control plane and then reports it could not
    /// remove it.
    #[test]
    fn a_service_from_the_image_is_refused_and_not_even_stopped() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.begin_uninstall(&mut k, "a", false);

        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "an image service was stopped by an uninstall: {:?}",
            s.state_of("a")
        );
    }

    /// A name that is not one never reaches a path.
    #[test]
    fn a_name_that_is_not_usable_removes_nothing() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut s = supervisor(vec![]);
        s.begin_uninstall(&mut k, "../../system", false);
        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
    }

    // ----------------------------------------------------------------------
    // The declared device capability — ADR-015 §6.2, kiosk mode, 2026-08-31
    // ----------------------------------------------------------------------

    /// A kiosk application, with `/dev` populated the way a Pi 4 populates it:
    /// a framebuffer, four event nodes, no `/dev/dri` — measured off the
    /// maintainer's board on 2026-08-31.
    fn kiosk_def(k: &mut FakeKernel, name: &str, devices: Vec<Device>) -> ServiceDef {
        // `/dev` as a Pi 4 populates it, trimmed to what matters here: a
        // framebuffer, and NO `dri` directory — the kernel builds DRM as a
        // module and Rivet loads none (`PLAN.md` §103.12).
        k.dir_entries.insert(
            "/dev".to_string(),
            vec!["fb0".to_string(), "null".to_string(), "tty1".to_string()],
        );
        k.dir_entries.insert(
            "/dev/input".to_string(),
            vec![
                "event0".to_string(),
                "event1".to_string(),
                "mice".to_string(),
                "mouse0".to_string(),
            ],
        );
        let mut d = application_def(name);
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        d.devices = devices;
        crate::verify::fixture::install_signed(k, name, "", &d.program.path);
        d
    }

    /// **The same board, once DRM has bound.** `/dev/dri` holding `card0`,
    /// `card1` and `renderD128` is what a Raspberry Pi 4 with VC4 and V3D up
    /// reported, read off the board and recorded in
    /// `docs/PLAN-Archive-08-2026-08-30-to-08-31.md` — not a set composed to
    /// suit a test.
    ///
    /// It is separate from [`kiosk_def`] rather than folded into it, because the
    /// no-`/dev/dri` machine is still a real one and
    /// `a_device_class_whose_nodes_are_absent_still_starts_the_service` is about
    /// exactly that.
    fn with_drm(k: &mut FakeKernel) {
        k.dir_entries.insert(
            "/dev/dri".to_string(),
            vec![
                "card0".to_string(),
                "card1".to_string(),
                "renderD128".to_string(),
            ],
        );
    }

    /// **A definition that controls the kiosk** — `connects =
    /// ["rivet-sysd:kiosk"]`, which is what makes an application the arbiter of
    /// the screen and is the launcher's own declaration.
    fn controlling_the_kiosk(mut d: ServiceDef) -> ServiceDef {
        use rivet_core::package::application::Endpoint;
        d.connects = vec![
            Endpoint::parse(
                "connects",
                &format!(
                    "{}:{}",
                    rivet_core::kiosk::PROVIDER,
                    rivet_core::kiosk::ENDPOINT
                ),
            )
            .unwrap_or_else(|_| unreachable!("the kiosk endpoint parses")),
        ];
        d
    }

    /// The same shape for `ADR-058` §4's compositor: it PROVIDES the
    /// Wayland socket, which is the declaration `Supervisor::display_role`
    /// now reads, and it connects to nothing.
    fn providing_the_compositor(mut d: ServiceDef) -> ServiceDef {
        use rivet_core::package::application::Endpoint;
        d.provides = vec![
            Endpoint::parse(
                "provides",
                &format!("socket:{}", rivet_core::compositor::ENDPOINT),
            )
            .unwrap_or_else(|_| unreachable!("the compositor endpoint parses")),
        ];
        d
    }

    /// **A converted kiosk application, declared exactly as `aura-renderer`
    /// declares itself** — `connects = ["rivet-displayd:wayland"]` and
    /// `devices = []`.
    ///
    /// The `devices` argument is deliberately not offered: a fixture that
    /// could pass `vec![Device::Display]` here is the fixture that hid the
    /// defect this exists for. Every other kiosk in these tests is built by
    /// `kiosk_def(…, vec![Device::Display])`, so the whole suite declared the
    /// one thing `ADR-058` removed and agreed with a supervisor that still
    /// required it.
    fn drawing_through_the_compositor(mut d: ServiceDef) -> ServiceDef {
        use rivet_core::package::application::Endpoint;
        d.devices = vec![];
        d.connects = vec![
            Endpoint::parse(
                "connects",
                &format!(
                    "{}:{}",
                    rivet_core::compositor::PROVIDER,
                    rivet_core::compositor::ENDPOINT
                ),
            )
            .unwrap_or_else(|_| unreachable!("the compositor endpoint parses")),
        ];
        d
    }

    /// **A Wayland client is on the menu, and it declares no device at all.**
    ///
    /// Found on `rivet-06f5c` on 2026-09-20, by looking at the screen: the
    /// launcher drew *"no application on this machine offers a screen"* while
    /// `aura-renderer` was `started` and drawing a clock. `offered_displays`
    /// still tested `devices.contains(Display)`, which the conversion to a
    /// surface had removed from the manifest — so the one application on the
    /// machine that wanted the screen was invisible to the thing that hands
    /// it out.
    ///
    /// This is the fourth defect in this subsystem found by a board and not by
    /// a test, and the reason is the same each time: the fixtures were written
    /// against the declaration that was true when they were written.
    #[test]
    fn an_application_that_connects_to_the_compositor_is_offered_the_screen() {
        let mut k = kernel();
        let compositor =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        let launcher = controlling_the_kiosk(kiosk_def(&mut k, "rivet-launcher", vec![]));
        let app = drawing_through_the_compositor(kiosk_def(&mut k, "aura-renderer", vec![]));
        let s = supervisor(vec![compositor, launcher, app]);

        assert_eq!(
            s.display_offered,
            vec!["aura-renderer".to_string()],
            "a client that declares `connects = [\"rivet-displayd:wayland\"]` must be \
             offerable; neither the compositor nor the launcher is an item on the menu"
        );
    }

    /// **And the lease's declaration still counts**, on the same machine and
    /// at the same time.
    ///
    /// The two spellings are alternatives in the manifest and additive here.
    /// A card written before `ADR-058` runs applications that say `display`
    /// and has no compositor to connect to; one written after runs
    /// applications that say `connects` and grants no DRM node. Both must
    /// produce a menu, and an application carrying both is offered once —
    /// this is a filter, not a sum.
    #[test]
    fn a_leasing_application_and_a_wayland_client_are_both_offered_once() {
        let mut k = kernel();
        let compositor =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        let legacy = kiosk_def(&mut k, "old-kiosk", vec![Device::Display]);
        let client = drawing_through_the_compositor(kiosk_def(&mut k, "new-kiosk", vec![]));
        // Mid-conversion: it has gained the surface and not yet dropped the
        // device. It must appear exactly once.
        let mut both = drawing_through_the_compositor(kiosk_def(&mut k, "both-kiosk", vec![]));
        both.devices = vec![Device::Display];
        let s = supervisor(vec![compositor, legacy, client, both]);

        assert_eq!(
            s.display_offered,
            vec![
                "old-kiosk".to_string(),
                "new-kiosk".to_string(),
                "both-kiosk".to_string()
            ],
            "both declarations are live, and neither duplicates an application"
        );
    }

    /// The grant happens **before** the spawn, so a program cannot start, find
    /// its device unopenable, and give up.
    ///
    /// **The node asserted is the RENDER node**, because this application is a
    /// lessee: it declares nothing about controlling the kiosk, so `ADR-036`
    /// §3.1 gives it `renderD*` and nothing else. A render node is shared, so
    /// the grant is the node at `root:render 0660` and the render group on the
    /// program that is spawned — `crate::devices::groups_for`.
    #[test]
    fn a_declared_display_is_granted_to_the_application_before_it_starts() {
        let mut k = kernel();
        let d = kiosk_def(&mut k, "a", vec![Device::Display]);
        with_drm(&mut k);
        let mut s = showing(vec![d], "a");
        s.converge(&mut k);

        assert!(
            k.chowns.iter().any(|(p, u, g)| p == "/dev/dri/renderD128"
                && *u == 0
                && *g == rivet_core::etc::RENDER_GID),
            "{:?}",
            k.chowns
        );
        assert!(
            k.chmods.iter().any(|(p, m)| p == "/dev/dri/renderD128" && *m == 0o660),
            "{:?}",
            k.chmods
        );
        assert_eq!(k.spawned.len(), 1, "it should still have started");
        assert!(
            k.spawned[0].1.groups.contains(&rivet_core::etc::RENDER_GID),
            "the application was spawned without the render group: {:?}",
            k.spawned[0].1.groups
        );
    }

    /// **A LESSEE is granted the render node and NOTHING that reaches the
    /// screen** — `ADR-036` §3.1 as amended 2026-09-04 (`.rulings/` R-138),
    /// register `R-32`, driven through the engine rather than through
    /// `crate::devices` alone.
    ///
    /// The defect: every application declaring `display` was chowned
    /// `/dev/dri/card*` and `/dev/fb0`, so one handed a lease could open the
    /// card by path instead and take DRM master the instant the launcher's own
    /// dropped — and could write the framebuffer around the lease at any time.
    /// `ADR-036` §3.1's *"no compatibility path"* was untrue by two device
    /// nodes.
    #[test]
    fn a_lessee_is_granted_no_card_node_and_no_framebuffer() {
        let mut k = kernel();
        let d = kiosk_def(&mut k, "a", vec![Device::Display]);
        with_drm(&mut k);
        let mut s = showing(vec![d], "a");
        s.converge(&mut k);

        assert_eq!(k.spawned.len(), 1, "the lessee must still start");
        assert!(
            k.chowns.iter().any(|(p, _, _)| p == "/dev/dri/renderD128"),
            "the lessee was given no render node at all: {:?}",
            k.chowns
        );
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p.contains("card")),
            "a card node was handed to a lessee: {:?}",
            k.chowns
        );
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p == "/dev/fb0"),
            "the framebuffer was handed to a lessee: {:?}",
            k.chowns
        );
    }

    /// **The ARBITER still gets the card and the framebuffer**, and it is the
    /// declaration that decides — `connects = ["rivet-sysd:kiosk"]`, the same
    /// one `crate::main` chowns the kiosk socket by.
    ///
    /// Without this the narrowing would be a machine with nobody able to drive
    /// the screen, which is worse than the defect it repairs.
    ///
    /// **Not `showing`, and that is the fixture telling the truth about the
    /// arbiter.** `Supervisor::offered_displays` excludes anything that controls
    /// the kiosk — a launcher is not an item on its own menu — so an arbiter is
    /// never *selected*; it runs because it is enabled, like any other service.
    #[test]
    fn the_arbiter_of_the_screen_is_still_granted_the_card() {
        let mut k = kernel();
        let d = controlling_the_kiosk(kiosk_def(
            &mut k,
            crate::definitions::LAUNCHER_NAME,
            vec![Device::Display],
        ));
        with_drm(&mut k);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1, "the arbiter never started: {:?}", k.spawned);

        for node in ["/dev/dri/card0", "/dev/dri/card1", "/dev/fb0"] {
            assert!(
                k.chowns.iter().any(|(p, _, _)| p == node),
                "the arbiter was not granted {node}: {:?}",
                k.chowns
            );
        }
    }

    /// **The compositor is the arbiter, and the launcher beside it is not** —
    /// `ADR-058` §4, the change of 2026-09-20.
    ///
    /// This is the case the whole rule exists for and the one that was broken
    /// for an afternoon: a card carrying both, with `rivet-displayd` enabled.
    /// Under `.rulings/` R-138's rule the launcher was the arbiter and the
    /// compositor was a LESSEE — so the compositor was put in the offered set,
    /// the display gate disabled it, and it never started at all. The board
    /// reported `enabled: true, state: idle` and nothing in the journal said
    /// why, because a lessee that was not selected is not an error.
    #[test]
    fn the_compositor_takes_the_screen_from_the_launcher_that_used_to_hold_it() {
        let mut k = kernel();
        let compositor =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        let launcher = controlling_the_kiosk(kiosk_def(&mut k, "rivet-launcher", vec![]));
        let s = supervisor(vec![compositor.clone(), launcher.clone()]);

        assert_eq!(
            s.display_role(&compositor),
            rivet_core::service::DisplayRole::Arbiter,
            "the compositor must be the one program that may drive the screen"
        );
        assert_eq!(
            s.display_role(&launcher),
            rivet_core::service::DisplayRole::Lessee,
            "the launcher still selects applications and must NOT also take the device"
        );
    }

    /// **A second compositor takes nothing from the one the document names.**
    /// Offering `socket:wayland` is a request for the screen and never a grant
    /// of it: an application that declared it used to make two providers,
    /// and two providers meant nobody drew. One service holds the role, so a
    /// declaration can never hand two programs a way to take DRM master from
    /// each other (`ADR-015` §6 point 3).
    #[test]
    fn a_second_compositor_takes_nothing_from_the_named_one() {
        let mut k = kernel();
        let ours =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        let theirs =
            providing_the_compositor(kiosk_def(&mut k, "cosmic-comp", vec![Device::Display]));
        let s = supervisor(vec![ours.clone(), theirs.clone()]);

        assert_eq!(
            s.display_role(&ours),
            rivet_core::service::DisplayRole::Arbiter,
            "the image's compositor is the default the document did not override"
        );
        assert_eq!(
            s.display_role(&theirs),
            rivet_core::service::DisplayRole::Lessee
        );
    }

    /// **The document can name a different compositor**, which is how a
    /// desktop machine runs one: a document change and no supervisor change.
    #[test]
    fn the_document_can_name_a_different_compositor() {
        let mut k = kernel();
        let ours =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        let theirs =
            providing_the_compositor(kiosk_def(&mut k, "cosmic-comp", vec![Device::Display]));
        let mut s = supervisor(vec![ours.clone(), theirs.clone()]);
        s.set_display_authority(crate::document::DisplayAuthority {
            compositor: "cosmic-comp".to_string(),
            ..crate::document::DisplayAuthority::default()
        });

        assert_eq!(
            s.display_role(&theirs),
            rivet_core::service::DisplayRole::Arbiter
        );
        assert_eq!(
            s.display_role(&ours),
            rivet_core::service::DisplayRole::Lessee
        );
    }

    /// **A name without the declaration is nothing.** The document names the
    /// one service that may hold a role, and the service still has to ask for
    /// it in its manifest: naming an ordinary application as the compositor
    /// must not give it the card.
    #[test]
    fn a_named_service_that_does_not_declare_the_role_is_given_nothing() {
        let mut k = kernel();
        let app = kiosk_def(&mut k, "aura-renderer", vec![Device::Display]);
        let mut s = supervisor(vec![app.clone()]);
        s.set_display_authority(crate::document::DisplayAuthority {
            launcher: "aura-renderer".to_string(),
            compositor: "aura-renderer".to_string(),
        });
        assert_eq!(
            s.display_role(&app),
            rivet_core::service::DisplayRole::Lessee
        );
    }

    /// **A card that carries a compositor and leaves it disabled behaves
    /// exactly as the card before it.** `ADR-050` §2's tier 2 applied to the
    /// arbitration: the document decides, and until it does, `.rulings/`
    /// R-138's rule is still the one in force.
    ///
    /// Without this, shipping `rivet-displayd` on an image would silently take
    /// the screen away from every existing kiosk — a card that boots, converges
    /// and shows nothing.
    #[test]
    fn a_disabled_compositor_leaves_the_launcher_holding_the_screen() {
        let mut k = kernel();
        let mut compositor =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        compositor.enabled = false;
        let launcher = controlling_the_kiosk(kiosk_def(&mut k, "rivet-launcher", vec![]));
        let s = supervisor(vec![compositor.clone(), launcher.clone()]);

        assert_eq!(
            s.display_role(&launcher),
            rivet_core::service::DisplayRole::Arbiter,
            "an image carrying a disabled compositor must not change what a card does"
        );
        assert_eq!(
            s.display_role(&compositor),
            rivet_core::service::DisplayRole::Lessee
        );
    }

    /// **The compositor is not an item on the launcher's menu.** It declares a
    /// display like everything the launcher offers, so without the exclusion
    /// the first selection would disable the program holding the screen — and
    /// take every other application's surface with it.
    #[test]
    fn the_compositor_is_not_offered_as_something_to_select() {
        let mut k = kernel();
        let compositor =
            providing_the_compositor(kiosk_def(&mut k, "rivet-displayd", vec![Device::Display]));
        let app = kiosk_def(&mut k, "aura-renderer", vec![Device::Display]);
        let s = supervisor(vec![compositor, app]);

        assert_eq!(
            s.display_offered,
            vec!["aura-renderer".to_string()],
            "the compositor must not appear among the things a person can choose"
        );
    }

    /// **Applications claiming the kiosk that the document does not name are
    /// lessees**, however many of them there are. Neither is the launcher, so
    /// neither is given the card or the framebuffer.
    #[test]
    fn applications_claiming_the_kiosk_without_being_named_are_lessees() {
        let mut k = kernel();
        let a = controlling_the_kiosk(kiosk_def(&mut k, "a", vec![Device::Display]));
        let b = controlling_the_kiosk(kiosk_def(&mut k, "b", vec![Device::Display]));
        with_drm(&mut k);
        let mut s = supervisor(vec![a, b]);
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 2, "both should still run: {:?}", k.spawned);

        assert!(
            !k.chowns.iter().any(|(p, _, _)| p.contains("card")),
            "a card node was granted to an application the document does not name: {:?}",
            k.chowns
        );
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p == "/dev/fb0"),
            "the framebuffer was granted to an application the document does not name: {:?}",
            k.chowns
        );
    }

    /// **A second application claiming the launcher's role takes nothing
    /// from the launcher.** This was the denial of service: one more
    /// `connects = ["rivet-sysd:kiosk"]` on the machine and the launcher lost
    /// the card along with the kiosk socket. The document names the launcher,
    /// so the claim is refused and the launcher keeps both.
    #[test]
    fn a_second_launcher_claim_takes_nothing_from_the_named_launcher() {
        let mut k = kernel();
        let launcher = controlling_the_kiosk(kiosk_def(
            &mut k,
            crate::definitions::LAUNCHER_NAME,
            vec![Device::Display],
        ));
        let rogue = controlling_the_kiosk(kiosk_def(&mut k, "a-rogue", vec![Device::Display]));
        let rogue_uid = rogue.program.identity.expect("an application has one").uid;
        with_drm(&mut k);
        let mut s = supervisor(vec![launcher.clone(), rogue.clone()]);

        assert_eq!(
            s.display_role(&launcher),
            rivet_core::service::DisplayRole::Arbiter
        );
        assert_eq!(
            s.display_role(&rogue),
            rivet_core::service::DisplayRole::Lessee
        );

        s.converge(&mut k);
        assert!(
            k.chowns.iter().any(|(p, _, _)| p == "/dev/dri/card0"),
            "the named launcher was not granted the card: {:?}",
            k.chowns
        );
        assert!(
            !k.chowns
                .iter()
                .any(|(p, u, _)| p.contains("card") && *u == rogue_uid),
            "the claimant the document does not name was granted a card node: {:?}",
            k.chowns
        );
    }

    /// `input` expands to every `event*` the machine has, not to a fixed list —
    /// how many a board carries depends on what is plugged into it.
    #[test]
    fn declaring_input_grants_every_event_node_the_machine_has() {
        let mut k = kernel();
        let d = kiosk_def(&mut k, "a", vec![Device::Input]);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        for node in ["event0", "event1", "mice", "mouse0"] {
            let path = format!("/dev/input/{node}");
            assert!(
                k.chowns.iter().any(|(p, _, _)| *p == path),
                "{path} was not granted: {:?}",
                k.chowns
            );
        }
        // And nothing it did not declare.
        assert!(!k.chowns.iter().any(|(p, _, _)| p == "/dev/fb0"));
    }

    /// A class whose nodes are absent is not a refusal. `/dev/dri` is missing on
    /// every board built to date — the kernel builds DRM as a module and Rivet
    /// loads none (`PLAN.md` §103.12) — and an application asking for the
    /// display still starts and gets the framebuffer.
    #[test]
    fn a_device_class_whose_nodes_are_absent_still_starts_the_service() {
        let mut k = kernel();
        let d = kiosk_def(&mut k, "a", vec![Device::Display]);
        let mut s = showing(vec![d], "a");
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1);
        assert!(!k.chowns.iter().any(|(p, _, _)| p.starts_with("/dev/dri")));
    }

    /// Declaring nothing grants nothing — every application written before
    /// 2026-08-31, and most after.
    #[test]
    fn an_application_that_declares_no_device_is_granted_none() {
        let mut k = kernel();
        let d = kiosk_def(&mut k, "a", Vec::new());
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(k.chowns.iter().all(|(p, _, _)| !p.starts_with("/dev/")));
    }

    /// **The revoke.** §6.2 makes the grant per BOOT; this makes it per RUN,
    /// which is stricter — a stopped application must not keep the screen.
    /// A render node goes back to its shared form, `root:render`, which no uid
    /// owns and only a running service holding the group reaches.
    #[test]
    fn stopping_the_owner_puts_the_node_back_to_root() {
        let mut k = kernel();
        let mut d = kiosk_def(&mut k, "a", vec![Device::Display]);
        with_drm(&mut k);
        d.enabled = true;
        let mut s = showing(vec![d], "a");
        s.converge(&mut k);
        k.chowns.clear();

        s.set_application_enabled("a", false);
        s.converge(&mut k);
        k.mono_ms += 60_000;
        s.converge(&mut k);

        assert!(
            k.chowns
                .iter()
                .any(|(p, u, g)| p == "/dev/dri/renderD128"
                    && *u == 0
                    && *g == rivet_core::etc::RENDER_GID),
            "the node was never taken back: {:?}",
            k.chowns
        );
    }

    // ----------------------------------------------------------------------
    // Listening sockets — ADR-015 §5.2, ADR-017 §3.4, built 2026-08-31
    // ----------------------------------------------------------------------

    /// An application that declares `sockets`, with the signed package the
    /// launch check demands already in place.
    ///
    /// **`crate::verify` runs before the sockets are opened**, deliberately, so
    /// a refusal leaves nothing bound behind it. That ordering is why these
    /// tests need a really signed fixture rather than a stub — the same reason
    /// `a_notify_service_is_given_a_socket_in_its_environment` needs one.
    fn listening_def(k: &mut FakeKernel, name: &str, sockets: Vec<Socket>) -> ServiceDef {
        let mut d = application_def(name);
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        d.listen = sockets;
        let version_dir = d
            .program
            .path
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
        crate::verify::fixture::install_signed(k, name, &version_dir, &d.program.path);
        d
    }

    /// **A provider re-grants its endpoint every time it binds it.**
    ///
    /// A regression test for a defect found on a
    /// board rather than an invented case. `open_listeners` unlinks the old
    /// socket and binds a new one, root-owned `0600`; granting only when the
    /// CONSUMER started meant a provider restart silently revoked the grant.
    /// Both services stayed healthy, neither reported anything, and the screen
    /// froze because `aura-renderer` could no longer open a socket it had been
    /// connected to a moment earlier.
    ///
    /// The assertion is on the SECOND start specifically. A test that only
    /// checked the first would have passed against the broken build.
    #[test]
    fn a_provider_regrants_its_endpoint_when_it_rebinds() {
        use rivet_core::package::application::Endpoint;

        let mut k = kernel();
        let mut provider = listening_def(&mut k, "a-provider", Vec::new());
        provider.provides = vec![
            Endpoint::parse("provides", "socket:api").unwrap_or_else(|_| unreachable!()),
        ];
        provider.listen = vec![Socket::Unix("api".to_string())];

        let mut consumer = listening_def(&mut k, "b-consumer", Vec::new());
        consumer.connects = vec![
            Endpoint::parse("connects", "a-provider:api").unwrap_or_else(|_| unreachable!()),
        ];

        let consumer_identity = rivet_core::service::app_identity(
            &rivet_core::service::ServiceName::new("b-consumer")
                .unwrap_or_else(|| unreachable!("a service name")),
        );
        let socket = rivet_core::service::provided_socket_path(
            &rivet_core::service::ServiceName::new("a-provider")
                .unwrap_or_else(|| unreachable!("a service name")),
            "api",
        );

        let mut s = supervisor(vec![provider, consumer]);
        s.converge(&mut k);

        let granted = |k: &crate::testkernel::FakeKernel| {
            k.chowns
                .iter()
                .filter(|(path, uid, _)| {
                    path == &socket && *uid == consumer_identity.uid
                })
                .count()
        };
        assert!(
            granted(&k) >= 1,
            "the endpoint was never granted at all: {:?}",
            k.chowns
        );

        // The provider restarts and rebinds. Nothing about the consumer
        // changes: it is still running and still holds no new descriptor.
        k.chowns.clear();
        let Some(State::Started { pid, .. }) = s.state_of("a-provider") else {
            unreachable!("the provider is not running");
        };
        let pid = *pid;
        let _ = s.restart_service(&mut k, "a-provider", true);
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a-provider"), Some(State::Started { .. })),
            "the provider did not come back: {:?}",
            s.state_of("a-provider")
        );

        assert!(
            granted(&k) >= 1,
            "the provider rebound its socket and did NOT re-grant it to \
             b-consumer, so a running consumer can no longer open it: {:?}",
            k.chowns
        );
    }

    /// **A definition declaring a UDP socket gets a descriptor from the
    /// supervisor** — `PLAN.md` §150.1, 2026-09-06. `Socket::Udp` is new; this
    /// is the same assertion `a_service_that_declares_sockets_has_them_opened_
    /// in_manifest_order` already makes for `tcp` and `unix`, so a build that
    /// forgot to teach `open_listeners` about the third variant fails here
    /// rather than only at `parse_socket`.
    #[test]
    fn a_service_that_declares_a_udp_socket_gets_a_descriptor() {
        let mut k = kernel();
        let d = listening_def(&mut k, "a-resolver", vec![Socket::Udp(53)]);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        let (names, owner) = k
            .listeners_open
            .get("a-resolver")
            .expect("the udp socket was never opened");
        // **The manifest's socket first and the reserved health channel after
        // it** — `ADR-045` §3. The order is the assertion: an application
        // reading `LISTEN_FDS` positionally would be handed the wrong one if
        // the reserved entry were ever prepended.
        assert_eq!(
            names,
            &[
                "udp-53".to_string(),
                rivet_core::health::CHANNEL_NAME.to_string()
            ]
        );
        assert_eq!(
            owner,
            &Some(
                rivet_core::service::app_identity(
                    &rivet_core::service::ServiceName::new("a-resolver")
                        .unwrap_or_else(|| unreachable!("a service name"))
                )
                .uid
            ),
            "the socket must be opened for the identity the child will run as"
        );
        assert_eq!(k.spawned.len(), 1, "the service should have started");
    }

    /// **A proxied application's ports are bound on loopback, a `:direct` one
    /// on every address, and an unproxied application's on every address** —
    /// `ADR-046` §7, `.rulings/` R-188. The bind is the kernel's and the
    /// address is `Socket::bind_address`'s; this is the supervisor handing the
    /// definition's `proxied` through, and journalling what was bound.
    #[test]
    fn a_proxied_applications_ports_are_bound_on_loopback_unless_direct() {
        let mut k = kernel();
        let mut reliquary = listening_def(
            &mut k,
            "a-reliquary",
            vec![
                Socket::Tcp(8444),
                Socket::TcpDirect(8443),
                Socket::Udp(5354),
            ],
        );
        reliquary.proxied = true;
        let proxy = listening_def(&mut k, "b-proxy", vec![Socket::Tcp(443)]);
        let mut s = supervisor(vec![reliquary, proxy]);
        s.converge(&mut k);

        assert_eq!(
            k.listeners_bound.get("a-reliquary"),
            Some(&vec![
                "127.0.0.1:8444".to_string(),
                "0.0.0.0:8443".to_string(),
                "127.0.0.1:5354".to_string(),
            ]),
            "a proxied application's routed port must be loopback and its direct port every \
             address"
        );
        assert_eq!(
            k.listeners_bound.get("b-proxy"),
            Some(&vec!["0.0.0.0:443".to_string()]),
            "an application with no proxy-path must still be reachable from the network"
        );
        // The descriptor names are unchanged by the reach, so an application
        // looking up `tcp-8443` is handed the same socket it always was.
        let (names, _) = k.listeners_open.get("a-reliquary").expect("opened");
        assert_eq!(&names[..3], &["tcp-8444", "tcp-8443", "udp-5354"]);

        let said = |service: &str| {
            k.records
                .iter()
                .filter(|r| r.svc == service && r.msg.starts_with("this service's tcp and udp"))
                .map(|r| format!("{:?}", r.fields))
                .collect::<Vec<_>>()
        };
        let reliquary_said = said("a-reliquary");
        assert_eq!(reliquary_said.len(), 1, "{reliquary_said:?}");
        assert!(
            reliquary_said[0].contains("127.0.0.1:8444,0.0.0.0:8443,127.0.0.1:5354"),
            "{reliquary_said:?}"
        );
        assert!(said("b-proxy")[0].contains("0.0.0.0:443"), "{:?}", said("b-proxy"));
    }

    /// **The regrant property holds for UDP too, matching
    /// `a_provider_regrants_its_endpoint_when_it_rebinds`.** That test exists
    /// because a provider's Unix listener used to be re-bound on restart
    /// without the running consumer being told; the mechanism underneath both
    /// is the same one — `open_listeners` unconditionally closes whatever it
    /// held for a service before it opens anything new (`Kernel::open_listeners`,
    /// `cmd/rivet-sysd/src/kernel.rs`) — so a stale descriptor surviving a
    /// restart would be the identical defect wearing a UDP socket instead of a
    /// Unix one. This asserts the SECOND start specifically, for
    /// `a_provider_regrants_its_endpoint_when_it_rebinds`'s reason: a test that
    /// only checked the first start would pass against a kernel that opened the
    /// socket once and then silently kept serving the old descriptor forever.
    #[test]
    fn a_udp_listener_is_closed_and_reopened_when_its_service_restarts() {
        let mut k = kernel();
        let d = listening_def(&mut k, "a-resolver", vec![Socket::Udp(53)]);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(
            k.listeners_open.contains_key("a-resolver"),
            "the udp socket was never opened on the first start"
        );

        let Some(State::Started { pid, .. }) = s.state_of("a-resolver") else {
            unreachable!("the service is not running");
        };
        let pid = *pid;
        k.listeners_closed.clear();
        let _ = s.restart_service(&mut k, "a-resolver", true);
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a-resolver"), Some(State::Started { .. })),
            "the service did not come back: {:?}",
            s.state_of("a-resolver")
        );
        assert!(
            k.listeners_closed.iter().any(|n| n == "a-resolver"),
            "the old udp socket was never closed on restart: {:?}",
            k.listeners_closed
        );
        let (names, _) = k
            .listeners_open
            .get("a-resolver")
            .expect("the udp socket was not reopened after the restart");
        assert_eq!(
            names,
            &[
                "udp-53".to_string(),
                rivet_core::health::CHANNEL_NAME.to_string()
            ],
            "the service did not get a fresh udp descriptor after restarting"
        );
    }

    // -----------------------------------------------------------------------
    // The display selection — PLAN.md §104.40
    // -----------------------------------------------------------------------

    /// **`enabled` offers a screen; a selection gives one.** Two applications
    /// are enabled, one is selected, and exactly one runs.
    #[test]
    fn only_the_selected_display_runs_and_the_other_is_still_offered() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let b = kiosk_def(&mut k, "b", vec![Device::Display]);
        let mut s = showing(vec![a, b], "b");
        s.converge(&mut k);

        assert_eq!(k.spawned.len(), 1, "exactly one should run: {:?}", k.spawned);
        assert!(
            k.spawned.iter().any(|(name, _)| name == "b"),
            "the selected one did not run: {:?}",
            k.spawned
        );
        // And `a` is still OFFERED — the thing that used to be destroyed. It can
        // be chosen without the document being touched.
        assert!(s.select_display("a").is_ok(), "`a` stopped being offerable");
    }

    /// **`get status` must agree with the document about `enabled`** —
    /// `PLAN.md` §112.4, found on the maintainer's board and fixed 2026-09-02.
    ///
    /// The board's document said `[services.kiosk-demo] enabled = true` and
    /// `get status` answered `enabled: false`. Both were "right": the operator
    /// had enabled it, and `apply_display_gate` had overwritten `def.enabled`
    /// with *is this the selected one*, which is what convergence needs. The
    /// status frame reported the second, and the operator was reading it
    /// against the first.
    ///
    /// This asserts the shape the board actually had — two display
    /// applications enabled, one selected:
    ///
    /// | | `enabled` | runs |
    /// | :-- | :-- | :-- |
    /// | the selected one | `true` | yes |
    /// | the offered-but-not-selected one | **`true`** | no |
    ///
    /// The second row is the whole test. Before the fix it read `false`, which
    /// told an operator their document had not taken.
    #[test]
    fn an_offered_display_application_reports_the_enabled_the_document_asked_for() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let b = kiosk_def(&mut k, "b", vec![Device::Display]);
        let s = showing(vec![a, b], "b");

        let status = s.status();
        let of = |name: &str| {
            status
                .services
                .iter()
                .find(|x| x.name == name)
                .unwrap_or_else(|| panic!("{name} is not in the status"))
        };
        assert!(of("b").enabled, "the selected application must report enabled");
        assert!(
            of("a").enabled,
            "an OFFERED display application must report the `enabled` the document asked for. \
             Reporting false here is the board defect PLAN.md §112.4 recorded: the operator \
             wrote `enabled = true`, the machine answered `false`, and nothing said why"
        );
    }

    /// The gate itself is untouched: `def.enabled` still means *should this be
    /// running*, which is what convergence reads. **Only the REPORT changed**,
    /// and this is what says so — it fails if the fix was made by weakening the
    /// gate instead of by telling the truth in the status.
    #[test]
    fn the_display_gate_still_decides_what_actually_runs() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let b = kiosk_def(&mut k, "b", vec![Device::Display]);
        let mut s = showing(vec![a, b], "b");
        s.converge(&mut k);

        assert_eq!(k.spawned.len(), 1, "exactly one should run: {:?}", k.spawned);
        assert!(
            k.spawned.iter().any(|(name, _)| name == "b"),
            "the selected one did not run: {:?}",
            k.spawned
        );
    }

    /// A service that declares no display is unaffected in either direction:
    /// its `enabled` is the document's and was never overwritten, so the new
    /// rule must not reach it.
    #[test]
    fn a_service_without_a_display_reports_exactly_what_the_document_said() {
        let mut k = kernel();
        let mut plain = kiosk_def(&mut k, "plain", vec![]);
        plain.enabled = false;
        let s = supervisor(vec![plain]);
        assert!(
            !s.status().services[0].enabled,
            "a disabled ordinary service must not be reported as enabled by the display rule"
        );
    }

    /// **The selection survives a reboot.** A machine that comes up with a
    /// stored selection naming an offered application restores it, and the gate
    /// then enables that application — which is what makes a kiosk resume its
    /// screen instead of forgetting it on every boot. Before this,
    /// `display_selection` was `None` at construction and nothing wrote it
    /// down, so `apply_display_gate` held every offered display application off
    /// after any restart. `crate::kiosk_selection` is the repair.
    #[test]
    fn a_stored_selection_naming_an_offered_application_is_restored_on_boot() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        // What the previous boot wrote to /state before the power cut.
        k.files_mut().insert(
            crate::kiosk_selection::path(),
            Script::always(Ok("a\n".to_string())),
        );
        let mut s = supervisor(vec![a]);
        // The exact state a reboot lands in: nothing selected, and the gate has
        // disabled the offered application.
        assert_eq!(s.display_selection(), None);
        assert!(!s.services[0].def.enabled, "the gate should start it disabled");

        crate::kiosk_selection::restore(&mut k, &mut s);

        assert_eq!(s.display_selection(), Some("a"), "the selection was not restored");
        assert!(
            s.services[0].def.enabled,
            "the gate did not enable the restored application, so the kiosk stays blank"
        );
    }

    /// **A stored selection for an application that is no longer offered is
    /// dropped.** Uninstall the application, or stop enabling it, and the stale
    /// file must not hold the machine to a selection that can never take:
    /// `select_display` refuses it, the file is cleared, and the machine comes
    /// up with nothing selected — without error and without refusing to boot.
    #[test]
    fn a_stored_selection_for_an_unoffered_application_is_cleared_without_error() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        // The stored name is an application this machine does not offer.
        k.files_mut().insert(
            crate::kiosk_selection::path(),
            Script::always(Ok("gone\n".to_string())),
        );
        let mut s = supervisor(vec![a]);

        crate::kiosk_selection::restore(&mut k, &mut s);

        assert_eq!(s.display_selection(), None, "a stale selection was restored");
        assert!(
            k.removed.contains(&crate::kiosk_selection::path()),
            "the stale selection file was not cleared: {:?}",
            k.removed
        );
        assert_eq!(k.contents(&crate::kiosk_selection::path()), None);
    }

    /// **A display application enabled AFTER boot can be selected.**
    /// `PLAN.md` §106.5, found on hardware 2026-09-01.
    ///
    /// The offered set was read once, in the constructor — the one moment
    /// `enabled` means *offered* rather than *running*. Nothing read it again,
    /// so an application the operator enabled later was recorded by
    /// `set_enabled`, narrowed straight back by `apply_display_gate`, and never
    /// captured in between. The launcher offered it; the supervisor refused it
    /// and named a set computed before the operator had spoken.
    ///
    /// This asserts the property rather than the mechanism: enable it, refresh,
    /// and it is selectable. It fails without `refresh_display_offered`.
    #[test]
    fn a_display_enabled_after_boot_becomes_offerable() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let mut b = kiosk_def(&mut k, "b", vec![Device::Display]);
        // `b` is installed and the document does NOT enable it — the state of
        // every application before an operator says otherwise.
        b.enabled = false;
        let mut s = showing(vec![a, b], "a");
        s.converge(&mut k);

        assert!(
            s.select_display("b").is_err(),
            "the fixture is wrong: `b` should not be offerable before it is enabled"
        );

        // The operator enables it — which is what a document reload does.
        s.set_enabled("b", true);
        s.refresh_display_offered();

        assert!(
            s.select_display("b").is_ok(),
            "`b` was enabled and still could not be selected: the offered set was \
             not re-read, so the document's answer was narrowed away before anything \
             recorded it"
        );
    }

    /// Choosing another application stops the one that held the screen. One
    /// owner at a time is `ADR-015` §6.2's rule and this is where it is kept.
    #[test]
    fn selecting_another_display_stops_the_one_that_held_it() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let b = kiosk_def(&mut k, "b", vec![Device::Display]);
        let mut s = showing(vec![a, b], "a");
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));

        s.select_display("b").expect("b is offered");
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Stopping { .. })),
            "the previous holder was not stopped: {:?}",
            s.state_of("a")
        );
    }

    /// Returning to the menu stops whatever held the screen and starts nothing.
    #[test]
    fn returning_the_display_stops_the_holder() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let mut s = showing(vec![a], "a");
        s.converge(&mut k);

        let said = s.return_display(&mut k);
        assert!(said.contains('a'), "{said}");
        assert_eq!(s.display_selection(), None);
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Stopping { .. })));
    }

    /// **A withdrawn selection takes its display channel with it** —
    /// `ADR-042` §8.4, measured on `rivet-01` on 2026-09-05.
    ///
    /// `crate::main`'s `grant_display` writes the lease into the channel
    /// **before** the spawn, so that it is buffered and waiting when the child
    /// first reads. When the application is not started on that pass — one that
    /// has given up, or is in backoff — the message stays in the socket buffer
    /// while the launcher, seeing nothing running, revokes the lease. The board
    /// then started the application forty seconds later, on a document reload,
    /// and handed it that revoked descriptor:
    ///
    /// ```text
    /// aura-renderer: no display was handed over: a display was handed over and could not be
    /// driven: a leased display: no connected output with a mode — is a cable in?
    /// ```
    ///
    /// A revoked lease answers every ioctl and describes a display with no
    /// connectors, so the sentence named a cable that had not moved and the
    /// whole thing read as a confinement regression. It was not one: the same
    /// binary drew at 1920x1080 eleven seconds afterwards, under the same
    /// Landlock ruleset.
    ///
    /// **This is the half of the repair the supervisor owns.** The other half is
    /// the launcher telling it — `cmd/rivet-launcher`'s health poll now asks
    /// `return` when it abandons a selection, where before it revoked the lease
    /// and said nothing.
    #[test]
    fn returning_the_display_closes_the_channel_the_lease_was_written_to() {
        let mut k = kernel();
        let a = kiosk_def(&mut k, "a", vec![Device::Display]);
        let mut s = showing(vec![a], "a");
        s.converge(&mut k);
        // Nothing has stopped, so `release` has not run and this is the only
        // thing that could have closed it.
        assert!(
            k.displays_closed.is_empty(),
            "something closed the channel before the selection was withdrawn: {:?}",
            k.displays_closed
        );

        s.return_display(&mut k);

        assert!(
            k.displays_closed.iter().any(|n| n == "a"),
            "the display channel outlived the selection, so a descriptor written to it is \
             still waiting for whatever starts next: closed = {:?}",
            k.displays_closed
        );
    }

    /// An application the document did not enable is not offerable, however it
    /// is asked for. The refusal names what IS offered, because a launcher that
    /// asked for the wrong name has nothing else to go on.
    #[test]
    fn an_application_that_was_not_enabled_cannot_be_selected() {
        let mut k = kernel();
        let mut a = kiosk_def(&mut k, "a", vec![Device::Display]);
        a.enabled = false;
        let b = kiosk_def(&mut k, "b", vec![Device::Display]);
        let mut s = supervisor(vec![a, b]);

        let Err(why) = s.select_display("a") else {
            panic!("a disabled application was selected");
        };
        assert!(why.contains('b'), "the refusal should name what is offered: {why}");
        assert!(s.select_display("nonesuch").is_err());
    }

    /// **The launcher survives its own selection**, which it would not have.
    ///
    /// It declares a display like everything it offers, so a gate that included
    /// it would disable and stop the process that made the selection — and the
    /// machine would have no way back to a menu. Found by reading the board's
    /// status before a person hit it.
    #[test]
    fn the_kiosk_controller_is_not_one_of_its_own_choices() {
        let mut k = kernel();
        let mut launcher = kiosk_def(&mut k, "a-launcher", vec![Device::Display]);
        launcher.connects = vec![
            rivet_core::package::application::Endpoint::parse(
                "connects",
                &format!(
                    "{}:{}",
                    rivet_core::kiosk::PROVIDER,
                    rivet_core::kiosk::ENDPOINT
                ),
            )
            .unwrap_or_else(|_| unreachable!("a valid endpoint")),
        ];
        let app = kiosk_def(&mut k, "b-app", vec![Device::Display]);
        let mut s = supervisor(vec![launcher, app]);

        // It is not offerable...
        assert!(
            s.select_display("a-launcher").is_err(),
            "the launcher offered itself a screen"
        );
        // ...and selecting something else leaves it running.
        s.select_display("b-app").expect("b-app is offered");
        s.converge(&mut k);
        assert!(
            k.spawned.iter().any(|(name, _)| name == "a-launcher"),
            "the launcher was stopped by a selection it made: {:?}",
            k.spawned
        );
    }

    /// A daemon is not on a screen, so the gate never touches its `enabled`.
    /// The failure this prevents is a display selection silently stopping
    /// `rivet-mcpd` and taking the operator's way in with it.
    #[test]
    fn the_gate_does_not_touch_a_service_that_declares_no_display() {
        let mut k = kernel();
        let daemon = def("a-daemon");
        let screen = kiosk_def(&mut k, "b-screen", vec![Device::Display]);
        let mut s = showing(vec![daemon, screen], "b-screen");
        s.converge(&mut k);

        assert!(
            k.spawned.iter().any(|(name, _)| name == "a-daemon"),
            "the daemon was stopped by a display selection: {:?}",
            k.spawned
        );
        s.return_display(&mut k);
        s.converge(&mut k);
        assert!(
            !matches!(s.state_of("a-daemon"), Some(State::Stopping { .. })),
            "returning the display stopped a daemon"
        );
    }

    /// The sockets are opened **before** the spawn, in the manifest's order,
    /// and the order is the assertion: `LISTEN_FDS` hands descriptors over
    /// positionally, so a reordering would give an application the wrong socket
    /// with nothing anywhere reporting it.
    #[test]
    fn a_service_that_declares_sockets_has_them_opened_in_manifest_order() {
        let mut k = kernel();
        let d = listening_def(
            &mut k,
            "a",
            vec![Socket::Unix("api".to_string()), Socket::Tcp(8080)],
        );
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        let (names, owner) = k.listeners_open.get("a").expect("sockets were not opened");
        assert_eq!(
            names,
            &[
                "api".to_string(),
                "tcp-8080".to_string(),
                // **Last, always** — `ADR-045` §3. Appending is what keeps
                // every index a manifest implies where it was.
                rivet_core::health::CHANNEL_NAME.to_string()
            ]
        );
        // **No token may contain the separator.** `LISTEN_FDNAMES` is
        // colon-joined, and the manifest spells these `unix:api` and `tcp:8080`
        // — so using that spelling would hand an application four names for two
        // sockets. A board reported exactly that before this was fixed.
        assert!(
            names.iter().all(|token| !token.contains(':')),
            "a colon in an FDNAME makes the list ambiguous: {names:?}"
        );
        assert_eq!(
            owner,
            &Some(rivet_core::service::app_identity(
                &rivet_core::service::ServiceName::new("a")
                    .unwrap_or_else(|| unreachable!("\"a\" is a service name"))
            )
            .uid),
            "the socket must be opened for the identity the child will run as"
        );
        assert_eq!(k.spawned.len(), 1, "the service should have started");
    }

    /// `LISTEN_FDNAMES` travels the ordinary way because it depends on the
    /// manifest and not on the pid. `LISTEN_FDS` and `LISTEN_PID` deliberately
    /// do NOT appear here — they are set in the child, and `Kernel::spawn` says
    /// why.
    #[test]
    fn the_socket_names_reach_the_child_and_the_pid_bound_ones_do_not() {
        let mut k = kernel();
        let d = listening_def(
            &mut k,
            "a",
            vec![Socket::Unix("api".to_string()), Socket::Tcp(8080)],
        );
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        let (_, program) = &k.spawned[0];
        assert!(
            program
                .env
                .iter()
                .any(|(key, value)| key == "LISTEN_FDNAMES" && value == "api:tcp-8080:rivet-health"),
            "{:?}",
            program.env
        );
        for absent in ["LISTEN_FDS", "LISTEN_PID"] {
            assert!(
                !program.env.iter().any(|(key, _)| key == absent),
                "{absent} must be set in the child, not handed through Program::env"
            );
        }
    }

    /// ~~A service with no `listen` opens nothing.~~ **An APPLICATION with no
    /// `listen` is handed exactly one socket, and it is the health channel** —
    /// `ADR-045` §3, 2026-09-08: *the manifest does not declare it; every
    /// application gets one*.
    ///
    /// This is the case that says the reserved entry does not depend on a
    /// manifest having asked for anything, which is the overwhelmingly common
    /// application and the one the whole poll has to work for.
    #[test]
    fn an_application_that_declares_no_socket_is_still_handed_its_health_channel() {
        let mut k = kernel();
        let mut d = application_def("a");
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        crate::verify::fixture::install_signed(&mut k, "a", "", &d.program.path);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let (names, _) = k
            .listeners_open
            .get("a")
            .expect("an application is handed a health channel whether or not it asked");
        assert_eq!(names, &[rivet_core::health::CHANNEL_NAME.to_string()]);
        let (_, program) = &k.spawned[0];
        assert!(
            program
                .env
                .iter()
                .any(|(key, value)| key == "LISTEN_FDNAMES" && value == "rivet-health"),
            "{:?}",
            program.env
        );
    }

    /// **An IMAGE service is handed neither**, and that is a decision rather
    /// than an omission — `SPEC-03` is the contract with an *application*.
    ///
    /// Image services are compiled into this binary, are already watched by
    /// pid and cgroup, and none of them speaks this protocol. A
    /// socket bound for them would be one nothing ever accepts on, followed by
    /// a warning per boot saying so — a report about a machine that is fine.
    #[test]
    fn an_image_service_is_handed_no_health_channel_and_is_never_asked() {
        let mut k = kernel();
        let d = def("rivet-netd");
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(
            k.listeners_open.is_empty(),
            "an image service was given a health channel: {:?}",
            k.listeners_open
        );
        // And the poll never reaches it, however long the machine runs.
        k.mono_ms += rivet_core::health::INTERVAL_MS * 4;
        s.converge(&mut k);
        assert!(
            k.health_asked.is_empty(),
            "an image service was asked how it is: {:?}",
            k.health_asked
        );
    }

    // -----------------------------------------------------------------------
    // ADR-045: the supervisor ASKS.
    // -----------------------------------------------------------------------

    /// A running application, so the health tests all start from the same
    /// place.
    fn running_application(k: &mut FakeKernel, name: &str) -> Supervisor {
        let mut d = application_def(name);
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        crate::verify::fixture::install_signed(k, name, "", &d.program.path);
        let mut s = supervisor(vec![d]);
        s.converge(k);
        assert!(
            matches!(s.state_of(name), Some(State::Started { .. })),
            "the fixture did not start: {:?}",
            s.state_of(name)
        );
        s
    }

    fn health_of(s: &Supervisor, name: &str) -> rivet_core::health::State {
        s.status()
            .services
            .iter()
            .find(|svc| svc.name == name)
            .map_or_else(
                || unreachable!("{name} is not in the status"),
                |svc| svc.health,
            )
    }

    /// **The facts answer and `get status` are one view of one machine** —
    /// `ADR-057`, `ADR-011`.
    ///
    /// The property, asserted rather than argued: a dashboard on the machine's
    /// own screen and an operator asking over the control plane read the same
    /// derivation, so the machine cannot tell them two different stories about
    /// what is running on it.
    #[test]
    fn the_facts_answer_is_the_view_get_status_gives() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=degraded one sensor is quiet");

        let status = s.status();
        let facts = s.facts(&mut k);

        assert_eq!(facts.phase, status.phase);
        assert_eq!(facts.machine, status.machine);
        assert_eq!(facts.mode, status.kiosk.mode);
        assert_eq!(facts.version, status.version);
        assert_eq!(facts.services.len(), status.services.len());
        for (fact, svc) in facts.services.iter().zip(status.services.iter()) {
            assert_eq!(fact.name, svc.name);
            assert_eq!(fact.state, svc.state);
            assert_eq!(fact.health, svc.health);
            assert_eq!(fact.enabled, svc.enabled);
            assert_eq!(fact.restarts, svc.restarts);
            assert_eq!(fact.pid, svc.pid.and_then(|p| u32::try_from(p).ok()));
        }
    }

    /// **A health word reaches a screen with its own age beside it** —
    /// `ADR-057` §3.
    ///
    /// The supervisor polls every thirty seconds, so this is the one field a
    /// dashboard is most likely to draw as live when it is half a minute old.
    /// The age is measured from the ANSWER, which is why it is stored rather
    /// than derived from when the next question is due.
    #[test]
    fn a_health_word_carries_how_old_it_is() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");

        let before = s.facts(&mut k);
        let svc = before
            .services
            .iter()
            .find(|f| f.name == "a")
            .unwrap_or_else(|| unreachable!("the service is in the answer"));
        assert_eq!(
            svc.health_age_ms, None,
            "nothing has answered, so there is no age — and 0 would read as `it answered this \
             instant`"
        );

        s.note_answered(&mut k, "a", "HEALTH=healthy");
        k.mono_ms += 12_000;

        let after = s.facts(&mut k);
        let svc = after
            .services
            .iter()
            .find(|f| f.name == "a")
            .unwrap_or_else(|| unreachable!("the service is in the answer"));
        assert_eq!(svc.health, rivet_core::health::State::Healthy);
        assert_eq!(svc.health_age_ms, Some(12_000));
    }

    /// **A restarted service does not carry the old process's age**, any more
    /// than it carries the old process's opinion.
    #[test]
    fn a_restart_clears_the_age_with_the_word_it_belonged_to() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=healthy");
        k.mono_ms += 5_000;
        assert!(s.facts(&mut k).services[0].health_age_ms.is_some());

        // A restart: the process that held that opinion is gone, and the one
        // that replaces it has not said anything yet.
        let _ = s.restart_service(&mut k, "a", true);
        s.converge(&mut k);

        let facts = s.facts(&mut k);
        let svc = facts
            .services
            .iter()
            .find(|f| f.name == "a")
            .unwrap_or_else(|| unreachable!("the service is in the answer"));
        assert_eq!(svc.health, rivet_core::health::State::Unknown);
        assert_eq!(
            svc.health_age_ms, None,
            "an age outlived the word it was the age of, so a screen would draw a dead \
             process's opinion with a live-looking timestamp"
        );
    }

    /// **The two boot-fixed facts are what the supervisor was TOLD** —
    /// `ADR-011`, and `ADR-057` §4's rule that nothing here makes the
    /// supervisor go and look.
    #[test]
    fn the_machines_name_and_slot_are_handed_over_and_not_derived() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");

        let untold = s.facts(&mut k);
        assert_eq!(untold.hostname, None, "a supervisor nobody told says so");
        assert_eq!(untold.slot, None);

        s.set_machine_identity(Some("rivet-06f5c".to_string()), Some("b".to_string()));
        let told = s.facts(&mut k);
        assert_eq!(told.hostname.as_deref(), Some("rivet-06f5c"));
        assert_eq!(told.slot.as_deref(), Some("b"));
    }

    /// **A supervisor that is still answering and has stopped turning is a
    /// drawable state**, and it is not the same thing as silence.
    ///
    /// The age is measured from the last convergence pass, so an answer that
    /// arrives carrying a five-hour-old view says something no timeout can.
    #[test]
    fn the_view_carries_how_long_ago_the_supervisor_last_turned() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.converge(&mut k);
        assert!(s.facts(&mut k).view_age_ms < 1_000);

        k.mono_ms += 5 * 60 * 60 * 1_000;
        assert_eq!(s.facts(&mut k).view_age_ms, 5 * 60 * 60 * 1_000);

        s.converge(&mut k);
        assert!(
            s.facts(&mut k).view_age_ms < 1_000,
            "a pass ran and the view is still reported as five hours old"
        );
    }

    /// **A service that has not been asked yet is `unknown`, never `healthy`**
    /// — `ADR-045` §4, and it is the default the whole design rests on.
    #[test]
    fn a_service_with_no_answer_on_record_is_unknown() {
        let mut k = kernel();
        let s = running_application(&mut k, "a");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
    }

    /// **The supervisor asks, and it asks on ITS cadence** — `ADR-045` §3.
    ///
    /// Not in the pass that started the service, once shortly after, and then
    /// once per interval — never twice in a pass and never in a tight loop.
    #[test]
    fn the_supervisor_asks_on_its_own_cadence_and_not_in_a_loop() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        assert!(
            k.health_asked.is_empty(),
            "a service was asked in the pass that started it"
        );

        // Several passes inside the first grace period: still nothing.
        for _ in 0..5 {
            s.converge(&mut k);
        }
        assert!(k.health_asked.is_empty(), "{:?}", k.health_asked);

        k.mono_ms += rivet_core::health::PATIENCE_MS;
        s.converge(&mut k);
        assert_eq!(k.health_asked, vec!["a".to_string()]);

        // **Ten more passes at the same instant ask nothing more.** This is the
        // tight-retry loop the ADR forbids, as an assertion.
        for _ in 0..10 {
            s.converge(&mut k);
        }
        assert_eq!(k.health_asked.len(), 1, "{:?}", k.health_asked);

        k.mono_ms += rivet_core::health::INTERVAL_MS;
        s.converge(&mut k);
        assert_eq!(k.health_asked.len(), 2, "{:?}", k.health_asked);
    }

    /// **`ADR-045` §11 step 1**, in the supervisor: it answers `degraded`
    /// with a sentence, and `get status` says so.
    #[test]
    fn an_application_that_answers_degraded_is_reported_degraded_with_its_caption() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=degraded one sensor is quiet");

        let status = s.status();
        let svc = status
            .services
            .iter()
            .find(|svc| svc.name == "a")
            .unwrap_or_else(|| unreachable!("the service is in the status"));
        assert_eq!(svc.health, rivet_core::health::State::Degraded);
        assert_eq!(svc.caption.as_deref(), Some("one sensor is quiet"));
        // Not critical, so the MACHINE is still healthy — `ADR-045` §4's
        // first stated consequence.
        assert_eq!(status.machine, rivet_core::health::State::Healthy);
    }

    /// **`ADR-045` §11 step 1a — THE test the telling design could not
    /// pass.**
    ///
    /// A wedged application answered `healthy` a moment ago and then stopped
    /// answering. Its process is still there, its state is still `started`, and
    /// its restart count is still zero — so every signal a telling design has
    /// says it is fine. This asserts that the supervisor reports it as
    /// `unknown` rather than as the last thing it said.
    #[test]
    fn an_application_that_stops_answering_is_unknown_and_not_its_last_healthy_word() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=healthy");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Healthy);

        // It wedges. The next question times out.
        s.note_unanswered(&mut k, "a", "no answer within 2000 ms");

        assert_eq!(
            health_of(&s, "a"),
            rivet_core::health::State::Unknown,
            "a wedged application was reported as the last thing it said"
        );
        let status = s.status();
        let svc = status
            .services
            .iter()
            .find(|svc| svc.name == "a")
            .unwrap_or_else(|| unreachable!("the service is in the status"));
        // Every signal the telling design had still says it is fine, which is
        // the whole point of the case.
        assert_eq!(svc.state, "started");
        assert!(svc.pid.is_some());
        assert_eq!(svc.restarts, 0);
        // And the caption goes with the state it described rather than
        // outliving it.
        assert_eq!(svc.caption, None);
    }

    /// **A `critical` service that stops answering takes the machine with
    /// it** — `ADR-045` §4, `unknown` above `degraded`.
    #[test]
    fn a_critical_service_that_stops_answering_makes_the_machine_unknown() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=healthy");
        assert_eq!(s.status().machine, rivet_core::health::State::Healthy);

        // The operator marks it critical — the document's word, applied to the
        // definition exactly as `crate::document` applies it.
        s.services[0].def.critical = true;
        s.note_unanswered(&mut k, "a", "no answer within 2000 ms");
        assert_eq!(s.status().machine, rivet_core::health::State::Unknown);
    }

    /// **A service that answers again stops being unknown.** Silence is a fact
    /// about the last question, not a latch — a service that recovers must be
    /// reportable as recovered.
    #[test]
    fn an_application_that_answers_again_is_no_longer_unknown() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_unanswered(&mut k, "a", "no answer within 2000 ms");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
        s.note_answered(&mut k, "a", "HEALTH=healthy");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Healthy);
    }

    /// **An answer this build cannot read leaves the service `unknown`**, and
    /// says so — `ADR-045` §3's second outcome. It is not silence and it is
    /// not a state; it is a program answering a question it did not understand.
    #[test]
    fn an_answer_outside_the_vocabulary_is_reported_and_leaves_the_service_unknown() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=wobbly");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
        s.note_answered(&mut k, "a", "I am fine thanks");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
    }

    /// **A restart discards the previous process's opinion of itself.**
    ///
    /// Without this a service that answered `healthy`, crashed, and came back
    /// would be reported `healthy` on the strength of a process that no longer
    /// exists — which is the same class of defect as the wedged case and
    /// arrives by a different road.
    #[test]
    fn a_restarted_application_does_not_inherit_the_previous_runs_health() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        s.note_answered(&mut k, "a", "HEALTH=healthy");
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Healthy);

        let Some(State::Started { pid, .. }) = s.state_of("a") else {
            unreachable!("the fixture is running");
        };
        let pid = *pid;
        k.exits.push((pid, ExitStatus::Signalled(9)));
        s.reap(&mut k);
        // **The restart path, which releases nothing.** `restart = "always"`
        // sends this through `schedule_retry`, so the clearing has to be on the
        // START edge or the dead process's word survives into the live one.
        k.mono_ms += 60_000;
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "the fixture did not come back: {:?}",
            s.state_of("a")
        );
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
    }

    /// **`HEALTH=` on `NOTIFY_SOCKET` moves the next question forward and
    /// carries no state** — `ADR-045` §3.
    ///
    /// The push says *ask me now*. If it set the state instead, an application
    /// that pushed `healthy` once at startup would have declared itself
    /// permanently healthy — the exact power §3.1.1 takes away.
    #[test]
    fn a_health_push_asks_for_a_poll_and_does_not_set_the_state() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        // Well inside the interval: nothing would be asked on its own.
        k.mono_ms += rivet_core::health::PATIENCE_MS;
        s.converge(&mut k);
        assert_eq!(k.health_asked.len(), 1);
        s.converge(&mut k);
        assert_eq!(k.health_asked.len(), 1, "the interval was not respected");

        s.note_health_hint(&mut k, "a");
        s.converge(&mut k);
        assert_eq!(
            k.health_asked.len(),
            2,
            "an `ask me now` did not bring the next question forward"
        );
        // And it said nothing about the state.
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
    }

    /// **An ask that fails outright is an answer, not a retry.** The socket is
    /// not there, so the service is not answering — recorded and left until the
    /// next interval rather than tried again on the very next pass.
    #[test]
    fn an_ask_that_cannot_reach_the_socket_is_reported_as_not_answering() {
        let mut k = kernel();
        let mut s = running_application(&mut k, "a");
        k.ask_health_error = Some(Errno(libc::ECONNREFUSED));
        k.mono_ms += rivet_core::health::PATIENCE_MS;
        s.converge(&mut k);
        assert_eq!(health_of(&s, "a"), rivet_core::health::State::Unknown);
        // And not again until the interval is up.
        s.converge(&mut k);
        assert_eq!(k.health_asked.len(), 0, "the failing ask never reached the kernel");
    }

    /// **The sleep is shortened by the next question**, or a supervisor with
    /// nothing else pending never asks one — `next_timeout_ms` is what makes
    /// the cadence real rather than a field nobody reads.
    #[test]
    fn the_next_health_question_shortens_the_sleep() {
        let mut k = kernel();
        let s = running_application(&mut k, "a");
        let now = k.mono_ms;
        assert_eq!(
            s.next_timeout_ms(now),
            Some(rivet_core::health::PATIENCE_MS),
            "a supervisor with one running application and nothing else pending must still wake \
             up to ask it"
        );
    }

    /// **A socket that cannot be opened is a service that does not start.**
    ///
    /// The alternative is worse than a failure: an application told to expect
    /// descriptors and given none blocks for ever on `accept`, which reaches an
    /// operator as a service that is *running* and does nothing.
    #[test]
    fn a_socket_that_cannot_be_opened_stops_the_service_rather_than_starting_it_blind() {
        let mut k = kernel();
        k.listener_error = Some(Errno(libc::EADDRINUSE));
        let d = listening_def(&mut k, "a", vec![Socket::Tcp(8080)]);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert!(k.spawned.is_empty(), "it must not have been started");
        assert_eq!(s.services()[0].state, State::Failed);
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.contains("could not open them")),
            "the refusal must name what happened"
        );
    }

    /// Stopping closes them, beside the notify socket. A stopped service that
    /// held a port would be a port bound by nothing.
    #[test]
    fn stopping_a_service_closes_its_sockets() {
        let mut k = kernel();
        let mut d = listening_def(&mut k, "a", vec![Socket::Tcp(8080)]);
        d.enabled = true;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(k.listeners_open.contains_key("a"));

        s.set_application_enabled("a", false);
        s.converge(&mut k);
        // The stop is signalled first and completes when the child is reaped;
        // drive it to the end.
        k.mono_ms += 60_000;
        s.converge(&mut k);
        assert!(
            k.listeners_closed.iter().any(|s| s == "a"),
            "close_listeners was never called: {:?}",
            k.listeners_closed
        );
    }

    // ----------------------------------------------------------------------
    // The exit edge — `PLAN.md` §123, Wave 2.3
    // ----------------------------------------------------------------------

    /// **A service that finishes by itself gives back what it held.**
    ///
    /// Until 2026-09-03 only the supervisor's own `stop` released anything, so
    /// a `oneshot` that ran to completion left its listening socket open in
    /// this process for the whole boot — bound by a program that no longer
    /// exists, and invisible, because nothing said so.
    #[test]
    fn a_service_that_exits_for_good_releases_its_sockets_and_its_display_channel() {
        let mut k = kernel();
        let mut d = listening_def(&mut k, "a", vec![Socket::Tcp(8080)]);
        d.enabled = true;
        // `Never`: it runs once and is `Done` when it ends, which is the whole
        // point — nothing is going to reopen these for it.
        d.restart = RestartPolicy::Never;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(k.listeners_open.contains_key("a"), "it must have been given its socket");

        let pid = match s.state_of("a") {
            Some(State::Started { pid, .. }) => *pid,
            other => unreachable!("{other:?}"),
        };
        k.exits.push((pid, ExitStatus::Exited(0)));
        k.mono_ms += 1_000;
        s.reap(&mut k);
        s.converge(&mut k);

        assert_eq!(s.services()[0].state, State::Done);
        assert!(
            k.listeners_closed.iter().any(|n| n == "a"),
            "the socket is still open in this supervisor: closed = {:?}",
            k.listeners_closed
        );
        assert!(
            k.displays_closed.iter().any(|n| n == "a"),
            "the display channel is still open: closed = {:?}",
            k.displays_closed
        );
    }

    /// **A display owner that exits for good gives the screen back.**
    ///
    /// `ADR-015` §6.2 point 4 says the splash returns when the owner exits, and
    /// until 2026-09-03 that was true only when the supervisor had done the
    /// stopping. An application that ended by itself kept its uid on `/dev/fb0`
    /// and the card node, this machine went on reporting that it held the
    /// screen, and there was nothing on it.
    ///
    /// Driven through `Done` — `restart = "never"`, one clean exit — rather
    /// than through five crashes and a give-up. Both reach `release_finally` by
    /// the same two lines, and the crash path additionally depends on the
    /// backoff, the success window and the attempt count, none of which this is
    /// about.
    #[test]
    fn a_display_owner_that_exits_releases_the_screen_and_the_selection() {
        let mut k = kernel();
        let mut d = kiosk_def(&mut k, "a", vec![Device::Display]);
        d.restart = RestartPolicy::Never;
        let mut s = showing(vec![d], "a");
        s.converge(&mut k);
        assert_eq!(s.display_selection(), Some("a"), "the fixture must be showing it");
        let Some(&State::Started { pid, .. }) = s.state_of("a") else {
            unreachable!("it did not start: {:?}", s.state_of("a"))
        };

        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert_eq!(s.services()[0].state, State::Done);
        assert!(
            k.displays_closed.iter().any(|n| n == "a"),
            "the display channel is still open: {:?}",
            k.displays_closed
        );
        assert_eq!(
            s.display_selection(),
            None,
            "this machine still says a program that is not running holds the screen"
        );
        assert!(
            k.records.iter().any(|r| r.msg.contains("the screen is the launcher's again")),
            "nothing said the screen came back: {:?}",
            k.records.iter().map(|r| r.msg.clone()).collect::<Vec<_>>()
        );
    }

    /// **A lessee that exits while the launcher is up does NOT run the
    /// splash.**
    ///
    /// The audit of 2026-09-03's LOW list: *"splash run by root on every stop
    /// edge even while a DRM master holds the screen"*. `cmd/rivet-splash`
    /// writes `/dev/fb0`, and `PLAN.md` §104.33.4's reading is that DRM master
    /// SUSPENDS the framebuffer console — so with a launcher holding master the
    /// splash drew nothing and this supervisor then journalled that it had.
    ///
    /// Two assertions, and the second is the one that matters: nothing was run,
    /// and nothing claimed the screen had been redrawn.
    #[test]
    fn a_lessee_that_exits_under_a_running_launcher_does_not_run_the_splash() {
        let mut k = kernel();
        let launcher = controlling_the_kiosk(kiosk_def(
            &mut k,
            "launcher",
            vec![Device::Display, Device::Input],
        ));
        let mut app = kiosk_def(&mut k, "z-app", vec![Device::Display]);
        app.restart = RestartPolicy::Never;
        with_drm(&mut k);

        let mut s = showing(vec![launcher, app], "z-app");
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("launcher"), Some(&State::Started { .. })),
            "the fixture needs the launcher running: {:?}",
            s.state_of("launcher")
        );
        let Some(&State::Started { pid, .. }) = s.state_of("z-app") else {
            unreachable!("the lessee did not start: {:?}", s.state_of("z-app"))
        };
        k.ran.clear();
        k.records.clear();

        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert_eq!(s.services()[1].state, State::Done);
        assert!(
            !k.ran.iter().any(|p| p.path.contains("splash")),
            "the splash was run while the launcher held DRM master: {:?}",
            k.ran.iter().map(|p| p.path.clone()).collect::<Vec<_>>()
        );
        assert!(
            !k.records.iter().any(|r| r.msg.contains("the splash was drawn again")),
            "this machine reported drawing a splash that could not have reached the screen: \
             {:?}",
            k.records.iter().map(|r| r.msg.clone()).collect::<Vec<_>>()
        );
        assert!(
            k.records.iter().any(|r| r.msg.contains("holds DRM master")),
            "nothing said why the splash was skipped: {:?}",
            k.records.iter().map(|r| r.msg.clone()).collect::<Vec<_>>()
        );
    }

    /// And with **no** arbiter on the machine it still comes back, which is
    /// `ADR-015` §6.2 point 4 unchanged. Without this the test above would pass
    /// against a supervisor that had simply stopped restoring the splash.
    #[test]
    fn a_display_owner_that_exits_with_no_launcher_still_brings_the_splash_back() {
        let mut k = kernel();
        let mut app = kiosk_def(&mut k, "solo", vec![Device::Display]);
        app.restart = RestartPolicy::Never;
        with_drm(&mut k);

        let mut s = showing(vec![app], "solo");
        s.converge(&mut k);
        let Some(&State::Started { pid, .. }) = s.state_of("solo") else {
            unreachable!("it did not start: {:?}", s.state_of("solo"))
        };
        k.ran.clear();

        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert!(
            k.ran.iter().any(|p| p.path.contains("splash")),
            "the splash did not come back on a machine with no arbiter: {:?}",
            k.ran.iter().map(|p| p.path.clone()).collect::<Vec<_>>()
        );
    }

    /// **The splash runs under a deadline, and one that is missed is said.**
    /// `ADR-017` §3: the restore is a stop edge inside the supervisor's loop,
    /// so an unbounded wait on it is a supervisor that stops supervising for as
    /// long as the splash takes.
    #[test]
    fn a_splash_that_misses_its_deadline_is_stopped_and_reported() {
        let mut k = kernel();
        let mut app = kiosk_def(&mut k, "solo", vec![Device::Display]);
        app.restart = RestartPolicy::Never;
        with_drm(&mut k);

        let mut s = showing(vec![app], "solo");
        s.converge(&mut k);
        let Some(&State::Started { pid, .. }) = s.state_of("solo") else {
            unreachable!("it did not start: {:?}", s.state_of("solo"))
        };
        k.run_limits.clear();
        k.run_results = Script::always(Err(Errno::ETIMEDOUT));

        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert_eq!(
            k.run_limits,
            [2_000],
            "the splash was not run under its deadline, or not run at all"
        );
        assert!(
            k.records.iter().any(|r| r.sev == Severity::Warn
                && r.msg.contains("did not finish within its deadline")),
            "a splash stopped at its deadline was not reported: {:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **The incoming display owner keeps the device nodes it was just
    /// given.**
    ///
    /// `PLAN.md` §123, Wave 2.7. A convergence pass used to decide and act on
    /// each service in the order they are stored. When the application taking
    /// the screen sorts BEFORE the one giving it up, that order was
    /// `Start(a-incoming)` and then `Stop(z-outgoing)` — and the stop's revoke
    /// chowned `/dev/fb0` and `/dev/input/*` back to `root:root` after the new
    /// owner had already been spawned and given them. A kiosk application with
    /// a screen it cannot open, and nothing anywhere saying why.
    ///
    /// **The order that matters is POSITION in the service list**, which
    /// `Supervisor::new` takes from the order it is handed and does not sort —
    /// so on a machine it is whatever built the definitions, and the names here
    /// are a reminder of that rather than the mechanism. The incoming
    /// application is listed FIRST on purpose: a fixture in the other order
    /// passes whether or not the fix is present, and would prove nothing.
    #[test]
    fn handing_the_screen_over_in_one_pass_leaves_it_with_the_new_owner() {
        let mut k = kernel();
        let incoming = kiosk_def(&mut k, "a-incoming", vec![Device::Display]);
        let outgoing = kiosk_def(&mut k, "z-outgoing", vec![Device::Display]);
        with_drm(&mut k);
        assert!("a-incoming" < "z-outgoing", "the fixture must exercise the bad order");

        let mut s = showing(vec![incoming, outgoing], "z-outgoing");
        s.converge(&mut k);
        assert!(
            k.spawned.iter().any(|(n, _)| n == "z-outgoing"),
            "the outgoing application never started: {:?}",
            k.spawned
        );

        // The hand-over, in ONE pass: the selection moves, which disables one
        // and enables the other, and both actions fall out of the same
        // convergence.
        k.chowns.clear();
        s.select_display("a-incoming").expect("it is offered");
        s.converge(&mut k);

        // What matters is the LAST word on each node: whoever the kernel was
        // told owns it when the pass ended.
        let last = |path: &str| -> Option<(u32, u32)> {
            k.chowns.iter().rev().find(|(p, _, _)| p == path).map(|(_, u, g)| (*u, *g))
        };
        // **The render node, because both of these are lessees** — `ADR-036`
        // §3.1 as amended 2026-09-04. It used to be `/dev/fb0`, which no
        // application handed a lease is given any more.
        let node = last("/dev/dri/renderD128")
            .expect("the render node was chowned during the hand-over");
        assert_eq!(
            node,
            (0, rivet_core::etc::RENDER_GID),
            "the render node left its shared form during the hand-over — chowns in order: {:?}",
            k.chowns
        );
        let incoming = k
            .spawned
            .iter()
            .rev()
            .find(|(n, _)| n == "a-incoming")
            .expect("the incoming application started");
        assert!(
            incoming.1.groups.contains(&rivet_core::etc::RENDER_GID),
            "the new owner was started without the render group: {:?}",
            incoming.1.groups
        );
    }

    // ----------------------------------------------------------------------
    // The program a reinstall changed — Shared Rivets, 2026-08-31
    // ----------------------------------------------------------------------

    /// The defect, in a test: a manifest that changed what an application runs
    /// WITH must reach the definition, or the next start uses a reading nobody
    /// can see. Found on a board — `greeter` resolved `libgreet-1.0.0` while its
    /// own manifest named `libgreet-2.0.0`.
    #[test]
    fn a_reinstall_that_changes_the_closure_reaches_the_definition() {
        let d = application_def("a");
        let mut s = supervisor(vec![d.clone()]);

        let mut fresh = d.clone();
        fresh.program.env = vec![(
            "LD_LIBRARY_PATH".to_string(),
            "/state/lib/libgreet-2.0.0-ff/lib".to_string(),
        )];
        assert!(s.take_application_definition("a", &fresh));
        assert_eq!(s.services()[0].def.program.env, fresh.program.env);
    }

    /// **The second half of the same defect, four hours later.** The first fix
    /// took the *program* out of a fresh reading; `listen` went stale in exactly
    /// the same way and the supervisor opened no socket for an application whose
    /// manifest on the card declared one. Found on a board.
    #[test]
    fn a_reinstall_that_changes_listen_reaches_the_definition() {
        let d = application_def("a");
        assert!(d.listen.is_empty());
        let mut s = supervisor(vec![d.clone()]);

        let mut fresh = d.clone();
        fresh.listen = vec![Socket::Unix("kiosk.sock".to_string())];
        assert!(s.take_application_definition("a", &fresh));
        assert_eq!(s.services()[0].def.listen, fresh.listen);
    }

    /// Twice with the same reading is not a change, so the record does not say
    /// one happened. An install edge fires on every install, and most change
    /// nothing the supervisor acts on.
    #[test]
    fn the_same_definition_read_again_is_not_a_change() {
        let d = application_def("a");
        let fresh = d.clone();
        let mut s = supervisor(vec![d]);
        assert!(!s.take_application_definition("a", &fresh));
    }

    /// **The image's own services are never touched by an install edge**, which
    /// is `set_application_enabled`'s rule and holds here for the same reason:
    /// their definitions are compiled in, so a manifest on `/state` has no
    /// business rewriting one.
    #[test]
    fn an_image_service_never_takes_a_definition_from_an_install() {
        let d = def("a"); // Origin::Image, from IMAGE_DEFINED
        let mut s = supervisor(vec![d.clone()]);
        let mut fresh = d;
        fresh.program.path = "/state/app/a/current/bin/a".to_string();
        fresh.listen = vec![Socket::Tcp(80)];
        assert!(!s.take_application_definition("a", &fresh));
        assert_eq!(s.services()[0].def.program.path, "/bin/svc");
        assert!(s.services()[0].def.listen.is_empty());
    }

    // ----------------------------------------------------------------------
    // Readiness — ADR-017 §3.3
    // ----------------------------------------------------------------------

    #[test]
    fn a_notify_service_is_given_a_socket_in_its_environment() {
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let (_, program) = &k.spawned[0];
        assert!(
            program
                .env
                .iter()
                .any(|(key, value)| key == "NOTIFY_SOCKET" && value.contains('a')),
            "{:?}",
            program.env
        );
    }

    #[test]
    fn a_service_that_does_not_use_notify_gets_no_socket() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(k.notify_open.is_empty());
        assert!(k.spawned[0].1.env.is_empty());
    }

    /// **An application's notify socket is owned by the identity that has to
    /// write to it**, which is the one thing that makes `ready = "notify"`
    /// reachable for an application at all.
    ///
    /// A board answered *"could not declare readiness: Permission denied"*
    /// (2026-08-25): the supervisor binds the socket as root and `ADR-015`
    /// §5.1's application runs as its own uid, so it could never send. The
    /// service came up **running but never ready**, for ever, and the report
    /// said exactly that without anyone noticing it was unreachable rather than
    /// merely unreported.
    #[test]
    fn an_applications_notify_socket_is_owned_by_the_identity_that_writes_to_it() {
        let identity = rivet_core::service::AppIdentity {
            uid: 27_751,
            gid: 27_751,
        };
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        d.origin = Origin::Application;
        // An application is on a volume, and `finish_uninstalls` refuses to
        // remove one whose volume it cannot name (`ADR-031` §8 item 1).
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        d.program.identity = Some(identity);
        // **The application must pass launch verification to reach the socket
        // at all** — `crate::verify` runs before it, deliberately, so a refusal
        // leaves nothing open behind it. That ordering is why this test needs a
        // really signed package rather than a stub, and why the fixture is
        // shared with `verify`'s own tests rather than duplicated.
        let version_dir = d
            .program
            .path
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .expect("the program path has a version directory above it");
        crate::verify::fixture::install_signed(&mut k, "a", &version_dir, &d.program.path);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert_eq!(
            k.notify_owners.get("a"),
            Some(&Some((identity.uid, identity.gid))),
            "the socket must be chowned to the application: {:?}",
            k.notify_owners
        );
    }

    /// **And a root service's is left alone.** Chowning to `0:0` would be a
    /// syscall that changes nothing and can still fail — and every service from
    /// the image runs as root, so that would be the common path taking a risk
    /// for no gain.
    #[test]
    fn a_root_services_notify_socket_is_not_chowned_at_all() {
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert_eq!(
            k.notify_owners.get("a"),
            Some(&None),
            "a root service must not be chowned: {:?}",
            k.notify_owners
        );
    }

    #[test]
    fn started_and_ready_are_different_states() {
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        assert!(matches!(
            s.state_of("a"),
            Some(State::Started { ready: false, .. })
        ));
        s.note_ready(&mut k, "a");
        assert!(matches!(
            s.state_of("a"),
            Some(State::Started { ready: true, .. })
        ));
        assert!(k.records.iter().any(|r| r.msg == "ready" && r.svc == "a"));
    }

    #[test]
    fn a_repeated_readiness_notification_is_recorded_once() {
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        s.note_ready(&mut k, "a");
        s.note_ready(&mut k, "a");
        assert_eq!(k.records.iter().filter(|r| r.msg == "ready").count(), 1);
    }

    #[test]
    fn readiness_from_a_service_that_is_not_running_is_reported_not_believed() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_ready(&mut k, "a");
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Warn && r.msg.starts_with("readiness declared by"))
        );
    }

    #[test]
    fn readiness_from_a_name_the_supervisor_does_not_know_is_reported() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        s.note_ready(&mut k, "ghost");
        assert!(
            k.records
                .iter()
                .any(|r| r.msg == "readiness notification for an unknown service")
        );
    }

    #[test]
    fn a_service_that_never_declares_readiness_is_reported_and_left_running() {
        let mut k = kernel();
        let mut d = def("a");
        d.readiness = Readiness::Notify;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        k.mono_ms = Limits::default().ready_timeout_ms;
        s.converge(&mut k);
        s.converge(&mut k);

        assert!(matches!(
            s.state_of("a"),
            Some(State::Started { ready: false, .. })
        ));
        assert_eq!(
            k.records
                .iter()
                .filter(|r| r.msg.starts_with("running, but has not declared readiness"))
                .count(),
            1,
            "said once, not on every pass"
        );
        assert!(k.signals.is_empty(), "a slow service must not be killed");
    }

    #[test]
    fn a_status_message_is_recorded_against_the_service() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_status(&mut k, "a", "listening on 22");
        assert!(
            k.records
                .iter()
                .any(|r| r.svc == "a" && r.msg == "status"),
            "{:?}",
            k.records
        );
    }

    // -- ADR-022 §4's capture, at this layer ---------------------------------

    /// A mechanism's line becomes a record **in the mechanism's name**, which
    /// is what makes `rivet get log | where svc == rivet-netd` show what the
    /// program said and not only what the supervisor said about it.
    #[test]
    fn a_captured_line_becomes_a_record_carrying_the_services_name_and_the_stream() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("rivet-netd")]);
        s.note_output(
            &mut k,
            "rivet-netd",
            Stream::Stderr,
            "configured eth0 with 192.0.2.10/24",
            false,
        );
        let record = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("configured eth0"))
            .unwrap_or_else(|| panic!("the line was not journalled: {:?}", k.records));
        assert_eq!(record.svc, "rivet-netd");
        assert_eq!(
            record.fields,
            vec![("stream".to_string(), Value::Str("stderr".to_string()))]
        );
    }

    /// **stderr is not an error.** A mechanism that writes everything there
    /// including its banner is ordinary, so a supervisor that read the stream
    /// as a severity would make every boot show errors — and an operator who
    /// learns to ignore them stops reading the ones that matter.
    #[test]
    fn a_line_on_stderr_is_not_recorded_as_an_error() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("rivet-netd")]);
        s.note_output(&mut k, "rivet-netd", Stream::Stderr, "Not backgrounding", false);
        assert!(
            k.records
                .iter()
                .filter(|r| r.msg == "Not backgrounding")
                .all(|r| r.sev == Severity::Info),
            "{:?}",
            k.records
        );
    }

    /// A cut line says so, and says at what length — so a reader comparing the
    /// journal with a console can see why the two differ rather than
    /// concluding the journal lost something.
    #[test]
    fn a_truncated_line_carries_the_length_it_was_cut_at() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_output(&mut k, "a", Stream::Stdout, "very long", true);
        let record = k
            .records
            .iter()
            .find(|r| r.msg == "very long")
            .unwrap_or_else(|| panic!("{:?}", k.records));
        assert!(
            record
                .fields
                .iter()
                .any(|(n, v)| n == "truncated"
                    && *v
                        == Value::Int(
                            i64::try_from(crate::kernel::MAX_CAPTURED_LINE).unwrap_or(i64::MAX)
                        )),
            "{record:?}"
        );
    }

    /// A line the supervisor cannot attribute is dropped rather than recorded
    /// against nothing. It cannot arrive through `Kernel::wait` — the captures
    /// are made by `spawn` — and the arm exists because the port does not say
    /// so, which means a future one could.
    #[test]
    fn a_line_from_a_service_this_supervisor_does_not_know_is_dropped() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_output(&mut k, "not-a-service", Stream::Stdout, "orphan line", false);
        assert!(
            !k.records.iter().any(|r| r.msg == "orphan line"),
            "{:?}",
            k.records
        );
    }

    /// **`ADR-022` §4's first path, arriving through the second.**
    ///
    /// `cmd/rivet-timed` emits proper §3 records on stdout. Wrapping one as
    /// the `msg` of another record would bury its severity at `Info`, so
    /// `rivet get log | where sev == error` would miss a time client that
    /// failed. This is the loss that makes the unwrap worth doing.
    #[test]
    fn a_line_that_is_itself_a_record_keeps_its_severity_and_its_fields() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("rivet-timed")]);
        let emitted = Record {
            ts: Some("2026-08-22T14:02:11.412Z".to_string()),
            boot: "the-childs-idea-of-the-boot".to_string(),
            mono: 9.5,
            sev: Severity::Error,
            svc: "rivet-timed".to_string(),
            corr: "child-corr".to_string(),
            actor: "timed".to_string(),
            msg: "the SNTP exchange failed".to_string(),
            fields: vec![("server".to_string(), Value::Str("pool".to_string()))],
        };
        s.note_output(
            &mut k,
            "rivet-timed",
            Stream::Stdout,
            emitted.to_jsonl().trim_end(),
            false,
        );

        let record = k
            .records
            .iter()
            .find(|r| r.msg == "the SNTP exchange failed")
            .unwrap_or_else(|| panic!("the record was not unwrapped: {:?}", k.records));
        assert_eq!(record.sev, Severity::Error, "the severity was flattened");
        assert!(
            record
                .fields
                .iter()
                .any(|(n, v)| n == "server" && *v == Value::Str("pool".to_string())),
            "the child's fields were dropped: {record:?}"
        );
        // Nothing it knows is lost: its own causal chain survives as fields.
        assert!(
            record
                .fields
                .iter()
                .any(|(n, v)| n == "emitted_corr" && *v == Value::Str("child-corr".to_string())),
            "{record:?}"
        );
        assert!(
            record.fields.iter().any(|(n, _)| n == "wrapped"),
            "a forwarded record does not say it was one: {record:?}"
        );
    }

    /// **And a service cannot claim to be a different service, or another
    /// boot.** The thing on the other end of a pipe is a program; `ADR-011`'s
    /// one authority for *who said something* is the supervisor that started
    /// it, not the speaker.
    #[test]
    fn a_forwarded_record_cannot_forge_its_service_or_its_boot() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("rivet-timed")]);
        let forged = Record {
            ts: None,
            boot: "some-other-boot".to_string(),
            mono: 1.0,
            sev: Severity::Info,
            svc: "rivet-sysd".to_string(),
            corr: "c".to_string(),
            actor: "root".to_string(),
            msg: "everything is fine".to_string(),
            fields: vec![
                // Reserved names must not reach the record's own columns
                // either — Record::RESERVED_FIELD_NAMES would drop them on the
                // way out, but they would still have been carried this far.
                ("svc".to_string(), Value::Str("rivet-initd".to_string())),
                ("boot".to_string(), Value::Str("elsewhere".to_string())),
            ],
        };
        s.note_output(
            &mut k,
            "rivet-timed",
            Stream::Stdout,
            forged.to_jsonl().trim_end(),
            false,
        );
        let record = k
            .records
            .iter()
            .find(|r| r.msg == "everything is fine")
            .unwrap_or_else(|| panic!("{:?}", k.records));
        assert_eq!(record.svc, "rivet-timed", "a service forged its own name");
        assert_ne!(record.boot, "some-other-boot", "a service forged a boot id");
        assert!(
            !record.fields.iter().any(|(n, _)| n == "svc" || n == "boot"),
            "a reserved column arrived as a field: {record:?}"
        );
    }

    /// A line that merely *starts* like JSON is not a record, and is wrapped as
    /// text like anything else. A mechanism printing `{` is not a claim.
    #[test]
    fn a_line_that_is_not_a_record_is_still_wrapped_as_text() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("rivet-netd")]);
        s.note_output(&mut k, "rivet-netd", Stream::Stderr, "{not a record", false);
        let record = k
            .records
            .iter()
            .find(|r| r.msg == "{not a record")
            .unwrap_or_else(|| panic!("{:?}", k.records));
        assert_eq!(record.sev, Severity::Info);
        assert!(!record.fields.iter().any(|(n, _)| n == "wrapped"), "{record:?}");
    }

    /// The correlation identifier ties a mechanism's line back to the start
    /// that produced it (`ADR-022` §5) — one causal chain, across the
    /// supervisor's records *and* the service's own words.
    #[test]
    fn a_captured_line_shares_the_correlation_id_of_the_start_that_produced_it() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let started = k
            .records
            .iter()
            .find(|r| r.msg == "started" && r.svc == "a")
            .map(|r| r.corr.clone())
            .unwrap_or_else(|| panic!("the service was not started: {:?}", k.records));
        s.note_output(&mut k, "a", Stream::Stdout, "hello from the service", false);
        let line = k
            .records
            .iter()
            .find(|r| r.msg == "hello from the service")
            .unwrap_or_else(|| panic!("{:?}", k.records));
        assert_eq!(line.corr, started);
    }

    // -- ADR-022 §4.1's kernel messages --------------------------------------

    /// **The `svc` is `kernel`**, which is the whole query this exists for:
    /// `rivet get log | where svc == kernel` is `dmesg`, over the control
    /// plane, on the same timeline as everything else.
    #[test]
    fn a_kernel_message_is_recorded_against_the_kernel_and_not_the_supervisor() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_kernel_message(
            &mut k,
            "3,224,83932,-;brcmfmac: brcmf_fw_alloc_request: using brcm/brcmfmac43455-sdio for chip BCM4345/6",
        );
        let record = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("brcmfmac"))
            .unwrap_or_else(|| panic!("{:?}", k.records));
        assert_eq!(record.svc, "kernel");
        assert_eq!(record.sev, Severity::Error, "priority 3 is an error");
        assert!(
            record
                .fields
                .iter()
                .any(|(n, v)| n == "priority" && *v == Value::Int(3)),
            "{record:?}"
        );
    }

    /// `ADR-022` §5 ties together the records of **one causal chain**, and a
    /// kernel message belongs to none Rivet started. One identifier for all
    /// of them would say the whole boot's dmesg was one chain.
    #[test]
    fn each_kernel_message_gets_its_own_correlation_identifier() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_kernel_message(&mut k, "6,1,0,-;one");
        s.note_kernel_message(&mut k, "6,2,0,-;two");
        let corrs: Vec<String> = k
            .records
            .iter()
            .filter(|r| r.svc == "kernel")
            .map(|r| r.corr.clone())
            .collect();
        assert_eq!(corrs.len(), 2);
        assert_ne!(corrs[0], corrs[1]);
    }

    /// A record this build cannot read is journalled **carrying the raw line**,
    /// never dropped — `AGENTS.md` §5. A kernel whose format differs is exactly
    /// when its messages matter most.
    #[test]
    fn a_kmsg_record_that_will_not_parse_is_recorded_with_the_line_the_kernel_wrote() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.note_kernel_message(&mut k, "this is not a kmsg record");
        let record = k
            .records
            .iter()
            .find(|r| r.svc == "kernel")
            .unwrap_or_else(|| panic!("{:?}", k.records));
        assert_eq!(record.sev, Severity::Warn);
        assert!(
            record
                .fields
                .iter()
                .any(|(n, v)| n == "raw"
                    && *v == Value::Str("this is not a kmsg record".to_string())),
            "the kernel's own line was dropped: {record:?}"
        );
    }

    // ----------------------------------------------------------------------
    // Adoption — ADR-017 §3.1.2 and §3.1.3
    // ----------------------------------------------------------------------

    /// The property the whole process split buys: a supervisor restart does not
    /// interrupt services, and the new supervisor must not start a second copy.
    #[test]
    fn a_service_already_in_its_cgroup_is_adopted_rather_than_started_again() {
        let mut k = kernel();
        k.observations = Script::always(Observation::Members(vec![101, 102]));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);

        assert!(k.spawned.is_empty(), "a second copy was started");
        assert!(matches!(s.state_of("a"), Some(State::Adopted { .. })));
        assert!(
            k.records
                .iter()
                .any(|r| r.msg == "already running; adopted rather than started")
        );
    }

    #[test]
    fn an_adopted_service_that_goes_away_is_started_again() {
        let mut k = kernel();
        k.observations = Script::sequence(
            vec![Observation::Members(vec![101])],
            Observation::Empty,
        );
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Adopted { .. })));

        k.mono_ms = Limits::default().adopt_poll_ms;
        s.converge(&mut k); // notices it is gone
        s.converge(&mut k); // starts it
        assert_eq!(k.spawned.len(), 1);
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));
    }

    // ----------------------------------------------------------------------
    // ADR-017 §3.1.3.1: `ENOENT` is two facts, and they are told apart
    // ----------------------------------------------------------------------

    /// Every record this crate writes about being unable to observe a service.
    ///
    /// Matched on the clause they share rather than on either full sentence, so
    /// that a test asserting **silence** cannot be satisfied by a wording change
    /// — which is the way this family of assertions would otherwise rot.
    fn unobservable_warnings(k: &FakeKernel) -> Vec<&crate::journal::Record> {
        k.records
            .iter()
            .filter(|r| r.msg.contains("cannot be established"))
            .collect()
    }

    /// **The boot that produced this defect, asserted as silence.**
    ///
    /// A Raspberry Pi 4 on 2026-08-23 had cgroup2 mounted (`ADR-024` §8.1) and
    /// no `/sys/fs/cgroup/rivet/<service>`, because neither service had run
    /// yet. The supervisor warned about a double start that could not happen —
    /// once per service, on every boot, for ever.
    ///
    /// There is nothing to adopt on a first boot (`ADR-017` §3.1.1), so there
    /// is nothing to say: the `started` record two lines later carries the pid
    /// and says it better.
    #[test]
    fn a_first_boot_on_a_healthy_machine_says_nothing_about_the_cgroup() {
        let mut k = kernel();
        // Exactly what the board answered: the hierarchy is there, this
        // service's own cgroup is not.
        k.hierarchy_results = Script::always(Hierarchy::Present);
        k.observations = Script::always(Observation::Unavailable(Errno::ENOENT));

        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);

        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));
        assert!(
            unobservable_warnings(&k).is_empty(),
            "a first boot must say nothing: {:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
        // And the fact it was silent about is still on the console, once, from
        // `crate::run` — not from here.
        assert!(
            k.records.iter().any(|r| r.msg == "started"),
            "the started record is what actually says it was not running"
        );
    }

    /// The dangerous case: no cgroup2 at all.
    ///
    /// `ADR-017` §3.1.3 makes the cgroup **the** authority for what is running.
    /// Without a hierarchy there is no authority, so a restarted supervisor
    /// cannot tell a live service from a dead one — and this must stay a
    /// warning, at the same severity as before, because that is the case the
    /// old text described and the one it was drowned out by.
    #[test]
    fn a_machine_with_no_cgroup2_hierarchy_is_warned_about_and_starts_anyway() {
        let mut k = kernel();
        k.hierarchy_results = Script::always(Hierarchy::Unreachable(Errno::ENOENT));
        k.observations = Script::always(Observation::Unavailable(Errno::ENOENT));

        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));

        let warnings = unobservable_warnings(&k);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].sev, Severity::Warn);
        assert!(
            warnings[0].msg.contains("no readable cgroup2 hierarchy"),
            "it must name the hierarchy, not the service's directory: {}",
            warnings[0].msg
        );
        // The consequence is stated conditionally, because it IS conditional —
        // the old text asserted a double start would happen on a boot where
        // nothing was running at all.
        assert!(
            warnings[0].msg.contains("if a previous supervisor left one running"),
            "{}",
            warnings[0].msg
        );
        // What the kernel said about the hierarchy is a different field from
        // what it said about the service (`AGENTS.md` §5).
        let names: Vec<&str> = warnings[0].fields.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"hierarchy_errno"), "{names:?}");
        assert!(names.contains(&"errno"), "{names:?}");

        // Restart it a few times; the warning must not repeat on every attempt.
        for _ in 0..3 {
            if let Some(State::Started { pid, .. }) = s.state_of("a").cloned() {
                k.exits.push((pid, ExitStatus::Exited(1)));
                s.reap(&mut k);
            }
            if let Some(State::Backoff { until_ms }) = s.state_of("a") {
                k.mono_ms = *until_ms;
            }
            s.converge(&mut k);
        }
        assert_eq!(unobservable_warnings(&k).len(), 1);
    }

    /// The third state, which is neither of the two the ruling is named for:
    /// the hierarchy is fine and *this* service's cgroup cannot be read.
    ///
    /// `EACCES` is not "it has never run" — something is there, or something is
    /// refusing — so the supervisor is blind about this service and says so in
    /// its own words. Told apart from the case above by the errno, which is why
    /// the branch is `Present && ENOENT` rather than `Present`.
    #[test]
    fn a_readable_hierarchy_with_an_unreadable_service_cgroup_is_still_a_warning() {
        let mut k = kernel();
        k.hierarchy_results = Script::always(Hierarchy::Present);
        k.observations = Script::always(Observation::Unavailable(Errno::EACCES));

        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);

        let warnings = unobservable_warnings(&k);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].msg.contains("hierarchy is readable but this service's own cgroup"),
            "{}",
            warnings[0].msg
        );
        // No hierarchy errno, because the hierarchy did not fail. A field that
        // reported one would be inventing a failure.
        let names: Vec<&str> = warnings[0].fields.iter().map(|(n, _)| n.as_str()).collect();
        assert!(!names.contains(&"hierarchy_errno"), "{names:?}");
    }

    /// A machine-wide failure is per-service news, and every service gets it.
    ///
    /// The latch is per-service on purpose: the hazard it warns about is a
    /// second copy of *that* service. A supervisor-wide latch would name one
    /// service and leave the rest of a blind machine unreported.
    #[test]
    fn a_blind_machine_warns_for_every_service_rather_than_only_the_first() {
        let mut k = kernel();
        k.hierarchy_results = Script::always(Hierarchy::Unreachable(Errno::ENOENT));
        k.observations = Script::always(Observation::Unavailable(Errno::ENOENT));

        let mut s = supervisor(vec![def("a"), def("b"), def("c")]);
        s.converge(&mut k);

        let warned: Vec<&str> = unobservable_warnings(&k)
            .iter()
            .map(|r| r.svc.as_str())
            .collect();
        assert_eq!(warned, ["a", "b", "c"]);
    }

    /// **The regression this ruling could most easily introduce.**
    ///
    /// The benign branch returns before the latch, so a machine that boots
    /// healthy and loses its hierarchy later still gets the warning. Latching
    /// on the silent path would make the one failure that matters the one that
    /// is never reported.
    #[test]
    fn a_silent_first_boot_does_not_use_up_the_warning() {
        let mut k = kernel();
        k.hierarchy_results = Script::sequence(
            vec![Hierarchy::Present],
            Hierarchy::Unreachable(Errno::ENOENT),
        );
        k.observations = Script::always(Observation::Unavailable(Errno::ENOENT));

        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k); // healthy: silent
        assert!(unobservable_warnings(&k).is_empty());

        // The service dies and is restarted; by now the hierarchy has gone.
        if let Some(State::Started { pid, .. }) = s.state_of("a").cloned() {
            k.exits.push((pid, ExitStatus::Exited(1)));
            s.reap(&mut k);
        }
        if let Some(State::Backoff { until_ms }) = s.state_of("a") {
            k.mono_ms = *until_ms;
        }
        s.converge(&mut k);

        assert_eq!(unobservable_warnings(&k).len(), 1);
    }

    /// The hierarchy is re-read, never remembered (`AGENTS.md` §5), and it is
    /// asked **only** where its answer could change what is said.
    #[test]
    fn the_hierarchy_is_asked_on_the_failing_path_and_nowhere_else() {
        // An observation that succeeded settles the question by itself.
        let mut k = kernel();
        k.observations = Script::always(Observation::Empty);
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert_eq!(k.hierarchy_calls, 0, "an Empty cgroup needs no second probe");

        // One that failed asks, and asks again on the next attempt rather than
        // reusing the first answer.
        let mut k = kernel();
        k.observations = Script::always(Observation::Unavailable(Errno::ENOENT));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert_eq!(k.hierarchy_calls, 1);
        if let Some(State::Started { pid, .. }) = s.state_of("a").cloned() {
            k.exits.push((pid, ExitStatus::Exited(1)));
            s.reap(&mut k);
        }
        if let Some(State::Backoff { until_ms }) = s.state_of("a") {
            k.mono_ms = *until_ms;
        }
        s.converge(&mut k);
        assert_eq!(k.hierarchy_calls, 2, "the verdict must not be cached");
    }

    #[test]
    fn a_cgroup_placement_failure_is_reported_and_the_service_keeps_running() {
        let mut k = kernel();
        k.place_results = Script::always(Err(Errno::EPERM));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.starts_with("started, but could not be placed"))
        );
    }

    #[test]
    fn disabling_an_adopted_service_signals_the_processes_it_did_not_spawn() {
        let mut k = kernel();
        k.observations = Script::always(Observation::Members(vec![101, 102]));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.services[0].def.enabled = false;
        s.converge(&mut k);

        // **`Self::stop_adopted`'s own signal, then `Self::begin_drain`'s
        // fresh re-check** — S-1's fix. The fake's cgroup never empties, so
        // both rounds see the same members: this is not a double-count bug,
        // it is `begin_drain` refusing to trust a pid list that might already
        // be stale by the time it asks again.
        assert!(
            k.signals.iter().filter(|s| **s == (101, Sig::Term)).count() >= 1,
                "101 was never signalled: {:?}", k.signals
        );
        assert!(
            k.signals.iter().filter(|s| **s == (102, Sig::Term)).count() >= 1,
                "102 was never signalled: {:?}", k.signals
        );
        // A cgroup that never empties must not be reported `Idle` — that is
        // exactly the false report S-1 found. `Draining` is the honest
        // answer here, and this fake's `Observation::Members` never
        // resolves any other way: `a_stop_of_an_adopted_service_kills_the_\
        // cgroup_and_waits_for_it_to_empty` is the test that carries this
        // all the way to `Idle`.
        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "a cgroup that never empties must not be reported Idle: {:?}",
            s.state_of("a")
        );
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.starts_with("signalled processes this supervisor did not spawn"))
        );
    }

    // ----------------------------------------------------------------------
    // The journal, as the supervisor uses it
    // ----------------------------------------------------------------------

    #[test]
    fn one_attempts_records_share_a_correlation_and_the_next_attempt_does_not() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!()
        };
        let first = s.services()[0].corr.clone();
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        k.mono_ms = 10_000;
        s.converge(&mut k);
        let second = s.services()[0].corr.clone();

        assert_ne!(first, second);
        let started: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg == "started")
            .map(|r| r.corr.clone())
            .collect();
        assert_eq!(started, vec![first.clone(), second]);
        // The exit belongs to the attempt that produced it.
        assert!(
            k.records
                .iter()
                .any(|r| r.msg == "exited" && r.corr == first)
        );
    }

    #[test]
    fn every_record_carries_a_boot_an_actor_and_a_correlation() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.note(&mut k, Severity::Info, "hello", vec![]);
        assert!(!k.records.is_empty());
        for r in &k.records {
            assert_eq!(r.boot, "test-boot");
            assert_eq!(r.actor, ACTOR);
            assert!(!r.corr.is_empty(), "{r:?}");
            assert!(!r.svc.is_empty(), "{r:?}");
        }
    }

    #[test]
    fn the_journal_buffers_what_it_emitted() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert_eq!(s.journal().pending().len(), k.records.len());
        assert_eq!(s.journal().dropped(), 0);
    }

    // -- ingest (`ADR-022` §11) ---------------------------------------------

    use crate::kernel::Ingest;
    use rivet_core::journal::Peer;

    const A_PEER: Peer = Peer {
        uid: 1000,
        gid: 1000,
        pid: 4242,
    };

    fn sent(record: &Record) -> Ingest {
        Ingest::Line(record.to_jsonl().trim_end().to_string())
    }

    fn an_edit_record() -> Record {
        Record {
            ts: Some("2026-08-24T14:02:11.412Z".to_string()),
            boot: "the-senders-idea-of-a-boot".to_string(),
            mono: 4321.0,
            sev: Severity::Info,
            svc: "rivet".to_string(),
            corr: "senders-chain".to_string(),
            actor: "operator".to_string(),
            msg: "replace /state/x: 7 bytes to 6".to_string(),
            fields: vec![("path".to_string(), Value::Str("/state/x".to_string()))],
        }
    }

    /// **`ADR-022` §7 is intact and this is the test that says so.** The record
    /// was composed by somebody else and every column that places it in this
    /// machine's history is this process's: the boot it belongs to, the
    /// monotonic reading that orders it, and a correlation identifier minted
    /// here.
    #[test]
    fn an_ingested_record_is_stamped_by_the_supervisor_and_not_by_its_sender() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        k.mono_ms = 7_000;
        s.note_ingest(&mut k, A_PEER, &sent(&an_edit_record()));

        let r = k.records.last().unwrap_or_else(|| panic!("no record"));
        assert_eq!(r.boot, "test-boot", "the sender chose the boot");
        assert!((r.mono - 7.0).abs() < 1e-9, "{}", r.mono);
        assert_ne!(r.corr, "senders-chain");
        // And what only the sender knew survived.
        assert_eq!(r.msg, "replace /state/x: 7 bytes to 6");
        assert_eq!(r.svc, "rivet");
        assert_eq!(
            r.field("emitted_corr"),
            Some(&Value::Str("senders-chain".to_string()))
        );
    }

    /// **The security claim, stated as an assertion.** A record that arrived on
    /// the socket carries the kernel's uid as its actor, so no sender — however
    /// it fills in its own columns — can produce a record that reads as one the
    /// supervisor made.
    #[test]
    fn no_ingested_record_can_be_dressed_as_one_the_supervisor_produced() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        let mut hostile = an_edit_record();
        hostile.actor = ACTOR.to_string();
        hostile.svc = SELF_NAME.to_string();
        hostile.msg = "all services stopped; exiting".to_string();
        s.note_ingest(&mut k, A_PEER, &sent(&hostile));

        let r = k.records.last().unwrap_or_else(|| panic!("no record"));
        assert_eq!(r.actor, "uid:1000");
        assert_ne!(r.actor, ACTOR, "a sender claimed the supervisor's actor");
        // ~~It may still claim the *service*, and that is `ADR-022` §11.4's
        // recorded trade — but the kernel's own answer is beside it, so the
        // claim is contradictable rather than believed.~~
        //
        // **Changed 2026-09-04**, for the audit of 2026-09-03. §11.4's trade
        // stands for every OTHER name and this one is excluded, because `svc`
        // is the column a reader filters by: a local process's chosen text
        // returned by `where svc == rivet-sysd` sits inside the supervisor's own
        // account of the boot, and the reader has to go and check `actor` to
        // find out it does not belong there. The claim is kept rather than
        // dropped, so nothing is lost and the attempt is visible.
        assert_eq!(r.svc, rivet_core::journal::INGEST_SVC);
        assert_eq!(
            r.field("svc_claimed"),
            Some(&Value::Str(SELF_NAME.to_string())),
            "the claim was discarded instead of being recorded"
        );
        assert_eq!(r.field("ingest_pid"), Some(&Value::Int(4242)));
        assert_eq!(r.field("ingest_uid"), Some(&Value::Int(1000)));
    }

    /// **A claim of any OTHER service's name is still kept**, which is the half
    /// of `ADR-022` §11.4 that did not change — and it has to be asserted, or
    /// the exclusion above could be widened into the refusal §11.4 rejected
    /// without any test noticing.
    #[test]
    fn a_sender_may_still_claim_a_service_that_is_not_the_supervisor() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        let mut hostile = an_edit_record();
        hostile.svc = "photo-index".to_string();
        s.note_ingest(&mut k, A_PEER, &sent(&hostile));

        let r = k.records.last().unwrap_or_else(|| panic!("no record"));
        assert_eq!(r.svc, "photo-index");
        assert_eq!(r.field("svc_claimed"), None);
        assert_eq!(r.field("ingest_pid"), Some(&Value::Int(4242)));
    }

    /// **The regression test for the audit of 2026-09-03's eviction finding.**
    ///
    /// An unprivileged process could file records until `ADR-022` §8's rotation
    /// dropped the oldest file — a denial-of-evidence attack against the one
    /// subsystem whose job is to still have the evidence. This is
    /// `PLAN.md` §11.2's option (c), which the maintainer's own recommendation
    /// named for *"when the first application ships"*.
    ///
    /// The assertion is that the flood **stops**, and that it stops with a
    /// record rather than in silence.
    #[test]
    fn a_uid_flooding_the_ingest_socket_stops_filling_the_journal() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        k.mono_ms = 1_000;

        // Everything one process can send inside a single millisecond: the
        // bucket cannot refill, so this is the burst and nothing more.
        let attempts = usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0) * 8;
        for _ in 0..attempts {
            s.note_ingest(&mut k, A_PEER, &sent(&an_edit_record()));
        }

        let filed = k.records.iter().filter(|r| r.actor == "uid:1000").count();
        assert_eq!(
            filed,
            usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0),
            "the flood was not bounded at one uid's burst"
        );
        assert!(filed < attempts, "every record was accepted");

        // And it is SAID, once, with the count — a journal that dropped records
        // in silence would be the failure `ADR-022` §11.5 forbids.
        let said: Vec<&Record> = k
            .records
            .iter()
            .filter(|r| r.field("dropped").is_some())
            .collect();
        assert_eq!(said.len(), 1, "the drop was silent, or said once per record");
        let note = said.first().unwrap_or_else(|| panic!("just asserted"));
        assert_eq!(note.sev, Severity::Warn);
        assert_eq!(note.svc, SELF_NAME, "the note is the supervisor's own record");
        assert_eq!(note.actor, ACTOR);
        assert_eq!(note.field("ingest_uid"), Some(&Value::Int(1000)));
        assert_eq!(note.field("ingest_pid"), Some(&Value::Int(4242)));
        // **One per window, so this is the FIRST drop and not the total.** The
        // rest of the window is counted and carried on the next note; a note per
        // dropped record would be the flood arriving under the supervisor's own
        // name. `rivet_core::journal::INGEST_REPORT_EVERY_MS` states what that
        // costs — a flood that stops inside a window leaves its tail counted and
        // unreported.
        assert_eq!(note.field("dropped"), Some(&Value::Int(1)));

        // The window elapses and the rest is reported, so the counts across
        // notes add up to every record that was refused.
        k.mono_ms = 1_000 + rivet_core::journal::INGEST_REPORT_EVERY_MS;
        let before = k.records.len();
        for _ in 0..(rivet_core::journal::INGEST_BURST * 8) {
            s.note_ingest(&mut k, A_PEER, &sent(&an_edit_record()));
        }
        let second: Vec<&Record> = k.records[before..]
            .iter()
            .filter(|r| r.field("dropped").is_some())
            .collect();
        assert_eq!(second.len(), 1, "the second window said nothing, or said too much");
        let Some(&Value::Int(also)) = second.first().and_then(|r| r.field("dropped")) else {
            panic!("the second note carries no count")
        };
        assert!(also > 1, "the second note repeated the first one's count: {also}");
    }

    /// **One uid's flood must not cost another uid its records.** The limit is
    /// per uid, and a shared bucket would make a hostile application able to
    /// silence the operator's own tools — which is the attack, one door over.
    #[test]
    fn one_uid_over_its_share_does_not_silence_another() {
        const OTHER: Peer = Peer {
            uid: 1001,
            gid: 1001,
            pid: 99,
        };
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        k.mono_ms = 1_000;

        for _ in 0..(rivet_core::journal::INGEST_BURST * 4) {
            s.note_ingest(&mut k, A_PEER, &sent(&an_edit_record()));
        }
        s.note_ingest(&mut k, OTHER, &sent(&an_edit_record()));

        let r = k.records.last().unwrap_or_else(|| panic!("no record"));
        assert_eq!(r.actor, "uid:1001", "the second uid's record was dropped too");
        assert_eq!(r.msg, "replace /state/x: 7 bytes to 6");
    }

    /// **uid 0 is exempt, deliberately.** Root can append to
    /// `/state/log/journal.jsonl` directly, so a limit on it buys nothing
    /// against a hostile root and costs the operator's own `apply edit` records.
    /// Asserted rather than left implicit, because it is the one hole in the
    /// mechanism and it should fail loudly if somebody closes it by accident.
    #[test]
    fn root_is_not_rate_limited_because_it_can_write_the_file_anyway() {
        const ROOT: Peer = Peer {
            uid: 0,
            gid: 0,
            pid: 7,
        };
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        k.mono_ms = 1_000;

        let attempts = rivet_core::journal::INGEST_BURST * 4;
        for _ in 0..attempts {
            s.note_ingest(&mut k, ROOT, &sent(&an_edit_record()));
        }
        let filed = k.records.iter().filter(|r| r.actor == "uid:0").count();
        assert_eq!(filed, usize::try_from(attempts).unwrap_or(0));
    }

    /// **S-3.** Until this existed, `note_output` had no admission check at
    /// all: a claim over the ingest socket was bounded per uid and a line the
    /// supervisor CAPTURED from the same application's own stdout was not, so
    /// an application could evict the whole journal by printing quickly.
    /// Mirrors `a_uid_flooding_the_ingest_socket_stops_filling_the_journal`
    /// for the capture path.
    #[test]
    fn an_applications_captured_output_is_rate_limited_and_the_suppression_is_recorded() {
        let mut k = kernel();
        let mut s = supervisor(vec![application_def("a")]);
        k.mono_ms = 1_000;

        // Everything one application can print inside a single millisecond:
        // the bucket cannot refill, so this is the burst and nothing more.
        let attempts = usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0) * 8;
        for _ in 0..attempts {
            s.note_output(&mut k, "a", Stream::Stdout, "flood", false);
        }

        let filed = k.records.iter().filter(|r| r.msg == "flood").count();
        assert_eq!(
            filed,
            usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0),
            "the flood of captured output was not bounded at the service's burst"
        );
        assert!(filed < attempts, "every captured line was accepted");

        // And it is SAID, once, with the count — silent suppression is
        // exactly what `ADR-022` §11.5 forbids for the ingest path, and a
        // captured line is no less a record for having arrived on a pipe.
        let said: Vec<&Record> = k
            .records
            .iter()
            .filter(|r| r.field("dropped").is_some())
            .collect();
        assert_eq!(said.len(), 1, "the suppression was silent, or said more than once");
        let note = said.first().unwrap_or_else(|| panic!("just asserted"));
        assert_eq!(note.sev, Severity::Warn);
        assert_eq!(
            note.svc, "a",
            "the suppression is attributed to the service whose output it was"
        );
        assert_eq!(note.field("uid"), Some(&Value::Int(i64::from(rivet_core::service::app_identity(
            &rivet_core::service::ServiceName::new("a").unwrap_or_else(|| unreachable!()),
        ).uid))));
    }

    /// The other capture path S-3 named: `STATUS=` lines over the notify
    /// socket, exercised through [`Supervisor::note_status`] rather than
    /// [`Supervisor::note_output`].
    #[test]
    fn an_applications_status_lines_are_rate_limited_and_the_suppression_is_recorded() {
        let mut k = kernel();
        let mut s = supervisor(vec![application_def("a")]);
        k.mono_ms = 1_000;

        let attempts = usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0) * 8;
        for _ in 0..attempts {
            s.note_status(&mut k, "a", "still starting");
        }

        let filed = k
            .records
            .iter()
            .filter(|r| r.field("status") == Some(&Value::Str("still starting".to_string())))
            .count();
        assert_eq!(
            filed,
            usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0),
            "the flood of status lines was not bounded at the service's burst"
        );
        assert!(filed < attempts, "every status line was accepted");
        assert_eq!(
            k.records.iter().filter(|r| r.field("dropped").is_some()).count(),
            1,
            "the suppression was silent, or said more than once"
        );
    }

    /// The third capture path S-3 named: a `READY=1` from a service the
    /// supervisor is not running, exercised through
    /// [`Supervisor::note_ready`]. Mirrors the two tests above.
    #[test]
    fn readiness_from_a_service_not_running_is_rate_limited_and_the_suppression_is_recorded() {
        let mut k = kernel();
        let mut s = supervisor(vec![application_def("a")]);
        k.mono_ms = 1_000;

        // "a" is `Idle` — this fixture never converges it — so every one of
        // these hits the "not running" branch rather than the ordinary
        // first-readiness path.
        let attempts = usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0) * 8;
        for _ in 0..attempts {
            s.note_ready(&mut k, "a");
        }

        let filed = k
            .records
            .iter()
            .filter(|r| r.msg == "readiness declared by a service the supervisor is not running")
            .count();
        assert_eq!(
            filed,
            usize::try_from(rivet_core::journal::INGEST_BURST).unwrap_or(0),
            "the flood of readiness notices was not bounded at the service's burst"
        );
        assert!(filed < attempts, "every readiness notice was accepted");
        assert_eq!(
            k.records.iter().filter(|r| r.field("dropped").is_some()).count(),
            1,
            "the suppression was silent, or said more than once"
        );
    }

    /// **A root-run image service is unaffected**, exactly as uid 0 is exempt
    /// on the ingest path — they share one uid and a limit on it would make
    /// them starve each other rather than bound any one of them.
    #[test]
    fn a_root_image_services_captured_output_is_not_rate_limited() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        k.mono_ms = 1_000;

        let attempts = rivet_core::journal::INGEST_BURST * 4;
        for _ in 0..attempts {
            s.note_output(&mut k, "a", Stream::Stdout, "flood", false);
        }

        let filed = k.records.iter().filter(|r| r.msg == "flood").count();
        assert_eq!(
            filed,
            usize::try_from(attempts).unwrap_or(0),
            "a root-run image service's captured output was throttled"
        );
        assert!(
            !k.records.iter().any(|r| r.field("dropped").is_some()),
            "a root-run image service's captured output was reported as suppressed"
        );
    }

    /// Every record the supervisor makes itself still carries its own actor, so
    /// the two are separable in both directions rather than only one.
    #[test]
    fn the_supervisors_own_records_are_untouched_by_the_ingest_path() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.note_ingest(&mut k, A_PEER, &sent(&an_edit_record()));
        let mine: Vec<&Record> = k.records.iter().filter(|r| r.actor == ACTOR).collect();
        let theirs: Vec<&Record> = k.records.iter().filter(|r| r.actor != ACTOR).collect();
        assert!(!mine.is_empty());
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].actor, "uid:1000");
    }

    /// **A refusal is a record**, and it is the supervisor's own — the peer did
    /// not compose it. Both shapes of refusal say what was dropped and who the
    /// kernel says sent it.
    #[test]
    fn a_refused_ingest_is_journalled_by_the_supervisor_naming_the_peer() {
        for (what, expected) in [
            (Ingest::TooLarge { held: 99 }, "DROPPED"),
            (Ingest::Unfinished { held: 12 }, "DROPPED"),
        ] {
            let mut k = kernel();
            let mut s = supervisor(vec![]);
            s.note_ingest(&mut k, A_PEER, &what);
            let r = k.records.last().unwrap_or_else(|| panic!("no record"));
            assert_eq!(r.sev, Severity::Warn);
            assert_eq!(r.svc, SELF_NAME);
            assert_eq!(r.actor, ACTOR, "a refusal is not the peer's record");
            assert!(r.msg.contains(expected), "{}", r.msg);
            assert_eq!(r.field("ingest_pid"), Some(&Value::Int(4242)));
        }
    }

    /// A sender speaking nonsense still gets a record, and it is a `warn`
    /// carrying the bytes — never nothing, which would be the journal answering
    /// *"there is nothing about that in the log"* falsely.
    #[test]
    fn a_sender_that_speaks_nonsense_still_lands_in_the_journal() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        s.note_ingest(&mut k, A_PEER, &Ingest::Line("Segmentation fault".to_string()));
        let r = k.records.last().unwrap_or_else(|| panic!("no record"));
        assert_eq!(r.sev, Severity::Warn);
        assert_eq!(r.msg, "Segmentation fault");
        assert_eq!(r.actor, "uid:1000");
    }

    /// Whatever arrives, what lands on the card is a line `rivet get log`
    /// reads back — the round trip that makes this a journal rather than a
    /// hope.
    #[test]
    fn anything_ingested_is_written_as_a_line_the_readers_own_parser_accepts() {
        let mut k = kernel();
        let mut s = supervisor(vec![]);
        for what in [
            sent(&an_edit_record()),
            Ingest::Line("not a record".to_string()),
            Ingest::Line("{\"sev\":\"error\"".to_string()),
            Ingest::TooLarge { held: 1 },
        ] {
            s.note_ingest(&mut k, A_PEER, &what);
            let line = k
                .records
                .last()
                .unwrap_or_else(|| panic!("no record"))
                .to_jsonl();
            Record::from_jsonl(&line)
                .unwrap_or_else(|e| panic!("{what:?} became a line that will not parse: {e}"));
        }
    }

    /// **`ADR-032` §3.2's answer, built from this supervisor's own state.**
    ///
    /// The mapping is the only place this crate's vocabulary meets the domain's,
    /// so it is asserted rather than assumed — a state word that stopped
    /// matching would make `rivet get status` report something no journal line
    /// ever says.
    #[test]
    fn the_status_answer_reports_every_service_and_the_phase() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a"), def("b")]);
        let before = s.status();
        // `running`, not the loop's own name (`ADR-032` §8).
        assert_eq!(before.phase, "running");
        assert_eq!(before.services.len(), 2);
        // Nothing has been started, so nothing has a pid and nothing is ready.
        assert!(before.services.iter().all(|x| x.pid.is_none()));
        assert!(before.services.iter().all(|x| !x.ready));
        assert_eq!(before.services[0].state, "idle");

        s.converge(&mut k);
        let running = s.status();
        assert_eq!(running.services[0].state, "started");
        assert!(running.services[0].pid.is_some(), "a started service has a pid");
        assert_eq!(running.services[0].restarts, 0, "a service that started first time has not been restarted");
        // Started is not ready — `ADR-017` §3.3, and this is the distinction a
        // status would be worthless without.
        assert!(!running.services[0].ready);

        s.begin_shutdown(&mut k, Terminating::PowerOff);
        assert_eq!(s.status().phase, "shutting-down");
    }

    /// **The whole answer survives the wire**, which is the property the format
    /// exists for: this supervisor renders and `cmd/rivet` parses, and the two
    /// have no other way to agree.
    #[test]
    fn what_this_supervisor_renders_is_what_the_cli_would_read_back() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a"), def("b")]);
        s.converge(&mut k);
        let status = s.status();
        assert_eq!(
            rivet_core::operation::Status::parse(&status.render()),
            Ok(status)
        );
    }

    // ----------------------------------------------------------------------
    // ADR-017 §6.3.2 — converge-only service control, built 2026-08-29
    // ----------------------------------------------------------------------

    use rivet_core::operation::{Doing, ServiceRestartRefusal as Refusal};

    /// A service in a given state, for the walks below. The supervisor is
    /// driven into the state rather than having it assigned, wherever that is
    /// possible without a clock.
    fn holding(state: State) -> (FakeKernel, Supervisor) {
        let k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.services[0].state = state;
        (k, s)
    }

    /// Every state a service can be in, as fixtures.
    fn every_state() -> Vec<State> {
        vec![
            State::Idle,
            State::Started {
                pid: 41,
                since_ms: 0,
                ready: false,
                ready_reported: false,
            },
            State::Adopted {
                pids: vec![42],
                next_poll_ms: 0,
            },
            State::Done,
            State::Backoff { until_ms: 9_999 },
            State::Failed,
            State::Stopping {
                pid: 43,
                deadline_ms: 9_999,
                killed: false,
            },
        ]
    }

    /// **THE RULE. A service the document disables cannot be started by this
    /// surface, from any state it can be in.** `ADR-017` §6.3.2.
    ///
    /// Walked over every state rather than asserted on one, because the guard
    /// is a guard: `decide` with `Asked::Restart` would happily `Start` a
    /// disabled service, and the only thing that stops it is that the flag
    /// cannot be set. Break the `enabled` check in `Supervisor::restart_service`
    /// and this fails on the first state.
    #[test]
    fn a_service_the_document_disables_cannot_be_started_by_this_surface() {
        for state in every_state() {
            let (mut k, mut s) = holding(state.clone());
            s.services[0].def.enabled = false;
            let refused = s
                .restart_service(&mut k, "a", true)
                .expect_err("a disabled service was restarted");
            assert!(
                matches!(refused, Refusal::Disabled { .. }),
                "{state:?} gave {refused:?}"
            );
            assert_eq!(
                s.services[0].asked,
                Asked::Nothing,
                "{state:?}: the flag was set by a refused request"
            );
            let before = k.spawned.len();
            s.converge(&mut k);
            assert_eq!(
                k.spawned.len(),
                before,
                "{state:?}: a disabled service was spawned after a refusal"
            );
        }
    }

    /// **And a disabled service that IS running cannot be left running by
    /// anything in this surface.** The other half of §6.3.2's property: the
    /// only request that can be spelled is refused, and the ordinary
    /// convergence that follows still takes the service down.
    #[test]
    fn a_disabled_service_that_is_running_cannot_be_left_running() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(matches!(s.state_of("a"), Some(State::Started { .. })));

        // The document says otherwise now — `enabled` is configuration.
        s.set_enabled("a", false);
        let refused = s
            .restart_service(&mut k, "a", true)
            .expect_err("a disabled service was restarted");
        assert!(matches!(refused, Refusal::Disabled { .. }));

        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Stopping { .. })),
            "the machine kept running something the document disables: {:?}",
            s.state_of("a")
        );
        assert!(
            k.signals.iter().any(|(pid, sig)| *pid > 0 && *sig == Sig::Term),
            "nothing was signalled: {:?}",
            k.signals
        );
    }

    /// **A running service is signalled, and the very next pass starts it
    /// again** — because the document says it should be running and nothing
    /// changed that. Two ordinary convergences, not a new mechanism.
    #[test]
    fn restarting_a_running_service_stops_it_and_the_next_pass_starts_it_again() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let State::Started { pid: first, .. } = s.services[0].state else {
            unreachable!("it did not start");
        };

        let answer = s.restart_service(&mut k, "a", true).expect("permitted");
        assert_eq!(answer.was, "started");
        assert_eq!(answer.doing, Doing::Stop);
        // Nothing has been signalled yet: the answer is written first, and the
        // convergence at the bottom of `crate::main`'s loop does the work.
        assert!(k.signals.is_empty(), "{:?}", k.signals);

        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Stopping { .. })),
            "{:?}",
            s.state_of("a")
        );
        // It exits, and the next pass has it running again.
        k.exits.push((first, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "a restart left the service down: {:?}",
            s.state_of("a")
        );
        assert_eq!(k.spawned.len(), 2, "it was not started a second time");
    }

    /// **A failed service is started and its attempt count is forgiven**, or
    /// the restart would be one retry into the same give-up latch. `ADR-017`
    /// §3.5.
    #[test]
    fn restarting_a_failed_service_starts_it_and_forgives_the_attempts() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.services[0].state = State::Failed;
        s.services[0].attempts = s.limits.give_up_after;

        let answer = s.restart_service(&mut k, "a", true).expect("permitted");
        assert_eq!(answer.was, "failed");
        assert_eq!(answer.doing, Doing::Start);
        assert_eq!(s.services[0].attempts, 0, "the give-up latch was not cleared");

        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "{:?}",
            s.state_of("a")
        );
        // And the forgiveness is on the record rather than silent.
        assert!(
            k.records.iter().any(|r| {
                r.msg.contains("restart requested")
                    && r.to_jsonl().contains("\"attempts_forgiven\"")
            }),
            "no record of the restart: {:?}",
            k.records.iter().map(|r| r.msg.clone()).collect::<Vec<_>>()
        );
    }

    /// A `oneshot` that ran to completion is run again — `rivet-netd` is the
    /// case, and §6.3.4 names it as the one an operator on a wireless board
    /// actually wants.
    #[test]
    fn restarting_a_service_that_ran_to_completion_runs_it_again() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.services[0].state = State::Done;
        let answer = s.restart_service(&mut k, "a", true).expect("permitted");
        assert_eq!(answer.was, "done");
        assert_eq!(answer.doing, Doing::Start);
        s.converge(&mut k);
        assert_eq!(k.spawned.len(), 1);
    }

    /// A service already on its way down is not signalled twice and its
    /// `ADR-017` §3.5 deadline is neither shortened nor extended.
    #[test]
    fn restarting_a_service_that_is_already_stopping_asks_for_nothing() {
        let (mut k, mut s) = holding(State::Stopping {
            pid: 43,
            deadline_ms: 9_999,
            killed: false,
        });
        let answer = s.restart_service(&mut k, "a", true).expect("permitted");
        assert_eq!(answer.doing, Doing::Nothing);
        s.converge(&mut k);
        assert_eq!(
            s.state_of("a"),
            Some(&State::Stopping {
                pid: 43,
                deadline_ms: 9_999,
                killed: false
            }),
            "the deadline moved"
        );
        assert!(k.signals.is_empty(), "{:?}", k.signals);
    }

    /// **The flag is cleared by the pass that sees it**, so one request is one
    /// restart. Without this a service would be signalled on every convergence
    /// for ever, which is `ADR-017` §3.5's hazard produced by the control
    /// surface.
    #[test]
    fn one_request_is_one_restart() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let State::Started { pid, .. } = s.services[0].state else {
            unreachable!()
        };
        s.restart_service(&mut k, "a", true).expect("permitted");
        s.converge(&mut k);
        assert_eq!(s.services[0].asked, Asked::Nothing);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        for _ in 0..5 {
            s.converge(&mut k);
        }
        assert_eq!(
            k.spawned.len(),
            2,
            "the restart repeated: {:?}",
            k.spawned.len()
        );
    }

    /// **A plan changes nothing at all** — not the flag, not the attempt count,
    /// not the journal. It is `ADR-006` §6.4's `plan` verb and a dry run that
    /// wrote to the card would be one behind a question.
    #[test]
    fn a_plan_changes_nothing_and_says_the_same_thing_the_apply_would() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        s.services[0].attempts = 3;
        let before = k.records.len();

        let planned = s.restart_service(&mut k, "a", false).expect("permitted");
        assert_eq!(s.services[0].asked, Asked::Nothing);
        assert_eq!(s.services[0].attempts, 3, "a plan forgave a failure");
        assert_eq!(k.records.len(), before, "a plan wrote to the journal");

        let applied = s.restart_service(&mut k, "a", true).expect("permitted");
        assert_eq!(
            planned, applied,
            "the plan and the apply described different things"
        );
    }

    /// The three refusals, each from the state that produces it.
    #[test]
    fn the_three_refusals_are_the_whole_of_what_this_surface_will_not_do() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a"), def("b")]);

        let Err(Refusal::UnknownService { known, .. }) = s.restart_service(&mut k, "c", true)
        else {
            unreachable!("an unknown service was accepted")
        };
        assert_eq!(known, vec!["a".to_string(), "b".to_string()]);

        s.begin_shutdown(&mut k, Terminating::PowerOff);
        assert!(matches!(
            s.restart_service(&mut k, "a", true),
            Err(Refusal::ShuttingDown { .. })
        ));
    }

    // -------------------------------------------------------------------
    // `set_proxy_listen` and `service_is_running` — ADR-046 §7, ADR-061 §7,
    // `.rulings/` R-221
    // -------------------------------------------------------------------

    /// **Narrow on purpose: every name but `rivet-proxyd`'s is refused**,
    /// with nothing changed and no service required to exist under that
    /// name at all.
    #[test]
    fn set_proxy_listen_refuses_every_name_but_the_proxys() {
        let mut s = supervisor(vec![def("a")]);
        let before = s.services[0].def.listen.clone();
        assert!(!s.set_proxy_listen("a", vec![rivet_core::package::application::Socket::Tcp(80)]));
        assert_eq!(s.services[0].def.listen, before, "a name that is not the proxy's changed something");

        // Even a name this supervisor has never heard of is refused rather
        // than silently doing nothing for a different reason.
        assert!(!s.set_proxy_listen(
            "rivet-proxyd",
            vec![rivet_core::package::application::Socket::Tcp(80)]
        ));
    }

    /// **The stored value changes, and only it** — no socket is opened or
    /// closed, no state moves, and the return value says whether anything
    /// changed so the caller journals only a real change.
    #[test]
    fn set_proxy_listen_changes_only_the_stored_value() {
        use rivet_core::package::application::Socket;
        let mut proxy = def(rivet_core::service::PROXYD_SERVICE_NAME);
        proxy.listen = vec![Socket::Tcp(443)];
        let mut s = supervisor(vec![proxy]);

        assert!(s.set_proxy_listen(
            rivet_core::service::PROXYD_SERVICE_NAME,
            vec![Socket::Tcp(443), Socket::Tcp(80)]
        ));
        assert_eq!(
            s.services[0].def.listen,
            vec![Socket::Tcp(443), Socket::Tcp(80)]
        );
        assert_eq!(s.services[0].state, State::Idle, "a stored change moved a state");

        // The identical value again changes nothing and says so.
        assert!(!s.set_proxy_listen(
            rivet_core::service::PROXYD_SERVICE_NAME,
            vec![Socket::Tcp(443), Socket::Tcp(80)]
        ));
    }

    /// **`Started` and `Adopted` are "running"; every other state is not** —
    /// the same pair [`Supervisor::restart_service`]'s own `Doing::Stop`
    /// reads as running.
    #[test]
    fn service_is_running_answers_for_started_and_adopted_only() {
        for state in every_state() {
            let (_k, s) = holding(state.clone());
            let expect = matches!(state, State::Started { .. } | State::Adopted { .. });
            assert_eq!(
                s.service_is_running("a"),
                expect,
                "{state:?}: service_is_running disagreed"
            );
        }
        let s = supervisor(vec![def("a")]);
        assert!(
            !s.service_is_running("nothing-by-this-name"),
            "an unknown name must not read as running"
        );
    }

    /// **The answer uses `get status`'s vocabulary**, from one definition —
    /// [`state_word`]. An operator reading `backoff` in a status and `backoff`
    /// in a restart's answer is reading one word (`AGENTS.md` §5).
    #[test]
    fn a_restart_answers_in_the_same_words_get_status_uses() {
        for state in every_state() {
            let (mut k, mut s) = holding(state.clone());
            let answer = s.restart_service(&mut k, "a", false).expect("permitted");
            let status = s.status();
            assert_eq!(
                answer.was, status.services[0].state,
                "{state:?}: two vocabularies"
            );
            // And it survives the wire, which is the only way the CLI sees it.
            assert_eq!(
                rivet_core::operation::service_restart::parse_answer(&answer.render()),
                Ok(Ok(answer))
            );
        }
    }

    /// **`REACHABILITY_SET` is REPORTED, never refused.** The set exists so a
    /// *reload* cannot durably stop the way in (`ADR-032` §3.2.1); a restart
    /// converges and comes back, so §6.3.2's rule reaches it unchanged. What
    /// the caller is owed is the honest caveat, and the supervisor is the
    /// authority for who is in the set.
    #[test]
    fn restarting_a_service_that_carries_the_way_in_is_permitted_and_flagged() {
        let mut k = kernel();
        let mut defs = crate::definitions::system_services();
        for d in &mut defs {
            d.enabled = true;
        }
        let mut s = supervisor(defs);
        for name in crate::definitions::REACHABILITY_SET {
            let answer = s
                .restart_service(&mut k, name, true)
                .unwrap_or_else(|e| panic!("{name} was refused: {e:?}"));
            assert!(
                answer.reachability,
                "{name} is in REACHABILITY_SET and the answer did not say so"
            );
            assert!(
                answer.caveat().is_some_and(|c| c.contains("MAY NOT ARRIVE")),
                "{name} carried no caveat"
            );
        }
        // And an ordinary service is not flagged, or the caveat means nothing.
        let ordinary = s
            .restart_service(&mut k, crate::definitions::TIMED_NAME, true)
            .expect("permitted");
        assert!(!ordinary.reachability);
    }

    /// **A refusal survives the wire too**, which is how an operator ever sees
    /// it: the supervisor renders facts and `rivet-core` composes the sentence.
    #[test]
    fn a_refusal_reaches_the_operator_as_the_sentence_the_domain_composes() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.services[0].def.enabled = false;
        let refused = s.restart_service(&mut k, "a", true).expect_err("disabled");
        let Ok(Err(read_back)) =
            rivet_core::operation::service_restart::parse_answer(&refused.render())
        else {
            unreachable!("a refusal did not survive the wire: {:?}", refused.render())
        };
        assert_eq!(read_back, refused);
        let sentence = read_back.describe();
        assert!(sentence.contains("apply edit"), "{sentence}");
        assert!(sentence.contains("apply reload"), "{sentence}");
    }

    // -----------------------------------------------------------------------
    // ADR-017 §6 — a stop held on one application (`.rulings/` R-183)
    // -----------------------------------------------------------------------

    use rivet_core::operation::service_hold::Doing as HoldDoing;
    use rivet_core::operation::{ServiceHoldKind as HoldKind, ServiceHoldRefusal as HoldRefusal};

    /// An installed, verified, RUNNING application named `a`, on a kernel that
    /// holds its tree — the fixture `begin_uninstall`'s test uses, so a stop
    /// is proven against the start path a real application takes.
    fn a_running_held_candidate() -> (FakeKernel, Supervisor, i32) {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut d = def("a");
        d.origin = Origin::Application;
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        d.program.identity = Some(rivet_core::service::AppIdentity {
            uid: 27_751,
            gid: 27_751,
        });
        let version_dir = d
            .program
            .path
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .expect("a version directory");
        crate::verify::fixture::install_signed(&mut k, "a", &version_dir, &d.program.path);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            panic!("the fixture must be running: {:?}", s.state_of("a"));
        };
        (k, s, pid)
    }

    /// **THE PROPERTY. A stopped application stays stopped**: it is signalled,
    /// it exits, and no number of passes starts it again — the supervisor does
    /// not read the exit as a crash, whatever the restart policy says.
    #[test]
    fn a_stopped_application_is_not_restarted_as_a_crash() {
        let (mut k, mut s, pid) = a_running_held_candidate();
        let answer = s
            .hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("an application may be stopped");
        assert_eq!(answer.was, "started");
        assert_eq!(answer.doing, HoldDoing::Stop);
        assert!(k.signals.is_empty(), "the answer is written before anything is signalled");

        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Stopping { .. })),
            "{:?}",
            s.state_of("a")
        );
        assert!(k.signals.contains(&(pid, Sig::Term)), "{:?}", k.signals);

        // Exit in every way the restart policy (`Always`) would want back.
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        let spawned = k.spawned.len();
        for _ in 0..10 {
            s.converge(&mut k);
        }
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert_eq!(k.spawned.len(), spawned, "a held application was started again");
        assert_eq!(s.services[0].asked, Asked::Stop, "a pass cleared the hold");
        // And `enabled` was never touched — the document's answer is intact.
        assert!(s.services[0].def.enabled);
    }

    /// **A crash while held is not a crash to retry.** A service that dies on
    /// its own after the hold is taken — before the stop reaches it — comes to
    /// rest instead of waiting out a backoff that would start it.
    #[test]
    fn an_application_that_crashes_while_held_comes_to_rest_and_is_not_retried() {
        let (mut k, mut s, pid) = a_running_held_candidate();
        s.hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        let spawned = k.spawned.len();
        for _ in 0..10 {
            s.converge(&mut k);
        }
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert_eq!(k.spawned.len(), spawned, "a held application was retried");
    }

    /// **`decide` never starts a held service from any state**, whatever
    /// `enabled` says. Walked over every state for the restart walk's reason.
    #[test]
    fn decide_never_starts_a_held_service_from_any_state() {
        let l = Limits::default();
        for state in every_state() {
            for enabled in [true, false] {
                let mut d = def("a");
                d.enabled = enabled;
                for now in [0, 9_999, u64::MAX / 2] {
                    let action = decide(Phase::Converging, &d, &state, Asked::Stop, &l, now);
                    assert_ne!(action, Action::Start, "{state:?} enabled={enabled} now={now}");
                    assert_ne!(action, Action::PollAdoption, "{state:?}: an adoption kept");
                }
            }
        }
    }

    /// **Starting it again releases the hold and the next pass starts it**,
    /// because the document enables it. The pair is an inverse.
    #[test]
    fn service_start_releases_the_hold_and_the_application_runs_again() {
        let (mut k, mut s, pid) = a_running_held_candidate();
        s.hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        s.converge(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        let spawned = k.spawned.len();

        let answer = s
            .hold_service(&mut k, HoldKind::Start, "a", true)
            .expect("a held application may be started");
        assert_eq!(answer.was, "idle");
        assert_eq!(answer.doing, HoldDoing::Start);
        assert_eq!(s.services[0].asked, Asked::Nothing);
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "{:?}",
            s.state_of("a")
        );
        assert_eq!(k.spawned.len(), spawned + 1);
    }

    /// **A stop never touches `/state/app/<name>`** — not the versions, not
    /// `current`, not the data. The only removal an application start makes is
    /// its scratch `tmp`, which the fixture's own start already did.
    #[test]
    fn stopping_an_application_touches_nothing_under_its_directory() {
        let (mut k, mut s, pid) = a_running_held_candidate();
        let trees = k.removed_paths();
        let files = k.removed.clone();
        s.hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        for _ in 0..3 {
            s.converge(&mut k);
        }
        let app = "/state/app/a";
        let new_trees: Vec<_> = k
            .removed_paths()
            .into_iter()
            .skip(trees.len())
            .filter(|p| p.starts_with(app) && !p.ends_with("/tmp"))
            .collect();
        let new_files: Vec<_> = k.removed[files.len()..]
            .iter()
            .filter(|p| p.starts_with(app))
            .collect();
        assert!(new_trees.is_empty(), "a stop removed {new_trees:?}");
        assert!(new_files.is_empty(), "a stop removed {new_files:?}");
        assert_eq!(s.services.len(), 1, "a stop removed the service from the set");
    }

    /// **The control plane and the interface bring-up cannot be stopped**, and
    /// the refusal is that reason rather than *not an application*.
    #[test]
    fn stopping_a_service_that_carries_the_way_in_is_refused() {
        let mut k = kernel();
        let mut defs = crate::definitions::system_services();
        for d in &mut defs {
            d.enabled = true;
        }
        let mut s = supervisor(defs);
        for name in crate::definitions::REACHABILITY_SET {
            for apply in [false, true] {
                let refused = s
                    .hold_service(&mut k, HoldKind::Stop, name, apply)
                    .expect_err("the way in was stopped");
                assert!(
                    matches!(refused, HoldRefusal::CarriesTheWayIn { .. }),
                    "{name}: {refused:?}"
                );
            }
            let at = s.index_of(name).expect("in the set");
            assert_eq!(s.services[at].asked, Asked::Nothing, "{name} was marked");
        }
        // Any other image service is refused as not an application.
        assert!(matches!(
            s.hold_service(&mut k, HoldKind::Stop, crate::definitions::TIMED_NAME, true),
            Err(HoldRefusal::NotAnApplication { .. })
        ));
    }

    /// The remaining refusals, each from the state that produces it.
    #[test]
    fn the_hold_refuses_what_it_cannot_honestly_do() {
        let (mut k, mut s, _) = a_running_held_candidate();

        let Err(HoldRefusal::UnknownService { known, .. }) =
            s.hold_service(&mut k, HoldKind::Stop, "b", true)
        else {
            panic!("an unknown service was accepted")
        };
        assert_eq!(known, vec!["a".to_string()]);

        assert!(matches!(
            s.hold_service(&mut k, HoldKind::Start, "a", true),
            Err(HoldRefusal::NotStopped { .. })
        ));

        s.hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        assert!(matches!(
            s.hold_service(&mut k, HoldKind::Stop, "a", true),
            Err(HoldRefusal::AlreadyStopped { .. })
        ));
        // A restart would release the hold as a side effect, so it is refused.
        assert!(matches!(
            s.restart_service(&mut k, "a", true),
            Err(Refusal::Held { .. })
        ));
        assert_eq!(s.services[0].asked, Asked::Stop);

        // A service the document does not enable has nothing to stop.
        let (mut k2, mut s2, _) = a_running_held_candidate();
        s2.set_application_enabled("a", false);
        assert!(matches!(
            s2.hold_service(&mut k2, HoldKind::Stop, "a", true),
            Err(HoldRefusal::NotEnabled { .. })
        ));

        s.begin_shutdown(&mut k, Terminating::PowerOff);
        assert!(matches!(
            s.hold_service(&mut k, HoldKind::Start, "a", true),
            Err(HoldRefusal::ShuttingDown { .. })
        ));
    }

    /// **A plan changes nothing**: not the hold, not the journal.
    #[test]
    fn a_hold_plan_changes_nothing_and_matches_the_apply() {
        let (mut k, mut s, _) = a_running_held_candidate();
        let before = k.records.len();
        let planned = s
            .hold_service(&mut k, HoldKind::Stop, "a", false)
            .expect("permitted");
        assert_eq!(s.services[0].asked, Asked::Nothing);
        assert_eq!(k.records.len(), before, "a plan wrote to the journal");
        let applied = s
            .hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        assert_eq!(planned, applied);
        assert!(k.records.len() > before, "the apply was not journalled");
    }

    /// **`get status` says why a held application is idle**, keeps reporting
    /// `enabled: true`, and the answer survives the wire.
    #[test]
    fn get_status_reports_the_hold_as_the_reason_and_enabled_unchanged() {
        let (mut k, mut s, pid) = a_running_held_candidate();
        s.hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        s.converge(&mut k);
        let status = s.status();
        let row = &status.services[0];
        assert_eq!(row.state, "idle");
        assert!(row.enabled);
        assert!(
            row.reason.as_deref().is_some_and(|r| r.contains("service-start")),
            "{:?}",
            row.reason
        );
        assert_eq!(
            rivet_core::operation::Status::parse(&status.render()),
            Ok(status)
        );
    }

    /// **A reload does not release a hold.** The document still enables the
    /// application and says nothing about the hold, so nothing changes.
    #[test]
    fn a_reapplied_document_leaves_the_hold_in_place() {
        let (mut k, mut s, pid) = a_running_held_candidate();
        s.hold_service(&mut k, HoldKind::Stop, "a", true)
            .expect("permitted");
        s.converge(&mut k);
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        // The document is taken again, with the same answer and a flip.
        s.set_enabled("a", false);
        s.set_enabled("a", true);
        let spawned = k.spawned.len();
        for _ in 0..3 {
            s.converge(&mut k);
        }
        assert_eq!(k.spawned.len(), spawned, "a reload released the hold");
    }

    /// **A fresh supervisor has no holds** — which is what a reboot, or a
    /// restart of the supervisor, builds. The machine returns to its document.
    #[test]
    fn a_new_supervisor_starts_with_no_hold_so_a_reboot_returns_to_the_document() {
        let (_, s, _) = a_running_held_candidate();
        assert!(s.services.iter().all(|m| m.asked == Asked::Nothing));
    }

    /// **Both answers survive the wire** as the domain reads them.
    #[test]
    fn a_hold_answer_and_refusal_survive_the_wire() {
        let (mut k, mut s, _) = a_running_held_candidate();
        let answer = s
            .hold_service(&mut k, HoldKind::Stop, "a", false)
            .expect("permitted");
        assert_eq!(
            rivet_core::operation::service_hold::parse_answer(&answer.render()),
            Ok(Ok(answer))
        );
        let refused = s
            .hold_service(&mut k, HoldKind::Start, "a", true)
            .expect_err("not held");
        assert_eq!(
            rivet_core::operation::service_hold::parse_answer(&refused.render()),
            Ok(Err(refused))
        );
    }

    // -----------------------------------------------------------------------
    // ADR-042: the Landlock ruleset
    // -----------------------------------------------------------------------

    /// An application, verified and startable, with a policy on it.
    fn confinable(k: &mut FakeKernel, name: &str) -> ServiceDef {
        let d = listening_def(k, name, Vec::new());
        assert!(
            d.program.confinement.is_some(),
            "the fixture carries no policy, so nothing below is about ADR-042"
        );
        d
    }

    /// **The ruling of `ADR-042` §5, as a test.** A kernel with no Landlock
    /// does not get a less confined application; it gets no application.
    ///
    /// The refusal is the whole of the argument: a Rivet ships its kernel and
    /// its image in one payload (`ADR-018` §5.2), so this state is a broken
    /// build of Rivet's own kernel rather than a machine an operator chose,
    /// and starting anyway would keep a promise the journal has already made.
    #[test]
    fn an_application_is_not_started_on_a_kernel_without_landlock() {
        let mut k = kernel();
        k.landlock = (0, libc::ENOSYS);
        let d = confinable(&mut k, "a");
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Failed)),
            "it started an application it cannot confine: {:?}",
            s.state_of("a")
        );
        assert!(
            k.spawned.is_empty(),
            "it spawned something after refusing: {:?}",
            k.spawned
        );
        assert!(
            k.records
                .iter()
                .any(|r| r.svc == "a" && r.msg.contains("no Landlock")),
            "the refusal does not name what is missing: {:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// And the other direction, because a refusal that fired for every kernel
    /// would pass the test above and stop the machine working.
    #[test]
    fn the_same_application_starts_on_a_kernel_that_has_it() {
        let mut k = kernel();
        k.landlock = (crate::landlock::MINIMUM_ABI, 0);
        let d = confinable(&mut k, "a");
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Started { .. })),
            "it refused an application on a kernel that can confine it: {:?}",
            s.state_of("a")
        );
        let spawned = k
            .spawned
            .iter()
            .find(|(name, _)| name == "a")
            .map(|(_, p)| p.clone())
            .expect("it was spawned");
        let policy = spawned
            .confinement
            .expect("the program handed to the kernel carries the ruleset it is to be given");
        assert!(
            policy
                .grants
                .iter()
                .any(|g| g.path == "/state/app/a/data" && g.access.holds(rivet_core::confinement::Access::WRITE)),
            "the writable directory is not in the policy the kernel was handed: {:?}",
            policy.grants
        );
    }

    /// **The device nodes are added to the policy, and only the ones the glob
    /// actually found.** `ADR-011`: the chown and the ruleset are the same
    /// list, walked once.
    #[test]
    fn a_declared_device_reaches_the_ruleset_as_well_as_the_chown() {
        use rivet_core::package::application::Device;

        let mut k = kernel();
        // devtmpfs creates what the hardware needs; this machine has one event
        // node, so the glob finds one and the policy should carry exactly it.
        k.dir_entries
            .insert("/dev/input".to_string(), vec!["event0".to_string()]);
        let mut d = confinable(&mut k, "a");
        d.devices = vec![Device::Input];
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        let spawned = k
            .spawned
            .iter()
            .find(|(name, _)| name == "a")
            .map(|(_, p)| p.clone())
            .expect("it was spawned");
        let policy = spawned.confinement.expect("a policy");
        let node = policy
            .grants
            .iter()
            .find(|g| g.path == "/dev/input/event0")
            .expect("the node the class expanded to is not in the ruleset");
        assert!(
            node.access.holds(rivet_core::confinement::Access::DEVICE_CONTROL),
            "a device node granted without ioctl is one the application cannot drive"
        );
        assert!(
            !policy.grants.iter().any(|g| g.path == "/dev/input"),
            "the whole directory was granted rather than the nodes the glob found: {:?}",
            policy.grants
        );
    }

    /// **The paths a confined launcher used to READ are not in any policy, and
    /// the ones it is HANDED are.** `ADR-042` §8, 2026-09-05.
    ///
    /// This is the regression, as a test at the supervisor's own boundary. The
    /// launcher was refused `/system/share/fonts/Geist.ttf`,
    /// `/state/config/machine.toml` and a walk of `/state/app`; the first was
    /// the policy being wrong about fonts and the other two were the launcher
    /// being wrong about reading the operator's document. So the assertion is
    /// asymmetric on purpose: the fonts must be reachable and the document must
    /// not.
    #[test]
    fn the_ruleset_a_launcher_is_given_reaches_the_fonts_and_not_the_document() {
        let mut k = kernel();
        k.landlock = (crate::landlock::MINIMUM_ABI, 0);
        let d = confinable(&mut k, "rivet-launcher");
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        let spawned = k
            .spawned
            .iter()
            .find(|(name, _)| name == "rivet-launcher")
            .map(|(_, p)| p.clone())
            .expect("it was spawned");
        let policy = spawned.confinement.expect("a policy");
        let has = |path: &str| policy.grants.iter().any(|g| g.path == path);

        assert!(
            has(rivet_core::etc::SYSTEM_FONT_DIR),
            "the typefaces are not in the ruleset, so this launcher draws a screen with no text \
             on it: {:?}",
            policy.grants
        );
        assert!(
            has("/state/app/rivet-launcher/settings"),
            "RIVET_APP_SETTINGS is handed to this program and is not in its ruleset: {:?}",
            policy.grants
        );
        for refused in [
            "/state/config",
            "/state/config/machine.toml",
            "/state/app",
            "/system",
            "/system/bin",
        ] {
            assert!(
                !has(refused),
                "{refused} is granted; the launcher asks `offer` on the kiosk socket for what it \
                 used to read there (ADR-042 §8.2), so nothing needs this"
            );
        }
    }

    // -----------------------------------------------------------------------
    // ADR-042 §8.2: the `offer` verb
    // -----------------------------------------------------------------------

    /// **What the launcher is told is what `select` will accept.**
    ///
    /// The property the verb exists for. Until 2026-09-05 the launcher computed
    /// its own offered set from the document and `/state/app`, so a menu could
    /// name something the supervisor then refused; now both come from
    /// `display_offered` and there is one answer.
    #[test]
    fn the_offer_answer_is_the_set_select_validates_against() {
        use rivet_core::package::application::Device;

        let mut k = kernel();
        let mut menu = kiosk_def(&mut k, "kiosk-demo", vec![Device::Display]);
        menu.enabled = true;
        let mut other = kiosk_def(&mut k, "aura-renderer", vec![Device::Display]);
        other.enabled = true;
        // A daemon: enabled, and no business on a screen list.
        let mut daemon = listening_def(&mut k, "rivet-mdnsd", Vec::new());
        daemon.enabled = true;
        // The launcher itself, which declares a display and must never be on
        // its own menu.
        let launcher = controlling_the_kiosk({
            let mut d = kiosk_def(&mut k, "rivet-launcher", vec![Device::Display]);
            d.enabled = true;
            d
        });

        let mut s = supervisor(vec![menu, other, daemon, launcher]);
        s.set_kiosk_mode(rivet_core::operation::status::KioskMode::kiosk(
            true,
            Some("aura-renderer".to_string()),
        ));

        let (mode, locked, auto_run, offered) = s.kiosk_offer();
        assert_eq!(mode, rivet_core::operation::status::MODE_KIOSK);
        assert!(locked);
        assert_eq!(auto_run.as_deref(), Some("aura-renderer"));
        let mut sorted = offered.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["aura-renderer".to_string(), "kiosk-demo".to_string()],
            "the offer answer is not the offered set: {offered:?}"
        );
        // And the two halves genuinely agree: everything offered is selectable.
        for name in &offered {
            assert!(
                s.select_display(name).is_ok(),
                "`{name}` was offered to the launcher and `select` refused it, which is the \
                 disagreement this verb exists to remove"
            );
        }
    }

    /// **The launcher and `get status` cannot disagree, because they read the
    /// same value.** `crate::document::kiosk_mode` is the one place `mode` and
    /// `[kiosk]` are reconciled (`.rulings/` R-164, R-165); `set_kiosk_mode`
    /// carries its answer in, and `kiosk_offer` — the launcher's `offer` — and
    /// `get status`'s `kiosk` line are both reads of the same `self.kiosk`.
    /// This proves the `mode = "kiosk"`, no-table case: asked for, not
    /// refused, and identical on both surfaces.
    #[test]
    fn kiosk_mode_and_the_launchers_offer_answer_the_same_way_when_unconfigured() {
        let mut s = supervisor(Vec::new());
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, ACTOR.to_string());
        let path = rivet_store::ConfigStore::at_default().machine_path();
        k.files_mut().insert(
            path,
            Script::always(Ok("version = 1\nmode = \"kiosk\"\n".to_string())),
        );
        let desired = crate::document::load(&mut k, &mut j);
        let computed = crate::document::kiosk_mode(&desired);
        assert!(computed.is_kiosk());
        assert!(computed.why_unconfigured.is_some());

        s.set_kiosk_mode(computed.clone());
        let (mode, locked, auto_run, _offered) = s.kiosk_offer();
        assert_eq!(mode, computed.mode, "the launcher's mode must be sysd's mode");
        assert_eq!(locked, computed.locked);
        assert_eq!(auto_run, computed.auto_run);
        assert!(!locked, "asked-for-but-not-configured behaves as headless does today");
        assert!(auto_run.is_none());
    }

    /// **A supervisor that was never told answers `unknown`, not `headless`.**
    ///
    /// `KioskMode::unknown` is the constructed state, and a launcher that read
    /// it as *headless, unlocked* would draw a menu on a machine that might be
    /// locked. The distinction has to survive the wire.
    #[test]
    fn a_supervisor_that_was_never_told_says_so_on_the_kiosk_socket() {
        let s = supervisor(Vec::new());
        let (mode, locked, auto_run, offered) = s.kiosk_offer();
        assert_eq!(mode, rivet_core::operation::status::MODE_UNKNOWN);
        assert!(!locked);
        assert!(auto_run.is_none());
        assert!(offered.is_empty());

        let rendered = rivet_core::kiosk::Answer::Offer {
            mode,
            locked,
            auto_run,
            offered,
        }
        .render();
        let read_back = rivet_core::kiosk::Answer::parse(&rendered);
        assert!(
            matches!(read_back, rivet_core::kiosk::Answer::Offer { ref mode, .. }
                if mode == rivet_core::operation::status::MODE_UNKNOWN),
            "the launcher cannot tell `nobody told me` from `this is a headless box`: {rendered}"
        );
    }

    /// **The image's proxy is spawned as its own uid, under a ruleset that
    /// grants its program, and told where its data is** — `ADR-060`, from the
    /// spawn record rather than from the definition, so what reaches the kernel
    /// is what is asserted.
    #[test]
    fn the_image_proxy_is_spawned_as_its_own_uid_under_a_ruleset() {
        let mut proxy = crate::definitions::system_services()
            .into_iter()
            .find(|d| d.name == rivet_core::service::PROXYD_SERVICE_NAME)
            .expect("the image defines the proxy");
        proxy.enabled = true;
        let mut k = kernel();
        let mut s = supervisor(vec![proxy]);
        s.converge(&mut k);

        let (name, program) = k.spawned.first().expect("the proxy was spawned");
        assert_eq!(name, rivet_core::service::PROXYD_SERVICE_NAME);
        let own = rivet_core::service::app_identity(
            &rivet_core::service::ServiceName::new(rivet_core::service::PROXYD_SERVICE_NAME)
                .expect("a name"),
        );
        assert_eq!(program.identity, Some(own));
        assert_ne!(own.uid, 0, "the front door is not root");
        let policy = program.confinement.as_ref().expect("the spawn carries a ruleset");
        assert!(
            policy.grants.iter().any(|g| g.path == rivet_core::service::PROXYD_PATH
                && g.access.holds(rivet_core::confinement::Access::EXECUTE)),
            "the ruleset does not let it exec its own program"
        );
        assert!(
            program.env.iter().any(|(key, value)| key == rivet_core::service::APP_DATA_ENV
                && value == "/state/app/rivet-proxyd/data"),
            "{:?}",
            program.env
        );
    }

    /// The image's BLE observer, enabled, and its name.
    fn enabled_ble_observer() -> (ServiceDef, String) {
        let mut observer = crate::definitions::system_services()
            .into_iter()
            .find(|d| d.name == crate::definitions::BLUETOOTHD_NAME)
            .expect("the image defines the BLE observer");
        observer.enabled = true;
        let name = observer.name.clone();
        (observer, name)
    }

    /// `cmd/rivet-bluetoothd`'s exit when its controller goes away. A failure,
    /// so `on-failure` restarts it.
    const CONTROLLER_GONE: i32 = 4;

    /// **A controller unplugged and plugged back in, again and again, never
    /// ends the machine's Bluetooth** — `ADR-041` §6, `.rulings/` R-253.
    /// `rivet-bluetoothd` exits when its controller goes away and the
    /// supervisor brings it back. A scan that ran for the success window is a
    /// success, so each of those exits wipes the give-up count and the
    /// threshold is never reached, however many cycles there are.
    #[test]
    fn a_ble_observer_that_loses_its_controller_after_a_long_scan_is_restarted_every_time() {
        let (observer, name) = enabled_ble_observer();
        let mut k = kernel();
        let mut s = supervisor(vec![observer]);
        let cycles = Limits::default().give_up_after * 3;
        for cycle in 0..cycles {
            s.converge(&mut k);
            let Some(State::Started { pid, .. }) = s.state_of(&name).cloned() else {
                panic!("cycle {cycle}: not started, {:?}", s.state_of(&name));
            };
            k.mono_ms += Limits::default().success_window_ms;
            k.exits.push((pid, ExitStatus::Exited(CONTROLLER_GONE)));
            s.reap(&mut k);
            let Some(State::Backoff { until_ms }) = s.state_of(&name).cloned() else {
                panic!("cycle {cycle}: not backing off, {:?}", s.state_of(&name));
            };
            assert_eq!(s.services()[0].attempts, 1, "cycle {cycle}: the slate was not wiped");
            k.mono_ms = until_ms;
        }
        s.converge(&mut k);
        assert!(
            matches!(s.state_of(&name), Some(State::Started { .. })),
            "{:?}",
            s.state_of(&name)
        );
        assert!(
            !k.records.iter().any(|r| r.msg.starts_with("giving up")),
            "an ordinary unplug and replug ended the machine's Bluetooth"
        );
    }

    /// **The bound, stated as a test so it is not a surprise.** Five losses,
    /// each before the success window, is a controller that will not stay up,
    /// and the supervisor gives up exactly as it would on any service.
    #[test]
    fn a_ble_observer_that_loses_its_controller_five_times_inside_the_window_is_given_up_on() {
        let (observer, name) = enabled_ble_observer();
        let mut k = kernel();
        let mut s = supervisor(vec![observer]);
        for _ in 0..Limits::default().give_up_after {
            s.converge(&mut k);
            let Some(State::Started { pid, .. }) = s.state_of(&name).cloned() else {
                break;
            };
            k.mono_ms += Limits::default().success_window_ms - 1;
            k.exits.push((pid, ExitStatus::Exited(CONTROLLER_GONE)));
            s.reap(&mut k);
            if let Some(State::Backoff { until_ms }) = s.state_of(&name).cloned() {
                k.mono_ms = until_ms;
            }
        }
        s.converge(&mut k);
        assert_eq!(s.state_of(&name), Some(&State::Failed));
    }

    // -----------------------------------------------------------------------
    // S-10: a refusal in `start()` after the device grant releases what the
    // grant gave, rather than leaving it owned by an application's uid with
    // nothing running to use it.
    // -----------------------------------------------------------------------

    /// A board with one input event node — the same fixture
    /// `a_declared_device_reaches_the_ruleset_as_well_as_the_chown` uses —
    /// so `crate::devices::grant` has something real to hand out.
    fn with_one_input_node(k: &mut FakeKernel) {
        k.dir_entries
            .insert("/dev/input".to_string(), vec!["event0".to_string()]);
    }

    /// A grant was made (chowned away from root) and then given back
    /// (chowned back to `0:0`) for the same path — `crate::devices::revoke`'s
    /// own signature, and the only shape that proves `Self::release` ran
    /// rather than merely that a refusal happened.
    fn granted_then_released(chowns: &[(String, u32, u32)], path: &str) -> (bool, bool) {
        let granted = chowns
            .iter()
            .any(|(p, u, g)| p == path && (*u, *g) != (0, 0));
        let released = chowns
            .iter()
            .any(|(p, u, g)| p == path && *u == 0 && *g == 0);
        (granted, released)
    }

    #[test]
    fn a_refusal_with_no_landlock_releases_the_device_the_grant_gave() {
        let mut k = kernel();
        k.landlock = (0, libc::ENOSYS);
        with_one_input_node(&mut k);
        let mut d = confinable(&mut k, "a");
        d.devices = vec![Device::Input];
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Failed)),
            "the fixture did not reach the no-Landlock refusal: {:?}",
            s.state_of("a")
        );
        let (granted, released) = granted_then_released(&k.chowns, "/dev/input/event0");
        assert!(granted, "no grant happened, so this proves nothing: {:?}", k.chowns);
        assert!(
            released,
            "S-10: the no-Landlock refusal after the grant did not release the device: {:?}",
            k.chowns
        );
    }

    #[test]
    fn a_refused_connects_grant_releases_the_device_the_grant_gave() {
        use rivet_core::package::application::Endpoint;

        let mut k = kernel();
        with_one_input_node(&mut k);
        let mut d = confinable(&mut k, "a");
        d.devices = vec![Device::Input];
        // No such provider is enabled — `endpoints::resolve` refuses this
        // with `NoSuchProvider`, which `start()` reports as `Failed` before
        // ever reaching the spawn.
        d.connects = vec![
            Endpoint::parse("connects", "no-such-provider:socket").unwrap_or_else(|_| unreachable!()),
        ];
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Failed)),
            "the fixture did not reach the connects refusal: {:?}",
            s.state_of("a")
        );
        assert!(k.spawned.is_empty(), "it spawned after refusing: {:?}", k.spawned);
        let (granted, released) = granted_then_released(&k.chowns, "/dev/input/event0");
        assert!(granted, "no grant happened, so this proves nothing: {:?}", k.chowns);
        assert!(
            released,
            "S-10: the connects refusal after the grant did not release the device: {:?}",
            k.chowns
        );
    }

    /// **A refusal the enabled set still earns is not retried and not said
    /// again.** `Supervisor::retry_refused_connects` runs every pass, and a
    /// machine runs for months: a consumer whose provider stays switched off
    /// must cost the journal one record, not one per pass.
    #[test]
    fn a_consumer_still_refused_stays_failed_and_is_said_once() {
        use rivet_core::package::application::Endpoint;

        let mut k = kernel();
        let mut d = confinable(&mut k, "a");
        d.connects = vec![
            Endpoint::parse("connects", "no-such-provider:socket").unwrap_or_else(|_| unreachable!()),
        ];
        let mut s = supervisor(vec![d]);
        for _ in 0..5 {
            s.converge(&mut k);
        }
        assert!(matches!(s.state_of("a"), Some(State::Failed)), "{:?}", s.state_of("a"));
        assert!(k.spawned.is_empty(), "it spawned while refused: {:?}", k.spawned);
        let said = k
            .records
            .iter()
            .filter(|r| r.svc == "a" && r.msg.contains("connection was refused"))
            .count();
        assert_eq!(said, 1, "the refusal was repeated across passes");
        assert!(
            !k.records.iter().any(|r| r.msg.contains("now resolve")),
            "a retry was announced for a refusal that still holds"
        );
    }

    #[test]
    fn a_grant_chown_failure_releases_the_device_the_grant_gave() {
        use rivet_core::package::application::Endpoint;

        let mut k = kernel();
        with_one_input_node(&mut k);
        let mut provider = confinable(&mut k, "a-provider");
        provider.provides =
            vec![Endpoint::parse("provides", "socket:api").unwrap_or_else(|_| unreachable!())];
        provider.listen = vec![Socket::Unix("api".to_string())];

        let mut consumer = confinable(&mut k, "b-consumer");
        consumer.devices = vec![Device::Input];
        consumer.connects =
            vec![Endpoint::parse("connects", "a-provider:api").unwrap_or_else(|_| unreachable!())];

        // The chown calls, in the order this fixture actually makes them
        // (read off a failing run rather than assumed — `AGENTS.md` §6):
        // 1. the provider re-granting its own socket to the consumer;
        // 2. the provider's OWN scratch directory, chowned at ITS spawn
        //    (`Self::empty_scratch`, called just before every `Kernel::spawn`);
        // 3. the consumer's device grant, which MUST succeed here — a failed
        //    one is reported and skipped rather than fatal
        //    (`crate::devices::grant`'s own contract), so scripting it to
        //    fail would leave nothing granted for `Self::release` to give
        //    back and prove nothing;
        // 4. the consumer's OWN connects grant of the same socket — refused.
        // Everything after (including `release`'s own revoke) succeeds,
        // which is `Script`'s `forever` value.
        k.chown_results = Script::sequence(vec![Ok(()), Ok(()), Ok(()), Err(Errno::EIO)], Ok(()));

        let mut s = supervisor(vec![provider, consumer]);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("b-consumer"), Some(State::Failed)),
            "the fixture did not reach the grant-chown-failure refusal: {:?}",
            s.state_of("b-consumer")
        );
        let (granted, released) = granted_then_released(&k.chowns, "/dev/input/event0");
        assert!(granted, "no grant happened, so this proves nothing: {:?}", k.chowns);
        assert!(
            released,
            "S-10: a grant-chown failure after the device grant did not release the device: {:?}",
            k.chowns
        );
    }

    #[test]
    fn a_program_this_image_cannot_run_releases_the_device_the_grant_gave() {
        let mut k = kernel();
        with_one_input_node(&mut k);
        let mut d = confinable(&mut k, "a");
        d.devices = vec![Device::Input];
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Failed)),
            "the fixture did not reach the cannot-run refusal: {:?}",
            s.state_of("a")
        );
        assert!(k.spawned.is_empty(), "it spawned after refusing: {:?}", k.spawned);
        let (granted, released) = granted_then_released(&k.chowns, "/dev/input/event0");
        assert!(granted, "no grant happened, so this proves nothing: {:?}", k.chowns);
        assert!(
            released,
            "S-10: a program this image cannot run left the device owned by the app after the \
             grant: {:?}",
            k.chowns
        );
    }

    /// **A-1.** A consumer whose provider has not bound its socket yet must be
    /// retried, not failed for good — services converge in name order, so on
    /// a first boot any consumer that sorts before its provider hits this
    /// every time. Mirrors [`a_grant_chown_failure_releases_the_device_the_grant_gave`],
    /// with `ENOENT` where that test uses `EIO`, because the two must NOT be
    /// treated alike.
    #[test]
    fn a_grant_chown_enoent_is_retried_rather_than_failed_for_good() {
        use rivet_core::package::application::Endpoint;

        let mut k = kernel();
        let mut provider = confinable(&mut k, "a-provider");
        provider.provides =
            vec![Endpoint::parse("provides", "socket:api").unwrap_or_else(|_| unreachable!())];
        provider.listen = vec![Socket::Unix("api".to_string())];

        let mut consumer = confinable(&mut k, "b-consumer");
        consumer.connects =
            vec![Endpoint::parse("connects", "a-provider:api").unwrap_or_else(|_| unreachable!())];

        // The calls this fixture makes, in order (as
        // `a_grant_chown_failure_releases_the_device_the_grant_gave` reads
        // them off a failing run): the provider's own re-grant of its socket
        // to the consumer, then the provider's own scratch directory at its
        // spawn — both succeed, the provider starts normally — and then the
        // consumer's OWN connects grant, which fails with `ENOENT` exactly
        // once, as it would while the provider has bound the socket but this
        // consumer has not yet been granted it. Every call after is `Ok`,
        // `Script`'s `forever` value: the retry's own connects grant, and
        // then the consumer's own scratch directory once it finally spawns.
        k.chown_results = Script::sequence(vec![Ok(()), Ok(()), Err(Errno::ENOENT)], Ok(()));

        let mut s = supervisor(vec![provider, consumer]);
        s.converge(&mut k);

        assert!(
            !matches!(s.state_of("b-consumer"), Some(State::Failed)),
            "A-1: an ENOENT chown on the endpoint grant was treated as a permanent refusal \
             instead of a transient one: {:?}",
            s.state_of("b-consumer")
        );
        let Some(State::Backoff { until_ms }) = s.state_of("b-consumer").cloned() else {
            unreachable!(
                "expected the consumer waiting out a backoff after the transient refusal, not: \
                 {:?}",
                s.state_of("b-consumer")
            )
        };

        // Past the backoff, the consumer is retried — and this time the
        // provider has bound the socket, so the grant succeeds.
        k.mono_ms = until_ms;
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("b-consumer"), Some(State::Started { .. })),
            "the consumer never came up once the transient refusal cleared: {:?}",
            s.state_of("b-consumer")
        );
    }

    // -----------------------------------------------------------------------
    // S-1: a confined app can outlive stop, uninstall and revocation. Every
    // stop edge now kills the whole cgroup and waits for it to empty before
    // the service rests at Idle.
    // -----------------------------------------------------------------------

    #[test]
    fn a_draining_service_within_its_deadline_and_before_its_poll_does_nothing() {
        let l = Limits::default();
        let draining = State::Draining {
            deadline_ms: 10_000,
            killed: false,
            next_poll_ms: 5_000,
            then: DrainRest::Idle,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &draining, Asked::Nothing, &l, 4_999),
            Action::Nothing
        );
    }

    #[test]
    fn a_draining_service_past_its_poll_time_is_asked_again() {
        let l = Limits::default();
        let draining = State::Draining {
            deadline_ms: 10_000,
            killed: false,
            next_poll_ms: 5_000,
            then: DrainRest::Idle,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &draining, Asked::Nothing, &l, 5_000),
            Action::PollDrain
        );
    }

    #[test]
    fn a_draining_service_past_its_deadline_escalates_when_not_yet_killed() {
        let l = Limits::default();
        let draining = State::Draining {
            deadline_ms: 10_000,
            killed: false,
            next_poll_ms: 5_000,
            then: DrainRest::Idle,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &draining, Asked::Nothing, &l, 10_000),
            Action::EscalateDrain
        );
    }

    #[test]
    fn a_draining_service_past_its_deadline_is_abandoned_once_already_killed() {
        let l = Limits::default();
        let draining = State::Draining {
            deadline_ms: 10_000,
            killed: true,
            next_poll_ms: 5_000,
            then: DrainRest::Idle,
        };
        assert_eq!(
            decide(Phase::Converging, &def("a"), &draining, Asked::Nothing, &l, 10_000),
            Action::AbandonDrain
        );
    }

    /// **Acceptance (a), the started half.** A helper the main pid forked,
    /// and that ignores `SIGTERM`, stays in the cgroup after the main pid is
    /// reaped — and this service does not rest at `Idle` while it is there.
    ///
    /// The stop is an explicit one (`enabled = false`), not a bare crash:
    /// `def("a")`'s policy is `RestartPolicy::Always`, and an UNASKED exit
    /// that wants restarting goes to `Backoff`, not through
    /// `Self::begin_drain` at all — `Self::start`'s own leftover check is
    /// what catches THAT case, on the next attempt, and has its own test.
    #[test]
    fn a_stop_of_a_started_service_kills_the_cgroup_and_waits_for_it_to_empty_before_idle() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };

        s.services[0].def.enabled = false;
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Stopping { .. })),
            "{:?}",
            s.state_of("a")
        );

        k.observations = Script::always(Observation::Members(vec![777]));
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "the exit must not settle to Idle while the cgroup still holds a process: {:?}",
            s.state_of("a")
        );
        assert!(
            k.signals.contains(&(777, Sig::Term)),
            "the leftover was never signalled: {:?}",
            k.signals
        );

        // The leftover actually exits. Past the poll cadence, or nothing
        // re-asks the cgroup.
        k.observations = Script::always(Observation::Empty);
        k.mono_ms = Limits::default().adopt_poll_ms;
        s.converge(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert!(
            k.records.iter().any(|r| r.msg == "the cgroup emptied; stopped"),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **What `rivet-netd` leaves in its cgroup is signalled when it exits** —
    /// the reason a DHCP client or a supplicant cannot be netd's child. The
    /// sweep is about the cgroup, not the program, so whatever netd started
    /// and left running — `udhcpc` renewing a lease, `wpa_supplicant` holding
    /// an association — is sent `SIGTERM` the moment netd's clean exit is
    /// reaped. R-253 point 1 moved both into services of their own; this pins
    /// the sweep they had to move out of the way of.
    #[test]
    fn anything_netd_leaves_in_its_cgroup_is_signalled_when_netd_exits() {
        let netd = crate::definitions::system_services()
            .into_iter()
            .find(|d| d.name == crate::definitions::NETD_NAME)
            .expect("the image defines netd");
        let mut k = kernel();
        let mut s = supervisor(vec![netd]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of(crate::definitions::NETD_NAME).cloned()
        else {
            unreachable!("netd did not start")
        };

        // The client or supplicant it started, still in its cgroup.
        let left_behind = 4242;
        k.observations = Script::always(Observation::Members(vec![left_behind]));
        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert!(
            k.signals.contains(&(left_behind, Sig::Term)),
            "a process left in netd's cgroup was not signalled: {:?}",
            k.signals
        );
        assert!(
            k.records.iter().any(|r| r.msg.starts_with("the cgroup still held a process")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **A supplicant the supervisor restarts after `rivet-netd` has run gets
    /// netd run again**, because the network it joins lives only in its
    /// memory and netd is what tells it.
    #[test]
    fn a_supplicant_restarted_after_netd_ran_runs_netd_again() {
        let netd = crate::definitions::system_services()
            .into_iter()
            .find(|d| d.name == crate::definitions::NETD_NAME)
            .expect("the image defines netd");
        let document = rivet_config::Document::parse(
            "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\nssid = \"home\"\n",
        )
        .expect("a valid document");
        let mut defs = vec![netd];
        defs.extend(crate::definitions::network_services(&document));
        let supplicant = rivet_core::service::supplicant_service_name("wlan0");

        let mut k = kernel();
        let mut s = supervisor(defs);
        s.converge(&mut k);
        let netd_spawns = |k: &FakeKernel| {
            k.spawned
                .iter()
                .filter(|(n, _)| n == crate::definitions::NETD_NAME)
                .count()
        };
        assert_eq!(netd_spawns(&k), 1, "at boot netd and the supplicant start together");

        let Some(State::Started { pid: netd_pid, .. }) =
            s.state_of(crate::definitions::NETD_NAME).cloned()
        else {
            unreachable!("netd did not start")
        };
        k.exits.push((netd_pid, ExitStatus::Exited(0)));
        s.reap(&mut k);
        assert_eq!(s.state_of(crate::definitions::NETD_NAME), Some(&State::Done));

        let Some(State::Started { pid: wpa_pid, .. }) = s.state_of(&supplicant).cloned() else {
            unreachable!("the supplicant did not start")
        };
        k.exits.push((wpa_pid, ExitStatus::Signalled(9)));
        s.reap(&mut k);
        // Past any backoff, so the supplicant is started again.
        k.mono_ms = 600_000;
        s.converge(&mut k);
        assert!(
            matches!(s.state_of(&supplicant), Some(State::Started { .. })),
            "{:?}",
            s.state_of(&supplicant)
        );
        s.converge(&mut k);
        assert_eq!(
            netd_spawns(&k),
            2,
            "netd was not run again to tell the restarted supplicant its network: {:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **Acceptance (a), the adopted half.**
    #[test]
    fn a_stop_of_an_adopted_service_kills_the_cgroup_and_waits_for_it_to_empty_before_idle() {
        let mut k = kernel();
        k.observations = Script::always(Observation::Members(vec![101, 102]));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Adopted { .. })),
            "the fixture did not adopt: {:?}",
            s.state_of("a")
        );

        s.services[0].def.enabled = false;
        s.converge(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "disabling an adopted service whose cgroup never empties must not settle to Idle: \
             {:?}",
            s.state_of("a")
        );

        // Past the poll cadence, or nothing re-asks the cgroup.
        k.observations = Script::always(Observation::Empty);
        k.mono_ms = Limits::default().adopt_poll_ms;
        s.converge(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
    }

    /// **Acceptance (a): "An adopted service that ignores SIGTERM is
    /// SIGKILLed at the deadline."** Carried through both of `Draining`'s
    /// own deadlines: escalate once the first passes, then abandon — rather
    /// than wait for ever — if the cgroup still will not empty after that.
    #[test]
    fn a_draining_service_that_ignores_sigterm_is_killed_at_the_deadline() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };
        s.services[0].def.enabled = false;
        s.converge(&mut k);

        k.observations = Script::always(Observation::Members(vec![777]));
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        let State::Draining { deadline_ms, .. } = *s.state_of("a").expect("draining") else {
            unreachable!("expected Draining: {:?}", s.state_of("a"))
        };

        // Still there when the grace period expires: escalate.
        k.mono_ms = deadline_ms;
        s.converge(&mut k);
        assert_eq!(k.cgroup_kills, vec!["a".to_string()]);
        assert!(
            matches!(s.state_of("a"), Some(State::Draining { killed: true, .. })),
            "{:?}",
            s.state_of("a")
        );

        // And it STILL will not go, even after the hard kill: abandon at the
        // second deadline rather than wait for ever.
        let State::Draining {
            deadline_ms: second, ..
        } = *s.state_of("a").expect("still draining")
        else {
            unreachable!()
        };
        k.mono_ms = second;
        s.converge(&mut k);
        assert_eq!(s.state_of("a"), Some(&State::Idle));
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.starts_with("the cgroup did not empty even after being killed")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// The preferred `Kernel::cgroup_kill` is always tried first; when a
    /// kernel does not have it, the escalation falls back to a manual
    /// `SIGKILL` of whatever the cgroup currently holds.
    #[test]
    fn escalation_falls_back_to_a_manual_sigkill_sweep_when_cgroup_kill_is_unavailable() {
        let mut k = kernel();
        k.cgroup_kill_results = Script::always(Err(Errno(libc::ENOSYS)));
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };
        s.services[0].def.enabled = false;
        s.converge(&mut k);

        k.observations = Script::always(Observation::Members(vec![777]));
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);
        let State::Draining { deadline_ms, .. } = *s.state_of("a").expect("draining") else {
            unreachable!()
        };

        k.mono_ms = deadline_ms;
        s.converge(&mut k);

        assert_eq!(
            k.cgroup_kills,
            vec!["a".to_string()],
            "the atomic kill must still be TRIED first even though it is scripted to fail"
        );
        assert!(
            k.signals.contains(&(777, Sig::Kill)),
            "the fallback never sent SIGKILL to what the cgroup still held: {:?}",
            k.signals
        );
    }

    /// **Acceptance (b): "An uninstall does not delete the data directory
    /// while the cgroup still holds a process."**
    #[test]
    fn an_uninstall_does_not_remove_the_tree_while_the_cgroup_still_holds_a_process() {
        let mut k = kernel();
        installed(&mut k, "a");
        let mut d = def("a");
        d.origin = Origin::Application;
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        d.program.identity = Some(rivet_core::service::AppIdentity {
            uid: 27_751,
            gid: 27_751,
        });
        let version_dir = d
            .program
            .path
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .expect("a version directory");
        crate::verify::fixture::install_signed(&mut k, "a", &version_dir, &d.program.path);
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let pid = match s.state_of("a") {
            Some(State::Started { pid, .. }) => *pid,
            other => unreachable!("{other:?}"),
        };

        s.begin_uninstall(&mut k, "a", false);
        s.converge(&mut k);

        // The main pid exits, but a helper it forked is still in the
        // cgroup — S-1's exact scenario.
        k.observations = Script::always(Observation::Members(vec![999]));
        k.exits.push((pid, ExitStatus::Signalled(15)));
        s.reap(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "the main pid exiting must not settle this service while its cgroup still holds a \
             process: {:?}",
            s.state_of("a")
        );
        s.converge(&mut k);
        let while_holding: Vec<_> = k
            .removed_paths()
            .into_iter()
            .filter(|p| !p.ends_with("/tmp"))
            .collect();
        assert!(
            while_holding.is_empty(),
            "S-1's fix: an uninstall removed the tree while the cgroup still held a process: \
             {while_holding:?}"
        );

        // The cgroup actually empties. Past the poll cadence, or nothing
        // re-asks it.
        k.observations = Script::always(Observation::Empty);
        k.mono_ms = Limits::default().adopt_poll_ms;
        s.converge(&mut k);

        assert!(
            k.removed_paths().iter().any(|p| p == "/state/app/a/0.1.0"),
            "it never removed the tree once the cgroup actually emptied: {:?}",
            k.removed_paths()
        );
        assert!(
            s.state_of("a").is_none(),
            "the service is still in the table after being uninstalled"
        );
    }

    /// **Acceptance (c): a non-empty cgroup at `start()`, within one
    /// supervisor lifetime, is killed rather than adopted** — S-1's fix.
    /// `a_service_already_in_its_cgroup_is_adopted_rather_than_started_again`
    /// is this same fixture's FIRST `start()`, which still legitimately
    /// adopts; this is its second.
    #[test]
    fn a_non_empty_cgroup_at_a_repeat_start_is_killed_not_adopted() {
        let mut k = kernel();
        let mut s = supervisor(vec![def("a")]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };

        // The service crashes and is scheduled to retry — an ordinary
        // restart, not a supervisor restart.
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);
        assert!(
            matches!(s.state_of("a"), Some(State::Backoff { .. })),
            "{:?}",
            s.state_of("a")
        );

        // Something is left in the cgroup by the time the retry's own
        // `start()` asks — a leftover this supervisor's own earlier stop
        // did not clean up, within this same lifetime.
        k.observations = Script::always(Observation::Members(vec![999]));
        k.mono_ms = 10_000;
        s.converge(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "a repeat start with a non-empty cgroup must kill it, not adopt it: {:?}",
            s.state_of("a")
        );
        assert!(
            k.signals.contains(&(999, Sig::Term)),
            "the leftover was never signalled: {:?}",
            k.signals
        );
        assert!(
            !k
                .records
                .iter()
                .any(|r| r.msg == "already running; adopted rather than started"),
            "it adopted a leftover instead of killing it: {:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // S-1's remainder: `Done` and `Failed` never drained their cgroup, and
    // `empty_scratch` ran on every exit whether or not the cgroup was
    // actually empty.
    // -----------------------------------------------------------------------

    /// **Acceptance (a), the `Done` half.** A `oneshot`'s helper can survive
    /// its own reap exactly as a stopped service's can, and until this fix
    /// `Done` was reached with no drain at all — the helper kept running
    /// under devices and sockets `release_finally` had already given back,
    /// with the record reading `done`. Also proves the scratch half: it must
    /// not be reclaimed while the helper could still be using it, and must
    /// be reclaimed once the cgroup is actually empty.
    #[test]
    fn a_oneshot_whose_helper_survives_is_drained_before_it_rests_at_done() {
        let mut k = kernel();
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        let mut d = app_on_volume(&mut k, "a");
        d.restart = RestartPolicy::Never;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };
        // The spawn edge already emptied it once — `Self::empty_scratch` is
        // called at both moments, and what this test is about is whether a
        // SECOND removal happens before the cgroup is confirmed empty.
        let after_spawn = scratch_set_aside(&k);

        // The main pid exits successfully — a `oneshot` that "finished" —
        // but a helper it forked is still in the cgroup.
        k.observations = Script::always(Observation::Members(vec![777]));
        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "a oneshot must not settle to Done while its cgroup still holds a process: {:?}",
            s.state_of("a")
        );
        assert!(
            k.signals.contains(&(777, Sig::Term)),
            "the leftover was never signalled: {:?}",
            k.signals
        );
        assert_eq!(
            scratch_set_aside(&k),
            after_spawn,
            "scratch was emptied while the cgroup still held a process: {:?}",
            k.renames
        );

        // The leftover actually exits. Past the poll cadence, or nothing
        // re-asks the cgroup.
        k.observations = Script::always(Observation::Empty);
        k.mono_ms = Limits::default().adopt_poll_ms;
        s.converge(&mut k);

        assert_eq!(
            s.state_of("a"),
            Some(&State::Done),
            "the drain must settle back to Done, not Idle, or the very next pass would restart \
             a oneshot that already ran"
        );
        assert_eq!(
            scratch_set_aside(&k),
            after_spawn + 1,
            "scratch was never reclaimed once the cgroup was confirmed empty: {:?}",
            k.renames
        );
    }

    /// **Acceptance (a), the `Failed` half.** A service that gives up can
    /// have a helper outlive it exactly as a stopped or completed one can,
    /// and until this fix `Failed` was reached with no drain at all.
    #[test]
    fn a_service_that_gives_up_with_a_surviving_helper_is_drained_before_it_rests_at_failed() {
        let mut k = kernel();
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        let d = app_on_volume(&mut k, "a");
        let limits = Limits {
            give_up_after: 1,
            ..Limits::default()
        };
        let mut s = match Supervisor::new(vec![d], limits, journal()) {
            Ok(s) => s,
            Err(e) => unreachable!("fixture refused: {e}"),
        };
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };
        // The spawn edge already emptied it once — see the `Done` test above
        // for why this counts rather than asserting emptiness outright.
        let after_spawn = scratch_set_aside(&k);

        // The main pid crashes — with `give_up_after: 1` this is the one
        // attempt this fixture gets — but a helper it forked is still in the
        // cgroup.
        k.observations = Script::always(Observation::Members(vec![777]));
        k.exits.push((pid, ExitStatus::Exited(1)));
        s.reap(&mut k);

        assert!(
            matches!(s.state_of("a"), Some(State::Draining { .. })),
            "a service giving up must not settle to Failed while its cgroup still holds a \
             process: {:?}",
            s.state_of("a")
        );
        assert!(
            k.signals.contains(&(777, Sig::Term)),
            "the leftover was never signalled: {:?}",
            k.signals
        );
        assert_eq!(
            scratch_set_aside(&k),
            after_spawn,
            "scratch was emptied while the cgroup still held a process: {:?}",
            k.renames
        );

        // The leftover actually exits. Past the poll cadence, or nothing
        // re-asks the cgroup.
        k.observations = Script::always(Observation::Empty);
        k.mono_ms = limits.adopt_poll_ms;
        s.converge(&mut k);

        assert_eq!(
            s.state_of("a"),
            Some(&State::Failed),
            "the drain must settle back to Failed, not Idle, or the very next pass would \
             restart a service that already gave up"
        );
        assert_eq!(
            scratch_set_aside(&k),
            after_spawn + 1,
            "scratch was never reclaimed once the cgroup was confirmed empty: {:?}",
            k.renames
        );
    }

    /// **Blind mode is unchanged** — no cgroup hierarchy to ask, which is
    /// `Observation::Unavailable`, `kernel()`'s own default. `Done` and
    /// `Failed` must be reached exactly as they always were, and scratch
    /// must still be reclaimed on the very same pass, with no drain and no
    /// delay.
    #[test]
    fn blind_mode_reaches_done_and_reclaims_scratch_with_no_drain() {
        let mut k = kernel();
        k.observations = Script::always(Observation::Unavailable(Errno::ENOENT));
        k.dirs_mut().insert("/state/app/a/tmp".to_string(), vec![]);
        let mut d = app_on_volume(&mut k, "a");
        d.restart = RestartPolicy::Never;
        let mut s = supervisor(vec![d]);
        s.converge(&mut k);
        let Some(State::Started { pid, .. }) = s.state_of("a").cloned() else {
            unreachable!("the fixture did not start")
        };
        let after_spawn = scratch_set_aside(&k);

        k.exits.push((pid, ExitStatus::Exited(0)));
        s.reap(&mut k);

        assert_eq!(
            s.state_of("a"),
            Some(&State::Done),
            "blind mode must settle straight to Done, with no cgroup authority to wait on: {:?}",
            s.state_of("a")
        );
        assert!(
            scratch_set_aside(&k) > after_spawn,
            "blind mode must still reclaim scratch on the exit that produced no drain: {:?}",
            k.renames
        );
    }
}
