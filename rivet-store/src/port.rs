//! The seam between this crate's decisions and a filesystem.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! It is the same split `cmd/rivet-initd`'s `kernel` module and
//! `rivet-adapter-net`'s `port` module make, and for the same reason: the
//! branches that matter here cannot be reached from a test on a developer's
//! machine, and several of them cannot be reached from a *board* either without
//! deliberately damaging it.
//!
//! The list is not hypothetical. Every one of these is a state a Rivet machine
//! reaches on its own:
//!
//! | State | What produces it | What arrives here |
//! | :-- | :-- | :-- |
//! | `/state` did not mount | a card written by another tool, an unformatted `p5`, a dying card (`cmd/rivet-initd`'s `state` module) | `EROFS` — the mount point is a directory on the read-only root (`ADR-018` §3) |
//! | `/state` is full | `ADR-018` §7's one writable partition, no reserved-space guarantee implemented yet (§4) | `ENOSPC`, and on `ext4` **at `fsync` rather than at `write`** — see [`Filesystem::sync_file`] |
//! | nothing is stored yet | every first boot | `ENOENT` |
//! | the stored document is unreadable | `ADR-018` §8, a damaged `/state` | `EIO`, or bytes that are not UTF-8, or text that is not a valid document |
//! | a previous save was interrupted | the board loses power without warning, which is the normal way a Pi is switched off | a leftover temp file — see [`crate::store::ConfigStore::save`] |
//!
//! **`StdFilesystem` must stay thin, and should get thinner rather than
//! smarter.** Whatever logic lives in it is logic no test can drive. Its job is
//! to make one `std::fs` call, translate the result into an [`Errno`], and stop.
//!
//! # Two deliberate differences from `cmd/rivet-initd`'s port
//!
//! Recorded because they look like inconsistency and are not.
//!
//! 1. **This port has no `say`.** PID 1's `Kernel` reports to the console
//!    through the port; here every decision returns a typed value
//!    ([`crate::store::StoreError`], [`crate::store::Seeding`]) and the caller
//!    renders it — `ADR-006` §4, rendering is an adapter concern. Tests then
//!    assert on values rather than on prose a reword would break.
//! 2. **The write is four calls, not one.** A single `write_atomically` would be
//!    a smaller port and would put the whole crash-safety sequence below the
//!    seam, where no test can drive its middle. The sequence is the subject of
//!    this crate; see [`crate::store::ConfigStore::save`] for what each step
//!    guarantees and what it does not.
//!
//! # Why `StdFilesystem` is `cfg(unix)`
//!
//! `sync_dir` has no Windows equivalent: `FlushFileBuffers` takes a file handle
//! and a directory is not one. An implementation that returned `Ok(())` there
//! would report a durability barrier that had not happened, which is
//! `AGENTS.md` §3's rule exactly — *stubs refuse; they never fake success*, and
//! a faked `fsync` is the most expensive possible instance of it, because the
//! lie is only discovered by a power cut.
//!
//! So the real adapter compiles on Unix and nowhere else, exactly as
//! `rivet-adapter-net`'s socket compiles on Linux and nowhere else. Every
//! *decision* in this crate is above the port and its tests run on the Windows
//! development host against `crate::testfs::FakeFs`; `StdFilesystem`'s own
//! tests run under WSL and on the target.

