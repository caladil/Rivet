//! The mount phase, and the verdict it produces.
//!
//! Everything here is a decision, so everything here is above the
//! [`Kernel`] seam and drivable by a fake.

use rivet_core::mount::{CGROUP2_MOUNT, MountFlag, MountSpec, stage0_plan};
use rivet_core::state::STATE_MOUNT_POINT;

use crate::kernel::{Errno, Kernel, MountFlags};

// ---------------------------------------------------------------------------
// What "fatal" means for a process that cannot exit
// ---------------------------------------------------------------------------

/// The filesystems PID 1 cannot do **its own** job list without.
///
/// Ruled here, 2026-08-22, because "report it and carry on" was the behaviour
/// by default rather than by decision, and `AGENTS.md` §1 says a choice that
/// shapes the software gets written down.
///
/// **The rule: essential means PID 1 itself is blind or broken without it.
/// Everything else is the supervisor's precondition, and the supervisor is
/// allowed to fail** (`ADR-017` §3.1). By that rule the list is `/proc` alone:
///
/// * PID 1 reads `/proc/self/mounts` to converge its own mounts. Without it,
///   [`perform_mount`] cannot tell mounted from unmounted, and a re-run
///   *stacks* tmpfs mounts rather than converging — tmpfs mints a new
///   superblock per mount, so the kernel does not refuse the second one.
/// * PID 1 reads `/proc/<pid>/stat` to audit for zombies (`PLAN.md` §2.1).
/// * PID 1 reads `/proc/device-tree/chosen/bootloader/tryboot` to confirm the
///   boot (`ADR-018` §5.3, confirmed on hardware 2026-08-22).
///
/// `/sys`, `/sys/fs/cgroup`, `/dev/pts`, `/dev/shm` and `/run` are read by
/// PID 1 nowhere. Neither is `/state`, which PID 1 began mounting on
/// 2026-08-23 (`rivet_core::state`, `ADR-017` §3.1.5 item 1 as amended) —
/// **and its exclusion here is a positive ruling rather than an inherited
/// default.** `ADR-018` §8: a damaged `/state` survives an A/B rollback
/// untouched, so declaring it essential would roll the image back to a
/// previous one that finds the same damage, for ever, without addressing it.
/// A machine that boots and can be reached is one that can be repaired.
///
/// They are needed by the supervisor and by services — the
/// cgroup2 hierarchy (`ADR-017` §3.1.3), pre-created sockets under `/run`
/// (§3.2) — and a supervisor that cannot start is already handled by §3.1.4
/// rather than by PID 1 treating the mount as fatal.
///
/// **cgroup2 was added to the plan on 2026-08-22 and this list was not
/// changed**, deliberately. It is the sharpest test of the rule so far: a
/// missing `/sys/fs/cgroup` is genuinely dangerous — [`consequence_of`]
/// spells out that a restarted supervisor may double-start a live service —
/// but it is dangerous to the *supervisor*, and §3.1.6 puts the supervisor's
/// preconditions outside PID 1's jurisdiction on purpose. Widening this list
/// is the maintainer's call and belongs in `ADR-017` §3.1.6's table first.
pub const ESSENTIAL_TO_PID1: &[&str] = &["/proc"];

pub fn is_essential(target: &str) -> bool {
    ESSENTIAL_TO_PID1.contains(&target)
}

/// What the mount phase leaves the system in.
///
/// **PID 1 cannot exit, so "fatal" cannot mean exit.** `ADR-017` §3.1.4 already
/// names what it means for this project: PID 1 is what marks a boot successful,
/// and a boot it never confirms falls back to the previous image on the next
/// reboot, automatically, with no emergency shell required (`ADR-005` §5.4
/// removed that deliberately). So the escalation ladder is:
///
/// | | Meaning |
/// | :-- | :-- |
/// | [`Healthy`](Self::Healthy) | every filesystem is present |
/// | [`Degraded`](Self::Degraded) | a non-essential filesystem is missing. Say so loudly, keep going, still confirm the boot — a machine that reaps and can be reached is worth more than one that rolls back |
/// | [`Unconfirmable`](Self::Unconfirmable) | `/proc` is missing. Never confirm the boot; the A/B scheme reverts to the last image that worked |
///
/// In all three PID 1 mounts what it can, reaps for ever, and never returns.
/// The difference is what it is willing to certify.
///
/// **This type is the verdict, not the act.** Performing the confirmation is
/// `ADR-018` §5's job and is not written yet; this is the value it will read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootHealth {
    Healthy,
    Degraded { missing: Vec<String> },
    Unconfirmable { missing: Vec<String> },
}

impl BootHealth {
    /// Whether the boot may be marked good once the system is otherwise
    /// healthy (`ADR-017` §3.1.5 item 5).
    pub fn may_confirm_boot(&self) -> bool {
        !matches!(self, Self::Unconfirmable { .. })
    }

    /// Fold the whole plan's outcomes into one verdict.
    ///
    /// A missing *essential* filesystem wins over any number of missing
    /// non-essential ones, and `missing` always lists everything that failed —
    /// truncating it to the essential one would hide the rest from the console,
    /// which is the only diagnostic channel this system has.
    pub fn from_outcomes(outcomes: &[(&str, MountOutcome)]) -> Self {
        let mut missing = Vec::new();
        let mut essential_missing = false;
        for (target, outcome) in outcomes {
            if matches!(outcome, MountOutcome::Refused { .. }) {
                missing.push((*target).to_string());
                if is_essential(target) {
                    essential_missing = true;
                }
            }
        }
        if missing.is_empty() {
            Self::Healthy
        } else if essential_missing {
            Self::Unconfirmable { missing }
        } else {
            Self::Degraded { missing }
        }
    }

    /// Fold one more outcome into an existing verdict.
    ///
    /// `/state` is not in `stage0_plan` — it is a real block device with failure
    /// modes a tmpfs cannot have (`crate::state`) — so its outcome arrives after
    /// [`from_outcomes`](Self::from_outcomes) has already run. This is how it
    /// joins, and it goes through [`is_essential`] rather than deciding for
    /// itself, so that *what counts as essential* stays in one place.
    ///
    /// The escalation is monotone: a verdict never improves. An
    /// [`Unconfirmable`](Self::Unconfirmable) boot with a working `/state` is
    /// still unconfirmable.
    pub fn with(self, target: &str, outcome: &MountOutcome) -> Self {
        if !matches!(outcome, MountOutcome::Refused { .. }) {
            return self;
        }
        let (mut missing, was_unconfirmable) = match self {
            Self::Healthy => (Vec::new(), false),
            Self::Degraded { missing } => (missing, false),
            Self::Unconfirmable { missing } => (missing, true),
        };
        missing.push(target.to_string());
        if was_unconfirmable || is_essential(target) {
            Self::Unconfirmable { missing }
        } else {
            Self::Degraded { missing }
        }
    }
}

// ---------------------------------------------------------------------------
// Performing one spec
// ---------------------------------------------------------------------------

/// Which step of performing a [`MountSpec`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `rivet-core` refused the spec itself.
    Validate,
    /// The mount point could not be created.
    MakeDir,
    /// `mount(2)` refused.
    Mount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountOutcome {
    /// The kernel mounted it on this call.
    Mounted,
    /// It was already there. `ADR-015` §7 — re-running converges, it does not
    /// fail.
    Converged,
    /// Reported and survived. Never fatal to the process; see [`BootHealth`]
    /// for what it is fatal *to*.
    Refused {
        step: Step,
        errno: Option<Errno>,
        detail: String,
    },
}

/// Does the kernel's mount table show something mounted at `target`?
///
/// Field 2 of a mount-table line is the mount point. Exact field comparison,
/// deliberately: `/run` must not match `/run/lock`, and `/proc` must not match
/// `/proc/sys/fs/binfmt_misc`, both of which appear in a real table.
///
/// The kernel escapes a space in a mount point as `\040`. No stage 0 target
/// contains one, so this comparison is exact for this plan; a target that did
/// would fail to match, attempt the mount, and converge on `EBUSY` — it fails
/// closed rather than wrong.
pub fn mount_table_has(table: &str, target: &str) -> bool {
    mount_table_fstype(table, target).is_some()
}

