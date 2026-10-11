//! `/state`: the one filesystem a Rivet machine may write to.
//!
//! `ADR-008` §3 defines it — "machine configuration, keys, persistent data",
//! mutable, and **never touched by an image update**. `ADR-018` §4 makes it
//! partition 5 and `ADR-018` §7 chooses `ext4` and the mount options. This
//! module is those decisions expressed so a program can act on them, in the same
//! shape and for the same reason as [`crate::mount`]: nothing here performs I/O,
//! so the *decisions* are testable without root, without a card, and without a
//! kernel.
//!
//! # Who mounts it — ruled 2026-08-23
//!
//! **PID 1 does**, as a non-essential entry in its mount phase. `ADR-017`
//! §3.1.5 item 1 is amended from "mount the pseudo-filesystems" to "mount the
//! boot-time filesystems", with the same date and the reasoning below.
//!
//! It was not an obvious call, and the record pointed both ways. `ADR-017` §3.1
//! is emphatic that PID 1 "does the non-negotiable work of `ADR-005` §2.2 —
//! mount the pseudo-filesystems, reap every orphan on the system, never exit"
//! and that "everything else … lives in a process that is allowed to fail".
//! `/state` is not a pseudo-filesystem, and by §3.1.6's essential-set rule PID 1
//! is not blind without it. Both of those argue for the supervisor.
//!
//! Three things argue the other way, and the first is decisive.
//!
//! 1. **The journal lives on `/state`** (`ADR-017` §7, `ADR-022`). `ADR-017` §8
//!    item 2 already answered the general form of this question — the journal is
//!    part of the supervisor rather than a service, because "a journal that were
//!    itself a service could fail to start, and the record of *why* would have
//!    nowhere to go". A supervisor that mounted its own log destination has
//!    exactly that defect one layer down: the failure to mount `/state` is the
//!    single event least able to be journalled, because journalling it is what
//!    just failed. Whatever mounts `/state` must sit **below** the journal, and
//!    PID 1 is what is below it.
//! 2. **The cost to the process that must not fail is a table row, not
//!    machinery.** PID 1 already converges mounts by reading the kernel's mount
//!    table, already treats a refusal as a verdict rather than an error, and
//!    already mounts a real block device — `provision` mounts the FAT boot
//!    partition. In the supervisor the same job costs restart-idempotency logic
//!    in the process that restarts, for a filesystem that has to be there before
//!    that process can say anything about it.
//! 3. **`ADR-017` §3.1.6 asks for exactly this re-derivation.** Its closing
//!    amendment is that "a rule derived from *what PID 1 reads today* goes stale
//!    the moment PID 1 does anything new, and that is an argument for
//!    re-deriving it at every ADR pass rather than at a named stage". This is
//!    such a pass.
//!
//! # And it is **non-essential**, which is a decision rather than a default
//!
//! `ADR-017` §3.1.6's table has three verdicts. `/state` failing to mount is
//! `Degraded` — say so loudly, carry on, **and confirm the boot anyway**.
//!
//! That is not inherited from "PID 1 does not read it". It is `ADR-018` §8:
//!
//! > A/B rollback does not help here, because `/state` is deliberately never
//! > touched by an update. A corrupt `/state` is a machine that boots into a
//! > healthy image with no configuration.
//!
//! An `Unconfirmable` verdict would roll the image back on the next reset, the
//! previous image would find the same damaged `/state`, and the machine would
//! alternate between two healthy images for ever without addressing the fault.
//! **Rolling back a fault the rollback cannot fix is worse than carrying on**,
//! and a machine that is reachable is one somebody can repair. The console says
//! plainly what has been lost — see `cmd/rivet-initd`'s `consequence_of`.
//!
//! # What is unproven
//!
//! The device path. [`state_device`] composes it from the disk name its caller
//! supplies, and `cmd/rivet-initd`'s is `/dev/mmcblk0` — inferred from
//! `ADR-002` §4's `cmdline.txt` and **confirmed on hardware 2026-08-23** for
//! `p1` only, when PID 1 mounted `/dev/mmcblk0p1` and read the provisioning
//! document through it. That the same disk carries a `p5` this image formatted
//! follows from `cmd/rivet-image` having written it, and **has not been read
//! back off a board.** A board booting from USB would present `/dev/sda` and
//! this would be wrong; see [`STATE_PARTITION`] for why it is not looked up by
//! label instead.

use crate::mount::MountFlag;

/// `/state`'s partition number. `ADR-018` §4.
///
/// **One fact, one place** (`ADR-011`): `cmd/rivet-image`'s `layout` module is
/// the authority for the partition table and now takes this number from here,
/// because `cmd/rivet-initd` needs the same number to build a device path and a
/// second literal `5` in another crate is how the two drift apart.
///
/// ~~**Why a number and not `PARTLABEL=state`.** The obvious answer is to mount by
/// label — `cmd/rivet-image` writes one, and it survives repartitioning in a
/// way a number does not. It does not work here: `/dev/disk/by-partlabel/` is
/// **udev's**, and `ADR-005` §2.1 rules that Rivet has no udev. Nothing on a
/// Rivet board creates those symlinks. Resolving a `PARTLABEL` without udev
/// means parsing the GPT out of the raw disk, which is real machinery in the
/// process that must not fail. The number is what the image just wrote.~~
///
/// **CORRECTED 2026-08-28 — `PLAN.md` §62.** Both premises are true and the
/// conclusion does not follow. `/dev/disk/by-partlabel/` is indeed udev's and
/// Rivet indeed has no udev, but there is a third resolver that is neither udev
/// nor a GPT parser: **the kernel**, which parsed the GPT itself and publishes
/// the label on each partition's sysfs `uevent`. `crate::partlabel` carries the
/// source citations. `cmd/rivet-image` had already relied on the same fact from
/// the other direction — `cmdline.txt` says `root=PARTLABEL=system-a`, resolved
/// by `block/early-lookup.c` before any userspace runs — so this file and that
/// one had named the same card two irreconcilable ways.
///
/// **The number is still here and is still the authority for the LAYOUT.** What
/// changed is how a *running machine* finds the partition: `cmd/rivet-initd`
/// looks for [`STATE_PARTLABEL`] and falls back to composing a path from this
/// number. It is the fall-back that keeps a board booting when sysfs cannot be
/// read, and `cmd/rivet-image` still writes partition five.
pub const STATE_PARTITION: u32 = 5;

