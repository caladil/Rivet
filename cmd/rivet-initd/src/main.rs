//! Rivet's PID 1.
//!
//! `ADR-017` §3.1.5 is the complete job list, and it is deliberately short:
//!
//! | | Item | Where |
//! | :-- | :-- | :-- |
//! | 1 | mount the boot-time filesystems (`ADR-017` §3.1.7) | [`boot`], [`state`] |
//! | 2 | read the provisioning file from the boot partition, **and store it** (`ADR-017` §3.1.9) | [`provision`], [`seed`] |
//! | 3 | spawn and restart `rivet-sysd`, with backoff | [`supervise`] |
//! | 4 | reap **every** orphan on the system | [`reap`], driven by [`pid1`] |
//! | 5 | confirm the boot once the system is healthy | [`confirm`] |
//! | 6 | handle `SIGTERM` for shutdown and reboot — **and `SIGINT`, and since 2026-08-23 `SIGUSR1` for a trial boot** (`ADR-017` §3.1.10) | [`pid1`] |
//! | 7 | never return | [`pid1::Pid1::run_forever`] |
//!
//! **Item 6 is the only one that grew**, and it grew by a signal rather than by
//! a job: `ADR-017` §3.1.10 rules that a trial boot (`ADR-018` §5.1) is a
//! shutdown like the other two and belongs on the same path, rather than being
//! a `reboot(2)` a login shell makes with the machine's services still running.
//! The three-signal contract is stated once, in [`pid1`]'s module docs, and
//! `SIGUSR1` means that and nothing else on a Rivet machine.
//!
//! Anything not on that list belongs in `cmd/rivet-sysd`, which is
//! allowed to fail and be restarted. PID 1 is not.
//!
//! Two rules govern everything here:
//!
//! 1. **It must never return.** A return from PID 1 is
//!    `Kernel panic - not syncing: Attempted to kill init!`, and the exit code
//!    in that message is the only diagnostic anyone gets. `main` is therefore
//!    written `-> !`, so the compiler refuses a path that falls off the end.
//!    Item 6 does not change this: a `reboot(2)` the kernel *refuses* leaves
//!    PID 1 running and saying why, because the alternative is a panic.
//! 2. **It must never panic.** Same outcome, less warning. Hence
//!    `panic = "abort"` in the release profile, `unwrap`/`expect`/`panic`
//!    denied in this crate's lints — tests included — and errors that are
//!    reported and survived rather than propagated.
//!
//! Iterate on this under QEMU — `tools/qemu/run.sh`, or `tools/qemu/smoke.sh`
//! for a pass/fail — and read `ADR-026` §4 before believing that a green run
//! means it works on hardware. QEMU settles mounting, `SIGCHLD` and reaping. It
//! settles **nothing** about the VideoCore boot chain, and `tryboot` — which is
//! the whole of item 5 — is in its right-hand column: not emulated at all.
//!
//! ## The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`kernel`] | the syscall seam: the `Kernel` port, and `LibcKernel`, which holds every `unsafe` block in the workspace |
//! | [`boot`] | the mount phase and the verdict it produces |
//! | [`provision`] | the boot partition, and the document on it |
//! | [`seed`] | writing that document to `/state/config` on a first boot |
//! | [`supervise`] | one child, restarted with backoff, given up on eventually |
//! | [`slots`] | the adapter from [`kernel`]'s `Kernel` to `rivet-adapter-image`'s `BootMedium` |
//! | [`confirm`] | whether PID 1 will certify this boot, and what it says about it |
//! | [`reap`] | draining, and the stage 0 self-test |
//! | [`pid1`] | the loop that never returns |
//! | `main` | the composition root: build a kernel and a boot-slot adapter, run the phases, park. No logic. |
//!
//! **The A/B boot primitive is a port for the same reason [`kernel`] is, and
//! the reason is not symmetry.** The maintainer's 2026-08-23 ruling settles why:
//! `ADR-018` §5's `tryboot` is a *firmware* one-shot that needs no userspace on
//! the failure path, and x86 has no equivalent — so the operations go behind a
//! trait now, at the cost of one trait and one implementor, rather than
//! becoming a rewrite of the update mechanism later.
//!
//! **That port lives in `rivet-adapter-image` since 2026-08-23**, having spent
//! a few hours in this crate. It moved because **a binary crate has no lib
//! target**, so `BootSlots` was unreachable from the supervisor, from the CLI
//! and from any future update path — and `ADR-028` §4 says an authenticated
//! principal supplies the authority to install an update without saying which
//! process executes it. [`slots`] is what is left here: the adapter, and the
//! mapping from the port's "which reboot carries this arming" onto
//! [`kernel::RebootHow`]. No behaviour changed in the move.
//!
//! The split is not tidiness. PID 1's failure mode is a kernel panic with no
//! console, and the branches that matter most — `/proc/self/mounts` absent on
//! every first boot, a read-only `/system` (`ADR-018` §3), `signalfd(2)`
//! refused, a supervisor that will not start, a `tryboot` flag no emulator
//! produces — cannot be reached from a test that needs root, a kernel and an
//! emulator. Putting the decisions above a port makes them reachable from a
//! fake. `AGENTS.md` §4: everything tested to its boundary.
//!
//! ## Why `libc` and not `rustix`
//!
//! `ADR-009` §3 says survey before writing, and record the reason for the step
//! taken. `rustix` was the preferred candidate — pure Rust syscalls, no C in the
//! call path, which suits `ADR-003`'s static-musl posture and `ADR-026` §3.1's
//! finding that Rivet's own binaries need no C cross-toolchain. It covers
//! `mount(2)` and `waitpid(2)` with a typed, safe API and it would have been the
//! choice for those two alone.
//!
//! It does not cover the third thing PID 1 needs. Reading `rustix` 1.1.4's
//! source: `src/not_implemented.rs` carries `not_implemented!(signalfd)`, and
//! signal *disposition* lives only in `rustix::runtime`, which is `doc(hidden)`,
//! declares its API unstable, and warns that several of its functions "cannot be
//! used in a process which also has a libc present" — which this process is,
//! because `std` links musl on `aarch64-unknown-linux-musl` regardless. Taking
//! both crates to do one job puts two syscall vocabularies in the one process
//! that must not fail, so this uses `libc` alone: conventional, complete, and on
//! a musl target it resolves against the musl `std` has already linked, so it
//! still needs no C cross-toolchain. `libc` is pure declarations with no C
//! compiled by its build script.
//!
//! The dependency is walled per `ADR-009` §3.4: no `libc` type appears outside
//! [`kernel`], and none reaches `rivet-core`.

