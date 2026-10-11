//! Mounting `/state`, the one filesystem this machine may write to.
//!
//! **`ADR-017` §3.1.5 item 1 was amended on 2026-08-23** from "mount the
//! pseudo-filesystems" to "mount the boot-time filesystems: the
//! pseudo-filesystems and `/state`". `rivet_core::state`'s module
//! documentation carries the argument — in short: the journal lives on `/state`
//! (`ADR-017` §7), and `ADR-017` §8 item 2 already ruled that a component
//! cannot own the thing its own failure would have to be recorded in.
//!
//! # Why this is not just another entry in `stage0_plan`
//!
//! It looks like one. It is not, and the difference is the whole content of this
//! module: **a pseudo-filesystem cannot fail in an interesting way, and a block
//! device can.**
//!
//! `mount("tmpfs", "/run", "tmpfs", …)` fails if the kernel lacks tmpfs or the
//! mount point is missing, and that is the end of the list. `/state` can be
//! absent (a card written with a different tool), unformatted (`p5` exists and
//! holds nothing), corrupt (`EIO` from the card), or *present and not what we
//! think it is*. The last is the one worth code: an `ext4` partition mounted as
//! `ext2` **succeeds** and silently runs without a journal, so the type is read
//! back from the kernel rather than assumed from the fact that `mount(2)`
//! returned zero (`AGENTS.md` §5 — report what the system said).
//!
//! It also has a source. Every entry in `stage0_plan` has a conventional source
//! string the kernel ignores; this one names a device that has to be right.
//! `rivet_core::state::state_device` composes it, and `crate::provision` owns
//! the disk name — the same one it mounted `p1` from on the first hardware boot.
//!
//! # It never fails the boot
//!
//! Same rule as everything else PID 1 does: report and survive. The verdict
//! folds into `crate::boot::BootHealth` as **`Degraded`**, never
//! `Unconfirmable`, for the reason `ADR-018` §8 gives — a rollback cannot repair
//! a `/state` an update never touches, so rolling back would loop without
//! addressing the fault. `crate::boot::consequence_of` says on the console what
//! has actually been lost.
//!
//! # Which device, and how that is decided — amended 2026-08-28
//!
//! ~~The device path. [`state_device`] composes it from the disk it is on, and
//! this crate's is `/dev/mmcblk0`.~~ **`PLAN.md` §62.** The disk name was the
//! one thing here that a card cannot tell you: the same card is `/dev/mmcblk0`
//! in the board's slot, `/dev/sda` in a USB enclosure and `/dev/vda` behind
//! virtio, and a machine that composes the path from a constant is right about
//! exactly one of those.
//!
//! So [`resolve_device`] asks the kernel which partition carries `/state`'s GPT
//! label, through `rivet_core::partlabel` — the same label
//! `cmd/rivet-image` wrote, found the same way the kernel already resolves
//! `cmdline.txt`'s `root=PARTLABEL=system-a`. It is a directory listing and a
//! handful of small reads, all through the [`Kernel`] port, and it is the
//! cheapest thing in this file.
//!
//! **The composed path survives as the fall-back**, so a machine whose `/sys`
//! did not mount behaves exactly as every machine did before this change. That
//! is deliberate: this is PID 1, and a new lookup that could make a booting
//! board stop booting would be a poor trade for a correctness the board did not
//! need.
//!
//! # What is unproven
//!
//! **This has never run on a board.** `cmd/rivet-image` formats `p5` as
//! `ext4` and the first hardware boot proved `/dev/mmcblk0p1` is reachable at
//! that path, but nothing has mounted `p5` on real hardware. What ~~a green
//! QEMU run~~ settles nothing here either, *because the emulator boots from an
//! initramfs and has no `/dev/mmcblk0` at all* — **is no longer true, and that
//! sentence was the whole of `PLAN.md` §61.2.** `tools/qemu/run.sh --card`
//! attaches a card image as a virtio-blk disk, the kernel parses the GPT
//! `cmd/rivet-image` wrote, and `/state` is a real `ext4` on a real partition
//! table under the emulator.
//!
//! What is still the board's, and `ADR-026` §4's row narrows to exactly this:
//! **SD/MMC timing, card wear, and the firmware.** A file behind virtio-blk
//! answers none of the three.

use rivet_core::partlabel::{self, Located};
use rivet_core::state::{
    STATE_FLAGS, STATE_FSTYPE, STATE_MOUNT_POINT, STATE_PARTLABEL, state_device,
};

use crate::boot::{BootHealth, MountOutcome, Step, mount_table_fstype};
use crate::kernel::{Errno, Kernel, MountFlags};
use rivet_adapter_image::BOOT_DISK;

/// The device `/state` would be on **if this machine booted from the disk this
/// crate was built assuming**, which is the fall-back and no longer the answer.
///
/// Composed rather than spelled, so that the partition number has one
/// definition (`rivet_core::state::STATE_PARTITION`, which
/// `cmd/rivet-image`'s `layout` also uses) and the `p` infix rule is applied
/// rather than assumed.
pub fn composed_device() -> String {
    state_device(BOOT_DISK)
}