/// The GPT label `cmd/rivet-image` writes on `/state`, and the name a running
/// machine finds it by. `ADR-018` §4.
///
/// **One fact, one place** (`ADR-011`), the same division [`STATE_PARTITION`]
/// already has with `cmd/rivet-image`'s `layout`: that module is the authority
/// for the partition *table* and takes this string from here, because the label
/// is now load-bearing at run time and a second literal `"state"` in another
/// crate is a machine that writes one name and looks for another.
pub const STATE_PARTLABEL: &str = "state";

/// The partition whose `autoboot.txt` the Raspberry Pi firmware actually reads.
///
/// **`ADR-018` §4's *first* boot partition, always — not the one that booted.**
/// This is a fact about the firmware, so `ADR-011` puts it in one place and has
/// `cmd/rivet-initd` read it rather than infer it.
///
/// **Proven on hardware 2026-08-23, by two boots of one card.** Both boot
/// partitions were made to disagree, and the firmware obeyed `p1` each time:
///
/// | `p1` says | `p2` says | Booted |
/// | :-- | :-- | :-- |
/// | `boot_partition=1` | `boot_partition=2` | `p1` |
/// | `boot_partition=2` | `boot_partition=1` | `p2` |
///
/// The second boot is the one that matters. It rules out *"the firmware ignores
/// `autoboot.txt` and defaults to 1"*, which the first boot alone could not:
/// `p1` asked for a partition that is not itself and got it. Raspberry Pi's
/// documentation is therefore right, confirmed on hardware 2026-08-23.
///
/// **What this cost.** `cmd/rivet-initd::confirm` rewrote the `autoboot.txt`
/// of the partition the firmware *booted*. That is the same file for slot A and
/// a file nothing reads for slot B — so a slot-B commit would have written into
/// the void, slot B could never become the default, and every A/B update would
/// have rolled back while reporting success. The failure direction was safe,
/// which is exactly why it would have run for months unnoticed.
pub const AUTOBOOT_PARTITION: u32 = 1;

