//! The seam between this crate's decisions and the machine.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! It is the same split `cmd/rivet-initd`'s `kernel` module,
//! `rivet-adapter-net`'s `port` module and `rivet-store`'s make, and here it
//! earns itself on one list: **`ADR-026` §4 puts `tryboot` and
//! `/proc/device-tree/chosen/bootloader` in the column QEMU does not emulate at
//! all.** Not "emulates badly" — does not have. So every decision in this crate
//! is reachable from a test only if it sits above a seam, and every one of them
//! does.
//!
//! | State | What produces it | What arrives here |
//! | :-- | :-- | :-- |
//! | no device tree at all | every emulator run, and any non-Pi machine | `ENOENT` from [`BootMedium::read_property`] |
//! | a boot partition that will not mount | a card written by another tool, a dying card | `ENODEV`, `EIO` |
//! | a card that has gone read-only | SD cards fail this way rather than by refusing | `EROFS` at the write or at the rename |
//! | an `autoboot.txt` nothing recognises | an operator edited it from Windows (`ADR-002` §4.1) | text this crate refuses rather than rewrites |
//!
//! # Named for what it needs, not for what implements it
//!
//! [`BootMedium`] is *the medium the firmware boots from*, as this crate has to
//! reach it: the firmware's own published properties, and one FAT filesystem it
//! can mount and rewrite a file on. It is deliberately **not** named for
//! `cmd/rivet-initd`'s `Kernel`, which is the only thing that implements it
//! today — a UEFI machine, behind the same boot-slot port, would implement
//! the same operations against EFI variables and an ESP, and would not be PID 1.
//!
//! # What is deliberately not on it
//!
//! * **No reboot.** [`crate::slots::BootSlots::arm_trial_boot`] returns
//!   [`crate::slots::ArmedReboot`] and stops. A port operation that stopped the
//!   machine could not be tested and could not be undone, and the reboot itself
//!   is a syscall like every other — it belongs on the caller's own seam, which
//!   on this hardware is `cmd/rivet-initd`'s `Kernel::reboot`.
//! * **No `say`.** `rivet-store`'s port records the same absence for the same
//!   reason: every decision here returns a typed value and the caller renders it
//!   (`ADR-006` §4, rendering is an adapter concern), so the tests assert on
//!   values rather than on prose a reword would break. The one exception is
//!   [`BootMedium::unmount`] — see its documentation.
//!
//! # This crate ships no implementor
//!
//! Unlike `rivet-store`, which ships `StdFilesystem`, there is no
//! `StdBootMedium` here. `mount(2)` has no `std` equivalent, so a real one would
//! mean taking `libc` in this crate and standing up a second syscall seam beside
//! the one PID 1 already owns — `ADR-011`'s second authority, over syscalls.
//! `cmd/rivet-initd/src/slots.rs` adapts its own `Kernel` instead, exactly as
//! `cmd/rivet-sysd/src/clock.rs` adapts it to
//! `rivet_store::Filesystem`.
//!
//! **That is a decision with a known expiry.** `ADR-028` §4 says an
//! authenticated principal supplies the authority to install an update and does
//! not say which process executes it. If that turns out to be anything other
//! than PID 1, this is where the question of a second implementor is answered.

