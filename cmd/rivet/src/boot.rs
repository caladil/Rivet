//! The composition root's boot half: two adapters, stacked, and nothing else.
//!
//! ```text
//! rivet_core::operation::Machine     the DOMAIN port  — slots and stops, nothing platform-shaped
//!            ▲  PiMachine (here)
//!            │
//!            ├── arming ──────────────────────────────┬── stopping ─────────────────────
//!            ▼                                        ▼
//! rivet_adapter_image::BootSlots                crate::shutdown::InitProcess
//!   the A/B primitive — autoboot.txt, [tryboot]     the I/O seam — /proc/1, one signal
//!            ▲  SyscallMedium (here)                  ▲  ProcInit
//! rivet_adapter_image::BootMedium                crate::syscall::kill_init
//!   the I/O seam — mounts, Errno, files                     │
//!            ▲  crate::syscall                              │
//!            libc ◄────────────────────────────────────────┘
//! ```
//!
//! **The two columns never touch**, and that is `ADR-018` §5's separation of
//! arming from restarting made structural rather than remembered.
//!
//! **`AGENTS.md` §3.1's two kinds of seam, both present in one file**, which is
//! why they are drawn rather than described. `BootMedium` is *an adapter's own
//! I/O seam* — it carries an `Errno`, a mount and a filesystem type, and
//! `rivet-core` could not name it. `Machine` is *a domain port* — it carries
//! `Slot` and nothing else, and it is the first one this repository has. This
//! module is the two conversions between them and no decisions at all
//! (`ADR-006` §3: the CLI parses arguments and renders results).
//!
//! It is the same shape `cmd/rivet-initd/src/slots.rs` has, where a `Kernel`
//! becomes a `BootMedium`, and `cmd/rivet-sysd/src/clock.rs`, where the
//! same `Kernel` becomes a `rivet_store::Filesystem`. Going through a port
//! rather than to the platform is what keeps every branch reachable from a test —
//! and the branches that matter here are ones `ADR-026` §4 says the emulator
//! cannot produce and a board should not be asked to: no device tree at all, a
//! boot partition that will not mount, a card that has gone read-only mid-write.
//!
//! # What is deliberately not here
//!
//! ~~**A reboot.** `Machine` has no such method, `BootMedium` has no such method,
//! and `ADR-017` §3.1.5 item 6 gives shutdown to PID 1. `rivet` arms and stops;
//! see `rivet_core::operation::trial` for what that leaves open.~~
//!
//! **Amended 2026-08-23, and the half that mattered is unchanged.** `Machine`
//! gained two methods and `BootMedium` gained none: **this file still cannot
//! reboot anything.** What `rivet` can now do is *ask* pid 1 to, which is
//! `ADR-017` §3.1.5 item 6 respected rather than taken over — the request goes
//! through `crate::shutdown`, over a seam of its own, and nothing in the boot
//! stack drawn above is involved in it. `PiMachine` therefore holds two seams
//! that never meet: an arming reads a card and stops, and a stop signals pid 1
//! and never reads a card.
//!
//! **That separation is `ADR-018` §5's, made structural.** A stop that had to
//! mount the boot partition first would be a machine that cannot be stopped
//! because its card has gone read-only.
//!
//! **Amended again 2026-08-24, and the same half is still unchanged.** `Machine`
//! gained a seventh method — `read_filesystems`, `ADR-005` §4.1.3's `df` — and
//! `BootMedium` gained none. It is not drawn above because it is not in this
//! file's stack at all: like `read_journal`, it delegates to a module of its own
//! (`crate::storage`), which has its own seam and its own diagram. `PiMachine`
//! therefore holds **two** platform seams and forwards **two** questions to
//! modules that hold their own, which is the shape rather than a drift: what
//! this type owns is the card and pid 1, and neither of the read-only questions
//! is about either.
//!
//! **Anything that writes a payload.** `ADR-027` §9 item 1 leaves the payload
//! format undecided and `ADR-028` §8 leaves the signature scheme open, so a
//! verification here would be inventing a ruling. `BootSlots::payload` says where
//! one would go and this module does not call it.

// **Unused on Windows, load-bearing on the target.** The `Detached`/`Syscall`
// implementations below are `cfg`-gated to unix, so a Windows build sees no
// use of these names and warns. They are NOT removable: deleting them on that
// warning's word broke the aarch64-musl cross build -- which is the build that
// makes a card -- and it went unnoticed because the check had only ever been
// run on the host. `PLAN.md`: a warning is a claim about ONE configuration.
#[cfg_attr(windows, allow(unused_imports))]
use rivet_adapter_image::{
    ArmRefusal, ArmedReboot, BootMedium, BootSlots as _, Errno, Mounted, PiFirmware, SlotId,
};
use rivet_core::operation::{
    Board, Deployer, JournalRead, Machine, MachineError, Observed, Reboot, Stop, StorageRead,
};
use rivet_core::state::Slot;

use crate::shutdown::{self, InitProcess};

/// `rivet_adapter_image::BootSlots` and `crate::shutdown` as the domain's
/// [`Machine`].
///
/// Generic over both seams so that every conversion below is reachable from a
/// test on a development host — they are the only platform in the stack.
///
/// **Two adapters in one type, and they share nothing.** `medium` is the card and
/// `init` is pid 1; no method touches both. That is not tidiness: `ADR-018` §5
/// separates arming from restarting because on other hardware they are separate,
/// and a `request_stop` that had to read a card would make a dying card into a
/// machine that cannot be stopped.
pub struct PiMachine<M: BootMedium, I: InitProcess + crate::reload::Supervisor> {
    medium: M,
    init: I,
}

impl<M: BootMedium, I: InitProcess + crate::reload::Supervisor> PiMachine<M, I> {
    pub const fn new(medium: M, init: I) -> Self {
        Self { medium, init }
    }

    /// The slot the firmware says it booted, or `None` when it would not say.
    ///
    /// For the install channel's *already running* check
    /// (`crate::deploy::Requests::not_already_running`). **The firmware's
    /// reading and not `get slot`'s two-authority one**, deliberately: it is the
    /// reading `cmd/rivet-deploy` computes its target from, so the channel and the
    /// installer name the same slot as running. `None` is not a refusal — an
    /// unreadable slot leaves that decision to the installer.
    fn booted_slot(&mut self) -> Option<Slot> {
        Machine::running_slot(self).ok()
    }

    /// **Which slot an ordinary reset would boot**, or `Err` naming why not.
    ///
    /// For the install channel's uncommitted-trial check
    /// (`crate::deploy::Requests::not_uncommitted_trial`, `ADR-018` §5,
    /// `PLAN-Feature-Set-2026-09-12` §3). The same card record
    /// `cmd/rivet-deploy`'s installer reads, so the channel and the installer
    /// can never disagree about which slot a reset falls back to — only about
    /// whether they were able to read it at all.
    fn default_slot(&mut self) -> Result<Slot, String> {
        known_slot_or_reason(PiFirmware.default_slot(&mut self.medium)?)
    }
}

/// A firmware-reported partition number, as a [`Slot`] — or the sentence
/// [`Observed::read_slots`] and [`PiMachine::default_slot`] both compose when
/// it names neither. **One literal, not two** (`ADR-011`): both readers ask
/// the same question of a number the firmware handed over, and a caller with
/// its own wording would be free to drift from the other's.
fn known_slot_or_reason(id: SlotId) -> Result<Slot, String> {
    Slot::from_boot_partition(id.0).ok_or_else(|| {
        format!(
            "autoboot.txt names boot_partition={} and ADR-018 §4 does not describe that partition",
            id.0
        )
    })
}

/// The one place an image-port refusal becomes a domain refusal.
///
/// The two vocabularies were cut to the same shape on purpose — see
/// `rivet_adapter_image::ArmRefusal`, which explains why an arming is the only
/// operation on that port carrying a typed refusal — so this is the mapping and
/// not a re-interpretation. **Nothing is reworded on the way through**, which is
/// `ADR-006` §3's "the CLI contains no logic" applied to the failure path.
fn to_machine_error(refusal: ArmRefusal) -> MachineError {
    match refusal {
        ArmRefusal::Unsafe(why) => MachineError::Unsafe(why),
        ArmRefusal::Medium(why) => MachineError::Refused(why),
    }
}

/// Which domain restart an image-port arming needs.
///
/// The third leg of the conversion `rivet_adapter_image::ArmedReboot` and
/// `cmd/rivet-initd`'s `reboot_for` already document: the platforms differ in
/// *where the one-shot lives*, and each layer names that in its own words rather
/// than one layer's enum travelling through all three crates.
const fn to_reboot(armed: ArmedReboot) -> Reboot {
    match armed {
        ArmedReboot::Ordinary => Reboot::Ordinary,
        ArmedReboot::OneShot => Reboot::OneShot,
    }
}

impl<M: BootMedium, I: InitProcess + crate::reload::Supervisor> Machine for PiMachine<M, I> {
    fn running_slot(&mut self) -> Result<Slot, MachineError> {
        let id = PiFirmware
            .running_slot(&mut self.medium)
            .map_err(MachineError::Refused)?;
        // The firmware's number becomes a *slot* here and nowhere below: the
        // image port keeps it as the number the firmware reported, precisely so
        // that nothing above it does arithmetic on one (see `SlotId`). A number
        // outside the layout is refused rather than defaulted — `AGENTS.md` §5,
        // and the guess would name a partition an update would overwrite.
        Slot::from_boot_partition(id.0).ok_or_else(|| {
            MachineError::Unsafe(format!(
                "the firmware reports this machine booted from partition {id}, which is \
                 neither of ADR-018 §4's two boot partitions ({} and {}). This is not a card \
                 that layout describes, and nothing will be armed on it.",
                Slot::A.boot_partition(),
                Slot::B.boot_partition(),
            ))
        })
    }

    fn can_arm_trial_boot(&mut self, slot: Slot) -> Result<(), MachineError> {
        PiFirmware
            .can_arm_trial_boot(&mut self.medium, to_slot_id(slot))
            .map_err(to_machine_error)
    }

    fn arm_trial_boot(&mut self, slot: Slot) -> Result<Reboot, MachineError> {
        PiFirmware
            .arm_trial_boot(&mut self.medium, to_slot_id(slot))
            .map(to_reboot)
            .map_err(to_machine_error)
    }

    /// The interlock, and **nothing else** — `crate::shutdown::identify` reads
    /// pid 1 and sends no signal at all.
    fn can_request_stop(&mut self, _stop: Stop) -> Result<(), MachineError> {
        shutdown::identify(&mut self.init).map(|_| ())
    }

