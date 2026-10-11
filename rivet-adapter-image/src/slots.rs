//! The A/B boot primitive, behind a port.
//!
//! `ADR-018` §5 rules the mechanism and it is the *firmware's*, not Rivet's:
//! the Raspberry Pi bootloader sets a one-shot `tryboot` flag, boots the other
//! slot exactly once, and returns to the previous slot on the next reset if
//! nothing commits — **with no userspace involvement on the failure path at
//! all.** That last property is the whole of why §5 is a good design, and it is
//! also the reason this module exists.
//!
//! # Why a port — ruled 2026-08-23
//!
//! **x86 has no `tryboot`.** `PLAN.md` §9 has Rivet replacing every one of the
//! maintainer's home servers, explicitly including a small-form-factor x86
//! machine, and the 2026-08-23 survey behind this ruling weighed five ways x86 could supply the same
//! primitive — UEFI `BootNext`, an own `uefi-rs` loader, systemd-boot's boot
//! counting, GRUB's `grubenv`, U-Boot's `bootcount`. All five satisfy the
//! no-userspace-on-failure property; the recommendation is `BootNext` first,
//! an own loader as the fallback, and not GRUB. None of it is tested, and §5.3
//! exists because Raspberry Pi's own documentation and this repository once
//! disagreed about a partition — so treat that survey exactly as unproven as
//! this module's Pi half.
//!
//! **Done this way an x86 machine is an adapter; done the other way it is a
//! rewrite of the update mechanism.** The cost today is [`BootSlots`] and one
//! implementor.
//!
//! # Why this is a library crate — moved 2026-08-23
//!
//! **It was `cmd/rivet-initd/src/slots.rs`, and `cmd/rivet-initd` is a binary
//! crate with no lib target**, so nothing outside PID 1 could name [`BootSlots`]
//! at all. `ADR-028` §4 rules that an authenticated principal supplies the
//! authority to install an update and **does not say which process executes
//! it** — and if the process that arms a trial boot is anything other than PID 1,
//! a port trapped inside PID 1's binary is not a port. The move happened before
//! that question was answered rather than after, because the answer would
//! otherwise arrive as a refactor with no forcing function (`ADR-014` §2's own
//! argument for the full crate split).
//!
//! **Nothing about the behaviour changed in the move.** It was corrected on
//! hardware on 2026-08-23 and that correction is not this change's to touch; the
//! slot-B regression is asserted here *and* in `cmd/rivet-initd`'s `confirm`,
//! because the defect was reached through that call path.
//!
//! # The two constraints the record places on the cut
//!
//! Both are from hardware, both are easy to miss, and both are the reason this
//! trait is wider than *commit / arm / am-I-in-one*.
//!
//! 1. **The commit needs a *location*, not just a flag.** Two boots of one card
//!    on 2026-08-23 proved the Pi firmware reads the **first** FAT partition's
//!    `autoboot.txt` and only that one, never the booted partition's —
//!    `rivet_core::state::AUTOBOOT_PARTITION` carries the evidence, and
//!    `cmd/rivet-initd`'s `confirm` had it wrong until that board corrected it.
//!    UEFI's answer is different *in kind*: `BootNext` names a boot entry in an
//!    EFI variable. A trait whose commit took no location would bake in the Pi's
//!    answer — and bake in one that was wrong here for as long as it stood. So
//!    [`BootSlots::default_slot_record`] is part of the surface.
//! 2. **It must abstract where a slot's payload goes.** `ADR-018` §4 gives the
//!    Pi *two* FAT boot partitions and two `/system` partitions; a UEFI machine
//!    would have one ESP holding `/EFI/rivet/a/` and `/EFI/rivet/b/`. A port
//!    offering only the flag leaves §4's five-partition layout quietly
//!    Pi-specific while looking ported. So [`BootSlots::payload`] is part of the
//!    surface too.
//!
//! # What this module is NOT
//!
//! **It is not the update path.** Nothing here fetches, verifies or writes a
//! payload. [`BootSlots::payload`] says *where* one would go and stops there —
//! `ADR-028` §8 item 1 leaves the signature scheme unruled, so anything that
//! verified would be inventing a ruling. The port is deliberately shaped so
//! that `ADR-028` §4's answer to *who authorises an update* — an authenticated
//! principal through the administration surface — is still free to be given:
//! nothing here decides, and nothing here can be reached except by a caller
//! that already has.
//!
//! # The two safety interlocks on arming — added 2026-08-23
//!
//! [`BootSlots::arm_trial_boot`] refuses two states, and neither is a tidy-up.
//! Both fail closed, both name what to change, and both are also reachable
//! without writing anything through [`BootSlots::can_arm_trial_boot`], so that a
//! caller computing a plan cannot show one and then be refused at the moment it
//! is confirmed (`ADR-006` §6.2 — every mutating operation supports a dry run).
//!
//! 1. **`tryboot_a_b=1` must actually apply.** `ADR-018` §5.3's trap, in its own
//!    words: *"In tryboot mode the firmware loads `tryboot.txt` **instead of**
//!    `config.txt`. On this system `config.txt` carries `arm_64bit=1`, and there
//!    is no 32-bit kernel on the boot partition — so a tryboot reboot with no
//!    `tryboot.txt` present would have attempted a 32-bit boot and lost the
//!    machine."* `tryboot_a_b=1` is what makes the firmware read `config.txt`
//!    anyway. `cmd/rivet-image` writes it under `[all]` on every card it
//!    builds, so a card this repository produced is safe — **a hand-edited one is
//!    not**, and `ADR-002` §4.1 says the operator edits that partition from
//!    Windows. [`require_tryboot_a_b`] is the check.
//! 2. **The running slot may not be trialled.** A "trial" of the slot already
//!    running is a reboot that comes back identical, and then looks exactly like
//!    an update that succeeded — the same silent-success failure direction
//!    `ADR-018` §5.1 records the slot-B commit bug having.
//!
//! **What neither of them checks, said plainly.** Whether the *target* slot's
//! boot partition carries a `tryboot.txt` at all. That file is on the partition
//! the trial would boot, which this crate does not mount — it mounts only
//! `rivet_core::state::AUTOBOOT_PARTITION`'s. With `tryboot_a_b=1` set the
//! firmware should not read it; that "should" is `ADR-018` §5.3's own
//! documentation-derived clause and no board has separated the two files.
//!
//! # Half of this has no production caller, and it no longer needs an allow
//!
//! **[`BootSlots::in_trial_boot`], [`BootSlots::running_slot`] and
//! [`BootSlots::commit`] are PID 1's item 5, via `cmd/rivet-initd`'s
//! `confirm`.** ~~Arming~~ **has a caller since 2026-08-23** — `cmd/rivet`, the
//! CLI, over an authenticated session, which is `ADR-028` §4's authenticated
//! principal supplying the *authority*; `rivet_core::operation::trial` is the
//! operation and `ADR-006` §3 is why it is in the core rather than in the CLI.
//! The payload locations and the record's location still have no production
//! caller, because **the thing that would call them is the update path, which is
//! deliberately not built** (see above).
//!
//! Until 2026-08-23 each carried its own `#[allow(dead_code)]` naming the caller
//! it was missing. **Those are gone, and not because the problem went away:**
//! this is a library crate now, so a public item is reachable by definition and
//! the lint no longer fires. That is a real loss of a signal, so the check the
//! allows existed to force is written here instead — **an operation still
//! uncalled when the update path exists is an operation that was cut wrong**,
//! and the boot-slot port (x86 has no `tryboot`; ruled 2026-08-23) carries the same sentence.
//!
//! # None of this has run
//!
//! `ADR-026` §4 puts `tryboot` and `/proc/device-tree/chosen/bootloader` in the
//! column QEMU does not emulate **at all**. Every line below is exercised by
//! unit tests against [`BootMedium`] and by nothing else. That is the same
//! posture `cmd/rivet-initd`'s `confirm` has always had and it is not a
//! substitute for the board; `AGENTS.md` §6.

use rivet_core::package::update::TargetClass;
use rivet_core::state::{Slot, partition_device};

use crate::firmware::{
    AUTOBOOT_LOCK, AUTOBOOT_PARTITION_MOUNT, BOOT_DISK, BOOT_FSTYPE, DT_TRYBOOT,
    autoboot_partition_device,
    boot_partition_device, mount_autoboot_partition, parse_dt_u32, umount_autoboot_partition,
};
use crate::port::{BootMedium, Listing};

/// `ADR-018` §5.1 and `ADR-002` §4 both name the file.
pub const AUTOBOOT_FILE: &str = "autoboot.txt";

/// Written, synced, then renamed over the original, so a power cut during the
/// write cannot leave a half-written `autoboot.txt` — which on a Pi means a
/// board that will not boot.
pub const AUTOBOOT_TEMP: &str = "autoboot.txt.new";

/// The setting that decides whether a tryboot boot reads `config.txt` or
/// `tryboot.txt`. `ADR-018` §5.3.
///
/// Named rather than spelled at the two places that need it, because getting it
/// wrong by one character turns the check that guards the trap into a check that
/// always refuses — which is safe, and which would be blamed on the card.
pub const TRYBOOT_A_B: &str = "tryboot_a_b";

/// Why an arming or a commit was refused, in the one distinction a caller has
/// to act on.
///
/// An arming is an operator's, and `ADR-006` §4.1 requires that an operator —
/// or the agent standing in for one — can branch on *what kind* of failure it
/// was without reading English. A commit is PID 1's, and PID 1 branches on the
/// same distinction: a [`Self::Medium`] refusal is the one kind a later attempt
/// can answer differently, so it is the one kind PID 1 retries.
///
/// * [`Self::Unsafe`] — retrying will not help. Something about this card or
///   this machine has to change first, and the message says what.
/// * [`Self::Medium`] — the card or the kernel refused. The message carries what
///   the system said (`AGENTS.md` §5), and the next step is to look at hardware
///   rather than at the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmRefusal {
    /// A safety interlock, or an `autoboot.txt` this crate will not rewrite.
    /// **Fails closed**: nothing was written.
    Unsafe(String),
    /// The medium could not be read or written.
    Medium(String),
}

impl ArmRefusal {
    /// The message, whichever kind this is.
    pub fn why(&self) -> &str {
        match self {
            Self::Unsafe(why) | Self::Medium(why) => why,
        }
    }
}

impl core::fmt::Display for ArmRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.why())
    }
}

/// Which slot, in whatever way the platform's firmware names one.
///
/// **Deliberately not `enum { A, B }` at this seam**, even though
/// [`rivet_core::state::Slot`] is exactly that. The number here is the one the
/// *firmware* reported — on the Pi, `/proc/device-tree/chosen/bootloader/
/// partition` — and `cmd/rivet-initd`'s `confirm` writes it back into
/// `autoboot.txt` verbatim. Mapping it onto a name at this seam would mean this
/// crate deciding what an unrecognised number means, which is the inference
/// `AGENTS.md` §5 forbids, and it would change behaviour that a board corrected
/// on 2026-08-23.
///
/// The implementor may of course refuse a number it does not recognise, and the
/// Pi one does — see [`PiFirmware::payload`], where naming the wrong partition
/// would have an update overwrite the running system.
///
/// **No code above this port may do arithmetic on the number.** "The other
/// slot" is [`BootSlots::other_slot`]'s to answer, because on a UEFI machine it
/// is not `3 - n` and on the Pi it is only accidentally that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotId(pub u32);

