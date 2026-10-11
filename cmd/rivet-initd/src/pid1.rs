//! The loop: wait, reap, restart, confirm, shut down, never return.
//!
//! `ADR-017` §3.1.5 items 3, 4, 5, 6 and 7 all land here, because they are all
//! the same loop. What each of them *decides* lives in its own module —
//! [`crate::supervise`], [`crate::confirm`] — and this is the order they happen
//! in.
//!
//! # The three signals, which are this machine's entire shutdown contract
//!
//! Item 6 is *"handle `SIGTERM` for shutdown and reboot"*, and `ADR-017` §3.1.10
//! grew it to three on 2026-08-23. **Nothing else may send any of them for any
//! other reason**, on a board or under the emulator:
//!
//! | Signal | Means | Reaches the kernel as |
//! | :-- | :-- | :-- |
//! | `SIGTERM` | shut down and stay off | [`RebootHow::PowerOff`] |
//! | `SIGINT` | shut down and come back (the kernel sends it for Ctrl-Alt-Delete) | [`RebootHow::Restart`] |
//! | `SIGUSR1` | shut down and come back **once** in the slot `rivet apply trial-boot` armed | [`RebootHow::TrialRestart`], `ADR-018` §5.1 |
//!
//! All three take the same [`Pid1::shut_down`] path — stop the supervisor, sync
//! the filesystems, then ask the kernel — because the third is a reboot like the
//! other two and a trial boot that skipped stopping services would be a *worse*
//! shutdown than the ordinary one, on the boot that is most likely to be rolled
//! back.
//!
//! **Why a signal at all, rather than the CLI calling `reboot(2)` itself.**
//! Three shapes were weighed and this is the second: the smallest
//! one hands item 6's responsibility to a login shell, which would stop the
//! machine with services running and `/state` unflushed. `ADR-017` §3.1.10
//! carries the ruling.
//!
//! **`SIGUSR1`'s default disposition is to terminate the process.** It is
//! blocked by [`crate::kernel::HANDLED_SIGNALS`] before any of this runs; see
//! [`crate::kernel::Sig::User1`], where that is the whole of the documentation.

use rivet_adapter_image::BootSlots;

use crate::boot::{BootHealth, proc_state, zombie_census};
use crate::confirm::{self, Confirmation};
use crate::kernel::{Errno, Kernel, RebootHow, Sig};
use crate::provision::Provisioning;
use crate::reap::{SelfTest, drain};
use crate::supervise::{STOP_GRACE_MS, Supervisor};

/// How long the loop sleeps when it has nothing scheduled.
///
/// With a signalfd this is a backstop rather than a poll — a signal wakes the
/// loop long before this expires. It exists so that no scheduling bug can wedge
/// PID 1 for ever.
pub const IDLE_WAIT_MS: u64 = 60_000;

/// The degraded reaper's poll interval, used when there is no signalfd.
pub const POLL_INTERVAL_MS: u64 = 200;

/// How often to look while waiting for a supervisor to stop.
pub const STOP_POLL_MS: u64 = 50;

/// How the loop learns that something happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeMode {
    /// Signals are blocked and read from a descriptor. Race-free: a signal that
    /// arrives while we are not reading becomes *pending* rather than lost,
    /// which is the defect in the textbook handler-plus-`pause()` design.
    Signals,
    /// `signalfd(2)` was refused. Poll for children instead — and **no signal
    /// is received at all**, so shutdown requests are not honoured.
    DegradedPoll,
}

/// Arm the signals if the kernel will allow it, and say which loop is running.
///
/// PID 1 does not get to fail here, so a refusal downgrades rather than
/// propagating.
pub fn choose_mode(k: &mut dyn Kernel) -> WakeMode {
    match k.arm_signals() {
        Ok(()) => {
            k.say(
                "rivet-initd: reaping on SIGCHLD and listening for SIGTERM/SIGINT/SIGUSR1 via signalfd",
            );
            WakeMode::Signals
        }
        Err(e) => {
            k.say(&format!(
                "rivet-initd: DEGRADED signalfd refused ({e}); reaping by poll every {POLL_INTERVAL_MS}ms"
            ));
            k.say(
                "             SIGTERM, SIGINT and SIGUSR1 cannot be received in this mode, so an",
            );
            k.say("             orderly shutdown is not available — and neither is a trial boot.");
            k.say("             Reaping and restarting still work.");
            WakeMode::DegradedPoll
        }
    }
}

/// Everything PID 1 is still responsible for after the boot phases.
pub struct Pid1 {
    pub mode: WakeMode,
    pub health: BootHealth,
    pub provisioning: Provisioning,
    pub supervisor: Supervisor,
    pub selftest: Option<SelfTest>,
    pub confirmation: Option<Confirmation>,
    /// Whether the console has already been told this boot is waiting to be
    /// reached. Said once — PLAN.md §104.20.
    said_waiting_to_be_reached: bool,
    /// When the next commit attempt is due, while one is scheduled — after a
    /// medium refusal ([`confirm::COMMIT_RETRY_DELAY_MS`]), or while a trial
    /// boot waits to be reached ([`confirm::REACHED_CHECK_INTERVAL_MS`]).
    ///
    /// A timer in this loop rather than a sleep, so that the wait between
    /// attempts is spent reaping children and taking signals like any other.
    pub next_commit_at_ms: Option<u64>,
    /// How many commit attempts the medium has refused in this boot's one
    /// decision. At most [`confirm::COMMIT_ATTEMPTS`].
    commit_refusals: u32,
    pub shutting_down: bool,
    /// **The break-glass console** — `ADR-035` §8A.1.2. Holds a partial line
    /// across ticks, because a serial console delivers keystrokes rather than
    /// messages.
    pub console: crate::breakglass::Console,
    /// The A/B boot primitive, injected by the composition root.
    ///
    /// Behind the boot-slot port, ruled 2026-08-23: x86 has no `tryboot`, so the platform's
    /// answer arrives here rather than being reached for. Boxed rather than
    /// generic for the same reason [`Kernel`] is `dyn` — one compiled copy of
    /// the decisions, and the tests drive exactly the code a board runs.
    pub slots: Box<dyn BootSlots>,
}

impl Pid1 {
    pub fn new(
        mode: WakeMode,
        health: BootHealth,
        provisioning: Provisioning,
        supervisor: Supervisor,
        selftest: Option<SelfTest>,
        slots: Box<dyn BootSlots>,
    ) -> Self {
        Self {
            mode,
            health,
            provisioning,
            supervisor,
            selftest,
            confirmation: None,
            said_waiting_to_be_reached: false,
            next_commit_at_ms: None,
            commit_refusals: 0,
            shutting_down: false,
            console: crate::breakglass::Console::new(),
            slots,
        }
    }