    /// The interlock, and then one signal. **The card is not touched**, which is
    /// this type's whole reason for holding two seams.
    fn request_stop(&mut self, stop: Stop) -> Result<(), MachineError> {
        shutdown::request(&mut self.init, stop)
    }

    /// **`ADR-032` §3.2's reload**, and both halves go through `crate::reload`
    /// — the interlock and the signal — exactly as the three stops go through
    /// `crate::shutdown`. Nothing is decided here.
    ///
    /// The card is not touched by either. That is why this type holds two seams
    /// and why the `init` one now answers a third question: reading `/proc` and
    /// signalling a process is one platform's business, and a reload needs
    /// nothing else.
    fn can_request_reload(&mut self) -> Result<(), MachineError> {
        crate::reload::find(&mut self.init).map(|_| ())
    }

    fn request_reload(&mut self) -> Result<(), MachineError> {
        crate::reload::request(&mut self.init)
    }

    /// **`ADR-012` §9's press.** Both halves go through `crate::press`, exactly
    /// as the reload above goes through `crate::reload` — nothing is decided
    /// here.
    ///
    /// It touches neither the card nor `init`: the launcher's socket is the
    /// whole of the machine this needs, which is why it is a third seam rather
    /// than a question for either of the two this type already holds.
    fn can_send_input(&mut self) -> Result<(), MachineError> {
        crate::press::can_press()
    }

    fn send_input(&mut self, action: &str) -> Result<(), MachineError> {
        crate::press::press(action)
    }

    /// **`ADR-017` §6.3.2's service control, over `ADR-032` §3.2's socket** —
    /// added 2026-08-29, and it goes through `crate::status` rather than
    /// `crate::reload`.
    ///
    /// That is the whole difference between this and the reload above and it is
    /// worth saying: a reload is a *signal*, so it needs `/proc`, an interlock
    /// that identifies pid 1 and its child, and it cannot carry a reply. This
    /// crosses the supervisor's own socket, so the supervisor **is** the
    /// interlock — it refuses in its own words, from the state it is actually
    /// holding — and the answer comes back.
    ///
    /// There is no `can_` interlock of this adapter's own for that reason. One
    /// would be a second opinion about a machine this process is not the
    /// authority for, which is `ADR-011` §3.
    fn can_request_service_restart(
        &mut self,
        service: &str,
    ) -> Result<rivet_core::operation::Restarting, MachineError> {
        crate::status::restart(service, false)
    }

    fn request_service_restart(
        &mut self,
        service: &str,
    ) -> Result<rivet_core::operation::Restarting, MachineError> {
        crate::status::restart(service, true)
    }

    /// **`ADR-017` §6's held stop, over the same socket** — the restart's
    /// argument unchanged: the supervisor is the interlock and answers from the
    /// state it holds, so this adapter has no opinion of its own.
    fn can_request_service_hold(
        &mut self,
        kind: rivet_core::operation::ServiceHoldKind,
        service: &str,
    ) -> Result<rivet_core::operation::ServiceHold, MachineError> {
        crate::status::hold(kind, service, false)
    }

    fn request_service_hold(
        &mut self,
        kind: rivet_core::operation::ServiceHoldKind,
        service: &str,
    ) -> Result<rivet_core::operation::ServiceHold, MachineError> {
        crate::status::hold(kind, service, true)
    }

    /// `ADR-021` §3's floor, through the supervisor's socket, because the
    /// supervisor writes the stored clock and nothing else may.
    fn can_request_clock_floor_reset(
        &mut self,
    ) -> Result<rivet_core::operation::clock_floor::Floor, MachineError> {
        crate::status::clock_floor(false)
    }

    fn request_clock_floor_reset(
        &mut self,
    ) -> Result<rivet_core::operation::clock_floor::Floor, MachineError> {
        crate::status::clock_floor(true)
    }
}

/// The observation half, split out on 2026-08-24 when `rivet_core` cut
/// `Observed` from `Machine`.
///
/// **One type still answers both, and that is right for this adapter.** A
/// board is a machine you can both command and ask. What the split buys here
/// is not fewer implementations but a narrower one where it matters: `get log`
/// and `get storage` are handed `&mut dyn Observed` and could not arm a trial
/// boot if they tried.
impl<M: BootMedium, I: InitProcess + crate::reload::Supervisor> Observed for PiMachine<M, I> {
    /// `/proc`, which needs no `mount(2)`, no device tree and no pid 1 —
    /// [`Self::read_journal`]'s argument, and true of any Linux this runs on.
    fn read_processes(
        &mut self,
    ) -> Result<rivet_core::operation::machine::ProcessRead, MachineError> {
        crate::process::read()
    }
    /// `/proc/meminfo`, `/proc/pressure/memory` and the slab listings, which need no `mount(2)`,
    /// no device tree and no pid 1 — [`Self::read_journal`]'s argument, and it
    /// earns the same answer on a host that is not a Rivet: the numbers are
    /// true about whatever Linux this is running on.
    fn read_memory(&mut self) -> Result<rivet_core::operation::machine::MemoryRead, MachineError> {
        crate::memory::read()
    }
    /// `/proc/stat` and `/proc/loadavg`, on [`Self::read_journal`]'s argument
    /// once more. **It answers counters and not a utilisation**, because a
    /// utilisation is a rate and this port takes one reading — `crate::cpu` and
    /// `rivet_core::operation::cpu` carry the whole of that argument.
    fn read_cpu(&mut self) -> Result<rivet_core::operation::machine::CpuRead, MachineError> {
        crate::cpu::read()
    }
    /// `/system/trust`, on [`Self::read_journal`]'s argument once more — a file
    /// read, with no `mount(2)`, no device tree and no pid 1 behind it.
    ///
    /// **Read here rather than cached**, because `ADR-028` §5.5.4 makes
    /// revocation an image update that replaces the file. See `crate::trust`.
    fn read_trust(&mut self) -> Result<rivet_core::operation::machine::TrustRead, MachineError> {
        crate::trust::read()
    }
    /// `/sys/class/thermal` and `/sys/devices/system/cpu/*/cpufreq`, which need
    /// no `mount(2)`, no device tree and no pid 1 — [`Self::read_journal`]'s
    /// argument again, and it earns the same answer on any Linux: the zones and
    /// the clock are true about whatever machine this is running on.
    ///
    /// **It is not behind `BootMedium` and it is not on this type's two seams.**
    /// See `crate::thermal`, which holds the whole of the adapter half and the
    /// argument for the split the survey's §7.2 asked for.
    fn read_thermal(
        &mut self,
    ) -> Result<rivet_core::operation::machine::ThermalRead, MachineError> {
        crate::thermal::read()
    }
    /// `/proc/uptime` and `/proc/sys/kernel/random/boot_id`, on
    /// [`Self::read_journal`]'s argument once more. **The list of boots is not
    /// here**: `rivet_core::journal::boots_in_order` computes it from the
    /// journal this type already answers, and a second implementation would be
    /// two answers to which boot came first (`ADR-011` §3).
    fn read_boot(&mut self) -> Result<rivet_core::operation::machine::BootRead, MachineError> {
        crate::uptime::read()
    }

    /// `/sys/class/net` and `/proc/net`, on [`Self::read_journal`]'s argument
    /// once more: no `mount(2)`, no device tree, no pid 1. **It is not behind
    /// `rivet-adapter-net`** — that crate is a writer and has no dump
    /// operation, and `crate::network` carries the whole argument along with
    /// the one gap the decision leaves.
    fn read_network(
        &mut self,
    ) -> Result<rivet_core::operation::machine::NetworkRead, MachineError> {
        crate::network::read()
    }

    /// `/sys/class/block`, and **no mount at all** — which is what separates it
    /// from [`Self::read_filesystems`]. A partition that failed to mount is
    /// invisible to that one and present here, and on an A/B machine the
    /// inactive slot's `/system` is exactly that.
    fn read_devices(
        &mut self,
    ) -> Result<rivet_core::operation::machine::DeviceRead, MachineError> {
        crate::device::read()
    }

