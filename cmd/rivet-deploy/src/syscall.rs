//! The calls `std` does not have, and what happens where they do not exist.
//!
//! | Call | Why `std` cannot |
//! | :-- | :-- |
//! | `mount(2)` | no `std` equivalent at all |
//! | `umount2(2)` | likewise |
//! | `sync(2)` | `std` flushes a *file*, never a filesystem |
//! | `clock_gettime(CLOCK_MONOTONIC)` | `std::time::Instant` is monotonic and cannot be rendered as milliseconds since boot, which is what `ADR-022` §3.2's `mono` field is |
//! | `flock(2)` | `std` has no advisory lock |
//!
//! **Every `unsafe` block in this crate is inside [`linux`], each with a
//! SAFETY comment.** That sentence and
//! the test at the bottom of this file are the wall around
//! `[lints.rust] unsafe_code = "allow"`, the same one `cmd/rivet/src/syscall.rs`
//! and `cmd/rivet-timed`'s `kernel.rs` put around theirs (`ADR-009` §3.4: no
//! `libc` type appears outside this module).
//!
//! # Off Linux they refuse, and that is `AGENTS.md` §3
//!
//! *"Stubs refuse; they never fake success."* The development host is Windows
//! (`AGENTS.md` §10.5), and the rest of this crate is deliberately portable:
//! reading a payload, verifying every member and writing bytes into a file are
//! all `std`, so the whole install sequence can be driven on the host against
//! **files standing in for partitions**. What cannot be is [`mount`], and
//! `elsewhere` returns `ENOSYS` rather than `Ok(())` — so an install attempted
//! on a machine with no slots fails at the boot half and reports it, instead of
//! claiming to have updated something.

/// `ENOSYS` — function not implemented. Linux's number, for the reason
/// `crate::kernel::Errno`'s constants are Linux's.
///
/// **Unused on Linux itself**, which is the target that matters: it is what
/// `elsewhere`'s refusals answer, and a Linux build has no `elsewhere`. The
/// `allow` is therefore the honest one — the constant is live on the
/// development host and dead on the board.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const ENOSYS: i32 = 38;

#[cfg(target_os = "linux")]
mod linux {
    /// Mount a filesystem.
    ///
    /// The flags are this crate's rather than the image adapter's, exactly as
    /// `rivet_adapter_image::BootMedium::mount_boot_filesystem` documents:
    /// they are a platform's own bits, and a FAT filesystem an operator edits
    /// from a Windows machine is mounted `noexec`, `nosuid` and `nodev`
    /// (`ADR-002` §4.1). The same three PID 1 uses, so an update's mount is no
    /// more permissive than the boot's.
    pub fn mount(device: &str, at: &str, fstype: &str, writable: bool) -> Result<(), i32> {
        let (Ok(device), Ok(at), Ok(fstype)) = (
            std::ffi::CString::new(device),
            std::ffi::CString::new(at),
            std::ffi::CString::new(fstype),
        ) else {
            // An interior NUL. Refused here rather than truncated, because a
            // truncated device path can name a *different* partition.
            return Err(libc::EINVAL);
        };
        let mut flags = libc::MS_NOEXEC | libc::MS_NOSUID | libc::MS_NODEV;
        if !writable {
            flags |= libc::MS_RDONLY;
        }
        // SAFETY: three live NUL-terminated pointers and a null data argument,
        // which `mount(2)` documents as permitted for a filesystem taking no
        // options. Nothing is retained past the call.
        let rc = unsafe {
            libc::mount(
                device.as_ptr(),
                at.as_ptr(),
                fstype.as_ptr(),
                flags,
                std::ptr::null(),
            )
        };
        if rc == 0 { Ok(()) } else { Err(last()) }
    }

    /// Unmount a filesystem.
    pub fn unmount(at: &str) -> Result<(), i32> {
        let Ok(at) = std::ffi::CString::new(at) else {
            return Err(libc::EINVAL);
        };
        // SAFETY: one live NUL-terminated pointer, no flags, nothing retained.
        let rc = unsafe { libc::umount2(at.as_ptr(), 0) };
        if rc == 0 { Ok(()) } else { Err(last()) }
    }