/// One of `ADR-018` §4's two A/B slots.
///
/// **Why this is in a module called `state`.** It is not about `/state`, and it
/// is here for the same reason [`AUTOBOOT_PARTITION`] is: it is a fact about
/// `ADR-018` §4's partition table that more than one crate now needs, and
/// `ADR-011` says a fact lives in exactly one place. The alternative was a
/// second literal `3` in `cmd/rivet-initd`, which is precisely the failure
/// `PLAN.md` §0.0.2 item 1 is the epitaph for.
///
/// **`cmd/rivet-image/src/layout.rs` carries a `Slot` of its own with the
/// same four numbers, and it predates this one.** It is the authority for the
/// partition *table*; this is the authority for the *numbers*, in the same
/// division that module already applies to [`STATE_PARTITION`]. Its `Slot` now
/// takes them from here rather than restating them.
///
/// The names are `A` and `B` because the record's are: `boot-a`, `boot-b`,
/// `system-a`, `system-b` are the GPT labels `cmd/rivet-image` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    /// The 1-based FAT boot partition. `ADR-018` §4 — `boot-a` is `p1`.
    ///
    /// This is also what `autoboot.txt`'s `boot_partition=` names and what the
    /// firmware reports in `/proc/device-tree/chosen/bootloader/partition`, so
    /// it is the number a slot is identified *by* on this hardware.
    pub const fn boot_partition(self) -> u32 {
        match self {
            Self::A => 1,
            Self::B => 2,
        }
    }

    /// The 1-based `/system` partition. `ADR-018` §4 — `system-a` is `p3`.
    pub const fn system_partition(self) -> u32 {
        match self {
            Self::A => 3,
            Self::B => 4,
        }
    }

    /// **The GPT label on this slot's `/system` partition**, and the name
    /// `cmdline.txt` mounts the root filesystem by: `root=PARTLABEL=system-a`.
    ///
    /// Added 2026-08-29 for `crate::operation::slot`, which reads
    /// `/proc/cmdline` to find out **which slot is actually running**. That
    /// question is answered from the mount table or the kernel command line and
    /// never by hashing a binary — a hash comparison fails silently, and the
    /// failure looks like an answer.
    ///
    /// **Here for [`STATE_PARTLABEL`]'s reason, in the same words:** the label
    /// is load-bearing at run time now, and a second literal `"system-a"` in
    /// another crate is a machine that writes one name and looks for another.
    ///
    /// **`cmd/rivet-image/src/layout.rs` still carries its own literals**, and
    /// that is a real second authority rather than a tidy division — the same
    /// one [`STATE_PARTLABEL`] closed for `/state` and this has not closed for
    /// the slots. Making that module take these strings from here is the fix,
    /// and it is a change to the image builder rather than to this file.
    pub const fn system_partlabel(self) -> &'static str {
        match self {
            Self::A => "system-a",
            Self::B => "system-b",
        }
    }

    /// **The wire form, and the word an operator types.** Part of the interface
    /// contract; never renamed.
    ///
    /// One spelling for every consumer. It lived in
    /// `crate::operation::trial::slot_word` and was private to that module,
    /// which was fine while the only thing that named a slot was the operation
    /// that armed one — and stopped being fine on 2026-08-26, when `ADR-022`
    /// §12 item 1 required `cmd/rivet-sysd`'s boot record to carry it too. A
    /// second spelling in the supervisor is `ADR-011` §3 with the two halves in
    /// different processes, and the disagreement would surface as a journal
    /// whose `slot` did not match the one `rivet apply trial-boot` accepts.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }

    /// The slot an update writes: the one that is not this one.
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    /// Which slot a boot partition number belongs to, or `None`.
    ///
    /// **`None` rather than a default.** The number comes from the firmware
    /// (`ADR-018` §5.3), and a number that is not one of the two means the card
    /// is not the one this layout describes. `AGENTS.md` §5 — refuse rather
    /// than guess, because the guess would name a partition an update would
    /// then overwrite.
    pub const fn from_boot_partition(partition: u32) -> Option<Self> {
        match partition {
            1 => Some(Self::A),
            2 => Some(Self::B),
            _ => None,
        }
    }

    /// Which slot a `/system` partition number belongs to, or `None`.
    ///
    /// [`Self::system_partition`] backwards, and it exists for one caller with
    /// one constraint: **`cmd/rivet-sysd` must be able to say which slot it is
    /// running without mounting anything.**
    ///
    /// The other way to that answer is the firmware's, through
    /// `rivet_adapter_image::BootSlots::running_slot` — and that mounts the FAT
    /// boot partition. `crate::operation::stop` already refused to put a FAT
    /// mount on the shutdown path, because *"a machine that cannot be stopped
    /// because its card would not mount is a worse failure than the surprise it
    /// prevents"*; the same argument applies with more force to the supervisor's
    /// startup, which is the one path a board has no way around.
    ///
    /// `/proc/self/mounts` already names the device `/system` came from, so the
    /// answer is a derivation rather than a second reading — `AGENTS.md` §5's
    /// *derive, never store*, and `PLAN.md` §26.4.1 records why the alternative
    /// people reach for (hashing the running binary) fails silently in the
    /// direction that matters.
    ///
    /// **`None` rather than a default**, for [`Self::from_boot_partition`]'s
    /// reason exactly: a number outside the layout means the card is not the one
    /// `ADR-018` §4 describes, and a guess would name a partition an update
    /// would overwrite.
    pub const fn from_system_partition(partition: u32) -> Option<Self> {
        match partition {
            3 => Some(Self::A),
            4 => Some(Self::B),
            _ => None,
        }
    }

    /// **Which slot this machine is running, out of the kernel's mount table.**
    /// `ADR-022` §12 item 1; `PLAN.md` §26.4.1.
    ///
    /// `text` is `/proc/self/mountinfo` verbatim — the adapter reads it, this
    /// derives from it, and `rivet-core` performs no I/O (`ADR-014` §3).
    ///
    /// The line looked for is the one mounted at `/`, and the answer is the
    /// **minor** of its `major:minor`. **`None` for everything else**, and the
    /// cases that covers are ordinary rather than exotic: a development host's
    /// root is not one of `ADR-018` §4's two partitions, and the emulator's is
    /// an initramfs (`ADR-026` §4). Neither is a fault.
    ///
    /// # Why `mountinfo` and the root mount, and not the two obvious things
    ///
    /// **Both obvious readings are wrong on a real board, and each was tried.**
    ///
    /// * **`/proc/self/mounts` and a `/system` mount point.** There is no
    ///   `/system` mount: the system partition **is** `/` (`ADR-018` §3 — the
    ///   image is the root filesystem), and `/system` is a directory on it. A
    ///   reader looking for that mount point finds nothing and answers *unknown*
    ///   on the one machine the question is about. This function did exactly that
    ///   until a board answered `unknown: no /system in the mount table` on
    ///   2026-08-26 — the only place it could have been caught.
    /// * **The device name.** `mounts` reports the root as `/dev/root`, which
    ///   carries no partition number at all, and `PLAN.md` §26.4.1 records that
    ///   `/proc/cmdline` reads back **empty** on this hardware. `mountinfo` is
    ///   the only one of the three that carries the number.
    ///
    /// And the reading people reach for instead — **hashing the running binary
    /// — is not a slot test.** §26.4.1 records it costing a wrong conclusion:
    /// two images built from commits that changed only documentation hold
    /// byte-identical binaries, so the digest matches across slots and the
    /// reading says *the trial boot did not happen* when it did. It fails
    /// silently in the direction that matters.
    ///
    /// # The minor number is the partition number, and a wrong reading declines
    ///
    /// `mmcblk0p3` is `179:3` and `mmcblk0p4` is `179:4`; `sda1` is `8:1`. The
    /// major is deliberately **not** checked — it names a driver, and pinning
    /// `179` would refuse the same layout on any other medium. What guards a
    /// mistaken reading is [`Self::from_system_partition`], which knows only two
    /// numbers and answers `None` for the rest: a machine whose root is minor 2
    /// gets *unknown* rather than a slot it is not running.
    ///
    /// **The assumption in this heading is not general.** A minor is the
    /// partition index only for a driver's first disk — `sdb3` is minor 19, not
    /// 3 — so a second disk on the same scheme declines here rather than
    /// answering wrongly. A caller that can read a partition's own sysfs
    /// `partition` attribute has the true number and should prefer it; see
    /// [`root_device`] for the pair such a caller resolves against, unmapped.
    pub fn from_mountinfo(text: &str) -> Option<Self> {
        let (_major, minor) = root_device(text)?;
        Self::from_system_partition(minor)
    }
}

/// The root mount's raw device number, as `(major, minor)`, out of
/// `/proc/self/mountinfo` text.
///
/// [`Slot::from_mountinfo`]'s own first step, factored out: that function then
/// treats the minor as a partition index, which only holds for a driver's
/// first disk (see its documentation). A platform adapter that can read a
/// partition's own sysfs `partition` attribute needs this raw pair to ask the
/// kernel, rather than the guess `Slot::from_mountinfo` makes alone.
///
/// `text` is `/proc/self/mountinfo` verbatim; this performs no I/O of its own
/// (`ADR-014` §3). `None` when no line is mounted at `/`, or that line's
/// `major:minor` field does not parse as two integers.
pub fn root_device(text: &str) -> Option<(u32, u32)> {
    let device = text.lines().find_map(|line| {
        // `mount-id parent-id major:minor root mount-point options…`
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.get(4)? == &"/").then_some(*fields.get(2)?)
    })?;
    let (major, minor) = device.split_once(':')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Where it goes. `ADR-008` §3.