/// Which device this boot is going to mount, and **how it came to that**.
///
/// The provenance is carried rather than discarded because the console line it
/// produces is the only place an operator can see the difference, and the
/// difference is large: a device found by its label is a fact about the card in
/// this machine, and a composed one is an assumption this crate was compiled
/// with. `AGENTS.md` §5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chosen {
    /// A partition carries `/state`'s GPT label, and this is it.
    ByLabel {
        device: String,
        /// Its 1-based number, when the kernel gave one. Reported because a
        /// label found on a partition other than `ADR-018` §4's fifth is a card
        /// somebody should look at, and it is not this process's business to
        /// refuse one — the label is the authority, and the number is evidence.
        number: Option<u32>,
    },
    /// Nothing carries the label, so the composed path is used.
    ///
    /// `why` is what was actually observed, not a category: `/sys` unmounted,
    /// the directory unreadable, or a disk with no such label on it are three
    /// different repairs and the console has to distinguish them.
    Composed { device: String, why: String },
    /// **More than one partition carries the label, and nothing is mounted.**
    ///
    /// Two cards written by `cmd/rivet-image` on one machine. Either choice
    /// mounts data that belongs to another machine as this one's `/state`, and
    /// the wrong choice is silent and irreversible. Refusing degrades the boot
    /// — which `ADR-018` §8 already accepts for `/state` — and the operator is
    /// told both device names.
    Ambiguous { devices: Vec<String> },
}

impl Chosen {
    /// The device to hand `mount(2)`, or `None` when this boot will not mount.
    fn device(&self) -> Option<&str> {
        match self {
            Self::ByLabel { device, .. } | Self::Composed { device, .. } => Some(device),
            Self::Ambiguous { .. } => None,
        }
    }
}

/// Ask the kernel which partition carries `/state`'s label.
///
/// Every failure to read something is folded into "no label found", with the
/// observation kept for the console: from this decision's point of view an
/// unreadable `uevent` and a partition that is not the one wanted are the same
/// thing, and treating a missing `/sys` as an error would give PID 1 a way to
/// fail at a step that is meant to be an improvement on a constant.
pub fn resolve_device(k: &mut dyn Kernel) -> Chosen {
    let composed = composed_device();

    let names = match k.read_dir(partlabel::SYS_BLOCK_DIR) {
        Ok(names) => names,
        Err(errno) => {
            return Chosen::Composed {
                device: composed,
                why: format!(
                    "cannot read {}: {errno}. Either /sys did not mount or this \
                     kernel has no sysfs; without it there is no way to ask which \
                     disk this is",
                    partlabel::SYS_BLOCK_DIR
                ),
            };
        }
    };

    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        if let Ok(uevent) = k.read_file(&partlabel::uevent_path(&name)) {
            entries.push((name, uevent));
        }
    }

    match partlabel::find(STATE_PARTLABEL, &entries) {
        Located::At(part) => Chosen::ByLabel {
            device: part.device,
            number: part.number,
        },
        Located::Nowhere => Chosen::Composed {
            device: composed,
            why: format!(
                "no partition of the {} block device(s) this kernel reports carries \
                 the GPT label {STATE_PARTLABEL:?}",
                entries.len()
            ),
        },
        Located::Ambiguous(devices) => Chosen::Ambiguous { devices },
    }
}

/// Say which device was chosen and why, before anything is mounted on it.
///
/// Before rather than after, so that a mount which hangs or panics still leaves
/// the operator the name of the device it was hanging on.
fn report_choice(k: &mut dyn Kernel, chosen: &Chosen) {
    match chosen {
        Chosen::ByLabel { device, number } => {
            let at = match number {
                Some(n) => format!("partition {n}"),
                None => "a partition the kernel did not number".to_string(),
            };
            k.say(&format!(
                "  found     {device} by its GPT label {STATE_PARTLABEL:?}, at {at}"
            ));
            if !matches!(number, Some(n) if *n == rivet_core::state::STATE_PARTITION) {
                k.say(&format!(
                    "  NOTE    ADR-018 §4 puts that label on partition {}. The label is \
                     the authority and this is what will be mounted; the card is worth a look.",
                    rivet_core::state::STATE_PARTITION
                ));
            }
        }
        Chosen::Composed { device, why } => {
            k.say(&format!("  no label  {why}"));
            k.say(&format!(
                "  assuming  {device}, composed from the disk this build assumes ({BOOT_DISK}).",
            ));
            k.say("            That is right for a card in a Pi's own slot and wrong for the");
            k.say("            same card over USB or behind virtio (PLAN.md §62).");
        }
        Chosen::Ambiguous { devices } => {
            k.say(&format!(
                "  REFUSED   {} partitions carry the GPT label {STATE_PARTLABEL:?}: {}",
                devices.len(),
                devices.join(", ")
            ));
            k.say("            Each is some machine's /state. Mounting either would put another");
            k.say("            machine's data here, silently and irreversibly, so nothing is");
            k.say("            mounted. Detach all but one and reboot (AGENTS.md §5).");
        }
    }
}

