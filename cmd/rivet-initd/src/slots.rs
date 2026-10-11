//! The adapter from PID 1's [`Kernel`] to `rivet-adapter-image`'s
//! [`BootMedium`] port.
//!
//! **This module used to be the port itself.** `BootSlots`, `PiFirmware`, the
//! `autoboot.txt` rewriting and their tests moved to `rivet-adapter-image` on
//! 2026-08-23, with no change of behaviour, because **a binary crate has no lib
//! target**: nothing outside PID 1 could name the trait, and `ADR-028` §4 rules
//! who *authorises* an update without saying which process *executes* it. What
//! is left here is the wiring, and `cmd/rivet-initd` stays the composition
//! root.
//!
//! It is the same shape as `cmd/rivet-sysd/src/clock.rs`'s `StateFs`,
//! which adapts that crate's own `Kernel` to `rivet_store::Filesystem`, and it
//! is here for the same reason: going through the port PID 1 already owns keeps
//! every branch reachable from a test. The states this code has to be right
//! about — no device tree at all, a boot partition that will not mount, a card
//! that has gone read-only mid-write — are states `ADR-026` §4 says the emulator
//! cannot produce and a board should not be asked to.
//!
//! # What sits on which side of the seam
//!
//! | Where | What |
//! | :-- | :-- |
//! | `rivet_adapter_image` | **the decisions** — which slot, which file, which line of it, and in what order it is written |
//! | [`crate::kernel`] | **the syscalls** — `mount(2)`, `rename(2)`, `sync(2)`, and the `MS_*` bits |
//! | here | the conversion between the two vocabularies, and nothing else |
//!
//! Two of those conversions are worth stating rather than reading off:
//!
//! * **The mount flags stay on this side.** `ADR-014` §3 puts the `MS_*`
//!   translation in this crate, and the policy that goes with it — a FAT
//!   filesystem an operator edits from a Windows machine is mounted `noexec`,
//!   `nosuid` and `nodev` (`ADR-002` §4.1) — is PID 1's rather than the image
//!   adapter's. So is treating "already mounted here" as the convergent answer;
//!   `crate::provision::mount_boot_filesystem` owns both, and both partitions
//!   reach `mount(2)` through it.
//! * **The reboot stays on this side too**, which is item 5 of the port's own
//!   design. See [`reboot_for`].

use rivet_adapter_image::{
    ArmedReboot, BootMedium, DirEntry, Errno as MediumErrno, Listing, Mounted,
};

use crate::kernel::{Errno, Kernel, RebootHow};
use crate::provision::mount_boot_filesystem;

/// `rivet-adapter-image`'s [`BootMedium`] port over this crate's [`Kernel`].
///
/// Holds a borrow rather than owning the kernel, so a caller can reborrow it for
/// the port call and keep it for the console line afterwards — the pattern
/// `cmd/rivet-sysd/src/clock.rs` uses, and the reason both stay
/// borrowing adapters rather than becoming a second `Kernel`.
pub struct KernelMedium<'k> {
    k: &'k mut dyn Kernel,
}

impl<'k> KernelMedium<'k> {
    pub fn new(k: &'k mut dyn Kernel) -> Self {
        Self { k }
    }
}

/// The two error vocabularies are separate types on purpose; on the target they
/// are the same numbers, so the conversion is the number and nothing else.
///
/// The same conversion `cmd/rivet-sysd`'s `to_store_errno` makes, and it
/// is where `rivet_adapter_image::Errno`'s constants are actually checked —
/// see the tests below.
const fn to_medium_errno(e: Errno) -> MediumErrno {
    MediumErrno(e.0)
}