/// A Linux error number, kept as data so a test can produce any of them.
///
/// **The constants are Linux's own**, from `asm-generic/errno-base.h`, for the
/// reason `rivet-adapter-net`'s and `rivet-store`'s ports both give: the
/// numbers a Rivet machine produces are a Linux kernel's, and the process
/// reasoning about them may have been compiled on Windows, where the platform's
/// table means something else.
///
/// **Nothing above this seam branches on the value**, which is the difference
/// from `rivet-store`, where `ENOSPC` and `EROFS` select different outcomes.
/// Here an errno is carried into a message and into the tests' vocabulary and
/// nowhere else, so a wrong constant could mislabel a console line and could not
/// change what the machine does. It is checked anyway:
/// `cmd/rivet-initd/src/slots.rs` carries
/// `every_named_errno_survives_the_conversion_with_the_platforms_own_number`,
/// which compares each of these against `libc`'s **and** against the conversion
/// that reaches them — a stronger check than this crate could make alone, and
/// one that needs no dependency here (`AGENTS.md` §4: a suite that cannot be run
/// rots, so the check runs where it can).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub const EPERM: Self = Self(1);
    /// No such file or directory. **The ordinary state under an emulator**:
    /// `ADR-026` §4 says there is no `/proc/device-tree/chosen/bootloader` at
    /// all, so this is what every firmware read answers there.
    pub const ENOENT: Self = Self(2);
    pub const EIO: Self = Self(5);
    /// The advisory lock on the default-slot record is held by another process
    /// (`EWOULDBLOCK` is the same number on Linux). What [`BootMedium::lock`]
    /// answers when it will not wait, or has waited as long as it may.
    pub const EAGAIN: Self = Self(11);
    pub const EACCES: Self = Self(13);
    /// Already mounted at this point. Handled **below** this seam, because it is
    /// the convergent answer rather than a failure — see
    /// [`BootMedium::mount_boot_filesystem`], and see
    /// [`crate::firmware::AUTOBOOT_PARTITION_MOUNT`] for why treating it that
    /// way is only safe when each caller has its own mount point.
    pub const EBUSY: Self = Self(16);
    pub const ENODEV: Self = Self(19);
    pub const EINVAL: Self = Self(22);
    pub const ENOSPC: Self = Self(28);
    /// Read-only filesystem. **How an SD card usually announces that it is
    /// dying**: the kernel remounts it read-only rather than refusing, so this
    /// arrives at a write or at a rename on a machine that was healthy a moment
    /// earlier.
    pub const EROFS: Self = Self(30);
    /// The operation is not offered. What [`BootMedium::list_dir`] answers on an
    /// implementor that does not supply it — a refusal, never an empty listing
    /// that would read as an empty directory.
    pub const ENOSYS: Self = Self(38);

    /// The name and meaning, for the errnos this crate can receive.
    ///
    /// `None` for anything else, so the caller prints the number rather than a
    /// guess (`AGENTS.md` §5 — a number is what the system said).
    pub fn described(self) -> Option<&'static str> {
        Some(match self {
            Self::EPERM => "EPERM: not permitted",
            Self::ENOENT => "ENOENT: no such file or directory",
            Self::EIO => "EIO: the device returned an I/O error",
            Self::EAGAIN => "EAGAIN: another process holds it",
            Self::EACCES => "EACCES: permission denied",
            Self::EBUSY => "EBUSY: it is busy",
            Self::ENODEV => "ENODEV: no such device",
            Self::EINVAL => "EINVAL: invalid argument",
            Self::ENOSPC => "ENOSPC: no space left on the device",
            Self::EROFS => "EROFS: read-only filesystem",
            Self::ENOSYS => "ENOSYS: this medium does not offer that operation",
            _ => return None,
        })
    }
}

impl core::fmt::Display for Errno {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.described() {
            Some(text) => write!(f, "{text} (errno {})", self.0),
            None if self.0 == 0 => write!(f, "an error with no OS error number"),
            None => write!(f, "errno {}", self.0),
        }
    }
}

/// One entry of a directory listing, as the filesystem reported it.
///
/// **The size is a `Result` per entry**, because a listing taken to diagnose a
/// failure must still say which names were there when one of them cannot be
/// stat'd — dropping that entry would hide exactly the file being looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub size: Result<u64, Errno>,
}

/// A bounded directory listing: the first entries in name order, and how many
/// the directory held in all.
///
/// `total` is carried so a truncated listing says it was truncated, rather than
/// reading as a complete directory with fewer files in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<DirEntry>,
    pub total: usize,
}

/// What a [`BootMedium::mount_boot_filesystem`] that succeeded actually did.
///
/// **A caller that does not hold a lock excluding every other user of the
/// mount point unmounts only a mount it made.** A mount it found already in
/// place belongs to whoever made it, and that owner may be in the middle of
/// using it: unmounting it from under a writer between its write and its
/// rename sends the rename into the bare mount-point directory, where it fails
/// with `ENOENT`, and the owner's own unmount then fails with `EINVAL`.
///
/// **A caller holding such a lock reclaims instead**, because a mount it finds
/// can belong to nobody still using it —
/// [`crate::firmware::mount_autoboot_partition`] is that case, under
/// [`crate::firmware::AUTOBOOT_LOCK`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Mounted {
    /// This call mounted the filesystem, and its caller unmounts it.
    Made,
    /// Something was already mounted at the mount point. A caller without the
    /// lock reads or writes through it and leaves it mounted; a caller holding
    /// it reclaims it.
    Adopted,
}