mod boot;
mod breakglass;
mod breakglass_board;
mod carried;
#[cfg(test)]
#[path = "carried_tests.rs"]
mod carried_tests;
mod confirm;
mod fleet_root;
mod identity;
#[cfg(test)]
#[path = "identity_tests.rs"]
mod identity_tests;
mod kernel;
mod pid1;
mod provision;
mod reap;
mod seed;
mod slots;
mod splash;
mod staging;
#[cfg(test)]
#[path = "staging_tests.rs"]
mod staging_tests;
mod state;
mod supervise;
#[cfg(test)]
mod testkernel;
mod unbooted;

use rivet_adapter_image::{BootSlots, PiFirmware};

use kernel::{Kernel, LibcKernel};
use supervise::{SUPERVISOR_PATH, Supervisor};

fn main() -> ! {
    let mut k = LibcKernel::new();
    // The two adapters, both built here and nowhere else: the syscall seam and
    // the A/B boot primitive. `AGENTS.md` §3 — the composition root wires, it
    // does not decide.
    run(&mut k, Box::new(PiFirmware))
}

/// The whole of PID 1's boot, over any [`Kernel`].
///
/// Split from `main` only so the phase order is one readable thing; there is no
/// logic here beyond the order itself.
fn run(k: &mut dyn Kernel, slots: Box<dyn BootSlots>) -> ! {
    let pid = k.self_pid();
    k.say("");
    k.say(&format!("rivet-initd: starting as pid {pid}"));
    // Which A/B primitive this build is wired to. `ADR-026` §4 puts `tryboot`
    // in the column QEMU cannot reach, so on a board this line is the only
    // evidence that the adapter the card needs is the one that was compiled in.
    k.say(&format!("rivet-initd: boot slots via {}", slots.platform()));
    if pid != 1 {
        // Not fatal — it is useful to run this under a shell while developing —
        // but it must be said, because none of the reasoning below applies.
        // Most visibly: orphans reparent to the system's init, not to us.
        k.say("rivet-initd: WARNING not pid 1; not acting as init");
    }

    // Item 1, and the verdict it produces (boot::BootHealth).
    let health = boot::perform_stage0_mounts(k);

    // Show that /proc is a live kernel filesystem and not an empty directory
    // we happened to create. AGENTS.md §5 — report what the system said.
    boot::report_proc_evidence(k);
    boot::report_cgroup_evidence(k);
    boot::report_mount_table(k);

    // Item 1's second half, added 2026-08-23 with the amendment to ADR-017
    // §3.1.5 item 1. It comes after the pseudo-filesystems because it converges
    // by reading /proc, and before everything below because /state is where the
    // journal, the configuration and the home directories live — and because
    // ADR-018 §8's recovery path, when it is written, is `read_provisioning`
    // deciding what to do about an unreadable /state/config.
    let health = state::bring_up_state(k, health);

    // Item 2. Needs /run, which the mount phase above has just provided.
    let provisioning = provision::read_provisioning(k);

    // Item 2's second half, added 2026-08-23 with ADR-017 §3.1.9: ADR-004 §6
    // step 2, the document written to the persistent configuration. It comes
    // after `read_provisioning` because that is where the document is, and
    // after `bring_up_state` because /state is where it goes. A fault here is
    // reported and never changes what PID 1 will certify — see `seed`.
    let _ = seed::seed_configuration(k, &provisioning);

    // Item 2's third part — ADR-004 §6 step 3, outstanding since 2026-08-23 and
    // implementable since ADR-020 §9.2 ruled where a device TLS seed rests.
    // AFTER `seed_configuration` because both want the boot partition and
    // neither may hold it while the other mounts it; after `bring_up_state`
    // because /state/secrets is where the identity goes.
    let _ = identity::provision_identity(k);

    // Item 2's fourth part — `ADR-049`, the material an APPLICATION on this
    // machine is handed before it first starts. AFTER `provision_identity` for
    // the reason that one comes after `seed_configuration`: both want the boot
    // partition writable and neither may hold it while the other mounts it.
    //
    // A fault here is reported and changes nothing else about the boot. Nothing
    // in `/state/staging` is delivered to anybody by this process; the
    // supervisor does that, with the uid it derives, and a machine whose
    // staging never arrived is a machine an operator can still reach and repair.
    let _ = staging::provision_staging(k);

    // Item 2's fifth part — `ADR-050` §4, the APPLICATIONS this card carries.
    // AFTER `provision_staging` for that one's reason: three couriers want the
    // boot partition and none may hold it while another mounts it.
    //
    // **After staging rather than before it, and the order is deliberate.**
    // `ADR-049`'s material is delivered by the SUPERVISOR at the discovery pass
    // that first sees an application installed, so the staging has to be on
    // `/state` before an install can finish — and the install cannot even start
    // until this process has exited and the supervisor is converging. Putting
    // the payloads first would not break it; putting the staging first makes
    // the ordering true by construction rather than by timing.
    //
    // A fault here is reported and changes nothing else about the boot: a
    // machine whose payloads never arrived is a machine an operator can still
    // reach, because `rivet-mcpd` is in the image (`ADR-050` §4).
    let _ = carried::provision_carried(k);

    // Arm the signals *before* anything is spawned, so no child exit and no
    // shutdown request can be missed between the fork and the wait.
    let mode = pid1::choose_mode(k);

    // **The boot splash, and only on an ordinary boot.**
    //
    // Started HERE, after everything above has had its say, because pid 1
    // writes to the console and would paint over the picture it had just drawn.
    // Started as a child rather than drawn in-process for `ADR-035` §8A.1.2's
    // reason: a megabyte of artwork and a scaler do not belong in the process
    // whose failure mode is the machine.
    //
    // **Not on a trial boot.** That is the maintainer's requirement of
    // 2026-08-30 and the whole reason this is conditional: a trial is the boot
    // somebody is WATCHING, and covering the console with a logo during the one
    // boot whose errors matter most would be exactly backwards.
    //
    // Deliberately NOT keyed off the kernel command line. `ADR-018` §5.3
    // records that whether the firmware reads `config.txt` or `tryboot.txt`
    // depends on state `cmd/rivet-image` cannot see, so a quiet/verbose split
    // written into those files would have failed silently in the trial case.
    // The slot primitive is the authority and pid 1 already holds it.
    splash::maybe_draw(k, slots.as_ref());

    // Stage 0's exit criterion needs a child that exits and an orphan that
    // reparents. PLAN.md §2.1. Not part of the job list.
    let selftest = reap::spawn_selftest(k);

    // Items 3, 4, 5, 6 and 7 are all the same loop.
    pid1::Pid1::new(
        mode,
        health,
        provisioning,
        Supervisor::new(SUPERVISOR_PATH),
        selftest,
        slots,
    )
    .run_forever(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::BootHealth;
    use crate::kernel::Errno;
    use crate::provision::Provisioning;
    use crate::testkernel::{FakeKernel, Script};

    /// The composition root's phase order, checked against `run`'s own source.
    ///
    /// **Why this is not a normal test.** `run` is `-> !`; it cannot be called
    /// and returned from, so no test can drive it and every test below drives
    /// the phases by hand *in an order the test itself chooses*. That means a
    /// defect in `run` — a phase reordered, or one deleted outright — passes
    /// every one of them. Injecting exactly that (removing the
    /// `report_cgroup_evidence` call) confirmed it: nothing failed.
    ///
    /// `AGENTS.md` §4's answer to a suite that cannot reach something is a
    /// cheap guard that fails on every cycle rather than an acknowledgement in
    /// prose, so this reads the text of `run` and asserts the order it is
    /// written in. It proves the source says the right thing, which is strictly
    /// weaker than proving the program does it — but it is not nothing, and it
    /// is the only thing available for a function of this type.
    #[test]
    fn the_phases_appear_in_run_in_the_order_the_reasoning_requires() {
        let source = include_str!("main.rs");
        let start = source.find("fn run(k: &mut dyn Kernel, slots: Box<dyn BootSlots>) -> ! {");
        // `rfind`: there are two `#[cfg(test)]` in this file and the first, on
        // `mod testkernel`, is *above* `run`.
        let end = source.rfind("#[cfg(test)]");
        assert!(
            start.is_some() && end.is_some(),
            "cannot locate run() in this file"
        );
        let body = match (start, end) {
            (Some(s), Some(e)) if s < e => source.get(s..e).unwrap_or(""),
            _ => "",
        };
        assert!(!body.is_empty(), "run()'s body did not slice out");

        // Every phase, in the order the reasoning requires. The mount phase
        // provides /proc and /sys/fs/cgroup, so both evidence readers follow
        // it; /state converges by reading /proc, so it follows too; /run must
        // exist before the provisioning file is read from it; the seed needs
        // both the document `read_provisioning` produced and the /state
        // `bring_up_state` mounted, so it follows both; signals must be armed
        // before anything is forked.
        let phases = [
            "boot::perform_stage0_mounts(k)",
            "boot::report_proc_evidence(k)",
            "boot::report_cgroup_evidence(k)",
            "boot::report_mount_table(k)",
            "state::bring_up_state(k, health)",
            "provision::read_provisioning(k)",
            "seed::seed_configuration(k, &provisioning)",
            "pid1::choose_mode(k)",
            "reap::spawn_selftest(k)",
        ];
        let mut previous = 0usize;
        for phase in phases {
            let at = body.find(phase);
            assert!(at.is_some(), "run() no longer calls {phase}");
            let at = at.unwrap_or(0);
            assert!(at >= previous, "run() calls {phase} out of order");
            previous = at;
        }
    }

    /// The phase order is load-bearing: signals must be armed before anything
    /// is forked, or a child that exits immediately can be missed entirely, and
    /// a `SIGTERM` arriving during the boot would kill PID 1 outright rather
    /// than being queued.
    ///
    /// `run` never returns, so this drives the phases individually in the same
    /// order `run` does and asserts on the trace. It is a weaker test than
    /// driving `run` itself, and it is the strongest one available for a
    /// function whose type is `-> !`.
    #[test]
    fn signals_are_armed_before_any_child_is_forked() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );

        let _ = boot::perform_stage0_mounts(&mut k);
        boot::report_proc_evidence(&mut k);
        boot::report_cgroup_evidence(&mut k);
        boot::report_mount_table(&mut k);
        let provisioning = provision::read_provisioning(&mut k);
        let _ = seed::seed_configuration(&mut k, &provisioning);
        let armed_before = k.arm_calls;
        let _ = pid1::choose_mode(&mut k);
        let _ = reap::spawn_selftest(&mut k);

        assert_eq!(
            armed_before, 0,
            "nothing before choose_mode may arm signals"
        );
        assert_eq!(k.arm_calls, 1);
        assert!(
            k.spawn_calls.is_empty(),
            "the supervisor starts in the loop, not here"
        );
    }

    /// A boot in which nothing works must still reach the loop. Every phase
    /// below is fed a refusal; none of them may propagate it.
    #[test]
    fn every_phase_survives_a_kernel_that_refuses_everything() {
        let mut k = FakeKernel::new();
        k.mkdir_results = Script::always(Err(Errno::EROFS));
        k.mount_results = Script::always(Err(Errno::EPERM));
        k.arm_result = Err(Errno::EPERM);
        k.fork_results = Script::always(Err(Errno::EPERM));
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.wait_results = Script::always(Err(Errno::ECHILD));

        let health = boot::perform_stage0_mounts(&mut k);
        boot::report_proc_evidence(&mut k);
        boot::report_cgroup_evidence(&mut k);
        boot::report_mount_table(&mut k);
        let provisioning = provision::read_provisioning(&mut k);
        let seeding = seed::seed_configuration(&mut k, &provisioning);
        let mode = pid1::choose_mode(&mut k);
        let selftest = reap::spawn_selftest(&mut k);
        let mut loop_state = pid1::Pid1::new(
            mode,
            health.clone(),
            provisioning,
            Supervisor::new(SUPERVISOR_PATH),
            selftest,
            Box::new(PiFirmware),
        );
        loop_state.tick(&mut k);

        assert!(matches!(health, BootHealth::Unconfirmable { .. }));
        assert!(!health.may_confirm_boot());
        // Nothing was offered, so there was nothing to seed. The point of
        // asserting it is that the phase ran and returned rather than
        // propagating anything — the same standing rule as every other phase.
        assert_eq!(seeding, rivet_store::Seeding::NothingOffered);
        assert_eq!(mode, pid1::WakeMode::DegradedPoll);
        assert_eq!(loop_state.selftest, None);
        // And it is still going: the tick slept rather than returning. It slept
        // for the supervisor's first backoff rather than the full poll
        // interval, because the backoff is sooner — which is the scheduling
        // working, not a stray number.
        assert_eq!(k.slept_ms, vec![supervise::FIRST_BACKOFF_MS]);
        assert!(k.slept_ms.iter().all(|ms| *ms <= pid1::POLL_INTERVAL_MS));
    }

    #[test]
    fn a_first_boot_that_works_ends_healthy_with_the_signal_loop() {
        let table = "proc /proc proc rw 0 0\ndevtmpfs /dev devtmpfs rw 0 0\nsysfs /sys sysfs rw 0 0\n\
             cgroup2 /sys/fs/cgroup cgroup2 rw 0 0\n\
             devpts /dev/pts devpts rw 0 0\ntmpfs /dev/shm tmpfs rw 0 0\n\
             tmpfs /run tmpfs rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n";
        let plan_len = rivet_core::mount::stage0_plan().len();
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            // Nothing is mounted until this loop mounts it, so the table is
            // unreadable for every spec — one refusal each — and only then
            // reads back complete.
            Script::sequence(vec![Err(Errno::ENOENT); plan_len], Ok(table.to_string())),
        );

        let health = boot::perform_stage0_mounts(&mut k);
        boot::report_mount_table(&mut k);
        let mode = pid1::choose_mode(&mut k);

        assert_eq!(health, BootHealth::Healthy);
        assert_eq!(
            k.mount_calls.len(),
            plan_len,
            "a first boot mounts every spec"
        );
        assert_eq!(mode, pid1::WakeMode::Signals);
        assert_eq!(
            k.lines.iter().filter(|l| l.contains("present   ")).count(),
            plan_len
        );
    }

    /// Why `run` reads the controller set *after* the mount phase and not
    /// before: the file only exists once cgroup2 is mounted, so reading it
    /// first reports UNKNOWN on every boot and looks like a kernel with no
    /// cgroups at all.
    ///
    /// **This tests the behaviour, not `run`'s ordering.** `run` is `-> !` and
    /// cannot be driven, so the same limitation applies here as to
    /// `signals_are_armed_before_any_child_is_forked` above: the order in
    /// `run` itself is checked by reading it.
    #[test]
    fn the_controller_set_is_unreadable_until_cgroup2_is_mounted() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        // Absent until the mount phase runs, then present. That is exactly what
        // the fake's default (ENOENT for an unconfigured path) plus a scripted
        // answer expresses.
        boot::report_cgroup_evidence(&mut k);
        assert!(
            k.lines.iter().any(|l| l.contains("UNKNOWN")),
            "before the mount there is nothing to read: {:?}",
            k.lines
        );

        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        k.files_mut().insert(
            "/sys/fs/cgroup/cgroup.controllers".to_string(),
            Script::always(Ok("cpuset cpu io memory pids\n".to_string())),
        );
        let health = boot::perform_stage0_mounts(&mut k);
        boot::report_cgroup_evidence(&mut k);
        assert_eq!(health, BootHealth::Healthy);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("cpuset cpu io memory pids")),
            "{:?}",
            k.lines
        );
        assert!(!k.lines.iter().any(|l| l.contains("UNKNOWN")));
    }

    /// The emulator's shape, end to end: no boot partition, no supervisor
    /// binary, no device tree. It must reach the loop, reap, and never claim to
    /// have confirmed anything.
    #[test]
    fn the_emulators_world_boots_reaps_and_confirms_nothing() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.dirs_mut()
            .insert("/proc".to_string(), Script::always(Ok(vec!["1".into()])));
        k.files_mut().insert(
            "/proc/1/stat".into(),
            Script::always(Ok("1 (init) R 0".into())),
        );

        let health = boot::perform_stage0_mounts(&mut k);
        let provisioning = provision::read_provisioning(&mut k);
        let mode = pid1::choose_mode(&mut k);
        let selftest = reap::spawn_selftest(&mut k);
        let mut loop_state = pid1::Pid1::new(
            mode,
            health,
            provisioning.clone(),
            Supervisor::new(SUPERVISOR_PATH),
            selftest,
            Box::new(PiFirmware),
        );

        // The three self-test children, then nothing but backoff.
        k.wait_results = Script::sequence(
            vec![
                Ok(crate::kernel::Reaped::Child {
                    pid: 53,
                    status: crate::kernel::ExitStatus::Exited(7),
                }),
                Ok(crate::kernel::Reaped::Child {
                    pid: 54,
                    status: crate::kernel::ExitStatus::Exited(0),
                }),
                Ok(crate::kernel::Reaped::Child {
                    pid: 60,
                    status: crate::kernel::ExitStatus::Exited(9),
                }),
            ],
            Err(Errno::ECHILD),
        );
        loop_state.tick(&mut k);

        assert!(matches!(provisioning, Provisioning::Absent { .. }));
        // Both halves of the census, spelled out: `0 in state Z` alone still
        // matched after ADR-017 §3.1.11 split the count, which would have let
        // this assertion pass on a board with someone else's zombies on it.
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("0 in state Z parented to pid 1, 0 to someone else")),
            "{:?}",
            k.lines
        );
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("Stage 0 reap criterion met"))
        );
        assert!(
            !k.lines.iter().any(|l| l.contains("boot confirmed")),
            "nothing may claim a confirmation without a tryboot flag"
        );
    }
}
