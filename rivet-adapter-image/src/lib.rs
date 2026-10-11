//! Rivet's A/B boot-slot adapter. `ADR-014` §2.
//!
//! `ADR-014` §2 names this crate *"A/B image store, update, rollback"*.
//!
//! ~~**One third of that exists**, and the missing two thirds are missing on
//! purpose: nothing here fetches, verifies or writes an update payload, because
//! `ADR-028` §8 item 1 leaves the signature scheme unruled and anything that
//! verified would be inventing a ruling.~~ **Superseded 2026-08-24.**
//! `ADR-029` §7.1 ruled the scheme on 2026-08-23 and `rivet-core`'s
//! `package` module is the verifier; [`install`] is this crate's half of
//! `ADR-029` §9.2 — *which* member goes to *which* target, in what order, and
//! which ones may be skipped. **Still not here, and still on purpose: nothing
//! fetches, and nothing writes.** A plan is a decision; every byte of I/O is
//! `cmd/rivet-deploy`'s, at its own composition root.
//!
//! What is here is the primitive everything else stands on — *which slot am I
//! in, is this a trial, make this one the default, arm a trial of that one,
//! where does a payload go, and what does this platform reserve* — behind a
//! port, because x86 has no `tryboot` and the maintainer ruled it one, 2026-08-23.
//!
//! # Why the crate exists now — created 2026-08-23
//!
//! `ADR-014` §5 is explicit that the layout is *"the destination, not a
//! scaffolding requirement before the first boot"*, so a crate is created by the
//! first thing that needs it. The thing that needed it is this:
//! [`BootSlots`] and [`PiFirmware`] were `cmd/rivet-initd/src/slots.rs`, and
//! **`cmd/rivet-initd` is a binary crate with no lib target** — so the port was
//! unreachable from `cmd/rivet-sysd`, from `cmd/rivet`, and from any
//! future update path. `ADR-028` §4 rules that an authenticated principal
//! supplies the *authority* to install an update and does not say which
//! *process* executes it; a port that only PID 1 can name has answered that
//! question by accident.
//!
//! **Nothing about the behaviour changed in the move.** The slot-B commit was
//! corrected on hardware on 2026-08-23 and that correction was not this change's
//! to touch.
//!
//! # The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`port`] | the seam: the [`BootMedium`] port and [`Errno`]. **No implementor** — see that module for why |
//! | [`firmware`] | what the Raspberry Pi firmware publishes, and where the A/B layout lives on the card |
//! | [`slots`] | the [`BootSlots`] port, the [`PiFirmware`] implementor, and the `autoboot.txt` rewriting |
//! | [`install`] | `ADR-029` §9.2 as a **plan**: which member goes to which target, in what order, and which are skipped. No I/O |
//! | [`grow`] | `ADR-018` §10.3: the GPT rewrite and the `ext4` resize that make `/state` fill the card it was written to, behind [`grow::GrowMedium`] |
//! | `testio` | a [`BootMedium`] in a `BTreeMap`, under `cfg(test)` |
//!
//! **[`grow`] is here rather than in a crate of its own** because the fact it
//! turns on is this crate's subject: where `ADR-018` §4's partitions are on the
//! medium the firmware boots from. It reads the table `cmd/rivet-image` wrote
//! and moves one entry of it; nothing else in the repository knows that layout
//! at the level of an LBA.
//!
//! # Its edges
//!
//! It depends on `rivet-core` and on nothing else. `ADR-014` §3's table
//! forbids core → adapter and adapter → adapter, and neither appears:
//! `rivet-core::state` owns `ADR-018` §4's partition numbers and this crate
//! reads them from there. `cmd/rivet-initd` depends on **this** crate, which is
//! the direction `ADR-014` §3 permits — *"`cmd/*` may depend on everything and
//! contains no logic"* — and it supplies the only [`BootMedium`] there is, over
//! the `Kernel` port it already owns.
//!
//! # ~~Nothing in this crate has ever run~~ — false since 2026-08-23
//!
//! Said plainly, because `AGENTS.md` §3 forbids a stub that looks further along
//! than it is and §6 forbids reporting "wired up" as "works".
//!
//! ~~`ADR-026` §4 puts `tryboot` and `/proc/device-tree/chosen/bootloader` in the
//! column QEMU does **not emulate at all**, so every line here is exercised by
//! unit tests against a fake and by nothing else. … `ADR-018` §5.3's second open
//! item — that a `[tryboot]` section selects a different `boot_partition` at all
//! — is still open, and the experiment that would close it has not been run.~~
//!
//! **Corrected 2026-08-24, and the correction is `AGENTS.md` §7 in both
//! directions.** `ADR-018` §5.3.2 records the whole of §5 running on a Raspberry
//! Pi 4 on 2026-08-23 — an arming, a trial boot of the other slot, and §5.3.3's
//! rollback — so the paragraph above outlived its own subject by a day.
//!
//! **The half that is still true, and it is the important half:** the sentence
//! about QEMU is unchanged. **No payload written by this crate's plan has
//! reached a card.** What a board has settled is the *arming and rollback
//! primitive*; what it has not settled is anything about installing.
//!
//! [`install`] has run outside a unit test exactly once — driven by
//! `cmd/rivet-deploy` against a loop device over a copy of a card image,
//! `ADR-029` §14.6. That settles the plan against real block devices and a real
//! FAT filesystem, and settles nothing about an SD card.

pub mod firmware;
pub mod grow;
pub mod install;
pub mod port;
pub mod slots;

#[cfg(test)]
mod testio;

pub use firmware::{
    AUTOBOOT_LOCK, AUTOBOOT_PARTITION_MOUNT, AutobootMount, BOOT_DISK, BOOT_FSTYPE,
    DT_BOOT_PARTITION, DT_TRYBOOT, PAYLOAD_BOOT_MOUNT, PROVISIONING_FILE,
    autoboot_partition_device, boot_partition_device, mount_autoboot_partition, parse_dt_u32,
    umount_autoboot_partition,
};
pub use grow::{
    FsPlan, FsRefusal, FsSize, Grew, GrowMedium, Plan as GrowPlan, Refusal as GrowRefusal,
    Step as GrowStep, grow_state,
};
pub use install::{
    BootFileAction, BootHalf, BootWrite, InstallPlan, PlanRefusal, SystemWrite, action_for,
    slot_named, target_slot,
};
pub use port::{BootMedium, DirEntry, Errno, Listing, Mounted};
pub use slots::{
    AUTOBOOT_FILE, AUTOBOOT_TEMP, ArmRefusal, ArmedReboot, BootSlots, Committed, PayloadTarget,
    PiFirmware, SlotId, SlotPayload, SlotRecord, TRYBOOT_A_B, require_tryboot_a_b,
    rewrite_autoboot_default, rewrite_autoboot_tryboot,
};
