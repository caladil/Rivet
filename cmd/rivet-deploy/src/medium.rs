//! The adapter from this binary's [`Kernel`] to `rivet-adapter-image`'s
//! [`BootMedium`] port.
//!
//! The same shape as `cmd/rivet-initd/src/slots.rs` and `cmd/rivet/src/boot.rs`,
//! and it exists here for a reason those two record in advance.
//! `rivet-adapter-image`'s `port` module says:
//!
//! > **That is a decision with a known expiry.** `ADR-028` §4 says an
//! > authenticated principal supplies the authority to install an update and
//! > does not say which process executes it. If that turns out to be anything
//! > other than PID 1, this is where the question of a second implementor is
//! > answered.
//!
//! **The maintainer's 2026-08-24 ruling answered it: a new supervised
//! binary.** So the question falls due, and the answer taken is the one the
//! existing shape already implies — **`rivet-adapter-image` ships no
//! implementor, and this composition root supplies one over the syscall seam it
//! already owns.** That crate's manifest states the alternative and rejects it:
//! a `StdBootMedium` of its own would need `libc` for `mount(2)`, in a crate
//! whose whole posture is `unsafe_code = "forbid"` and no dependencies, and
//! would stand a second syscall authority beside the ones every binary already
//! has. `ADR-029` §12 item 4's second half is therefore answered: **no.**
//!
//! # What this adapter is used for, and what it is not
//!
//! It is used for the two operations that are the *firmware's*: which slot is
//! running, and arming a trial boot. It is **not** how the payload is written.
//!
//! That split is deliberate and is worth stating, because the obvious move is to
//! push everything through one port. [`BootMedium`] is *the medium the firmware
//! boots from, as `rivet-adapter-image` has to reach it* — a device-tree
//! property and one FAT file it rewrites. Its `read_file` returns a `String` and
//! its `write_file` takes a whole `&[u8]`; a 640 MiB member fits neither, and
//! widening the port would force `cmd/rivet-initd` — PID 1, which will never
//! install anything — to implement bulk transfer.
//!
//! So the payload's bytes go through [`Kernel`] directly, and this adapter
//! carries only what the boot port asks for. `AGENTS.md` §3.1's test, applied to
//! a port that already exists: the *adapter* is what is hiding a platform from
//! its own tests, and nothing about a 640 MiB stream is a firmware fact.

use rivet_adapter_image::{BootMedium, Errno as MediumErrno, Mounted};

use crate::kernel::{Errno, Kernel};

/// `rivet-adapter-image`'s [`BootMedium`] over this crate's [`Kernel`].
///
/// Holds a borrow rather than owning the kernel, so a caller can reborrow it for
/// the port call and keep it for the journal record afterwards — the pattern
/// `cmd/rivet-initd/src/slots.rs` and `cmd/rivet-sysd/src/clock.rs`
/// both use.
pub struct KernelMedium<'k> {
    k: &'k mut dyn Kernel,
    /// Failed unmounts, for the caller to put in the journal.
    ///
    /// The port gives the outcome of an unmount to the implementor because *it*
    /// has no caller who could act on one. This process has a journal, and a
    /// leaked mount outlives it — so the outcome is collected here rather than
    /// dropped, and [`Self::warnings`] is what the caller drains. Emitting from
    /// inside would need the journal borrowed at the same time as the kernel it
    /// writes through.
    warnings: Vec<String>,
}

impl<'k> KernelMedium<'k> {
    pub fn new(k: &'k mut dyn Kernel) -> Self {
        Self {
            k,
            warnings: Vec::new(),
        }
    }

    /// Take everything this adapter has to say, leaving it empty.
    pub fn warnings(&mut self) -> Vec<String> {
        core::mem::take(&mut self.warnings)
    }
}

/// The two error vocabularies are separate types on purpose; on the target they
/// are the same numbers, so the conversion is the number and nothing else.
///
/// The same conversion `cmd/rivet-initd`'s `to_medium_errno` and
/// `cmd/rivet-sysd`'s `to_store_errno` make.
const fn to_medium_errno(e: Errno) -> MediumErrno {
    MediumErrno(e.0)
}