impl BootMedium for KernelMedium<'_> {
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, MediumErrno> {
        self.k.read_bytes(path).map_err(to_medium_errno)
    }

    fn read_file(&mut self, path: &str) -> Result<String, MediumErrno> {
        self.k.read_file(path).map_err(to_medium_errno)
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), MediumErrno> {
        self.k.write_file(path, contents).map_err(to_medium_errno)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), MediumErrno> {
        self.k.rename(from, to).map_err(to_medium_errno)
    }

    fn sync(&mut self) {
        self.k.sync();
    }

    fn mount_boot_filesystem(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, MediumErrno> {
        // Through `crate::provision` rather than straight to `Kernel::mount`, so
        // that the flag policy and the convergent "already mounted here" are
        // stated once for both partitions this crate mounts. It hands back the
        // raw errno rather than prose, because the caller above the port is what
        // words a failure.
        mount_boot_filesystem(self.k, device, at, fstype, writable).map_err(to_medium_errno)
    }

    /// **Never waits.** PID 1's loop reaps children and takes signals, and a
    /// lock another process holds would stall both; `Kernel::try_lock` answers
    /// `EAGAIN` at once and the commit's retry schedules the next attempt.
    fn lock(&mut self, path: &str) -> Result<(), MediumErrno> {
        self.k.try_lock(path).map_err(to_medium_errno)
    }

    fn unlock(&mut self, path: &str) {
        self.k.unlock(path);
    }

    /// Said on the console, `Self::unmount`'s channel: it means some process
    /// died, or failed to unmount, while it held the partition.
    fn reclaimed_orphan(&mut self, at: &str) {
        self.k.say(&format!(
            "rivet-initd: reclaimed an orphaned mount at {at}: it was unmounted and the \
             partition mounted afresh, under the default-slot record's lock"
        ));
    }

    fn unmount(&mut self, at: &str) {
        // The outcome is reported here rather than returned, because the port
        // has no caller that could act on it and PID 1 has nowhere else to put
        // it — the console is a board's only diagnostic (`AGENTS.md` §5). This
        // is `crate::provision::umount_boot_partition`'s posture, with the mount
        // point as an argument.
        if let Err(e) = self.k.umount(at) {
            self.k.say(&format!("  could not unmount {at}: {e}"));
        }
    }

    /// Every name is read, sorted so two listings of one directory compare
    /// line for line, and only the first `limit` are stat'd — the bound is on
    /// the work and the console lines, not only on what is returned.
    fn list_dir(&mut self, path: &str, limit: usize) -> Result<Listing, MediumErrno> {
        let mut names = self.k.read_dir(path).map_err(to_medium_errno)?;
        names.sort();
        let total = names.len();
        names.truncate(limit);
        let entries = names
            .into_iter()
            .map(|name| {
                let size = self
                    .k
                    .file_size(&format!("{}/{name}", path.trim_end_matches('/')))
                    .map_err(to_medium_errno);
                DirEntry { name, size }
            })
            .collect();
        Ok(Listing { entries, total })
    }
}