    /// **The one method on this port that uses BOTH of this type's seams**, and
    /// the only place they meet.
    ///
    /// The firmware's half — which boot partition it read, and whether this is a
    /// `tryboot` — comes off `medium`, which is the card. The kernel's half
    /// comes off `/proc/cmdline`, which is `crate::slot` and needs nothing at
    /// all. **Neither is folded into the other**: they answer different
    /// questions that should agree, and `Observed::read_slots` carries why the
    /// disagreement is worth being able to see.
    ///
    /// It does not violate this file's *"the two columns never touch"* rule,
    /// because that rule is about **arming** and **stopping** — the two
    /// operations that change a machine. This one changes nothing and only
    /// reads, which is exactly the split `Observed` was cut from `Machine` for.
    fn read_slots(&mut self) -> Result<rivet_core::operation::machine::SlotRead, MachineError> {
        // `Machine::running_slot` rather than `PiFirmware` directly: it already
        // holds the conversion from the firmware's partition NUMBER to a domain
        // `Slot`, including the refusal for a number outside `ADR-018` §4's
        // layout. A second conversion here would be `ADR-011` §3 over a value
        // whose wrongness names a partition an update would overwrite.
        let (booted, booted_unmeasured) = match Machine::running_slot(self) {
            Ok(slot) => (Some(slot), None),
            Err(e) => (None, Some(e.why().to_string())),
        };

        let (root_argument, root_unmeasured) = match crate::slot::root_argument() {
            Ok(argument) => (Some(argument), None),
            Err(why) => (None, Some(why)),
        };

        // **`None` and not `false` for a flag that was not read.** `false`
        // asserts *this is a committed boot*, and a trial that is not committed
        // rolls back at the next restart (`ADR-018` §5.1).
        let (in_trial_boot, trial_unmeasured) =
            match PiFirmware.in_trial_boot(&mut self.medium) {
                Ok(flag) => (Some(flag), None),
                Err(why) => (None, Some(why)),
            };

        // **What an ordinary restart would boot**, which `in_trial_boot` does
        // not say — see `SlotRead::default_slot`. Read second and never fatal:
        // this reading mounts a partition, and a machine whose default cannot be
        // read still has a firmware statement and a kernel command line worth
        // reporting.
        let (default_slot, default_unmeasured) = match PiFirmware.default_slot(&mut self.medium) {
            Ok(id) => match known_slot_or_reason(id) {
                Ok(slot) => (Some(slot), None),
                Err(why) => (None, Some(why)),
            },
            Err(why) => (None, Some(why)),
        };

        // Nothing measured at all is a refusal rather than an empty answer: a
        // `SlotRead` of all-`None` says this machine has no slots, which is a
        // claim about the card rather than about this build's reach.
        if booted.is_none() && root_argument.is_none() && in_trial_boot.is_none() {
            return Err(MachineError::Refused(format!(
                "neither the firmware's boot partition, the kernel's root= argument nor the \
                 tryboot flag could be read, so NOTHING about which slot this machine is running \
                 was measured. The firmware said: {}. The kernel command line said: {}.",
                booted_unmeasured.unwrap_or_default(),
                root_unmeasured.unwrap_or_default(),
            )));
        }

        Ok(rivet_core::operation::machine::SlotRead {
            booted,
            booted_unmeasured,
            booted_from: rivet_adapter_image::DT_BOOT_PARTITION.to_string(),
            root_argument,
            root_unmeasured,
            root_from: crate::slot::CMDLINE.to_string(),
            in_trial_boot,
            trial_unmeasured,
            default_slot,
            default_unmeasured,
        })
    }
    /// Reading it needs no `mount(2)`, no device tree and no pid 1 — the same
    /// argument [`Self::read_journal`] makes, and it earns the same answer on a
    /// host that is not a Rivet: *"there is no document at /state/config"* is a
    /// true statement, not a refusal.
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        crate::document::read()
    }
    /// **Files, and nothing this type's two seams touch** — see
    /// `crate::journal`, which is the whole of the adapter half.
    ///
    /// It is here rather than behind `BootMedium` because the journal is not on
    /// the boot medium: `ADR-022` §8 puts it on `/state`, which an update never
    /// touches, precisely so the previous boot's account survives one. A
    /// `BootMedium` method would have put a mount and a partition number in the
    /// path of a plain `read_to_string`.
    fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
        crate::journal::read()
    }

    /// **The kernel's mount table and `statvfs(3)`, and nothing this type's two
    /// seams touch** — see `crate::storage`, which is the whole of the adapter
    /// half.
    ///
    /// It is here rather than behind `BootMedium` for `read_journal`'s reason
    /// and a sharper one: the filesystems this answers about include the boot
    /// medium and are not *of* it. A `BootMedium` method would have put a mount
    /// and a partition number in the path of a question whose answer includes
    /// the machine's `tmpfs`.
    fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
        crate::storage::read()
    }

    /// `ADR-006` §7, as a subject of `get` since 2026-08-25.
    ///
    /// **This is the whole reason the port method exists.** `ADR-014` §3.2
    /// makes `rivet-config` depend on `rivet-core`, so the core cannot
    /// name a schema; this binary has both crates and hands the value over.
    /// It answers identically on every platform, because the document a build
    /// accepts is a property of the build rather than of the machine it is
    /// running on — which is why the OTHER implementor below answers the same
    /// way rather than refusing.
    /// `ADR-032` §3.2's query socket. **The only method on this port that asks
    /// another PROCESS** — everything else here reads a file or the kernel.
    /// `crate::status` is the client, and the refusal it produces when nothing
    /// is listening is §3.3 item 5's: it names the socket and points at the
    /// journal, which is on disk and survives a supervisor that is not.
    fn supervisor_status(&mut self) -> Result<rivet_core::operation::Status, MachineError> {
        crate::status::ask()
    }

    /// **R-168, added 2026-09-13**: the one place this answers DIFFERENTLY
    /// from the `Detached` implementor below, and the doc comment above still
    /// describes why every OTHER machine-agnostic fact does not — a zone
    /// database is a property of THIS image (`crate::zoneinfo`'s own module
    /// documentation), not of the build, so overlaying it here rather than in
    /// `rivet_config::schema()` itself is what keeps that function pure
    /// (`ADR-014` §3). A database this machine cannot read leaves the field
    /// exactly as `rivet_config::schema()` built it — free text, never a
    /// refusal of the whole subject over one unreadable directory.
    fn configuration_schema(&mut self) -> rivet_core::Value {
        let schema = rivet_config::schema();
        match crate::zoneinfo::zone_names() {
            Some(zones) => rivet_config::schema::with_timezone_zones(schema, &zones),
            None => schema,
        }
    }
}

/// **The third port, added 2026-08-24.**
///
/// ~~And it refuses on every platform, including a board … how a payload
/// reaches the installer is not ruled (the maintainer's 2026-08-24 install-request ruling, `ADR-029` §14.4).~~
/// **Superseded the same evening**, when the maintainer ruled the channel:
/// `rivet apply install` writes a request onto `/state` and the supervisor
/// takes it up on its next convergence pass (`ADR-017` §3.2.2). It is forwarded
/// to `crate::deploy`, which is where the whole of it is argued.
///
/// **It is not in this file's diagram and that is deliberate.** `PiMachine`
/// holds two platform seams that never meet — the card, and pid 1 — and this is
/// neither. It has a seam of its own, `crate::deploy::RequestStore`, and the
/// forwarding below is what keeps this type from acquiring a third.
impl<M: BootMedium, I: InitProcess + crate::reload::Supervisor> Deployer for PiMachine<M, I> {
    fn can_request_install(
        &mut self,
        digest: &rivet_core::artifact::Digest,
    ) -> Result<rivet_core::operation::Held, MachineError> {
        let running = self.booted_slot();
        let default = self.default_slot();
        crate::deploy::on_this_machine()
            .running_slot(running)
            .default_slot(Some(default))
            .can_request_install(digest)
    }

    fn request_install(
        &mut self,
        digest: &rivet_core::artifact::Digest,
    ) -> Result<rivet_core::operation::Held, MachineError> {
        let running = self.booted_slot();
        let default = self.default_slot();
        crate::deploy::on_this_machine()
            .running_slot(running)
            .default_slot(Some(default))
            .request_install(digest)
    }

    fn can_request_uninstall(&mut self, name: &str) -> Result<(), MachineError> {
        crate::deploy::on_this_machine().can_request_uninstall(name)
    }

    fn request_uninstall(&mut self, name: &str, purge: bool) -> Result<(), MachineError> {
        crate::deploy::on_this_machine().request_uninstall(name, purge)
    }

    fn can_request_library_uninstall(
        &mut self,
        selector: &rivet_core::uninstall_library::Selector,
    ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
        crate::deploy::on_this_machine().can_request_library_uninstall(selector)
    }

    fn request_library_uninstall(
        &mut self,
        selector: &rivet_core::uninstall_library::Selector,
    ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
        crate::deploy::on_this_machine().request_library_uninstall(selector)
    }

    fn can_request_repo_delete(&mut self, name: &str) -> Result<(), MachineError> {
        crate::deploy::on_this_machine().can_request_repo_delete(name)
    }

    fn request_repo_delete(&mut self, name: &str, id: &str) -> Result<(), MachineError> {
        crate::deploy::on_this_machine().request_repo_delete(name, id)
    }
}

/// **The fourth port, later on 2026-08-24** — `ADR-006` §8.9's `apply edit`.
///
/// Forwarded to `crate::edit` for `Deployer`'s reason, in the same words: this
/// type holds two platform seams that never meet, the card and pid 1, and
/// editing a file is neither. It has a seam of its own,
/// `crate::edit::FileStore`, and the forwarding below is what keeps this type
/// from acquiring a fourth.
impl<M: BootMedium, I: InitProcess + crate::reload::Supervisor> rivet_core::operation::Editor for PiMachine<M, I> {
    fn principal(&mut self) -> rivet_core::deploy::Principal {
        crate::deploy::this_process()
    }

    /// **The interlock runs before the read**, which is `crate::shutdown`'s rule
    /// applied to a different hazard: a plan that read a file on a development
    /// host and then refused would have described an edit to a file the operator
    /// never meant.
    fn read_file(
        &mut self,
        path: &str,
    ) -> Result<rivet_core::operation::FileRead, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().read_file(path)
    }

    /// **FS4, forwarded rather than defaulted.** `Editor::read_file_window`'s
    /// default reads the whole file through `read_file` above, which is
    /// exactly the allocation this override exists to avoid on the real
    /// machine — `crate::edit::on_this_machine()` is `Files<Filesystem>`, and
    /// its own override is the one that actually does the positioned read.
    fn read_file_window(
        &mut self,
        path: &str,
        at: usize,
        len: usize,
    ) -> Result<Option<rivet_core::operation::FileWindow>, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().read_file_window(path, at, len)
    }

    fn can_write_file(&mut self, path: &str, contents: &str) -> Result<(), MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().can_write_file(path, contents)
    }

    fn write_file(
        &mut self,
        write: &rivet_core::operation::Write<'_>,
    ) -> Result<rivet_core::operation::Wrote, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().write_file(write)
    }

    /// **`ADR-006` §8.10's four, added 2026-08-29**, and the interlock runs
    /// before each of them for the reason [`rivet_core::operation::Editor::read_file`] gives — sharpened,
    /// because three of these destroy or move something. A plan computed on a
    /// development host and then refused would have described a removal from a
    /// machine this build is not running on.
    fn stat_path(
        &mut self,
        path: &str,
    ) -> Result<rivet_core::operation::PathStat, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().stat_path(path)
    }

    fn resolve_owner(
        &mut self,
        user: Option<&str>,
        group: Option<&str>,
    ) -> Result<rivet_core::operation::OwnerIds, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().resolve_owner(user, group)
    }

    fn can_change_path(
        &mut self,
        change: &rivet_core::operation::PathChange<'_>,
    ) -> Result<(), MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().can_change_path(change)
    }

    fn change_path(
        &mut self,
        change: &rivet_core::operation::PathChange<'_>,
    ) -> Result<rivet_core::operation::Wrote, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        crate::edit::on_this_machine().change_path(change)
    }

    /// **`getrandom(2)`, which blocks until the kernel's pool is ready**
    /// rather than returning weak bytes — `ADR-063` §4's enrollment code. A
    /// syscall rather than `/dev/urandom`, because it needs no device node.
    fn random_bytes(&mut self, len: usize) -> Result<Vec<u8>, MachineError> {
        crate::edit::this_is_a_rivet_machine()?;
        getrandom_bytes(len)
    }

    /// SHA-256 by `sha2`, for `get fleet-root`'s fingerprint (`ADR-063` §4).
    fn sha256(&mut self, bytes: &[u8]) -> Result<[u8; 32], MachineError> {
        Ok(sha256_of(bytes))
    }
}

/// SHA-256, the one place this binary computes it for an operation.
fn sha256_of(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).into()
}

