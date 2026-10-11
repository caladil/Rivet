//! Reaping, and the stage 0 self-test that proves it happens.
//!
//! `ADR-017` §3.1.5 item 4 is "reap **every** orphan, not only its own
//! children". The loop that calls this lives in [`crate::pid1`]; what is here
//! is the draining itself and the children Stage 0 spawns to demonstrate it.

use crate::kernel::{Errno, ExitStatus, Kernel, Reaped};

/// The exit codes the self-test children use, so a reap can be told from noise.
const DIRECT_EXIT_CODE: i32 = 7;
const ORPHAN_EXIT_CODE: i32 = 9;
const ORPHAN_DELAY_MS: u64 = 200;

/// Reap every child that is ready, and classify what the kernel said.
///
/// The distinction that matters: `ECHILD` means there is nothing left to reap
/// and the drain is finished; `EINTR` means the call was interrupted and has
/// told us nothing at all. Treating `EINTR` as `ECHILD` would abandon a drain
/// half-done and leave a zombie behind — the exact failure `PLAN.md` §2.1's
/// exit criterion tests for.
pub fn drain(k: &mut dyn Kernel) -> Vec<(i32, ExitStatus)> {
    let mut reaped = Vec::new();
    loop {
        match k.waitpid_any() {
            Ok(Reaped::Child { pid, status }) => reaped.push((pid, status)),
            // Children exist, none has exited.
            Ok(Reaped::NoneReady) => return reaped,
            // No children at all. Normal, and the usual state at stage 0.
            Err(Errno::ECHILD) => return reaped,
            // Interrupted, not finished. Ask again.
            Err(Errno::EINTR) => continue,
            Err(e) => {
                k.say(&format!("rivet-initd: waitpid: {e}"));
                return reaped;
            }
        }
    }
}

/// What the boot is still waiting to see reaped.
///
/// This exists to satisfy `PLAN.md` §2.1's exit criterion under QEMU, where the
/// initramfs holds no other executable to spawn. It is not part of PID 1's job
/// list and comes out when `cmd/rivet-sysd` is real enough to be the
/// demonstration itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTest {
    /// A direct child that exits at once.
    pub direct: i32,
    /// A child that forks and exits at once, orphaning its own child onto us.
    pub maker: i32,
    /// Only pid 1 adopts orphans, so only pid 1 expects to reap one.
    pub expect_orphan: bool,
    pub seen_direct: bool,
    pub seen_maker: bool,
    pub seen_orphan: bool,
    pub reported: bool,
}

impl SelfTest {
    pub fn note(&mut self, pid: i32) {
        if pid == self.direct {
            self.seen_direct = true;
        } else if pid == self.maker {
            self.seen_maker = true;
        } else {
            // Anything else pid 1 can reap was never its child. That is the
            // whole of ADR-017 §3.1.5 item 4.
            self.seen_orphan = true;
        }
    }

    /// Whether this pid is one the self-test is watching for. The supervisor's
    /// exits are labelled by [`crate::supervise::Supervisor`] instead, and must
    /// not be mistaken for the orphan.
    pub fn watches(&self, pid: i32) -> bool {
        pid == self.direct || pid == self.maker || self.expect_orphan
    }

    pub fn label(&self, pid: i32) -> &'static str {
        if pid == self.direct {
            "direct child"
        } else if pid == self.maker {
            "orphan-maker"
        } else {
            "ORPHAN — never our child, reparented to pid 1"
        }
    }

    pub fn complete(&self) -> bool {
        self.seen_direct && self.seen_maker && (self.seen_orphan || !self.expect_orphan)
    }
}