    /// How long to wait before the loop must run again.
    ///
    /// Three things schedule work: the supervisor's next restart, the moment
    /// the boot becomes confirmable, and a commit attempt that is due again
    /// after the medium refused one or while a trial boot waits to be reached.
    /// All shorten the wait, because otherwise
    /// *when* they happen depends on whatever unrelated timer fires next —
    /// a boot would be confirmed at the idle wakeup rather than at its settle
    /// time. Never zero: a zero timeout turns the signalfd wait into a spin.
    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        let mut wait = match self.mode {
            WakeMode::Signals => IDLE_WAIT_MS,
            WakeMode::DegradedPoll => POLL_INTERVAL_MS,
        };
        if self.shutting_down {
            // Nothing will be started and nothing will be certified, so there
            // is nothing left to schedule. Reached when the kernel refused the
            // reboot and PID 1 is living out its days as a reaper.
            return wait;
        }
        if let Some(due) = self.supervisor.next_due_in_ms(now_ms) {
            wait = wait.min(due);
        }
        if self.confirmation.is_none() {
            // A scheduled attempt replaces the settle time rather than joining
            // it: the supervisor has already settled by then, and its settle
            // time, being past, would otherwise shorten every wait to nothing.
            let due = match self.next_commit_at_ms {
                Some(at) => Some(at.saturating_sub(now_ms)),
                None => self.supervisor.settles_in_ms(now_ms),
            };
            if let Some(due) = due {
                wait = wait.min(due);
            }
        }
        wait.max(1)
    }

    /// One pass: start what is due, wait, reap, act on a signal, confirm.
    pub fn tick(&mut self, k: &mut dyn Kernel) {
        if !self.shutting_down {
            let now = k.now_ms();
            self.supervisor.maybe_start(k, now);
        }

        let wait = self.wait_ms(k.now_ms());
        let signal = self.wait_for_signal(k, wait);

        // Always drain, whatever woke us. A timeout in signal mode still means
        // the backoff timer fired; in degraded mode draining is the whole job.
        self.reap_all(k);

        match signal {
            Some(Sig::Term) => self.shut_down(k, Sig::Term, RebootHow::PowerOff),
            Some(Sig::Interrupt) => self.shut_down(k, Sig::Interrupt, RebootHow::Restart),
            // `ADR-017` §3.1.10. The same orderly shutdown as the other two, and
            // then the one reboot that carries the firmware's one-shot flag.
            Some(Sig::User1) => self.shut_down(k, Sig::User1, RebootHow::TrialRestart),
            Some(Sig::Child) | None => {}
            Some(other) => k.say(&format!("rivet-initd: ignoring {other}")),
        }

        // **Break-glass, after the reaping and before the confirm** —
        // `ADR-035` §8A.1.2. After, because a dead supervisor must be reaped
        // whatever anybody is typing; before, because a console asking for a
        // restart should not race the confirmation of a trial boot.
        //
        // **Not while shutting down.** The machine is already going; accepting
        // a second stop would answer a command whose effect nobody could
        // observe.
        if !self.shutting_down {
            self.serve_console(k);
        }

        if !self.shutting_down {
            self.maybe_confirm(k);
        }
    }

    /// One line from the console, if there is one.
    ///
    /// A stop the console asked for is performed **here**, after the answer has
    /// been written — `crate::breakglass_board`'s `pending` exists for that
    /// ordering, so an operator sees the machine agree before it goes.
    fn serve_console(&mut self, k: &mut dyn Kernel) {
        let Some(line) = self.console.poll(k) else {
            return;
        };
        let line = match line {
            crate::breakglass::Line::Ready(line) => line,
            crate::breakglass::Line::TooLong => {
                k.say(&format!(
                    "break-glass: that line was longer than {} bytes and was DISCARDED, not \
                     truncated - a shortened command is a different command (ADR-035 §8A.1.2)",
                    crate::breakglass::LINE_MAX
                ));
                return;
            }
        };

        // The running slot, from the same authority pid 1 uses for everything
        // else about slots. A console that could not be told which half of the
        // card it is on is one an operator cannot safely act from.
        let slot = {
            let mut medium = crate::slots::KernelMedium::new(k);
            self.slots
                .running_slot(&mut medium)
                .and_then(rivet_adapter_image::slot_named)
        };
        let slot = match slot {
            Ok(slot) => slot,
            Err(why) => {
                k.say(&format!(
                    "break-glass: this machine cannot say which slot it is running ({why}), so \
                     it will not serve a command that might mean a different one"
                ));
                return;
            }
        };

        if let Some(how) = crate::breakglass::serve(&line, k, slot) {
            self.shut_down(k, Sig::Term, how);
        }
    }

    /// Never return, and never spin.
    pub fn run_forever(&mut self, k: &mut dyn Kernel) -> ! {
        loop {
            self.tick(k);
        }
    }

    fn wait_for_signal(&mut self, k: &mut dyn Kernel, wait_ms: u64) -> Option<Sig> {
        match self.mode {
            WakeMode::Signals => match k.await_signal(wait_ms) {
                Ok(signal) => signal,
                // Interrupted before anything was delivered. Ordinary.
                Err(Errno::EINTR) => None,
                Err(e) => {
                    // The descriptor is no longer a usable wakeup source.
                    // Continuing to trust it would leave PID 1 spinning on a
                    // broken fd and never reaping.
                    k.say(&format!(
                        "rivet-initd: signalfd read: {e}; falling back to polling every {POLL_INTERVAL_MS}ms"
                    ));
                    k.say("rivet-initd: shutdown signals can no longer be received");
                    self.mode = WakeMode::DegradedPoll;
                    None
                }
            },
            WakeMode::DegradedPoll => {
                k.sleep_ms(wait_ms);
                None
            }
        }
    }

    /// Reap everything that is ready, and route each exit to whoever owns it.
    fn reap_all(&mut self, k: &mut dyn Kernel) {
        for (pid, status) in drain(k) {
            let now = k.now_ms();
            // The supervisor first: it is PID 1's own child and its exits are
            // the only ones that mean anything to the restart policy.
            if self.supervisor.note_exit(k, pid, status, now) {
                continue;
            }
            match self.selftest.as_mut() {
                Some(t) if t.watches(pid) => {
                    let label = t.label(pid);
                    k.say(&format!(
                        "rivet-initd: reaped pid {pid} ({status}) — {label}"
                    ));
                    t.note(pid);
                }
                _ => k.say(&format!("rivet-initd: reaped pid {pid} ({status})")),
            }
        }

        let due = self
            .selftest
            .as_ref()
            .is_some_and(|t| !t.reported && t.complete());
        if due {
            if let Some(t) = self.selftest.as_mut() {
                t.reported = true;
            }
            self.report_census(k);
        }
    }

    /// Ask `/proc` whether anything the self-test spawned is still a zombie.
    ///
    /// The criterion is not "we called waitpid"; it is that the kernel no
    /// longer has a zombie. So the claim is read back out of `/proc` rather
    /// than asserted from our own return codes.
    ///
    /// # What the criterion is *not*, corrected 2026-08-23 from two boards
    ///
    /// It is not "no zombie exists anywhere on the system". Two boots of the
    /// same image printed opposite verdicts — one `ZOMBIES REMAIN — the reap
    /// loop is wrong` naming `pid 103 (rivet-netd) state Z`, one
    /// `Stage 0 reap criterion met` — because the audit runs the moment the
    /// self-test's own children are reaped and that instant **races
    /// `rivet-netd`'s exit**. `netd` is the *supervisor's* child, so its
    /// zombie is the supervisor's `waitpid` to make and is gone microseconds
    /// later. PID 1's reap loop was never wrong.
    ///
    /// [`crate::boot::Census`] splits the zombies by parent for exactly this reason, and
    /// only [`crate::boot::Census::unreaped`] — parented to PID 1, or with an unreadable
    /// parent — decides the verdict. The others are still printed, because a
    /// supervisor that stopped reaping is worth seeing; they just are not
    /// PID 1's defect and must not be announced as one. A line that cries wolf
    /// on half of all boots is worse than a silent bug: it teaches the reader
    /// to skip the one line that would matter.
    fn report_census(&mut self, k: &mut dyn Kernel) {
        let Some(t) = self.selftest.as_ref() else {
            return;
        };
        let (direct, maker) = (t.direct, t.maker);
        k.say("rivet-initd: self-test complete; auditing /proc for zombies");
        for (label, pid) in [("direct child", direct), ("orphan-maker", maker)] {
            match proc_state(k, pid) {
                Ok(None) => k.say(&format!("  /proc/{pid} is gone — {label} fully reaped")),
                Ok(Some(stat)) => k.say(&format!(
                    "  /proc/{pid} still present in state {} — {label} NOT reaped",
                    stat.state
                )),
                Err(e) => k.say(&format!("  /proc/{pid} unreadable: {e} — {label}")),
            }
        }
        match zombie_census(k) {
            Ok(census) => {
                k.say(&format!(
                    "  /proc lists {} process(es); {} in state Z parented to pid 1, {} to someone else",
                    census.processes,
                    census.unreaped.len(),
                    census.elsewhere.len()
                ));
                // Printed before the verdict either way, and never as part of
                // it: a zombie with a live parent that is not PID 1 is that
                // parent's to wait on. `rivet-netd` exits once and is one of
                // these for as long as the supervisor takes to notice.
                for z in &census.elsewhere {
                    k.say(&format!("    not pid 1's to reap: {z}"));
                }
                if census.unreaped.is_empty() {
                    if self.health.may_confirm_boot() {
                        k.say(
                            "rivet-initd: no zombies of pid 1 remain, per /proc. Stage 0 reap criterion met.",
                        );
                    } else {
                        // Honesty over tidiness: the reap criterion is met and
                        // the boot is still not one we will certify. Saying only
                        // the first half would be the "wired up means works"
                        // error.
                        k.say("rivet-initd: no zombies of pid 1 remain, per /proc. Reaping is sound, but the");
                        k.say("             mount phase was UNCONFIRMABLE — this boot is not certified.");
                    }
                } else {
                    for z in &census.unreaped {
                        k.say(&format!("    {z}"));
                    }
                    k.say("rivet-initd: ZOMBIES REMAIN — the reap loop is wrong.");
                }
            }
            Err(e) => k.say(&format!("  cannot read /proc: {e}")),
        }
    }

    /// Item 5, scheduled: decided once, when the system has stayed up.
    ///
    /// **One decision, of at most [`confirm::COMMIT_ATTEMPTS`] attempts.** An
    /// attempt the boot medium refused (`Failed { medium: true, .. }`) is
    /// journalled with its number and a listing of the default-slot record's
    /// partition, and — while attempts remain — scheduled again
    /// [`confirm::COMMIT_RETRY_DELAY_MS`] later as a timer in this loop, never
    /// as a sleep. Each attempt mounts the partition afresh. Any other outcome,
    /// or the last refusal, is the decision and is stored; nothing attempts the
    /// commit after that, and nothing does during a shutdown.
    fn maybe_confirm(&mut self, k: &mut dyn Kernel) {
        if self.confirmation.is_some() {
            return;
        }
        // A gate that already refuses will still refuse later, so say so now
        // rather than leaving the console silent about it.
        if let Err(why) = confirm::gate(&self.health, &self.provisioning, &self.supervisor) {
            let outcome = Confirmation::Withheld { why };
            k.say(&format!("rivet-initd: {}", outcome.summary()));
            self.confirmation = Some(outcome);
            return;
        }
        let now = k.now_ms();
        if !self.supervisor.settled(now) {
            return;
        }
        if self.next_commit_at_ms.is_some_and(|at| now < at) {
            return;
        }
        self.next_commit_at_ms = None;
        let outcome = confirm::confirm_boot(
            k,
            self.slots.as_ref(),
            &self.health,
            &self.provisioning,
            &self.supervisor,
        );
        // **`WaitingToBeReached` is not remembered.** It means *not yet*, so
        // storing it would make the first pass the last one and a machine
        // reached five minutes after boot would never confirm. The next check
        // is scheduled `REACHED_CHECK_INTERVAL_MS` from now, as a timer in this
        // loop: without it the supervisor's settle time, being past, would
        // shorten every wait to the loop's floor. Said ONCE, because a console
        // repeating the same paragraph every second is a console nobody reads.
        if outcome == confirm::Confirmation::WaitingToBeReached {
            self.next_commit_at_ms =
                Some(now.saturating_add(confirm::REACHED_CHECK_INTERVAL_MS));
            if !self.said_waiting_to_be_reached {
                self.said_waiting_to_be_reached = true;
                k.say(&format!("rivet-initd: {}", outcome.summary()));
            }
            return;
        }
        if let Confirmation::Failed { why, medium: true } = &outcome {
            self.commit_refusals = self.commit_refusals.saturating_add(1);
            let attempt = self.commit_refusals;
            k.say(&format!(
                "rivet-initd: boot commit attempt {attempt} of {} failed: {why}",
                confirm::COMMIT_ATTEMPTS
            ));
            self.say_record_listing(k, attempt);
            if attempt < confirm::COMMIT_ATTEMPTS {
                self.next_commit_at_ms = Some(now.saturating_add(confirm::COMMIT_RETRY_DELAY_MS));
                k.say(&format!(
                    "rivet-initd: attempting the commit again in {} ms, with the partition \
                     mounted afresh",
                    confirm::COMMIT_RETRY_DELAY_MS
                ));
                return;
            }
        }
        match &outcome {
            Confirmation::Confirmed { .. } | Confirmation::AlreadyDefault { .. }
                if self.commit_refusals > 0 =>
            {
                k.say(&format!(
                    "rivet-initd: {} (on attempt {} of {})",
                    outcome.summary(),
                    self.commit_refusals.saturating_add(1),
                    confirm::COMMIT_ATTEMPTS
                ));
            }
            _ => k.say(&format!("rivet-initd: {}", outcome.summary())),
        }
        self.confirmation = Some(outcome);
    }

    /// Journal what the default-slot record's partition holds, for a commit
    /// attempt the medium refused.
    ///
    /// **Never a reason to stop.** A listing that cannot be taken is said and
    /// the decision carries on exactly as it would have; the port mounts the
    /// partition read-only for it and unmounts it again, so the next attempt
    /// starts from the same place either way.
    fn say_record_listing(&self, k: &mut dyn Kernel, attempt: u32) {
        let listing = {
            let mut medium = crate::slots::KernelMedium::new(&mut *k);
            self.slots
                .record_medium_listing(&mut medium, confirm::LISTING_LIMIT)
        };
        let listing = match listing {
            Ok(listing) => listing,
            Err(why) => {
                k.say(&format!(
                    "rivet-initd: attempt {attempt}: could not list the default-slot record's \
                     partition: {why}"
                ));
                return;
            }
        };
        k.say(&format!(
            "rivet-initd: attempt {attempt}: the default-slot record's partition holds {} \
             entries; the first {} by name:",
            listing.total,
            listing.entries.len()
        ));
        for entry in &listing.entries {
            let size = match &entry.size {
                Ok(bytes) => format!("{bytes} bytes"),
                Err(e) => format!("size unreadable: {e}"),
            };
            k.say(&format!("rivet-initd: attempt {attempt}:   {} ({size})", entry.name));
        }
    }

    /// Item 6. Stops the supervisor, flushes, and asks the kernel to stop the
    /// machine — and returns if the kernel refuses, because PID 1 must not exit
    /// even then.
    ///
    /// **All three signals arrive here**, including `SIGUSR1`'s trial boot
    /// (`ADR-017` §3.1.10). The order is load-bearing for `ADR-021` §3.3: the
    /// supervisor is stopped *first*, and the shutdown does not proceed until it
    /// has exited or the grace period has run out, which is the window in which
    /// the supervisor writes `/state/clock`. `crate::supervise::STOP_GRACE_MS`
    /// bounds that window, and it is shorter than the supervisor's own worst
    /// case — see [`Pid1::stop_supervisor`].
    fn shut_down(&mut self, k: &mut dyn Kernel, cause: Sig, how: RebootHow) {
        self.shutting_down = true;
        k.say(&format!(
            "rivet-initd: {cause} received; shutting down to {how}"
        ));
        self.stop_supervisor(k);

        // A Pi has no battery and usually an SD card. ADR-015 §7 makes the same
        // point about half-installed applications being an ordinary failure.
        k.say("rivet-initd: syncing filesystems");
        k.sync();

        match k.reboot(how) {
            Ok(()) => k.say(&format!(
                "rivet-initd: the kernel accepted the {how} request; waiting for the machine to stop"
            )),
            Err(e) => {
                k.say(&format!("rivet-initd: the kernel refused to {how}: {e}"));
                k.say("rivet-initd: pid 1 cannot exit, so it keeps reaping. The board must be");
                k.say("             power-cycled to stop it.");
            }
        }
    }

    /// `SIGTERM`, then wait, then `SIGKILL`. Nothing else in the shutdown runs
    /// until this returns.
    ///
    /// **The grace period is also the supervisor's window to write the clock**
    /// (`ADR-021` §3.3). [`STOP_GRACE_MS`] is `rivet_core::shutdown`'s
    /// `SUPERVISOR_STOP_GRACE_MS`, derived to exceed the whole of what one
    /// service can take to stop under `rivet-sysd`'s own `stop_grace_ms` and
    /// `kill_grace_ms`, with a compile-time assertion pinning that ordering —
    /// so the two cannot drift apart by tuning either number alone, and PID 1
    /// cannot `SIGKILL` the supervisor while it is still inside a wait it is
    /// entitled to. The clock write is deliberately placed at the **start** of
    /// the supervisor's shutdown rather than at the end, so that it does not
    /// depend on how much of its own grace period the supervisor actually
    /// used.
    fn stop_supervisor(&mut self, k: &mut dyn Kernel) {
        // Whatever happens next, an exit from here is expected rather than a
        // crash: it must not schedule a restart or spend the retry budget.
        self.supervisor.expect_exit = true;

        let Some(pid) = self.supervisor.pid else {
            k.say("rivet-initd: no supervisor is running; nothing to stop");
            return;
        };
        k.say(&format!(
            "rivet-initd: stopping supervisor pid {pid} with SIGTERM"
        ));
        if let Err(e) = k.kill(pid, Sig::Term) {
            k.say(&format!("rivet-initd: could not signal pid {pid}: {e}"));
        }

        let deadline = k.now_ms().saturating_add(STOP_GRACE_MS);
        loop {
            self.reap_all(k);
            if self.supervisor.pid.is_none() {
                k.say("rivet-initd: supervisor stopped");
                return;
            }
            if k.now_ms() >= deadline {
                break;
            }
            k.sleep_ms(STOP_POLL_MS);
        }

        k.say(&format!(
            "rivet-initd: supervisor pid {pid} did not stop within {STOP_GRACE_MS}ms; SIGKILL"
        ));
        if let Err(e) = k.kill(pid, Sig::Kill) {
            k.say(&format!("rivet-initd: could not kill pid {pid}: {e}"));
        }
        self.reap_all(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{ExitStatus, Reaped};
    use crate::supervise::{CONFIRM_SETTLE_MS, RETRY_BUDGET, SUPERVISOR_PATH};
    use crate::testkernel::{FakeKernel, Script};
    use crate::confirm::{COMMIT_ATTEMPTS, COMMIT_RETRY_DELAY_MS, REACHED_CHECK_INTERVAL_MS};
    use rivet_adapter_image::{
        AUTOBOOT_FILE, AUTOBOOT_PARTITION_MOUNT, AUTOBOOT_TEMP, DT_BOOT_PARTITION, DT_TRYBOOT,
        PiFirmware,
    };

    fn child(pid: i32, code: i32) -> Result<Reaped, Errno> {
        Ok(Reaped::Child {
            pid,
            status: ExitStatus::Exited(code),
        })
    }

    fn selftest() -> SelfTest {
        SelfTest {
            direct: 53,
            maker: 54,
            expect_orphan: true,
            seen_direct: false,
            seen_maker: false,
            seen_orphan: false,
            reported: false,
        }
    }

    fn pid1(mode: WakeMode) -> Pid1 {
        Pid1::new(
            mode,
            BootHealth::Healthy,
            Provisioning::Absent {
                why: "test".to_string(),
            },
            Supervisor::new(SUPERVISOR_PATH),
            None,
            Box::new(PiFirmware),
        )
    }

    fn quiet_proc(k: &mut FakeKernel) {
        k.dirs_mut()
            .insert("/proc".to_string(), Script::always(Ok(vec!["1".into()])));
        k.files_mut().insert(
            "/proc/1/stat".into(),
            Script::always(Ok("1 (init) R 0".into())),
        );
    }

    // -- mode selection -------------------------------------------------------

    #[test]
    fn a_working_signalfd_gives_the_signal_loop() {
        let mut k = FakeKernel::new();
        assert_eq!(choose_mode(&mut k), WakeMode::Signals);
        assert!(!k.lines.iter().any(|l| l.contains("DEGRADED")));
    }

    #[test]
    fn a_refused_signalfd_degrades_and_admits_shutdown_is_unavailable() {
        let mut k = FakeKernel::new();
        k.arm_result = Err(Errno::EPERM);
        assert_eq!(choose_mode(&mut k), WakeMode::DegradedPoll);
        let console = k.lines.join("\n");
        assert!(console.contains("DEGRADED"), "{console}");
        assert!(console.contains("shutdown is not available"), "{console}");
    }

    #[test]
    fn the_degraded_loop_sleeps_between_polls_rather_than_spinning() {
        let mut k = FakeKernel::new();
        let mut p = pid1(WakeMode::DegradedPoll);
        p.supervisor.given_up = true; // nothing scheduled
        p.tick(&mut k);
        p.tick(&mut k);
        assert_eq!(k.slept_ms, vec![POLL_INTERVAL_MS; 2]);
        assert_eq!(k.await_calls, 0);
    }

    #[test]
    fn a_signalfd_that_breaks_mid_run_downgrades_instead_of_spinning_on_it() {
        let mut k = FakeKernel::new();
        k.await_results = Script::sequence(vec![Ok(None)], Err(Errno::EIO));
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.given_up = true;

        p.tick(&mut k);
        assert_eq!(p.mode, WakeMode::Signals);
        p.tick(&mut k);
        assert_eq!(p.mode, WakeMode::DegradedPoll);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("falling back to polling"))
        );
    }

    #[test]
    fn an_interrupted_wait_is_ordinary_and_does_not_downgrade() {
        let mut k = FakeKernel::new();
        k.await_results = Script::always(Err(Errno::EINTR));
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.given_up = true;
        p.tick(&mut k);
        assert_eq!(p.mode, WakeMode::Signals);
    }

    // -- how long the loop waits ---------------------------------------------

    #[test]
    fn the_wait_shortens_to_the_supervisors_next_attempt() {
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.next_attempt_at_ms = 700;
        assert_eq!(p.wait_ms(200), 500);
    }

    #[test]
    fn the_wait_is_never_zero_because_a_zero_timeout_is_a_spin() {
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.next_attempt_at_ms = 0;
        assert_eq!(p.wait_ms(1_000), 1);
    }

    #[test]
    fn a_running_supervisor_schedules_the_moment_the_boot_becomes_confirmable() {
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.pid = Some(200);
        p.supervisor.started_at_ms = 0;
        assert_eq!(p.wait_ms(0), CONFIRM_SETTLE_MS);
        assert_eq!(p.wait_ms(CONFIRM_SETTLE_MS - 5), 5);
    }

    #[test]
    fn once_the_boot_has_been_decided_only_the_idle_wait_remains() {
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.pid = Some(200);
        p.confirmation = Some(Confirmation::NothingToConfirm);
        assert_eq!(p.wait_ms(0), IDLE_WAIT_MS);
    }

    // -- restarting -----------------------------------------------------------

    #[test]
    fn the_loop_starts_the_supervisor_on_its_first_pass() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        let mut p = pid1(WakeMode::Signals);
        p.tick(&mut k);
        assert_eq!(k.spawn_calls, vec![SUPERVISOR_PATH.to_string()]);
        assert_eq!(p.supervisor.pid, Some(200));
    }

    #[test]
    fn a_supervisor_exit_is_labelled_as_the_supervisors_and_not_as_an_orphan() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        k.wait_results = Script::sequence(vec![child(200, 3)], Err(Errno::ECHILD));
        let mut p = Pid1::new(
            WakeMode::Signals,
            BootHealth::Healthy,
            Provisioning::Absent { why: "t".into() },
            Supervisor::new(SUPERVISOR_PATH),
            Some(selftest()),
            Box::new(PiFirmware),
        );

        p.tick(&mut k);

        let console = k.lines.join("\n");
        assert!(console.contains("supervisor pid 200"), "{console}");
        assert!(!console.contains("ORPHAN"), "{console}");
        assert!(
            p.selftest.as_ref().is_some_and(|t| !t.seen_orphan),
            "the supervisor's exit must not complete the self-test"
        );
    }

    // -- the zombie audit's verdict ------------------------------------------

    /// Stage `/proc` with `n` live processes plus whatever zombies are given,
    /// each as `(pid, comm, ppid)`.
    fn proc_with(k: &mut FakeKernel, live: &[(i32, &str)], zombies: &[(i32, &str, i32)]) {
        let mut names: Vec<String> = Vec::new();
        for (pid, comm) in live {
            names.push(pid.to_string());
            k.files_mut().insert(
                format!("/proc/{pid}/stat"),
                Script::always(Ok(format!("{pid} ({comm}) S 1"))),
            );
        }
        for (pid, comm, ppid) in zombies {
            names.push(pid.to_string());
            k.files_mut().insert(
                format!("/proc/{pid}/stat"),
                Script::always(Ok(format!("{pid} ({comm}) Z {ppid}"))),
            );
        }
        k.dirs_mut()
            .insert("/proc".to_string(), Script::always(Ok(names)));
    }

    /// Run a self-test to completion so that `report_census` fires.
    fn drive_selftest_to_the_audit(k: &mut FakeKernel) -> Pid1 {
        k.spawn_results = Script::always(Ok(200));
        // The two children the self-test forked, then the orphan the kernel
        // reparented onto pid 1. The third completes it and triggers the audit.
        k.wait_results = Script::sequence(
            vec![child(53, 0), child(54, 0), child(99, 0)],
            Err(Errno::ECHILD),
        );
        let mut p = Pid1::new(
            WakeMode::Signals,
            BootHealth::Healthy,
            Provisioning::Absent { why: "t".into() },
            Supervisor::new(SUPERVISOR_PATH),
            Some(selftest()),
            Box::new(PiFirmware),
        );
        p.tick(k);
        p
    }

    #[test]
    fn a_supervisor_owned_zombie_does_not_make_the_reap_loop_wrong() {
        // The board case, 2026-08-23. Two boots of one image disagreed: one
        // said the reap loop was wrong and named `pid 103 (rivet-netd)`, the
        // other said the criterion was met. `netd` is the supervisor's child,
        // so the audit was racing an exit that was never pid 1's to wait on.
        let mut k = FakeKernel::new();
        proc_with(
            &mut k,
            &[(1, "rivet-initd"), (200, "rivet-sysd")],
            &[(103, "rivet-netd", 200)],
        );

        let p = drive_selftest_to_the_audit(&mut k);

        assert!(
            p.selftest.as_ref().is_some_and(|t| t.reported),
            "the audit must have run for this test to mean anything"
        );
        let console = k.lines.join("\n");
        assert!(
            !console.contains("ZOMBIES REMAIN"),
            "a zombie parented to the supervisor is not pid 1's defect:\n{console}"
        );
        assert!(console.contains("Stage 0 reap criterion met"), "{console}");
        // It is still shown — a supervisor that stopped reaping is worth
        // seeing — just not as pid 1's failure.
        assert!(
            console.contains("not pid 1's to reap: pid 103 (rivet-netd) state Z, parent 200"),
            "{console}"
        );
    }

    #[test]
    fn a_zombie_parented_to_pid_one_still_fails_the_criterion() {
        // The other direction, and the one that must never be softened: this
        // is the reap loop's own child, left unwaited.
        let mut k = FakeKernel::new();
        proc_with(&mut k, &[(1, "rivet-initd")], &[(53, "leftover", 1)]);

        drive_selftest_to_the_audit(&mut k);

        let console = k.lines.join("\n");
        assert!(console.contains("ZOMBIES REMAIN"), "{console}");
        assert!(
            console.contains("pid 53 (leftover) state Z, parent 1"),
            "{console}"
        );
        assert!(!console.contains("Stage 0 reap criterion met"), "{console}");
    }

    #[test]
    fn a_crash_looping_supervisor_eventually_stops_being_restarted() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut p = pid1(WakeMode::Signals);
        for _ in 0..(RETRY_BUDGET + 5) {
            p.tick(&mut k);
        }
        assert!(p.supervisor.given_up);
        assert_eq!(
            k.spawn_calls.len(),
            usize::try_from(RETRY_BUDGET).unwrap_or(0)
        );
    }

    // -- SIGTERM --------------------------------------------------------------

    #[test]
    fn sigterm_stops_the_supervisor_syncs_and_powers_off() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        k.await_results = Script::sequence(vec![Ok(None), Ok(Some(Sig::Term))], Ok(None));
        // It is alive through both ticks' drains, and exits only once the
        // shutdown has actually signalled it.
        k.wait_results = Script::sequence(
            vec![Ok(Reaped::NoneReady), Ok(Reaped::NoneReady), child(200, 0)],
            Err(Errno::ECHILD),
        );
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k); // starts the supervisor
        p.tick(&mut k); // SIGTERM

        assert!(p.shutting_down);
        assert_eq!(k.kill_calls, vec![(200, Sig::Term)]);
        assert_eq!(k.sync_calls, 1);
        assert_eq!(k.reboot_calls, vec![RebootHow::PowerOff]);
    }

    #[test]
    fn sigint_reboots_rather_than_powering_off() {
        let mut k = FakeKernel::new();
        k.await_results = Script::always(Ok(Some(Sig::Interrupt)));
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut p = pid1(WakeMode::Signals);
        p.tick(&mut k);
        assert_eq!(k.reboot_calls, vec![RebootHow::Restart]);
    }

    // -- SIGUSR1: the trial boot (`ADR-017` §3.1.10) ---------------------------

    /// **The whole of Part 1, from the outside.** The signal that had no
    /// receiver reaches the reboot that had no caller.
    #[test]
    fn sigusr1_stops_the_supervisor_syncs_and_asks_for_the_trial_slot() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        k.await_results = Script::sequence(vec![Ok(None), Ok(Some(Sig::User1))], Ok(None));
        k.wait_results = Script::sequence(
            vec![Ok(Reaped::NoneReady), Ok(Reaped::NoneReady), child(200, 0)],
            Err(Errno::ECHILD),
        );
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k); // starts the supervisor
        p.tick(&mut k); // SIGUSR1

        assert!(p.shutting_down);
        assert_eq!(
            k.kill_calls,
            vec![(200, Sig::Term)],
            "a trial boot must stop the supervisor exactly as a power-off does"
        );
        assert_eq!(k.sync_calls, 1, "a trial boot must flush the card");
        assert_eq!(k.reboot_calls, vec![RebootHow::TrialRestart]);
    }

    /// The three requests must not collapse into one another. A `SIGUSR1` that
    /// reached `Restart` would reboot the board, look entirely healthy, and
    /// trial nothing — `ADR-018` §5.1's silent-success direction.
    #[test]
    fn each_of_the_three_signals_asks_the_kernel_for_a_different_thing() {
        let cases = [
            (Sig::Term, RebootHow::PowerOff),
            (Sig::Interrupt, RebootHow::Restart),
            (Sig::User1, RebootHow::TrialRestart),
        ];
        for (sig, expected) in cases {
            let mut k = FakeKernel::new();
            k.spawn_results = Script::always(Err(Errno::ENOENT));
            k.await_results = Script::always(Ok(Some(sig)));
            let mut p = pid1(WakeMode::Signals);
            p.tick(&mut k);
            assert_eq!(
                k.reboot_calls,
                vec![expected],
                "{sig} asked for the wrong thing"
            );
            // ...and it said which one it was, because PID 1's console lines are
            // the only diagnostic a board gives here.
            assert!(
                k.lines.iter().any(|l| l.contains(&sig.to_string())),
                "{sig} was not named on the console: {:?}",
                k.lines
            );
        }
    }

    /// A trial boot the kernel refuses must leave PID 1 running, exactly as a
    /// refused power-off does. `reboot(2)` with `LINUX_REBOOT_CMD_RESTART2` is
    /// a *different syscall* from the other two (`crate::kernel`), so it has its
    /// own way to fail — `EINVAL` from a firmware that does not know the
    /// argument, which is every non-Raspberry-Pi machine.
    #[test]
    fn a_refused_trial_boot_leaves_pid_one_running_like_any_other_refusal() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::sequence(vec![Ok(Some(Sig::User1))], Ok(None));
        k.reboot_results = Script::always(Err(Errno::EINVAL));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        let console = k.lines.join("\n");
        assert!(console.contains("the kernel refused"), "{console}");
        assert!(console.contains("cannot exit"), "{console}");
        assert!(p.shutting_down);

        // And it is still a working reaper afterwards.
        k.wait_results = Script::sequence(vec![child(99, 0)], Err(Errno::ECHILD));
        p.tick(&mut k);
        assert!(k.lines.iter().any(|l| l.contains("reaped pid 99")));
    }

    /// Nothing may be confirmed on the way out of a trial boot. The slot being
    /// left is the *current* one; certifying it here would commit the image the
    /// operator is trying to move away from.
    #[test]
    fn a_trial_boot_confirms_nothing_on_its_way_out() {
        let mut k = tryboot_fake(b"0");
        k.await_results = Script::sequence(vec![Ok(Some(Sig::User1))], Ok(None));
        let mut p = pid1(WakeMode::Signals);
        p.tick(&mut k);
        p.tick(&mut k);
        assert_eq!(p.confirmation, None, "a shutdown certified a boot");
        assert!(!k.lines.iter().any(|l| l.contains("not a tryboot boot")));
    }

    /// The required case: nothing has been started, so there is nothing to
    /// signal — and signalling pid 0 or -1 from pid 1 would hit the machine.
    #[test]
    fn sigterm_before_the_supervisor_ever_started_signals_nothing() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::always(Ok(Some(Sig::Term)));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);

        assert!(k.kill_calls.is_empty(), "nothing was running to signal");
        assert!(k.lines.iter().any(|l| l.contains("nothing to stop")));
        assert_eq!(k.reboot_calls, vec![RebootHow::PowerOff]);
    }

    /// The other required case: a child is mid-reap when the request arrives.
    /// The reap must complete and the shutdown must still happen.
    #[test]
    fn sigterm_arriving_while_a_child_is_being_reaped_loses_neither() {
        let mut k = FakeKernel::new();
        quiet_proc(&mut k);
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results =
            Script::sequence(vec![Ok(Some(Sig::Child)), Ok(Some(Sig::Term))], Ok(None));
        k.wait_results = Script::sequence(
            vec![child(53, 7), child(54, 0), child(55, 9)],
            Err(Errno::ECHILD),
        );
        let mut p = Pid1::new(
            WakeMode::Signals,
            BootHealth::Healthy,
            Provisioning::Absent { why: "t".into() },
            Supervisor::new(SUPERVISOR_PATH),
            Some(selftest()),
            Box::new(PiFirmware),
        );

        p.tick(&mut k); // SIGCHLD: reap all three
        let reaped_before_shutdown = k.lines.iter().filter(|l| l.contains("reaped pid")).count();
        p.tick(&mut k); // SIGTERM

        assert_eq!(reaped_before_shutdown, 3, "{:?}", k.lines);
        assert!(k.lines.iter().any(|l| l.contains("0 in state Z")));
        assert!(p.shutting_down);
        assert_eq!(k.reboot_calls, vec![RebootHow::PowerOff]);
    }

    #[test]
    fn a_supervisor_that_ignores_sigterm_is_killed_after_the_grace_period() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        k.await_results = Script::sequence(vec![Ok(None), Ok(Some(Sig::Term))], Ok(None));
        // It never exits, so the drain never reports it.
        k.wait_results = Script::always(Err(Errno::ECHILD));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        p.tick(&mut k);

        assert_eq!(k.kill_calls, vec![(200, Sig::Term), (200, Sig::Kill)]);
        assert!(k.lines.iter().any(|l| l.contains("SIGKILL")));
        assert_eq!(k.reboot_calls, vec![RebootHow::PowerOff]);
    }

    #[test]
    fn stopping_the_supervisor_deliberately_is_not_counted_as_a_failure() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        k.await_results = Script::sequence(vec![Ok(None), Ok(Some(Sig::Term))], Ok(None));
        k.wait_results = Script::sequence(
            vec![Ok(Reaped::NoneReady), Ok(Reaped::NoneReady), child(200, 0)],
            Err(Errno::ECHILD),
        );
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        p.tick(&mut k);

        assert_eq!(p.supervisor.failures, 0, "a deliberate stop is not a crash");
        assert!(
            !k.lines.iter().any(|l| l.contains("retry")),
            "{:?}",
            k.lines
        );
    }

    /// The required case: PID 1 must not return even when the kernel says no.
    #[test]
    fn a_refused_reboot_leaves_pid_one_running_and_saying_why() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::sequence(vec![Ok(Some(Sig::Term))], Ok(None));
        k.reboot_results = Script::always(Err(Errno::EPERM));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        let console = k.lines.join("\n");
        assert!(console.contains("the kernel refused"), "{console}");
        assert!(console.contains("cannot exit"), "{console}");

        // And it is still a working loop afterwards.
        k.wait_results = Script::sequence(vec![child(99, 0)], Err(Errno::ECHILD));
        p.tick(&mut k);
        assert!(k.lines.iter().any(|l| l.contains("reaped pid 99")));
    }

    /// Found by running it: after a refused reboot the supervisor's stale
    /// `next_attempt_at_ms` is in the past, so an unguarded loop computes a
    /// one-millisecond wait and spins a core for ever.
    #[test]
    fn a_refused_reboot_does_not_leave_pid_one_spinning() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::sequence(vec![Ok(Some(Sig::Term))], Ok(None));
        k.reboot_results = Script::always(Err(Errno::EPERM));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        assert!(p.shutting_down);
        assert!(p.supervisor.expect_exit);

        // Let real time pass the stale backoff deadline, which is the state
        // that produced the spin.
        k.sleep_ms(1_000);
        assert!(p.supervisor.next_attempt_at_ms <= k.now_ms());
        assert_eq!(p.wait_ms(k.now_ms()), IDLE_WAIT_MS);

        for _ in 0..5 {
            p.tick(&mut k);
        }
        assert!(
            k.awaited_timeouts_ms
                .iter()
                .skip(1)
                .all(|ms| *ms == IDLE_WAIT_MS),
            "the loop is spinning: {:?}",
            k.awaited_timeouts_ms
        );
    }

    /// A supervisor wedged in uninterruptible sleep survives both `SIGTERM` and
    /// `SIGKILL`, so it is still "running" while PID 1 shuts down. Nothing will
    /// be certified now, so the settle timer must stop shortening the wait too
    /// — otherwise PID 1 wakes every few milliseconds for a decision it will
    /// never take.
    #[test]
    fn a_shutdown_that_cannot_kill_the_supervisor_still_stops_scheduling() {
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.pid = Some(200);
        p.supervisor.started_at_ms = 0;
        assert_eq!(p.wait_ms(0), CONFIRM_SETTLE_MS, "before the shutdown");
        p.shutting_down = true;
        assert_eq!(p.wait_ms(0), IDLE_WAIT_MS, "after it");
    }

    #[test]
    fn a_supervisor_being_stopped_on_purpose_schedules_no_restart() {
        let mut s = Supervisor::new(SUPERVISOR_PATH);
        s.next_attempt_at_ms = 0;
        assert_eq!(s.next_due_in_ms(1_000), Some(0));
        s.expect_exit = true;
        assert_eq!(s.next_due_in_ms(1_000), None);
    }

    #[test]
    fn nothing_is_restarted_once_a_shutdown_has_begun() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::sequence(vec![Ok(Some(Sig::Term))], Ok(None));
        k.reboot_results = Script::always(Err(Errno::EPERM));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        let spawns = k.spawn_calls.len();
        p.tick(&mut k);
        p.tick(&mut k);
        assert_eq!(k.spawn_calls.len(), spawns, "shutdown must not restart it");
    }

    #[test]
    fn an_unhandled_signal_is_named_rather_than_acted_on() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::always(Ok(Some(Sig::Other(28))));
        let mut p = pid1(WakeMode::Signals);
        p.tick(&mut k);
        assert!(k.lines.iter().any(|l| l.contains("ignoring signal 28")));
        assert!(k.reboot_calls.is_empty());
    }

    // -- confirmation ---------------------------------------------------------

    fn tryboot_fake(flag: &[u8]) -> FakeKernel {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Ok(200));
        k.blobs_mut()
            .insert(DT_TRYBOOT.to_string(), Script::always(Ok(flag.to_vec())));
        k
    }

    #[test]
    fn an_unconfirmable_mount_phase_is_reported_at_once_rather_than_after_the_settle() {
        let mut k = tryboot_fake(b"0");
        let mut p = Pid1::new(
            WakeMode::Signals,
            BootHealth::Unconfirmable {
                missing: vec!["/proc".to_string()],
            },
            Provisioning::Absent { why: "t".into() },
            Supervisor::new(SUPERVISOR_PATH),
            None,
            Box::new(PiFirmware),
        );
        p.tick(&mut k);
        assert!(matches!(
            p.confirmation,
            Some(Confirmation::Withheld { .. })
        ));
        assert!(k.lines.iter().any(|l| l.contains("NOT confirmed")));
    }

    #[test]
    fn confirmation_waits_until_the_supervisor_has_stayed_up() {
        let mut k = tryboot_fake(b"0");
        // A signal arrives at once, so almost no time passes on this tick.
        k.await_results = Script::sequence(vec![Ok(Some(Sig::Child))], Ok(None));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        assert!(k.now_ms() < CONFIRM_SETTLE_MS);
        assert_eq!(p.confirmation, None, "too early to certify anything");

        // The next wait is scheduled for exactly the settle moment.
        p.tick(&mut k);
        assert!(k.now_ms() >= CONFIRM_SETTLE_MS);
        assert_eq!(p.confirmation, Some(Confirmation::NothingToConfirm));
    }

    /// **One decision per boot, and at most [`COMMIT_ATTEMPTS`] attempts
    /// inside it.** An ordinary boot decides once and says so once; a trial
    /// boot whose medium refuses every time is attempted exactly
    /// `COMMIT_ATTEMPTS` times, declines once, and is never attempted again
    /// however long the loop runs.
    #[test]
    fn confirmation_is_one_decision_of_at_most_three_attempts() {
        let mut k = tryboot_fake(b"0");
        let mut p = pid1(WakeMode::Signals);
        for _ in 0..4 {
            p.tick(&mut k);
        }
        assert_eq!(
            k.lines
                .iter()
                .filter(|l| l.contains("not a tryboot boot"))
                .count(),
            1
        );

        let mut k = trial_of_slot_b();
        k.rename_results = Script::always(Err(Errno::ENOENT));
        let mut p = pid1(WakeMode::Signals);
        for _ in 0..20 {
            p.tick(&mut k);
        }
        assert_eq!(k.rename_calls.len(), 3, "attempts: {:?}", k.lines);
        assert_eq!(COMMIT_ATTEMPTS, 3);
        assert_eq!(
            k.lines
                .iter()
                .filter(|l| l.contains("could NOT be confirmed"))
                .count(),
            1,
            "one decision: {:?}",
            k.lines
        );
    }

    // -- retrying the commit --------------------------------------------------

    const AUTOBOOT: &str = "[all]\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n";

    /// A trial boot of slot B, reached, whose autoboot partition lists two
    /// files. Hand-composed.
    fn trial_of_slot_b() -> FakeKernel {
        let mut k = tryboot_fake(b"1");
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(b"2".to_vec())),
        );
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
            Script::always(Ok(AUTOBOOT.to_string())),
        );
        k.dirs_mut().insert(
            AUTOBOOT_PARTITION_MOUNT.to_string(),
            Script::always(Ok(vec![
                AUTOBOOT_TEMP.to_string(),
                AUTOBOOT_FILE.to_string(),
            ])),
        );
        k.file_sizes_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
            Script::always(Ok(44)),
        );
        k.file_sizes_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_TEMP}"),
            Script::always(Ok(45)),
        );
        k
    }

    fn run_until_decided(p: &mut Pid1, k: &mut FakeKernel) {
        for _ in 0..20 {
            if p.confirmation.is_some() {
                return;
            }
            p.tick(k);
        }
    }

    fn lines_with(k: &FakeKernel, needle: &str) -> usize {
        k.lines.iter().filter(|l| l.contains(needle)).count()
    }

    /// **The failure this retry exists for**: the medium refuses the rename
    /// once and the second attempt, with the partition mounted afresh, commits.
    /// The first failure is journalled with its attempt number and with what
    /// the partition held.
    #[test]
    fn a_medium_refusal_on_the_first_attempt_is_retried_and_commits() {
        let mut k = trial_of_slot_b();
        k.rename_results = Script::sequence(vec![Err(Errno::ENOENT)], Ok(()));
        let mut p = pid1(WakeMode::Signals);
        run_until_decided(&mut p, &mut k);

        assert_eq!(p.confirmation, Some(Confirmation::Confirmed { partition: 2 }));
        assert_eq!(k.rename_calls.len(), 2);
        assert_eq!(lines_with(&k, "attempt 1 of 3 failed"), 1, "{:?}", k.lines);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("attempt 1 of 3 failed") && l.contains("ENOENT")),
            "{:?}",
            k.lines
        );
        assert_eq!(lines_with(&k, "attempt 2 of 3 failed"), 0);
        // The listing, name and size, taken for the failed attempt only.
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("attempt 1:") && l.contains(AUTOBOOT_TEMP) && l.contains("45")),
            "{:?}",
            k.lines
        );
        assert_eq!(lines_with(&k, "attempt 2:"), 0, "{:?}", k.lines);
        assert_eq!(lines_with(&k, "boot confirmed"), 1);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("boot confirmed") && l.contains("attempt 2 of 3")),
            "{:?}",
            k.lines
        );
        // The retry waited in the loop for the delay, and did not sleep.
        assert!(
            k.awaited_timeouts_ms.contains(&COMMIT_RETRY_DELAY_MS),
            "{:?}",
            k.awaited_timeouts_ms
        );
        assert!(k.slept_ms.is_empty(), "{:?}", k.slept_ms);
    }

    /// Three refusals: three attempts, each journalled with a listing, and then
    /// the same decline a single attempt gave — never a claim of success.
    #[test]
    fn three_medium_refusals_decline_after_three_attempts_each_with_a_listing() {
        let mut k = trial_of_slot_b();
        k.rename_results = Script::always(Err(Errno::ENOENT));
        let mut p = pid1(WakeMode::Signals);
        run_until_decided(&mut p, &mut k);

        assert!(
            matches!(
                p.confirmation,
                Some(Confirmation::Failed { medium: true, .. })
            ),
            "{:?}",
            p.confirmation
        );
        assert_eq!(k.rename_calls.len(), 3);
        for n in 1..=3 {
            assert_eq!(
                lines_with(&k, &format!("attempt {n} of 3 failed")),
                1,
                "{:?}",
                k.lines
            );
            assert!(
                k.lines
                    .iter()
                    .any(|l| l.contains(&format!("attempt {n}:")) && l.contains(AUTOBOOT_TEMP)),
                "attempt {n} has no listing: {:?}",
                k.lines
            );
        }
        assert_eq!(lines_with(&k, "boot could NOT be confirmed"), 1);
        assert_eq!(lines_with(&k, "boot confirmed"), 0);
        assert_eq!(k.sync_calls, 3, "one sync per attempt, never the second");
    }

    /// A record the port will not rewrite is refused the same way every time,
    /// so it is attempted once and nothing is listed.
    #[test]
    fn an_unsafe_refusal_is_attempted_once() {
        let mut k = trial_of_slot_b();
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
            Script::always(Ok("nonsense\n".to_string())),
        );
        let mut p = pid1(WakeMode::Signals);
        run_until_decided(&mut p, &mut k);
        for _ in 0..5 {
            p.tick(&mut k);
        }

        assert!(
            matches!(
                p.confirmation,
                Some(Confirmation::Failed { medium: false, .. })
            ),
            "{:?}",
            p.confirmation
        );
        assert_eq!(k.mount_calls.len(), 1, "one attempt and no listing mount");
        assert!(k.write_calls.is_empty());
        assert_eq!(lines_with(&k, "attempt 1 of 3"), 0, "{:?}", k.lines);
        assert_eq!(lines_with(&k, "boot could NOT be confirmed"), 1);
    }

    /// **The retry does not block the loop.** A shutdown arriving between
    /// attempts is honoured at once, and no further attempt is made on the
    /// way out.
    #[test]
    fn a_shutdown_between_attempts_is_handled_and_abandons_the_retry() {
        let mut k = trial_of_slot_b();
        k.rename_results = Script::always(Err(Errno::ENOENT));
        k.await_results = Script::sequence(vec![Ok(None), Ok(Some(Sig::Term))], Ok(None));
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        assert_eq!(k.rename_calls.len(), 1, "the first attempt: {:?}", k.lines);
        assert!(k.slept_ms.is_empty(), "the retry slept: {:?}", k.slept_ms);

        p.tick(&mut k);
        assert_eq!(
            k.awaited_timeouts_ms.get(1),
            Some(&COMMIT_RETRY_DELAY_MS),
            "the loop waited for the retry, on the signal descriptor"
        );
        assert!(p.shutting_down);
        assert_eq!(k.reboot_calls, vec![RebootHow::PowerOff]);

        p.tick(&mut k);
        assert_eq!(k.rename_calls.len(), 1, "a retry ran during shutdown");
        assert_eq!(p.confirmation, None, "a shutdown decided the boot");
    }

    /// A child exiting between attempts is reaped when it exits, and the retry
    /// still runs when it is due.
    #[test]
    fn a_child_reaped_between_attempts_does_not_disturb_the_retry() {
        let mut k = trial_of_slot_b();
        k.rename_results = Script::sequence(vec![Err(Errno::ENOENT)], Ok(()));
        k.await_results = Script::sequence(vec![Ok(None), Ok(Some(Sig::Child))], Ok(None));
        k.wait_results = Script::sequence(
            vec![Err(Errno::ECHILD), child(99, 0)],
            Err(Errno::ECHILD),
        );
        let mut p = pid1(WakeMode::Signals);

        p.tick(&mut k);
        p.tick(&mut k);
        assert_eq!(lines_with(&k, "reaped pid 99"), 1, "{:?}", k.lines);
        assert_eq!(k.rename_calls.len(), 1, "the retry ran early");

        run_until_decided(&mut p, &mut k);
        assert_eq!(p.confirmation, Some(Confirmation::Confirmed { partition: 2 }));
        assert_eq!(k.rename_calls.len(), 2);
    }

    /// A listing that cannot be taken is said, and the next attempt still
    /// happens and commits. Two ways: the directory will not read, and the
    /// read-only mount for the listing is refused.
    #[test]
    fn a_failure_to_list_does_not_stop_the_next_attempt() {
        let mut unreadable = trial_of_slot_b();
        unreadable
            .dirs_mut()
            .insert(AUTOBOOT_PARTITION_MOUNT.to_string(), Script::always(Err(Errno::EIO)));
        unreadable.rename_results = Script::sequence(vec![Err(Errno::ENOENT)], Ok(()));

        let mut unmountable = trial_of_slot_b();
        unmountable.rename_results = Script::sequence(vec![Err(Errno::ENOENT)], Ok(()));
        // The first attempt's mount, then the listing's, then the retry's.
        unmountable.mount_results =
            Script::sequence(vec![Ok(()), Err(Errno::ENODEV)], Ok(()));

        for (label, mut k) in [("unreadable", unreadable), ("unmountable", unmountable)] {
            let mut p = pid1(WakeMode::Signals);
            run_until_decided(&mut p, &mut k);
            assert_eq!(
                p.confirmation,
                Some(Confirmation::Confirmed { partition: 2 }),
                "{label}: {:?}",
                k.lines
            );
            assert_eq!(
                lines_with(&k, "could not list"),
                1,
                "{label}: {:?}",
                k.lines
            );
        }
    }

    #[test]
    fn a_scheduled_retry_shortens_the_wait_to_its_due_time() {
        let mut p = pid1(WakeMode::Signals);
        p.supervisor.pid = Some(200);
        p.supervisor.started_at_ms = 0;
        p.next_commit_at_ms = Some(CONFIRM_SETTLE_MS + COMMIT_RETRY_DELAY_MS);
        assert_eq!(
            p.wait_ms(CONFIRM_SETTLE_MS + 1_000),
            COMMIT_RETRY_DELAY_MS - 1_000
        );
    }

    // -- waiting to be reached ------------------------------------------------

    /// **A trial boot nothing has reached is checked every
    /// `REACHED_CHECK_INTERVAL_MS`, not every millisecond.** Once the
    /// supervisor has settled, its settle time is past; without a scheduled
    /// check the loop's wait fell to its one-millisecond floor and PID 1
    /// re-read the device tree and the marker about a thousand times a second.
    #[test]
    fn a_trial_boot_waiting_to_be_reached_is_checked_at_the_named_interval() {
        let mut k = trial_of_slot_b();
        k.files
            .remove(rivet_core::operation::status::REACHED_MARKER_PATH);
        let mut p = pid1(WakeMode::Signals);
        for _ in 0..6 {
            p.tick(&mut k);
        }
        assert_eq!(p.confirmation, None, "{:?}", k.lines);
        assert_eq!(lines_with(&k, "NOT confirmed yet"), 1, "{:?}", k.lines);
        // The first waits are the supervisor's settle; every one after the
        // first check is the interval, and none is the floor.
        assert!(
            k.awaited_timeouts_ms
                .iter()
                .skip_while(|&&w| w != REACHED_CHECK_INTERVAL_MS)
                .count()
                >= 3,
            "{:?}",
            k.awaited_timeouts_ms
        );
        assert!(
            k.awaited_timeouts_ms
                .iter()
                .skip_while(|&&w| w != REACHED_CHECK_INTERVAL_MS)
                .all(|&w| w == REACHED_CHECK_INTERVAL_MS),
            "{:?}",
            k.awaited_timeouts_ms
        );
        assert!(!k.awaited_timeouts_ms.contains(&1), "{:?}", k.awaited_timeouts_ms);
        assert_eq!(k.mount_calls.len(), 0, "an unreached boot mounted the card");

        // ...and it commits on the first check after it is reached.
        k.files.insert(
            rivet_core::operation::status::REACHED_MARKER_PATH.to_string(),
            Script::always(Ok(String::new())),
        );
        p.tick(&mut k);
        assert_eq!(p.confirmation, Some(Confirmation::Confirmed { partition: 2 }));
    }

    /// The console's promise matches the interval the loop keeps.
    #[test]
    fn the_waiting_sentence_names_the_interval_the_loop_keeps() {
        assert_eq!(REACHED_CHECK_INTERVAL_MS, 1_000);
        assert!(
            Confirmation::WaitingToBeReached
                .summary()
                .contains("checks again every second")
        );
    }

    // -- the record's lock ----------------------------------------------------

    /// **PID 1 finding the lock held does not block**: the attempt is a
    /// medium refusal, nothing is mounted, no time passes inside it, and the
    /// retry commits once the holder has gone.
    #[test]
    fn a_held_lock_is_a_medium_refusal_that_neither_blocks_nor_mounts() {
        let mut k = trial_of_slot_b();
        // The first attempt's lock, then the listing's, then the retry's.
        k.lock_results = Script::sequence(
            vec![Err(Errno::EAGAIN), Err(Errno::EAGAIN)],
            Ok(()),
        );
        let mut p = pid1(WakeMode::Signals);
        run_until_decided(&mut p, &mut k);

        assert_eq!(p.confirmation, Some(Confirmation::Confirmed { partition: 2 }));
        assert!(
            k.lines.iter().any(|l| l.contains("attempt 1 of 3 failed")
                && l.contains(rivet_adapter_image::AUTOBOOT_LOCK)),
            "{:?}",
            k.lines
        );
        assert_eq!(k.mount_calls.len(), 1, "only the retry mounted: {:?}", k.lines);
        assert!(k.slept_ms.is_empty(), "PID 1 slept on the lock: {:?}", k.slept_ms);
        let lock = format!("lock {}", rivet_adapter_image::AUTOBOOT_LOCK);
        let unlock = format!("unlock {}", rivet_adapter_image::AUTOBOOT_LOCK);
        assert_eq!(
            k.lock_calls,
            vec![lock.clone(), lock.clone(), lock, unlock],
            "a lock never taken was released"
        );
    }

    /// **A leaked mount at the record's mount point costs no attempt.** PID 1
    /// reclaims it inside the first attempt and the boot is confirmed then —
    /// not three medium refusals and a rollback.
    #[test]
    fn a_leaked_mount_is_reclaimed_within_the_first_attempt() {
        let mut k = trial_of_slot_b();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(format!(
                "/dev/mmcblk0p1 {AUTOBOOT_PARTITION_MOUNT} vfat ro 0 0\n"
            ))),
        );
        k.mount_results = Script::sequence(vec![Err(Errno::EBUSY)], Ok(()));
        let mut p = pid1(WakeMode::Signals);
        run_until_decided(&mut p, &mut k);

        assert_eq!(p.confirmation, Some(Confirmation::Confirmed { partition: 2 }));
        assert_eq!(lines_with(&k, "attempt 1 of 3 failed"), 0, "{:?}", k.lines);
        assert_eq!(lines_with(&k, "reclaimed an orphaned mount"), 1, "{:?}", k.lines);
    }

    #[test]
    fn a_supervisor_given_up_on_withholds_confirmation_and_says_so_once() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let mut p = pid1(WakeMode::Signals);
        for _ in 0..(RETRY_BUDGET + 3) {
            p.tick(&mut k);
        }
        assert!(matches!(
            p.confirmation,
            Some(Confirmation::Withheld { .. })
        ));
        let withheld = k
            .lines
            .iter()
            .filter(|l| l.contains("NOT confirmed"))
            .count();
        assert_eq!(withheld, 1, "said once, not on every tick");
    }
}