    /// Flush every filesystem's buffers. `sync(2)` cannot fail.
    pub fn sync() {
        // SAFETY: `sync(2)` takes no arguments, returns nothing and cannot fail.
        unsafe { libc::sync() };
    }

    /// Milliseconds from `CLOCK_MONOTONIC`.
    pub fn monotonic_ms() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes one `timespec` through a pointer to a
        // live local and reads nothing else. `CLOCK_MONOTONIC` is a valid clock
        // id on every Linux this targets.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
        if rc != 0 {
            return 0;
        }
        let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
        let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
        secs.saturating_mul(1000).saturating_add(nanos / 1_000_000)
    }

    /// Open `path` for an advisory lock, creating it mode `0600` and its
    /// directory if they are missing.
    pub fn open_lock_file(path: &str) -> Result<std::fs::File, i32> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let io = |e: std::io::Error| e.raw_os_error().unwrap_or(0);
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(io)
    }

    /// Try the exclusive `flock(2)` on `file` without blocking: `Ok(true)`
    /// taken, `Ok(false)` another descriptor holds it.
    pub fn try_lock(file: &std::fs::File) -> Result<bool, i32> {
        use std::os::unix::io::AsRawFd as _;
        loop {
            // SAFETY: `flock(2)` takes a descriptor and an integer operation and
            // touches no memory belonging to this process. The descriptor is
            // owned by `file`, borrowed for the whole call; the operation is two
            // `LOCK_*` constants.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(true);
            }
            match last() {
                libc::EINTR => {}
                e if e == libc::EWOULDBLOCK => return Ok(false),
                e => return Err(e),
            }
        }
    }

    fn last() -> i32 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }
}

#[cfg(not(target_os = "linux"))]
mod elsewhere {
    /// **Refuses.** `AGENTS.md` §3 — never `Ok(())`.
    pub fn mount(_d: &str, _at: &str, _fs: &str, _w: bool) -> Result<(), i32> {
        Err(super::ENOSYS)
    }

    /// Refuses, for the same reason.
    pub fn unmount(_at: &str) -> Result<(), i32> {
        Err(super::ENOSYS)
    }

    /// A no-op, and the one place this module does not refuse — because it
    /// cannot: `sync(2)` returns nothing, so there is no channel to refuse
    /// through.
    ///
    /// **It costs nothing here, and the reason is structural rather than
    /// hopeful.** Its only caller is the boot-half write, which happens inside a
    /// mount — and [`mount`] above refuses on this platform, so the call is
    /// unreachable outside a test. Every byte this crate writes off Linux goes
    /// through `File::sync_all`, which `std` does have.
    pub fn sync() {}

    /// Refuses: the lock guards a boot partition, and there is none to mount
    /// here either.
    pub fn open_lock_file(_path: &str) -> Result<std::fs::File, i32> {
        Err(super::ENOSYS)
    }

    /// Refuses, for the same reason.
    pub fn try_lock(_file: &std::fs::File) -> Result<bool, i32> {
        Err(super::ENOSYS)
    }

    /// A monotonic clock that is not the kernel's boot clock.
    ///
    /// `ADR-022` §3.2's `mono` is *"seconds since boot"*; this is seconds since
    /// this process started, because `std::time::Instant` has no epoch that can
    /// be named. It is honest for what it is used for — ordering the records of
    /// one run, on a host where there is no boot to count from — and it is
    /// never the field anything decides on.
    pub fn monotonic_ms() -> u64 {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        let start = START.get_or_init(Instant::now);
        u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(target_os = "linux")]
pub use linux::{monotonic_ms, mount, open_lock_file, sync, try_lock, unmount};

#[cfg(not(target_os = "linux"))]
pub use elsewhere::{monotonic_ms, mount, open_lock_file, sync, try_lock, unmount};

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::kernel::Errno;