/// A Linux error number, kept as data so a test can produce any of them.
///
/// **The constants are Linux's own**, from `asm-generic/errno-base.h`, for the
/// reason `rivet-adapter-net`'s port gives: the numbers a Rivet machine
/// produces are a Linux kernel's, and the process reasoning about them may have
/// been compiled on Windows, where the platform's table means something else.
/// `linux_errno_constants_match_the_platforms_own` checks them against `libc` in
/// a test that compiles on Linux alone.
///
/// [`Display`](core::fmt::Display) names the ones this crate can actually
/// receive and prints a bare number for anything else, so an unfamiliar failure
/// is reported rather than mislabelled (`AGENTS.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub const EPERM: Self = Self(1);
    /// No such file or directory. **The normal state of a first boot**, and not
    /// an error — see [`crate::store::ConfigStore::load`].
    pub const ENOENT: Self = Self(2);
    pub const EIO: Self = Self(5);
    pub const EACCES: Self = Self(13);
    pub const EEXIST: Self = Self(17);
    pub const ENOTDIR: Self = Self(20);
    pub const EINVAL: Self = Self(22);
    /// The filesystem is full. `ADR-018` §7's hazard, on the one partition that
    /// takes every write a Rivet machine makes.
    pub const ENOSPC: Self = Self(28);
    /// Read-only filesystem. **What an unmounted `/state` looks like**: the
    /// mount point is a directory on the `erofs`/`ext4`-`ro` root (`ADR-018`
    /// §3), so a write to it is refused by the root rather than by `/state`.
    pub const EROFS: Self = Self(30);
    /// The disk quota is exhausted. Not reachable on a Rivet machine today —
    /// there are no quotas — and named because `ext4` can produce it and a bare
    /// number would send an operator looking in the wrong place.
    pub const EDQUOT: Self = Self(122);

    /// The name and meaning, for the errnos this crate can receive.
    ///
    /// `None` for anything else, so the caller prints the number rather than a
    /// guess (`AGENTS.md` §5 — a number is what the system said).
    pub fn described(self) -> Option<&'static str> {
        Some(match self {
            Self::EPERM => "EPERM: not permitted",
            Self::ENOENT => "ENOENT: no such file or directory",
            Self::EIO => "EIO: the device returned an I/O error",
            Self::EACCES => "EACCES: permission denied",
            Self::EEXIST => "EEXIST: it already exists",
            Self::ENOTDIR => "ENOTDIR: a path component is not a directory",
            Self::EINVAL => "EINVAL: invalid argument",
            Self::ENOSPC => "ENOSPC: no space left on the device",
            Self::EROFS => "EROFS: read-only filesystem",
            Self::EDQUOT => "EDQUOT: the disk quota is exhausted",
            _ => return None,
        })
    }

    /// An `io::Error` may carry no OS number at all (a Rust-side error). Zero is
    /// not a valid errno, so it is unambiguous as "the platform did not say".
    pub fn from_io(e: &std::io::Error) -> Self {
        Self(e.raw_os_error().unwrap_or(0))
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

/// Everything this crate asks of a filesystem, and nothing else.
///
/// Seven methods, each one syscall's worth. See the module documentation for
/// why the crash-safe write is four of them rather than one.
pub trait Filesystem {
    /// Create a directory and every missing parent. Succeeds if it is already a
    /// directory; fails if the path exists as something else.
    fn create_dir_all(&mut self, path: &str) -> Result<(), Errno>;

    /// Read a whole file.
    ///
    /// **Bytes, not text.** A `/state` damaged in the way `ADR-018` §8 is about
    /// produces bytes that are not UTF-8, and deciding what that means is a
    /// decision — so it happens above this seam, where a test can drive it.
    fn read(&mut self, path: &str) -> Result<Vec<u8>, Errno>;

    /// Create or truncate `path` and write every byte.
    ///
    /// **This does not flush.** On `ext4` with delayed allocation the bytes are
    /// in the page cache when this returns and the blocks are not yet allocated,
    /// which is why [`Self::sync_file`] is a separate call and why it can fail
    /// with `ENOSPC` after this one succeeded.
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), Errno>;

    /// Flush one file's contents to the medium — `fsync(2)`.
    ///
    /// **The step most likely to be skipped and most expensive to skip.** Until
    /// it returns, a rename over the target can put a directory entry in place
    /// pointing at blocks the card has never been told to write, and a power cut
    /// then produces a file that is present, the right length, and full of
    /// zeroes.
    fn sync_file(&mut self, path: &str) -> Result<(), Errno>;

    /// Rename within one filesystem — `rename(2)`, which is atomic with respect
    /// to a concurrent reader and to a crash.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno>;

    /// Flush a *directory*, so that a rename inside it survives a power cut.
    ///
    /// Distinct from [`Self::sync_file`]: flushing the file makes its contents
    /// durable, and flushing the directory makes the *name* pointing at them
    /// durable. Without this a crash can lose the rename entirely — which leaves
    /// the previous document in place, so it is the safe direction, but it is
    /// not the durability a caller asked for.
    fn sync_dir(&mut self, path: &str) -> Result<(), Errno>;

    /// Remove a file. Used only to clear a temp this crate has just failed to
    /// write; see [`crate::store::ConfigStore::save`] for why that matters on a
    /// full filesystem.
    fn remove_file(&mut self, path: &str) -> Result<(), Errno>;
}

// ---------------------------------------------------------------------------
// The real one. Thin by policy: nothing here is reachable from a test that does
// not have a real filesystem underneath it.
// ---------------------------------------------------------------------------

/// The filesystem, via `std::fs`.
///
/// Unix only — see the module documentation for why `sync_dir` cannot be
/// honestly implemented anywhere else.
#[cfg(unix)]
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFilesystem;

#[cfg(unix)]
impl StdFilesystem {
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(unix)]
impl Filesystem for StdFilesystem {
    fn create_dir_all(&mut self, path: &str) -> Result<(), Errno> {
        std::fs::create_dir_all(path).map_err(|e| Errno::from_io(&e))
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        std::fs::read(path).map_err(|e| Errno::from_io(&e))
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), Errno> {
        std::fs::write(path, bytes).map_err(|e| Errno::from_io(&e))
    }