/// Which `reboot(2)` carries an arming.
///
/// **The mapping exists here rather than the type moving**, and the choice is
/// `rivet_adapter_image::ArmedReboot`'s to explain: [`RebootHow`] also carries
/// `PowerOff` and `Restart`, which are PID 1's shutdown handling (`ADR-017`
/// §3.1.5 item 6) and have nothing to do with images, and an enum cannot be
/// split.
///
/// **Nothing calls this**, and the reason has now changed twice.
///
/// ~~For the same reason nothing calls `BootSlots::arm_trial_boot`: the caller
/// is the update path, which is deliberately not built.~~ **`cmd/rivet` arms a
/// trial boot** since 2026-08-23 (`ADR-018` §5.3.1). ~~What is still true is
/// that nothing performs the *restart*: `ADR-017` §3.1.5 item 6 gives shutdown
/// to PID 1, PID 1 restarts only on `SIGINT`, and no binary in the image can
/// signal it.~~ **Also stale, later the same day**: `ADR-017` §3.1.10 gives PID 1
/// a third signal, and [`crate::pid1`] performs [`RebootHow::TrialRestart`] on
/// `SIGUSR1` — the maintainer's ruling, option 2 of three weighed.
///
/// **What is left is narrower and is the actual reason for the allow.** PID 1
/// names `RebootHow::TrialRestart` *directly*, because a `SIGUSR1` is a request
/// for the one-shot and carries no `ArmedReboot` to convert. This function is
/// the conversion from the image port's vocabulary, and its caller is the
/// **update path** — the thing that would arm and reboot in one operation —
/// which is still deliberately not built (`ADR-027` §9 item 1). It is named so a
/// test can drive that conversion, in the shape `crate::kernel::Errno::ENOSPC`
/// already uses in this crate. **Delete the allow when an update path arms and
/// reboots.**
#[allow(dead_code)]
pub const fn reboot_for(armed: ArmedReboot) -> RebootHow {
    match armed {
        // The one-shot is already recorded where the firmware will read it, so
        // an ordinary restart is what picks it up. Nothing produces this today —
        // it is UEFI `BootNext`'s answer.
        ArmedReboot::Ordinary => RebootHow::Restart,
        // The one-shot lives in the reboot call itself. `ADR-018` §5.1's
        // `reboot "0 tryboot"`, which this crate makes as `reboot(2)` with
        // `LINUX_REBOOT_CMD_RESTART2` — see `crate::kernel::TRYBOOT_ARGUMENT`.
        ArmedReboot::OneShot => RebootHow::TrialRestart,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_adapter_image::{
        AUTOBOOT_PARTITION_MOUNT, BOOT_FSTYPE, BootSlots as _, PiFirmware, SlotId,
        autoboot_partition_device,
    };

    /// Every errno `rivet-adapter-image` names must be the number this
    /// platform uses **and** must survive the conversion that reaches it.
    ///
    /// That crate declares Linux's numbers by hand and takes no `libc`, so this
    /// is the check that stops them going stale — and it is stronger than one
    /// inside that crate would be, because it exercises [`to_medium_errno`] as
    /// well as the constants. `AGENTS.md` §4: a suite that cannot be run rots,
    /// so the check runs where it can.
    #[test]
    fn every_named_errno_survives_the_conversion_with_the_platforms_own_number() {
        let pairs = [
            (Errno::EPERM, MediumErrno::EPERM, libc::EPERM),
            (Errno::ENOENT, MediumErrno::ENOENT, libc::ENOENT),
            (Errno::EIO, MediumErrno::EIO, libc::EIO),
            (Errno::EAGAIN, MediumErrno::EAGAIN, libc::EAGAIN),
            (Errno::EAGAIN, MediumErrno::EAGAIN, libc::EWOULDBLOCK),
            (Errno::EACCES, MediumErrno::EACCES, libc::EACCES),
            (Errno::EBUSY, MediumErrno::EBUSY, libc::EBUSY),
            (Errno::ENODEV, MediumErrno::ENODEV, libc::ENODEV),
            (Errno::EINVAL, MediumErrno::EINVAL, libc::EINVAL),
            (Errno::ENOSPC, MediumErrno::ENOSPC, libc::ENOSPC),
            (Errno::EROFS, MediumErrno::EROFS, libc::EROFS),
            (Errno(libc::ENOSYS), MediumErrno::ENOSYS, libc::ENOSYS),
        ];
        for (kernel, medium, platform) in pairs {
            assert_eq!(kernel.0, platform, "{kernel} is not this platform's number");
            assert_eq!(
                to_medium_errno(kernel),
                medium,
                "{kernel} converts to the wrong member of the image port's vocabulary"
            );
        }
    }

    /// The port's read of a firmware property must reach `Kernel::read_bytes`
    /// and not `read_file`: a device-tree property may be a big-endian cell,
    /// which is not text.
    #[test]
    fn a_firmware_property_is_read_as_bytes_rather_than_as_text() {
        let mut k = FakeKernel::new();
        k.blobs_mut()
            .insert("/p".to_string(), Script::always(Ok(vec![0, 0, 0, 2])));
        let mut m = KernelMedium::new(&mut k);
        assert_eq!(m.read_property("/p"), Ok(vec![0, 0, 0, 2]));
    }

    /// The mount reaches `mount(2)` with the flags `crate::provision` chooses,
    /// which is what keeps that policy on this side of the seam.
    #[test]
    fn a_mount_through_the_port_carries_pid_ones_own_flag_policy() {
        let mut k = FakeKernel::new();
        {
            let mut m = KernelMedium::new(&mut k);
            assert_eq!(
                m.mount_boot_filesystem("/dev/mmcblk0p1", "/run/x", BOOT_FSTYPE, true),
                Ok(Mounted::Made)
            );
        }
        let call = k.mount_calls.first();
        assert!(call.is_some_and(|c| c.source == "/dev/mmcblk0p1"
            && c.target == "/run/x"
            && c.fstype == BOOT_FSTYPE
            && !c.flags.contains(crate::kernel::MountFlags::RDONLY)
            && c.flags.contains(crate::kernel::MountFlags::NOEXEC)
            && c.flags.contains(crate::kernel::MountFlags::NOSUID)
            && c.flags.contains(crate::kernel::MountFlags::NODEV)));
    }

    /// A failed unmount is said on the console and not returned, because the
    /// port has no caller that could act on it.
    #[test]
    fn an_unmount_that_fails_is_reported_on_the_console_rather_than_swallowed() {
        let mut k = FakeKernel::new();
        k.umount_results = Script::always(Err(Errno::EBUSY));
        {
            let mut m = KernelMedium::new(&mut k);
            m.unmount("/run/x");
        }
        assert_eq!(k.umount_calls, vec!["/run/x".to_string()]);
        assert!(
            k.lines
                .iter()
                .any(|l| l.contains("could not unmount /run/x")),
            "{:?}",
            k.lines
        );
    }

    /// The listing reaches `read_dir` and `file_size`, sorts the names, stats
    /// only the first `limit`, and keeps an entry whose size could not be read
    /// rather than dropping it.
    #[test]
    fn a_listing_through_the_port_is_sorted_bounded_and_keeps_unstatable_names() {
        let mut k = FakeKernel::new();
        k.dirs_mut().insert(
            "/run/x".to_string(),
            Script::always(Ok(vec![
                "c".to_string(),
                "a".to_string(),
                "b".to_string(),
            ])),
        );
        k.file_sizes_mut()
            .insert("/run/x/a".to_string(), Script::always(Ok(10)));
        let got = {
            let mut m = KernelMedium::new(&mut k);
            m.list_dir("/run/x", 2)
        };
        assert_eq!(
            got,
            Ok(Listing {
                entries: vec![
                    DirEntry {
                        name: "a".to_string(),
                        size: Ok(10)
                    },
                    DirEntry {
                        name: "b".to_string(),
                        size: Err(MediumErrno::ENOENT)
                    },
                ],
                total: 3,
            })
        );
    }

    #[test]
    fn a_directory_that_cannot_be_read_is_refused_with_what_the_kernel_said() {
        let mut k = FakeKernel::new();
        let mut m = KernelMedium::new(&mut k);
        assert_eq!(m.list_dir("/nowhere", 64), Err(MediumErrno::ENOENT));
    }

    /// The whole adapter, end to end: the port's commit, driven through PID 1's
    /// own `Kernel` rather than through the image crate's fake. This is the
    /// join the two suites cannot make separately — `AGENTS.md` §4's
    /// individually-correct-but-jointly-wrong case.
    #[test]
    fn a_commit_driven_through_pid_ones_kernel_writes_the_first_partitions_file() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok(
                "[all]\nboot_partition=1\n[tryboot]\nboot_partition=2\n".to_string()
            )),
        );
        let committed = {
            let mut m = KernelMedium::new(&mut k);
            PiFirmware.commit(&mut m, SlotId(2))
        };
        assert!(committed.is_ok(), "{committed:?}");
        assert_eq!(
            k.mount_calls.first().map(|c| c.source.clone()),
            Some(autoboot_partition_device())
        );
        assert!(
            k.write_calls
                .first()
                .is_some_and(|(_, c)| String::from_utf8_lossy(c).contains("boot_partition=2"))
        );
        assert_eq!(k.umount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
    }

    /// The mapping item 5 chose. The trial restart must not become an ordinary
    /// one: that would reboot the board, look entirely healthy, and trial
    /// nothing.
    #[test]
    fn the_one_shot_arming_maps_onto_the_reboot_that_carries_it() {
        assert_eq!(reboot_for(ArmedReboot::OneShot), RebootHow::TrialRestart);
        assert_eq!(reboot_for(ArmedReboot::Ordinary), RebootHow::Restart);
        assert_ne!(
            reboot_for(ArmedReboot::OneShot),
            reboot_for(ArmedReboot::Ordinary)
        );
    }
}
