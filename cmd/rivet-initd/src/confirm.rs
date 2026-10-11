//! Item 5: confirm the boot once the system is healthy.
//!
//! `ADR-018` §5 is the mechanism and it is the firmware's, not Rivet's. The
//! bootloader sets a **one-shot** `tryboot` flag, boots the other slot exactly
//! once, and — if nothing commits — returns to the previous slot on the next
//! reset with no userspace involvement at all. Confirming is therefore not
//! "tell the bootloader it worked"; it is **making the slot we are running the
//! new default**.
//!
//! # What is left in this module, and what moved out — 2026-08-23
//!
//! **The mechanism moved behind [`BootSlots`].** The maintainer ruled the
//! boot primitive a port, 2026-08-23, because x86 has no `tryboot` and what makes
//! `ADR-018` §5 good — a firmware-owned one-shot needing no userspace on the
//! failure path — does not port by itself. `rivet-adapter-image` carries the
//! trait, the Raspberry Pi implementor, and the `autoboot.txt` rewriting that
//! used to live here. **None of that behaviour changed**: it was corrected on
//! hardware earlier the same day and this was a move, not an edit.
//!
//! It moved **twice** on that day and the second move is worth naming, because
//! this module's imports are the visible trace of it: the port spent a few hours
//! in `cmd/rivet-initd/src/slots.rs`, which is a binary crate with no lib
//! target, so nothing but PID 1 could ever have named it. `ADR-028` §4 does not
//! say which process installs an update. `crate::slots` is now the *adapter*
//! from [`Kernel`] to that crate's `BootMedium` port, and `cmd/rivet-initd`
//! stays the composition root.
//!
//! What stays here is what is *not* the firmware's: the **gate** — whether PID 1
//! is willing to certify this boot at all — and the console vocabulary for what
//! happened. Those are `ADR-017` §3.1.4 and §3.1.6, and they would read the same
//! on any platform, which is the test that the seam is in the right place.
//!
//! Two different numbers still appear below and must not be conflated. The slot
//! *committed* is the one the machine booted. The place the commit is *written*
//! is the port's, and on this hardware it is always the first FAT partition —
//! `rivet_core::state::AUTOBOOT_PARTITION` carries the two boots that proved
//! it and what believing otherwise cost.
//!
//! # This code has never run, and cannot run under the emulator
//!
//! `ADR-026` §4 is explicit: `tryboot` and
//! `/proc/device-tree/chosen/bootloader` are "not emulated at all". So the
//! entire content of this module and of `rivet-adapter-image` is exercised by
//! unit tests against [`Kernel`] and by nothing else
//! until it reaches a board. That is why the seams exist, and it is not a substitute for
//! the board.
//!
//! The flag itself *was* confirmed on hardware 2026-08-22 (`ADR-018` §5.3):
//! `/proc/device-tree/chosen/bootloader/tryboot` read `0` on a normal boot and
//! `1` after a tryboot reboot — requested from **stock Raspberry Pi OS**, not
//! from Rivet. `boot_partition=` under `[all]` is confirmed too, by the two
//! boots of 2026-08-23: the firmware read it, obeyed it, and selected a
//! partition it would not otherwise have booted.
//!
//! **Still documentation-derived:** that a `[tryboot]` section selects a
//! *different* `boot_partition` for the one-shot boot. Neither of those boots
//! used tryboot, so the override was only ever observed being left alone.
//! `ADR-018` §5.3 item 2. Rivet can now *request* such a boot — that is what
//! [`RebootHow::TrialRestart`](crate::kernel::RebootHow::TrialRestart) is — so
//! the experiment is available for the first time and has not been run. Which
//! reboot carries the arming is the port's to say and this crate's to perform —
//! [`crate::slots::reboot_for`] is the whole of the mapping.
//!
//! # Failing closed
//!
//! Every error path here declines to confirm. That is the safe direction: an
//! unconfirmed trial boot rolls back, and an unconfirmed *normal* boot changes
//! nothing at all. Confirming when we should not is the only outcome that
//! cannot be undone remotely.

use rivet_adapter_image::{ArmRefusal, BootSlots, Committed};

use crate::boot::BootHealth;
use crate::kernel::Kernel;
use crate::provision::Provisioning;
use crate::slots::KernelMedium;
use crate::supervise::Supervisor;