impl core::fmt::Display for SlotId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where the record of *which slot is the default* lives.
///
/// Constraint 1 above, as a type. The point is not that a caller reads it — the
/// port does the writing — it is that the operation is **located**, and that a
/// second platform states a location of a different kind rather than inheriting
/// the Pi's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotRecord {
    /// A file on one particular partition of the boot device.
    ///
    /// The Pi's answer. The *partition* is the half that matters and the half
    /// that was wrong here: it is always `ADR-018` §4's first FAT partition,
    /// never the one the firmware booted.
    FileOnPartition {
        device: String,
        fstype: &'static str,
        file: &'static str,
    },
    /// A variable the firmware owns, named rather than located.
    ///
    /// UEFI `BootNext` / `BootOrder` — the boot-slot port's
    /// recommended answer. **Nothing constructs this today**, and it is carried
    /// here rather than added later because a one-variant enum is a location
    /// that has not been abstracted: the whole of constraint 1 is that the two
    /// platforms answer *in different kinds*, and an enum with only the Pi's
    /// kind in it demonstrates nothing.
    ///
    /// It is **not** a stub in the `AGENTS.md` §3 sense — there is no
    /// implementor to fake success, only a shape recorded from a survey that
    /// the same section of `PLAN.md` says is untested documentation.
    FirmwareVariable { name: &'static str },
}

/// Where one slot's payload is written.
///
/// `ADR-018` §5.2 step 1: "the new image to the inactive `system-` partition and
/// its kernel to the inactive `boot-` partition". Two targets, and on this
/// hardware they are not the same *kind* of target — which is exactly the fact
/// constraint 2 says must not be left implicit.
///
/// Constructed by the tests and by nothing else until something writes a
/// payload; the update path is that thing and it does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPayload {
    /// The kernel and the firmware-read boot files for this slot.
    pub kernel: PayloadTarget,
    /// The `/system` filesystem image for this slot.
    pub system: PayloadTarget,
}

impl SlotPayload {
    /// The target a payload member's `target_class` names.
    ///
    /// `ADR-029` §3: *"**`target_class` is `PayloadTarget`** — deliberately the
    /// same two cases in the same order."* It cannot literally be this type,
    /// because `rivet-core` may not depend on an adapter (`ADR-014` §3) and the
    /// format has to be readable by a host tool that has no boot port at all. So
    /// there are two enumerations, and **this function is the one place they are
    /// tied together** — an installer that matched on the class itself would be a
    /// third spelling of the same correspondence (`ADR-011`).
    ///
    /// The drift guard is in this module's tests, where both types are visible.
    #[must_use]
    pub fn target_for(&self, class: TargetClass) -> &PayloadTarget {
        match class {
            TargetClass::WholePartition => &self.system,
            TargetClass::BootDirectory => &self.kernel,
        }
    }
}

/// One place a payload goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadTarget {
    /// A whole partition, replaced as a filesystem image.
    ///
    /// `ADR-018` §4's `system-a` / `system-b`.
    WholePartition { device: String },
    /// Files in a directory on a filesystem, which may be shared with the other
    /// slot.
    ///
    /// The Pi's boot half — each slot has its own FAT partition and the payload
    /// is *files* on it, not an image over it, because the firmware also reads
    /// `config.txt`, `cmdline.txt` and the device trees from there
    /// (`ADR-002` §4). It is also the shape the boot-slot port (x86 has no `tryboot`; ruled 2026-08-23) expects
    /// x86 to need for **both** halves: one ESP holding `/EFI/rivet/a/` and
    /// `/EFI/rivet/b/`, because UEFI finds the ESP by partition type GUID and
    /// two ESPs is legal but unreliable.
    DirectoryOnPartition {
        device: String,
        fstype: &'static str,
        path: String,
    },
}

/// What a commit turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Committed {
    /// The record already named this slot; nothing was written.
    AlreadyDefault,
    /// The record was moved to this slot.
    Changed,
}

/// Which reboot carries an arming, once [`BootSlots::arm_trial_boot`] has
/// recorded it.
///
/// **This is the port's own vocabulary and deliberately not the caller's.**
/// `cmd/rivet-initd`'s `Kernel` has a `RebootHow` with a `TrialRestart` member,
/// and the obvious move was to return that. It was rejected, and the reason is
/// worth keeping:
///
/// * `RebootHow` also carries `PowerOff` and `Restart`, which are PID 1's
///   *shutdown* handling (`ADR-017` §3.1.5 item 6) and have nothing to do with
///   images. Moving the enum here would put PID 1's shutdown vocabulary inside
///   the image adapter, so that a process which never touches an update would
///   still name this crate to ask for a power off. `ADR-014` §3 permits the
///   edge — `cmd/*` may depend on everything — which is precisely why the graph
///   would not catch it.
/// * The enum cannot be split: `TrialRestart` alone is not a type.
/// * `cmd/rivet-initd`'s own `kernel` module already states the rule this would
///   break — the trial reboot "lives on this port rather than beside the update
///   code … because it is a syscall: everything below this seam is". A reboot is
///   a syscall on every platform this port is meant to reach.
///
/// So the port says *what kind of reboot the arming needs* and the caller maps
/// it onto its own syscall vocabulary. `ADR-009` §4's rule — reuse
/// implementations, never interfaces — one level over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedReboot {
    /// The arming is already recorded somewhere the firmware will read, so an
    /// **ordinary** reboot picks it up.
    ///
    /// UEFI `BootNext`'s answer — the variable is written at arm time and the
    /// firmware deletes it before attempting the boot it names.
    /// **Nothing returns this today**, and it is
    /// carried for the same reason [`SlotRecord::FirmwareVariable`] is: a
    /// one-variant enum has abstracted nothing, and the whole point of this type
    /// is that the two platforms differ.
    Ordinary,
    /// The one-shot lives **in the reboot call itself**, so an ordinary reboot
    /// would boot the default slot and trial nothing.
    ///
    /// The Pi's answer: `ADR-018` §5.1's `reboot "0 tryboot"`, which
    /// `cmd/rivet-initd` makes as `reboot(2)` with
    /// `LINUX_REBOOT_CMD_RESTART2`. Arming has written the `[tryboot]` override;
    /// this is what makes the firmware consult it.
    OneShot,
}

impl core::fmt::Display for ArmedReboot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ordinary => write!(f, "an ordinary reboot"),
            Self::OneShot => write!(f, "a one-shot trial reboot"),
        }
    }
}

/// The A/B boot primitive, as a port.
///
/// Object-safe on purpose, for the same reason [`BootMedium`] is: the decisions
/// above it are compiled once and the tests drive exactly the code a board runs.
///
/// Every method takes `&mut dyn BootMedium` rather than owning one, because an
/// implementor of this trait is a *policy about the firmware*, not a second
/// route to the system. `AGENTS.md` §10.1 — the seam that already exists is the
/// one to reuse.
pub trait BootSlots {
    /// What this implementor is, for the console. `AGENTS.md` §5: a board's
    /// only diagnostic is what PID 1 printed.
    fn platform(&self) -> &'static str;

    /// `ADR-027` §3's `<platform>` token — `rpi4`, `rpizw`, `uefi`.
    ///
    /// **Not a shorter [`Self::platform`], and the two must not be merged.**
    /// §3.1's table is the distinction: this one answers *will this firmware
    /// boot it*, and `ADR-029` §9.2 step 2 compares it against a signed header
    /// before anything is written. The other is prose for a console line and a
    /// reword of it must not refuse a payload.
    ///
    /// It is on this port rather than a constant in the installer because
    /// §3.1 says *"`<platform>` names the boot contract, not the chassis"* —
    /// and the boot contract is exactly what a `BootSlots` implementor is. A
    /// UEFI implementor answers `uefi` whatever the machine is.
    fn platform_token(&self) -> &'static str;

    /// Which slot the machine is running, as the firmware reports it.
    fn running_slot(&self, m: &mut dyn BootMedium) -> Result<SlotId, String>;

    /// The slot an update writes: the one that is not `slot`.
    ///
    /// No caller until there is an update; see the module docs.
    fn other_slot(&self, slot: SlotId) -> Result<SlotId, String>;

    /// Is this a one-shot trial boot?
    ///
    /// **Yes or no, not a count.** `tryboot` and `BootNext` are both one-shot;
    /// systemd-boot and U-Boot count down instead, per the boot-slot port's
    /// own survey, and if either is ever adopted this becomes a decision rather than a
    /// widening — a count is only visible to an implementor that has one.
    fn in_trial_boot(&self, m: &mut dyn BootMedium) -> Result<bool, String>;

    /// **Which slot an ordinary reset boots.** Added 2026-09-05.
    ///
    /// The companion to [`Self::in_trial_boot`], and the reason it exists is
    /// that the two are routinely confused: that one is *how this boot started*
    /// and this one is *what happens next time*. On a committed trial they
    /// disagree, which is precisely when somebody is reading them.
    ///
    /// Behind the port because the record is a file on a FAT partition here and
    /// an EFI variable elsewhere — [`Self::default_slot_record`]'s reasoning,
    /// one operation over.
    fn default_slot(&self, m: &mut dyn BootMedium) -> Result<SlotId, String>;

    /// Where the default-slot record lives. Constraint 1.
    ///
    /// No caller until there is an update; see the module docs.
    fn default_slot_record(&self) -> SlotRecord;

    /// Where `slot`'s payload goes. Constraint 2.
    ///
    /// No caller until there is an update; see the module docs.
    fn payload(&self, slot: SlotId) -> Result<SlotPayload, String>;