impl BootMedium for KernelMedium<'_> {
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, MediumErrno> {
        // Bytes rather than text: a device-tree property may be a big-endian
        // cell, and deciding which is above this seam, in the port.
        self.k.read_bytes(path).map_err(to_medium_errno)
    }

    fn read_file(&mut self, path: &str) -> Result<String, MediumErrno> {
        self.k.read_file(path).map_err(to_medium_errno)
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), MediumErrno> {
        self.k
            .write_file_synced(path, contents)
            .map_err(to_medium_errno)
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
        self.k
            .mount(device, at, fstype, writable)
            .map_err(to_medium_errno)
    }

    fn unmount(&mut self, at: &str) {
        if let Err(e) = self.k.umount(at) {
            self.warnings.push(format!("could not unmount {at}: {e}"));
        }
    }

    /// Collected for the journal with the failed unmounts, for their reason.
    fn reclaimed_orphan(&mut self, at: &str) {
        self.warnings.push(format!(
            "reclaimed an orphaned mount at {at}: it was unmounted and the partition mounted \
             afresh, under the default-slot record's lock"
        ));
    }

    /// Waits up to [`LOCK_WAIT_MS`] for another holder, then refuses with
    /// `EAGAIN`: this binary is supervised, and an install that hung on a
    /// lock would be an install nobody could tell from a dead one.
    fn lock(&mut self, path: &str) -> Result<(), MediumErrno> {
        self.k
            .lock_within(path, LOCK_WAIT_MS)
            .map_err(to_medium_errno)
    }

    fn unlock(&mut self, path: &str) {
        self.k.unlock(path);
    }
}