/// Classify what `mount(2)` said about a **block device**, which is a different
/// set of possibilities from a pseudo-filesystem's.
///
/// Each arm names a state a card can actually be in and what would put it there.
/// `crate::boot::mount_detail` covers the pseudo-filesystem cases and shares
/// nothing with this deliberately: `ENODEV` means "no such filesystem type"
/// there and the same thing here, but `ENOENT` means "the mount point is
/// missing" there and "**the device node is missing**" here, which is a
/// completely different repair.
fn mount_detail(device: &str, errno: Errno, by_label: bool) -> String {
    match errno {
        // The one message that depends on how the device was chosen, because
        // the two ENOENTs are different faults. A composed path is missing
        // because the assumption about the disk was wrong; a path the kernel
        // itself just named is missing because devtmpfs did not make the node,
        // which is a kernel-configuration problem and not a card problem.
        Errno::ENOENT if by_label => format!(
            "mount({device}): {errno} — the kernel reported this partition in \
             {} and then had no device node for it. That is devtmpfs, not the card: \
             check CONFIG_DEVTMPFS and CONFIG_DEVTMPFS_MOUNT (ADR-024 §3)",
            partlabel::SYS_BLOCK_DIR
        ),
        Errno::ENOENT => format!(
            "mount({device}): {errno} — no such device node. The card was written by \
             something that did not lay out ADR-018 §4's five partitions, or this board \
             booted from a disk other than {BOOT_DISK}"
        ),
        Errno::ENODEV => format!(
            "mount({device}): {errno} — the kernel cannot mount {STATE_FSTYPE}; check \
             CONFIG_EXT4_FS (ADR-024)"
        ),
        Errno::EINVAL => format!(
            "mount({device}): {errno} — there is no {STATE_FSTYPE} filesystem there. The \
             partition exists and was never formatted, or its superblock is gone"
        ),
        Errno::EIO => format!(
            "mount({device}): {errno} — the card returned an I/O error. This is hardware, \
             not configuration (ADR-018 §7)"
        ),
        Errno::EACCES | Errno::EPERM => format!(
            "mount({device}): {errno} — no permission; pid 1 should have CAP_SYS_ADMIN, so \
             this is a namespace or LSM problem rather than a card problem"
        ),
        _ => format!("mount({device}): {errno}"),
    }
}

/// Mount `/state`, converging rather than failing if it is already there.
///
/// The mount point is **not** created. It is in the image
/// (`cmd/rivet-image`'s `system::STATE_MOUNT_POINT`) because the root
/// filesystem is read-only and a `mkdir` here would fail with `EROFS` — so an
/// attempt would only ever produce a misleading error about the wrong thing.
/// The refusal that matters is `mount(2)`'s.
pub fn mount_state(k: &mut dyn Kernel, chosen: &Chosen) -> MountOutcome {
    // Idempotence by asking the kernel, exactly as `boot::perform_mount` does.
    // Unlike tmpfs, a second mount of the same block device at the same point
    // gives EBUSY rather than stacking — but PID 1 does not get to rely on the
    // filesystem type's behaviour for its correctness.
    //
    // Checked BEFORE the ambiguity refusal below, and that order is deliberate:
    // a converged mount is a fact about this machine, and refusing a `/state`
    // that is already mounted and working would report a fault where there is
    // none.
    if let Ok(table) = k.read_mounts()
        && mount_table_fstype(&table, STATE_MOUNT_POINT).is_some()
    {
        return MountOutcome::Converged;
    }

    let Some(device) = chosen.device() else {
        let Chosen::Ambiguous { devices } = chosen else {
            unreachable!("only Ambiguous has no device")
        };
        return MountOutcome::Refused {
            step: Step::Mount,
            errno: None,
            detail: format!(
                "nothing was mounted: {} partitions carry the GPT label \
                 {STATE_PARTLABEL:?} ({}), and choosing between them would mount \
                 another machine's data here",
                devices.len(),
                devices.join(", ")
            ),
        };
    };
    let by_label = matches!(chosen, Chosen::ByLabel { .. });

    match k.mount(
        device,
        STATE_MOUNT_POINT,
        STATE_FSTYPE,
        MountFlags::from_core(STATE_FLAGS),
        None,
    ) {
        Ok(()) => MountOutcome::Mounted,
        Err(Errno::EBUSY) => MountOutcome::Converged,
        Err(errno) => MountOutcome::Refused {
            step: Step::Mount,
            errno: Some(errno),
            detail: mount_detail(device, errno, by_label),
        },
    }
}

/// Item 1's second half, end to end: mount `/state`, say what the kernel
/// thinks is there, and fold the result into the boot's verdict.
///
/// Takes and returns the verdict rather than an outcome, so that the
/// composition root stays a list of phases and the decision about **what a
/// failure here is worth** lives in this module with the reasoning for it.
///
/// The verdict is re-reported only when `/state` changed it. `ADR-017` §3.1.6's
/// three-line report is written for an operator at a serial console; printing
/// "mount phase healthy" twice on every boot would teach that reader to skip it.
pub fn bring_up_state(k: &mut dyn Kernel, health: BootHealth) -> BootHealth {
    let outcome = mount_and_report(k);
    let folded = health.clone().with(STATE_MOUNT_POINT, &outcome);
    if folded != health {
        crate::boot::report_health(k, &folded);
    }
    folded
}