    /// What this platform reserves on `target` — the names an update payload may
    /// not carry. `ADR-029` §4.
    ///
    /// # Why this is the port's operation and not the installer's
    ///
    /// `ADR-029` §4 says it in one sentence: *"on the Pi that is a pair of
    /// filenames; on a UEFI machine the equivalent object is an EFI variable and
    /// is not a file at all, which is exactly why it belongs behind the port."*
    /// An installer that carried the list would be carrying the Raspberry Pi's
    /// firmware contract into code that is supposed to work on both.
    ///
    /// # What the Pi reserves, and the conjunction that makes it necessary
    ///
    /// Three facts, each innocuous alone:
    ///
    /// 1. [`rivet_core::state::AUTOBOOT_PARTITION`] is **1**, and
    ///    [`PiFirmware::default_slot_record`] returns `p1` whichever slot is
    ///    running — proven on hardware 2026-08-23.
    /// 2. `PiFirmware::payload(SlotId(1))`'s boot half is
    ///    [`PayloadTarget::DirectoryOnPartition`] on that same `p1`.
    /// 3. `cmd/rivet-image` writes `autoboot.txt` onto **both** boot
    ///    partitions when it builds a card.
    ///
    /// So **when slot A is the inactive slot, the payload's boot target is the
    /// filesystem holding the only record of which slot the firmware boots.** A
    /// payload carrying `autoboot.txt` would, on an update to slot A, overwrite
    /// the commit record with a fresh-card default, and every subsequent boot
    /// would go to slot A — a machine whose default was silently reset by an
    /// update that reported success.
    ///
    /// `rivet.toml` is reserved for a second reason of the same class:
    /// `ADR-004` §6 has PID 1 *mutate* it, and `ADR-018` §8 makes it the recovery
    /// path for a damaged `/state`. An update restoring the blank template
    /// destroys both.
    ///
    /// **The caller refuses the whole payload, not the member** — `ADR-029` §4.
    /// Silently skipping would let a future builder believe it had shipped
    /// something it had not.
    ///
    /// `&'static [&'static str]` rather than a `Vec`: the list is a property of
    /// the platform, so there is nothing to allocate and nothing that can fail.
    /// A platform whose reserved object is not a file at all returns the empty
    /// slice here and refuses it some other way, which is honest — the empty
    /// slice says *no name is reserved*, not *nothing is protected*.
    fn reserved_names(&self, target: &PayloadTarget) -> &'static [&'static str];

    /// Make `slot` the default from the next boot on.
    ///
    /// This is PID 1's item 5 (`ADR-017` §3.1.5) and it must be safe to
    /// interrupt: a power cut in the middle leaves the previous default intact,
    /// never a record that names nothing.
    ///
    /// The refusal is typed so the caller can tell a medium that refused
    /// ([`ArmRefusal::Medium`], worth another attempt) from a record this
    /// implementor will not rewrite ([`ArmRefusal::Unsafe`], which no attempt
    /// changes). **Every attempt takes the record's lock and mounts afresh**,
    /// so calling this again after a medium refusal — another process holding
    /// the lock is one — is a retry with a new mount.
    fn commit(&self, m: &mut dyn BootMedium, slot: SlotId) -> Result<Committed, ArmRefusal>;

    /// What is on the medium holding the default-slot record: at most `limit`
    /// entries of its root directory, each with its size.
    ///
    /// **Diagnostic only**, for a commit that failed: the console line then
    /// says what the partition actually held rather than what it was expected
    /// to. The medium is mounted **read-only** for the listing, under the
    /// record's lock, and released again, so taking one cannot change what the
    /// next attempt finds, and a
    /// listing that fails is the caller's to report and never a reason to stop.
    fn record_medium_listing(
        &self,
        m: &mut dyn BootMedium,
        limit: usize,
    ) -> Result<Listing, String>;

    /// Make the **next** boot a one-shot trial of `slot`.
    ///
    /// Returns how the reboot must then be requested, because that is precisely
    /// where the platforms differ and it is not the caller's to know: on the Pi
    /// the one-shot lives *in the reboot call* ([`ArmedReboot::OneShot`]), and
    /// on a UEFI machine it lives in NVRAM and the reboot is an ordinary one.
    /// Splitting it this way keeps the reboot itself on the caller's own syscall
    /// seam, where every other syscall is.
    ///
    /// **This does not reboot.** A port operation that stopped the machine
    /// could not be tested and could not be undone — and there is no reboot on
    /// [`BootMedium`] at all, so it could not be done accidentally either.
    ///
    /// **Two safety interlocks refuse before anything is written**, and the
    /// module documentation carries both with their evidence: `tryboot_a_b=1`
    /// must actually apply to this card, and the slot may not be the one this
    /// machine is running.
    ///
    /// ~~No caller until there is an update.~~ **`cmd/rivet` calls it since
    /// 2026-08-23**, through `rivet_core::operation::trial`.
    fn arm_trial_boot(
        &self,
        m: &mut dyn BootMedium,
        slot: SlotId,
    ) -> Result<ArmedReboot, ArmRefusal>;

    /// Everything [`Self::arm_trial_boot`] checks, **writing nothing**.
    ///
    /// `ADR-006` §6.2: *"Every mutating operation supports a dry run."* §6.5's
    /// framework — a type that computes its diff separately from applying it —
    /// does not exist, so this is that property supplied by hand for the one
    /// operation that needs it, and it is honest about being that rather than a
    /// framework (`AGENTS.md` §3: a stub that looks further along than it is).
    ///
    /// The value is not tidiness. Confirmation happens in a *second* invocation
    /// (`ADR-012` §7.1, and this binary has no prompt — `ADR-005` §5.1), so a
    /// plan that had not looked at the card would show an operator a change,
    /// take their confirmation, and only then discover the card cannot carry it.
    fn can_arm_trial_boot(&self, m: &mut dyn BootMedium, slot: SlotId) -> Result<(), ArmRefusal>;
}

// ---------------------------------------------------------------------------
// The Raspberry Pi implementor.
// ---------------------------------------------------------------------------

/// `ADR-018` §5's mechanism on Raspberry Pi firmware.
///
/// Stateless: every fact it needs is read from the firmware at the point of use
/// (`AGENTS.md` §5 — the kernel is the authority for system state, never cache
/// its verdicts), and the two that are not read are constants the record owns.
pub struct PiFirmware;

impl BootSlots for PiFirmware {
    fn platform(&self) -> &'static str {
        "Raspberry Pi firmware (tryboot, ADR-018 §5)"
    }

    /// `ADR-027` §3's examples spell it `rpi4`, and `ADR-029` §3's header layout
    /// gives the field sixteen bytes.
    ///
    /// **The Pi tokens are board names only because on the Pi the board *is* the
    /// contract** — firmware blobs, DTB and `ADR-018` §5's `tryboot`
    /// (`ADR-027` §3.1). A Pi Zero W would be a different implementor answering
    /// `rpizw`, not this one answering conditionally.
    fn platform_token(&self) -> &'static str {
        "rpi4"
    }

    fn running_slot(&self, m: &mut dyn BootMedium) -> Result<SlotId, String> {
        boot_partition_device(m).map(|(n, _)| SlotId(n))
    }

    fn other_slot(&self, slot: SlotId) -> Result<SlotId, String> {
        Ok(SlotId(known_slot(slot)?.other().boot_partition()))
    }

    fn in_trial_boot(&self, m: &mut dyn BootMedium) -> Result<bool, String> {
        let bytes = m
            .read_property(DT_TRYBOOT)
            .map_err(|e| format!("cannot read {DT_TRYBOOT}: {e}"))?;
        let flag = parse_dt_u32(&bytes).map_err(|why| format!("{DT_TRYBOOT}: {why}"))?;
        Ok(flag != 0)
    }

    fn default_slot(&self, m: &mut dyn BootMedium) -> Result<SlotId, String> {
        // The same mount-read-unmount as `read_autoboot`, and it takes the
        // partition READ-ONLY: this answers a question and changes nothing, and
        // a reader that mounted the authoritative boot partition writable to
        // look at it would be one power cut away from a card that does not boot.
        let text = read_autoboot(m).map_err(|r| r.to_string())?;
        default_boot_partition(&text).map(SlotId)
    }

    fn default_slot_record(&self) -> SlotRecord {
        SlotRecord::FileOnPartition {
            device: autoboot_partition_device(),
            fstype: BOOT_FSTYPE,
            file: AUTOBOOT_FILE,
        }
    }

    fn payload(&self, slot: SlotId) -> Result<SlotPayload, String> {
        let slot = known_slot(slot)?;
        Ok(SlotPayload {
            // Files on the slot's own FAT partition, not an image over it: the
            // firmware reads config.txt, cmdline.txt and the device trees from
            // the same filesystem (`ADR-002` §4), and an update replaces the
            // kernel rather than the partition.
            kernel: PayloadTarget::DirectoryOnPartition {
                device: partition_device(BOOT_DISK, slot.boot_partition()),
                fstype: BOOT_FSTYPE,
                path: "/".to_string(),
            },
            // A whole partition, because `/system` is read-only by format
            // (`ADR-018` §3) and there is nothing on it to preserve.
            system: PayloadTarget::WholePartition {
                device: partition_device(BOOT_DISK, slot.system_partition()),
            },
        })
    }

    fn reserved_names(&self, target: &PayloadTarget) -> &'static [&'static str] {
        match target {
            // A whole partition is replaced as an image and has no member names
            // on it at all — there is nothing here to reserve. `/system` is
            // read-only by format (`ADR-018` §3) and an update replaces it
            // wholesale, so nothing on it survives to be overwritten by accident.
            PayloadTarget::WholePartition { .. } => &[],
            // **Not conditioned on which slot this target belongs to**, and that
            // is deliberate. A payload is built once and may be installed into
            // either slot; a list that depended on the target would make a
            // payload legal for B and illegal for A, so a builder would produce
            // one that passed on the bench and refused itself in the field.
            // Reserving both names on every boot-directory target costs the
            // ability to ship two files nothing should ship.
            PayloadTarget::DirectoryOnPartition { .. } => {
                &[AUTOBOOT_FILE, crate::firmware::PROVISIONING_FILE]
            }
        }
    }

    /// **Deliberately does not check the slot against `ADR-018` §4's table**,
    /// where [`Self::arm_trial_boot`] does. The asymmetry is not an oversight:
    /// a commit writes the slot the firmware *says it booted*, which is an
    /// observation, and refusing to record an observation because it is
    /// surprising would leave a running machine unable to make itself the
    /// default. Arming writes a slot somebody *chose*, and a choice outside the
    /// layout is a card that boots into nothing. This also keeps the behaviour
    /// a board corrected on 2026-08-23 exactly as it was.
    fn commit(&self, m: &mut dyn BootMedium, slot: SlotId) -> Result<Committed, ArmRefusal> {
        // `edit_autoboot` mounts, writes and unmounts on every call, so a second
        // call after a refusal starts from a fresh mount. A leftover
        // `AUTOBOOT_TEMP` from a failed attempt does not break it either:
        // `BootMedium::write_file` creates or truncates.
        edit_autoboot(m, |text| rewrite_autoboot_default(text, slot.0))
    }

    fn record_medium_listing(
        &self,
        m: &mut dyn BootMedium,
        limit: usize,
    ) -> Result<Listing, String> {
        with_autoboot_partition(m, false, |m| {
            m.list_dir(AUTOBOOT_PARTITION_MOUNT, limit).map_err(|e| {
                ArmRefusal::Medium(format!("cannot list {AUTOBOOT_PARTITION_MOUNT}: {e}"))
            })
        })
        .map_err(|refusal| refusal.to_string())
    }

    fn arm_trial_boot(
        &self,
        m: &mut dyn BootMedium,
        slot: SlotId,
    ) -> Result<ArmedReboot, ArmRefusal> {
        self.interlocks(m, slot)?;
        // The `tryboot_a_b` interlock happens *inside* the edit rather than
        // beside it, against the text that is about to be rewritten. Reading the
        // file twice — once to check, once to change — would leave a window in
        // which the two disagreed, on the one file whose corruption cannot be
        // repaired over the network.
        edit_autoboot(m, |text| {
            require_tryboot_a_b(text)?;
            rewrite_autoboot_tryboot(text, slot.0)
        })?;
        Ok(ArmedReboot::OneShot)
    }

    fn can_arm_trial_boot(&self, m: &mut dyn BootMedium, slot: SlotId) -> Result<(), ArmRefusal> {
        self.interlocks(m, slot)?;
        let text = read_autoboot(m)?;
        let path = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}");
        require_tryboot_a_b(&text).map_err(ArmRefusal::Unsafe)?;
        // And the file has to be one the rewrite recognises, for the same reason
        // this method exists at all: otherwise the plan would be shown, the
        // operator would confirm it, and *then* the file would be refused.
        rewrite_autoboot_tryboot(&text, slot.0)
            .map(|_| ())
            .map_err(|why| ArmRefusal::Unsafe(format!("{path}: {why}")))
    }
}

impl PiFirmware {
    /// The two interlocks that do not need the file's contents, in the order
    /// that touches least: a slot outside the layout is refused before the
    /// firmware is read, and the firmware is read before anything is mounted.
    fn interlocks(&self, m: &mut dyn BootMedium, slot: SlotId) -> Result<(), ArmRefusal> {
        // Refuse a slot this layout does not contain before touching anything:
        // `[tryboot] boot_partition=9` is a card that boots into nothing.
        known_slot(slot).map_err(ArmRefusal::Unsafe)?;
        let running = self.running_slot(m).map_err(ArmRefusal::Medium)?;
        refuse_the_running_slot(running, slot)
    }
}