/// What became of the boot confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmation {
    /// The system is not in a state PID 1 will certify.
    Withheld { why: String },
    /// **A trial boot that nothing has reached yet.** `PLAN.md` §104.20.
    ///
    /// Not a refusal and not a failure: it is the answer *not yet*, and the
    /// caller must ask again rather than storing it. That distinction is the
    /// whole of the variant — `Withheld` is remembered and this is not, so a
    /// machine reached five minutes after it booted still confirms.
    WaitingToBeReached,
    /// This is an ordinary boot, not a trial one. Nothing to commit.
    NothingToConfirm,
    /// The default-slot record already named this partition.
    AlreadyDefault { partition: u32 },
    /// The default was moved to the running slot.
    Confirmed { partition: u32 },
    /// It should have been confirmed and could not be.
    ///
    /// `medium` is true when the boot medium refused the commit — a mount, a
    /// read, a write or a rename of the default-slot record — which is the one
    /// failure a later attempt can answer differently. It is false for
    /// everything else, including a record the port will not rewrite, where
    /// another attempt would change nothing.
    Failed { why: String, medium: bool },
}

/// How many times, in all, PID 1 attempts the commit within its one decision
/// per boot.
///
/// More than one because a single refusal from the medium, on a board that is
/// otherwise healthy, would leave it uncommitted and silently rolled back at
/// its next reset. Few, because a refusal that persists fails every attempt
/// alike and the verdict is then the same [`Confirmation::Failed`] a single
/// attempt would have given, only later.
pub const COMMIT_ATTEMPTS: u32 = 3;

/// How long PID 1 waits between commit attempts.
///
/// Long enough for whatever else had the partition to let go of it, short
/// enough that every attempt is over well inside a minute. The wait is a timer
/// in PID 1's loop and never a sleep: signals are handled and children reaped
/// while it runs.
pub const COMMIT_RETRY_DELAY_MS: u64 = 5_000;

/// How long PID 1 waits between checks of a trial boot nothing has reached
/// yet ([`Confirmation::WaitingToBeReached`]).
///
/// **One second, so the commit follows the first connection promptly**: an
/// operator who has just reached a trial boot is usually about to ask it
/// whether it committed. A check is two small reads — the firmware's tryboot
/// flag and the reachability marker — so a check a second costs nothing a
/// board would notice. Without a wait here the loop's timeout falls to its
/// one-millisecond floor once the supervisor has settled, and PID 1 re-reads
/// both about a thousand times a second.
pub const REACHED_CHECK_INTERVAL_MS: u64 = 1_000;

/// At most this many entries of the autoboot partition's root are journalled
/// after a failed attempt. A Raspberry Pi boot partition's root holds a few
/// dozen, so this lists a normal one whole and bounds an abnormal one.
pub const LISTING_LIMIT: usize = 64;

impl Confirmation {
    pub fn summary(&self) -> String {
        match self {
            Self::Withheld { why } => format!("boot NOT confirmed: {why}"),
            Self::NothingToConfirm => "not a tryboot boot; there is no slot to commit".to_string(),
            Self::AlreadyDefault { partition } => {
                format!("partition {partition} is already the default; nothing to change")
            }
            Self::Confirmed { partition } => {
                format!("boot confirmed: partition {partition} is now the default")
            }
            Self::WaitingToBeReached => "this trial boot is NOT confirmed yet: nothing has \
                 reached the control plane since it came up. The mounts, the document and the \
                 supervisor are all healthy - what is missing is proof that anybody can still \
                 TALK to this machine, and ADR-035 left no second way in. PID 1 checks again \
                 every second. Connect to it and this confirms within a second; leave it and \
                 the next reset returns to the previous slot, which is the point \
                 (PLAN.md §104.20)"
                .to_string(),
            Self::Failed { why, .. } => format!("boot could NOT be confirmed: {why}"),
        }
    }
}

/// Whether PID 1 is willing to certify this boot at all.
///
/// Takes the verdicts rather than re-deriving them — `ADR-011`, one thing owns
/// each fact. `ADR-017` §3.1.6 owns the mount verdict, [`Provisioning`] owns
/// the document's, and §3.1.4 owns the supervisor's.
///
/// **Nothing here is platform-specific and nothing here reads the firmware.**
/// That is the argument for the seam: whether a machine deserves to become the
/// default is Rivet's question, and where the answer is written down is the
/// firmware's.
pub fn gate(
    health: &BootHealth,
    provisioning: &Provisioning,
    supervisor: &Supervisor,
) -> Result<(), String> {
    if !health.may_confirm_boot() {
        return Err("the mount phase was UNCONFIRMABLE (ADR-017 §3.1.6)".to_string());
    }
    if !provisioning.confirms_boot() {
        return Err("the provisioning document could not be applied".to_string());
    }
    if supervisor.given_up {
        return Err("the supervisor could not be kept running (ADR-017 §3.1.4)".to_string());
    }
    Ok(())
}