/// Fork the children the exit criterion needs.
///
/// A failure to fork is reported and survived: the self-test is diagnostics,
/// and PID 1 does not stop being PID 1 because a diagnostic could not run.
pub fn spawn_selftest(k: &mut dyn Kernel) -> Option<SelfTest> {
    let is_init = k.self_pid() == 1;
    k.say("rivet-initd: self-test — spawning a child and an orphan");

    let direct = match k.fork_child_exiting_with(DIRECT_EXIT_CODE) {
        Ok(pid) => pid,
        Err(e) => {
            k.say(&format!("  fork failed: {e}; skipping the self-test"));
            return None;
        }
    };
    k.say(&format!(
        "  spawned pid {direct} — a direct child that exits immediately"
    ));

    let maker = match k.fork_orphan_maker(ORPHAN_DELAY_MS, ORPHAN_EXIT_CODE) {
        Ok(pid) => pid,
        Err(e) => {
            k.say(&format!(
                "  second fork failed: {e}; skipping the self-test"
            ));
            return None;
        }
    };
    k.say(&format!(
        "  spawned pid {maker} — exits at once, orphaning its own child onto pid 1"
    ));
    if !is_init {
        k.say("  not pid 1: the orphan reparents elsewhere, so that leg is not expected");
    }

    Some(SelfTest {
        direct,
        maker,
        expect_orphan: is_init,
        seen_direct: false,
        seen_maker: false,
        seen_orphan: false,
        reported: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    fn child(pid: i32, code: i32) -> Result<Reaped, Errno> {
        Ok(Reaped::Child {
            pid,
            status: ExitStatus::Exited(code),
        })
    }

    #[test]
    fn echild_ends_the_drain_without_reporting_an_error() {
        let mut k = FakeKernel::new();
        k.wait_results = Script::always(Err(Errno::ECHILD));
        assert!(drain(&mut k).is_empty());
        assert!(k.lines.is_empty(), "ECHILD is normal: {:?}", k.lines);
    }

    /// `EINTR` and `ECHILD` are different answers. Confusing them abandons a
    /// drain half-finished and leaves a zombie.
    #[test]
    fn eintr_retries_rather_than_being_mistaken_for_no_children_left() {
        let mut k = FakeKernel::new();
        k.wait_results = Script::sequence(
            vec![
                Err(Errno::EINTR),
                child(53, 7),
                Err(Errno::EINTR),
                child(55, 9),
            ],
            Err(Errno::ECHILD),
        );

        let reaped = drain(&mut k);

        assert_eq!(
            reaped,
            vec![(53, ExitStatus::Exited(7)), (55, ExitStatus::Exited(9))]
        );
        assert!(
            k.lines.is_empty(),
            "EINTR is not an error to report: {:?}",
            k.lines
        );
    }

    #[test]
    fn none_ready_ends_the_drain_because_the_children_are_still_running() {
        let mut k = FakeKernel::new();
        k.wait_results = Script::sequence(vec![child(53, 0)], Ok(Reaped::NoneReady));
        assert_eq!(drain(&mut k).len(), 1);
        assert!(k.lines.is_empty());
    }

    #[test]
    fn an_unexpected_waitpid_errno_is_reported_and_ends_the_drain() {
        let mut k = FakeKernel::new();
        k.wait_results = Script::always(Err(Errno::EINVAL));
        assert!(drain(&mut k).is_empty());
        assert!(
            k.lines.iter().any(|l| l.contains("waitpid")),
            "{:?}",
            k.lines
        );
    }

    #[test]
    fn one_wakeup_drains_every_child_because_signals_coalesce() {
        // Two children exiting can produce one SIGCHLD. A drain that stopped
        // after the first would leave the second a zombie for ever.
        let mut k = FakeKernel::new();
        k.wait_results = Script::sequence(
            vec![child(53, 7), child(54, 0), child(55, 9)],
            Err(Errno::ECHILD),
        );
        assert_eq!(drain(&mut k).len(), 3);
    }

    fn selftest(expect_orphan: bool) -> SelfTest {
        SelfTest {
            direct: 53,
            maker: 54,
            expect_orphan,
            seen_direct: false,
            seen_maker: false,
            seen_orphan: false,
            reported: false,
        }
    }

    #[test]
    fn an_unknown_pid_is_an_orphan_because_pid_one_never_spawned_it() {
        let mut t = selftest(true);
        t.note(55);
        assert!(t.seen_orphan);
        assert_eq!(t.label(55), "ORPHAN — never our child, reparented to pid 1");
    }

    #[test]
    fn the_self_test_is_incomplete_until_the_orphan_arrives_when_we_are_pid_one() {
        let mut t = selftest(true);
        t.note(53);
        t.note(54);
        assert!(!t.complete(), "the orphan has not been reaped yet");
        t.note(55);
        assert!(t.complete());
    }

    #[test]
    fn the_self_test_does_not_wait_for_an_orphan_it_will_never_adopt() {
        let mut t = selftest(false);
        t.note(53);
        t.note(54);
        assert!(t.complete());
    }

    #[test]
    fn a_pid_that_is_both_would_not_be_double_counted() {
        // pids are unique among live processes; this pins the branch order so
        // a refactor cannot make `direct` fall through to the orphan arm.
        let mut t = selftest(true);
        t.note(53);
        assert!(t.seen_direct && !t.seen_orphan);
    }

    /// The supervisor is not an orphan. Once PID 1 has a real child of its own
    /// to restart, an unlabelled "anything else is the orphan" rule would claim
    /// the supervisor's exit and complete the self-test on a lie.
    #[test]
    fn the_self_test_does_not_watch_for_an_orphan_it_was_told_not_to_expect() {
        let t = selftest(false);
        assert!(t.watches(53));
        assert!(t.watches(54));
        assert!(
            !t.watches(200),
            "an unrelated pid is not this test's business"
        );
    }

    #[test]
    fn a_failed_fork_skips_the_self_test_instead_of_pretending_it_ran() {
        let mut k = FakeKernel::new();
        k.fork_results = Script::always(Err(Errno::EPERM));
        assert_eq!(spawn_selftest(&mut k), None);
        assert!(k.lines.iter().any(|l| l.contains("fork failed")));
    }

    #[test]
    fn a_failed_second_fork_also_skips_it() {
        let mut k = FakeKernel::new();
        k.fork_results = Script::sequence(vec![Ok(53)], Err(Errno::EPERM));
        assert_eq!(spawn_selftest(&mut k), None);
        assert!(k.lines.iter().any(|l| l.contains("second fork failed")));
    }

    #[test]
    fn not_being_pid_one_means_no_orphan_is_expected_and_the_console_says_why() {
        let mut k = FakeKernel::new();
        k.pid = 4321;
        let t = spawn_selftest(&mut k);
        assert!(t.is_some_and(|t| !t.expect_orphan));
        assert!(k.lines.iter().any(|l| l.contains("not pid 1")));
    }
}