///
/// The mount point ships as an empty directory in the image, because it cannot
/// be created at runtime: the root filesystem it would be created on is
/// read-only (`ADR-018` §3).
pub const STATE_MOUNT_POINT: &str = "/state";

/// Where Rivet's own configuration lives. `ADR-008` §3.1.
///
/// A **directory**, not a file. `ADR-017` §3.1.7 names it beside `/state/home`
/// and `/state/ext` as something "created by whatever writes into" it, and
/// `ADR-007` §3.4 has four layers of desired state, of which three are
/// persisted — so the thing at this path is a directory holding one file per
/// layer, and `rivet-store` is the authority for their names.
///
/// **Why the ADR rejected `/config`**, which is worth carrying here because it
/// is the argument for the whole layout: "a top-level name is supposed to state
/// ownership and lifetime, and `/config` states neither." `/state/config`
/// answers both — it is the operator's, and an update never touches it
/// (`ADR-008` §3, `ADR-018` §4).
///
/// Spelled out rather than composed from [`STATE_MOUNT_POINT`], because a
/// `const` cannot be built by `format!` and `concat!` cannot read one. The test
/// below is the guard that keeps the two agreeing.
pub const CONFIG_DIR: &str = "/state/config";

/// The desired-state document itself — `ADR-004` §6.
///
/// **`rivet-store`'s `ConfigStore` is what WRITES it**, and its `machine_path()`
/// is the authority. This constant exists because `rivet-core::operation` must
/// name the file to read it and `ADR-014` §3 does not let it depend on the
/// store. Spelled out for [`CONFIG_DIR`]'s reason — a `const` cannot be built by
/// `format!` — and guarded the same way: `cmd/rivet-sysd`'s
/// **Where a machine's secrets live** — `ADR-020` §5.
///
/// Root-owned. The document says *which* secrets exist and what each is for;
/// this directory holds the values, and the separation is what makes `rivet
/// export` safe to commit: an export reconstitutes a machine except its
/// secrets, which must be re-provisioned.
///
/// **One authority, and it is new on 2026-08-30** (`ADR-011`). The path was a
/// string literal in `cmd/rivet-initd`, `cmd/rivet-image` and `cmd/rivet-fleet`
/// before this constant existed, which is exactly the arrangement in which two

/// **The kernel's own ring buffer** — `ADR-022` §4.1's third journal source.
///
/// Here, and not in either of its two readers, because **both need it and
/// neither may depend on the other**: `cmd/rivet-sysd` ingests this file into
/// the journal, and `cmd/rivet-initd` WRITES to it so that pid 1's lines are
/// not lost with the console (`PLAN.md` §102.17). Pid 1 cannot depend on the
/// supervisor's crate — that is the whole reason it writes here rather than to
/// the journal — so a shared constant is the only way the two can be made to
/// agree at all. `ADR-011`.
pub const KMSG: &str = "/dev/kmsg";

/// of them come to disagree and nothing notices.
pub const SECRETS: &str = "/state/secrets";

/// `the_document_path_is_the_one_the_store_writes` fails if the two drift.
pub const MACHINE_DOCUMENT: &str = "/state/config/machine.toml";

/// **Where card-time material waits for the application it belongs to** —
/// `ADR-049`.
///
/// PID 1 copies the boot partition's staging bundle here before any service
/// runs, and the supervisor empties it into each application's own data
/// directory at the discovery pass that first sees that application installed.
/// It is root-owned and mode `0700`: everything in it is somebody else's
/// credential and no application may read another's.
///
/// A sibling of [`SECRETS`] rather than a corner of it, deliberately.
/// `/state/secrets` is the MACHINE's — the device seed, the peers file, the
/// values the document names — and it is read by root-owned programs. This is
/// material addressed to an unprivileged application that may not exist yet,
/// and it is deleted once delivered. Two lifetimes, two directories.
///
/// Spelled out rather than composed from [`STATE_MOUNT_POINT`], for the reason
/// [`CONFIG_DIR`] gives: a `const` cannot be built by `format!`.
pub const STAGING: &str = "/state/staging";

/// **Where the applications a card carried wait to be installed** — `ADR-050`
/// §4.
///
/// PID 1 copies each signed payload off the boot partition into
/// `/state/carried/<product>/payload.rvt` before any service runs, and the
/// supervisor asks `cmd/rivet-deploy` to install it on a convergence pass. The
/// **directory** is the record that the payload was carried and is never
/// removed; the payload inside it is removed once its install has finished. See
/// [`crate::carried`] for why that pairing is what makes a second boot copy
/// nothing.
///
/// A sibling of [`STAGING`] rather than a corner of it, on that constant's own
/// argument. Staging is per-machine *material* addressed to an application, is
/// secret, and is destroyed on the card; this is *software*, is a signed
/// artifact that is public by construction, and stays on the card. Two
/// lifetimes, two directories.
///
/// Spelled out rather than composed from [`STATE_MOUNT_POINT`], for the reason
/// [`CONFIG_DIR`] gives: a `const` cannot be built by `format!`.
pub const CARRIED: &str = "/state/carried";