/// What the kernel says is mounted at `target` — its word, not the plan's.
///
/// Field 3 of a mount-table line is the filesystem type. It is read rather
/// than assumed because "something is mounted there" is a weaker fact than the
/// one that matters: `ADR-024` §8.1's whole finding is that a path can look
/// right and the thing behind it be wrong. A `tmpfs` at `/sys/fs/cgroup`, or a
/// legacy `cgroup` v1 hierarchy with no controllers attached, satisfies a
/// path-only check and holds nothing `ADR-017` §3.1.3 can read.
///
/// The **last** matching line wins, because a stacked mount shadows the ones
/// beneath it and the top one is what a path lookup actually reaches.
pub fn mount_table_fstype<'t>(table: &'t str, target: &str) -> Option<&'t str> {
    table
        .lines()
        .rfind(|line| line.split(' ').nth(1) == Some(target))
        .and_then(|line| line.split(' ').nth(2))
}

/// Classify what `mkdir_p` said.
///
/// `EROFS` is called out because it is a real deployment state rather than a
/// hypothetical: `ADR-018` §3 makes `/system` an `erofs` image, read-only by
/// format and not by a flag anyone can flip.
fn mkdir_detail(target: &str, errno: Errno) -> String {
    match errno {
        Errno::EROFS => format!(
            "cannot create {target}: {errno} — the filesystem is read-only by format (ADR-018 §3)"
        ),
        Errno::EEXIST | Errno::ENOTDIR => {
            format!("cannot create {target}: {errno} — the path exists and is not a directory")
        }
        _ => format!("cannot create {target}: {errno}"),
    }
}

/// Classify what `mount(2)` said.
///
/// The distinction that matters: `EBUSY` is the kernel saying "that superblock
/// is already mounted here", which is the answer we wanted. Every other errno
/// is a refusal. Treating them alike would turn a genuinely unmounted `/proc`
/// into a silent success.
fn mount_detail(target: &str, errno: Errno) -> String {
    match errno {
        Errno::EACCES | Errno::EPERM => format!(
            "mount({target}): {errno} — no permission; PID 1 should have CAP_SYS_ADMIN, so this is a namespace or LSM problem, not a plan problem"
        ),
        Errno::ENOENT => format!("mount({target}): {errno} — the mount point does not exist"),
        Errno::ENODEV => format!(
            "mount({target}): {errno} — the kernel has no such filesystem type; check the kernel configuration (ADR-024)"
        ),
        _ => format!("mount({target}): {errno}"),
    }
}

/// Perform one [`MountSpec`]. Never fails the boot; the caller folds the
/// outcome into a [`BootHealth`].
pub fn perform_mount(k: &mut dyn Kernel, spec: &MountSpec) -> MountOutcome {
    if let Err(e) = spec.validate() {
        return MountOutcome::Refused {
            step: Step::Validate,
            errno: None,
            detail: format!("{e}"),
        };
    }

    // The mount points do not exist. Under QEMU the initramfs holds only /init
    // and /dev/console (ADR-026 §3); on the board it is whatever the image put
    // there.
    if let Err(errno) = k.mkdir_p(spec.target) {
        return MountOutcome::Refused {
            step: Step::MakeDir,
            errno: Some(errno),
            detail: mkdir_detail(spec.target, errno),
        };
    }

    // Idempotence, by asking the kernel rather than by swallowing an error.
    //
    // A second mount(2) of proc on /proc gives EBUSY, but a second tmpfs on
    // /run does *not*: tmpfs mints a new superblock each time, so the kernel
    // happily stacks it and the mount table grows on every re-run. Asking first
    // is what makes this convergent rather than merely non-fatal.
    //
    // `Err` here is NOT an error condition. It is the normal state of a first
    // boot — /proc/self/mounts cannot be read until /proc is mounted, and
    // mounting /proc is the first thing this loop does. So it falls through to
    // the mount and lets EBUSY be the backstop, and reports nothing.
    if let Ok(table) = k.read_mounts()
        && mount_table_has(&table, spec.target)
    {
        return MountOutcome::Converged;
    }

    match k.mount(
        spec.source,
        spec.target,
        spec.fstype,
        MountFlags::from_core(spec.flags),
        spec.data,
    ) {
        Ok(()) => MountOutcome::Mounted,
        Err(Errno::EBUSY) => MountOutcome::Converged,
        Err(errno) => MountOutcome::Refused {
            step: Step::Mount,
            errno: Some(errno),
            detail: mount_detail(spec.target, errno),
        },
    }
}

/// The name `mount(8)` would print for a flag.
///
/// One match rather than one per caller: `crate::state` renders `/state`'s
/// options too, and a second copy is how `noatime` ends up printed as `nodev`
/// on exactly the console somebody is using to diagnose a card.
pub fn flag_name(flag: MountFlag) -> &'static str {
    match flag {
        MountFlag::NoExec => "noexec",
        MountFlag::NoSuid => "nosuid",
        MountFlag::NoDev => "nodev",
        MountFlag::NoAtime => "noatime",
    }
}

/// Render options the way `mount(8)` would, for the console.
pub fn render_options(flags: &[MountFlag], data: Option<&str>) -> String {
    let names: Vec<&str> = flags.iter().copied().map(flag_name).collect();
    match data {
        Some(d) => format!("{},{d}", names.join(",")),
        None => names.join(","),
    }
}

fn options_of(spec: &MountSpec) -> String {
    render_options(spec.flags, spec.data)
}

/// Perform the whole stage 0 plan.
///
/// **Every spec is attempted.** One refusal does not stop the rest: a failure
/// on `/dev/pts` must not cost `/run`, and PID 1 has no reason to prefer a less
/// complete system. This is the "three of five mounted, then what" answer —
/// mount the other two anyway, then decide what the result is worth.
pub fn perform_stage0_mounts(k: &mut dyn Kernel) -> BootHealth {
    k.say("rivet-initd: performing the stage 0 mount plan");
    let mut outcomes: Vec<(&str, MountOutcome)> = Vec::new();
    for spec in stage0_plan() {
        let outcome = perform_mount(k, spec);
        match &outcome {
            MountOutcome::Mounted => k.say(&format!(
                "  mounted   {:<9} {:<9} on {:<9} [{}]",
                spec.fstype,
                spec.source,
                spec.target,
                options_of(spec)
            )),
            MountOutcome::Converged => k.say(&format!(
                "  already   {:<9} {:<9} on {:<9} (converged, not an error)",
                spec.fstype, spec.source, spec.target
            )),
            MountOutcome::Refused { detail, .. } => {
                k.say(&format!("  REFUSED   {detail}"));
            }
        }
        outcomes.push((spec.target, outcome));
    }
    let health = BootHealth::from_outcomes(&outcomes);
    report_health(k, &health);
    health
}

/// What a specific missing filesystem costs, when it is worth saying.
///
/// "Non-essential to pid 1" is a statement about PID 1's jurisdiction
/// (`ADR-017` §3.1.6), not a statement that nothing is lost. For most of the
/// plan the loss is obvious from the name. For cgroup2 it is not, and it is
/// severe, so it is spelled out: the console is the only diagnostic channel
/// this system has, and an operator who reads `DEGRADED /sys/fs/cgroup` has no
/// way to know it means services may be started twice.
///
/// Kept as a lookup rather than a paragraph appended to the `Degraded` arm,
/// because the entries accumulate: cgroup2 was the first and `/state` the
/// second (2026-08-23). `/dev/pts` is the one already predicted — the day
/// anything on this machine opens an interactive session, a missing pty is a
/// console that cannot be used and a diagnosis that names none of this
/// (§3.1.6).
fn consequence_of(target: &str) -> Option<&'static str> {
    match target {
        STATE_MOUNT_POINT => Some(
            "             /state is the ONLY writable filesystem on this machine (ADR-018 §4). \
             \
             Without it there is no journal (ADR-022), no /state/config (ADR-008 §3.1), no \
             \
             home directories (ADR-008 §7.3), no /state/secrets and therefore no device \
             \
             identity for the control plane to answer with (ADR-020 §5), and no system \
             \
             extensions (ADR-018 §6). The machine runs, keeps nothing, and forgets this on \
             \
             the next reboot. An A/B rollback will NOT fix it — an update never touches \
             \
             /state — so the boot is still confirmed and the card wants inspecting.",
        ),
        CGROUP2_MOUNT => Some(
            "             /sys/fs/cgroup is the authority for what is running (ADR-017 §3.1.3). \
             \
             Without it rivet-sysd cannot observe an already-running service, so a \
             \
             restarted supervisor may start a SECOND copy of one that is still up, and no \
             \
             resource limit can be applied (ADR-015 §5.1).",
        ),
        _ => None,
    }
}