#[cfg(test)]
mod sha256_tests {
    /// FIPS 180-2's "abc" vector — the digest `get fleet-root` answers is the
    /// standard one, so a trust store's fingerprint matches it.
    #[test]
    fn the_digest_is_the_standard_sha256() {
        let hex: String = super::sha256_of(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}

/// Off Linux there is no board and no `getrandom(2)`: refused, never faked.
#[cfg(not(target_os = "linux"))]
fn getrandom_bytes(len: usize) -> Result<Vec<u8>, MachineError> {
    Err(MachineError::Refused(format!(
        "{len} random bytes were asked for off a Rivet machine; NOTHING WAS DONE"
    )))
}

/// Fill `len` bytes from `getrandom(2)`, retrying an interrupted or short read.
#[cfg(target_os = "linux")]
fn getrandom_bytes(len: usize) -> Result<Vec<u8>, MachineError> {
    let mut out = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        let rest = &mut out[filled..];
        // SAFETY: `rest` is a live, writable buffer of exactly `rest.len()`
        // bytes, and `getrandom` writes at most that many.
        let got = unsafe { libc::getrandom(rest.as_mut_ptr().cast(), rest.len(), 0) };
        if got < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(MachineError::Refused(format!(
                "getrandom(2) failed, so no random bytes were drawn and NOTHING WAS DONE: {error}"
            )));
        }
        filled += usize::try_from(got).unwrap_or(0);
    }
    Ok(out)
}

/// A slot, as the firmware names one.
///
/// `rivet_core::state::Slot::boot_partition` is the authority for the number
/// (`ADR-011`), and this is the only direction the conversion runs in code above
/// the port — the reverse is [`Slot::from_boot_partition`], in `running_slot`,
/// where it can refuse.
const fn to_slot_id(slot: Slot) -> SlotId {
    SlotId(slot.boot_partition())
}

// ---------------------------------------------------------------------------
// The medium, over `crate::syscall`.
// ---------------------------------------------------------------------------

/// How long this command waits for another process to release
/// `rivet_adapter_image::AUTOBOOT_LOCK` before answering that the partition
/// is busy.
///
/// **Ten seconds, because the longest a holder keeps it is one commit or one
/// arming**: a mount, a read, a write of a few hundred bytes, two `sync(2)`s,
/// a rename and an unmount of a FAT filesystem — well under a second on a
/// healthy card, so ten is an order of magnitude of margin for a slow one.
/// PID 1 never waits on it, so a holder is never itself waiting on this
/// process. Longer would leave an operator's `get slot` hanging on a card that
/// has stopped answering; the answer *busy* is worth more than the wait.
#[cfg(any(test, target_os = "linux"))]
pub const LOCK_WAIT_MS: u64 = 10_000;

/// How often a waiting command tries the lock again.
#[cfg(any(test, target_os = "linux"))]
pub const LOCK_POLL_MS: u64 = 50;

/// `rivet_adapter_image::BootMedium` over Linux.
///
/// It holds one thing: the record's lock, from [`BootMedium::lock`] to
/// [`BootMedium::unlock`]. Every fact it deals in is read at the point of use.
#[cfg(target_os = "linux")]
pub struct SyscallMedium {
    lock: Option<crate::syscall::HeldLock>,
}

#[cfg(target_os = "linux")]
impl SyscallMedium {
    pub const fn new() -> Self {
        Self { lock: None }
    }
}

/// What a `mount(2)` at `at` came to.
///
/// **`EBUSY` alone is not "already mounted here"; `at_is_mount_point` is
/// what makes it true.** `cmd/rivet-deploy/src/kernel.rs`'s port
/// documentation carries the bench-measured case where `EBUSY` means
/// something else: this device mounted elsewhere with an incompatible flag.
/// A mount found in place is [`Mounted::Adopted`]; every caller here holds
/// the record's lock, so `rivet_adapter_image` reclaims it as an orphan.
#[cfg(any(test, target_os = "linux"))]
fn mount_outcome(
    result: Result<(), i32>,
    at_is_mount_point: impl FnOnce() -> bool,
) -> Result<Mounted, Errno> {
    match result {
        Ok(()) => Ok(Mounted::Made),
        Err(errno) if Errno(errno) == Errno::EBUSY && at_is_mount_point() => Ok(Mounted::Adopted),
        Err(errno) => Err(Errno(errno)),
    }
}

#[cfg(target_os = "linux")]
impl BootMedium for SyscallMedium {
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        // Bytes rather than text, because a device-tree property may be a
        // big-endian cell — deciding which is above this seam, in the port.
        std::fs::read(path).map_err(to_errno)
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        std::fs::read_to_string(path).map_err(to_errno)
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        std::fs::write(path, contents).map_err(to_errno)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        std::fs::rename(from, to).map_err(to_errno)
    }

    fn sync(&mut self) {
        crate::syscall::sync();
    }

    fn mount_boot_filesystem(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno> {
        // `/run` is a tmpfs PID 1 mounts at boot (`rivet_core::mount`), so the
        // mount point is created rather than assumed to exist.
        std::fs::create_dir_all(at).map_err(to_errno)?;
        mount_outcome(crate::syscall::mount(device, at, fstype, writable), || {
            is_mount_point(at)
        })
    }

    /// Waits up to [`LOCK_WAIT_MS`] for another holder, then answers
    /// `EAGAIN` — the operator gets *busy*, never a hang.
    fn lock(&mut self, path: &str) -> Result<(), Errno> {
        let held = crate::syscall::lock_exclusive_within(path, LOCK_WAIT_MS, LOCK_POLL_MS)
            .map_err(Errno)?;
        self.lock = Some(held);
        Ok(())
    }

    fn unlock(&mut self, _path: &str) {
        self.lock = None;
    }

    /// **Into the journal, as a warning**, through the ingest socket every
    /// record this binary writes goes through — and onto stderr if the
    /// supervisor is not listening, which is `crate::journal::send_or_say`'s
    /// floor. Not stdout, which is the operator's structured answer.
    fn reclaimed_orphan(&mut self, at: &str) {
        use crate::edit::FileStore as _;
        let stamp = crate::edit::Filesystem.stamp();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let corr = format!("{:08x}{:08x}", (stamp.mono * 1000.0) as u32, 1u32);
        let record = rivet_core::journal::Record {
            ts: stamp.wall_ms.map(rivet_core::journal::rfc3339),
            boot: stamp.boot,
            mono: stamp.mono,
            sev: rivet_core::journal::Severity::Warn,
            svc: crate::edit::SELF_NAME.to_string(),
            corr,
            actor: crate::edit::ACTOR.to_string(),
            msg: reclaim_message(at),
            fields: Vec::new(),
        };
        let _ = crate::journal::send_or_say(&record.to_jsonl());
    }

    fn unmount(&mut self, at: &str) {
        // The port gives the outcome to the implementor because no caller could
        // act on it. PID 1 says so on the console; this process has no console —
        // it is a `-c` invocation whose stdout is load-bearing
        // (measured on real hardware: a stray byte there costs a failed `scp`), and
        // stderr belongs to the refusal surface. So a failed unmount of a
        // tmpfs-backed mount point in a process that is about to exit is
        // deliberately not reported: the kernel drops it when the process's
        // last reference goes, and the alternative is prose on a stream an
        // `scp` client parses.
        //
        // **What that costs, so it is not discovered on a board.** A mount is
        // process-independent, so a failed unmount here outlives this process
        // and the *next* lock-holder finds it (EBUSY at a mount point). Holding
        // the record's lock, that caller knows nobody is using it, so
        // `rivet_adapter_image::mount_autoboot_partition` reclaims it —
        // unmounts it, mounts afresh, and journals the reclaim. If that unmount
        // fails too, the access is refused rather than made through the leak.
        // It has not been seen; `umount2(2)` on a FAT filesystem with no open
        // files does not usually refuse.
        let _ = crate::syscall::unmount(at);
    }
}

/// What this binary journals when it reclaims an orphaned mount.
#[cfg(any(test, target_os = "linux"))]
fn reclaim_message(at: &str) -> String {
    format!(
        "reclaimed an orphaned mount at {at}: it was unmounted and the partition mounted afresh, \
         under the default-slot record's lock. Some process died, or failed to unmount, while \
         it held the partition"
    )
}

/// One `std::io::Error`, as the number the port speaks in.
#[cfg(target_os = "linux")]
fn to_errno(error: std::io::Error) -> Errno {
    Errno(crate::syscall::errno_of(&error))
}

/// Is anything mounted at `path`?
///
/// **A copy of `cmd/rivet-deploy/src/kernel.rs`'s function of the same name,
/// not a shared one** — the two are separate composition roots and neither
/// depends on the other. Keep the two in agreement if either changes; the
/// reasoning for the comparison itself is not repeated here, only there.
///
/// Compares device numbers rather than reading `/proc/self/mountinfo`: a
/// mount point is by definition a directory whose filesystem differs from
/// its parent's, so `st_dev` answers it in two `stat(2)` calls with nothing
/// to parse.
///
/// A path that cannot be stat'd is **not** a mount point, which is the safe
/// direction here: the caller then reports the `EBUSY` rather than treating
/// an unreadable path as the convergent answer.
#[cfg(target_os = "linux")]
fn is_mount_point(path: &str) -> bool {
    use std::os::linux::fs::MetadataExt as _;
    let Ok(here) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(parent) = std::fs::metadata(format!("{}/..", path.trim_end_matches('/'))) else {
        return false;
    };
    here.st_dev() != parent.st_dev()
}

// ---------------------------------------------------------------------------
// Everywhere that is not Linux.
// ---------------------------------------------------------------------------

/// A machine this build cannot reach, which **refuses** rather than pretending.
///
/// `AGENTS.md` §3: *"Stubs refuse; they never fake success. An unsupported-
/// platform implementation returns an error. Never `Ok(())` — a `run` that
/// succeeded without running reports a state that is not true."*
///
/// It exists because the development host is Windows (`AGENTS.md` §10.5) and
/// `mount(2)` is Linux's. Every read-only operation this binary has — `explain`,
/// `schema`, `--version`, the whole login-shell contract — works unchanged there,
/// which is the property `src/main.rs` claims and this keeps true.
///
/// **Compiled where it is needed and where it is checked**, rather than always
/// and behind a `dead_code` allow: on a platform that is not Linux it is the
/// machine, and under `cfg(test)` it is a contract with tests. An allow would be
/// a place for a second unused thing to hide.
#[cfg(any(test, not(target_os = "linux")))]
pub struct Detached {
    platform: &'static str,
}