/// **Whether anything has reached this machine's control plane on this boot.**
///
/// `PLAN.md` §104.20. The gate above had three questions and none of them was
/// *can anybody talk to this machine*, so a system update that came up healthy
/// and unreachable confirmed itself and became the default — after which
/// `ADR-018`'s A/B had nothing left to roll back to.
///
/// **A two-state type rather than a `bool`**, because the argument at the call
/// site reads as a claim about the machine and a bare `true` does not.
///
/// # It only matters in a trial boot
///
/// `confirm_boot` is a no-op on an ordinary one — there is no slot to commit —
/// so an unreachable machine that was never trialling anything is unaffected.
/// What this changes is exactly the case it was built for: a slot on probation
/// does not get promoted until somebody has proved they can still talk to it.
///
/// # And a machine nobody ever connects to reverts, which is correct
///
/// That is the same default `ADR-018` §5 already relies on — *"a system that
/// fails to commit reverts by default"* — applied to a condition that was
/// missing rather than to a new one. An update nobody verified is an update
/// nobody wanted to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reached {
    /// A caller completed a mutually-authenticated handshake.
    Yes,
    /// Not yet. **Not "never"** — the gate is asked again on every pass, and
    /// this is the answer that makes PID 1 wait rather than refuse.
    NotYet,
}

impl Reached {
    /// Read the marker `cmd/rivet-mcpd` writes.
    ///
    /// **Existence only.** What the file means has no degrees: one pinned key
    /// completed one handshake.
    pub fn read(k: &mut dyn crate::kernel::Kernel) -> Self {
        // `read_file` rather than a stat: the Kernel port has no existence
        // predicate, and a marker whose whole meaning is existence is read the
        // same way either would answer.
        match k.read_file(rivet_core::operation::status::REACHED_MARKER_PATH) {
            Ok(_) => Self::Yes,
            Err(_) => Self::NotYet,
        }
    }
}