/// Say what the verdict is, in words an operator reading a console can act on.
pub fn report_health(k: &mut dyn Kernel, health: &BootHealth) {
    match health {
        BootHealth::Healthy => {
            k.say("rivet-initd: mount phase healthy; the boot may be confirmed");
        }
        BootHealth::Degraded { missing } => {
            k.say(&format!(
                "rivet-initd: DEGRADED {} not mounted, none of them essential to pid 1;",
                missing.join(", ")
            ));
            k.say("             continuing, and the boot may still be confirmed (ADR-017 §3.1.4)");
            report_consequences(k, missing);
        }
        BootHealth::Unconfirmable { missing } => {
            k.say(&format!(
                "rivet-initd: UNCONFIRMABLE {} not mounted, and /proc is essential to pid 1;",
                missing.join(", ")
            ));
            k.say("             pid 1 is blind: it cannot converge mounts, audit for zombies,");
            k.say("             or read the tryboot flag. This boot will NOT be confirmed, so");
            k.say(
                "             the next reboot falls back to the previous image (ADR-017 §3.1.4).",
            );
            k.say("             Reaping continues regardless; pid 1 never exits.");
            report_consequences(k, missing);
        }
    }
}

fn report_consequences(k: &mut dyn Kernel, missing: &[String]) {
    for target in missing {
        if let Some(text) = consequence_of(target) {
            k.say(text);
        }
    }
}

// ---------------------------------------------------------------------------
// Evidence, read from the kernel rather than asserted
// ---------------------------------------------------------------------------

/// Print something the kernel wrote, to show `/proc` is live.
///
/// `AGENTS.md` §5 — report what the system said, not what it was expected to
/// say. A `mount(2)` returning 0 is our claim; `/proc/version` is the kernel's.
/// The paths [`report_proc_evidence`] reads, named so a test can count them
/// rather than restate the number.
pub const PROC_EVIDENCE: &[&str] = &[
    "/proc/version",
    "/proc/cmdline",
    "/proc/uptime",
    "/proc/self/stat",
];

pub fn report_proc_evidence(k: &mut dyn Kernel) {
    k.say("rivet-initd: reading /proc to prove it is live, not an empty directory");
    // `/proc/cmdline` is what the kernel ACTUALLY received, which is not the
    // file Rivet wrote: the Pi firmware composes its own parameters and
    // appends `cmdline.txt` to them. Reasoning from the written file instead of
    // this one is what produced, and then hid, `ADR-024` §8.1's wrong diagnosis
    // of the missing memory controller. 240 chars — this line is long and the
    // interesting parameters are not always at the front.
    for path in PROC_EVIDENCE {
        match k.read_file(path) {
            Ok(text) => {
                let line = text.lines().next().unwrap_or("").trim();
                // **1024, raised from 240 on 2026-08-23 by the first boot that
                // used it.** The whole point of printing this file is that the
                // kernel's command line is not the file Rivet writes — the Pi
                // firmware composes its own parameters and appends `cmdline.txt`
                // to them (`ADR-024` §8.1). So **Rivet's half is at the end**,
                // and 240 characters did not reach it: the board's line spent
                // its whole budget on `coherent_pool`, `snd_bcm2835.*`,
                // `bcm2708_fb.*` and `numa=fake=1` before `console=` began.
                //
                // A cap that truncates exactly the half this line exists to show
                // is worse than no cap, because it looks like evidence. The
                // remaining cap is only a guard against a pathological file.
                let limit = if *path == "/proc/cmdline" { 1024 } else { 120 };
                let shown: String = line.chars().take(limit).collect();
                k.say(&format!("  {path}: {shown}"));
            }
            Err(e) => k.say(&format!("  {path}: unreadable: {e}")),
        }
    }
}

/// The controller `ADR-015` §5.1 names first and `ADR-024` §8.1 found missing.
const MEMORY_CONTROLLER: &str = "memory";

/// Read back what the cgroup2 hierarchy says it can actually do.
///
/// The same discipline as [`report_proc_evidence`], for the same reason and
/// with a sharper edge. `ADR-024` §8.1 found that stock Raspberry Pi OS ships
/// `cgroup_disable=memory` in `cmdline.txt`: `/sys/fs/cgroup` is a real
/// `cgroup2fs` and `cgroup.controllers` reads `cpuset cpu io pids`, with **no
/// `memory`**. A memory limit set against that kernel is accepted and ignored.
/// So a successful `mount(2)` is not the claim worth making — the claim worth
/// making is the list the kernel publishes, printed verbatim.
///
/// **This is evidence, not verification.** `ADR-024` §7's boot-time check of
/// the whole requirements table is unwritten; this reads one file and says what
/// it contained. It changes no verdict — a missing `memory` controller is a
/// fact about `cmdline.txt`, which Rivet owns (`ADR-002` §4), and is fixed
/// there rather than worked around here.
pub fn report_cgroup_evidence(k: &mut dyn Kernel) {
    let path = format!("{CGROUP2_MOUNT}/cgroup.controllers");
    k.say("rivet-initd: reading the cgroup2 controllers the kernel actually offers");
    match k.read_file(&path) {
        Ok(text) => {
            let line = text.lines().next().unwrap_or("").trim();
            k.say(&format!("  {path}: {line}"));
            let controllers: Vec<&str> = line.split_whitespace().collect();
            if controllers.is_empty() {
                k.say("  no controllers at all — the file is empty, which is not a kernel");
                k.say("  Rivet can apply any resource limit on (ADR-015 §5.1)");
            } else if !controllers.contains(&MEMORY_CONTROLLER) {
                k.say("  NO 'memory' CONTROLLER. ADR-015 §5.1 names memory limits first, and a");
                k.say("  memory limit set against this kernel is accepted and silently ignored.");
                k.say("  ADR-024 §8.1: the Pi FIRMWARE composes a command line carrying");
                k.say("  cgroup_disable=memory and appends cmdline.txt to it, so owning");
                k.say("  cmdline.txt cannot delete it — cgroup_enable=memory counters it,");
                k.say("  and rivet-image writes that. Seeing this line means it did not");
                k.say("  take: read /proc/cmdline, printed above, for what the kernel got.");
            }
        }
        // Not "unavailable, therefore fine". `AGENTS.md` §5 — report what the
        // system said, and say what it could not say.
        Err(e) => {
            k.say(&format!("  {path}: unreadable: {e}"));
            k.say("  so the controller set is UNKNOWN — not empty, and not assumed present");
        }
    }
}

