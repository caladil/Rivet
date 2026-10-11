//! **The boot partition this boot did NOT come from** — `ADR-019` §3, applied
//! to the second copy every card carries.
//!
//! `cmd/rivet-image` writes the device identity and the staging bundle onto
//! BOTH boot partitions, because a card is provisioned rather than a slot.
//! [`crate::identity`] and [`crate::staging`] consume and blank the copy on the
//! partition the firmware booted. Without this module the other copy stays on
//! a FAT filesystem any card reader can read until that slot first boots,
//! which on a machine that never takes an update is never.
//!
//! # What it does
//!
//! It finds the other partition from the same device-tree reading
//! `rivet_adapter_image::boot_partition_device` makes, mounts it READ-ONLY at
//! its own mount point to look at one file, and remounts it writable only when
//! the caller has decided that file is to be destroyed. It never leaves the
//! partition mounted: every path out of [`tidy`] unmounts what it mounted.
//!
//! # What it does not decide
//!
//! **Whether a copy may go is the caller's**, because only the caller knows
//! what "durably on `/state`" means for its file: [`crate::identity`] removes a
//! seed only when `/state` holds the same bytes and a certificate beside them,
//! and [`crate::staging`] removes a bundle only when it is byte-identical to the
//! one this boot has just staged. Anything else is left in place and said: a
//! copy that is not proven to be on `/state` may be the only copy there is.

use rivet_adapter_image::{BOOT_DISK, BOOT_FSTYPE, boot_partition_device};
use rivet_core::state::{Slot, partition_device};

use crate::kernel::{Errno, Kernel};
use crate::provision::mount_boot_filesystem;
use crate::slots::KernelMedium;

/// Where the unbooted boot partition is mounted, transiently.
///
/// **Its own mount point**, for the reason `rivet_adapter_image`'s
/// `AUTOBOOT_PARTITION_MOUNT` gives: `mount_boot_filesystem` treats *already
/// mounted here* as convergence, so a mount point shared with the booted
/// partition's would hand back the wrong filesystem without failing.
pub const UNBOOTED_PARTITION_MOUNT: &str = "/run/unbooted-boot-partition";

/// What the caller decided about the copy it was shown.
pub enum Verdict {
    /// Proven to be on `/state`: blank it and unlink it.
    Remove,
    /// Not proven, so it stays; the sentence says why, for the console.
    Keep(String),
}

/// What became of one file on the unbooted partition.
#[derive(Debug, PartialEq, Eq)]
pub enum Tidied {
    /// There is no other partition to look at, or it would not mount. Under
    /// an emulator there is no boot partition at all (`ADR-026` §4).
    NoPartition(String),
    /// The file is not there: the ordinary case on every boot after the first.
    Absent,
    /// Blanked and unlinked.
    Removed,
    /// Left in place, for the caller's reason.
    Kept(String),
    /// It was to be removed and could not be. **The copy is still there.**
    Refused(String),
}

/// The device node of the boot partition the firmware did not boot.
///
/// # Errors
///
/// The device tree cannot be read, or names a partition that is neither slot's
/// boot partition — in which case which one is "the other" cannot be known and
/// nothing is guessed.
pub fn unbooted_partition_device(k: &mut dyn Kernel) -> Result<String, String> {
    let (booted, _) = {
        let mut m = KernelMedium::new(&mut *k);
        boot_partition_device(&mut m)?
    };
    let slot = Slot::from_boot_partition(booted).ok_or_else(|| {
        format!(
            "the firmware booted partition {booted}, which is neither slot's boot partition, so \
             which partition is the other one cannot be known"
        )
    })?;
    Ok(partition_device(BOOT_DISK, slot.other().boot_partition()))
}

/// Look at `file` on the unbooted boot partition, ask `decide` about its
/// bytes, and destroy it with `destroy` when the answer is [`Verdict::Remove`].
///
/// `destroy` is the caller's own blank-then-unlink, so each courier keeps the
/// procedure it already documents rather than depending on another's.
pub fn tidy(
    k: &mut dyn Kernel,
    file: &str,
    decide: impl FnOnce(&mut dyn Kernel, &[u8]) -> Verdict,
    destroy: fn(&mut dyn Kernel, &str, usize) -> Result<(), Errno>,
) -> Tidied {
    let device = match unbooted_partition_device(k) {
        Ok(device) => device,
        Err(why) => return Tidied::NoPartition(why),
    };
    if let Err(e) = mount_boot_filesystem(k, &device, UNBOOTED_PARTITION_MOUNT, BOOT_FSTYPE, false)
    {
        return Tidied::NoPartition(format!(
            "cannot mount {device} on {UNBOOTED_PARTITION_MOUNT}: {e}"
        ));
    }
    let path = format!("{UNBOOTED_PARTITION_MOUNT}/{file}");
    let bytes = match k.read_bytes(&path) {
        Ok(bytes) => bytes,
        Err(Errno::ENOENT) => {
            unmount(k);
            return Tidied::Absent;
        }
        Err(e) => {
            unmount(k);
            return Tidied::Refused(format!("{device}'s {file} could not be read: {e}"));
        }
    };
    let verdict = decide(k, &bytes);
    unmount(k);
    if let Verdict::Keep(why) = verdict {
        return Tidied::Kept(why);
    }

    // Writable only now, and only for as long as the destruction takes.
    if let Err(e) = mount_boot_filesystem(k, &device, UNBOOTED_PARTITION_MOUNT, BOOT_FSTYPE, true)
    {
        return Tidied::Refused(format!(
            "{device}'s {file} is to be removed and the partition would not mount writable: {e}"
        ));
    }
    let outcome = match destroy(k, &path, bytes.len()) {
        Ok(()) => Tidied::Removed,
        Err(e) => Tidied::Refused(format!("{device}'s {file} could not be removed: {e}")),
    };
    unmount(k);
    outcome
}

/// Unmount it again, reporting but not caring — `crate::provision`'s rule.
fn unmount(k: &mut dyn Kernel) {
    if let Err(e) = k.umount(UNBOOTED_PARTITION_MOUNT) {
        k.say(&format!("  could not unmount {UNBOOTED_PARTITION_MOUNT}: {e}"));
    }
}