/// Item 5, end to end, as ONE attempt; every failure declines to confirm.
///
/// Whether to attempt again is the caller's: [`crate::pid1`] repeats a
/// `Failed { medium: true, .. }` up to [`COMMIT_ATTEMPTS`] times in all, as a
/// timer in its loop, because only the loop can wait without stopping the
/// reaping and the signal handling.
pub fn confirm_boot(
    k: &mut dyn Kernel,
    slots: &dyn BootSlots,
    health: &BootHealth,
    provisioning: &Provisioning,
    supervisor: &Supervisor,
) -> Confirmation {
    if let Err(why) = gate(health, provisioning, supervisor) {
        return Confirmation::Withheld { why };
    }

    // Each port call reborrows the kernel through the adapter rather than
    // holding one across the whole function — the shape
    // `cmd/rivet-sysd/src/clock.rs` uses, and the reason `k` is still
    // available to this crate between the calls.
    //
    // Is this a trial boot at all? Fail closed: if we cannot tell, we must
    // assume it might be, and decline — declining a normal boot costs nothing.
    match slots.in_trial_boot(&mut KernelMedium::new(&mut *k)) {
        Ok(false) => return Confirmation::NothingToConfirm,
        Ok(true) => {}
        Err(why) => return Confirmation::Failed { why, medium: false },
    }

    // **Only now, and only in a trial boot** — `PLAN.md` §104.20.
    //
    // This deliberately is NOT one of `gate`'s questions, and the tests are
    // what said so. `gate` runs before this function knows whether there is
    // anything to commit, so a reachability check inside it made an ORDINARY
    // boot report `NOTHING HAS REACHED THIS MACHINE` — a frightening sentence
    // about a machine with nothing to confirm. Reachability is a condition on
    // PROMOTING A SLOT, not on PID 1's general willingness to certify.
    if Reached::read(k) == Reached::NotYet {
        return Confirmation::WaitingToBeReached;
    }

    let slot = match slots.running_slot(&mut KernelMedium::new(&mut *k)) {
        Ok(slot) => slot,
        Err(why) => return Confirmation::Failed { why, medium: false },
    };

    match slots.commit(&mut KernelMedium::new(&mut *k), slot) {
        Ok(Committed::AlreadyDefault) => Confirmation::AlreadyDefault { partition: slot.0 },
        Ok(Committed::Changed) => Confirmation::Confirmed { partition: slot.0 },
        Err(ArmRefusal::Medium(why)) => Confirmation::Failed { why, medium: true },
        Err(ArmRefusal::Unsafe(why)) => Confirmation::Failed { why, medium: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::Errno;
    use crate::supervise::{RETRY_BUDGET, SUPERVISOR_PATH};
    use crate::testkernel::{FakeKernel, Script};
    use rivet_adapter_image::{
        AUTOBOOT_FILE, AUTOBOOT_PARTITION_MOUNT, AUTOBOOT_TEMP, DT_BOOT_PARTITION, DT_TRYBOOT,
        PiFirmware, autoboot_partition_device,
    };

    pub(super) const AUTOBOOT: &str = "[all]\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n";

    pub(super) fn healthy() -> BootHealth {
        BootHealth::Healthy
    }

    pub(super) fn no_document() -> Provisioning {
        Provisioning::Absent {
            why: "test".to_string(),
        }
    }

    pub(super) fn running_supervisor() -> Supervisor {
        let mut s = Supervisor::new(SUPERVISOR_PATH);
        s.pid = Some(200);
        s
    }

    pub(super) fn tryboot_machine(
        flag: &[u8],
        partition: &[u8],
        autoboot: Script<Result<String, Errno>>,
    ) -> FakeKernel {
        let mut k = FakeKernel::new();
        k.blobs_mut()
            .insert(DT_TRYBOOT.to_string(), Script::always(Ok(flag.to_vec())));
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(partition.to_vec())),
        );
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
            autoboot,
        );
        k
    }

    // -- the gate -------------------------------------------------------------

    #[test]
    fn an_unconfirmable_mount_phase_withholds_confirmation() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let health = BootHealth::Unconfirmable {
            missing: vec!["/proc".to_string()],
        };
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &health,
            &no_document(),
            &running_supervisor(),
        );

        assert!(matches!(got, Confirmation::Withheld { .. }), "{got:?}");
        // And nothing was touched: no mount, no write, no rename.
        assert!(k.mount_calls.is_empty());
        assert!(k.write_calls.is_empty());
        assert!(k.rename_calls.is_empty());
    }

    #[test]
    fn a_refused_provisioning_document_withholds_confirmation() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let refused = Provisioning::Refused {
            detail: "test".to_string(),
        };
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &refused,
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Withheld { .. }), "{got:?}");
        assert!(k.write_calls.is_empty());
    }

    #[test]
    fn a_supervisor_that_was_given_up_on_withholds_confirmation() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let mut s = running_supervisor();
        s.given_up = true;
        s.failures = RETRY_BUDGET;
        let got = confirm_boot(&mut k, &PiFirmware, &healthy(), &no_document(), &s);
        assert!(matches!(got, Confirmation::Withheld { .. }), "{got:?}");
        assert!(k.write_calls.is_empty());
    }

    #[test]
    fn the_gate_names_which_of_the_three_refused() {
        let unconfirmable = BootHealth::Unconfirmable { missing: vec![] };
        let refused = Provisioning::Refused {
            detail: String::new(),
        };
        let mut given_up = running_supervisor();
        given_up.given_up = true;

        assert!(
            gate(&unconfirmable, &no_document(), &running_supervisor())
                .is_err_and(|e| e.contains("mount phase"))
        );
        assert!(
            gate(&healthy(), &refused, &running_supervisor())
                .is_err_and(|e| e.contains("provisioning"))
        );
        assert!(
            gate(&healthy(), &no_document(), &given_up).is_err_and(|e| e.contains("supervisor"))
        );
        assert_eq!(
            gate(&healthy(), &no_document(), &running_supervisor()),
            Ok(())
        );
    }

    /// The gate is the platform-independent half, so it must not need the port
    /// at all. If this ever stops compiling without one, the seam has moved.
    #[test]
    fn the_gate_reaches_no_firmware_and_no_kernel() {
        assert_eq!(
            gate(&healthy(), &no_document(), &running_supervisor()),
            Ok(())
        );
    }

    // -- reading the flag -----------------------------------------------------

    #[test]
    fn a_normal_boot_has_nothing_to_confirm_and_writes_nothing() {
        let mut k = tryboot_machine(b"0", b"1", Script::always(Ok(AUTOBOOT.to_string())));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert_eq!(got, Confirmation::NothingToConfirm);
        assert!(
            k.mount_calls.is_empty(),
            "a normal boot must not even mount"
        );
        assert!(k.write_calls.is_empty());
    }

    /// Under QEMU there is no `/proc/device-tree/chosen/bootloader` at all
    /// (`ADR-026` §4). Not knowing must fail closed.
    #[test]
    fn an_unreadable_tryboot_flag_declines_rather_than_assuming_a_normal_boot() {
        let mut k = FakeKernel::new();
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
        assert!(k.write_calls.is_empty());
    }

    #[test]
    fn an_undecodable_tryboot_flag_declines() {
        let mut k = tryboot_machine(b"yes", b"1", Script::always(Ok(AUTOBOOT.to_string())));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
    }

    /// A trial boot whose *partition* cannot be read must decline as well —
    /// there is a slot to commit and no way to say which.
    #[test]
    fn a_trial_boot_with_no_readable_partition_declines_without_writing() {
        let mut k = FakeKernel::new();
        k.blobs_mut()
            .insert(DT_TRYBOOT.to_string(), Script::always(Ok(b"1".to_vec())));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
        assert!(k.mount_calls.is_empty());
        assert!(k.write_calls.is_empty());
    }

    #[test]
    fn a_tryboot_flag_as_a_big_endian_cell_is_understood_too() {
        let mut k = tryboot_machine(
            &[0, 0, 0, 1],
            &[0, 0, 0, 2],
            Script::always(Ok(AUTOBOOT.to_string())),
        );
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert_eq!(got, Confirmation::Confirmed { partition: 2 });
    }

    // -- committing, through the port -----------------------------------------

    #[test]
    fn a_trial_boot_writes_a_temp_file_syncs_renames_and_syncs_again() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );

        assert_eq!(got, Confirmation::Confirmed { partition: 2 });
        let temp = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_TEMP}");
        let path = format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}");
        assert_eq!(k.write_calls.len(), 1);
        assert!(k.write_calls.first().is_some_and(
            |(p, c)| p == &temp && String::from_utf8_lossy(c).contains("boot_partition=2")
        ));
        assert_eq!(k.rename_calls, vec![(temp, path)]);
        assert_eq!(k.sync_calls, 2, "once before the rename and once after");
        assert!(
            k.umount_calls
                .contains(&AUTOBOOT_PARTITION_MOUNT.to_string())
        );
    }

    /// The regression this path was corrected for on 2026-08-23.
    ///
    /// Booted from `p2`, the commit must write `p1`'s file and put `2` in it.
    /// The old code wrote `p2`'s — a file the firmware was then watched
    /// ignoring, twice, on a real board. Kept here as well as in
    /// `rivet-adapter-image` deliberately: the defect was reached through
    /// *this* function, so this is the call path that must stay proven — and it
    /// now crosses a crate boundary and an adapter to get there, which is one
    /// more place it could have been broken.
    #[test]
    fn a_slot_b_commit_writes_the_first_partitions_file_and_not_the_booted_one() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );

        assert_eq!(got, Confirmation::Confirmed { partition: 2 });

        let source = k.mount_calls.first().map(|c| c.source.clone());
        assert_eq!(
            source,
            Some(autoboot_partition_device()),
            "the commit mounted the booted partition instead of the first one"
        );
        assert_eq!(source, Some("/dev/mmcblk0p1".to_string()));

        let target = k.mount_calls.first().map(|c| c.target.clone());
        assert_eq!(
            target,
            Some(AUTOBOOT_PARTITION_MOUNT.to_string()),
            "a shared mount point lets a convergent EBUSY substitute the wrong filesystem"
        );

        assert!(
            k.write_calls
                .first()
                .is_some_and(|(_, c)| String::from_utf8_lossy(c).contains("boot_partition=2")),
            "the value written is the RUNNING slot's, even though the file is p1's"
        );
    }

    #[test]
    fn the_partition_is_mounted_writable_for_the_commit_and_unmounted_after() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let _ = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        let call = k.mount_calls.first();
        assert!(call.is_some_and(|c| !c.flags.contains(crate::kernel::MountFlags::RDONLY)));
        assert_eq!(k.umount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    #[test]
    fn an_already_correct_file_is_not_rewritten_at_all() {
        let mut k = tryboot_machine(b"1", b"1", Script::always(Ok(AUTOBOOT.to_string())));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert_eq!(got, Confirmation::AlreadyDefault { partition: 1 });
        assert!(k.write_calls.is_empty());
        assert_eq!(k.sync_calls, 0);
        assert_eq!(k.umount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    #[test]
    fn an_unreadable_autoboot_file_declines_and_still_unmounts() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Err(Errno::EIO)));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
        assert!(k.write_calls.is_empty());
        assert_eq!(k.umount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    #[test]
    fn a_failed_rename_declines_and_does_not_claim_success() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        k.rename_results = Script::always(Err(Errno::EROFS));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
        assert_eq!(k.sync_calls, 1, "the second sync must not have happened");
    }

    /// Which failures a later attempt could answer differently. The medium
    /// refusing is one; a record the port will not rewrite, and a firmware
    /// that will not say whether this is a trial, are not.
    #[test]
    fn a_failure_says_whether_the_medium_refused() {
        let mut rename = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        rename.rename_results = Script::always(Err(Errno::ENOENT));
        let got = confirm_boot(
            &mut rename,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(
            matches!(got, Confirmation::Failed { medium: true, ref why } if why.contains("ENOENT")),
            "{got:?}"
        );

        let mut nonsense = tryboot_machine(b"1", b"2", Script::always(Ok("nonsense\n".into())));
        let got = confirm_boot(
            &mut nonsense,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(
            matches!(got, Confirmation::Failed { medium: false, .. }),
            "{got:?}"
        );

        let mut no_flag = FakeKernel::new();
        let got = confirm_boot(
            &mut no_flag,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(
            matches!(got, Confirmation::Failed { medium: false, .. }),
            "{got:?}"
        );
    }

    #[test]
    fn a_failed_write_declines_before_touching_the_real_file() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        k.write_results = Script::always(Err(Errno::EROFS));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
        assert!(k.rename_calls.is_empty());
    }

    #[test]
    fn a_boot_partition_that_will_not_mount_declines() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        k.mount_results = Script::always(Err(Errno::ENODEV));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(matches!(got, Confirmation::Failed { .. }), "{got:?}");
        assert!(k.write_calls.is_empty());
    }

    /// A mount table showing a filesystem at the record's mount point — the
    /// leaked read-only mount of a reader that died holding it. Hand-composed.
    fn with_a_leaked_mount(k: &mut FakeKernel) {
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(format!(
                "/dev/mmcblk0p1 {AUTOBOOT_PARTITION_MOUNT} vfat ro,nosuid,nodev,noexec 0 0\n"
            ))),
        );
    }

    /// **A leaked mount at the record's mount point is reclaimed, and the
    /// commit lands on its first attempt.** PID 1 holds the lock, so the mount
    /// belongs to nobody still using it: it is unmounted, the partition mounted
    /// afresh writable, the reclaim said on the console, and the commit
    /// written through the fresh mount.
    #[test]
    fn a_leaked_mount_is_reclaimed_and_the_commit_lands() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        with_a_leaked_mount(&mut k);
        // The leak answers the first mount; the fresh one succeeds.
        k.mount_results = Script::sequence(vec![Err(Errno::EBUSY)], Ok(()));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert_eq!(got, Confirmation::Confirmed { partition: 2 });
        assert_eq!(k.mount_calls.len(), 2);
        assert!(
            k.mount_calls
                .iter()
                .all(|c| !c.flags.contains(crate::kernel::MountFlags::RDONLY))
        );
        assert_eq!(
            k.umount_calls,
            vec![
                AUTOBOOT_PARTITION_MOUNT.to_string(),
                AUTOBOOT_PARTITION_MOUNT.to_string()
            ],
            "the orphan, then the fresh mount"
        );
        assert!(
            k.lines.iter().any(|l| l.contains("reclaimed an orphaned mount")),
            "{:?}",
            k.lines
        );
        assert_eq!(
            k.lock_calls,
            vec![
                format!("lock {}", rivet_adapter_image::AUTOBOOT_LOCK),
                format!("unlock {}", rivet_adapter_image::AUTOBOOT_LOCK),
            ]
        );
    }

    /// `EBUSY` with nothing at the record's mount point is not a mount to
    /// reclaim or to use: it is a medium refusal, and nothing is written.
    #[test]
    fn busy_with_nothing_at_the_mount_point_declines_and_writes_nothing() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        k.mount_results = Script::always(Err(Errno::EBUSY));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(
            matches!(&got, Confirmation::Failed { medium: true, why } if why.contains("EBUSY")),
            "{got:?}"
        );
        assert!(k.write_calls.is_empty());
        assert!(k.umount_calls.is_empty());
    }

    /// **A lock another process holds is a medium refusal, at once.** Nothing
    /// is mounted or written, and no time passes: PID 1's loop does not wait on
    /// it.
    #[test]
    fn a_held_lock_declines_as_the_medium_without_mounting_or_waiting() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        k.lock_results = Script::always(Err(Errno::EAGAIN));
        let got = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(
            matches!(&got, Confirmation::Failed { medium: true, why }
                if why.contains(rivet_adapter_image::AUTOBOOT_LOCK)),
            "{got:?}"
        );
        assert!(k.mount_calls.is_empty());
        assert!(k.write_calls.is_empty());
        assert!(k.slept_ms.is_empty());
        assert_eq!(k.now_ms(), 0);
    }

    #[test]
    fn every_outcome_has_a_console_line_that_says_what_happened() {
        let outcomes = [
            Confirmation::Withheld { why: "w".into() },
            Confirmation::NothingToConfirm,
            Confirmation::AlreadyDefault { partition: 1 },
            Confirmation::Confirmed { partition: 2 },
            Confirmation::Failed {
                why: "f".into(),
                medium: true,
            },
        ];
        for outcome in outcomes {
            assert!(!outcome.summary().is_empty(), "{outcome:?} has no summary");
        }
    }
}