#[cfg(any(test, not(target_os = "linux")))]
impl Detached {
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }

    fn refusal(&self) -> MachineError {
        MachineError::Refused(format!(
            "this rivet was built for {}, and reading a machine's boot slots needs Linux's \
             mount(2) and its device tree (ADR-018 §5). Every read-only operation works here; \
             run this one on the board.",
            self.platform
        ))
    }

    /// **A separate sentence, because it is a separate reason.** An operator on
    /// a workstation who was told "this needs mount(2) and a device tree" after
    /// asking for a power off would go looking for the wrong thing.
    fn stop_refusal(&self) -> MachineError {
        MachineError::Unsafe(format!(
            "this rivet was built for {}, which has no pid 1 to ask. Rivet stops a machine \
             by signalling its own pid 1 (ADR-017 §3.1.5 item 6), and this build cannot read \
             one, let alone identify it. NOTHING WAS SENT. {}",
            self.platform,
            rivet_core::control::ASK_A_BOARD
        ))
    }

    /// See [`Machine::can_request_reload`] on this type.
    fn reload_refusal(&self) -> MachineError {
        MachineError::Unsafe(format!(
            "this rivet was built for {}, which has no Rivet supervisor to ask. A reload is \
             delivered to this machine's own `rivet-sysd` (ADR-032 §3.2), and this build cannot \
             read a /proc to find one. NOTHING WAS SENT. {}",
            self.platform,
            rivet_core::control::ASK_A_BOARD
        ))
    }

    /// `ADR-017` §6.3.2's, and a fourth distinct sentence — see the two methods
    /// that use it.
    fn service_restart_refusal(&self, service: &str) -> MachineError {
        MachineError::Unsafe(format!(
            "this rivet was built for {}, which runs no Rivet supervisor, so there is nothing \
             listening on {} to ask about `{service}`. NOTHING WAS SENT — and nothing on this \
             host was restarted or stopped either. {}",
            self.platform,
            rivet_core::operation::QUERY_SOCKET_PATH,
            rivet_core::control::ASK_A_BOARD
        ))
    }
}

#[cfg(any(test, not(target_os = "linux")))]
impl Machine for Detached {
    fn running_slot(&mut self) -> Result<Slot, MachineError> {
        Err(self.refusal())
    }

    fn can_arm_trial_boot(&mut self, _slot: Slot) -> Result<(), MachineError> {
        Err(self.refusal())
    }

    fn arm_trial_boot(&mut self, _slot: Slot) -> Result<Reboot, MachineError> {
        Err(self.refusal())
    }

    /// **A detached build refuses to stop a machine, and it is the same refusal
    /// `crate::shutdown` gives for the same reason.** A Windows host has no pid 1
    /// to identify and nothing to signal; saying `Ok` would report a machine
    /// asked to stop that was never asked anything.
    fn can_request_stop(&mut self, _stop: Stop) -> Result<(), MachineError> {
        Err(self.stop_refusal())
    }

    fn request_stop(&mut self, _stop: Stop) -> Result<(), MachineError> {
        Err(self.stop_refusal())
    }

    /// **A third sentence, because it is a third reason** — the same discipline
    /// [`Self::stop_refusal`] exists for. An operator told *this needs mount(2)*
    /// after asking for a reload would go looking for a card problem.
    fn can_request_reload(&mut self) -> Result<(), MachineError> {
        Err(self.reload_refusal())
    }

    fn request_reload(&mut self) -> Result<(), MachineError> {
        Err(self.reload_refusal())
    }

    /// **A fourth sentence, because it is a fourth reason.** A reload needs a
    /// `/proc` this build cannot read; this needs a *supervisor listening on a
    /// socket*, which is a different absence and a different next move.
    fn can_request_service_restart(
        &mut self,
        service: &str,
    ) -> Result<rivet_core::operation::Restarting, MachineError> {
        Err(self.service_restart_refusal(service))
    }

    fn request_service_restart(
        &mut self,
        service: &str,
    ) -> Result<rivet_core::operation::Restarting, MachineError> {
        Err(self.service_restart_refusal(service))
    }

    /// The same absence as the restart's — no supervisor listening — so the
    /// same sentence.
    fn can_request_service_hold(
        &mut self,
        _kind: rivet_core::operation::ServiceHoldKind,
        service: &str,
    ) -> Result<rivet_core::operation::ServiceHold, MachineError> {
        Err(self.service_restart_refusal(service))
    }

    fn request_service_hold(
        &mut self,
        _kind: rivet_core::operation::ServiceHoldKind,
        service: &str,
    ) -> Result<rivet_core::operation::ServiceHold, MachineError> {
        Err(self.service_restart_refusal(service))
    }
}

#[cfg(any(test, not(target_os = "linux")))]
/// What a detached build can still answer, and what it cannot.
///
/// The asymmetry below is the point of this type, and the split makes it
/// legible: a host with no board can read a journal copied off a card and
/// cannot see the board's filesystems.
impl Observed for Detached {
    /// `/proc`, which needs no `mount(2)`, no device tree and no pid 1 —
    /// [`Self::read_journal`]'s argument, and true of any Linux this runs on.
    fn read_processes(
        &mut self,
    ) -> Result<rivet_core::operation::machine::ProcessRead, MachineError> {
        crate::process::read()
    }
    /// `/proc/meminfo`, `/proc/pressure/memory` and the slab listings, which need no `mount(2)`,
    /// no device tree and no pid 1 — [`Self::read_journal`]'s argument, and it
    /// earns the same answer on a host that is not a Rivet: the numbers are
    /// true about whatever Linux this is running on.
    fn read_memory(&mut self) -> Result<rivet_core::operation::machine::MemoryRead, MachineError> {
        crate::memory::read()
    }
    /// **The real reader, and honest on either host.** A Linux development host
    /// has a `/proc/stat` and this answers about it; a Windows one has none and
    /// `crate::cpu` refuses, naming the file. Neither is a claim about a board,
    /// and neither is a utilisation — see `rivet_core::operation::cpu`.
    fn read_cpu(&mut self) -> Result<rivet_core::operation::machine::CpuRead, MachineError> {
        crate::cpu::read()
    }
    /// **The real reader, and it is honest on a workstation for the reason a
    /// fake would not be**: `/system/trust` is an absolute path that exists on a
    /// Rivet and on nothing else, so a detached build reads it, finds nothing,
    /// and refuses **naming the file**. That is a true statement about the
    /// machine this is running on rather than a platform refusal this type had
    /// to invent — and it is exactly the sentence an operator needs, because the
    /// same refusal on a *board* means the image was built `--no-trust`.
    fn read_trust(&mut self) -> Result<rivet_core::operation::machine::TrustRead, MachineError> {
        crate::trust::read()
    }
    /// **The real reader here too, and on Windows it refuses of its own
    /// accord.** `crate::thermal` reads `/sys`, so on a Linux development host
    /// it answers honestly about that host and on a Windows one it reports that
    /// neither interface exists — which is a true statement rather than a
    /// platform refusal this type had to invent.
    fn read_thermal(
        &mut self,
    ) -> Result<rivet_core::operation::machine::ThermalRead, MachineError> {
        crate::thermal::read()
    }
    /// **The real reader, and honest on either host.** A Linux development host
    /// has a `/proc/uptime` and this answers about it; a Windows one has none
    /// and `crate::uptime` refuses, naming the file. Neither is a claim about a
    /// board.
    fn read_boot(&mut self) -> Result<rivet_core::operation::machine::BootRead, MachineError> {
        crate::uptime::read()
    }

    /// `/sys/class/net` and `/proc/net`, on [`Self::read_journal`]'s argument
    /// once more: no `mount(2)`, no device tree, no pid 1. **It is not behind
    /// `rivet-adapter-net`** — that crate is a writer and has no dump
    /// operation, and `crate::network` carries the whole argument along with
    /// the one gap the decision leaves.
    fn read_network(
        &mut self,
    ) -> Result<rivet_core::operation::machine::NetworkRead, MachineError> {
        crate::network::read()
    }

    /// `/sys/class/block`, and **no mount at all** — which is what separates it
    /// from [`Self::read_filesystems`]. A partition that failed to mount is
    /// invisible to that one and present here, and on an A/B machine the
    /// inactive slot's `/system` is exactly that.
    fn read_devices(
        &mut self,
    ) -> Result<rivet_core::operation::machine::DeviceRead, MachineError> {
        crate::device::read()
    }

    /// **The half a detached build can honestly answer, and the halves it
    /// cannot** — which makes this the clearest example of why the three
    /// readings are separate fields rather than one.
    ///
    /// A workstation has no device tree and no `tryboot`, so the firmware's
    /// boot partition and the trial flag are **unmeasured** and say so. It may
    /// well have a `/proc/cmdline`, and reading it is honest: what comes back
    /// is that host's own `root=`, which will not be a Rivet `PARTLABEL=` and
    /// will therefore resolve to no slot. That is the right answer rather than
    /// a refusal — the machine really was started that way.
    fn read_slots(&mut self) -> Result<rivet_core::operation::machine::SlotRead, MachineError> {
        let (root_argument, root_unmeasured) = match crate::slot::root_argument() {
            Ok(argument) => (Some(argument), None),
            Err(why) => (None, Some(why)),
        };

        // Nothing at all — a Windows host — is a refusal naming the platform,
        // for `crate::storage`'s reason: *"there are no slots"* would be false
        // about a machine that simply is not this one.
        if root_argument.is_none() {
            return Err(MachineError::Refused(format!(
                "this rivet was built for {}, which has no device tree to name a boot partition \
                 and no tryboot flag — and its kernel command line could not be read either, so \
                 NOTHING about A/B slots was measured. {}",
                self.platform,
                rivet_core::control::ASK_A_BOARD
            )));
        }

        Ok(rivet_core::operation::machine::SlotRead {
            default_slot: None,
            default_unmeasured: Some(format!(
                "this rivet was built for {}, which has no autoboot.txt to read a default slot \
                 from",
                self.platform
            )),
            booted: None,
            booted_unmeasured: Some(format!(
                "this rivet was built for {}, which has no {} to read a boot partition from",
                self.platform,
                rivet_adapter_image::DT_BOOT_PARTITION
            )),
            booted_from: rivet_adapter_image::DT_BOOT_PARTITION.to_string(),
            root_argument,
            root_unmeasured,
            root_from: crate::slot::CMDLINE.to_string(),
            in_trial_boot: None,
            trial_unmeasured: Some(format!(
                "this rivet was built for {}, which has no ADR-018 §5 one-shot boot to be in",
                self.platform
            )),
        })
    }
    /// Reading it needs no `mount(2)`, no device tree and no pid 1 — the same
    /// argument [`Self::read_journal`] makes, and it earns the same answer on a
    /// host that is not a Rivet: *"there is no document at /state/config"* is a
    /// true statement, not a refusal.
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        crate::document::read()
    }
    /// **The one operation on this port a detached build does NOT refuse.**
    ///
    /// Reading the journal needs no `mount(2)`, no device tree and no pid 1 —
    /// it is `read_to_string` over absolute paths, and on a machine that is not
    /// a Rivet it finds nothing. *"There is no journal at /state/log"* is a
    /// true statement about this host; a refusal would have been a stub
    /// pretending the question could not be asked (`AGENTS.md` §3).
    ///
    /// It also makes `rivet get log` testable on the development host against
    /// a card's `/state` copied out of an image, which is the only way this
    /// operation gets exercised without a board.
    fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
        crate::journal::read()
    }

    /// **And this one it does refuse**, which is the other half of the paragraph
    /// above and the reason the two are worth stating together.
    ///
    /// A Windows host has filesystems and this build cannot see them, so *"there
    /// are none"* would be false about a machine that plainly has some — unlike
    /// *"there is no journal at /state/log"*, which is true. The refusal is
    /// `crate::storage`'s, which owns the sentence.
    fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
        Err(crate::storage::refusal(self.platform))
    }

    /// **Answered, not refused, on a detached build.** Everything else on
    /// this implementor refuses because it would be describing a machine this
    /// build is not running on. The schema is not a fact about a machine: it
    /// is what THIS BUILD accepts, and a workstation asking it gets the right
    /// answer — which is the point of being able to ask before writing a
    /// document (`ADR-006` §7).
    /// A detached build has no supervisor to ask, and says so in its own words
    /// rather than the socket client's — the platform is the reason here, not a
    /// process that is down.
    fn supervisor_status(&mut self) -> Result<rivet_core::operation::Status, MachineError> {
        Err(MachineError::Refused(format!(
            "this rivet was built for {}, which has no Rivet supervisor to ask. `get status` is \
             the one question that must be asked of a running process (ADR-032 §3.2). {}",
            self.platform,
            rivet_core::control::ASK_A_BOARD
        )))
    }

    /// **R-168.** Same overlay as `PiMachine`'s, and harmless here for the
    /// same reason the rest of this file's doc comment gives: a development
    /// host has no `rivet_core::etc::ZONEINFO_DIR`, so `crate::zoneinfo::
    /// zone_names` answers `None` and this is `rivet_config::schema()`
    /// unchanged — but a `Detached` build run against a mounted card's copy of
    /// `/system/share/zoneinfo` (`crate::journal::read`'s own precedent for
    /// what this implementor can see) gets the real dropdown rather than a
    /// second, silently different answer from `PiMachine`'s.
    fn configuration_schema(&mut self) -> rivet_core::Value {
        let schema = rivet_config::schema();
        match crate::zoneinfo::zone_names() {
            Some(zones) => rivet_config::schema::with_timezone_zones(schema, &zones),
            None => schema,
        }
    }
}