/// **The machine's artifact store** — `ADR-052` §7, and the only place an
/// artifact this machine can install is kept.
///
/// A digest-keyed object store. `crate::artifact` is the whole of its layout and
/// its retention rule; this constant is here rather than there for the reason
/// every other `/state` path is here — one file answers *what is on `/state`*.
///
/// A sibling of [`CARRIED`] rather than a corner of it, on that constant's own
/// argument one notch further. `/state/carried` is what a **card** brought and
/// its entries are named by product because the card knew the product; this is
/// what has been **transferred** to a running machine and its entries are named
/// by digest because that is the only name the transfer could verify. Two
/// origins, two directories — and `ADR-050`'s carried payload is deliberately
/// not routed through this one, because PID 1 has already put it somewhere it
/// cannot be swapped for something else.
///
/// Spelled out rather than composed from [`STATE_MOUNT_POINT`], for the reason
/// [`CONFIG_DIR`] gives: a `const` cannot be built by `format!`.
pub const ARTIFACTS: &str = "/state/artifacts";

// ---------------------------------------------------------------------------
// The journal on disk — `ADR-022` §8
// ---------------------------------------------------------------------------

/// Where the journal is kept. `ADR-022` §8 names it.
///
/// **Every decision in this block is an SD-wear decision** (`ADR-018` §7), and
/// they are here rather than in [`crate::journal`] because that module is about
/// *what a record is* and this is about where the machine puts it. Two
/// programs read these: `cmd/rivet-sysd` writes, `rivet log` reads
/// (`ADR-022` §9), and a second copy of the path in either would be the failure
/// `ADR-011` names — silent, because a reader looking in the wrong place finds
/// an empty log rather than an error.
pub const LOG_DIR: &str = "/state/log";

/// `0700` on the directory and `0600` on the files.
///
/// **Not because records hold secrets** — `ADR-020` §4's `Secret` type redacts
/// in `Debug` and `Display`, so a secret rendered into a field emits
/// `<secret>`, and `ADR-022` §3.3 makes that a property of the type rather than
/// a rule anybody has to remember. It is because a journal is a complete account
/// of what a machine did, which is worth protecting on its own, and because
/// Rivet has exactly one account today (`ADR-008` §7.3 puts multi-user at
/// Stage 4) so a wider mode would grant access to nobody and cost the property
/// the day an account is added.
pub const LOG_DIR_MODE: u32 = 0o700;

/// See [`LOG_DIR_MODE`].
pub const LOG_FILE_MODE: u32 = 0o600;

/// The file records are appended to.
pub const ACTIVE_LOG: &str = "/state/log/journal.jsonl";

/// How large the active file may get before it is rotated — **1 MiB**.
///
/// `ADR-022` §8: size-capped files, capped total, oldest deleted first, because
/// "an unbounded log kills the card" — the same hazard `ADR-017` §3.5's restart
/// loop has, arriving by a slower route.
///
/// A boot of the supervisor writes on the order of thirty records, so a megabyte
/// is thousands of boots' worth for a machine that is behaving. It is the
/// machine that is *not* behaving — a service in a restart loop — that fills
/// this, which is exactly the case the cap exists for.
///
/// **The number is not configuration yet, and that is tracked rather than
/// chosen.** `ADR-022` §8 says "retention is configuration — a field in the
/// desired-state document like anything else", and `rivet_config::Document`
/// has no such key. Inventing one is `AGENTS.md` §1's kind of decision.
pub const LOG_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// How many files the journal keeps, including the active one — **8**, so
/// 8 MiB total against [`LOG_FILE_MAX_BYTES`].
///
/// The bound that matters is the *total*, and it is what `ADR-022` §8 caps.
/// Expressed as a count of size-capped files rather than as a byte total
/// because rotation is what enforces it and rotation counts files.
pub const LOG_FILES_KEPT: usize = 8;

/// The name of rotated file `n`, where 1 is the most recent.
///
/// `n == 0` is [`ACTIVE_LOG`]; a caller asking for it gets that, so the two
/// naming schemes cannot disagree.
#[must_use]
pub fn rotated_log(n: usize) -> String {
    if n == 0 {
        return ACTIVE_LOG.to_string();
    }
    format!("{LOG_DIR}/journal.{n}.jsonl")
}

/// Every file a reader must concatenate, **oldest first**.
///
/// Oldest first because that is the order the records were written in, and
/// [`crate::journal::boots_in_order`] depends on it: a boot identifier is a
/// random UUID and a machine with no RTC starts every boot at the epoch, so
/// *file order* is the only thing that can say which boot came first
/// (`ADR-021`, `ADR-022` §3.2).
///
/// The list names files that may not exist — a machine that has not rotated yet
/// has only [`ACTIVE_LOG`] — and saying so is the reader's job rather than
/// this function's, which has no way to look.
#[must_use]
pub fn log_files_oldest_first() -> Vec<String> {
    (1..LOG_FILES_KEPT)
        .rev()
        .map(rotated_log)
        .chain(std::iter::once(ACTIVE_LOG.to_string()))
        .collect()
}

/// `ext4`, ruled in `ADR-018` §7 over `f2fs`.
///
/// The reasoning there is worth keeping in view because it is unusual: `f2fs` is
/// designed for flash and would probably wear the card less, and `ext4` was
/// chosen anyway, "during bring-up, \[because\] every recovery tool understands it
/// and every failure mode is documented". Removing unfamiliarity from "why will
/// this not mount" beat a wear improvement nobody has measured.
///
/// Unlike `crate::mount`'s pseudo-filesystems, getting this wrong is not a
/// refusal: mounting an `ext4` partition as some other type fails, but mounting
/// it as `ext2` **succeeds** and silently drops the journal. So the type is
/// named rather than left to a kernel guess, and `ADR-024` §8.1's audit is the
/// authority for `CONFIG_EXT4_FS` being present.
pub const STATE_FSTYPE: &str = "ext4";