#[cfg(test)]
mod reachability_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::tests::{AUTOBOOT, healthy, no_document, running_supervisor, tryboot_machine};
    use super::*;
    use rivet_adapter_image::PiFirmware;
    use crate::testkernel::{FakeKernel, Script};

    /// Remove the marker the fake seeds by default, leaving a machine nobody
    /// has reached.
    fn unreached(k: &mut FakeKernel) {
        k.files
            .remove(rivet_core::operation::status::REACHED_MARKER_PATH);
    }

    /// **The failure this was built for** — `PLAN.md` §104.20. A trial boot
    /// that is healthy in every other way, and that nothing can talk to, is
    /// not promoted.
    #[test]
    fn a_trial_boot_nothing_has_reached_is_not_confirmed() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        unreached(&mut k);
        let outcome = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert_eq!(outcome, Confirmation::WaitingToBeReached);
    }

    /// **`NothingToConfirm` still comes first.** An ordinary boot has no slot
    /// on probation, so reachability is not its business — and a check placed
    /// in `gate` made an ordinary boot report `NOTHING HAS REACHED THIS
    /// MACHINE`, which is a frightening sentence about a machine that is fine.
    /// The existing tests are what caught it.
    #[test]
    fn an_ordinary_boot_is_unaffected_by_reachability() {
        let mut k = tryboot_machine(b"0", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        unreached(&mut k);
        let outcome = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert_eq!(outcome, Confirmation::NothingToConfirm);
    }

    /// **It says what to do**, because the console is the only surface a
    /// machine in this state has — by definition, since the thing it is
    /// missing is a way to be talked to.
    #[test]
    fn the_summary_says_how_to_resolve_it_and_what_happens_otherwise() {
        let said = Confirmation::WaitingToBeReached.summary();
        assert!(said.contains("Connect to it"), "{said}");
        assert!(
            said.contains("returns to the previous slot"),
            "the consequence of leaving it must be stated: {said}"
        );
        assert!(
            !said.contains("could NOT be confirmed"),
            "this is `not yet`, not a failure: {said}"
        );
    }

    /// A machine that HAS been reached confirms exactly as it always did, so
    /// the ordinary deploy-and-check flow is untouched.
    #[test]
    fn a_trial_boot_that_was_reached_still_confirms() {
        let mut k = tryboot_machine(b"1", b"2", Script::always(Ok(AUTOBOOT.to_string())));
        let outcome = confirm_boot(
            &mut k,
            &PiFirmware,
            &healthy(),
            &no_document(),
            &running_supervisor(),
        );
        assert!(
            matches!(
                outcome,
                Confirmation::Confirmed { .. } | Confirmation::AlreadyDefault { .. }
            ),
            "a reached trial boot must still be promoted, got {outcome:?}"
        );
    }
}