/// How long this binary waits for another process to release
/// `rivet_adapter_image::AUTOBOOT_LOCK`.
///
/// **`cmd/rivet`'s bound, for `cmd/rivet`'s reason**: the longest a holder
/// keeps the lock is one commit or one arming — a mount, a small write, two
/// `sync(2)`s, a rename and an unmount of a FAT filesystem — and PID 1, the
/// holder most likely to be met, never waits on it itself.
pub const LOCK_WAIT_MS: u64 = 10_000;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_adapter_image::{
        AUTOBOOT_LOCK, AUTOBOOT_PARTITION_MOUNT, BOOT_FSTYPE, BootSlots as _, DT_BOOT_PARTITION,
        PiFirmware, SlotId, autoboot_partition_device,
    };

    /// Every errno the image adapter names must survive the conversion that
    /// reaches it. `cmd/rivet-initd` carries the same check against `libc`;
    /// this one is about the *conversion*, which that crate's cannot see.
    #[test]
    fn every_named_errno_survives_the_conversion() {
        let pairs = [
            (Errno::ENOENT, MediumErrno::ENOENT),
            (Errno::EIO, MediumErrno::EIO),
            (Errno::EAGAIN, MediumErrno::EAGAIN),
            (Errno::EBUSY, MediumErrno::EBUSY),
            (Errno::EINVAL, MediumErrno::EINVAL),
            (Errno::ENOSPC, MediumErrno::ENOSPC),
            (Errno::EROFS, MediumErrno::EROFS),
        ];
        for (mine, theirs) in pairs {
            assert_eq!(to_medium_errno(mine), theirs, "{mine} converts wrongly");
        }
    }

    /// A firmware property must reach `read_bytes` and not `read_file`: a
    /// device-tree property may be a big-endian cell, which is not text.
    #[test]
    fn a_firmware_property_is_read_as_bytes_rather_than_as_text() {
        let mut k = FakeKernel::new();
        k.bytes_mut()
            .insert("/p".to_string(), Script::always(Ok(vec![0, 0, 0, 2])));
        let mut m = KernelMedium::new(&mut k);
        assert_eq!(m.read_property("/p"), Ok(vec![0, 0, 0, 2]));
    }

    /// A boot-half write through the port must be **synced**, because on a FAT
    /// filesystem with no journal an unsynced `autoboot.txt` is a board that
    /// does not boot. The port's `write_file` says nothing about syncing; this
    /// adapter routes it to the call that does.
    #[test]
    fn a_write_through_the_port_is_the_synced_one() {
        let mut k = FakeKernel::new();
        {
            let mut m = KernelMedium::new(&mut k);
            assert_eq!(m.write_file("/run/x/f", b"hello"), Ok(()));
        }
        assert_eq!(
            k.synced_writes,
            vec![("/run/x/f".to_string(), b"hello".to_vec())]
        );
    }

    /// A failed unmount is collected for the journal rather than dropped: a
    /// leaked mount outlives this process and the next invocation converges
    /// onto it.
    #[test]
    fn a_failed_unmount_is_kept_for_the_journal_rather_than_swallowed() {
        let mut k = FakeKernel::new();
        k.umount_results = Script::always(Err(Errno::EBUSY));
        let mut m = KernelMedium::new(&mut k);
        m.unmount("/run/x");
        let said = m.warnings();
        assert_eq!(said.len(), 1);
        assert!(said[0].contains("/run/x"), "{said:?}");
        assert!(m.warnings().is_empty(), "the warnings were not drained");
    }

    /// The whole adapter, end to end: the port's arming, driven through this
    /// binary's own `Kernel`. `AGENTS.md` §4's individually-correct-but-
    /// jointly-wrong case — neither suite can make this join alone.
    #[test]
    fn an_arming_driven_through_this_binarys_kernel_rewrites_the_first_partitions_file() {
        let mut k = FakeKernel::new();
        k.bytes_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 1])),
        );
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok("[all]\ntryboot_a_b=1\nboot_partition=1\n\
                               [tryboot]\nboot_partition=1\n"
                .to_string())),
        );
        let armed = {
            let mut m = KernelMedium::new(&mut k);
            PiFirmware.arm_trial_boot(&mut m, SlotId(2))
        };
        assert!(armed.is_ok(), "{armed:?}");
        assert_eq!(
            k.mount_calls.first().map(|c| c.device.clone()),
            Some(autoboot_partition_device())
        );
        assert_eq!(
            k.mount_calls.first().map(|c| c.fstype.clone()),
            Some(BOOT_FSTYPE.to_string())
        );
        assert!(
            k.synced_writes.first().is_some_and(
                |(_, c)| String::from_utf8_lossy(c).contains("[tryboot]\nboot_partition=2")
            ),
            "{:?}",
            k.synced_writes
        );
        assert_eq!(k.umount_calls, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
        assert_eq!(
            k.lock_calls,
            vec![
                format!("lock {AUTOBOOT_LOCK} within {LOCK_WAIT_MS}"),
                format!("unlock {AUTOBOOT_LOCK}"),
            ],
            "the lock is taken with this binary's bound and released"
        );
    }

    /// **A mount found in place under the lock is reclaimed and the reclaim
    /// kept for the journal**, and a lock another process holds past the
    /// bound refuses without mounting.
    #[test]
    fn an_orphaned_mount_is_reclaimed_for_the_journal_and_a_busy_lock_mounts_nothing() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            format!("{AUTOBOOT_PARTITION_MOUNT}/autoboot.txt"),
            Script::always(Ok("[all]\nboot_partition=1\n".to_string())),
        );
        k.mount_results = Script::sequence(vec![Ok(Mounted::Adopted)], Ok(Mounted::Made));
        let (read, said) = {
            let mut m = KernelMedium::new(&mut k);
            let read = PiFirmware.default_slot(&mut m);
            (read, m.warnings())
        };
        assert_eq!(read, Ok(SlotId(1)));
        assert_eq!(k.mount_calls.len(), 2);
        assert_eq!(k.umount_calls.len(), 2, "the orphan, then the fresh mount");
        assert!(
            said.iter().any(|w| w.contains("reclaimed an orphaned mount")),
            "{said:?}"
        );

        let mut busy = FakeKernel::new();
        busy.lock_results = Script::always(Err(Errno::EAGAIN));
        let read = {
            let mut m = KernelMedium::new(&mut busy);
            PiFirmware.default_slot(&mut m)
        };
        assert!(read.as_ref().is_err_and(|why| why.contains(AUTOBOOT_LOCK)), "{read:?}");
        assert!(busy.mount_calls.is_empty());
        assert!(busy.umount_calls.is_empty());
    }
}