/// Everything this crate asks of the medium the firmware boots from, and
/// nothing else.
///
/// Object-safe on purpose, for the same reason `cmd/rivet-initd`'s `Kernel` is:
/// the decisions above it are compiled once, and the tests drive exactly the
/// code a board runs.
pub trait BootMedium {
    /// Read a property the firmware publishes, as bytes.
    ///
    /// **Bytes, not text.** A device-tree property may be a NUL-terminated
    /// string or a big-endian cell, and deciding which is a decision — so it
    /// happens above this seam, in [`crate::firmware::parse_dt_u32`], where a
    /// test can drive it.
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, Errno>;

    /// Read a whole text file from a mounted filesystem.
    ///
    /// Text rather than bytes, because the one file this crate reads is
    /// `autoboot.txt` and it is `config.txt` syntax. Its line endings are
    /// preserved byte-for-byte by the rewrite above (`ADR-002` §4.1 — the
    /// operator edits it from Windows), which is a property of the rewrite and
    /// not of this call.
    fn read_file(&mut self, path: &str) -> Result<String, Errno>;

    /// Create or truncate `path` and write every byte.
    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno>;

    /// Rename within one filesystem — atomic with respect to a concurrent
    /// reader and to a crash.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno>;

    /// Flush filesystem buffers.
    ///
    /// **Cannot fail**, because `sync(2)` cannot. A Pi has no battery and
    /// usually an SD card, so the ordering around this call is the whole of what
    /// stops a power cut leaving an `autoboot.txt` that will not boot.
    fn sync(&mut self);