/// Interlock 2: a trial of the slot already running is a no-op reboot.
///
/// **The failure direction is what makes this worth a fuse rather than a note.**
/// The machine would restart, come back byte-identical, pass every health check
/// it has, and PID 1 would commit it — so a broken update would be reported as
/// an update that worked. `ADR-018` §5.1 records the slot-B commit bug failing
/// in exactly that direction and says why it would have run for months.
fn refuse_the_running_slot(running: SlotId, slot: SlotId) -> Result<(), ArmRefusal> {
    if running != slot {
        return Ok(());
    }
    // "The other slot" is never arithmetic above this port (see `SlotId`), so it
    // is asked of the layout — and if the firmware named a partition the layout
    // does not contain, the advice is dropped rather than invented.
    let advice = match Slot::from_boot_partition(running.0) {
        Some(known) => format!(
            "Arm slot {} instead — that is the slot an update writes.",
            known.other().boot_partition()
        ),
        None => "Arm the other slot instead.".to_string(),
    };
    Err(ArmRefusal::Unsafe(format!(
        "this machine is running boot partition {running}, and a trial boot of the slot \
         that is already running changes nothing: the board would restart, come back \
         identical, pass every check it has, and be committed — an update that did \
         nothing, reported as one that worked. {advice}"
    )))
}

/// Interlock 1: refuse unless `tryboot_a_b=1` applies to every boot.
///
/// **`ADR-018` §5.3's trap, as a fuse.** Without this setting the firmware reads
/// `tryboot.txt` instead of `config.txt` on the trial boot, and §5.3 measured
/// what that costs: a `config.txt` carrying `arm_64bit=1` with no 32-bit kernel
/// present *"would have attempted a 32-bit boot and lost the machine"* — on a
/// headless board, with no console and no way back.
///
/// **A setting inside a conditional section does not count**, for
/// [`rewrite_autoboot_tryboot`]'s reason one direction over: `config.txt` syntax
/// applies a setting before the first `[section]` header, or under `[all]`, to
/// every boot, and anything else only sometimes. "Sometimes" is not a guarantee
/// this may act on, and the refusal says which case it found so the operator can
/// tell an absent setting from a misplaced one.
///
/// Refuses two settings rather than choosing between them, exactly as the two
/// rewrites do: a file with two answers is a file this crate does not
/// understand, and understanding it wrongly is unrecoverable.
/// **The DEFAULT boot partition** — what an ordinary reset boots — read out of
/// `autoboot.txt`'s unconditional section.
///
/// Added 2026-09-05. `BootSlots::in_trial_boot` says how a boot was *entered*
/// and cannot say whether it has since been committed, because committing is
/// `PiFirmware::commit` rewriting exactly this value while the machine runs.
/// Three readers have been misled by the difference —
/// `rivet_core::operation::machine::SlotRead::default_slot` carries that
/// history. This is the other half of the comparison.
///
/// **Unconditional means the same thing here as in [`require_tryboot_a_b`]**,
/// and for the same reason: `config.txt` syntax applies a setting before the
/// first `[section]` header, or under `[all]`, to every boot. A
/// `boot_partition` under `[tryboot]` is the ONE-SHOT and is emphatically not
/// this — reading it as the default would answer the exact question wrongly, in
/// the exact situation the caller is trying to resolve.
///
/// Two unconditional answers is a refusal rather than a choice, for that
/// function's reason: a file with two answers is one this crate does not
/// understand.
pub fn default_boot_partition(text: &str) -> Result<u32, String> {
    let mut unconditional = true;
    let mut found: Vec<u32> = Vec::new();

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            unconditional = trimmed.eq_ignore_ascii_case("[all]");
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("boot_partition") || !unconditional {
            continue;
        }
        match value.trim().parse::<u32>() {
            Ok(n) => found.push(n),
            Err(_) => {
                return Err(format!(
                    "autoboot.txt has an unconditional boot_partition={} that is not a number",
                    value.trim()
                ));
            }
        }
    }

    match found.as_slice() {
        [one] => Ok(*one),
        [] => Err(
            "autoboot.txt names no unconditional boot_partition, so which slot an ordinary \
             restart boots is not stated in the file that decides it"
                .to_string(),
        ),
        many => Err(format!(
            "autoboot.txt has {} unconditional boot_partition settings and this crate will not \
             choose between them",
            many.len()
        )),
    }
}

pub fn require_tryboot_a_b(text: &str) -> Result<(), String> {
    let mut unconditional = true;
    let mut applies: Vec<Option<u32>> = Vec::new();
    let mut conditional = 0usize;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            unconditional = trimmed.eq_ignore_ascii_case("[all]");
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case(TRYBOOT_A_B) {
            continue;
        }
        if unconditional {
            applies.push(value.trim().parse::<u32>().ok());
        } else {
            conditional = conditional.saturating_add(1);
        }
    }

    let trap = "without it the firmware reads tryboot.txt instead of config.txt on the trial \
                boot, and ADR-018 §5.3 measured what that costs on a card whose config.txt \
                carries arm_64bit=1 with no 32-bit kernel beside it: a headless board that \
                does not come back";
    match applies.as_slice() {
        [Some(1)] => Ok(()),
        [Some(other)] => Err(format!(
            "`{TRYBOOT_A_B}={other}` — a trial boot may only be armed on a card where it \
             is 1, because {trap}. Set `{TRYBOOT_A_B}=1` under [all] and arm again."
        )),
        [None] => Err(format!(
            "`{TRYBOOT_A_B}` is set to something that is not a number; refusing to guess \
             what the firmware makes of it, because {trap}."
        )),
        [] if conditional > 0 => Err(format!(
            "`{TRYBOOT_A_B}` is set only inside a conditional section, so it does not \
             apply to every boot — and {trap}. Move `{TRYBOOT_A_B}=1` under [all], which \
             is where cmd/rivet-image writes it for a fresh card, and arm again."
        )),
        [] => Err(format!(
            "no `{TRYBOOT_A_B}=1` applies unconditionally in this {AUTOBOOT_FILE}, and \
             {trap}. Add `{TRYBOOT_A_B}=1` under [all], which is where cmd/rivet-image \
             writes it for a fresh card, and arm again."
        )),
        many => Err(format!(
            "{} unconditional `{TRYBOOT_A_B}` settings; refusing to guess which one the \
             firmware uses, because {trap}.",
            many.len()
        )),
    }
}

/// Read the authoritative `autoboot.txt` without mounting it writable.
///
/// The read-only half of [`edit_autoboot`], for [`BootSlots::can_arm_trial_boot`]
/// — a dry run that mounted the card writable would have already done the one
/// thing it exists to avoid.
fn read_autoboot(m: &mut dyn BootMedium) -> Result<String, ArmRefusal> {
    with_autoboot_partition(m, false, |m| {
        let path = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}");
        m.read_file(&path)
            .map_err(|e| ArmRefusal::Medium(format!("cannot read {path}: {e}")))
    })
}

/// **Every access to the default-slot record's partition has this shape**:
/// take [`AUTOBOOT_LOCK`], mount — reclaiming an orphaned mount found there
/// (see [`mount_autoboot_partition`]) — do `work`, unmount, release the lock.
///
/// The lock is what stops two processes interleaving on one mount point: PID
/// 1 committing a trial boot while `rivet get slot` reads the default, where
/// the reader's unmount landed between PID 1's write and its rename and the
/// rename failed with `ENOENT`. A lock that cannot be taken is a
/// [`ArmRefusal::Medium`] refusal and nothing is mounted — another process
/// holding it is exactly a refusal a later attempt can answer differently.
fn with_autoboot_partition<T>(
    m: &mut dyn BootMedium,
    writable: bool,
    work: impl FnOnce(&mut dyn BootMedium) -> Result<T, ArmRefusal>,
) -> Result<T, ArmRefusal> {
    m.lock(AUTOBOOT_LOCK).map_err(|e| {
        ArmRefusal::Medium(format!(
            "cannot take {AUTOBOOT_LOCK}, so the partition holding {AUTOBOOT_FILE} was not \
             mounted: {e}"
        ))
    })?;
    let outcome = match mount_autoboot_partition(m, writable) {
        Ok(mount) => {
            let outcome = work(m);
            umount_autoboot_partition(m, mount);
            outcome
        }
        Err(why) => Err(ArmRefusal::Medium(why)),
    };
    m.unlock(AUTOBOOT_LOCK);
    outcome
}

/// Which of `ADR-018` §4's two slots a firmware-reported number is.
///
/// Refuses rather than defaulting. A number outside the layout means the card
/// is not the one `ADR-018` §4 describes, and every use below would otherwise
/// name a partition an update would overwrite.
pub(crate) fn known_slot(slot: SlotId) -> Result<Slot, String> {
    Slot::from_boot_partition(slot.0).ok_or_else(|| {
        format!(
            "partition {slot} is neither of ADR-018 §4's two boot partitions \
             ({} and {}); refusing to act on a card this layout does not describe",
            Slot::A.boot_partition(),
            Slot::B.boot_partition()
        )
    })
}

/// Mount the authoritative boot partition, rewrite `autoboot.txt`, release it.
///
/// **Both callers go through here**, so the write-sync-rename-sync order and
/// the release-whatever-happened posture ([`with_autoboot_partition`]) are
/// stated once. Reached by
/// [`PiFirmware::commit`] and [`PiFirmware::arm_trial_boot`], which differ only
/// in which line of the file they move.
///
/// The partition is `rivet_core::state::AUTOBOOT_PARTITION`'s and never the
/// booted one, and it is mounted at its own mount point so that no convergent
/// "already mounted" can quietly substitute the other filesystem — see
/// [`crate::firmware::AUTOBOOT_PARTITION_MOUNT`].
fn edit_autoboot(
    m: &mut dyn BootMedium,
    rewrite: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<Committed, ArmRefusal> {
    with_autoboot_partition(m, true, |m| {
        let path = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}");
        let temp = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_TEMP}");
        write_autoboot(m, &path, &temp, rewrite)
    })
}

