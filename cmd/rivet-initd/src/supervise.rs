//! Item 3: spawn and restart `rivet-sysd`, with backoff.
//!
//! PID 1 supervises exactly one process (`ADR-017` §3.1). Everything about
//! *services* lives in that process; everything here is about keeping it
//! alive without turning a crash into a fork bomb.
//!
//! # What happens when it will not stay up — ruled here
//!
//! `ADR-017` §3.1.4 already answered this and the answer needs no invention:
//!
//! > If the supervisor cannot be started after its retry budget, PID 1 never
//! > confirms the boot, and the next boot falls back to the previous image
//! > automatically.
//!
//! So after [`RETRY_BUDGET`] failures PID 1 **stops trying**, says so once, and
//! keeps reaping for ever. It does not keep restarting (that is the fork bomb),
//! it does not exit (item 7), and it does not reboot itself — rebooting would
//! fight the firmware's one-shot `tryboot` flag rather than cooperate with it.
//! Withholding confirmation is the whole mechanism, and it costs nothing to
//! use: a trial boot rolls back on the next reset, and a normal boot stays put
//! and stays diagnosable.
//!
//! # Why a long run resets the budget
//!
//! Without [`HEALTHY_RUN_MS`], a machine that has been up for a month and then
//! loses its supervisor ten times in a row would be treated as a failed boot
//! and give up. The budget is meant to catch a *crash loop at boot*, so a run
//! long enough to prove the binary works clears it.

use crate::kernel::{ExitStatus, Kernel};

/// Where the supervisor binary lives.
///
/// `ADR-008` §3 puts the image's binaries in `/system`, and `ADR-016` §3's
/// comparison table names `/system/bin` as the base image's binary directory.
///
/// **A conflict in the record, reported rather than resolved:** `ADR-002` §4's
/// `cmdline.txt` says `init=/sbin/rivet-initd`, which predates `ADR-008`'s
/// layout ruling and cannot both be right. This constant follows the later,
/// ruled layout. Image assembly is what settles it, and it does not exist yet.
pub const SUPERVISOR_PATH: &str = "/system/bin/rivet-sysd";

/// The delay before the first restart.
pub const FIRST_BACKOFF_MS: u64 = 100;

/// The ceiling the delay doubles towards.
pub const MAX_BACKOFF_MS: u64 = 30_000;

/// How many consecutive failures before PID 1 stops trying.
pub const RETRY_BUDGET: u32 = 10;

/// A run at least this long clears the failure count.
pub const HEALTHY_RUN_MS: u64 = 60_000;

/// How long the supervisor must have been continuously up before PID 1 is
/// willing to certify the boot (`ADR-017` §3.1.5 item 5, "once the system is
/// healthy").
///
/// A chosen value, not a derived one: long enough that an image which crashes
/// its supervisor on startup never confirms, short enough that a real boot
/// commits well inside a reboot cycle. The maintainer's to rule on.
pub const CONFIRM_SETTLE_MS: u64 = 30_000;

/// How long a supervisor gets to exit after `SIGTERM` before `SIGKILL`.
///
/// **Was 5_000, and that was shorter than what it contains.** `rivet-sysd`
/// gives one service 10 s and then 2 s, so any service using more than five
/// seconds of its own budget meant PID 1 killing the supervisor mid-shutdown —
/// `SIGKILL` of `rivet-sysd` as the NORMAL path, every slow stop reading as a
/// crash, and a service that needed to flush not reliably getting to. Found in
/// the audit of 2026-09-03; ruled 2026-09-04 (`.rulings/` R-141).
///
/// **Derived rather than chosen**, which is the half of the ruling that
/// outlasts the number. The defect was two ordered values picked independently
/// in two crates, neither naming the other; `rivet_core::shutdown` now computes
/// this one from the service budget it has to exceed, so tuning either end
/// cannot silently reintroduce it. 15 s as the constants stand.
pub const STOP_GRACE_MS: u64 = rivet_core::shutdown::SUPERVISOR_STOP_GRACE_MS;

/// PID 1's single child, and the state needed to keep restarting it sanely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supervisor {
    pub path: &'static str,
    /// The pid, while it is running.
    pub pid: Option<i32>,
    /// Consecutive failures. Cleared by a run of at least [`HEALTHY_RUN_MS`].
    pub failures: u32,
    /// The monotonic time the current run started.
    pub started_at_ms: u64,
    /// The earliest monotonic time at which another start may be attempted.
    pub next_attempt_at_ms: u64,
    /// The budget is spent. PID 1 stops trying and never confirms the boot.
    pub given_up: bool,
    /// Set while PID 1 is deliberately stopping it, so that an orderly shutdown
    /// does not look like a crash loop: the exit is reported, no retry is
    /// scheduled, and no budget is spent.
    pub expect_exit: bool,
}