/// Mount it, then say what the kernel thinks is there.
///
/// The read-back is not decoration. `mount(2)` returning zero is *our* claim
/// that `/state` is an `ext4`; `/proc/self/mounts` is the kernel's, and the two
/// can differ — mounting an `ext4` partition as `ext2` succeeds and drops the
/// journal, which is a machine that loses writes on a power cut and looks
/// perfectly healthy until it does. `AGENTS.md` §5.
fn mount_and_report(k: &mut dyn Kernel) -> MountOutcome {
    k.say("rivet-initd: mounting /state, the only writable filesystem (ADR-018 §4)");
    let composed = composed_device();
    let chosen = resolve_device(k);
    report_choice(k, &chosen);
    let outcome = mount_state(k, &chosen);
    // The composed path when nothing is going to be mounted, so the two lines
    // below still name something rather than being written around a None. They
    // are unreachable for `Ambiguous`, which takes the Refused arm.
    let device = chosen.device().unwrap_or(&composed).to_string();
    match &outcome {
        MountOutcome::Mounted => k.say(&format!(
            "  mounted   {STATE_FSTYPE:<9} {device:<15} on {STATE_MOUNT_POINT:<9} [{}]",
            options()
        )),
        MountOutcome::Converged => k.say(&format!(
            "  already   {STATE_FSTYPE:<9} {device:<15} on {STATE_MOUNT_POINT:<9} (converged, not an error)"
        )),
        MountOutcome::Refused { detail, .. } => k.say(&format!("  REFUSED   {detail}")),
    }
    report_state_evidence(k, &outcome);
    outcome
}

/// Read back what the kernel says is at [`STATE_MOUNT_POINT`], and complain if
/// it is not what was asked for.
fn report_state_evidence(k: &mut dyn Kernel, outcome: &MountOutcome) {
    if matches!(outcome, MountOutcome::Refused { .. }) {
        return;
    }
    match k.read_mounts() {
        Ok(table) => match mount_table_fstype(&table, STATE_MOUNT_POINT) {
            Some(fstype) if fstype == STATE_FSTYPE => {
                k.say(&format!("  the kernel reports {STATE_MOUNT_POINT} as {fstype}"));
            }
            Some(fstype) => {
                // Not a refusal — something is mounted and it is writable — but
                // it is not the filesystem this design reasoned about.
                k.say(&format!(
                    "  WARNING the kernel reports {STATE_MOUNT_POINT} as {fstype}, not {STATE_FSTYPE}."
                ));
                k.say("          An ext4 mounted as ext2 runs with no journal and loses writes on");
                k.say("          a power cut, silently. ADR-018 §7 chose ext4 on purpose.");
            }
            None => k.say(&format!(
                "  WARNING mount(2) succeeded but the kernel's table shows nothing at {STATE_MOUNT_POINT}"
            )),
        },
        // The normal state of a boot in which /proc did not mount, which is
        // already Unconfirmable for its own reasons.
        Err(e) => k.say(&format!("  cannot read the mount table to confirm: {e}")),
    }
}