/// **The classification lives here and nowhere else**: everything the *medium*
/// refused is [`ArmRefusal::Medium`], and everything the *file* refused —
/// whatever the closure said — is [`ArmRefusal::Unsafe`], because in both cases
/// nothing was written and retrying changes nothing.
fn write_autoboot(
    m: &mut dyn BootMedium,
    path: &str,
    temp: &str,
    rewrite: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<Committed, ArmRefusal> {
    let text = m
        .read_file(path)
        .map_err(|e| ArmRefusal::Medium(format!("cannot read {path}: {e}")))?;
    let replacement = match rewrite(&text) {
        Ok(None) => return Ok(Committed::AlreadyDefault),
        Ok(Some(text)) => text,
        Err(why) => return Err(ArmRefusal::Unsafe(format!("{path}: {why}"))),
    };
    m.write_file(temp, replacement.as_bytes())
        .map_err(|e| ArmRefusal::Medium(format!("cannot write {temp}: {e}")))?;
    // Sync before the rename so the new contents are on the card before
    // anything points at them, and again after so the rename itself is.
    m.sync();
    m.rename(temp, path)
        .map_err(|e| ArmRefusal::Medium(format!("cannot rename {temp} over {path}: {e}")))?;
    m.sync();
    Ok(Committed::Changed)
}

// ---------------------------------------------------------------------------
// `autoboot.txt`, which is `config.txt` syntax.
//
// Settings before any `[section]` header apply to all, and a `[section]` header
// applies until the next one. Line endings are preserved exactly — the file is
// edited from Windows (`ADR-002` §4.1), so CRLF is the expected case, not the
// exotic one.
//
// Both functions **refuse a file they do not recognise** rather than rewriting
// it hopefully: a corrupted `autoboot.txt` on a headless board is not
// recoverable over the network.
// ---------------------------------------------------------------------------

/// Move `boot_partition=` under `[all]` to `partition`.
///
/// * `Ok(None)` — it already says that; nothing to write.
/// * `Ok(Some(text))` — the new file, byte-identical except for that one value.
/// * `Err` — the file is not the shape `ADR-018` §5.1 describes.
///
/// **Moved from `cmd/rivet-initd::confirm` on 2026-08-23, and into this crate
/// later the same day, body unchanged both times.** It was corrected on hardware
/// earlier that day and its behaviour is not either move's to touch.
/// `cmd/rivet-image/src/boot.rs` vendors this function verbatim and a drift
/// guard there compares the two texts every cycle, so reformatting it fails that
/// suite — which is the point of the guard and the reason its sibling below is a
/// second function rather than a shared one.
pub fn rewrite_autoboot_default(text: &str, partition: u32) -> Result<Option<String>, String> {
    let mut in_all = true;
    let mut found = 0usize;
    let mut existing: Option<u32> = None;
    let mut out = String::with_capacity(text.len() + 8);

    for line in text.split_inclusive('\n') {
        let core = line.trim_end_matches(['\r', '\n']);
        let eol = line.get(core.len()..).unwrap_or("");
        let trimmed = core.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_all = trimmed.eq_ignore_ascii_case("[all]");
            out.push_str(line);
            continue;
        }
        if in_all
            && let Some((key, value)) = trimmed.split_once('=')
            && key.trim().eq_ignore_ascii_case("boot_partition")
        {
            found = found.saturating_add(1);
            existing = value.trim().parse::<u32>().ok();
            out.push_str(&format!("boot_partition={partition}{eol}"));
            continue;
        }
        out.push_str(line);
    }

    match found {
        0 => Err(format!(
            "no `boot_partition=` setting outside a conditional section; \
             this is not the {AUTOBOOT_FILE} ADR-018 §5.1 describes"
        )),
        1 => {
            if existing == Some(partition) {
                Ok(None)
            } else {
                Ok(Some(out))
            }
        }
        n => Err(format!(
            "{n} `boot_partition=` settings apply unconditionally; refusing to guess \
             which one the firmware uses"
        )),
    }
}