/// **And a detached build refuses to install.**
///
/// ~~This is the one port where the two implementations are identical, and the
/// identity is the point rather than an economy: what is missing is not a
/// platform but a *ruling*, so a Windows host and a Raspberry Pi are in exactly
/// the same position and telling them apart would be inventing a
/// distinction.~~
///
/// **Superseded 2026-08-24 evening**, and the sentence it replaces is worth
/// keeping because it was right about the reason and the reason changed. The
/// channel is ruled, so a board *can* now do this and a workstation cannot —
/// they are no longer in the same position, and the distinction is a real one
/// rather than an invented one. `crate::deploy::refusal` names the platform for
/// exactly that: *you are not on the machine*, which is a different next action
/// from *this cannot be done anywhere*.
#[cfg(any(test, not(target_os = "linux")))]
impl Deployer for Detached {
    fn can_request_install(
        &mut self,
        digest: &rivet_core::artifact::Digest,
    ) -> Result<rivet_core::operation::Held, MachineError> {
        crate::deploy::NoMachine::new(self.platform).can_request_install(digest)
    }

    fn request_install(
        &mut self,
        digest: &rivet_core::artifact::Digest,
    ) -> Result<rivet_core::operation::Held, MachineError> {
        crate::deploy::NoMachine::new(self.platform).request_install(digest)
    }

    fn can_request_uninstall(&mut self, name: &str) -> Result<(), MachineError> {
        crate::deploy::NoMachine::new(self.platform).can_request_uninstall(name)
    }

    fn request_uninstall(&mut self, name: &str, purge: bool) -> Result<(), MachineError> {
        crate::deploy::NoMachine::new(self.platform).request_uninstall(name, purge)
    }

    fn can_request_library_uninstall(
        &mut self,
        selector: &rivet_core::uninstall_library::Selector,
    ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
        crate::deploy::NoMachine::new(self.platform).can_request_library_uninstall(selector)
    }

    fn request_library_uninstall(
        &mut self,
        selector: &rivet_core::uninstall_library::Selector,
    ) -> Result<rivet_core::uninstall_library::Removal, MachineError> {
        crate::deploy::NoMachine::new(self.platform).request_library_uninstall(selector)
    }

    fn can_request_repo_delete(&mut self, name: &str) -> Result<(), MachineError> {
        crate::deploy::NoMachine::new(self.platform).can_request_repo_delete(name)
    }

    fn request_repo_delete(&mut self, name: &str, id: &str) -> Result<(), MachineError> {
        crate::deploy::NoMachine::new(self.platform).request_repo_delete(name, id)
    }
}

/// **And a detached build refuses to edit a file**, which is the one refusal in
/// this type that is about *which machine* rather than about a missing
/// platform.
///
/// A Windows host has files this build could replace perfectly well, and that is
/// exactly why it must not: the paths `apply edit` is for are a Rivet
/// machine's, and the command mistyped into a development checkout would rewrite
/// a file on the maintainer's own disk (`AGENTS.md` §9). `crate::edit::refusal`
/// owns the sentence.
#[cfg(any(test, not(target_os = "linux")))]
impl rivet_core::operation::Editor for Detached {
    fn principal(&mut self) -> rivet_core::deploy::Principal {
        crate::deploy::this_process()
    }

    fn read_file(
        &mut self,
        path: &str,
    ) -> Result<rivet_core::operation::FileRead, MachineError> {
        crate::edit::NoMachine::new(self.platform).read_file(path)
    }

    fn can_write_file(&mut self, path: &str, contents: &str) -> Result<(), MachineError> {
        crate::edit::NoMachine::new(self.platform).can_write_file(path, contents)
    }

    fn write_file(
        &mut self,
        write: &rivet_core::operation::Write<'_>,
    ) -> Result<rivet_core::operation::Wrote, MachineError> {
        crate::edit::NoMachine::new(self.platform).write_file(write)
    }

    /// `ADR-006` §8.10's four, added 2026-08-29. Same refusal, and the argument
    /// above applies with more force to three of them: a Windows host has files
    /// this build could remove, and after a removal there is nothing left to
    /// look at.
    fn stat_path(
        &mut self,
        path: &str,
    ) -> Result<rivet_core::operation::PathStat, MachineError> {
        crate::edit::NoMachine::new(self.platform).stat_path(path)
    }

    fn resolve_owner(
        &mut self,
        user: Option<&str>,
        group: Option<&str>,
    ) -> Result<rivet_core::operation::OwnerIds, MachineError> {
        crate::edit::NoMachine::new(self.platform).resolve_owner(user, group)
    }

    fn can_change_path(
        &mut self,
        change: &rivet_core::operation::PathChange<'_>,
    ) -> Result<(), MachineError> {
        crate::edit::NoMachine::new(self.platform).can_change_path(change)
    }

    fn change_path(
        &mut self,
        change: &rivet_core::operation::PathChange<'_>,
    ) -> Result<rivet_core::operation::Wrote, MachineError> {
        crate::edit::NoMachine::new(self.platform).change_path(change)
    }
}