impl Supervisor {
    pub fn new(path: &'static str) -> Self {
        Self {
            path,
            pid: None,
            failures: 0,
            started_at_ms: 0,
            // Zero rather than "now": the first attempt is due immediately.
            next_attempt_at_ms: 0,
            given_up: false,
            expect_exit: false,
        }
    }

    /// How long to wait before the `n`th consecutive retry.
    ///
    /// Doubling, capped. Saturating throughout: a shift that overflowed would
    /// wrap to a *short* delay, which is the fork bomb this exists to prevent.
    pub fn backoff_for(failures: u32) -> u64 {
        if failures <= 1 {
            return FIRST_BACKOFF_MS;
        }
        let steps = failures.saturating_sub(1).min(31);
        FIRST_BACKOFF_MS
            .checked_shl(steps)
            .unwrap_or(MAX_BACKOFF_MS)
            .min(MAX_BACKOFF_MS)
    }

    pub fn is_running(&self) -> bool {
        self.pid.is_some()
    }

    /// How long until another start is due, or `None` if none ever will be.
    ///
    /// The `expect_exit` arm matters more than it looks: without it, a shutdown
    /// whose `reboot(2)` the kernel refused leaves a stale `next_attempt_at_ms`
    /// in the past, the loop computes a one-millisecond wait, and PID 1 spins a
    /// core for ever on a machine nobody can reach to fix.
    pub fn next_due_in_ms(&self, now_ms: u64) -> Option<u64> {
        if self.given_up || self.expect_exit || self.is_running() {
            return None;
        }
        Some(self.next_attempt_at_ms.saturating_sub(now_ms))
    }

    /// Whether the supervisor has been up long enough for the boot to count as
    /// healthy.
    pub fn settled(&self, now_ms: u64) -> bool {
        self.is_running() && now_ms.saturating_sub(self.started_at_ms) >= CONFIRM_SETTLE_MS
    }

    /// How long until [`Self::settled`] becomes true, or `None` if nothing is
    /// running for it to become true about.
    ///
    /// The loop needs this to schedule its own wakeup: without it the settle
    /// time is only noticed on whatever wakeup happens next, which makes when
    /// a boot is confirmed depend on unrelated timers.
    pub fn settles_in_ms(&self, now_ms: u64) -> Option<u64> {
        if !self.is_running() {
            return None;
        }
        Some(CONFIRM_SETTLE_MS.saturating_sub(now_ms.saturating_sub(self.started_at_ms)))
    }

    /// Start it if one is due. Does nothing at all otherwise.
    pub fn maybe_start(&mut self, k: &mut dyn Kernel, now_ms: u64) {
        if self.given_up
            || self.expect_exit
            || self.is_running()
            || now_ms < self.next_attempt_at_ms
        {
            return;
        }
        match k.spawn(self.path) {
            Ok(pid) => {
                self.pid = Some(pid);
                self.started_at_ms = now_ms;
                k.say(&format!("rivet-initd: started {} as pid {pid}", self.path));
            }
            Err(e) => {
                // A spawn that never happened is a failure exactly like a
                // process that died instantly; the console distinguishes them
                // because the operator's fix is different.
                k.say(&format!("rivet-initd: cannot spawn {}: {e}", self.path));
                self.record_failure(k, now_ms);
            }
        }
    }

    /// Tell the supervisor about a reaped pid.
    ///
    /// Returns `true` if it was ours, so the caller can label the console line.
    pub fn note_exit(
        &mut self,
        k: &mut dyn Kernel,
        pid: i32,
        status: ExitStatus,
        now_ms: u64,
    ) -> bool {
        if self.pid != Some(pid) {
            return false;
        }
        self.pid = None;
        let ran_ms = now_ms.saturating_sub(self.started_at_ms);
        if ran_ms >= HEALTHY_RUN_MS {
            // It worked for long enough to prove the binary is fine. Whatever
            // killed it now is not the crash loop the budget is guarding.
            if self.failures > 0 {
                k.say(&format!(
                    "rivet-initd: supervisor had run {ran_ms}ms; clearing {} earlier failure(s)",
                    self.failures
                ));
            }
            self.failures = 0;
        }
        k.say(&format!(
            "rivet-initd: supervisor pid {pid} ({status}) after {ran_ms}ms"
        ));
        if self.expect_exit {
            // PID 1 asked for this. It is not a failure and schedules nothing.
            return true;
        }
        self.record_failure(k, now_ms);
        true
    }