    fn sync_file(&mut self, path: &str) -> Result<(), Errno> {
        // Reopened rather than kept open across `write`. `fsync(2)` flushes the
        // *inode*, not the descriptor, so a read-only descriptor is enough — and
        // keeping a handle across two port calls would put state in the adapter,
        // which is the thing this seam exists to avoid.
        let file = std::fs::File::open(path).map_err(|e| Errno::from_io(&e))?;
        file.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        std::fs::rename(from, to).map_err(|e| Errno::from_io(&e))
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), Errno> {
        // Opening a directory read-only and calling `fsync` on it is the POSIX
        // way to make a rename durable, and it is what every database does.
        let dir = std::fs::File::open(path).map_err(|e| Errno::from_io(&e))?;
        dir.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), Errno> {
        std::fs::remove_file(path).map_err(|e| Errno::from_io(&e))
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The constants are Linux's, written down here rather than read from a
    /// header, so this is what stops them going stale (`AGENTS.md` §4 — a suite
    /// that cannot be run rots, so the check runs wherever it can).
    #[test]
    #[cfg(target_os = "linux")]
    fn linux_errno_constants_match_the_platforms_own() {
        assert_eq!(Errno::EPERM.0, libc::EPERM);
        assert_eq!(Errno::ENOENT.0, libc::ENOENT);
        assert_eq!(Errno::EIO.0, libc::EIO);
        assert_eq!(Errno::EACCES.0, libc::EACCES);
        assert_eq!(Errno::EEXIST.0, libc::EEXIST);
        assert_eq!(Errno::ENOTDIR.0, libc::ENOTDIR);
        assert_eq!(Errno::EINVAL.0, libc::EINVAL);
        assert_eq!(Errno::ENOSPC.0, libc::ENOSPC);
        assert_eq!(Errno::EROFS.0, libc::EROFS);
        assert_eq!(Errno::EDQUOT.0, libc::EDQUOT);
    }

    #[test]
    fn a_known_errno_is_named_and_an_unknown_one_is_a_number() {
        assert!(Errno::ENOSPC.to_string().contains("ENOSPC"));
        assert!(Errno::ENOSPC.to_string().contains("28"));
        assert_eq!(Errno(4242).to_string(), "errno 4242");
        assert!(Errno(0).to_string().contains("no OS error number"));
    }

    #[test]
    fn an_io_error_with_no_os_number_becomes_zero_rather_than_a_guess() {
        let e = std::io::Error::other("a Rust-side failure");
        assert_eq!(Errno::from_io(&e), Errno(0));
    }

    // -- the real adapter, against a real filesystem -------------------------
    //
    // These run under WSL and on the target and are skipped on the Windows
    // development host, where `StdFilesystem` does not compile. `AGENTS.md` §6:
    // what is verified, and by what method.

    #[cfg(unix)]
    fn scratch(name: &str) -> String {
        // A path is an argument, never a constant (`AGENTS.md` §8.3), and the
        // pid keeps two concurrent runs apart.
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-store-{}-{name}", std::process::id()));
        dir.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn the_real_adapter_writes_syncs_renames_and_reads_back() {
        let dir = scratch("roundtrip");
        let mut fs = StdFilesystem::new();
        assert_eq!(fs.create_dir_all(&dir), Ok(()));
        let temp = format!("{dir}/a.new");
        let target = format!("{dir}/a");
        assert_eq!(fs.write(&temp, b"hello"), Ok(()));
        assert_eq!(fs.sync_file(&temp), Ok(()));
        assert_eq!(fs.rename(&temp, &target), Ok(()));
        assert_eq!(fs.sync_dir(&dir), Ok(()));
        assert_eq!(fs.read(&target), Ok(b"hello".to_vec()));
        // The rename moved it: nothing is left at the temp path.
        assert_eq!(fs.read(&temp), Err(Errno::ENOENT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The one that matters most, because it is the branch every first boot
    /// takes: an absent file must produce `ENOENT` and not some other number
    /// this crate would report as a fault.
    #[cfg(unix)]
    #[test]
    fn the_real_adapter_reports_an_absent_file_as_enoent() {
        let mut fs = StdFilesystem::new();
        let missing = scratch("definitely-not-here/nor-this");
        assert_eq!(fs.read(&missing), Err(Errno::ENOENT));
        assert_eq!(fs.sync_file(&missing), Err(Errno::ENOENT));
        assert_eq!(fs.remove_file(&missing), Err(Errno::ENOENT));
    }

    /// `create_dir_all` over an existing *file* must fail rather than silently
    /// doing nothing — the boot-time equivalent is a `/state/config` that is
    /// somehow a file, and a save that then wrote nothing would be the worst
    /// available outcome.
    #[cfg(unix)]
    #[test]
    fn the_real_adapter_refuses_a_directory_over_a_file_and_says_what_unix_said() {
        let dir = scratch("dir-over-file");
        let mut fs = StdFilesystem::new();
        assert_eq!(fs.create_dir_all(&dir), Ok(()));
        let file = format!("{dir}/occupied");
        assert_eq!(fs.write(&file, b"x"), Ok(()));
        let err = fs.create_dir_all(&file);
        // Reported rather than predicted: `AGENTS.md` §5. The value is asserted
        // to be an error and *printed*, so a platform that answers differently
        // is visible in the log rather than turning this red for the wrong
        // reason.
        eprintln!("create_dir_all over a file answered: {err:?}");
        assert!(err.is_err(), "a directory over a file must refuse");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_real_adapter_is_idempotent_over_a_directory_that_exists() {
        let dir = scratch("idempotent");
        let mut fs = StdFilesystem::new();
        assert_eq!(fs.create_dir_all(&dir), Ok(()));
        assert_eq!(fs.create_dir_all(&dir), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