/// The machine this build can reach, whatever that is.
///
/// The one place the platform is chosen, so that `main` is the same four lines
/// it was before this file existed.
pub fn machine() -> Box<dyn Board> {
    #[cfg(target_os = "linux")]
    {
        Box::new(PiMachine::new(SyscallMedium::new(), crate::shutdown::ProcInit))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(Detached::new(std::env::consts::OS))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::shutdown::{InitSignal, NoInit};
    use rivet_adapter_image::{AUTOBOOT_FILE, AUTOBOOT_PARTITION_MOUNT, DT_BOOT_PARTITION, Errno};
    use std::collections::BTreeMap;

    /// A `BootMedium` in two maps.
    ///
    /// A second fake rather than `rivet-adapter-image`'s, which is
    /// `#[cfg(test)]` in that crate and therefore unreachable from here. It is
    /// deliberately smaller: this module's job is two conversions, so what has to
    /// be reachable is a card that answers, a card that refuses, and a firmware
    /// that is not there — `ADR-026` §4's ordinary state under an emulator.
    #[derive(Default)]
    struct FakeMedium {
        properties: BTreeMap<String, Vec<u8>>,
        files: BTreeMap<String, String>,
        mounts: Vec<(String, String, String, bool)>,
        writes: Vec<(String, Vec<u8>)>,
        renames: Vec<(String, String)>,
        unmounts: Vec<String>,
        syncs: usize,
        write_fails: Option<Errno>,
        /// Answer this many mounts, from the next, as one found already in
        /// place — a leaked mount, until something unmounts it.
        adopt_next: usize,
        /// What `reclaimed_orphan` would have journalled.
        reclaimed: Vec<String>,
        /// Refuse every lock with this, as another holder would.
        lock_fails: Option<Errno>,
        /// `lock` and `unlock`, in order.
        locks: Vec<&'static str>,
    }

    impl FakeMedium {
        /// A card `cmd/rivet-image` would have written, booted from `running`.
        fn card(running: u32) -> Self {
            let mut m = Self::default();
            m.properties.insert(
                DT_BOOT_PARTITION.to_string(),
                running.to_string().into_bytes(),
            );
            m.files.insert(
                format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
                "[all]\ntryboot_a_b=1\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n"
                    .to_string(),
            );
            m
        }
    }

    impl BootMedium for FakeMedium {
        fn read_property(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
            self.properties.get(path).cloned().ok_or(Errno::ENOENT)
        }

        fn read_file(&mut self, path: &str) -> Result<String, Errno> {
            self.files.get(path).cloned().ok_or(Errno::ENOENT)
        }

        fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
            if let Some(errno) = self.write_fails {
                return Err(errno);
            }
            self.writes.push((path.to_string(), contents.to_vec()));
            Ok(())
        }

        fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
            self.renames.push((from.to_string(), to.to_string()));
            Ok(())
        }

        fn sync(&mut self) {
            self.syncs += 1;
        }

        fn mount_boot_filesystem(
            &mut self,
            device: &str,
            at: &str,
            fstype: &str,
            writable: bool,
        ) -> Result<Mounted, Errno> {
            self.mounts.push((
                device.to_string(),
                at.to_string(),
                fstype.to_string(),
                writable,
            ));
            Ok(if self.adopt_next > 0 {
                self.adopt_next -= 1;
                Mounted::Adopted
            } else {
                Mounted::Made
            })
        }

        fn unmount(&mut self, at: &str) {
            self.unmounts.push(at.to_string());
        }

        fn reclaimed_orphan(&mut self, at: &str) {
            self.reclaimed.push(reclaim_message(at));
        }

        fn lock(&mut self, _path: &str) -> Result<(), Errno> {
            if let Some(errno) = self.lock_fails {
                return Err(errno);
            }
            self.locks.push("lock");
            Ok(())
        }

        fn unlock(&mut self, _path: &str) {
            self.locks.push("unlock");
        }
    }

    // -- the domain conversion -------------------------------------------------

    /// The firmware's number becomes a domain slot, and the two directions agree.
    #[test]
    fn the_firmwares_partition_number_becomes_the_domains_slot() {
        for (partition, slot) in [(1, Slot::A), (2, Slot::B)] {
            let mut machine = PiMachine::new(FakeMedium::card(partition), NoInit::new("test"));
            assert_eq!(machine.running_slot(), Ok(slot));
            assert_eq!(to_slot_id(slot), SlotId(partition));
        }
    }

    /// **A partition outside `ADR-018` §4's layout is refused, not defaulted.**
    /// Guessing would name a partition an update would overwrite.
    #[test]
    fn a_partition_the_layout_does_not_contain_is_refused_and_the_refusal_names_it() {
        let mut machine = PiMachine::new(FakeMedium::card(7), NoInit::new("test"));
        let got = machine.running_slot();
        assert!(
            matches!(got, Err(MachineError::Unsafe(ref why)) if why.contains('7')),
            "{got:?}"
        );
    }

    /// No device tree is the ordinary state under an emulator (`ADR-026` §4),
    /// and it is the *machine* that could not answer — not the request that was
    /// wrong. The operator's next step differs, so the code must.
    #[test]
    fn a_machine_with_no_device_tree_reports_a_refusal_rather_than_an_interlock() {
        let mut machine = PiMachine::new(FakeMedium::default(), NoInit::new("test"));
        assert!(matches!(
            machine.running_slot(),
            Err(MachineError::Refused(_))
        ));
        assert!(matches!(
            machine.arm_trial_boot(Slot::B),
            Err(MachineError::Refused(_))
        ));
    }

    // -- the whole stack, end to end -------------------------------------------

    /// **Arming is idempotent, and that is `ADR-006` §6.2's requirement rather
    /// than a nicety**: *"Agents retry. An operation that is unsafe to repeat is
    /// a defect."* On a fresh card `cmd/rivet-image` writes, the `[tryboot]`
    /// override already names the other slot — so arming it succeeds and writes
    /// **nothing**, and a second attempt does the same.
    #[test]
    fn arming_a_slot_the_override_already_names_succeeds_and_writes_nothing() {
        let mut machine = PiMachine::new(FakeMedium::card(1), NoInit::new("test"));
        for attempt in 1..=2 {
            assert_eq!(
                machine.arm_trial_boot(Slot::B),
                Ok(Reboot::OneShot),
                "attempt {attempt}"
            );
        }
        assert!(
            machine.medium.writes.is_empty(),
            "a no-op arming wrote to the card"
        );
        assert!(machine.medium.renames.is_empty());
        assert_eq!(machine.medium.syncs, 0);
        assert_eq!(
            machine.medium.unmounts,
            vec![
                AUTOBOOT_PARTITION_MOUNT.to_string(),
                AUTOBOOT_PARTITION_MOUNT.to_string()
            ],
            "every attempt must unmount, whatever it did"
        );
    }

    /// The one that moves a value: booted from slot B, arm slot A.
    #[test]
    fn arming_the_other_slot_moves_only_the_override_line() {
        let mut machine = PiMachine::new(FakeMedium::card(2), NoInit::new("test"));
        assert_eq!(machine.arm_trial_boot(Slot::A), Ok(Reboot::OneShot));

        let medium = machine.medium;
        let (path, bytes) = medium.writes.first().expect("the override had to move");
        let text = String::from_utf8_lossy(bytes);
        assert!(path.starts_with(AUTOBOOT_PARTITION_MOUNT), "{path}");
        assert!(text.contains("[tryboot]\nboot_partition=1"), "{text}");
        assert!(
            text.contains("[all]\ntryboot_a_b=1\nboot_partition=1"),
            "the [all] default must be untouched: {text}"
        );
        assert_eq!(medium.syncs, 2, "once before the rename and once after");
        assert_eq!(medium.renames.len(), 1);
    }

    /// **Interlock 2, through the whole stack.** The slot is named in the
    /// domain's vocabulary and the refusal comes back in it.
    #[test]
    fn the_running_slot_cannot_be_armed_through_the_domain_port_either() {
        let mut machine = PiMachine::new(FakeMedium::card(1), NoInit::new("test"));
        let got = machine.arm_trial_boot(Slot::A);
        assert!(matches!(got, Err(MachineError::Unsafe(_))), "{got:?}");
        assert!(
            machine.medium.mounts.is_empty(),
            "a refusal mounted the card"
        );
    }

    /// **Interlock 1, through the whole stack**, on the card `ADR-018` §5.3
    /// describes losing: `tryboot_a_b` absent, `arm_64bit=1` in `config.txt`,
    /// no 32-bit kernel.
    #[test]
    fn a_card_without_tryboot_a_b_cannot_be_armed_through_the_domain_port_either() {
        let mut medium = FakeMedium::card(1);
        medium.files.insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
            "[all]\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n".to_string(),
        );
        let mut machine = PiMachine::new(medium, NoInit::new("test"));
        let got = machine.arm_trial_boot(Slot::B);
        assert!(
            matches!(got, Err(MachineError::Unsafe(ref why)) if why.contains("tryboot_a_b")),
            "{got:?}"
        );
        assert!(machine.medium.writes.is_empty(), "the card was written to");
    }

    /// A card that has gone read-only mid-write — how an SD card usually
    /// announces that it is dying — is the machine's fault, and reaches the
    /// operator as such.
    #[test]
    fn a_card_that_refuses_the_write_is_reported_as_the_machine_rather_than_the_request() {
        let mut medium = FakeMedium::card(2);
        medium.write_fails = Some(Errno::EROFS);
        let mut machine = PiMachine::new(medium, NoInit::new("test"));
        let got = machine.arm_trial_boot(Slot::A);
        assert!(
            matches!(got, Err(MachineError::Refused(ref why)) if why.contains("EROFS")),
            "{got:?}"
        );
        assert!(
            machine.medium.renames.is_empty(),
            "a failed write was renamed over"
        );
    }

    // -- the dry run -----------------------------------------------------------

    /// `ADR-006` §6.2 through the domain port: the plan reaches the same verdict
    /// and touches nothing.
    #[test]
    fn the_dry_run_reaches_the_same_verdict_as_the_arming_and_writes_nothing() {
        for (running, target, allowed) in [
            (1u32, Slot::B, true),
            (2, Slot::A, true),
            (1, Slot::A, false),
            (2, Slot::B, false),
        ] {
            let mut dry = PiMachine::new(FakeMedium::card(running), NoInit::new("test"));
            let planned = dry.can_arm_trial_boot(target);
            assert_eq!(
                planned.is_ok(),
                allowed,
                "{running}->{target:?}: {planned:?}"
            );
            assert!(dry.medium.writes.is_empty());
            assert_eq!(dry.medium.syncs, 0);

            let mut wet = PiMachine::new(FakeMedium::card(running), NoInit::new("test"));
            assert_eq!(
                wet.arm_trial_boot(target).is_ok(),
                planned.is_ok(),
                "the plan and the apply disagreed for {running}->{target:?}"
            );
        }
    }

    // -- the two conversions, exhaustively -------------------------------------

    /// The one-shot must not become an ordinary restart. That mapping getting
    /// lost is a board that reboots, comes back healthy, and trials nothing —
    /// which is indistinguishable from success.
    #[test]
    fn the_one_shot_arming_maps_onto_the_restart_that_carries_it() {
        assert_eq!(to_reboot(ArmedReboot::OneShot), Reboot::OneShot);
        assert_eq!(to_reboot(ArmedReboot::Ordinary), Reboot::Ordinary);
        assert_ne!(
            to_reboot(ArmedReboot::OneShot),
            to_reboot(ArmedReboot::Ordinary)
        );
    }

    /// An interlock and a broken card must not collapse into one code: an agent
    /// stops on the first and investigates hardware on the second.
    #[test]
    fn the_two_kinds_of_arm_refusal_stay_two_kinds_of_machine_error() {
        assert_eq!(
            to_machine_error(ArmRefusal::Unsafe("why".to_string())),
            MachineError::Unsafe("why".to_string())
        );
        assert_eq!(
            to_machine_error(ArmRefusal::Medium("why".to_string())),
            MachineError::Refused("why".to_string())
        );
    }

    // -- stopping, through the same domain port --------------------------------

    /// A pid 1 that answers as scripted, so the whole stack can be driven.
    ///
    /// A second fake rather than `crate::shutdown`'s, which is `#[cfg(test)]`
    /// there and unreachable from here. Deliberately smaller: what has to be
    /// reachable in *this* module is that a stop reaches the init seam and never
    /// the card, and that the two refusals stay two codes.
    #[derive(Default)]
    struct FakeInit {
        comm: Option<&'static str>,
        sent: Vec<InitSignal>,
    }

    impl FakeInit {
        const fn board() -> Self {
            Self {
                comm: Some("rivet-initd"),
                sent: Vec::new(),
            }
        }

        const fn workstation() -> Self {
            Self {
                comm: Some("systemd"),
                sent: Vec::new(),
            }
        }
    }

    /// **The reload seam refuses here**, and that is the assertion rather than
    /// an omission: what has to be reachable in this module is that a stop
    /// reaches the init seam and never the card. A reload is
    /// `crate::reload`'s own tests' business, and a test here that started
    /// reloading fails loudly (`AGENTS.md` §3).
    impl crate::reload::Supervisor for FakeInit {
        fn read_proc(&mut self, _path: &str) -> Result<Vec<u8>, i32> {
            Err(38)
        }
        fn pids(&mut self) -> Result<Vec<i32>, i32> {
            Err(38)
        }
        fn hangup(&mut self, _pid: i32) -> Result<(), i32> {
            Err(38)
        }
    }

    impl crate::shutdown::InitProcess for FakeInit {
        fn read_proc(&mut self, _path: &str) -> Result<Vec<u8>, i32> {
            self.comm
                .map(|c| format!("{c}\n").into_bytes())
                .ok_or(libc_enoent())
        }

        fn signal_init(&mut self, signal: InitSignal) -> Result<(), i32> {
            self.sent.push(signal);
            Ok(())
        }
    }

    const fn libc_enoent() -> i32 {
        2
    }

    /// **A stop never reads the card, and an arming never signals pid 1.** The
    /// two halves of `PiMachine` are separate seams and this is what says so —
    /// a `request_stop` that mounted the boot partition would be a machine that
    /// cannot be stopped once its card has gone read-only.
    #[test]
    fn stopping_the_machine_touches_the_init_seam_and_never_the_card() {
        for stop in [Stop::PowerOff, Stop::Restart, Stop::TrialRestart] {
            let mut machine = PiMachine::new(FakeMedium::card(1), FakeInit::board());
            assert_eq!(machine.can_request_stop(stop), Ok(()), "{stop}");
            assert_eq!(machine.request_stop(stop), Ok(()), "{stop}");
            assert_eq!(machine.init.sent, vec![InitSignal::of(stop)], "{stop}");
            assert!(machine.medium.mounts.is_empty(), "{stop} mounted the card");
            assert!(machine.medium.writes.is_empty(), "{stop} wrote to the card");
            assert_eq!(machine.medium.syncs, 0, "{stop} synced the card");
        }

        // ...and the other direction: arming reads the card and signals nothing.
        let mut machine = PiMachine::new(FakeMedium::card(2), FakeInit::board());
        assert_eq!(machine.arm_trial_boot(Slot::A), Ok(Reboot::OneShot));
        assert!(machine.init.sent.is_empty(), "an arming signalled pid 1");
    }

    /// **The interlock, through the domain port.** A development host is refused
    /// on both the plan and the confirmed path, and nothing is sent.
    #[test]
    fn a_machine_whose_pid_one_is_not_rivets_is_refused_through_the_domain_port() {
        for stop in [Stop::PowerOff, Stop::Restart, Stop::TrialRestart] {
            let mut machine = PiMachine::new(FakeMedium::card(1), FakeInit::workstation());
            for got in [machine.can_request_stop(stop), machine.request_stop(stop)] {
                assert!(
                    matches!(got, Err(MachineError::Unsafe(ref why)) if why.contains("systemd")),
                    "{stop}: {got:?}"
                );
            }
            assert!(machine.init.sent.is_empty(), "{stop} was signalled anyway");
        }
    }

    /// Fail closed through the port as well: a `/proc` that will not be read is a
    /// refusal, and it is the interlock's kind rather than the card's.
    #[test]
    fn a_machine_that_cannot_be_identified_is_refused_rather_than_signalled() {
        let mut machine = PiMachine::new(FakeMedium::card(1), FakeInit::default());
        let got = machine.request_stop(Stop::PowerOff);
        assert!(matches!(got, Err(MachineError::Unsafe(_))), "{got:?}");
        assert!(machine.init.sent.is_empty());
    }

    // -- the platform that is not Linux ----------------------------------------

    /// **It refuses, and it never says `Ok`** (`AGENTS.md` §3). It also has to
    /// say *why* — an operator on a workstation who gets "no" with no reason
    /// will conclude the board is broken.
    #[test]
    fn a_build_that_cannot_reach_a_machine_refuses_every_call_and_names_the_platform() {
        let mut detached = Detached::new("windows");
        assert!(matches!(
            detached.running_slot(),
            Err(MachineError::Refused(_))
        ));
        assert!(matches!(
            detached.can_arm_trial_boot(Slot::B),
            Err(MachineError::Refused(_))
        ));
        let got = detached.arm_trial_boot(Slot::B);
        assert!(
            matches!(got, Err(MachineError::Refused(ref why)) if why.contains("windows")),
            "{got:?}"
        );
    }

    /// And it is a *machine* refusal rather than an interlock: nothing about the
    /// operator's request was wrong.
    #[test]
    fn a_detached_build_blames_itself_rather_than_the_request() {
        let mut detached = Detached::new("macos");
        assert!(!matches!(
            detached.arm_trial_boot(Slot::A),
            Err(MachineError::Unsafe(_))
        ));
    }

    /// **A detached build refuses to stop anything**, and it refuses as an
    /// *interlock* rather than as a broken machine — because what is wrong is
    /// that this is not a board, which is the same thing
    /// `crate::shutdown::identify` says on a Linux workstation. Both must reach
    /// the operator as `E_UNSAFE`, so an agent stops rather than retrying.
    #[test]
    fn a_detached_build_refuses_every_stop_as_an_interlock_and_names_the_platform() {
        for stop in [Stop::PowerOff, Stop::Restart, Stop::TrialRestart] {
            let mut detached = Detached::new("windows");
            for got in [detached.can_request_stop(stop), detached.request_stop(stop)] {
                assert!(
                    matches!(got, Err(MachineError::Unsafe(ref why))
                        if why.contains("windows") && why.contains("NOTHING WAS SENT")),
                    "{stop}: {got:?}"
                );
            }
        }
    }

    /// The two refusals a detached build gives must not be the same sentence: an
    /// operator told about `mount(2)` after asking for a power off would go
    /// looking for the wrong thing.
    #[test]
    fn a_detached_build_gives_a_different_reason_for_a_stop_than_for_an_arming() {
        let detached = Detached::new("windows");
        assert_ne!(detached.refusal().why(), detached.stop_refusal().why());
        assert!(detached.stop_refusal().why().contains("pid 1"));
        assert!(!detached.refusal().why().contains("pid 1"));
    }

    // -- R1: `EBUSY` is convergent only at a real mount point -------------------
    //
    // `SyscallMedium::mount_boot_filesystem` calls the real `mount(2)`, and
    // this container's own build has no `CAP_SYS_ADMIN` to make one with
    // (`tools/container/exec.sh` grants none) — `cmd/rivet-deploy/src/bench.rs`
    // exercises a real mount by hand, outside `cargo test`, for the same
    // reason. So these tests drive `is_mount_point` itself: the ONE piece of
    // new logic, at the narrowest seam this environment can reach without
    // mounting anything — proven against facts already true of any Linux
    // container rather than against a mount this process cannot make.

    /// **The "EBUSY with the right device mounted, where it passes" half.**
    /// `/proc` is a real mount point in any Linux environment this binary
    /// runs in, including this one: the container runtime mounts it before
    /// this process starts, so reading it needs no privilege this test does
    /// not already have.
    #[cfg(target_os = "linux")]
    #[test]
    fn is_mount_point_is_true_for_a_real_mount() {
        assert!(
            is_mount_point("/proc"),
            "/proc did not answer as its own device in this container"
        );
    }

    /// **The "EBUSY with a different device or no mount, where it is
    /// refused" half.** An ordinary directory this test just created is on
    /// the same device as its parent — nothing was mounted there.
    #[cfg(target_os = "linux")]
    #[test]
    fn is_mount_point_is_false_for_an_ordinary_directory() {
        let mut directory = std::env::temp_dir();
        directory.push(format!("rivet-boot-mount-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");
        assert!(
            !is_mount_point(directory.to_str().expect("a utf-8 temporary path")),
            "a freshly created directory answered as a mount point"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    // -- the record's lock, and whose mount is whose ---------------------------

    /// **`EBUSY` at a real mount point is a mount found in place, and only a
    /// mount this call made is `Made`.** Anything else `EBUSY` means is a
    /// refusal, never convergence.
    #[test]
    fn a_mount_found_in_place_is_adopted_and_only_a_mount_made_here_is_made() {
        assert_eq!(mount_outcome(Ok(()), || panic!("not asked")), Ok(Mounted::Made));
        assert_eq!(
            mount_outcome(Err(Errno::EBUSY.0), || true),
            Ok(Mounted::Adopted)
        );
        assert_eq!(
            mount_outcome(Err(Errno::EBUSY.0), || false),
            Err(Errno::EBUSY)
        );
        assert_eq!(
            mount_outcome(Err(Errno::ENODEV.0), || true),
            Err(Errno::ENODEV)
        );
    }

    /// **`get slot` finding a mount in place under the lock reclaims it** —
    /// unmounts it, mounts afresh, reads, unmounts its own — and says so for
    /// the journal. The lock is taken and released around each access.
    #[test]
    fn get_slot_reclaims_an_orphaned_mount_and_reports_it() {
        let mut medium = FakeMedium::card(2);
        medium.adopt_next = 1;
        let mut machine = PiMachine::new(medium, NoInit::new("test"));
        let slots = machine.read_slots().expect("the firmware half was readable");
        assert_eq!(slots.default_slot, Some(Slot::A), "{slots:?}");
        assert_eq!(machine.medium.mounts.len(), 2);
        assert_eq!(machine.medium.unmounts.len(), 2, "the orphan, then its own");
        assert_eq!(machine.medium.reclaimed.len(), 1);
        assert!(machine.medium.reclaimed[0].contains(AUTOBOOT_PARTITION_MOUNT));
        assert_eq!(machine.arm_trial_boot(Slot::A), Ok(Reboot::OneShot));
        assert_eq!(
            machine.medium.locks,
            vec!["lock", "unlock", "lock", "unlock"],
            "get slot's read, then the arming's write"
        );
    }

    /// **A reader finding the lock held reports the partition busy and never
    /// touches the mount.** `get slot` still answers everything it could
    /// measure, with the default slot unmeasured and the reason given; the
    /// install channel's check hears the same reason.
    #[test]
    fn get_slot_with_the_record_locked_reports_busy_and_touches_no_mount() {
        let mut medium = FakeMedium::card(2);
        medium.lock_fails = Some(Errno::EAGAIN);
        let mut machine = PiMachine::new(medium, NoInit::new("test"));
        let slots = machine.read_slots().expect("the firmware half was readable");
        assert_eq!(slots.booted, Some(Slot::B));
        assert_eq!(slots.default_slot, None);
        let why = slots.default_unmeasured.unwrap_or_default();
        assert!(
            why.contains(rivet_adapter_image::AUTOBOOT_LOCK) && why.contains("EAGAIN"),
            "{why}"
        );
        assert!(machine.medium.mounts.is_empty(), "a busy reader mounted");
        assert!(machine.medium.unmounts.is_empty(), "a busy reader unmounted");
        assert!(machine.medium.locks.is_empty(), "a lock never taken was released");

        let armed = machine.arm_trial_boot(Slot::A);
        assert!(
            matches!(armed, Err(MachineError::Refused(ref why)) if why.contains("EAGAIN")),
            "{armed:?}"
        );
        assert!(machine.medium.writes.is_empty());
    }

    // The bound is long enough to outlast one commit and short enough that an
    // operator is answered, and it is polled many times inside it.
    const _: () = assert!(
        LOCK_WAIT_MS == 10_000 && LOCK_POLL_MS > 0 && LOCK_POLL_MS * 10 <= LOCK_WAIT_MS
    );

    /// The root directory is its own parent, so the function's own doc says
    /// it must answer `false` here rather than mistaking that for a mount.
    #[cfg(target_os = "linux")]
    #[test]
    fn is_mount_point_is_false_for_the_root_directory() {
        assert!(!is_mount_point("/"));
    }
}