/// Confirm every stage 0 target against the kernel's own mount table.
///
/// The type printed is the kernel's, taken from the table, not the spec's —
/// which is the only way this is evidence rather than a restatement of what
/// was asked for. A type that does not match is reported as a MISMATCH and not
/// as a success.
pub fn report_mount_table(k: &mut dyn Kernel) {
    k.say("rivet-initd: confirming the plan against the kernel's mount table");
    for spec in stage0_plan() {
        match k.read_mounts() {
            Ok(table) => match mount_table_fstype(&table, spec.target) {
                Some(fstype) if fstype == spec.fstype => {
                    k.say(&format!("  present   {:<14} as {fstype}", spec.target));
                }
                Some(fstype) => k.say(&format!(
                    "  MISMATCH  {:<14} is mounted, but as {fstype}, not {}",
                    spec.target, spec.fstype
                )),
                None => k.say(&format!(
                    "  MISSING   {} — not in the kernel's mount table",
                    spec.target
                )),
            },
            Err(e) => k.say(&format!(
                "  UNKNOWN   {} — cannot read the mount table: {e}",
                spec.target
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// The zombie audit
// ---------------------------------------------------------------------------

/// One process's `/proc/<pid>/stat`, parsed as far as PID 1 has any use for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStat {
    /// `comm`, without the parentheses the kernel wraps it in.
    pub comm: String,
    /// The single state letter — `R`, `S`, `Z`, and so on.
    pub state: String,
    /// The parent's pid, or `None` when the field is missing or does not parse.
    ///
    /// `None` is deliberately not folded into "not ours": a zombie whose parent
    /// cannot be read is a zombie nobody has been shown to own, and
    /// [`Census`] counts it against the reap loop rather than excusing it.
    pub ppid: Option<i32>,
}

/// Pull the state letter, command name and parent out of one `/proc/<pid>/stat`.
///
/// `comm` may itself contain spaces and parentheses — a process can be named
/// `(evil) Z ` — so the fields after it are taken from after the **last** `)`.
/// That is the only parse of this file that is correct, and getting it wrong
/// would report a zombie where there is none, or worse, miss one.
///
/// After that `)` the kernel's field order is `state ppid pgrp …`, so the two
/// this function wants are the first two whitespace-separated tokens.
pub fn parse_stat(stat: &str) -> Option<ProcStat> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = stat.get(open + 1..close)?.to_string();
    let mut rest = stat.get(close + 1..)?.split_whitespace();
    let state = rest.next()?.to_string();
    let ppid = rest.next().and_then(|f| f.parse::<i32>().ok());
    Some(ProcStat { comm, state, ppid })
}

/// The state of one pid, or `Ok(None)` if the kernel no longer has it.
pub fn proc_state(k: &mut dyn Kernel, pid: i32) -> Result<Option<ProcStat>, Errno> {
    match k.read_file(&format!("/proc/{pid}/stat")) {
        Ok(text) => Ok(parse_stat(&text)),
        // Gone is the answer we want after a reap, not a failure to get one.
        Err(Errno::ENOENT) => Ok(None),
        Err(e) => Err(e),
    }
}

/// What one sweep of `/proc` found, with the zombies split by who owns them.
///
/// **The split is the whole point of this type**, and it exists because the
/// undivided count cried wolf on roughly half of all boots (`PLAN.md` §0.0).
/// PID 1's reap loop is answerable for the processes whose parent is PID 1 —
/// its own children, plus every orphan the kernel reparents to it. A zombie
/// belonging to some *other* parent is that parent's `waitpid` to call, and on
/// this system the supervisor has one: `rivet-netd` runs once, exits 0, and
/// is a zombie for the moment between its exit and the supervisor's reap.
/// Counting it as PID 1's failure trains a reader to ignore the one line that
/// would matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Census {
    /// Every numeric entry `/proc` listed.
    pub processes: usize,
    /// Zombies parented to PID 1, or whose parent could not be read. **These
    /// are the reap criterion**; anything here is a defect in the reap loop.
    pub unreaped: Vec<String>,
    /// Zombies with a live parent that is not PID 1. Reported, never counted
    /// against the criterion — they are someone else's to wait on.
    pub elsewhere: Vec<String>,
}

/// Count every process the kernel knows about, and every zombie among them.
pub fn zombie_census(k: &mut dyn Kernel) -> Result<Census, Errno> {
    let entries = k.read_dir("/proc")?;
    let mut census = Census {
        processes: 0,
        unreaped: Vec::new(),
        elsewhere: Vec::new(),
    };
    for name in entries {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        census.processes += 1;
        let Ok(text) = k.read_file(&format!("/proc/{name}/stat")) else {
            // A process that exited between the readdir and the read is
            // ordinary, and is emphatically not a zombie.
            continue;
        };
        let Some(stat) = parse_stat(&text) else {
            continue;
        };
        if stat.state != "Z" {
            continue;
        }
        match stat.ppid {
            Some(1) => census
                .unreaped
                .push(format!("pid {name} ({}) state Z, parent 1", stat.comm)),
            Some(ppid) => census
                .elsewhere
                .push(format!("pid {name} ({}) state Z, parent {ppid}", stat.comm)),
            None => census.unreaped.push(format!(
                "pid {name} ({}) state Z, parent UNREADABLE",
                stat.comm
            )),
        }
    }
    Ok(census)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    const TABLE: &str = "proc /proc proc rw,nosuid,nodev,noexec 0 0\n\
         devtmpfs /dev devtmpfs rw,nosuid,noexec,mode=755 0 0\n\
         sysfs /sys sysfs rw,nosuid,nodev,noexec 0 0\n\
         cgroup2 /sys/fs/cgroup cgroup2 rw,nosuid,nodev,noexec 0 0\n\
         devpts /dev/pts devpts rw,nosuid,noexec 0 0\n\
         tmpfs /dev/shm tmpfs rw,nosuid,nodev 0 0\n\
         tmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n\
         tmpfs /tmp tmpfs rw,noexec,nosuid,nodev,size=64m,mode=1777 0 0\n";

    /// How many filesystems the plan carries. Derived rather than written, so
    /// that adding one to the plan does not need this file edited in six
    /// places — and so that a count assertion below cannot silently drift away
    /// from what `stage0_plan()` actually contains.
    /// A mount script in which every spec succeeds except `target`.
    ///
    /// **Derived from the plan rather than written as a position**, which is
    /// the same reason [`plan_len`] exists. Two tests used to say
    /// `sequence(vec![Ok(()), Err(..)], ..)` and mean "`/sys` refuses"; when
    /// `/dev` entered the plan at position 2 on 2026-08-24 they silently began
    /// refusing a different filesystem and failed naming one nobody had
    /// written. Asking the plan where the target is cannot drift that way.
    fn refuses_only(target: &str) -> Script<Result<(), Errno>> {
        let at = stage0_plan().iter().position(|s| s.target == target);
        assert!(
            at.is_some(),
            "{target} is not in stage0_plan(), so it cannot refuse"
        );
        let at = at.unwrap_or(0);
        let mut steps = vec![Ok(()); at];
        steps.push(Err(Errno::ENODEV));
        Script::sequence(steps, Ok(()))
    }

    fn plan_len() -> usize {
        stage0_plan().len()
    }

    fn spec_for(target: &str) -> &'static MountSpec {
        // `find` rather than an index, so a renamed target fails the assertion
        // instead of silently testing whichever spec happened to be first.
        static FALLBACK: MountSpec = MountSpec {
            source: "?",
            target: "/no-such-stage-0-target",
            fstype: "?",
            flags: &[],
            data: None,
        };
        let found = stage0_plan().iter().find(|s| s.target == target);
        assert!(found.is_some(), "no stage 0 spec for {target}");
        found.unwrap_or(&FALLBACK)
    }

    // -- the mount table parse ------------------------------------------------

    #[test]
    fn mount_table_matches_a_mount_point_exactly() {
        assert!(mount_table_has(TABLE, "/run"));
        assert!(mount_table_has(TABLE, "/dev/shm"));
    }

    #[test]
    fn mount_table_does_not_match_a_longer_path_that_starts_the_same() {
        // A real table carries /proc/sys/fs/binfmt_misc and /run/lock. Matching
        // by prefix would report /run mounted when only /run/lock is.
        let table =
            "tmpfs /run/lock tmpfs rw 0 0\nnone /proc/sys/fs/binfmt_misc binfmt_misc rw 0 0\n";
        assert!(!mount_table_has(table, "/run"));
        assert!(!mount_table_has(table, "/proc"));
    }

    #[test]
    fn mount_table_does_not_match_the_source_or_the_fstype_by_accident() {
        // Field 1 is the source and field 3 the type; only field 2 is the
        // mount point. "proc" as a source must not satisfy a query for "proc".
        assert!(!mount_table_has(TABLE, "proc"));
        assert!(!mount_table_has(TABLE, "tmpfs"));
    }

    #[test]
    fn an_empty_table_matches_nothing() {
        assert!(!mount_table_has("", "/proc"));
    }

    #[test]
    fn a_stacked_mount_still_reads_as_present() {
        let stacked = "tmpfs /run tmpfs rw 0 0\ntmpfs /run tmpfs rw 0 0\n";
        assert!(mount_table_has(stacked, "/run"));
    }

    // -- the first boot -------------------------------------------------------

    /// The boundary condition that looks like an error path and is not: on
    /// every first boot `/proc/self/mounts` cannot be read, because `/proc` is
    /// what we are about to mount.
    #[test]
    fn an_unreadable_mount_table_is_a_first_boot_not_a_failure() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );

        let outcome = perform_mount(&mut k, spec_for("/proc"));

        assert_eq!(outcome, MountOutcome::Mounted);
        assert_eq!(k.mount_calls.len(), 1, "it must fall through to mount(2)");
        assert!(
            !k.lines
                .iter()
                .any(|l| l.contains("REFUSED") || l.contains("ENOENT")),
            "an unreadable mount table must not be reported as an error: {:?}",
            k.lines
        );
    }

    /// The coordinator's sequence: read_mounts fails, mount succeeds, and the
    /// *second* read_mounts — now that /proc is up — succeeds and converges.
    #[test]
    fn the_table_becomes_readable_once_proc_is_mounted_and_the_rest_converges() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            // First call: no /proc yet. Every call after: the full table.
            Script::sequence(vec![Err(Errno::ENOENT)], Ok(TABLE.to_string())),
        );

        let health = perform_stage0_mounts(&mut k);

        assert_eq!(health, BootHealth::Healthy);
        // Only /proc was actually mounted; the rest were already there
        // according to the table, so mount(2) was never called for them.
        assert_eq!(k.mount_calls.len(), 1);
        assert!(k.mount_calls.iter().any(|c| c.target == "/proc"));
        let converged = k.lines.iter().filter(|l| l.contains("converged")).count();
        assert_eq!(converged, plan_len() - 1);
    }

    #[test]
    fn a_second_run_stacks_nothing() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(TABLE.to_string())),
        );

        let health = perform_stage0_mounts(&mut k);

        assert_eq!(health, BootHealth::Healthy);
        assert!(
            k.mount_calls.is_empty(),
            "everything was already mounted; mount(2) must not be called at all"
        );
    }

    // -- errno classification -------------------------------------------------

    #[test]
    fn ebusy_converges_because_the_kernel_means_already_mounted_here() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        k.mount_results = Script::always(Err(Errno::EBUSY));

        assert_eq!(
            perform_mount(&mut k, spec_for("/proc")),
            MountOutcome::Converged
        );
    }

    #[test]
    fn every_other_mount_errno_is_a_refusal_and_says_which() {
        for errno in [Errno::EACCES, Errno::ENOENT, Errno::ENODEV, Errno::EPERM] {
            let mut k = FakeKernel::new();
            k.files_mut().insert(
                crate::kernel::MOUNTS_PATH.to_string(),
                Script::always(Err(Errno::ENOENT)),
            );
            k.mount_results = Script::always(Err(errno));

            let outcome = perform_mount(&mut k, spec_for("/proc"));
            assert_eq!(
                outcome,
                MountOutcome::Refused {
                    step: Step::Mount,
                    errno: Some(errno),
                    detail: mount_detail("/proc", errno),
                }
            );
            // And the console line carries the number, so an operator reading
            // it can look the failure up.
            assert!(mount_detail("/proc", errno).contains(&errno.0.to_string()));
        }
    }

    #[test]
    fn enodev_names_the_kernel_configuration_because_that_is_the_actual_cause() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        k.mount_results = Script::always(Err(Errno::ENODEV));
        let outcome = perform_mount(&mut k, spec_for("/sys"));
        assert!(
            matches!(&outcome, MountOutcome::Refused { detail, .. } if detail.contains("kernel configuration"))
        );
    }

    // -- mkdir boundaries -----------------------------------------------------

    /// `ADR-018` §3 makes `/system` an erofs image — read-only by format, not
    /// by a flag. A read-only filesystem under a mount point is a deployment
    /// state, not a hypothetical.
    #[test]
    fn a_read_only_filesystem_refuses_at_the_mkdir_step_and_never_reaches_mount() {
        let mut k = FakeKernel::new();
        k.mkdir_results = Script::always(Err(Errno::EROFS));

        let outcome = perform_mount(&mut k, spec_for("/run"));

        assert_eq!(
            outcome,
            MountOutcome::Refused {
                step: Step::MakeDir,
                errno: Some(Errno::EROFS),
                detail: mkdir_detail("/run", Errno::EROFS),
            }
        );
        assert!(mkdir_detail("/run", Errno::EROFS).contains("read-only by format"));
        assert!(
            k.mount_calls.is_empty(),
            "a mount point that does not exist must not be mounted onto"
        );
    }

    #[test]
    fn a_mount_target_that_exists_as_a_file_refuses_at_the_mkdir_step() {
        for errno in [Errno::EEXIST, Errno::ENOTDIR] {
            let mut k = FakeKernel::new();
            k.mkdir_results = Script::always(Err(errno));
            let outcome = perform_mount(&mut k, spec_for("/dev/pts"));
            assert_eq!(
                outcome,
                MountOutcome::Refused {
                    step: Step::MakeDir,
                    errno: Some(errno),
                    detail: mkdir_detail("/dev/pts", errno),
                }
            );
            assert!(mkdir_detail("/dev/pts", errno).contains("not a directory"));
            assert!(k.mount_calls.is_empty());
        }
    }

    // -- partial success, and what it is worth --------------------------------

    #[test]
    fn one_refusal_does_not_stop_the_remaining_specs() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        // Everything before /sys mounts, /sys refuses, then the rest must
        // still be tried.
        k.mount_results = refuses_only("/sys");

        let health = perform_stage0_mounts(&mut k);

        assert_eq!(
            k.mount_calls.len(),
            plan_len(),
            "every spec must be attempted"
        );
        assert_eq!(
            health,
            BootHealth::Degraded {
                missing: vec!["/sys".to_string()]
            }
        );
    }

    /// The specific regression this whole change exists for.
    ///
    /// The supervisor's re-adoption (`ADR-017` §3.1.2–§3.1.3) was demonstrated
    /// under WSL2, where something else had already mounted cgroup2. On a real
    /// image nothing had, so `observe()` answered `Unavailable` and the
    /// supervisor started a second copy. If cgroup2 leaves the plan, this fails.
    #[test]
    fn a_first_boot_mounts_cgroup2_at_the_path_the_supervisor_reads() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );

        let health = perform_stage0_mounts(&mut k);

        assert_eq!(health, BootHealth::Healthy);
        let cg = k.mount_calls.iter().find(|c| c.target == CGROUP2_MOUNT);
        assert!(
            cg.is_some(),
            "cgroup2 never reached mount(2): {:?}",
            k.mount_calls
        );
        assert!(
            cg.is_some_and(|c| c.fstype == "cgroup2"
                && c.source == "cgroup2"
                && c.data.is_none()
                && c.flags.contains(MountFlags::NOEXEC)
                && c.flags.contains(MountFlags::NOSUID)
                && c.flags.contains(MountFlags::NODEV)),
            "cgroup2 reached the kernel wrong: {cg:?}"
        );
        // `rivet-sysd` reads /sys/fs/cgroup/rivet/<service>, so the
        // mount point it hangs off must be exactly this.
        assert_eq!(CGROUP2_MOUNT, "/sys/fs/cgroup");
    }

    /// Ordering, at the layer that performs it rather than the layer that
    /// declares it. Mounting sysfs after cgroup2 would leave the mount table
    /// looking correct and every lookup under `/sys/fs/cgroup` landing in
    /// sysfs.
    #[test]
    fn sysfs_is_mounted_before_the_cgroup2_hierarchy_inside_it() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );

        let _ = perform_stage0_mounts(&mut k);

        let sys = k.mount_calls.iter().position(|c| c.target == "/sys");
        let cg = k.mount_calls.iter().position(|c| c.target == CGROUP2_MOUNT);
        assert!(sys.is_some() && cg.is_some(), "{:?}", k.mount_calls);
        assert!(sys < cg, "sysfs must reach mount(2) before cgroup2 does");
        // And the same for mkdir: the mount point lives inside sysfs, which
        // the kernel creates. Creating it before sysfs is mounted makes a
        // directory on the root filesystem that sysfs then covers.
        let sys_dir = k.mkdir_calls.iter().position(|p| p == "/sys");
        let cg_dir = k.mkdir_calls.iter().position(|p| p == CGROUP2_MOUNT);
        assert!(sys_dir < cg_dir, "{:?}", k.mkdir_calls);
    }

    /// A missing cgroup2 is not fatal to PID 1 and is severe for everything
    /// else, so the console must say what it costs rather than only naming it.
    #[test]
    fn a_missing_cgroup2_is_degraded_but_the_console_names_the_double_start() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        // Everything before cgroup2 mounts; cgroup2 refuses; the rest mount.
        k.mount_results = refuses_only(CGROUP2_MOUNT);

        let health = perform_stage0_mounts(&mut k);

        assert_eq!(
            health,
            BootHealth::Degraded {
                missing: vec![CGROUP2_MOUNT.to_string()]
            }
        );
        assert!(
            health.may_confirm_boot(),
            "ADR-017 §3.1.6 — not PID 1's to adjudicate"
        );
        let joined = k.lines.join("\n");
        assert!(joined.contains("SECOND copy"), "{joined}");
        assert!(joined.contains("ADR-017 §3.1.3"), "{joined}");
    }

    #[test]
    fn a_missing_filesystem_with_no_recorded_consequence_says_nothing_extra() {
        let mut k = FakeKernel::new();
        report_health(
            &mut k,
            &BootHealth::Degraded {
                missing: vec!["/dev/shm".into()],
            },
        );
        assert!(
            !k.lines.iter().any(|l| l.contains("SECOND copy")),
            "the cgroup2 consequence must not be printed for an unrelated mount"
        );
        assert!(consequence_of("/dev/shm").is_none());
        assert!(consequence_of(CGROUP2_MOUNT).is_some());
    }

    #[test]
    fn a_partial_plan_leaves_a_degraded_boot_that_may_still_be_confirmed() {
        let outcomes = [
            ("/proc", MountOutcome::Mounted),
            ("/sys", MountOutcome::Mounted),
            (CGROUP2_MOUNT, MountOutcome::Mounted),
            ("/dev/pts", MountOutcome::Mounted),
            ("/dev/shm", refusal()),
            ("/run", refusal()),
        ];
        let health = BootHealth::from_outcomes(&outcomes);
        assert_eq!(
            health,
            BootHealth::Degraded {
                missing: vec!["/dev/shm".to_string(), "/run".to_string()]
            }
        );
        assert!(health.may_confirm_boot());
    }

    #[test]
    fn losing_proc_makes_the_boot_unconfirmable_however_much_else_worked() {
        let outcomes = [
            ("/proc", refusal()),
            ("/sys", MountOutcome::Mounted),
            (CGROUP2_MOUNT, MountOutcome::Mounted),
            ("/dev/pts", MountOutcome::Mounted),
            ("/dev/shm", MountOutcome::Mounted),
            ("/run", MountOutcome::Mounted),
        ];
        let health = BootHealth::from_outcomes(&outcomes);
        assert_eq!(
            health,
            BootHealth::Unconfirmable {
                missing: vec!["/proc".to_string()]
            }
        );
        assert!(!health.may_confirm_boot());
    }

    #[test]
    fn an_essential_failure_outranks_non_essential_ones_but_still_lists_them_all() {
        let outcomes = [
            ("/proc", refusal()),
            ("/sys", refusal()),
            (CGROUP2_MOUNT, refusal()),
            ("/dev/pts", MountOutcome::Mounted),
            ("/dev/shm", MountOutcome::Mounted),
            ("/run", refusal()),
        ];
        assert_eq!(
            BootHealth::from_outcomes(&outcomes),
            BootHealth::Unconfirmable {
                missing: vec![
                    "/proc".to_string(),
                    "/sys".to_string(),
                    CGROUP2_MOUNT.to_string(),
                    "/run".to_string(),
                ],
            }
        );
    }

    #[test]
    fn everything_mounted_is_healthy_and_converged_counts_as_mounted() {
        let outcomes = [
            ("/proc", MountOutcome::Converged),
            ("/sys", MountOutcome::Mounted),
            (CGROUP2_MOUNT, MountOutcome::Converged),
            ("/dev/pts", MountOutcome::Converged),
            ("/dev/shm", MountOutcome::Mounted),
            ("/run", MountOutcome::Converged),
        ];
        assert_eq!(BootHealth::from_outcomes(&outcomes), BootHealth::Healthy);
    }

    /// The essential set is `ADR-017` §3.1.6's table and belongs to the
    /// maintainer. cgroup2 joining the plan did **not** join this list — see
    /// [`ESSENTIAL_TO_PID1`]'s doc comment for the argument. This test is what
    /// makes widening it a deliberate act.
    #[test]
    fn only_proc_is_essential_to_pid_one() {
        assert!(is_essential("/proc"));
        assert_eq!(ESSENTIAL_TO_PID1, ["/proc"]);
        for other in ["/sys", CGROUP2_MOUNT, "/dev/pts", "/dev/shm", "/run"] {
            assert!(!is_essential(other), "{other} is not read by pid 1 itself");
        }
    }

    #[test]
    fn an_unconfirmable_boot_says_so_on_the_console_in_words_and_says_it_keeps_reaping() {
        let mut k = FakeKernel::new();
        report_health(
            &mut k,
            &BootHealth::Unconfirmable {
                missing: vec!["/proc".into()],
            },
        );
        let joined = k.lines.join("\n");
        assert!(joined.contains("UNCONFIRMABLE"), "{joined}");
        assert!(joined.contains("NOT be confirmed"), "{joined}");
        assert!(joined.contains("never exits"), "{joined}");
    }

    fn refusal() -> MountOutcome {
        MountOutcome::Refused {
            step: Step::Mount,
            errno: Some(Errno::ENODEV),
            detail: String::new(),
        }
    }

    // -- validation -----------------------------------------------------------

    #[test]
    fn a_relative_target_refuses_before_any_syscall_is_made() {
        let bad = MountSpec {
            source: "proc",
            target: "proc",
            fstype: "proc",
            flags: &[],
            data: None,
        };
        let mut k = FakeKernel::new();
        let outcome = perform_mount(&mut k, &bad);
        assert!(matches!(
            outcome,
            MountOutcome::Refused {
                step: Step::Validate,
                errno: None,
                ..
            }
        ));
        assert!(k.mkdir_calls.is_empty());
        assert!(k.mount_calls.is_empty());
    }

    // -- what actually reaches mount(2) --------------------------------------

    #[test]
    fn the_spec_reaches_the_kernel_intact_including_flags_and_data() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );

        let _ = perform_mount(&mut k, spec_for("/dev/pts"));

        let call = k.mount_calls.first();
        assert!(call.is_some_and(|c| c.source == "devpts"
            && c.target == "/dev/pts"
            && c.fstype == "devpts"
            && c.data.as_deref() == Some("gid=5,mode=620")
            && c.flags.contains(MountFlags::NOEXEC)
            && c.flags.contains(MountFlags::NOSUID)
            && !c.flags.contains(MountFlags::NODEV)));
    }

    // -- /proc/<pid>/stat parsing --------------------------------------------

    #[test]
    fn stat_state_is_read_after_the_last_paren_not_the_first() {
        // A process may legally be named ") Z (". Parsing to the first ')'
        // would read the state as part of the name and report a zombie that is
        // not there — or miss one that is.
        let stat = "42 (weird) Z (name) R 1 42 42 0 -1 4194304 87 0";
        assert_eq!(
            parse_stat(stat),
            Some(ProcStat {
                comm: "weird) Z (name".to_string(),
                state: "R".to_string(),
                ppid: Some(1),
            })
        );
    }

    #[test]
    fn a_plain_zombie_parses_as_one() {
        assert_eq!(
            parse_stat("53 (init) Z 1 1 0 0 -1"),
            Some(ProcStat {
                comm: "init".to_string(),
                state: "Z".to_string(),
                ppid: Some(1),
            })
        );
    }

    #[test]
    fn the_ppid_is_the_field_after_the_state_not_the_pid_in_front_of_comm() {
        // The leading 103 is the pid; the parent is the 87 after the state.
        // Reading the wrong one would attribute every process to itself.
        assert_eq!(
            parse_stat("103 (rivet-netd) Z 87 87 0 0 -1"),
            Some(ProcStat {
                comm: "rivet-netd".to_string(),
                state: "Z".to_string(),
                ppid: Some(87),
            })
        );
    }

    #[test]
    fn a_stat_line_with_no_parens_parses_to_nothing_rather_than_guessing() {
        assert_eq!(parse_stat("53 init Z 1"), None);
        assert_eq!(parse_stat(""), None);
        assert_eq!(parse_stat("53 ) init ( Z"), None);
    }

    #[test]
    fn a_stat_line_that_ends_at_the_paren_has_no_state() {
        assert_eq!(parse_stat("53 (init)"), None);
    }

    #[test]
    fn an_unparseable_ppid_is_none_rather_than_a_guessed_parent() {
        let unknown = Some(ProcStat {
            comm: "init".to_string(),
            state: "Z".to_string(),
            ppid: None,
        });
        assert_eq!(parse_stat("53 (init) Z ?"), unknown);
        // And a missing one, which is the truncated-read case.
        assert_eq!(parse_stat("53 (init) Z"), unknown);
    }

    // -- the zombie audit -----------------------------------------------------

    #[test]
    fn the_census_counts_processes_and_finds_a_zombie() {
        let mut k = FakeKernel::new();
        k.dirs_mut().insert(
            "/proc".to_string(),
            Script::always(Ok(vec![
                "1".into(),
                "53".into(),
                "self".into(),
                "meminfo".into(),
            ])),
        );
        k.files_mut().insert(
            "/proc/1/stat".into(),
            Script::always(Ok("1 (init) R 0".into())),
        );
        k.files_mut().insert(
            "/proc/53/stat".into(),
            Script::always(Ok("53 (child) Z 1".into())),
        );

        // Only numeric entries are processes: `self` and `meminfo` are not.
        assert_eq!(
            zombie_census(&mut k),
            Ok(Census {
                processes: 2,
                unreaped: vec!["pid 53 (child) state Z, parent 1".to_string()],
                elsewhere: Vec::new(),
            })
        );
    }

    #[test]
    fn a_zombie_belonging_to_another_parent_is_not_the_reap_loops() {
        // The board case, 2026-08-23: `rivet-netd` is the supervisor's child,
        // exits 0 once, and is a zombie until the supervisor waits on it. Two
        // boots of the same image disagreed because the audit raced that exit.
        // Provenance: pid and comm are the ones the console actually printed;
        // the parent pid is composed, since the capture did not carry it.
        let mut k = FakeKernel::new();
        k.dirs_mut().insert(
            "/proc".to_string(),
            Script::always(Ok(vec!["1".into(), "87".into(), "103".into()])),
        );
        k.files_mut().insert(
            "/proc/1/stat".into(),
            Script::always(Ok("1 (rivet-initd) S 0".into())),
        );
        k.files_mut().insert(
            "/proc/87/stat".into(),
            Script::always(Ok("87 (rivet-sysd) S 1".into())),
        );
        k.files_mut().insert(
            "/proc/103/stat".into(),
            Script::always(Ok("103 (rivet-netd) Z 87".into())),
        );

        // Nothing lands in `unreaped`: that zombie is the supervisor's to wait
        // on, and pid 1's reap loop was never wrong about it.
        assert_eq!(
            zombie_census(&mut k),
            Ok(Census {
                processes: 3,
                unreaped: Vec::new(),
                elsewhere: vec!["pid 103 (rivet-netd) state Z, parent 87".to_string()],
            })
        );
    }

    #[test]
    fn a_zombie_with_an_unreadable_parent_counts_against_the_reap_loop() {
        // Unknown ownership must not read as an acquittal — the whole value of
        // the split is that it never excuses a zombie it cannot attribute.
        let mut k = FakeKernel::new();
        k.dirs_mut()
            .insert("/proc".to_string(), Script::always(Ok(vec!["53".into()])));
        k.files_mut().insert(
            "/proc/53/stat".into(),
            Script::always(Ok("53 (child) Z".into())),
        );

        assert_eq!(
            zombie_census(&mut k),
            Ok(Census {
                processes: 1,
                unreaped: vec!["pid 53 (child) state Z, parent UNREADABLE".to_string()],
                elsewhere: Vec::new(),
            })
        );
    }

    #[test]
    fn a_process_that_exits_between_readdir_and_read_is_not_counted_as_a_zombie() {
        let mut k = FakeKernel::new();
        k.dirs_mut().insert(
            "/proc".to_string(),
            Script::always(Ok(vec!["1".into(), "99".into()])),
        );
        k.files_mut().insert(
            "/proc/1/stat".into(),
            Script::always(Ok("1 (init) R 0".into())),
        );
        // /proc/99/stat is absent from the fake, so it reads ENOENT. It is
        // still counted as a process — the kernel listed it — but it is not a
        // zombie, and inferring one from an unreadable stat would be a guess.
        assert_eq!(
            zombie_census(&mut k),
            Ok(Census {
                processes: 2,
                unreaped: Vec::new(),
                elsewhere: Vec::new(),
            })
        );
    }

    #[test]
    fn the_census_reports_the_errno_when_proc_cannot_be_read_at_all() {
        let mut k = FakeKernel::new();
        k.dirs_mut()
            .insert("/proc".to_string(), Script::always(Err(Errno::ENOENT)));
        assert_eq!(zombie_census(&mut k), Err(Errno::ENOENT));
    }

    #[test]
    fn a_reaped_pid_reads_as_gone_rather_than_as_an_error() {
        let mut k = FakeKernel::new();
        assert_eq!(proc_state(&mut k, 53), Ok(None));
    }

    #[test]
    fn a_pid_that_is_still_a_zombie_reads_as_one() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            "/proc/53/stat".into(),
            Script::always(Ok("53 (child) Z 1".into())),
        );
        assert_eq!(
            proc_state(&mut k, 53),
            Ok(Some(ProcStat {
                comm: "child".into(),
                state: "Z".into(),
                ppid: Some(1),
            }))
        );
    }

    #[test]
    fn an_unreadable_stat_is_an_error_and_not_silently_read_as_gone() {
        let mut k = FakeKernel::new();
        k.files_mut()
            .insert("/proc/53/stat".into(), Script::always(Err(Errno::EACCES)));
        assert_eq!(proc_state(&mut k, 53), Err(Errno::EACCES));
    }

    // -- evidence -------------------------------------------------------------

    /// Every path is reported, none is skipped — and the count is derived
    /// rather than written down.
    ///
    /// **It was the literal `3` until 2026-08-23**, when `/proc/cmdline` was
    /// added and this failed with 4 against it. That failure was correct and
    /// the test was right to have it, but a hand-maintained count only ever
    /// fails *after* the change, which makes it a chore rather than a guard.
    /// What it is actually asserting is "as many reports as there are paths",
    /// so it now says that.
    #[test]
    fn unreadable_evidence_is_reported_rather_than_skipped() {
        let mut k = FakeKernel::new();
        report_proc_evidence(&mut k);
        let reported = k.lines.iter().filter(|l| l.contains("unreadable")).count();
        assert_eq!(reported, PROC_EVIDENCE.len());
        assert!(reported > 0, "no paths are read at all");
    }

    /// `/proc/cmdline` is read, and the reason is worth a test of its own.
    ///
    /// It is the only one of these paths that says what the kernel was *told*
    /// rather than what it *is*, and its absence is what let `ADR-024` §8.1's
    /// wrong diagnosis stand for a day — the command line was reasoned about
    /// from the file Rivet writes, which is not what the kernel receives.
    #[test]
    fn the_kernel_command_line_is_read_because_it_is_not_the_file_we_wrote() {
        assert!(
            PROC_EVIDENCE.contains(&"/proc/cmdline"),
            "{PROC_EVIDENCE:?}"
        );
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            "/proc/cmdline".into(),
            Script::always(Ok("cgroup_disable=memory console=tty1\n".to_string())),
        );
        report_proc_evidence(&mut k);
        assert!(
            k.lines.iter().any(|l| l.contains("cgroup_disable=memory")),
            "the command line was not printed verbatim: {:?}",
            k.lines
        );
    }

    #[test]
    fn the_mount_table_report_distinguishes_missing_from_unknown() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        report_mount_table(&mut k);
        assert_eq!(
            k.lines.iter().filter(|l| l.contains("UNKNOWN")).count(),
            plan_len()
        );
        assert!(!k.lines.iter().any(|l| l.contains("MISSING")));

        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(String::new())),
        );
        report_mount_table(&mut k);
        assert_eq!(
            k.lines.iter().filter(|l| l.contains("MISSING")).count(),
            plan_len()
        );
        assert!(!k.lines.iter().any(|l| l.contains("UNKNOWN")));
    }

    /// The kernel's own mount table must show cgroup2 by that fstype, not just
    /// something at that path. A tmpfs at `/sys/fs/cgroup` would satisfy a
    /// path-only check and hold no controllers at all.
    #[test]
    fn the_mount_table_report_confirms_the_cgroup2_target_too() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(TABLE.to_string())),
        );
        report_mount_table(&mut k);
        assert!(
            k.lines.iter().any(|l| l.contains("present")
                && l.contains(CGROUP2_MOUNT)
                && l.contains("as cgroup2")),
            "{:?}",
            k.lines
        );
        assert_eq!(mount_table_fstype(TABLE, CGROUP2_MOUNT), Some("cgroup2"));
    }

    /// The failure `ADR-024` §8.1 warns about, in the form this code could
    /// meet it: something *is* mounted at the path and it is not the unified
    /// hierarchy. A path-only check calls that success.
    #[test]
    fn a_wrong_filesystem_at_a_stage_0_path_is_a_mismatch_and_not_a_success() {
        // cgroup v1 at the same path: mounts, reports, and holds nothing
        // ADR-017 §3.1.3 can read.
        let table = "proc /proc proc rw 0 0\ndevtmpfs /dev devtmpfs rw 0 0\nsysfs /sys sysfs rw 0 0\n\
             cgroup /sys/fs/cgroup cgroup rw 0 0\ndevpts /dev/pts devpts rw 0 0\n\
             tmpfs /dev/shm tmpfs rw 0 0\ntmpfs /run tmpfs rw 0 0\n\
             tmpfs /tmp tmpfs rw 0 0\n";
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(table.to_string())),
        );
        report_mount_table(&mut k);
        let joined = k.lines.join("\n");
        assert!(joined.contains("MISMATCH"), "{joined}");
        assert!(joined.contains("as cgroup, not cgroup2"), "{joined}");
        assert_eq!(
            k.lines.iter().filter(|l| l.contains("present")).count(),
            plan_len() - 1
        );
    }

    #[test]
    fn the_fstype_comes_from_field_three_of_the_line_that_matches() {
        assert_eq!(mount_table_fstype(TABLE, "/dev/pts"), Some("devpts"));
        assert_eq!(mount_table_fstype(TABLE, "/run"), Some("tmpfs"));
        assert_eq!(mount_table_fstype(TABLE, "/nowhere"), None);
        assert_eq!(mount_table_fstype("", "/proc"), None);
    }

    /// A stacked mount shadows what is beneath it, so the *last* line is the
    /// one a path lookup actually reaches. Reading the first would report the
    /// buried filesystem as the live one.
    #[test]
    fn a_stacked_mount_reports_the_topmost_filesystem_not_the_buried_one() {
        let stacked = "tmpfs /sys/fs/cgroup tmpfs rw 0 0\ncgroup2 /sys/fs/cgroup cgroup2 rw 0 0\n";
        assert_eq!(mount_table_fstype(stacked, CGROUP2_MOUNT), Some("cgroup2"));
        let other_way =
            "cgroup2 /sys/fs/cgroup cgroup2 rw 0 0\ntmpfs /sys/fs/cgroup tmpfs rw 0 0\n";
        assert_eq!(mount_table_fstype(other_way, CGROUP2_MOUNT), Some("tmpfs"));
    }

    // -- the cgroup2 controller set ------------------------------------------

    const CONTROLLERS: &str = "/sys/fs/cgroup/cgroup.controllers";

    #[test]
    fn the_controller_set_is_printed_verbatim_rather_than_summarised() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            CONTROLLERS.into(),
            Script::always(Ok("cpuset cpu io memory pids\n".into())),
        );
        report_cgroup_evidence(&mut k);
        let joined = k.lines.join("\n");
        assert!(joined.contains("cpuset cpu io memory pids"), "{joined}");
        assert!(!joined.contains("NO 'memory' CONTROLLER"), "{joined}");
    }

    /// `ADR-024` §8.1's finding, as a test. Raspberry Pi OS ships
    /// `cgroup_disable=memory`; `/sys/fs/cgroup` is a real cgroup2fs and
    /// `memory` is simply not in the list. Inferring the controller from the
    /// filesystem type is exactly the mistake §8.1 says not to make.
    #[test]
    fn a_missing_memory_controller_is_named_and_pointed_at_cmdline_txt() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            CONTROLLERS.into(),
            Script::always(Ok("cpuset cpu io pids\n".into())),
        );
        report_cgroup_evidence(&mut k);
        let joined = k.lines.join("\n");
        assert!(joined.contains("cpuset cpu io pids"), "{joined}");
        assert!(joined.contains("NO 'memory' CONTROLLER"), "{joined}");
        assert!(joined.contains("cgroup_disable=memory"), "{joined}");
        assert!(joined.contains("cmdline.txt"), "{joined}");
    }

    /// The substring trap: `memory_recursiveprot` is not the `memory`
    /// controller, and `memoryfoo` is not either. Splitting on whitespace is
    /// what makes this right; `contains("memory")` would report a controller
    /// that is not there.
    #[test]
    fn a_controller_whose_name_merely_contains_memory_does_not_count_as_it() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            CONTROLLERS.into(),
            Script::always(Ok("cpuset cpu io memoryfoo pids\n".into())),
        );
        report_cgroup_evidence(&mut k);
        assert!(
            k.lines.iter().any(|l| l.contains("NO 'memory' CONTROLLER")),
            "{:?}",
            k.lines
        );
    }

    #[test]
    fn an_empty_controller_file_says_so_rather_than_reading_as_satisfied() {
        let mut k = FakeKernel::new();
        k.files_mut()
            .insert(CONTROLLERS.into(), Script::always(Ok("\n".into())));
        report_cgroup_evidence(&mut k);
        assert!(
            k.lines.iter().any(|l| l.contains("no controllers at all")),
            "{:?}",
            k.lines
        );
    }

    /// Absence read two ways is the point: an unreadable file is UNKNOWN, not
    /// empty and not fine (`AGENTS.md` §7).
    #[test]
    fn an_unreadable_controller_file_is_unknown_rather_than_absent_or_present() {
        let mut k = FakeKernel::new();
        report_cgroup_evidence(&mut k);
        let joined = k.lines.join("\n");
        assert!(joined.contains("unreadable"), "{joined}");
        assert!(joined.contains("UNKNOWN"), "{joined}");
        assert!(!joined.contains("NO 'memory' CONTROLLER"), "{joined}");
        assert!(!joined.contains("no controllers at all"), "{joined}");
    }

    /// The evidence must be read from under the mount point the plan actually
    /// creates. A hard-coded path here that drifted from `CGROUP2_MOUNT` would
    /// read a file that is never there and report UNKNOWN for ever.
    #[test]
    fn the_controller_file_is_read_from_under_the_mount_point_the_plan_uses() {
        assert_eq!(CONTROLLERS, format!("{CGROUP2_MOUNT}/cgroup.controllers"));
    }
}