    fn record_failure(&mut self, k: &mut dyn Kernel, now_ms: u64) {
        self.failures = self.failures.saturating_add(1);
        if self.failures >= RETRY_BUDGET {
            self.given_up = true;
            k.say(&format!(
                "rivet-initd: GIVING UP on {} after {} consecutive failures.",
                self.path, self.failures
            ));
            k.say("             pid 1 will not retry and will NOT confirm this boot, so the");
            k.say("             next reset falls back to the previous image (ADR-017 §3.1.4).");
            k.say("             Reaping continues; pid 1 never exits.");
            return;
        }
        let delay = Self::backoff_for(self.failures);
        self.next_attempt_at_ms = now_ms.saturating_add(delay);
        k.say(&format!(
            "rivet-initd: retry {} of {RETRY_BUDGET} in {delay}ms",
            self.failures
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::Errno;
    use crate::testkernel::{FakeKernel, Script};

    fn sup() -> Supervisor {
        Supervisor::new(SUPERVISOR_PATH)
    }

    // -- the backoff curve ----------------------------------------------------

    #[test]
    fn backoff_doubles_from_the_first_delay() {
        assert_eq!(Supervisor::backoff_for(0), FIRST_BACKOFF_MS);
        assert_eq!(Supervisor::backoff_for(1), FIRST_BACKOFF_MS);
        assert_eq!(Supervisor::backoff_for(2), FIRST_BACKOFF_MS * 2);
        assert_eq!(Supervisor::backoff_for(3), FIRST_BACKOFF_MS * 4);
        assert_eq!(Supervisor::backoff_for(4), FIRST_BACKOFF_MS * 8);
    }

    #[test]
    fn backoff_is_capped_and_never_wraps_to_something_short() {
        // A shift that overflowed would produce a *small* delay, which is
        // precisely the fork bomb the backoff exists to prevent.
        for failures in [8, 9, 20, 31, 32, 100, u32::MAX] {
            let delay = Supervisor::backoff_for(failures);
            assert!(
                (FIRST_BACKOFF_MS..=MAX_BACKOFF_MS).contains(&delay),
                "backoff_for({failures}) = {delay}"
            );
        }
        assert_eq!(Supervisor::backoff_for(u32::MAX), MAX_BACKOFF_MS);
    }

    #[test]
    fn backoff_never_decreases() {
        let mut previous = 0;
        for failures in 0..40 {
            let delay = Supervisor::backoff_for(failures);
            assert!(delay >= previous, "backoff went down at {failures}");
            previous = delay;
        }
    }

    // -- a supervisor that exits instantly, repeatedly ------------------------

    /// The headline failure: an immediate-exit crash loop must slow down, not
    /// spin.
    #[test]
    fn a_supervisor_that_exits_instantly_backs_off_further_every_time() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        let mut delays = Vec::new();

        for _ in 0..4 {
            let now = k.now_ms();
            s.maybe_start(&mut k, now);
            assert!(s.is_running());
            // It dies at once.
            let now = k.now_ms();
            s.note_exit(&mut k, 200, ExitStatus::Exited(1), now);
            delays.push(s.next_attempt_at_ms.saturating_sub(now));
            // Time passes until the next attempt is due.
            let wait = s.next_due_in_ms(k.now_ms()).unwrap_or(0);
            k.sleep_ms(wait);
        }

        assert_eq!(delays, vec![100, 200, 400, 800]);
        assert_eq!(k.spawn_calls.len(), 4);
    }

    #[test]
    fn a_start_is_not_attempted_before_its_backoff_has_elapsed() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.maybe_start(&mut k, 0);
        s.note_exit(&mut k, 200, ExitStatus::Exited(1), 0);
        assert_eq!(s.next_attempt_at_ms, FIRST_BACKOFF_MS);

        s.maybe_start(&mut k, FIRST_BACKOFF_MS - 1);
        assert_eq!(
            k.spawn_calls.len(),
            1,
            "too early: it must not have started again"
        );

        s.maybe_start(&mut k, FIRST_BACKOFF_MS);
        assert_eq!(k.spawn_calls.len(), 2);
    }

    #[test]
    fn a_running_supervisor_is_never_started_twice() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.maybe_start(&mut k, 0);
        s.maybe_start(&mut k, 10_000);
        s.maybe_start(&mut k, 20_000);
        assert_eq!(k.spawn_calls.len(), 1);
    }

    // -- a supervisor that cannot be spawned at all ---------------------------