/// How it is mounted, and the interesting entry is the one that is absent.
///
/// * **`noatime`** — `ADR-018` §7 names it first, for the reason that makes it
///   more than a tweak on this hardware: "every read otherwise becomes a write",
///   and §2 established that *all* of a Rivet machine's card wear is
///   concentrated in this one partition, because `/system` is read-only and
///   everything else is a tmpfs.
/// * **`nosuid`** — nothing in Rivet is set-user-ID. `ADR-017` §3.1 runs the
///   supervisor as root already, so there is nothing for the bit to buy, and
///   this is the one filesystem an attacker could write to.
/// * **`nodev`** — device nodes are the kernel's, on devtmpfs (`ADR-024` §3).
///
/// **`noexec` is deliberately not here.** It is the flag one reaches for on a
/// writable partition, and it would be a decision the record has not taken:
/// `ADR-018` §6 loop-mounts extension images out of `/state/ext`, and `ADR-015`
/// gives applications artifacts that have to run from somewhere. The same
/// judgement is already recorded one filesystem over — `crate::mount`'s `/run`
/// omits `noexec` "because services are started from it", and
/// `run_is_executable_because_services_are_started_from_it` is the test that
/// keeps it omitted. Adding `noexec` here should be a ruling with a stated
/// consequence, not a tidy-up.
pub const STATE_FLAGS: &[MountFlag] = &[MountFlag::NoAtime, MountFlag::NoSuid, MountFlag::NoDev];

/// The device node for `/state`, given the disk it is on.
///
/// The `p` infix is the kernel's convention for a partition of a device whose
/// name ends in a digit — `mmcblk0` becomes `mmcblk0p5` — and it is **not**
/// universal: `sda` becomes `sda5` with no `p`. This function applies the rule
/// rather than assuming one, so a USB-booted board is a caller change and not a
/// silent mismount.
///
/// **Demoted to a fall-back 2026-08-28, `PLAN.md` §62.** It is still correct and
/// still used, and it is no longer how a running machine *first* answers the
/// question — [`crate::partlabel`] is, because this composition needs the disk's
/// name and that is the one thing about a card that changes with the reader.
/// The caller that still reaches here is `cmd/rivet-initd` with no readable
/// sysfs, which is a machine in enough trouble that its own `/dev/mmcblk0` is
/// the best remaining guess.
pub fn state_device(disk: &str) -> String {
    partition_device(disk, STATE_PARTITION)
}