    /// Every errno this crate names must be the number a **Linux** kernel uses,
    /// because that is the kernel a Rivet machine runs — and this crate is
    /// compiled on Windows too, where the platform's table means something else.
    /// `rivet-adapter-image` declares the same numbers by hand and takes no
    /// `libc`; this is the check it cannot make.
    #[cfg(target_os = "linux")]
    #[test]
    fn every_named_errno_is_the_kernels_own_number() {
        assert_eq!(Errno::ENOENT.0, libc::ENOENT);
        assert_eq!(Errno::EIO.0, libc::EIO);
        assert_eq!(Errno::EAGAIN.0, libc::EAGAIN);
        assert_eq!(Errno::EAGAIN.0, libc::EWOULDBLOCK);
        assert_eq!(Errno::EBUSY.0, libc::EBUSY);
        assert_eq!(Errno::EINVAL.0, libc::EINVAL);
        assert_eq!(Errno::ENOSPC.0, libc::ENOSPC);
        assert_eq!(Errno::EROFS.0, libc::EROFS);
        assert_eq!(ENOSYS, libc::ENOSYS);
        assert_eq!(
            rivet_adapter_image::Errno::EROFS.0,
            libc::EROFS,
            "the image adapter's hand-declared EROFS is not this kernel's"
        );
    }

    /// The numbers must all differ, or one of them would be unreportable — and
    /// a console line is the only diagnostic a board gives (`AGENTS.md` §5).
    /// Runs on every platform, unlike the check above.
    #[test]
    fn every_named_errno_is_a_distinct_number() {
        let all = [
            Errno::ENOENT,
            Errno::EIO,
            Errno::EAGAIN,
            Errno::EBUSY,
            Errno::EINVAL,
            Errno::ENOSPC,
            Errno::EROFS,
            Errno(ENOSYS),
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                assert!(i == j || a != b, "{a} and {b} share a number");
            }
        }
    }

    /// A path with an interior NUL is refused before it reaches the kernel;
    /// truncating one could name a different partition.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_path_with_an_interior_nul_is_refused_rather_than_truncated() {
        assert_eq!(
            mount("/dev/x\0y", "/run/x", "vfat", false),
            Err(libc::EINVAL)
        );
        assert_eq!(unmount("/run/x\0y"), Err(libc::EINVAL));
    }

    /// **`AGENTS.md` §3, as a test rather than as a comment.** Off Linux the
    /// mount must refuse. A stub that answered `Ok(())` would let an install
    /// report a boot half it never opened.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn off_linux_a_mount_refuses_rather_than_pretending() {
        assert_eq!(mount("/dev/x", "/run/x", "vfat", true), Err(ENOSYS));
        assert_eq!(unmount("/run/x"), Err(ENOSYS));
    }

    /// **A held lock is waited on for the bound, then refused as `EAGAIN`,
    /// and a released one is taken** — against the real `flock(2)`, through
    /// the kernel this binary runs with. The holder is a second `StdKernel`
    /// and so a second open file description, which `flock` treats as another
    /// process.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_held_lock_is_waited_on_for_the_bound_and_a_released_one_is_taken() {
        use crate::kernel::{ConsoleForm, Kernel as _, StdKernel};
        use std::os::unix::fs::PermissionsExt as _;
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-deploy-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("rivet").join("autoboot.lock");
        let path = path.to_str().expect("a utf-8 temporary path").to_string();

        let mut holder = StdKernel::new(ConsoleForm::Text);
        let mut reader = StdKernel::new(ConsoleForm::Text);
        assert_eq!(holder.lock_within(&path, 0), Ok(()));
        let mode = std::fs::metadata(&path).expect("the lock file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");

        let started = std::time::Instant::now();
        assert_eq!(reader.lock_within(&path, 200), Err(Errno::EAGAIN));
        let waited = started.elapsed();
        assert!(waited >= std::time::Duration::from_millis(200), "{waited:?}");
        assert!(waited < std::time::Duration::from_secs(5), "{waited:?}");

        holder.unlock(&path);
        assert_eq!(reader.lock_within(&path, 200), Ok(()));
        reader.unlock(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The monotonic clock must not go backwards. It is the one call here a
    /// test can safely make on any platform.
    #[test]
    fn the_monotonic_clock_answers_and_does_not_go_backwards() {
        let first = monotonic_ms();
        let second = monotonic_ms();
        assert!(second >= first, "{second} < {first}");
    }
}