    #[test]
    fn a_supervisor_that_is_not_there_backs_off_the_same_way() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut s = sup();
        s.maybe_start(&mut k, 0);
        assert!(!s.is_running());
        assert_eq!(s.failures, 1);
        assert_eq!(s.next_attempt_at_ms, FIRST_BACKOFF_MS);
        assert!(
            k.lines.iter().any(|l| l.contains("cannot spawn")),
            "{:?}",
            k.lines
        );
    }

    #[test]
    fn the_budget_runs_out_and_pid_one_stops_trying() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut s = sup();
        let mut now = 0;
        for _ in 0..RETRY_BUDGET {
            s.maybe_start(&mut k, now);
            now = s.next_attempt_at_ms.max(now);
        }
        assert!(s.given_up);
        assert_eq!(
            k.spawn_calls.len(),
            usize::try_from(RETRY_BUDGET).unwrap_or(0)
        );

        // And it really stops: no further attempt, however long we wait.
        s.maybe_start(&mut k, now + 10_000_000);
        assert_eq!(
            k.spawn_calls.len(),
            usize::try_from(RETRY_BUDGET).unwrap_or(0)
        );
        assert_eq!(s.next_due_in_ms(now), None);
    }

    #[test]
    fn giving_up_says_what_it_means_for_the_boot() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut s = sup();
        let mut now = 0;
        for _ in 0..RETRY_BUDGET {
            s.maybe_start(&mut k, now);
            now = s.next_attempt_at_ms.max(now) + 1;
        }
        let console = k.lines.join("\n");
        assert!(console.contains("GIVING UP"), "{console}");
        assert!(console.contains("NOT confirm"), "{console}");
        assert!(console.contains("never exits"), "{console}");
    }

    // -- a long run clears the count ------------------------------------------

    #[test]
    fn a_run_long_enough_to_prove_the_binary_works_clears_the_failures() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.failures = RETRY_BUDGET - 1;
        s.maybe_start(&mut k, 0);
        s.note_exit(&mut k, 200, ExitStatus::Exited(0), HEALTHY_RUN_MS);

        // Cleared, then this one exit counted: one failure, not the budget.
        assert_eq!(s.failures, 1);
        assert!(!s.given_up);
        assert!(k.lines.iter().any(|l| l.contains("clearing")));
    }

    #[test]
    fn a_run_just_short_of_healthy_does_not_clear_them() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.failures = 3;
        s.maybe_start(&mut k, 0);
        s.note_exit(&mut k, 200, ExitStatus::Exited(0), HEALTHY_RUN_MS - 1);
        assert_eq!(s.failures, 4);
    }

    // -- bookkeeping ----------------------------------------------------------

    #[test]
    fn an_exit_that_is_not_the_supervisors_is_ignored_by_it() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.maybe_start(&mut k, 0);
        let claimed = s.note_exit(&mut k, 999, ExitStatus::Exited(0), 10);
        assert!(!claimed, "pid 999 was never the supervisor");
        assert!(s.is_running(), "and the real one is still running");
        assert_eq!(s.failures, 0);
    }

    #[test]
    fn an_exit_before_anything_was_started_is_ignored() {
        let mut k = FakeKernel::new();
        let mut s = sup();
        assert!(!s.note_exit(&mut k, 200, ExitStatus::Exited(0), 0));
        assert_eq!(s.failures, 0);
    }

    #[test]
    fn nothing_is_due_while_it_is_running_or_after_giving_up() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.maybe_start(&mut k, 0);
        assert_eq!(s.next_due_in_ms(0), None);
        s.pid = None;
        assert_eq!(s.next_due_in_ms(0), Some(0));
        s.given_up = true;
        assert_eq!(s.next_due_in_ms(0), None);
    }

    #[test]
    fn a_supervisor_is_not_settled_until_it_has_stayed_up() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.maybe_start(&mut k, 1_000);
        assert!(!s.settled(1_000));
        assert!(!s.settled(1_000 + CONFIRM_SETTLE_MS - 1));
        assert!(s.settled(1_000 + CONFIRM_SETTLE_MS));
    }

    #[test]
    fn a_supervisor_that_is_not_running_is_never_settled() {
        let s = sup();
        assert!(!s.settled(u64::MAX));
    }

    #[test]
    fn the_console_reports_how_long_a_supervisor_lasted() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut s = sup();
        s.maybe_start(&mut k, 500);
        s.note_exit(&mut k, 200, ExitStatus::Signalled(9), 1_700);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("1200ms") && l.contains("killed by signal 9")),
            "{:?}",
            k.lines
        );
    }
}