/// The device node for any 1-based partition of a disk.
///
/// The rule [`state_device`] documents, applied to the other four partitions in
/// `ADR-018` §4's table as well. Extracted 2026-08-23 when `cmd/rivet-initd`'s
/// boot-slot port needed the same composition for `boot-b` and `system-b`; the
/// alternative was a second copy of the `p`-infix rule in a different crate,
/// which is a rule that would then be true in one of them.
pub fn partition_device(disk: &str, partition: u32) -> String {
    let sep = match disk.chars().last() {
        Some(c) if c.is_ascii_digit() => "p",
        _ => "",
    };
    format!("{disk}{sep}{partition}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mount::stage0_plan;

    #[test]
    fn the_partition_number_is_adr_018_4s_fifth() {
        assert_eq!(STATE_PARTITION, 5);
    }

    /// **Which slot is running, from the kernel's own mount table.**
    /// `ADR-022` §12 item 1.
    ///
    /// **The fixture is the line a board actually produced**, copied off
    /// `10.0.0.2` on 2026-08-26 after a trial boot into slot A. The first
    /// version of this test invented its own — a `/system` mount, a device with
    /// a partition number — and every case passed against a shape no Rivet
    /// machine has. `AGENTS.md` §4: a round trip against one's own writer proves
    /// the two agree and nothing about the format.
    #[test]
    fn the_running_slot_is_read_off_the_kernels_mount_table() {
        let board = "\
20 1 179:3 / / ro,relatime - ext4 /dev/root ro
21 20 0:6 / /dev rw,relatime - devtmpfs devtmpfs rw,size=429548k,mode=755
22 20 0:16 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw
23 20 179:5 / /state rw,relatime - ext4 /dev/mmcblk0p5 rw
";
        assert_eq!(Slot::from_mountinfo(board), Some(Slot::A));
        assert_eq!(
            Slot::from_mountinfo(&board.replace("179:3 / /", "179:4 / /")),
            Some(Slot::B)
        );
        // The major names a driver, not a layout, so it is not checked: the
        // same partition scheme on an SD card, an NVMe or a loop device must
        // answer the same.
        assert_eq!(
            Slot::from_mountinfo("20 1 8:4 / / ro,relatime - ext4 /dev/root ro\n"),
            Some(Slot::B)
        );
    }

    /// **Absence is ordinary and is not a fault**, and a number outside the
    /// layout declines rather than guessing — the guess would name a partition
    /// an update overwrites.
    #[test]
    fn a_machine_that_is_not_a_rivet_card_answers_nothing_rather_than_guessing() {
        assert_eq!(Slot::from_mountinfo(""), None);
        // A development host: root is not one of ADR-018 §4's two partitions.
        assert_eq!(
            Slot::from_mountinfo("20 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw\n"),
            None
        );
        // The emulator: an initramfs root, which has no partition at all.
        assert_eq!(
            Slot::from_mountinfo("20 1 0:2 / / rw,relatime - rootfs rootfs rw\n"),
            None
        );
        // **`/state` must not be mistaken for the root**, which is the failure a
        // reader that searched for a partition number rather than the MOUNT
        // POINT would have: p5 is on every Rivet card and is not a slot.
        assert_eq!(
            Slot::from_mountinfo("23 20 179:5 / /state rw,relatime - ext4 /dev/mmcblk0p5 rw\n"),
            None
        );
        // A line too short to carry a mount point.
        assert_eq!(Slot::from_mountinfo("20 1 179:3\n"), None);
    }

    /// **The raw pair, for every naming scheme a board or a workstation might
    /// present** — proof that extraction does not lose a later disk's minor the
    /// way reading it as a partition number would. `sdb`'s own base minor is 16
    /// (`Documentation/admin-guide/devices.txt`), so `sdb3` and `sdb4` are 19 and
    /// 20; hand-composed from that table, not read off a board.
    #[test]
    fn root_device_extracts_the_raw_pair_for_every_naming_scheme() {
        let mounted_at_root = |device: &str| format!("20 1 {device} / / rw,relatime - ext4 /dev/root rw\n");
        assert_eq!(root_device(&mounted_at_root("8:3")), Some((8, 3)), "sda3");
        assert_eq!(root_device(&mounted_at_root("8:4")), Some((8, 4)), "sda4");
        assert_eq!(root_device(&mounted_at_root("8:19")), Some((8, 19)), "sdb3");
        assert_eq!(root_device(&mounted_at_root("8:20")), Some((8, 20)), "sdb4");
        assert_eq!(root_device(&mounted_at_root("179:3")), Some((179, 3)), "mmcblk0p3");
        assert_eq!(root_device(&mounted_at_root("259:3")), Some((259, 3)), "an NVMe root");
    }

    /// No root line and a line too short to carry one both decline rather than
    /// guessing — the same "absence is ordinary" cases
    /// `a_machine_that_is_not_a_rivet_card_answers_nothing_rather_than_guessing`
    /// covers at the slot level.
    #[test]
    fn root_device_declines_when_nothing_is_mounted_at_root() {
        assert_eq!(root_device(""), None);
        // Only /proc and /dev, nothing at "/".
        assert_eq!(
            root_device(
                "21 20 0:6 / /dev rw,relatime - devtmpfs devtmpfs rw\n\
                 22 20 0:16 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw\n"
            ),
            None
        );
        // A line too short to carry a mount point at all.
        assert_eq!(root_device("20 1 179:3\n"), None);
    }

    /// The two partition mappings are each other's inverse, which is what makes
    /// deriving a slot from a mount table the same fact as writing one.
    #[test]
    fn the_system_partition_mapping_round_trips() {
        for slot in [Slot::A, Slot::B] {
            assert_eq!(Slot::from_system_partition(slot.system_partition()), Some(slot));
        }
        assert_eq!(Slot::from_system_partition(5), None, "that is /state");
    }

    /// The `p` infix rule, both ways. Getting this wrong on a USB-booted board
    /// mounts nothing rather than mounting the wrong thing, which is the safe
    /// direction — but it is still a machine with no configuration.
    #[test]
    fn a_device_ending_in_a_digit_takes_the_p_infix_and_one_ending_in_a_letter_does_not() {
        assert_eq!(state_device("/dev/mmcblk0"), "/dev/mmcblk0p5");
        assert_eq!(state_device("/dev/nvme0n1"), "/dev/nvme0n1p5");
        assert_eq!(state_device("/dev/sda"), "/dev/sda5");
    }

    /// The same rule, for the four partitions that are not `/state`.
    #[test]
    fn the_p_infix_rule_is_the_same_one_for_every_partition() {
        assert_eq!(partition_device("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
        assert_eq!(partition_device("/dev/sda", 4), "/dev/sda4");
        // ...and `state_device` is that function, not a second copy of it.
        assert_eq!(
            state_device("/dev/mmcblk0"),
            partition_device("/dev/mmcblk0", STATE_PARTITION)
        );
    }

    /// `ADR-018` §4's table, as the two slots see it.
    #[test]
    fn the_slot_partition_numbers_are_adr_018_4s_table() {
        assert_eq!(Slot::A.boot_partition(), 1);
        assert_eq!(Slot::B.boot_partition(), 2);
        assert_eq!(Slot::A.system_partition(), 3);
        assert_eq!(Slot::B.system_partition(), 4);
    }

    /// The firmware reads slot A's `autoboot.txt` whichever slot booted
    /// (`AUTOBOOT_PARTITION`, proven on hardware). The two constants must
    /// therefore agree, or a slot-B commit writes a file nothing reads — which
    /// is the defect that correction was for.
    #[test]
    fn the_partition_the_firmware_reads_autoboot_from_is_slot_as_boot_partition() {
        assert_eq!(AUTOBOOT_PARTITION, Slot::A.boot_partition());
    }

    /// No partition number may belong to both slots, in either table. A mapping
    /// that collided would pass every test above and still overwrite the
    /// running system on an update.
    #[test]
    fn no_partition_number_is_shared_between_the_two_slots() {
        let all = [
            Slot::A.boot_partition(),
            Slot::B.boot_partition(),
            Slot::A.system_partition(),
            Slot::B.system_partition(),
            STATE_PARTITION,
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                assert!(i == j || a != b, "partition {a} is used twice");
            }
        }
    }

    #[test]
    fn the_other_slot_is_the_one_an_update_writes() {
        assert_eq!(Slot::A.other(), Slot::B);
        assert_eq!(Slot::B.other(), Slot::A);
        assert_eq!(Slot::A.other().other(), Slot::A);
    }

    /// The firmware's number is the input, so a number the layout does not
    /// contain must be refused rather than defaulted to `A`.
    #[test]
    fn a_boot_partition_outside_the_layout_belongs_to_no_slot() {
        assert_eq!(Slot::from_boot_partition(1), Some(Slot::A));
        assert_eq!(Slot::from_boot_partition(2), Some(Slot::B));
        for n in [0, 3, 4, STATE_PARTITION, 99] {
            assert_eq!(Slot::from_boot_partition(n), None, "{n} named a slot");
        }
    }

    /// Round-trip, so the two directions cannot drift apart.
    #[test]
    fn a_slots_boot_partition_maps_back_to_the_same_slot() {
        for slot in [Slot::A, Slot::B] {
            assert_eq!(Slot::from_boot_partition(slot.boot_partition()), Some(slot));
        }
    }

    /// `ADR-018` §7's first measure, and the one that matters most on an SD
    /// card: without it every read of `/state` becomes a write.
    #[test]
    fn state_is_mounted_noatime_because_all_of_the_cards_wear_is_here() {
        assert!(STATE_FLAGS.contains(&MountFlag::NoAtime));
    }

    #[test]
    fn state_is_nosuid_and_nodev_because_it_is_the_only_writable_filesystem() {
        assert!(STATE_FLAGS.contains(&MountFlag::NoSuid));
        assert!(STATE_FLAGS.contains(&MountFlag::NoDev));
    }

    /// Asserted as an **absence**, so that adding `noexec` has to change a test
    /// that says why it is not there — the same shape as
    /// `run_is_executable_because_services_are_started_from_it`.
    #[test]
    fn state_is_not_noexec_because_extensions_and_applications_run_from_it() {
        assert!(
            !STATE_FLAGS.contains(&MountFlag::NoExec),
            "ADR-018 §6 loop-mounts extension images out of /state and ADR-015 runs \
             application artifacts; noexec here needs a ruling, not a tidy-up"
        );
    }

    /// `/state` is not a pseudo-filesystem and must not drift into the plan that
    /// holds them: `crate::mount`'s specs are all sourceless types the kernel
    /// synthesises, and every caller of `stage0_plan` treats them that way.
    #[test]
    fn state_is_not_in_the_pseudo_filesystem_plan() {
        assert!(!stage0_plan().iter().any(|s| s.target == STATE_MOUNT_POINT));
        assert!(!stage0_plan().iter().any(|s| s.fstype == STATE_FSTYPE));
    }

    /// Its mount point must not sit under anything the plan mounts, or the
    /// filesystem mounted over the parent would hide it.
    #[test]
    fn nothing_in_the_plan_is_mounted_over_states_parent() {
        for spec in stage0_plan() {
            assert!(
                !STATE_MOUNT_POINT.starts_with(&format!("{}/", spec.target)),
                "{STATE_MOUNT_POINT} is inside {}, which would hide it",
                spec.target
            );
        }
    }

    /// `CONFIG_DIR` is written out rather than composed, so this is what keeps
    /// it from drifting away from the filesystem it is on. Moving `/state`
    /// without moving this would give a machine whose configuration is on the
    /// read-only root, where writing it fails with `EROFS`.
    #[test]
    fn the_configuration_directory_is_inside_state_and_survives_an_update() {
        assert_eq!(CONFIG_DIR, "/state/config");
        assert!(CONFIG_DIR.starts_with(&format!("{STATE_MOUNT_POINT}/")));
        // One component below it: `ADR-008` §3.1 puts it directly under
        // `/state`, beside `/state/home` and `/state/ext`.
        assert_eq!(CONFIG_DIR.matches('/').count(), 2);
    }

    #[test]
    fn the_mount_point_is_absolute_and_top_level() {
        assert_eq!(STATE_MOUNT_POINT, "/state");
        assert_eq!(STATE_MOUNT_POINT.matches('/').count(), 1);
    }

    // -- the journal's own files ---------------------------------------------

    /// The same guard `CONFIG_DIR` has, for the same reason: the path is
    /// spelled out, so nothing but a test keeps it on the filesystem an update
    /// leaves alone. A journal on the read-only root would fail every write
    /// with `EROFS`; one under `/system` would be **erased by every update**,
    /// which is worse because it succeeds.
    #[test]
    fn the_journal_is_on_the_filesystem_an_update_does_not_touch() {
        assert_eq!(LOG_DIR, "/state/log");
        assert!(LOG_DIR.starts_with(&format!("{STATE_MOUNT_POINT}/")));
        assert_eq!(LOG_DIR.matches('/').count(), 2);
        assert!(ACTIVE_LOG.starts_with(&format!("{LOG_DIR}/")));
    }

    /// `rotated_log(0)` is the active file and not `journal.0.jsonl`.
    ///
    /// Two naming schemes for one file is `ADR-011`'s failure in miniature, and
    /// the way it would show up is a rotation that renamed the active file to a
    /// name the reader never looks at — a boot's records present on the card and
    /// invisible to `rivet log`.
    #[test]
    fn the_active_file_has_exactly_one_name() {
        assert_eq!(rotated_log(0), ACTIVE_LOG);
        assert_eq!(rotated_log(1), "/state/log/journal.1.jsonl");
        assert_ne!(rotated_log(1), ACTIVE_LOG);
    }

    /// Oldest first, and the active file last. `crate::journal::boots_in_order`
    /// takes file order as the authority for which boot came first, so a
    /// reversed list would make `--boot previous` show the wrong boot — an
    /// answer that looks right.
    #[test]
    fn the_reader_gets_every_file_oldest_first_with_the_active_one_last() {
        let files = log_files_oldest_first();
        assert_eq!(files.len(), LOG_FILES_KEPT);
        assert_eq!(
            files.first().map(String::as_str),
            Some(rotated_log(LOG_FILES_KEPT - 1).as_str())
        );
        assert_eq!(files.last().map(String::as_str), Some(ACTIVE_LOG));
        // Every name distinct: a duplicate would have a reader count a boot's
        // records twice.
        let mut sorted = files.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), files.len(), "{files:?}");
    }

    /// The cap is a total, and the total is what `ADR-022` §8 is about. Stated
    /// as an assertion so that raising either number is a decision about how
    /// much of the card the journal may have rather than an edit to one
    /// constant.
    #[test]
    fn the_journal_may_not_have_more_than_eight_megabytes_of_the_card() {
        let total = LOG_FILE_MAX_BYTES * LOG_FILES_KEPT as u64;
        assert_eq!(total, 8 * 1024 * 1024);
    }

    #[test]
    fn the_journal_is_not_readable_by_a_future_unprivileged_account() {
        assert_eq!(LOG_DIR_MODE & 0o077, 0);
        assert_eq!(LOG_FILE_MODE & 0o077, 0);
        // And not executable: nothing in it is a program.
        assert_eq!(LOG_FILE_MODE & 0o111, 0);
    }
}
