//! The pseudo-filesystems Rivet requires, as a plan rather than as an action.
//!
//! `PLAN.md` §2.1 lists them; `ADR-005` §2.1 says why there is no udev, which
//! is why `devtmpfs` does the work `/dev` would otherwise need.
//!
//! Until 2026-08-24 this said `DEVTMPFS_MOUNT` did that work and `/dev` was
//! therefore not in the plan. That is true on a board and false under the
//! emulator — the kernel's automount does not apply when its root is an
//! initramfs — so `/dev` is now mounted here and converges to a no-op on
//! hardware. See the entry for it in [`stage0_plan`].
//!
//! Note what is *not* here: no `libc`, no `MS_*` constants, no `nix`. Those are
//! platform types, and `ADR-014` §3 keeps them out of core. The adapter that
//! performs a [`MountSpec`] translates [`MountFlag`] into whatever the platform
//! calls it — which is also what makes this testable without root.

use crate::CoreError;

/// A property the kernel must apply to a mount.
///
/// Named for what it means rather than for its `MS_*` spelling, so the mapping
/// lives in one adapter and this stays a statement of intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MountFlag {
    /// Programs may not be executed from this filesystem.
    NoExec,
    /// Set-user-ID and set-group-ID bits are ignored.
    NoSuid,
    /// Device special files are not interpreted.
    NoDev,
    /// Reading a file does not write its access time back.
    ///
    /// Added 2026-08-23 for [`crate::state`], and it is the one flag in this
    /// enum that exists for the *hardware* rather than for safety. `ADR-018` §7
    /// puts it first among the measures that make an SD card survive: "every
    /// read otherwise becomes a write". No pseudo-filesystem in
    /// [`stage0_plan`] carries it — a tmpfs atime costs nothing, since there is
    /// no card behind it.
    NoAtime,
}

/// One pseudo-filesystem Rivet expects to exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountSpec {
    /// What to mount. For pseudo-filesystems this is conventional, not a path.
    pub source: &'static str,
    /// Where it goes. Always absolute — see [`MountSpec::validate`].
    pub target: &'static str,
    /// The filesystem type as the kernel names it.
    pub fstype: &'static str,
    /// Properties the kernel must apply.
    pub flags: &'static [MountFlag],
    /// Filesystem-specific options, or `None`.
    pub data: Option<&'static str>,
}

impl MountSpec {
    /// Check the invariants a caller would otherwise have to trust.
    ///
    /// `AGENTS.md` §3 — refuse rather than guess. A relative mount target is
    /// not a thing to fix up silently at PID 1.
    pub fn validate(&self) -> Result<(), CoreError> {
        if !self.target.starts_with('/') {
            return Err(CoreError::NotAbsolute { path: self.target });
        }
        Ok(())
    }
}

/// Where the unified cgroup hierarchy goes.
///
/// The conventional path, and the one everything else already assumes:
/// `cmd/rivet-sysd` puts each service's cgroup at
/// `/sys/fs/cgroup/rivet/<service>`, and `ADR-024` §8.1 read
/// `/sys/fs/cgroup/cgroup.controllers` on real hardware at this path.
///
/// It is a constant because two crates name it. Naming it in two string
/// literals is `ADR-011` again in miniature — one fact, one place.
/// Where the kernel publishes what is mounted.
///
/// `/proc/self/mounts` rather than `/proc/mounts`: the two are the same file on
/// a machine with one mount namespace, and the `self` form is the one that
/// stays correct if Rivet ever gains a second (`ADR-015` §5 puts application
/// sandboxing there). Naming the namespace-aware path costs nothing now and is
/// the difference between an answer and a wrong answer later.
///
/// **Moved here 2026-08-28** from `rivet_adapter_system::mounts`, which now
/// takes it from this crate, when `cmd/rivet-sysd` became a third reader
/// (`PLAN.md` §62.7). `ADR-011`: one fact, one place. `cmd/rivet-initd` still
/// has its own copy — deliberately, because it is PID 1 and does not take a
/// dependency for one string; that copy and this one are pinned against each
/// other by nothing, which is worth knowing and is the reason a fourth would be
/// too many.
pub const MOUNT_TABLE: &str = "/proc/self/mounts";