    /// Mount the filesystem on `device` at `at`, creating the mount point.
    ///
    /// **Recognising that something is already mounted at `at` is the
    /// implementor's job**, and it answers [`Mounted::Adopted`] rather than
    /// failing — what the caller then does is [`Mounted`]'s to say. A mount
    /// this call made answers [`Mounted::Made`]. Reading
    /// through a mount at `at` is safe only because each caller has its own
    /// mount point — [`crate::firmware::AUTOBOOT_PARTITION_MOUNT`] carries what
    /// sharing one cost.
    ///
    /// `fstype` is named by the caller rather than guessed at, because mounting
    /// a filesystem as the wrong type can *succeed*: this crate's is
    /// [`crate::firmware::BOOT_FSTYPE`].
    ///
    /// **The mount flags are the implementor's**, deliberately. They are a
    /// platform's own bits, and on this hardware they carry a policy that is
    /// PID 1's rather than this crate's — a FAT filesystem an operator edits
    /// from a Windows machine is mounted `noexec`, `nosuid` and `nodev`.
    fn mount_boot_filesystem(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno>;

    /// Unmount `at` again. Called only for a mount [`Mounted::Made`] by the
    /// same caller.
    ///
    /// **Returns nothing, and that is the one place this port differs from
    /// `rivet-store`'s.** Every caller here unmounts on the way out of an
    /// operation that has already succeeded or already failed, and there is
    /// nothing a caller could do with the answer — so rather than have each of
    /// them discard a `Result`, the implementor owns the outcome. On this
    /// hardware that means PID 1 says so on the console, which is the only
    /// diagnostic a board gives (`AGENTS.md` §5).
    fn unmount(&mut self, at: &str);

    /// Report that an orphaned mount at `at` was unmounted and the filesystem
    /// mounted afresh — [`crate::firmware::mount_autoboot_partition`]'s
    /// reclaim.
    ///
    /// **The implementor owns the report, for [`Self::unmount`]'s reason**: no
    /// caller could act on it, and where it goes is the platform's. PID 1 says
    /// it on the console; a command puts it in the journal. It is worth a
    /// record because an orphaned mount means some process died, or failed to
    /// unmount, while it held the partition.
    fn reclaimed_orphan(&mut self, at: &str);

    /// Take the exclusive advisory lock at `path`, creating the file (mode
    /// `0600`) and its directory if they are not there.
    ///
    /// **Every process that mounts the partition holding the default-slot
    /// record holds this lock from before its mount until after its unmount**
    /// ([`crate::firmware::AUTOBOOT_LOCK`]), so no process can mount, adopt or
    /// unmount that partition while another is between its own mount and
    /// unmount. Exclusive for readers as well as writers: two readers share one
    /// mount point, and the one that made the mount would unmount it from under
    /// the one that adopted it.
    ///
    /// **How long to wait is the implementor's**, because it depends on who the
    /// caller is. PID 1 does not wait at all — its loop must never block — and
    /// answers [`Errno::EAGAIN`] at once; a command waits a bounded time and
    /// then answers `EAGAIN`. Either way `EAGAIN` means *another process holds
    /// it*, and nothing was mounted.
    fn lock(&mut self, path: &str) -> Result<(), Errno>;

    /// Release the lock [`Self::lock`] took. Cannot fail: closing the
    /// descriptor releases an advisory lock, and a process that exits releases
    /// it too.
    fn unlock(&mut self, path: &str);

    /// List a directory of a mounted filesystem: at most `limit` entries, in
    /// name order, each with its size.
    ///
    /// **Diagnostic only.** Nothing in this crate decides anything from the
    /// answer; it exists so a failed write to the default-slot record can say
    /// what was actually on the partition (see
    /// [`crate::slots::BootSlots::record_medium_listing`]).
    ///
    /// **The default refuses with [`Errno::ENOSYS`]** rather than answering an
    /// empty listing, which would read as an empty directory. Only PID 1 commits
    /// a boot, so only its implementor supplies this; the others inherit the
    /// refusal.
    fn list_dir(&mut self, path: &str, limit: usize) -> Result<Listing, Errno> {
        let _ = (path, limit);
        Err(Errno::ENOSYS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_errno_is_named_and_an_unknown_one_is_a_number() {
        assert!(Errno::EROFS.to_string().contains("EROFS"));
        assert!(Errno::EROFS.to_string().contains("30"));
        assert_eq!(Errno(4242).to_string(), "errno 4242");
        assert!(Errno(0).to_string().contains("no OS error number"));
    }

    /// Two errnos sharing a number would make one of them unreportable, and the
    /// console line is the only diagnostic a board gives.
    #[test]
    fn every_named_errno_is_a_distinct_number() {
        let all = [
            Errno::EPERM,
            Errno::ENOENT,
            Errno::EIO,
            Errno::EAGAIN,
            Errno::EACCES,
            Errno::EBUSY,
            Errno::ENODEV,
            Errno::EINVAL,
            Errno::ENOSPC,
            Errno::EROFS,
            Errno::ENOSYS,
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                assert!(i == j || a != b, "{a} and {b} share a number");
            }
            assert!(a.described().is_some(), "{a} has no description");
        }
    }

    /// An implementor that does not supply a listing refuses rather than
    /// answering an empty directory.
    #[test]
    fn a_medium_that_does_not_list_refuses_rather_than_answering_empty() {
        struct Minimal;
        impl BootMedium for Minimal {
            fn read_property(&mut self, _: &str) -> Result<Vec<u8>, Errno> {
                Err(Errno::ENOENT)
            }
            fn read_file(&mut self, _: &str) -> Result<String, Errno> {
                Err(Errno::ENOENT)
            }
            fn write_file(&mut self, _: &str, _: &[u8]) -> Result<(), Errno> {
                Err(Errno::EROFS)
            }
            fn rename(&mut self, _: &str, _: &str) -> Result<(), Errno> {
                Err(Errno::EROFS)
            }
            fn sync(&mut self) {}
            fn mount_boot_filesystem(
                &mut self,
                _: &str,
                _: &str,
                _: &str,
                _: bool,
            ) -> Result<Mounted, Errno> {
                Err(Errno::ENODEV)
            }
            fn unmount(&mut self, _: &str) {}
            fn reclaimed_orphan(&mut self, _: &str) {}
            fn lock(&mut self, _: &str) -> Result<(), Errno> {
                Err(Errno::EAGAIN)
            }
            fn unlock(&mut self, _: &str) {}
        }
        assert_eq!(Minimal.list_dir("/", 64), Err(Errno::ENOSYS));
    }
}