/// The options, rendered the way `mount(8)` would, for the console.
///
/// Through `crate::boot`'s renderer rather than a second match here, so that a
/// flag added to `rivet_core::mount::MountFlag` is named in one place. There
/// is no mount data: `ext4`'s defaults are what `ADR-018` §7 asked for and
/// anything else would be a decision the record has not taken.
fn options() -> String {
    crate::boot::render_options(STATE_FLAGS, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    const MOUNTED: &str = "proc /proc proc rw 0 0\n/dev/mmcblk0p5 /state ext4 rw 0 0\n";

    fn kernel_with_no_mount_table() -> FakeKernel {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Err(Errno::ENOENT)),
        );
        k
    }

    /// **The choice a machine with no readable `/sys` makes**, and the one most
    /// of these tests want: they are about what `mount(2)` answered, not about
    /// which device was chosen, and the composed path is what every one of them
    /// asserted against before the label lookup existed. Said once here rather
    /// than restated in each.
    fn composed() -> Chosen {
        Chosen::Composed {
            device: composed_device(),
            why: "these tests are not about the lookup".to_string(),
        }
    }

    /// A partition's `uevent`, in the kernel's own shape.
    fn uevent(devname: &str, partn: u32, partname: &str) -> String {
        format!(
            "MAJOR=254\nMINOR={partn}\nDEVNAME={devname}\nDEVTYPE=partition\n\
             PARTN={partn}\nPARTNAME={partname}\n"
        )
    }

    /// A whole disk's, which carries no label.
    fn disk_uevent(devname: &str) -> String {
        format!("MAJOR=254\nMINOR=0\nDEVNAME={devname}\nDEVTYPE=disk\n")
    }

    /// A kernel whose `/sys/class/block` holds exactly these entries.
    ///
    /// Anything not listed reads `ENOENT`, which is what a real sysfs does and
    /// is the case `resolve_device` has to fold into "not this one" rather than
    /// into a failure.
    fn kernel_with_block_devices(entries: &[(&str, String)]) -> FakeKernel {
        let mut k = kernel_with_no_mount_table();
        k.dirs_mut().insert(
            partlabel::SYS_BLOCK_DIR.to_string(),
            Script::always(Ok(entries.iter().map(|(n, _)| (*n).to_string()).collect())),
        );
        for (name, text) in entries {
            k.files_mut().insert(
                partlabel::uevent_path(name),
                Script::always(Ok(text.clone())),
            );
        }
        k
    }

    /// The emulator's card: one virtio disk with `ADR-018` §4's five partitions.
    fn a_card_behind_virtio() -> FakeKernel {
        kernel_with_block_devices(&[
            ("vda", disk_uevent("vda")),
            ("vda1", uevent("vda1", 1, "boot-a")),
            ("vda2", uevent("vda2", 2, "boot-b")),
            ("vda3", uevent("vda3", 3, "system-a")),
            ("vda4", uevent("vda4", 4, "system-b")),
            ("vda5", uevent("vda5", 5, STATE_PARTLABEL)),
        ])
    }

    // -- the device --------------------------------------------------------

    /// The partition number, the disk and the `p` infix, in one place. Getting
    /// any of the three wrong mounts nothing, or — worse — mounts a `/system`
    /// slot over `/state`.
    #[test]
    fn the_composed_device_is_partition_five_of_the_disk_this_build_assumes() {
        assert_eq!(composed_device(), "/dev/mmcblk0p5");
        assert_eq!(rivet_core::state::STATE_PARTITION, 5);
        // And it is not one of the four partitions that are not /state.
        for n in [1, 2, 3, 4] {
            assert_ne!(composed_device(), format!("{BOOT_DISK}p{n}"));
        }
    }

    // -- the mount itself --------------------------------------------------

    #[test]
    fn a_first_boot_mounts_the_device_as_ext4_with_the_ruled_flags() {
        let mut k = kernel_with_no_mount_table();
        assert_eq!(mount_state(&mut k, &composed()), MountOutcome::Mounted);
        let call = k.mount_calls.first();
        assert!(call.is_some(), "nothing was mounted");
        assert!(call.is_some_and(|c| c.source == "/dev/mmcblk0p5"
            && c.target == "/state"
            && c.fstype == "ext4"
            && c.data.is_none()));
    }

    /// `ADR-018` §7's measures, asserted against what actually reached
    /// `mount(2)` rather than against the constant that fed it — the same
    /// lesson `the_two_binaries_are_copied_from_the_inputs_and_are_executable`
    /// records, where comparing a constant with itself caught nothing.
    #[test]
    fn the_flags_that_reach_the_kernel_are_noatime_nosuid_nodev_and_not_noexec() {
        let mut k = kernel_with_no_mount_table();
        let _ = mount_state(&mut k, &composed());
        let call = k.mount_calls.first();
        assert!(call.is_some_and(|c| c.flags.contains(MountFlags::NOATIME)));
        assert!(call.is_some_and(|c| c.flags.contains(MountFlags::NOSUID)));
        assert!(call.is_some_and(|c| c.flags.contains(MountFlags::NODEV)));
        assert!(
            call.is_some_and(|c| !c.flags.contains(MountFlags::NOEXEC)),
            "noexec would stop ADR-018 §6's extensions and ADR-015's applications"
        );
        assert!(
            call.is_some_and(|c| !c.flags.contains(MountFlags::RDONLY)),
            "/state is the ONE writable filesystem; read-only defeats the point"
        );
    }

    #[test]
    fn an_already_mounted_state_converges_without_a_second_mount_call() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(MOUNTED.to_string())),
        );
        assert_eq!(mount_state(&mut k, &composed()), MountOutcome::Converged);
        assert!(k.mount_calls.is_empty(), "it mounted a second time");
    }

    /// The backstop for the case above, when `/proc` is not readable and the
    /// convergence check cannot run.
    #[test]
    fn ebusy_from_the_kernel_is_convergence_rather_than_a_refusal() {
        let mut k = kernel_with_no_mount_table();
        k.mount_results = Script::always(Err(Errno::EBUSY));
        assert_eq!(mount_state(&mut k, &composed()), MountOutcome::Converged);
    }

    // -- the ways a card can be wrong --------------------------------------

    /// Each errno a real card can produce, and the requirement that the console
    /// message names the repair rather than the syscall. These are read by
    /// somebody at a serial console with no other diagnostic channel.
    #[test]
    fn every_refusal_says_what_is_actually_wrong_with_the_card() {
        for (errno, must_contain) in [
            (Errno::ENOENT, "no such device node"),
            (Errno::EINVAL, "never formatted"),
            (Errno::EIO, "hardware"),
            (Errno::ENODEV, "CONFIG_EXT4_FS"),
            (Errno::EPERM, "CAP_SYS_ADMIN"),
        ] {
            let mut k = kernel_with_no_mount_table();
            k.mount_results = Script::always(Err(errno));
            let got = mount_state(&mut k, &composed());
            assert!(
                matches!(&got, MountOutcome::Refused { detail, .. } if detail.contains(must_contain)),
                "{errno} must produce a message containing {must_contain:?}, got {got:?}"
            );
            assert!(
                matches!(&got, MountOutcome::Refused { errno: Some(e), .. } if *e == errno),
                "the errno must survive into the outcome"
            );
        }
    }

    /// The distinction that would otherwise be lost: `ENOENT` from a *block
    /// device* mount is a missing device, not a missing mount point. Confusing
    /// the two sends the reader to look at the wrong filesystem.
    #[test]
    fn enoent_here_names_the_device_and_not_the_mount_point() {
        let mut k = kernel_with_no_mount_table();
        k.mount_results = Script::always(Err(Errno::ENOENT));
        let got = mount_state(&mut k, &composed());
        assert!(
            matches!(&got, MountOutcome::Refused { detail, .. }
                if detail.contains("device node") && !detail.contains("mount point does not exist")),
            "{got:?}"
        );
    }

    /// PID 1 must not create this directory. It is on the read-only root, so an
    /// attempt produces `EROFS` and a console line about the wrong problem.
    #[test]
    fn the_mount_point_is_never_mkdired_because_the_image_carries_it() {
        let mut k = kernel_with_no_mount_table();
        let _ = mount_and_report(&mut k);
        assert!(
            !k.mkdir_calls.iter().any(|p| p == STATE_MOUNT_POINT),
            "PID 1 tried to create {STATE_MOUNT_POINT}; the image ships it"
        );
    }

    // -- evidence, read back from the kernel -------------------------------

    #[test]
    fn a_healthy_mount_reports_the_type_the_kernel_says_it_is() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::sequence(vec![Err(Errno::ENOENT)], Ok(MOUNTED.to_string())),
        );
        assert_eq!(mount_and_report(&mut k), MountOutcome::Mounted);
        assert!(
            k.lines.iter().any(|l| l.contains("reports /state as ext4")),
            "{:?}",
            k.lines
        );
        assert!(!k.lines.iter().any(|l| l.contains("WARNING")));
    }

    /// The case that motivates reading the table at all: `mount(2)` succeeded
    /// and the kernel mounted something else. An `ext4` opened as `ext2` runs
    /// with no journal and loses writes on a power cut — on a board with no
    /// battery and an SD card.
    #[test]
    fn a_filesystem_that_is_not_the_one_asked_for_is_a_loud_warning_not_a_success() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::sequence(
                vec![Err(Errno::ENOENT)],
                Ok("/dev/mmcblk0p5 /state ext2 rw 0 0\n".to_string()),
            ),
        );
        let got = mount_and_report(&mut k);
        // Still Mounted: something writable IS there, and PID 1 does not get to
        // unmount a machine's only persistent storage on a suspicion.
        assert_eq!(got, MountOutcome::Mounted);
        let console = k.lines.join("\n");
        assert!(console.contains("WARNING"), "{console}");
        assert!(console.contains("ext2"), "{console}");
        assert!(console.contains("no journal"), "{console}");
    }

    #[test]
    fn a_mount_that_succeeded_but_left_no_trace_in_the_table_is_reported() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::sequence(
                vec![Err(Errno::ENOENT)],
                Ok("proc /proc proc rw 0 0\n".to_string()),
            ),
        );
        let _ = mount_and_report(&mut k);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("shows nothing at /state")),
            "{:?}",
            k.lines
        );
    }

    /// A refused mount must not then claim anything about what is there.
    #[test]
    fn a_refused_mount_reports_no_evidence_at_all() {
        let mut k = kernel_with_no_mount_table();
        k.mount_results = Script::always(Err(Errno::EIO));
        let _ = mount_and_report(&mut k);
        assert!(
            !k.lines.iter().any(|l| l.contains("the kernel reports")),
            "{:?}",
            k.lines
        );
    }

    // -- the verdict -------------------------------------------------------

    /// **The ruling of 2026-08-23, as a test.** `ADR-018` §8: a rollback cannot
    /// repair a `/state` an update never touches, so an unconfirmable verdict
    /// would alternate between two healthy images for ever. Injecting `/state`
    /// into `ESSENTIAL_TO_PID1` must break this.
    #[test]
    fn a_missing_state_degrades_the_boot_and_still_lets_it_be_confirmed() {
        let mut k = kernel_with_no_mount_table();
        k.mount_results = Script::always(Err(Errno::EINVAL));
        let health = bring_up_state(&mut k, BootHealth::Healthy);
        assert_eq!(
            health,
            BootHealth::Degraded {
                missing: vec![STATE_MOUNT_POINT.to_string()],
            }
        );
        assert!(
            health.may_confirm_boot(),
            "rolling back would find the same damaged /state — ADR-018 §8"
        );
        // And the operator is told, on the phase that noticed it.
        assert!(
            k.lines.iter().any(|l| l.contains("DEGRADED /state")),
            "{:?}",
            k.lines
        );
    }

    /// The other half of the re-report rule: a boot in which everything worked
    /// must not print its verdict twice.
    #[test]
    fn a_healthy_state_does_not_repeat_the_verdict_on_the_console() {
        let mut k = kernel_with_no_mount_table();
        assert_eq!(
            bring_up_state(&mut k, BootHealth::Healthy),
            BootHealth::Healthy
        );
        assert_eq!(
            k.lines
                .iter()
                .filter(|l| l.contains("mount phase healthy"))
                .count(),
            0,
            "the verdict was re-reported although /state changed nothing: {:?}",
            k.lines
        );
    }

    #[test]
    fn a_healthy_state_leaves_the_verdict_alone() {
        for outcome in [MountOutcome::Mounted, MountOutcome::Converged] {
            assert_eq!(
                BootHealth::Healthy.with(STATE_MOUNT_POINT, &outcome),
                BootHealth::Healthy
            );
        }
    }

    /// The fold is monotone: `/state` working does not rescue a boot that is
    /// already unconfirmable, and `/state` failing does not downgrade one.
    #[test]
    fn folding_state_in_never_improves_a_verdict() {
        let unconfirmable = BootHealth::Unconfirmable {
            missing: vec!["/proc".to_string()],
        };
        assert_eq!(
            unconfirmable
                .clone()
                .with(STATE_MOUNT_POINT, &MountOutcome::Mounted),
            unconfirmable
        );
        let refused = MountOutcome::Refused {
            step: Step::Mount,
            errno: Some(Errno::EIO),
            detail: "a test".to_string(),
        };
        let got = unconfirmable.with(STATE_MOUNT_POINT, &refused);
        assert!(!got.may_confirm_boot());
        assert!(
            matches!(&got, BootHealth::Unconfirmable { missing } if missing.len() == 2),
            "both must be listed on the console: {got:?}"
        );
    }

    /// The console has to say what was lost, not just that something was. An
    /// operator reading `DEGRADED /state` has no way to know it means the
    /// machine keeps nothing.
    #[test]
    fn the_console_spells_out_what_a_missing_state_costs() {
        let mut k = kernel_with_no_mount_table();
        k.mount_results = Script::always(Err(Errno::EIO));
        let _ = bring_up_state(&mut k, BootHealth::Healthy);
        let console = k.lines.join("\n");
        for expected in [
            "journal",
            "/state/config",
            "/state/secrets",
            "rollback will NOT",
        ] {
            assert!(
                console.contains(expected),
                "missing {expected:?} from: {console}"
            );
        }
    }

    // -- the options string -------------------------------------------------

    #[test]
    fn the_console_options_match_the_flags_that_are_actually_set() {
        assert_eq!(options(), "noatime,nosuid,nodev");
    }

    // -- which device, and how it was decided ------------------------------

    /// **`PLAN.md` §62, and the whole of it.** The same card behind virtio is
    /// `/dev/vda5` and in a Pi's slot is `/dev/mmcblk0p5`; the label on it is
    /// the same in both. This is the emulator's half — and note that nothing
    /// about the assertion mentions `vda`, because the code under test never
    /// composes a name: it uses the one the kernel reported.
    #[test]
    fn the_partition_carrying_the_label_is_what_gets_mounted_whatever_the_disk_is_called() {
        let mut k = a_card_behind_virtio();
        let chosen = resolve_device(&mut k);
        assert_eq!(
            chosen,
            Chosen::ByLabel {
                device: "/dev/vda5".to_string(),
                number: Some(5),
            }
        );
        assert_eq!(mount_state(&mut k, &chosen), MountOutcome::Mounted);
        assert!(
            k.mount_calls
                .first()
                .is_some_and(|c| c.source == "/dev/vda5"),
            "{:?}",
            k.mount_calls
        );
    }

    /// The board's half of the same fact, by the same route. A card in a Pi's
    /// own slot resolves to exactly what this crate used to assume — which is
    /// what makes the change safe to ship to hardware that has never run it.
    #[test]
    fn the_same_lookup_on_a_board_finds_the_path_this_crate_used_to_assume() {
        let mut k = kernel_with_block_devices(&[
            ("mmcblk0", disk_uevent("mmcblk0")),
            ("mmcblk0p1", uevent("mmcblk0p1", 1, "boot-a")),
            ("mmcblk0p5", uevent("mmcblk0p5", 5, STATE_PARTLABEL)),
        ]);
        assert_eq!(
            resolve_device(&mut k),
            Chosen::ByLabel {
                device: composed_device(),
                number: Some(5),
            }
        );
    }

    /// A partition of a *whole disk* whose name happens to match must never be
    /// selected: `/dev/vda` is the disk, and mounting it would mount the
    /// partition table.
    #[test]
    fn a_disk_is_never_chosen_only_a_partition_is() {
        let mut k = kernel_with_block_devices(&[(STATE_PARTLABEL, disk_uevent(STATE_PARTLABEL))]);
        assert!(matches!(resolve_device(&mut k), Chosen::Composed { .. }));
    }

    // -- falling back ------------------------------------------------------

    /// The safety property of the whole change: a machine that cannot read
    /// `/sys` behaves exactly as every machine behaved before it existed.
    #[test]
    fn an_unreadable_sysfs_falls_back_to_the_composed_path_and_says_why() {
        let mut k = kernel_with_no_mount_table(); // no /sys/class/block at all
        let chosen = resolve_device(&mut k);
        assert!(
            matches!(&chosen, Chosen::Composed { device, why }
                if device == &composed_device() && why.contains(partlabel::SYS_BLOCK_DIR)),
            "{chosen:?}"
        );
        assert_eq!(mount_state(&mut k, &chosen), MountOutcome::Mounted);
        assert!(
            k.mount_calls
                .first()
                .is_some_and(|c| c.source == "/dev/mmcblk0p5"),
            "{:?}",
            k.mount_calls
        );
    }

    /// A disk this tool did not write. Falling back is right — the card may
    /// still be the one this build assumes — and the console has to say that
    /// the lookup RAN and found nothing, not that it was skipped.
    #[test]
    fn a_disk_with_no_such_label_falls_back_and_says_how_many_were_examined() {
        let mut k = kernel_with_block_devices(&[
            ("sda", disk_uevent("sda")),
            ("sda1", uevent("sda1", 1, "EFI System Partition")),
        ]);
        let chosen = resolve_device(&mut k);
        assert!(
            matches!(&chosen, Chosen::Composed { why, .. } if why.contains('2')),
            "{chosen:?}"
        );
        report_choice(&mut k, &chosen);
        let console = k.lines.join("\n");
        assert!(console.contains("no label"), "{console}");
        assert!(console.contains(BOOT_DISK), "{console}");
    }

    /// An entry whose `uevent` cannot be read is not a failure — it is one
    /// fewer candidate. Some entries in this directory are always unreadable.
    #[test]
    fn an_unreadable_uevent_removes_a_candidate_rather_than_failing_the_lookup() {
        let mut k = a_card_behind_virtio();
        k.files_mut().insert(
            partlabel::uevent_path("vda3"),
            Script::always(Err(Errno::EIO)),
        );
        assert!(matches!(resolve_device(&mut k), Chosen::ByLabel { .. }));
    }

    // -- refusing ----------------------------------------------------------

    /// **Two cards, and nothing is mounted.** Each is some machine's `/state`;
    /// mounting the wrong one puts another machine's data here silently and
    /// irreversibly, so this is the one case in this module that refuses.
    #[test]
    fn two_partitions_with_the_label_mount_nothing_and_name_both() {
        let mut k = kernel_with_block_devices(&[
            ("vda5", uevent("vda5", 5, STATE_PARTLABEL)),
            ("vdb5", uevent("vdb5", 5, STATE_PARTLABEL)),
        ]);
        let chosen = resolve_device(&mut k);
        assert_eq!(
            chosen,
            Chosen::Ambiguous {
                devices: vec!["/dev/vda5".to_string(), "/dev/vdb5".to_string()],
            }
        );
        let outcome = mount_state(&mut k, &chosen);
        assert!(
            matches!(&outcome, MountOutcome::Refused { .. }),
            "{outcome:?}"
        );
        assert!(k.mount_calls.is_empty(), "it mounted one of them anyway");
        let MountOutcome::Refused { detail, errno, .. } = &outcome else {
            unreachable!()
        };
        assert!(
            detail.contains("/dev/vda5") && detail.contains("/dev/vdb5"),
            "{detail}"
        );
        assert_eq!(
            *errno, None,
            "no syscall was made, so there is no errno to report"
        );
    }

    /// And it degrades the boot rather than failing it, on `ADR-018` §8's
    /// terms: a rollback would find the same two cards.
    #[test]
    fn an_ambiguous_state_degrades_the_boot_and_still_lets_it_be_confirmed() {
        let mut k = kernel_with_block_devices(&[
            ("vda5", uevent("vda5", 5, STATE_PARTLABEL)),
            ("vdb5", uevent("vdb5", 5, STATE_PARTLABEL)),
        ]);
        let health = bring_up_state(&mut k, BootHealth::Healthy);
        assert_eq!(
            health,
            BootHealth::Degraded {
                missing: vec![STATE_MOUNT_POINT.to_string()],
            }
        );
        assert!(health.may_confirm_boot());
        let console = k.lines.join("\n");
        assert!(console.contains("Detach all but one"), "{console}");
    }

    /// A converged `/state` is not re-examined. Ordering: a machine that is
    /// already mounted and working must not be reported as ambiguous.
    #[test]
    fn a_state_that_is_already_mounted_converges_even_when_the_label_is_ambiguous() {
        let mut k = kernel_with_block_devices(&[
            ("vda5", uevent("vda5", 5, STATE_PARTLABEL)),
            ("vdb5", uevent("vdb5", 5, STATE_PARTLABEL)),
        ]);
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(MOUNTED.to_string())),
        );
        let chosen = resolve_device(&mut k);
        assert_eq!(mount_state(&mut k, &chosen), MountOutcome::Converged);
    }

    // -- what the console says ---------------------------------------------

    #[test]
    fn a_device_found_by_label_is_reported_as_found_with_the_label_in_it() {
        let mut k = a_card_behind_virtio();
        let chosen = resolve_device(&mut k);
        report_choice(&mut k, &chosen);
        let console = k.lines.join("\n");
        assert!(console.contains("/dev/vda5"), "{console}");
        assert!(console.contains(STATE_PARTLABEL), "{console}");
        assert!(
            !console.contains("NOTE"),
            "nothing is odd about this card: {console}"
        );
    }

    /// The label is the authority, so a card with it somewhere unexpected is
    /// mounted anyway — and said out loud, because it is a card worth reading.
    #[test]
    fn the_label_on_an_unexpected_partition_is_mounted_and_noted() {
        let mut k = kernel_with_block_devices(&[("vda2", uevent("vda2", 2, STATE_PARTLABEL))]);
        let chosen = resolve_device(&mut k);
        assert_eq!(
            chosen,
            Chosen::ByLabel {
                device: "/dev/vda2".to_string(),
                number: Some(2),
            }
        );
        report_choice(&mut k, &chosen);
        let console = k.lines.join("\n");
        assert!(console.contains("NOTE"), "{console}");
        assert!(
            console.contains(&rivet_core::state::STATE_PARTITION.to_string()),
            "{console}"
        );
    }

    /// The ENOENT that means two different repairs. A composed path missing is
    /// the wrong disk; a path the kernel itself just named being missing is
    /// devtmpfs.
    #[test]
    fn enoent_reads_differently_depending_on_how_the_device_was_chosen() {
        let mut k = a_card_behind_virtio();
        k.mount_results = Script::always(Err(Errno::ENOENT));
        let chosen = resolve_device(&mut k);
        let by_label = mount_state(&mut k, &chosen);
        assert!(
            matches!(&by_label, MountOutcome::Refused { detail, .. }
                if detail.contains("devtmpfs") && !detail.contains("did not lay out")),
            "{by_label:?}"
        );

        let mut k = kernel_with_no_mount_table();
        k.mount_results = Script::always(Err(Errno::ENOENT));
        let composed_outcome = mount_state(&mut k, &composed());
        assert!(
            matches!(&composed_outcome, MountOutcome::Refused { detail, .. }
                if detail.contains("no such device node") && !detail.contains("devtmpfs")),
            "{composed_outcome:?}"
        );
    }
}