/// **The device mounted at `target`**, according to the kernel's own table.
///
/// Field 1 of a mount-table line is the source and field 2 is the mount point,
/// so this is the answer to *"what is `/state` actually on"* — which is a
/// different question from the one this crate usually answers, and the
/// difference is the point. Everything else about `/state` is composed from
/// what the image WROTE; this reports what the machine IS RUNNING.
///
/// **Added 2026-08-28 for `PLAN.md` §62.7.** `cmd/rivet-sysd`'s grow needs to
/// know which disk to rewrite a GPT on, and the only safe answer is the disk
/// the partition it is already using belongs to.
///
/// The **last** matching line wins, because a later mount at the same point
/// shadows the earlier one and the last is what a reader would reach.
///
/// The kernel escapes a space in a path as `\040`, and this does not
/// unescape. No path this system mounts contains one; a target that did would
/// fail to match and the caller would report "not mounted", which is a refusal
/// rather than a wrong answer. `cmd/rivet-initd`'s `mount_table_fstype` reads
/// field 3 of the same lines and carries the same caveat for the same reason —
/// the two are deliberately not merged, because that crate is PID 1 and does
/// not take a dependency for one line of splitting.
pub fn source_mounted_at<'t>(table: &'t str, target: &str) -> Option<&'t str> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let source = fields.next()?;
            let at = fields.next()?;
            (at == target).then_some(source)
        })
        .next_back()
}

pub const CGROUP2_MOUNT: &str = "/sys/fs/cgroup";