/// Move `boot_partition=` under `[tryboot]` to `partition`.
///
/// The same shape as [`rewrite_autoboot_default`] and deliberately a second
/// function rather than a shared one with a section argument: that function is
/// vendored verbatim into `cmd/rivet-image/src/boot.rs` behind a text-equality
/// drift guard, so generalising it would mean editing another crate's copy in
/// the same change. Merging the two is worth doing once that guard is updated —
/// it is named in the report that introduced this file.
///
/// **A setting before the first section header does NOT count here**, and that
/// is the one real difference. In `config.txt` syntax such a setting applies
/// unconditionally, which is exactly what makes it *not* the `[tryboot]`
/// override — it would apply to a normal boot as well, and moving it would
/// change the default slot while claiming to arm a trial.
///
/// **What this is for.** After a commit,
/// `[all]` and `[tryboot]` name the same partition, so the next update's trial
/// boot would try the *running* slot. Writing the override at arm time is what
/// closes that, and it is the same operation as UEFI's `BootNext = <entry>`.
/// **Untested against firmware in either direction:** `ADR-018` §5.3's second
/// open item is that a `[tryboot]` section selects a different `boot_partition`
/// at all — the two boots that settled `[all]` never used tryboot, so the
/// override has only ever been observed being left alone.
pub fn rewrite_autoboot_tryboot(text: &str, partition: u32) -> Result<Option<String>, String> {
    let mut in_tryboot = false;
    let mut found = 0usize;
    let mut existing: Option<u32> = None;
    let mut out = String::with_capacity(text.len() + 8);

    for line in text.split_inclusive('\n') {
        let core = line.trim_end_matches(['\r', '\n']);
        let eol = line.get(core.len()..).unwrap_or("");
        let trimmed = core.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_tryboot = trimmed.eq_ignore_ascii_case("[tryboot]");
            out.push_str(line);
            continue;
        }
        if in_tryboot
            && let Some((key, value)) = trimmed.split_once('=')
            && key.trim().eq_ignore_ascii_case("boot_partition")
        {
            found = found.saturating_add(1);
            existing = value.trim().parse::<u32>().ok();
            out.push_str(&format!("boot_partition={partition}{eol}"));
            continue;
        }
        out.push_str(line);
    }

    match found {
        0 => Err(format!(
            "no `boot_partition=` setting under `[tryboot]`; \
             this is not the {AUTOBOOT_FILE} ADR-018 §5.1 describes"
        )),
        1 => {
            if existing == Some(partition) {
                Ok(None)
            } else {
                Ok(Some(out))
            }
        }
        n => Err(format!(
            "{n} `boot_partition=` settings apply under `[tryboot]`; refusing to guess \
             which one the firmware uses"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::DT_BOOT_PARTITION;
    use crate::port::Errno;
    use crate::testio::{FakeMedium, Script};

    /// The file `cmd/rivet-image` writes for a fresh card, in shape.
    const AUTOBOOT: &str =
        "[all]\ntryboot_a_b=1\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n";
    const AUTOBOOT_CRLF: &str = "[all]\r\nboot_partition=1\r\n[tryboot]\r\nboot_partition=2\r\n";

    fn card(autoboot: Script<Result<String, Errno>>) -> FakeMedium {
        card_running(1, autoboot)
    }

    /// A card whose firmware reports `running` as the booted partition.
    ///
    /// The running slot stopped being incidental on 2026-08-23, when arming grew
    /// the interlock that refuses it — so the arming tests below name it rather
    /// than inheriting `card`'s `1`.
    fn card_running(running: u32, autoboot: Script<Result<String, Errno>>) -> FakeMedium {
        let mut m = FakeMedium::new();
        m.properties_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(running.to_string().into_bytes())),
        );
        m.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
            autoboot,
        );
        m
    }

    // -- reading the firmware -------------------------------------------------

    #[test]
    fn the_running_slot_is_what_the_firmware_said_and_not_an_assumption() {
        let mut m = FakeMedium::new();
        m.properties_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 2])),
        );
        assert_eq!(PiFirmware.running_slot(&mut m), Ok(SlotId(2)));
    }

    /// Under QEMU there is no `/proc/device-tree/chosen/bootloader` at all
    /// (`ADR-026` §4), so every one of these reads must refuse rather than
    /// answer.
    #[test]
    fn with_no_device_tree_nothing_can_be_answered() {
        let mut m = FakeMedium::new();
        assert!(PiFirmware.running_slot(&mut m).is_err());
        assert!(PiFirmware.in_trial_boot(&mut m).is_err());
    }

    #[test]
    fn the_trial_flag_is_read_from_the_firmware_in_both_of_its_encodings() {
        for (bytes, expected) in [
            (b"0".to_vec(), false),
            (b"1".to_vec(), true),
            (vec![0, 0, 0, 0], false),
            (vec![0, 0, 0, 1], true),
        ] {
            let mut m = FakeMedium::new();
            m.properties_mut()
                .insert(DT_TRYBOOT.to_string(), Script::always(Ok(bytes.clone())));
            assert_eq!(PiFirmware.in_trial_boot(&mut m), Ok(expected), "{bytes:?}");
        }
    }

    #[test]
    fn an_undecodable_trial_flag_is_refused_rather_than_read_as_false() {
        let mut m = FakeMedium::new();
        m.properties_mut()
            .insert(DT_TRYBOOT.to_string(), Script::always(Ok(b"yes".to_vec())));
        assert!(PiFirmware.in_trial_boot(&mut m).is_err());
    }

    // -- constraint 1: the commit has a location ------------------------------

    /// The location is slot A's partition, whichever slot is running. That is
    /// the hardware fact of 2026-08-23 and the thing a flag-only trait would
    /// have hidden.
    #[test]
    fn the_default_slot_record_is_a_file_on_the_first_boot_partition() {
        assert_eq!(
            PiFirmware.default_slot_record(),
            SlotRecord::FileOnPartition {
                device: "/dev/mmcblk0p1".to_string(),
                fstype: "vfat",
                file: AUTOBOOT_FILE,
            }
        );
    }

    /// It must not move with the running slot. The port is asked twice on a
    /// board that booted `p2`, and answers `p1` both times.
    #[test]
    fn the_record_does_not_follow_the_booted_slot() {
        let a = PiFirmware.default_slot_record();
        let b = PiFirmware.default_slot_record();
        assert_eq!(a, b);
        assert_eq!(
            a,
            SlotRecord::FileOnPartition {
                device: autoboot_partition_device(),
                fstype: "vfat",
                file: AUTOBOOT_FILE,
            }
        );
    }

    // -- constraint 2: the payload has a location -----------------------------

    #[test]
    fn each_slots_payload_names_its_own_two_partitions() {
        assert_eq!(
            PiFirmware.payload(SlotId(1)),
            Ok(SlotPayload {
                kernel: PayloadTarget::DirectoryOnPartition {
                    device: "/dev/mmcblk0p1".to_string(),
                    fstype: "vfat",
                    path: "/".to_string(),
                },
                system: PayloadTarget::WholePartition {
                    device: "/dev/mmcblk0p3".to_string(),
                },
            })
        );
        assert_eq!(
            PiFirmware.payload(SlotId(2)),
            Ok(SlotPayload {
                kernel: PayloadTarget::DirectoryOnPartition {
                    device: "/dev/mmcblk0p2".to_string(),
                    fstype: "vfat",
                    path: "/".to_string(),
                },
                system: PayloadTarget::WholePartition {
                    device: "/dev/mmcblk0p4".to_string(),
                },
            })
        );
    }

    /// The two slots must not share a device anywhere. An update that wrote the
    /// running slot's `/system` would destroy the machine it is updating, and
    /// it would do so while every other test still passed.
    #[test]
    fn the_two_slots_share_no_device_at_all() {
        let a = PiFirmware.payload(SlotId(1));
        let b = PiFirmware.payload(SlotId(2));
        assert!(a.is_ok() && b.is_ok());
        assert_ne!(a, b);
        let devices = |p: &SlotPayload| {
            [&p.kernel, &p.system]
                .map(|t| match t {
                    PayloadTarget::WholePartition { device } => device.clone(),
                    PayloadTarget::DirectoryOnPartition { device, path, .. } => {
                        format!("{device}:{path}")
                    }
                })
                .to_vec()
        };
        if let (Ok(a), Ok(b)) = (a, b) {
            for one in devices(&a) {
                assert!(!devices(&b).contains(&one), "{one} is in both slots");
            }
        }
    }

    // -- ADR-029 §4: what the platform reserves on a payload target -----------

    /// The two names, on the boot half of **either** slot.
    ///
    /// Written in this module's existing style — assert the `Result`, then bind
    /// it — because this crate denies `unwrap`, `expect` and `panic` outright
    /// rather than allowing them in tests. `BootSlots::commit` is called from
    /// PID 1, where `panic = "abort"` is a kernel panic with no console, and the
    /// lint being on in the tests too is what keeps that habit.
    #[test]
    fn the_boot_half_of_every_slot_reserves_autoboot_and_the_provisioning_file() {
        for slot in [SlotId(1), SlotId(2)] {
            let payload = PiFirmware.payload(slot);
            assert!(payload.is_ok(), "slot {slot}: {payload:?}");
            if let Ok(payload) = payload {
                let reserved = PiFirmware.reserved_names(&payload.kernel);
                assert!(
                    reserved.contains(&"autoboot.txt"),
                    "slot {slot}: {reserved:?}"
                );
                assert!(
                    reserved.contains(&"rivet.toml"),
                    "slot {slot}: {reserved:?}"
                );
                assert_eq!(reserved.len(), 2);
            }
        }
    }

    /// A whole partition has no member names on it to reserve. Asserted rather
    /// than left implicit, because an empty list is exactly the sort of thing a
    /// later reader assumes is an oversight.
    #[test]
    fn a_whole_partition_target_reserves_nothing() {
        let payload = PiFirmware.payload(SlotId(1));
        assert!(payload.is_ok());
        if let Ok(payload) = payload {
            assert!(PiFirmware.reserved_names(&payload.system).is_empty());
        }
    }

    /// **The conjunction `ADR-029` §4 is built on, asserted end to end.** The
    /// file the port rewrites to record the default slot is on the same
    /// filesystem as slot A's boot payload — so a payload carrying that name
    /// would overwrite the commit record with a fresh-card default, and every
    /// subsequent boot would go to slot A. This test is what fails if either
    /// half moves.
    #[test]
    fn the_file_the_commit_record_lives_in_is_reserved_on_slot_as_boot_target() {
        let record = PiFirmware.default_slot_record();
        let slot_a = PiFirmware.payload(SlotId(1));
        let mut compared = false;
        if let (
            SlotRecord::FileOnPartition { device, file, .. },
            Ok(SlotPayload {
                kernel:
                    kernel @ PayloadTarget::DirectoryOnPartition {
                        device: target_device,
                        ..
                    },
                ..
            }),
        ) = (&record, &slot_a)
        {
            assert_eq!(
                device, target_device,
                "the commit record and slot A's boot payload are no longer on the same \
                 partition; ADR-029 §4's argument has changed shape and the reserved list must \
                 be re-derived"
            );
            assert!(PiFirmware.reserved_names(kernel).contains(file));
            compared = true;
        }
        assert!(
            compared,
            "the Pi's record must be a file on a partition and its boot half a directory on one: \
             {record:?} / {slot_a:?}"
        );
    }

    /// The provisioning file's name is spelled in three places. This is the
    /// guard for one of them; `cmd/rivet-initd` carries the other.
    #[test]
    fn the_reserved_provisioning_name_is_the_one_this_crate_declares() {
        assert_eq!(crate::firmware::PROVISIONING_FILE, "rivet.toml");
        let payload = PiFirmware.payload(SlotId(2));
        assert!(payload.is_ok());
        if let Ok(payload) = payload {
            assert!(
                PiFirmware
                    .reserved_names(&payload.kernel)
                    .contains(&crate::firmware::PROVISIONING_FILE)
            );
        }
    }

    // -- the drift guard between ADR-029 §3's target_class and PayloadTarget --

    /// `ADR-029` §3: *"`target_class` is `PayloadTarget` — deliberately the same
    /// two cases in the same order."* There are two enumerations because
    /// `rivet-core` may not depend on an adapter, and this is the only place
    /// both are visible.
    ///
    /// **What this catches:** a case added to one and not the other, or the
    /// order changed in either. Both would silently send a `/system` image to a
    /// FAT filesystem, or a kernel to a whole partition.
    #[test]
    fn the_containers_target_class_and_this_crates_payload_target_are_the_same_two_in_order() {
        assert_eq!(TargetClass::WholePartition.as_byte(), 0);
        assert_eq!(TargetClass::BootDirectory.as_byte(), 1);
        assert_eq!(TargetClass::from_byte(2), None);

        let payload = PiFirmware.payload(SlotId(1));
        assert!(payload.is_ok());
        if let Ok(payload) = payload {
            assert!(matches!(
                payload.target_for(TargetClass::WholePartition),
                PayloadTarget::WholePartition { .. }
            ));
            assert!(matches!(
                payload.target_for(TargetClass::BootDirectory),
                PayloadTarget::DirectoryOnPartition { .. }
            ));
            // And the mapping is onto: every case of the container's
            // enumeration reaches a distinct target.
            assert_ne!(
                payload.target_for(TargetClass::WholePartition),
                payload.target_for(TargetClass::BootDirectory)
            );
        }
    }

    /// The system half of a payload is the whole partition and the boot half is
    /// the directory — not the other way round. A transposition here would have
    /// an update write a 640 MiB filesystem image over a FAT partition.
    #[test]
    fn the_system_class_names_the_system_partition_and_not_the_boot_one() {
        for (slot, expected) in [(SlotId(1), Slot::A), (SlotId(2), Slot::B)] {
            let payload = PiFirmware.payload(slot);
            assert!(payload.is_ok(), "slot {slot}: {payload:?}");
            if let Ok(payload) = payload {
                assert_eq!(
                    payload.target_for(TargetClass::WholePartition),
                    &PayloadTarget::WholePartition {
                        device: partition_device(BOOT_DISK, expected.system_partition())
                    }
                );
            }
        }
    }

    /// `/state` is never a payload target: `ADR-018` §4 and §5.2 both say an
    /// update does not touch it, and §8 says a rollback could not repair it.
    #[test]
    fn no_payload_target_is_the_state_partition() {
        let state = partition_device(BOOT_DISK, rivet_core::state::STATE_PARTITION);
        for slot in [SlotId(1), SlotId(2)] {
            let payload = PiFirmware.payload(slot);
            assert!(!format!("{payload:?}").contains(&state), "{payload:?}");
        }
    }

    #[test]
    fn a_slot_the_layout_does_not_contain_is_refused_rather_than_defaulted() {
        for n in [0, 3, 4, 5, 99] {
            assert!(
                PiFirmware.payload(SlotId(n)).is_err(),
                "slot {n} was accepted"
            );
            assert!(PiFirmware.other_slot(SlotId(n)).is_err());
        }
    }

    #[test]
    fn the_other_slot_is_the_one_an_update_writes() {
        assert_eq!(PiFirmware.other_slot(SlotId(1)), Ok(SlotId(2)));
        assert_eq!(PiFirmware.other_slot(SlotId(2)), Ok(SlotId(1)));
    }

    // -- committing -----------------------------------------------------------

    #[test]
    fn a_commit_writes_a_temp_file_syncs_renames_and_syncs_again() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        let got = PiFirmware.commit(&mut m, SlotId(2));

        assert_eq!(got, Ok(Committed::Changed));
        let temp = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_TEMP}");
        let path = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}");
        assert_eq!(m.write_calls.len(), 1);
        assert!(m.write_calls.first().is_some_and(
            |(p, c)| p == &temp && String::from_utf8_lossy(c).contains("boot_partition=2")
        ));
        assert_eq!(m.rename_calls, vec![(temp, path)]);
        assert_eq!(m.sync_calls, 2, "once before the rename and once after");
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    /// The regression the record calls the expensive one: booted from `p2`, the
    /// commit writes `p1`'s file and puts `2` in it.
    #[test]
    fn a_slot_b_commit_writes_the_first_partitions_file_and_not_the_booted_one() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        let got = PiFirmware.commit(&mut m, SlotId(2));

        assert_eq!(got, Ok(Committed::Changed));
        let device = m.mount_calls.first().map(|c| c.device.clone());
        assert_eq!(
            device,
            Some(autoboot_partition_device()),
            "the commit mounted the booted partition instead of the first one"
        );
        // ...and the literal, because the line above compares the port's answer
        // against the same function the port used and would pass if *both* moved
        // together. `AGENTS.md` §4: a regression test that cannot fail proves
        // nothing, and injecting the 2026-08-23 defect — `autoboot_partition_device`
        // returning the booted partition — was checked to fail this line.
        assert_eq!(device, Some("/dev/mmcblk0p1".to_string()));
        assert_eq!(
            m.mount_calls.first().map(|c| c.at.clone()),
            Some(AUTOBOOT_PARTITION_MOUNT.to_string()),
            "a shared mount point lets a convergent EBUSY substitute the wrong filesystem"
        );
        assert!(
            m.write_calls
                .first()
                .is_some_and(|(_, c)| String::from_utf8_lossy(c).contains("boot_partition=2")),
            "the value written is the RUNNING slot's, even though the file is p1's"
        );
    }

    #[test]
    fn a_commit_of_the_slot_already_named_writes_nothing() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        assert_eq!(
            PiFirmware.commit(&mut m, SlotId(1)),
            Ok(Committed::AlreadyDefault)
        );
        assert!(m.write_calls.is_empty());
        assert_eq!(m.sync_calls, 0);
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    #[test]
    fn every_failure_still_unmounts_and_none_of_them_claims_success() {
        let unreadable = || card(Script::always(Err(Errno::EIO)));
        let mut cases: Vec<(&str, FakeMedium)> = Vec::new();
        cases.push(("unreadable file", unreadable()));
        let mut refuses_write = card(Script::always(Ok(AUTOBOOT.to_string())));
        refuses_write.write_results = Script::always(Err(Errno::EROFS));
        cases.push(("read-only card", refuses_write));
        let mut refuses_rename = card(Script::always(Ok(AUTOBOOT.to_string())));
        refuses_rename.rename_results = Script::always(Err(Errno::EROFS));
        cases.push(("rename refused", refuses_rename));
        cases.push((
            "unrecognised file",
            card(Script::always(Ok("nonsense\n".into()))),
        ));

        for (label, mut m) in cases {
            let got = PiFirmware.commit(&mut m, SlotId(2));
            assert!(got.is_err(), "{label} reported {got:?}");
            assert_eq!(
                m.unmount_calls,
                vec![AUTOBOOT_PARTITION_MOUNT.to_string()],
                "{label} left the partition mounted"
            );
        }
    }

    #[test]
    fn a_failed_rename_never_reaches_the_second_sync() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.rename_results = Script::always(Err(Errno::EROFS));
        assert!(PiFirmware.commit(&mut m, SlotId(2)).is_err());
        assert_eq!(m.sync_calls, 1, "the second sync must not have happened");
    }

    #[test]
    fn a_failed_write_never_reaches_the_real_file() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.write_results = Script::always(Err(Errno::EROFS));
        assert!(PiFirmware.commit(&mut m, SlotId(2)).is_err());
        assert!(m.rename_calls.is_empty());
    }

    #[test]
    fn a_partition_that_will_not_mount_declines_before_writing_anything() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.mount_results = Script::always(Err(Errno::ENODEV));
        assert!(PiFirmware.commit(&mut m, SlotId(2)).is_err());
        assert!(m.write_calls.is_empty());
    }

    /// A commit's refusal says which kind it is, because PID 1 retries one kind
    /// and not the other: the medium refusing may answer differently next
    /// time, and a file this crate will not rewrite never will.
    #[test]
    fn a_commit_refusal_says_whether_the_medium_or_the_file_refused() {
        let mut rename = card(Script::always(Ok(AUTOBOOT.to_string())));
        rename.rename_results = Script::always(Err(Errno::ENOENT));
        let got = PiFirmware.commit(&mut rename, SlotId(2));
        assert!(
            matches!(got, Err(ArmRefusal::Medium(ref why)) if why.contains("cannot rename")),
            "{got:?}"
        );

        let mut write = card(Script::always(Ok(AUTOBOOT.to_string())));
        write.write_results = Script::always(Err(Errno::EROFS));
        let got = PiFirmware.commit(&mut write, SlotId(2));
        assert!(matches!(got, Err(ArmRefusal::Medium(_))), "{got:?}");

        let mut mount = card(Script::always(Ok(AUTOBOOT.to_string())));
        mount.mount_results = Script::always(Err(Errno::ENODEV));
        let got = PiFirmware.commit(&mut mount, SlotId(2));
        assert!(matches!(got, Err(ArmRefusal::Medium(_))), "{got:?}");

        let mut unread = card(Script::always(Err(Errno::EIO)));
        let got = PiFirmware.commit(&mut unread, SlotId(2));
        assert!(matches!(got, Err(ArmRefusal::Medium(_))), "{got:?}");

        let mut nonsense = card(Script::always(Ok("nonsense\n".into())));
        let got = PiFirmware.commit(&mut nonsense, SlotId(2));
        assert!(matches!(got, Err(ArmRefusal::Unsafe(_))), "{got:?}");
        assert!(nonsense.write_calls.is_empty());
    }

    /// A second commit after a refused rename mounts again and writes the
    /// temporary file again — the retry PID 1 performs is exactly this call
    /// repeated, with nothing left over from the first attempt in the way.
    #[test]
    fn a_commit_after_a_refused_rename_mounts_afresh_and_succeeds() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.rename_results = Script::sequence(vec![Err(Errno::ENOENT)], Ok(()));
        assert!(PiFirmware.commit(&mut m, SlotId(2)).is_err());
        assert_eq!(PiFirmware.commit(&mut m, SlotId(2)), Ok(Committed::Changed));

        let temp = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_TEMP}");
        assert_eq!(m.mount_calls.len(), 2, "each attempt mounts");
        assert_eq!(m.unmount_calls.len(), 2, "each attempt unmounts");
        assert_eq!(m.write_calls.len(), 2);
        assert!(m.write_calls.iter().all(|(p, _)| p == &temp));
    }

    // -- the diagnostic listing -----------------------------------------------

    fn entry(name: &str, size: u64) -> crate::port::DirEntry {
        crate::port::DirEntry {
            name: name.to_string(),
            size: Ok(size),
        }
    }

    #[test]
    fn the_record_listing_mounts_read_only_lists_the_root_and_unmounts() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.list_results = Script::always(Ok(vec![
            entry("autoboot.txt", 44),
            entry("autoboot.txt.new", 44),
        ]));
        let got = PiFirmware.record_medium_listing(&mut m, 64);

        assert_eq!(
            got,
            Ok(Listing {
                entries: vec![entry("autoboot.txt", 44), entry("autoboot.txt.new", 44)],
                total: 2,
            })
        );
        let call = m.mount_calls.first();
        assert!(call.is_some_and(|c| !c.writable
            && c.at == AUTOBOOT_PARTITION_MOUNT
            && c.device == autoboot_partition_device()));
        assert_eq!(
            m.list_calls,
            vec![(AUTOBOOT_PARTITION_MOUNT.to_string(), 64)]
        );
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
        assert!(m.write_calls.is_empty());
        assert!(m.rename_calls.is_empty());
    }

    #[test]
    fn the_record_listing_is_bounded_and_says_how_many_there_were() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.list_results = Script::always(Ok(vec![entry("a", 1), entry("b", 2), entry("c", 3)]));
        let got = PiFirmware.record_medium_listing(&mut m, 2);
        assert!(
            got.as_ref()
                .is_ok_and(|l| l.entries.len() == 2 && l.total == 3),
            "{got:?}"
        );
    }

    #[test]
    fn a_record_listing_that_fails_still_unmounts() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.list_results = Script::always(Err(Errno::EIO));
        let got = PiFirmware.record_medium_listing(&mut m, 64);
        assert!(got.as_ref().is_err_and(|why| why.contains("EIO")), "{got:?}");
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    #[test]
    fn a_record_listing_whose_mount_fails_lists_nothing() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        m.mount_results = Script::always(Err(Errno::ENODEV));
        let got = PiFirmware.record_medium_listing(&mut m, 64);
        assert!(got.is_err(), "{got:?}");
        assert!(m.list_calls.is_empty());
    }

    // -- arming ---------------------------------------------------------------

    #[test]
    fn arming_moves_the_tryboot_override_and_leaves_the_default_alone() {
        // Running slot B, arming slot A: the interlock added 2026-08-23 refuses
        // a trial of the running slot, so this card has to be booted from the
        // other one.
        let mut m = card_running(2, Script::always(Ok(AUTOBOOT.to_string())));
        let got = PiFirmware.arm_trial_boot(&mut m, SlotId(1));

        assert_eq!(got, Ok(ArmedReboot::OneShot));
        let written = m
            .write_calls
            .first()
            .map(|(_, c)| String::from_utf8_lossy(c).into_owned());
        assert_eq!(
            written.as_deref(),
            Some("[all]\ntryboot_a_b=1\nboot_partition=1\n\n[tryboot]\nboot_partition=1\n"),
            "the [all] default must be untouched"
        );
        assert_eq!(m.sync_calls, 2);
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    /// On this platform the one-shot lives in the reboot call, so arming is not
    /// complete until the caller asks for that particular reboot. The port says
    /// which one rather than the caller knowing.
    ///
    /// **That arming does not stop the machine is now structural rather than
    /// asserted:** there is no reboot operation on [`BootMedium`] at all, so the
    /// old form of this test — "the fake recorded no reboot call" — no longer
    /// has anything to look at. What is left to check is that the answer is the
    /// one that actually carries the one-shot, and that arming really did only
    /// touch the file.
    #[test]
    fn arming_returns_the_reboot_that_actually_carries_the_one_shot() {
        let mut m = card_running(2, Script::always(Ok(AUTOBOOT.to_string())));
        assert_eq!(
            PiFirmware.arm_trial_boot(&mut m, SlotId(1)),
            Ok(ArmedReboot::OneShot)
        );
        assert_ne!(
            ArmedReboot::OneShot,
            ArmedReboot::Ordinary,
            "an ordinary reboot would boot the default slot and trial nothing"
        );
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    #[test]
    fn arming_a_slot_that_is_already_the_override_writes_nothing_and_still_arms() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        assert_eq!(
            PiFirmware.arm_trial_boot(&mut m, SlotId(2)),
            Ok(ArmedReboot::OneShot)
        );
        assert!(m.write_calls.is_empty());
    }

    #[test]
    fn arming_a_slot_outside_the_layout_is_refused_before_the_partition_is_mounted() {
        let mut m = card(Script::always(Ok(AUTOBOOT.to_string())));
        assert!(PiFirmware.arm_trial_boot(&mut m, SlotId(9)).is_err());
        assert!(
            m.mount_calls.is_empty(),
            "nothing may be mounted for a bad slot"
        );
        assert!(m.write_calls.is_empty());
    }

    #[test]
    fn a_file_with_no_tryboot_override_is_refused_rather_than_given_one() {
        // Corrupting autoboot.txt on a headless board is unrecoverable, and
        // inventing a section is a rewrite rather than an edit.
        //
        // `tryboot_a_b=1` is present so that this still tests the *override*
        // being missing rather than tripping interlock 1 on the way past.
        let mut m = card(Script::always(Ok(
            "[all]\ntryboot_a_b=1\nboot_partition=1\n".to_string(),
        )));
        let got = PiFirmware.arm_trial_boot(&mut m, SlotId(2));
        assert!(
            matches!(got, Err(ArmRefusal::Unsafe(ref why)) if why.contains("[tryboot]")),
            "{got:?}"
        );
        assert!(m.write_calls.is_empty());
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    // -- interlock 1: `tryboot_a_b=1` must actually apply ----------------------

    /// **`ADR-018` §5.3's trap, as a test.** A card whose `autoboot.txt` does not
    /// set `tryboot_a_b=1` unconditionally is one where a tryboot reboot reads
    /// `tryboot.txt` instead of `config.txt` — and §5.3 measured that costing a
    /// headless board. Nothing may be written to such a card.
    #[test]
    fn a_card_without_tryboot_a_b_is_refused_and_the_refusal_names_the_setting() {
        for text in [
            // Absent entirely — what a hand-edited card looks like.
            "[all]\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n",
            // Present and off.
            "[all]\ntryboot_a_b=0\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n",
            // Present, but only where it does not apply to every boot.
            "[all]\nboot_partition=1\n\n[tryboot]\ntryboot_a_b=1\nboot_partition=2\n",
            // Present twice unconditionally, with two answers.
            "tryboot_a_b=1\n[all]\ntryboot_a_b=0\nboot_partition=1\n[tryboot]\nboot_partition=2\n",
        ] {
            let mut m = card(Script::always(Ok(text.to_string())));
            let got = PiFirmware.arm_trial_boot(&mut m, SlotId(2));
            assert!(
                matches!(got, Err(ArmRefusal::Unsafe(ref why)) if why.contains(TRYBOOT_A_B)),
                "{text:?} was not refused by name: {got:?}"
            );
            assert!(m.write_calls.is_empty(), "{text:?} wrote to the card");
            assert!(m.rename_calls.is_empty(), "{text:?} renamed over the card");
            assert_eq!(
                m.unmount_calls,
                vec![AUTOBOOT_PARTITION_MOUNT.to_string()],
                "{text:?} left the partition mounted"
            );
        }
    }

    /// The check itself, at its edges. A setting is `config.txt` syntax: before
    /// the first header or under `[all]` it applies to every boot, and anywhere
    /// else it does not.
    #[test]
    fn tryboot_a_b_is_required_to_apply_unconditionally_and_to_be_one() {
        assert_eq!(require_tryboot_a_b("tryboot_a_b=1\n"), Ok(()));
        assert_eq!(require_tryboot_a_b("[all]\nTryboot_A_B = 1\r\n"), Ok(()));
        assert_eq!(require_tryboot_a_b(AUTOBOOT), Ok(()));
        for refused in [
            "",
            "tryboot_a_b=0\n",
            "tryboot_a_b=yes\n",
            "#tryboot_a_b=1\n",
            "[tryboot]\ntryboot_a_b=1\n",
            "[all]\ntryboot_a_b=1\ntryboot_a_b=1\n",
        ] {
            assert!(
                require_tryboot_a_b(refused).is_err(),
                "{refused:?} was accepted"
            );
        }
    }

    // -- interlock 2: the running slot may not be trialled ---------------------

    /// **A trial of the running slot is the failure that reports success.** The
    /// board restarts, comes back byte-identical, passes every check, and PID 1
    /// commits it — so an update that did nothing looks like one that worked.
    #[test]
    fn arming_the_slot_this_machine_is_running_is_refused_before_anything_is_mounted() {
        for running in [1, 2] {
            let mut m = card_running(running, Script::always(Ok(AUTOBOOT.to_string())));
            let got = PiFirmware.arm_trial_boot(&mut m, SlotId(running));
            assert!(
                matches!(got, Err(ArmRefusal::Unsafe(_))),
                "slot {running} was armed while running: {got:?}"
            );
            assert!(
                m.mount_calls.is_empty(),
                "the card was mounted for a refusal"
            );
            assert!(m.write_calls.is_empty());
        }
    }

    /// And the refusal has to be actionable: it names the slot to arm instead.
    #[test]
    fn the_running_slot_refusal_names_the_slot_to_arm_instead() {
        let mut m = card_running(2, Script::always(Ok(AUTOBOOT.to_string())));
        let got = PiFirmware.arm_trial_boot(&mut m, SlotId(2));
        assert!(
            matches!(got, Err(ArmRefusal::Unsafe(ref why)) if why.contains("slot 1")),
            "the refusal must name the slot to arm instead: {got:?}"
        );
    }

    /// A firmware that will not answer is a [`ArmRefusal::Medium`], not an
    /// interlock: the request may be perfectly good and the *machine* is what
    /// could not be read. Under an emulator this is every run (`ADR-026` §4).
    #[test]
    fn with_no_device_tree_an_arming_reports_the_machine_rather_than_the_request() {
        let mut m = FakeMedium::new();
        let got = PiFirmware.arm_trial_boot(&mut m, SlotId(2));
        assert!(matches!(got, Err(ArmRefusal::Medium(_))), "{got:?}");
        assert!(m.mount_calls.is_empty());
    }

    // -- the dry run ----------------------------------------------------------

    /// `ADR-006` §6.2: the plan must reach the same verdict as the apply, and
    /// must not write. Both interlocks and the file's shape, each checked with
    /// an empty `write_calls` afterwards.
    #[test]
    fn the_dry_run_reaches_the_same_verdict_as_the_arming_and_writes_nothing() {
        let cases: Vec<(&str, u32, u32, &str, bool)> = vec![
            ("a card this repo built", 1, 2, AUTOBOOT, true),
            ("the running slot", 1, 1, AUTOBOOT, false),
            ("a slot outside the layout", 1, 9, AUTOBOOT, false),
            (
                "no tryboot_a_b",
                1,
                2,
                "[all]\nboot_partition=1\n[tryboot]\nboot_partition=2\n",
                false,
            ),
            (
                "no [tryboot] override",
                1,
                2,
                "[all]\ntryboot_a_b=1\nboot_partition=1\n",
                false,
            ),
        ];
        for (label, running, target, text, allowed) in cases {
            let mut dry = card_running(running, Script::always(Ok(text.to_string())));
            let planned = PiFirmware.can_arm_trial_boot(&mut dry, SlotId(target));
            assert_eq!(planned.is_ok(), allowed, "{label}: {planned:?}");
            assert!(dry.write_calls.is_empty(), "{label}: the dry run wrote");
            assert!(dry.rename_calls.is_empty(), "{label}: the dry run renamed");
            assert_eq!(dry.sync_calls, 0, "{label}: the dry run synced");

            let mut wet = card_running(running, Script::always(Ok(text.to_string())));
            let armed = PiFirmware.arm_trial_boot(&mut wet, SlotId(target));
            assert_eq!(
                armed.is_ok(),
                planned.is_ok(),
                "{label}: the plan and the apply disagreed — {planned:?} then {armed:?}"
            );
        }
    }

    /// The dry run must not mount the card writable. A plan that opened the one
    /// file it exists to protect for writing has already taken the risk.
    #[test]
    fn the_dry_run_mounts_the_card_read_only() {
        let mut m = card_running(2, Script::always(Ok(AUTOBOOT.to_string())));
        assert_eq!(PiFirmware.can_arm_trial_boot(&mut m, SlotId(1)), Ok(()));
        let call = m.mount_calls.first();
        assert!(call.is_some_and(|c| !c.writable), "{:?}", m.mount_calls);
        assert_eq!(m.unmount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    /// Both members of [`ArmedReboot`] must have a console form, and they must
    /// not read alike: on a board the line PID 1 printed is the only evidence of
    /// which reboot it was about to ask for.
    #[test]
    fn each_kind_of_armed_reboot_says_which_one_it_is() {
        assert!(!ArmedReboot::OneShot.to_string().is_empty());
        assert!(!ArmedReboot::Ordinary.to_string().is_empty());
        assert_ne!(
            ArmedReboot::OneShot.to_string(),
            ArmedReboot::Ordinary.to_string()
        );
    }

    // -- the [all] rewrite ----------------------------------------------------

    #[test]
    fn the_default_moves_and_the_tryboot_override_is_left_alone() {
        assert_eq!(
            rewrite_autoboot_default(
                "[all]\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n",
                2
            ),
            Ok(Some(
                "[all]\nboot_partition=2\n\n[tryboot]\nboot_partition=2\n".to_string()
            ))
        );
    }

    #[test]
    fn a_file_that_already_names_the_partition_needs_no_write() {
        assert_eq!(rewrite_autoboot_default(AUTOBOOT, 1), Ok(None));
    }

    #[test]
    fn windows_line_endings_survive_the_rewrite_byte_for_byte() {
        // The operator edits this file from Windows: ADR-002 §4.1.
        assert_eq!(
            rewrite_autoboot_default(AUTOBOOT_CRLF, 2),
            Ok(Some(
                "[all]\r\nboot_partition=2\r\n[tryboot]\r\nboot_partition=2\r\n".to_string()
            ))
        );
    }

    #[test]
    fn a_setting_before_any_section_applies_unconditionally_and_is_the_one_rewritten() {
        // config.txt syntax: settings above the first [section] apply to all.
        let text = "boot_partition=1\n[tryboot]\nboot_partition=2\n";
        assert_eq!(
            rewrite_autoboot_default(text, 2),
            Ok(Some(
                "boot_partition=2\n[tryboot]\nboot_partition=2\n".to_string()
            ))
        );
    }

    #[test]
    fn comments_and_unrelated_settings_are_preserved_exactly() {
        let text = "# Rivet A/B\ntryboot_a_b=1\n[all]\nboot_partition=1\n";
        assert_eq!(
            rewrite_autoboot_default(text, 2),
            Ok(Some(
                "# Rivet A/B\ntryboot_a_b=1\n[all]\nboot_partition=2\n".to_string()
            ))
        );
    }

    #[test]
    fn a_commented_out_setting_is_not_mistaken_for_the_real_one() {
        let text = "[all]\n#boot_partition=9\nboot_partition=1\n";
        assert_eq!(
            rewrite_autoboot_default(text, 2),
            Ok(Some(
                "[all]\n#boot_partition=9\nboot_partition=2\n".to_string()
            ))
        );
    }

    #[test]
    fn a_file_with_no_unconditional_setting_is_refused_rather_than_rewritten() {
        // Corrupting autoboot.txt on a headless board is unrecoverable.
        let text = "[tryboot]\nboot_partition=2\n";
        assert!(rewrite_autoboot_default(text, 2).is_err());
        assert!(rewrite_autoboot_default("", 2).is_err());
        assert!(rewrite_autoboot_default("nothing to see here\n", 2).is_err());
    }

    #[test]
    fn two_unconditional_settings_are_refused_rather_than_guessed_between() {
        let text = "boot_partition=1\n[all]\nboot_partition=3\n";
        assert!(rewrite_autoboot_default(text, 2).is_err_and(|e| e.contains('2')));
    }

    #[test]
    fn a_file_with_no_trailing_newline_keeps_not_having_one() {
        assert_eq!(
            rewrite_autoboot_default("[all]\nboot_partition=1", 2),
            Ok(Some("[all]\nboot_partition=2".to_string()))
        );
    }

    #[test]
    fn section_names_and_the_key_are_matched_without_regard_to_case() {
        let text = "[ALL]\nBoot_Partition = 1\n";
        assert_eq!(
            rewrite_autoboot_default(text, 2),
            Ok(Some("[ALL]\nboot_partition=2\n".to_string()))
        );
    }

    // -- the [tryboot] rewrite ------------------------------------------------

    #[test]
    fn the_override_moves_and_the_default_is_left_alone() {
        assert_eq!(
            rewrite_autoboot_tryboot("[all]\nboot_partition=1\n[tryboot]\nboot_partition=2\n", 1),
            Ok(Some(
                "[all]\nboot_partition=1\n[tryboot]\nboot_partition=1\n".to_string()
            ))
        );
    }

    /// The asymmetry between the two functions, as a test rather than a comment.
    /// A setting before the first header applies to a *normal* boot, so it is
    /// not the override and moving it would change the default slot.
    #[test]
    fn a_setting_before_any_section_is_not_the_tryboot_override() {
        let text = "boot_partition=1\n";
        assert!(rewrite_autoboot_tryboot(text, 2).is_err());
        assert_eq!(
            rewrite_autoboot_default(text, 2),
            Ok(Some("boot_partition=2\n".to_string()))
        );
    }

    #[test]
    fn an_override_that_already_names_the_partition_needs_no_write() {
        assert_eq!(rewrite_autoboot_tryboot(AUTOBOOT, 2), Ok(None));
    }

    #[test]
    fn the_override_rewrite_preserves_windows_line_endings_too() {
        assert_eq!(
            rewrite_autoboot_tryboot(AUTOBOOT_CRLF, 1),
            Ok(Some(
                "[all]\r\nboot_partition=1\r\n[tryboot]\r\nboot_partition=1\r\n".to_string()
            ))
        );
    }

    #[test]
    fn a_missing_or_duplicated_override_is_refused_rather_than_guessed_at() {
        assert!(rewrite_autoboot_tryboot("[all]\nboot_partition=1\n", 2).is_err());
        assert!(rewrite_autoboot_tryboot("", 2).is_err());
        assert!(
            rewrite_autoboot_tryboot("[tryboot]\nboot_partition=1\nboot_partition=2\n", 1)
                .is_err_and(|e| e.contains('2'))
        );
    }

    #[test]
    fn a_second_tryboot_section_is_still_the_tryboot_section() {
        // config.txt syntax allows a section to be reopened. Two settings in two
        // [tryboot] blocks are still two, and are refused as such.
        let text =
            "[tryboot]\nboot_partition=1\n[all]\nboot_partition=1\n[tryboot]\nboot_partition=2\n";
        assert!(rewrite_autoboot_tryboot(text, 1).is_err());
    }

    #[test]
    fn a_commented_out_override_is_not_mistaken_for_the_real_one() {
        assert_eq!(
            rewrite_autoboot_tryboot("[tryboot]\n#boot_partition=9\nboot_partition=1\n", 2),
            Ok(Some(
                "[tryboot]\n#boot_partition=9\nboot_partition=2\n".to_string()
            ))
        );
    }

    /// The two rewrites must never touch each other's line. Run one after the
    /// other over the file the image writes and check that both moved and
    /// nothing else did — this is the individually-correct-but-jointly-wrong
    /// case `AGENTS.md` §4 asks for.
    #[test]
    fn committing_and_then_arming_moves_exactly_two_values() {
        let committed = rewrite_autoboot_default(AUTOBOOT, 2);
        assert_eq!(
            committed,
            Ok(Some(
                "[all]\ntryboot_a_b=1\nboot_partition=2\n\n[tryboot]\nboot_partition=2\n"
                    .to_string()
            ))
        );
        let armed = committed
            .ok()
            .flatten()
            .map(|text| rewrite_autoboot_tryboot(&text, 1));
        assert_eq!(
            armed,
            Some(Ok(Some(
                "[all]\ntryboot_a_b=1\nboot_partition=2\n\n[tryboot]\nboot_partition=1\n"
                    .to_string()
            ))),
            "after a commit the override must be movable off the running slot"
        );
    }

    #[test]
    fn the_platform_says_which_firmware_this_is() {
        let name = PiFirmware.platform();
        assert!(name.contains("tryboot"), "{name}");
        assert!(!name.is_empty());
    }
}

#[cfg(test)]
#[path = "slots_lock_tests.rs"]
mod lock_tests;