/// The pseudo-filesystems `cmd/rivet-initd` mounts, in the order it mounts
/// them.
///
/// **Order is load-bearing, and this is the only place it is expressed.**
/// A mount whose target lies inside another's is hidden if the outer one is
/// mounted after it:
///
/// * `/dev/pts` and `/dev/shm` are inside `/dev`, **which this plan now mounts
///   itself** — see the `devtmpfs` entry. The comment here used to say the
///   kernel had already populated it via `DEVTMPFS_MOUNT` (`ADR-024` §3) and
///   that nothing in this plan had to mount `/dev` first. That held on
///   hardware and never held under the emulator, and the ordering rule below
///   was silently satisfied by there being no `/dev` entry to order.
/// * [`CGROUP2_MOUNT`] is inside `/sys`, which **is** in this plan. Mounting
///   cgroup2 before sysfs would put it on a directory that the subsequent
///   sysfs mount then covers: the kernel's mount table would still list it,
///   and every path lookup under `/sys/fs/cgroup` would land in sysfs
///   instead. So sysfs comes first, and
///   `stage0_plan_mounts_a_parent_before_anything_inside_it` is what stops
///   that being re-ordered by someone tidying.
pub const fn stage0_plan() -> &'static [MountSpec] {
    &[
        MountSpec {
            source: "proc",
            target: "/proc",
            fstype: "proc",
            flags: &[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev],
            data: None,
        },
        // `/dev`, added 2026-08-24 on the maintainer's ruling that PID 1 mounts
        // `devtmpfs` itself, non-essential.
        //
        // **`CONFIG_DEVTMPFS_MOUNT=y` is set and it is not a lie — it simply
        // does not apply here.** `tools/buildroot/linux/rivet.config` line 26
        // sets it, and `ADR-024` §3 is right that on a board the kernel mounts
        // `/dev` before PID 1 draws breath. But that automount is skipped when
        // the kernel's root is an **initramfs**, which is exactly what
        // `tools/qemu/run.sh` boots. So the emulator has had an empty `/dev`
        // this whole time while hardware has had a full one, and every
        // difference between them traced back to this one line.
        //
        // **What that cost, so it is not paid twice.** `Stdio::null()` opens
        // `/dev/null` *in the parent, before the fork*; with no `/dev/null`
        // every spawn failed `ENOENT` — and the error named the binary, which
        // was present, rather than the device, which was not. That took netd
        // and timed down and was diagnosed twice, four hours apart,
        // the second time through `Command::output()` which nulls stdin by
        // default. `/dev/kmsg` is absent for the same reason, which is why
        // `ADR-022` §4.1's kernel ingest — the highest-value diagnostic in the
        // project — could only ever be proved against the build host.
        //
        // **Mounting it unconditionally is safe on hardware.** `perform_mount`
        // converges: it reads `/proc/self/mounts`, finds devtmpfs already at
        // `/dev`, and does nothing. This is not a second `/dev` on a board; it
        // is the emulator being made to match one.
        //
        // Non-essential, and that is the ruling rather than an oversight. PID 1
        // reads nothing under `/dev` — `ESSENTIAL_TO_PID1`'s rule is "PID 1
        // itself is blind or broken without it", and by that rule this is the
        // supervisor's precondition, not PID 1's.
        MountSpec {
            source: "devtmpfs",
            target: "/dev",
            fstype: "devtmpfs",
            // `NoDev` is conspicuously **absent, and must stay absent** —
            // it means "device special files are not interpreted", so setting
            // it on the device filesystem would leave every node present and
            // every one of them inert. `no_nodev_on_the_device_filesystem`
            // exists to stop that arriving as a tidy-up.
            flags: &[MountFlag::NoExec, MountFlag::NoSuid],
            data: Some("mode=755"),
        },
        MountSpec {
            source: "sysfs",
            target: "/sys",
            fstype: "sysfs",
            flags: &[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev],
            data: None,
        },
        // cgroup2, without which `ADR-017` §3.1.3 has no authority to read.
        //
        // The supervisor observes what is running by reading a service's
        // cgroup rather than by remembering pids (§3.1.2). If this is not
        // mounted, `observe()` answers `Unavailable`, the supervisor warns and
        // starts anyway — which is the double start that the whole mechanism
        // exists to prevent. It is also `ADR-015` §5.1's resource-limit
        // mechanism: memory, CPU and pids.
        //
        // v2 only, and `cgroup2` is the type name for the unified hierarchy.
        // Rivet has no v1 controllers and no reason to grow any.
        //
        // **No mount data, and that is a decision.** The option one reaches for
        // here is `nsdelegate`, which makes the kernel treat a cgroup-namespace
        // root as a delegation boundary. Rivet has no such boundary to draw:
        // `ADR-024` §3.1 keeps `CONFIG_USER_NS` deliberately off, and
        // `ADR-017` §3.1 has the supervisor running as root and creating
        // namespaces on applications' behalf, so nothing unprivileged is ever
        // handed a subtree to manage. `memory_recursiveprot` is the other
        // candidate and changes what `memory.low`/`memory.min` mean;
        // `ADR-015` §5.1 asks for limits, not protection, and the memory
        // controller may not even be present (`ADR-024` §8.1). Neither is
        // justified by the record, so neither is here.
        MountSpec {
            source: "cgroup2",
            target: CGROUP2_MOUNT,
            fstype: "cgroup2",
            // All three, for the same reason the other pseudo-filesystems
            // carry them: nothing under here is a program, a device node, or
            // owned by anyone who could benefit from a set-user-ID bit. It is
            // a tree of control files. `NoExec` is *not* the `/run` case —
            // nothing is ever started from `/sys/fs/cgroup`; a process is
            // written into it after it exists.
            flags: &[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev],
            data: None,
        },
        MountSpec {
            source: "devpts",
            target: "/dev/pts",
            fstype: "devpts",
            flags: &[MountFlag::NoExec, MountFlag::NoSuid],
            data: Some("gid=5,mode=620"),
        },
        MountSpec {
            source: "tmpfs",
            target: "/dev/shm",
            fstype: "tmpfs",
            flags: &[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev],
            data: None,
        },
        MountSpec {
            source: "tmpfs",
            target: "/run",
            fstype: "tmpfs",
            flags: &[MountFlag::NoSuid, MountFlag::NoDev],
            data: Some("mode=755"),
        },
        // `/tmp`, added 2026-08-23 on the maintainer's ruling.
        //
        // **Found by a survey rather than by a design pass**, which is why it
        // is worth saying what it is for. Rivet had no `/tmp` at all — not an
        // empty one, no such directory — and nothing noticed because nothing
        // Rivet ships uses it. A `nushell` feasibility build under QEMU on
        // Rivet's own kernel found `mktemp` failing with *"failed to create
        // file via template '/tmp/tmp.XXXXXXXXXX'"*, and `std::env::temp_dir()`
        // is `/tmp` for every crate in the tree, ours included.
        //
        // **`NoExec` is set, and unlike `/state` that costs nothing.**
        // `ADR-018` §4.1 deliberately leaves `/state` executable because
        // extension images loop-mount out of it and `ADR-015` gives
        // applications artifacts that have to run from somewhere. Nothing runs
        // from `/tmp`: an application's binary is its package's, verified at
        // launch (`ADR-015` §4.2). Extract-and-execute is exactly the pattern
        // this flag should stop, and no Rivet component needs it.
        //
        // **The size is a named guess, in `ADR-018` §9 item 2's tradition.** A
        // tmpfs with no `size=` takes half of RAM, and a runaway writer then
        // competes with the kernel rather than with a card — a worse failure
        // than filling `/state`. 64 MiB is chosen for a machine with no build
        // tooling and no package manager. **Revisit when something actually
        // uses it**; a limit nobody has hit is a limit nobody has measured.
        MountSpec {
            source: "tmpfs",
            target: "/tmp",
            fstype: "tmpfs",
            flags: &[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev],
            // 1777: world-writable with the sticky bit, so one uid cannot
            // remove another's files. `ADR-015` §5.1 gives every application
            // its own uid, which is what makes the sticky bit load-bearing here
            // rather than traditional.
            data: Some("size=64m,mode=1777"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- which device is mounted where -------------------------------------

    /// The board and the emulator, and the whole reason this exists: one
    /// question, two completely different answers, neither of them composable
    /// from a constant.
    #[test]
    fn the_source_at_a_mount_point_is_read_rather_than_composed() {
        let board = "/dev/mmcblk0p5 /state ext4 rw,nosuid,nodev,noatime 0 0\n";
        let emulator = "/dev/vda5 /state ext4 rw,nosuid,nodev,noatime 0 0\n";
        assert_eq!(source_mounted_at(board, "/state"), Some("/dev/mmcblk0p5"));
        assert_eq!(source_mounted_at(emulator, "/state"), Some("/dev/vda5"));
    }

    #[test]
    fn a_mount_point_that_is_not_in_the_table_is_none() {
        let table = "proc /proc proc rw 0 0\n";
        assert_eq!(source_mounted_at(table, "/state"), None);
    }

    /// Exact field comparison, not a prefix: `/state` and `/state/app` are
    /// different filesystems and `ADR-031` makes the second one real.
    #[test]
    fn a_mount_point_is_matched_whole_and_not_as_a_prefix() {
        let table = "/dev/vda6 /state/app ext4 rw 0 0\n";
        assert_eq!(source_mounted_at(table, "/state"), None);
        assert_eq!(source_mounted_at(table, "/state/app"), Some("/dev/vda6"));
    }

    /// A later mount at the same point shadows the earlier one, so the last
    /// line is the one a reader would actually reach.
    #[test]
    fn a_stacked_mount_is_reported_as_the_last_one() {
        let table = "/dev/vda5 /state ext4 rw 0 0\n/dev/vda9 /state ext4 rw 0 0\n";
        assert_eq!(source_mounted_at(table, "/state"), Some("/dev/vda9"));
    }

    /// A line that is not a mount-table line at all must not be read as one.
    #[test]
    fn a_short_or_empty_line_is_skipped_rather_than_panicking() {
        let table = "\n/dev/vda5\n   \n/dev/vda5 /state ext4 rw 0 0\n";
        assert_eq!(source_mounted_at(table, "/state"), Some("/dev/vda5"));
    }

    /// Not every source is a device, and this does not pretend otherwise — the
    /// caller is where "that is not a block device" gets decided.
    #[test]
    fn a_pseudo_filesystems_source_is_returned_verbatim() {
        let table = "tmpfs /state tmpfs rw 0 0\n";
        assert_eq!(source_mounted_at(table, "/state"), Some("tmpfs"));
    }

    #[test]
    fn stage0_plan_targets_are_absolute() {
        for spec in stage0_plan() {
            assert!(spec.validate().is_ok(), "{} is not absolute", spec.target);
        }
    }

    /// The exact contents, in the exact order.
    ///
    /// `PLAN.md` §2.1 still lists the first five; cgroup2 was added
    /// 2026-08-22 because `ADR-017` §3.1.3 makes it the authority for what is
    /// running and nothing was mounting it — see [`CGROUP2_MOUNT`]. This
    /// assertion is deliberately whole-list rather than a `contains`: it is
    /// what makes adding or re-ordering a mount a decision somebody has to
    /// take on purpose.
    #[test]
    fn stage0_plan_is_these_eight_in_this_order() {
        let targets: Vec<_> = stage0_plan().iter().map(|s| s.target).collect();
        assert_eq!(
            targets,
            [
                "/proc",
                "/dev",
                "/sys",
                "/sys/fs/cgroup",
                "/dev/pts",
                "/dev/shm",
                "/run",
                "/tmp"
            ]
        );
    }

    /// `/dev` must be in the plan, and it must come before the two mounts
    /// inside it.
    ///
    /// Added 2026-08-24. `stage0_plan_mounts_a_parent_before_anything_inside_it`
    /// already enforces the ordering as a general rule; this one asserts the
    /// *presence*, because for months the plan relied on the kernel having
    /// mounted `/dev` and the kernel only does that when its root is not an
    /// initramfs. The general rule was satisfied the whole time by `/dev`
    /// simply not being there to order.
    #[test]
    fn devtmpfs_is_in_the_plan_before_the_mounts_that_live_inside_it() {
        let plan = stage0_plan();
        let dev = plan.iter().position(|s| s.target == "/dev");
        assert!(
            dev.is_some(),
            "/dev is missing from the stage 0 plan; CONFIG_DEVTMPFS_MOUNT does \
             not apply to an initramfs, so nothing else will mount it"
        );
        assert!(
            plan.iter()
                .any(|s| s.target == "/dev" && s.fstype == "devtmpfs"),
            "/dev must be devtmpfs — a tmpfs there would be empty of device nodes"
        );
        for inner in ["/dev/pts", "/dev/shm"] {
            let at = plan.iter().position(|s| s.target == inner);
            assert!(at.is_some(), "{inner} vanished from the plan");
            assert!(
                dev < at,
                "{inner} is mounted before /dev, so devtmpfs covers it"
            );
        }
    }

    /// The one flag that must never be set here.
    ///
    /// `NoDev` reads as a safety flag and is on every other pseudo-filesystem
    /// in this plan, which is exactly what makes it a plausible tidy-up. On
    /// the device filesystem it means the kernel stops interpreting device
    /// special files: `/dev/null`, `/dev/kmsg` and `/dev/ptmx` would all still
    /// be listed by `ls` and none of them would work.
    #[test]
    fn devtmpfs_is_not_nodev_because_that_would_make_every_device_node_inert() {
        let dev = stage0_plan().iter().find(|s| s.target == "/dev");
        assert!(dev.is_some(), "/dev is missing from the stage 0 plan");
        assert!(
            !dev.is_some_and(|d| d.flags.contains(&MountFlag::NoDev)),
            "/dev carries NoDev; every device node under it would be present \
             and inert, which reads as 'mounted' and behaves as 'empty'"
        );
    }

    /// `/dev` is the supervisor's precondition, not PID 1's.
    ///
    /// Ruled non-essential 2026-08-24 (the maintainer's 2026-08-24 ruling). PID 1 reads
    /// nothing under `/dev`, so by `ESSENTIAL_TO_PID1`'s stated rule a missing
    /// `/dev` is [`BootHealth::Degraded`] rather than a boot PID 1 refuses to
    /// confirm. Asserted here so that widening it later is a decision rather
    /// than a drift — the list itself lives in `cmd/rivet-initd`, which
    /// cannot be imported from core, so this asserts the intent instead.
    #[test]
    fn devtmpfs_is_not_claimed_to_be_essential_by_this_plan() {
        // The plan expresses no essentialness of its own; the assertion that
        // matters is that nothing here marks `/dev` specially, and that the
        // flags are the ordinary ones. If `/dev` ever needs to be essential
        // that belongs in `ADR-017` §3.1.6's table first.
        let dev = stage0_plan().iter().find(|s| s.target == "/dev");
        assert!(dev.is_some_and(|d| d.data == Some("mode=755")));
        assert!(dev.is_some_and(|d| d.source == "devtmpfs"));
    }

    /// The ordering invariant, expressed as a rule rather than as a list.
    ///
    /// If one target lies inside another, the outer one must be mounted first
    /// or the inner mount is covered by it. `/sys/fs/cgroup` inside `/sys` is
    /// the case that exists today; the rule is written generally so the next
    /// nested mount is caught before it ships rather than after.
    #[test]
    fn stage0_plan_mounts_a_parent_before_anything_inside_it() {
        let plan = stage0_plan();
        for (outer_i, outer) in plan.iter().enumerate() {
            for (inner_i, inner) in plan.iter().enumerate() {
                // Component-wise containment: `/sys` contains `/sys/fs/cgroup`
                // but not `/system`, which is a different filesystem entirely
                // (`ADR-018` §3).
                let prefix = format!("{}/", outer.target);
                if outer_i != inner_i && inner.target.starts_with(&prefix) {
                    assert!(
                        outer_i < inner_i,
                        "{} is inside {}, so it must be mounted after it, not before",
                        inner.target,
                        outer.target
                    );
                }
            }
        }
    }

    #[test]
    fn no_two_specs_claim_the_same_target() {
        let mut targets: Vec<_> = stage0_plan().iter().map(|s| s.target).collect();
        let before = targets.len();
        targets.sort_unstable();
        targets.dedup();
        assert_eq!(targets.len(), before, "two specs mount onto the same path");
    }

    /// `ADR-017` §3.1.3's authority has to actually be in the plan, be the
    /// unified hierarchy, and be at the path `rivet-sysd` reads.
    #[test]
    fn cgroup2_is_in_the_plan_as_the_unified_hierarchy_at_the_conventional_path() {
        let cg = stage0_plan().iter().find(|s| s.target == CGROUP2_MOUNT);
        assert!(cg.is_some(), "cgroup2 is missing from the stage 0 plan");
        // v2, not v1: "cgroup" would mount a legacy hierarchy with no
        // controllers attached, which reads as present and does nothing.
        assert!(cg.is_some_and(|s| s.fstype == "cgroup2" && s.source == "cgroup2"));
        assert_eq!(CGROUP2_MOUNT, "/sys/fs/cgroup");
    }

    /// The `nsdelegate` decision, asserted so that adding it later has to
    /// change a test that says why it is absent.
    #[test]
    fn cgroup2_carries_no_mount_data_because_rivet_delegates_to_nobody() {
        let cg = stage0_plan().iter().find(|s| s.target == CGROUP2_MOUNT);
        assert!(
            cg.is_some_and(|s| s.data.is_none()),
            "cgroup2 has mount data; ADR-024 §3.1 (no user namespaces) and \
             ADR-017 §3.1 (a root supervisor) are why it should not"
        );
    }

    #[test]
    fn cgroup2_is_noexec_nosuid_nodev_because_it_holds_only_control_files() {
        let cg = stage0_plan().iter().find(|s| s.target == CGROUP2_MOUNT);
        for flag in [MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev] {
            assert!(
                cg.is_some_and(|s| s.flags.contains(&flag)),
                "cgroup2 is missing {flag:?}"
            );
        }
    }

    #[test]
    fn a_relative_target_is_refused_rather_than_repaired() {
        let bad = MountSpec {
            source: "proc",
            target: "proc",
            fstype: "proc",
            flags: &[],
            data: None,
        };
        assert_eq!(bad.validate(), Err(CoreError::NotAbsolute { path: "proc" }));
    }

    #[test]
    fn run_is_executable_because_services_are_started_from_it() {
        // Neither `expect` nor `panic!` here: both are warned across the
        // workspace, and a test is not exempt. The lint exists so the habit
        // never reaches PID 1, where an unwinding panic is a kernel panic.
        let plan = stage0_plan();
        let run = plan.iter().find(|s| s.target == "/run");
        assert!(run.is_some(), "/run is missing from the stage 0 plan");
        assert!(
            !run.is_some_and(|r| r.flags.contains(&MountFlag::NoExec)),
            "/run must not be noexec — services are started from it"
        );
    }
}
