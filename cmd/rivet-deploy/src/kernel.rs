//! The seam between this binary's decisions and the operating system.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! `AGENTS.md` §3.1's table places it squarely in the right-hand column — **an
//! adapter's own I/O seam, not a domain port**: its methods name [`Errno`],
//! block devices and mount points, all of which `rivet-core` is forbidden to
//! name, and its purpose is to let this binary be tested without the machine it
//! is installing onto.
//!
//! # This trait is the fourth of its shape, and the reason is unchanged
//!
//! `cmd/rivet-initd`, `cmd/rivet-sysd` and `cmd/rivet-timed` each
//! have a `Kernel`, and `Errno` is the same type in all of them. **A binary
//! crate has no library target**, so nothing can `use` one out of another;
//! `ADR-013` §7 ruled that the shared platform crate emerges at Stage 3 out of
//! accumulated evidence rather than being built speculatively, and this is one
//! more data point for it.
//!
//! **What this copy adds to that evidence is a shape none of the other three
//! has**: bulk transfer. [`Kernel::read_range`] and [`Kernel::write_device`]
//! move 640 MiB through a fixed buffer, and nothing else in the workspace needs
//! either. So the overlap that keeps recurring is still `Errno` and the
//! clock-and-record half — not this.
//!
//! # Why a payload is not read into memory
//!
//! `ADR-018` §4 makes a `/system` slot 640 MiB and `ADR-002` gives the board
//! 4 GB with no swap. Every operation over a member's bytes here is therefore a
//! stream through a one-megabyte buffer, and the member's length comes from the
//! **signed** header rather than from the file — `ADR-029` §5 makes that
//! ordering the whole point of the format.
//!
//! # What is deliberately not here
//!
//! * **No reboot.** `ADR-029` §9.1 rules that install and arm are one operation
//!   *and the reboot is separate*, and `rivet_adapter_image::ArmedReboot`'s
//!   own documentation gives the design reason. An armed-but-not-rebooted
//!   machine is completely inert, so this binary stops at the arming and says
//!   what kind of reboot carries it.
//! * **No `/state` access.** `ADR-029` §9.2 step 6 verifies each member by
//!   streaming it out of the payload, so there is no staging file and therefore
//!   **zero `/state` wear** — `ADR-018` §2 honoured rather than worked around.

use rivet_adapter_image::Mounted;
use rivet_core::journal::Record;

/// The read and write size for bulk transfer.
///
/// One MiB, which is what `cmd/rivet`'s `measure hash` already uses and for
/// the same reason: large enough that `read(2)`'s per-call cost is not what
/// dominates, small enough to stay in cache and to keep the resident set of a
/// 640 MiB operation flat.
pub const CHUNK: usize = 1024 * 1024;

/// A raw OS error number, kept as data so a test can produce any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

/// The numbers are **Linux's own**, from `asm-generic/errno-base.h`, written out
/// rather than taken from `libc`.
///
/// The same choice `rivet-adapter-image`'s port makes, and for the same
/// reason: the numbers a Rivet machine produces are a Linux kernel's, and this
/// crate is compiled on Windows as well (`AGENTS.md` §10.5), where the
/// platform's table means something else. `crate::syscall`'s tests compare each
/// against `libc`'s under `cfg(target_os = "linux")`, which is where the
/// comparison is meaningful.
///
/// **Four of these are named for the fakes and for the tests that drive them.**
///
/// The shipped code never constructs one: it reports whatever the platform set
/// (`AGENTS.md` §5 — *report what the system said*), so the `allow` is the same
/// one `cmd/rivet-timed`'s `Errno::EPERM` carries and for the same reason.
/// They earn their place by being the states this binary must be right about —
/// a card gone read-only mid-write, a boot half with no room, a payload that is
/// not there — and none of those is reachable on a developer's machine.
impl Errno {
    /// No such file or directory.
    #[allow(dead_code)]
    pub const ENOENT: Self = Self(2);
    /// The device returned an I/O error. Also what this crate answers for a file
    /// shorter than the signed header says it is.
    pub const EIO: Self = Self(5);
    /// Another process holds an advisory lock — what [`Kernel::lock_within`]
    /// answers once its bound has passed.
    pub const EAGAIN: Self = Self(11);
    /// Already mounted at this point — handled as the convergent answer by
    /// [`StdKernel::mount`], for the reason `rivet_adapter_image`'s port
    /// documents.
    pub const EBUSY: Self = Self(16);
    /// Invalid argument. A path with an interior NUL cannot be handed to
    /// `mount(2)`.
    #[allow(dead_code)]
    pub const EINVAL: Self = Self(22);
    /// No space left. What a boot half that will not take another kernel says.
    #[allow(dead_code)]
    pub const ENOSPC: Self = Self(28);
    /// Read-only filesystem — **how an SD card usually announces that it is
    /// dying.** The kernel remounts it read-only rather than refusing, so this
    /// arrives at a write on a machine that was healthy a moment earlier.
    #[allow(dead_code)]
    pub const EROFS: Self = Self(30);

    /// An `io::Error` may carry no OS number at all. Zero is not a valid errno,
    /// so it is unambiguous as "the platform did not say".
    pub fn from_io(e: &std::io::Error) -> Self {
        Self(e.raw_os_error().unwrap_or(0))
    }
}

impl core::fmt::Display for Errno {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 == 0 {
            return write!(f, "error with no OS error number");
        }
        write!(
            f,
            "{} (errno {})",
            std::io::Error::from_raw_os_error(self.0),
            self.0
        )
    }
}

/// Every operating-system fact this binary depends on.
///
/// Object-safe on purpose, like every other `Kernel` in this workspace: the
/// decisions above it are compiled once, and the fake drives exactly the code a
/// board runs.
pub trait Kernel {
    // -- the journal's inputs ------------------------------------------------
    /// Milliseconds from a monotonic clock. `ADR-022` §3.2's `mono`.
    fn now_ms(&mut self) -> u64;

    /// Milliseconds since the Unix epoch, or `None` if there is no readable
    /// wall clock. **`None` is the ordinary state of a cold Pi** (`ADR-021`).
    fn wall_ms(&mut self) -> Option<u64>;

    /// The journal's sink. Cannot fail: there is nowhere to report a failed
    /// report to.
    fn emit(&mut self, record: &Record);

    // -- reading -------------------------------------------------------------
    /// A whole text file. Used for the boot identifier and for `autoboot.txt`.
    fn read_file(&mut self, path: &str) -> Result<String, Errno>;

    /// A whole file as bytes — a public key, a device-tree property, or one
    /// file of a boot half being compared against the payload.
    ///
    /// **Never used for a member of the payload**: those are read with
    /// [`Self::read_range`] because the largest is a whole filesystem.
    fn read_bytes(&mut self, path: &str) -> Result<Vec<u8>, Errno>;

    /// How many bytes `path` holds.
    ///
    /// **Must answer for a block device as well as for a regular file**, which
    /// `stat(2)` does not: `st_size` is 0 for a block device. It is what
    /// `ADR-029` F8's fit check is made of, so an implementor that answered 0
    /// would refuse every payload.
    fn size_of(&mut self, path: &str) -> Result<u64, Errno>;

    /// Read `len` bytes of `path` starting at `offset`, a chunk at a time.
    ///
    /// The sink returns `false` to stop early — which is how a caller whose own
    /// step failed (a digest mismatch, say) stops reading 640 MiB it no longer
    /// wants. Stopping early is `Ok(())`; the caller holds the reason.
    ///
    /// Reading fewer than `len` bytes is [`Errno::EIO`], not a short answer. The
    /// length came from the signed header (`ADR-029` §5), so a file that cannot
    /// supply it is a truncated transfer rather than a shorter payload.
    fn read_range(
        &mut self,
        path: &str,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), Errno>;

    // -- writing -------------------------------------------------------------
    /// Copy `len` bytes of `src` from `offset` onto `dst` from its start, then
    /// `fsync`. `ADR-029` §9.2 step 7.
    ///
    /// **`dst` is opened for writing and never truncated**, because it is a
    /// block device: truncating one is meaningless, and a `create` would make a
    /// typo produce a regular file that silently succeeded.
    ///
    /// The `fsync` is not optional and is not the caller's to skip. On a card it
    /// is the whole of what stands between a completed write and a power cut.
    fn write_device(&mut self, src: &str, offset: u64, len: u64, dst: &str) -> Result<u64, Errno>;

    /// Create or truncate `path`, write every byte, and `fsync` **the file**.
    ///
    /// The sync is inside this call for the reason `ADR-029` §9's third bound on
    /// F5 gives: write-sync-rename is one sequence, and a caller that could
    /// perform two thirds of it would eventually.
    fn write_file_synced(&mut self, path: &str, bytes: &[u8]) -> Result<(), Errno>;

    /// Rename within one filesystem — atomic with respect to a concurrent
    /// reader and to a crash.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno>;

    /// Create `path` with `mode`, and **succeed if it is already there**.
    ///
    /// Convergent rather than idempotent-by-accident: `ADR-006` §6.2 makes every
    /// operation re-runnable, and an install that failed halfway is precisely
    /// the case somebody re-runs. An implementor that returned `EEXIST` would
    /// make the second attempt fail on the first directory.
    ///
    /// **It does not create parents.** `rivet_core::package::install`'s plan
    /// names every directory in order, so a missing parent means the plan is
    /// wrong and that should be a refusal rather than a `mkdir -p` papering over
    /// it.
    fn mkdir_mode(&mut self, path: &str, mode: u32) -> Result<(), Errno>;

    /// **Is there anything at `path` at all?** `ADR-015` §7.2.
    ///
    /// It exists for exactly one caller —
    /// `rivet_core::package::install::Plan::occupied`, which refuses to unpack
    /// over a version directory that is already there — and its shape is decided
    /// by that caller.
    ///
    /// **`bool` and not `Result`, deliberately.** The question this answers is
    /// *may this directory be created and written into*, and every way of failing
    /// to find out is the same answer: if the path cannot be reached, the
    /// directory is not there to be written over and the install proceeds to
    /// `mkdir_mode`, which is where a real permission or medium fault becomes a
    /// refusal that names the errno. A `Result` here would make the caller
    /// invent a second policy for `EACCES` and get it different.
    ///
    /// **It does not follow symbolic links.** A link at the version directory's
    /// name is something already there, whatever it points at.
    fn path_exists(&mut self, path: &str) -> bool;

    /// Set the mode of a path that already exists.
    ///
    /// Separate from [`Kernel::write_file_synced`] because a file is created
    /// under the process umask and then corrected — one syscall more than
    /// creating it with the mode, and one fewer thing for an implementor to get
    /// subtly wrong on a platform where `O_CREAT`'s mode argument is masked.
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), Errno>;

    /// Create a symbolic link at `path` pointing at `target`.
    ///
    /// `target` is written verbatim and may be relative — which is what
    /// `ADR-015` §7's `current` link is, so that nothing on the machine has to
    /// agree about an absolute prefix.
    ///
    /// **Fails if `path` exists.** The atomic swap is a link made under a
    /// staging name and then renamed over the real one, so an implementor that
    /// helpfully replaced an existing link would remove the property the whole
    /// sequence exists for.
    fn symlink(&mut self, target: &str, path: &str) -> Result<(), Errno>;

    /// Remove a symbolic link, and **succeed if it was not there**.
    ///
    /// **The only deletion this binary can perform**, and it is deliberately
    /// narrow. It exists for one case: an install that died between staging the
    /// `current` link and renaming it leaves the staging name behind, and
    /// `symlink(2)` would then answer `EEXIST` for ever — a machine that could
    /// never install that application again, from a crash that harmed nothing.
    ///
    /// An implementor must refuse anything that is not a symbolic link. A
    /// `remove` that would take a directory or a program is a different
    /// capability, and this binary has no business holding it.
    fn unlink_symlink(&mut self, path: &str) -> Result<(), Errno>;

    /// Flush filesystem buffers. Cannot fail, because `sync(2)` cannot.
    fn sync(&mut self);

    // -- mounting ------------------------------------------------------------
    /// Mount `device` at `at`, creating the mount point.
    ///
    /// Succeeding when it is already mounted **at that same point** is the
    /// implementor's job and is the convergent answer rather than a lie. It is
    /// safe only because this binary mounts at
    /// `rivet_adapter_image::PAYLOAD_BOOT_MOUNT` and
    /// `AUTOBOOT_PARTITION_MOUNT`, neither of which PID 1 uses — that constant
    /// carries what sharing one cost.
    ///
    /// # `EBUSY` alone is not "already mounted here", and a bench run proved it
    ///
    /// **Found 2026-08-24, running the real binary against a real card image.**
    /// `mount(2)` answers `EBUSY` for at least two different situations, and
    /// only one of them is convergent:
    ///
    /// | Why the kernel said `EBUSY` | Treating it as success is |
    /// | :-- | :-- |
    /// | this device is already mounted at **this** point | correct — the machine is in the state that was asked for |
    /// | this device is already mounted **somewhere else** with an incompatible flag — read-only there, writable here — and the superblock cannot be shared | **a lie** |
    ///
    /// The second case is not hypothetical: `cmd/rivet-initd`'s provisioning
    /// read mounts the *booted* boot partition, and on a machine running slot A
    /// that is the same `p1` an arming rewrites. If that mount is still in
    /// place, this call would report success over a mount that never happened —
    /// and the caller would then write the payload's kernel into an empty
    /// directory **on the root filesystem** and report an install.
    ///
    /// So an implementor must confirm that `at` really is a mount point before
    /// treating `EBUSY` as convergence. `cmd/rivet/src/boot.rs`'s
    /// `SyscallMedium` makes the same check.
    ///
    /// On success it says whether this call made the mount or found one in
    /// place ([`Mounted`]). A caller unmounts only a mount it made.
    fn mount(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno>;

    /// Take `path`'s exclusive advisory lock (`flock(2)`), creating the file
    /// mode `0600` and its directory if they are missing, and waiting at most
    /// `bound_ms` for another holder to let go — then [`Errno::EAGAIN`].
    /// Held until [`Self::unlock`], or until this process exits.
    fn lock_within(&mut self, path: &str, bound_ms: u64) -> Result<(), Errno>;

    /// Release a lock [`Self::lock_within`] took. Cannot fail: closing the
    /// descriptor releases it.
    fn unlock(&mut self, path: &str);

    /// Unmount `at` again.
    ///
    /// **Returns the outcome, where `rivet_adapter_image::BootMedium::unmount`
    /// does not**, and the difference is deliberate. That port gives the outcome
    /// to the implementor because *it* has no caller who could act on one; this
    /// binary has a journal, and a leaked mount is worth a record. A leak
    /// outlives the process, and the next invocation's mount converges onto it
    /// — `cmd/rivet`'s `SyscallMedium` carries what that costs on a
    /// read-only mount.
    fn umount(&mut self, at: &str) -> Result<(), Errno>;
}

// ---------------------------------------------------------------------------
// The real one. Thin by policy: nothing here is reachable from a test.
// ---------------------------------------------------------------------------

/// How this binary renders records.
///
/// `ADR-022` §4 has the supervisor capture a child's stdout and parse `ADR-022`
/// §3 records back out of it, so [`ConsoleForm::Jsonl`] is what a supervised run
/// wants and is the default. `rivet-timed` carries the same pair for the same
/// reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleForm {
    /// `ADR-022` §3 — the on-disk form, and the authoritative one.
    Jsonl,
    /// A *view* of it (`ADR-022` §2), for a human reading a terminal.
    Text,
}

/// The operating system, via `std` and [`crate::syscall`].
///
/// # It is portable on purpose, and exactly four calls are not
///
/// Reading a payload, verifying every member, writing bytes into a block device
/// and renaming a file are all `std`, so **this type exists on the Windows
/// development host too** and the whole install sequence can be driven there
/// against files standing in for partitions. That is what makes
/// `tests/install_end_to_end.rs` an integration test rather than a fake.
///
/// The four that are not portable are `mount`, `umount2`, `sync` and the
/// monotonic clock, and [`crate::syscall`] is where they live. Off Linux the
/// first two **refuse** — `AGENTS.md` §3, never `Ok(())` — so an install
/// attempted on a machine with no slots fails at the boot half and says so,
/// rather than reporting an update it did not perform.
pub struct StdKernel {
    form: ConsoleForm,
    /// Advisory locks held, by path — [`Kernel::lock_within`]. Removing an
    /// entry closes its descriptor, which is what releases the lock.
    locks: std::collections::BTreeMap<String, std::fs::File>,
}

impl StdKernel {
    pub const fn new(form: ConsoleForm) -> Self {
        Self {
            form,
            locks: std::collections::BTreeMap::new(),
        }
    }
}

/// How often [`StdKernel::lock_within`] tries a held lock again.
const LOCK_POLL_MS: u64 = 50;

impl Kernel for StdKernel {
    fn now_ms(&mut self) -> u64 {
        crate::syscall::monotonic_ms()
    }

    fn wall_ms(&mut self) -> Option<u64> {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
    }

    fn emit(&mut self, record: &Record) {
        use std::io::Write as _;
        let line = match self.form {
            ConsoleForm::Jsonl => record.to_jsonl(),
            ConsoleForm::Text => format!("{}\n", record.to_text()),
        };
        // A closed console must not become the reason an install dies halfway,
        // so both results are dropped deliberately.
        let mut out = std::io::stdout();
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        std::fs::read_to_string(path).map_err(|e| Errno::from_io(&e))
    }

    fn read_bytes(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        std::fs::read(path).map_err(|e| Errno::from_io(&e))
    }

    fn size_of(&mut self, path: &str) -> Result<u64, Errno> {
        use std::io::{Seek as _, SeekFrom};
        // Seeking to the end rather than `metadata().len()`, because `st_size`
        // is 0 for a block device and this number is `ADR-029` F8's fit check.
        // `lseek(2)` on a block device answers its size in bytes.
        let mut file = std::fs::File::open(path).map_err(|e| Errno::from_io(&e))?;
        file.seek(SeekFrom::End(0)).map_err(|e| Errno::from_io(&e))
    }

    fn read_range(
        &mut self,
        path: &str,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), Errno> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let mut file = std::fs::File::open(path).map_err(|e| Errno::from_io(&e))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| Errno::from_io(&e))?;
        let mut buffer = vec![0u8; CHUNK];
        let mut left = len;
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
            match file.read(&mut buffer[..want]) {
                // The length came from the signed header. A short file is a
                // truncated transfer, not a smaller payload.
                Ok(0) => return Err(Errno::EIO),
                Ok(n) => {
                    if !sink(&buffer[..n]) {
                        return Ok(());
                    }
                    left -= n as u64;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(Errno::from_io(&e)),
            }
        }
        Ok(())
    }

    fn write_device(&mut self, src: &str, offset: u64, len: u64, dst: &str) -> Result<u64, Errno> {
        use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
        let mut source = std::fs::File::open(src).map_err(|e| Errno::from_io(&e))?;
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|e| Errno::from_io(&e))?;
        // `write(true)` and nothing else: no `create`, no `truncate`. A typo
        // must fail rather than produce a regular file that succeeds.
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .open(dst)
            .map_err(|e| Errno::from_io(&e))?;

        let mut buffer = vec![0u8; CHUNK];
        let mut left = len;
        while left > 0 {
            let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
            source
                .read_exact(&mut buffer[..want])
                .map_err(|e| Errno::from_io(&e))?;
            target
                .write_all(&buffer[..want])
                .map_err(|e| Errno::from_io(&e))?;
            left -= want as u64;
        }
        target.sync_all().map_err(|e| Errno::from_io(&e))?;
        Ok(len)
    }

    fn write_file_synced(&mut self, path: &str, bytes: &[u8]) -> Result<(), Errno> {
        use std::io::Write as _;
        let mut file = std::fs::File::create(path).map_err(|e| Errno::from_io(&e))?;
        file.write_all(bytes).map_err(|e| Errno::from_io(&e))?;
        file.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        std::fs::rename(from, to).map_err(|e| Errno::from_io(&e))
    }

    fn mkdir_mode(&mut self, path: &str, mode: u32) -> Result<(), Errno> {
        match make_dir(path, mode) {
            Ok(()) => Ok(()),
            // Already there is the CONVERGENT answer, not a failure. The mode is
            // deliberately not corrected: a directory somebody else made is a
            // fact about this machine, and quietly widening its permissions
            // would be a change nobody asked for, made by a tool nobody was
            // watching.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(Errno::from_io(&e)),
        }
    }

    /// `lstat(2)` through `symlink_metadata`, which succeeds for a directory, a
    /// file, or a dangling symbolic link — every one of which is *something
    /// already there*.
    fn path_exists(&mut self, path: &str) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), Errno> {
        set_mode(path, mode).map_err(|e| Errno::from_io(&e))
    }

    fn symlink(&mut self, target: &str, path: &str) -> Result<(), Errno> {
        make_symlink(target, path).map_err(|e| Errno::from_io(&e))
    }
    fn unlink_symlink(&mut self, path: &str) -> Result<(), Errno> {
        match std::fs::symlink_metadata(path) {
            // Not there is the answer this wanted anyway.
            Err(_) => Ok(()),
            Ok(meta) if meta.file_type().is_symlink() => {
                std::fs::remove_file(path).map_err(|e| Errno::from_io(&e))
            }
            // A directory or a regular file at the staging name is not
            // something this made, and removing it is not something this may
            // do. Refused, so an operator finds out rather than losing it.
            Ok(_) => Err(Errno(libc::EPERM)),
        }
    }

    fn sync(&mut self) {
        crate::syscall::sync();
    }

    fn mount(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno> {
        // `/run` is a tmpfs PID 1 mounts at boot (`rivet_core::mount`), so the
        // mount point is created rather than assumed to exist.
        std::fs::create_dir_all(at).map_err(|e| Errno::from_io(&e))?;
        match crate::syscall::mount(device, at, fstype, writable) {
            Ok(()) => Ok(Mounted::Made),
            // Convergent **only if something really is mounted there** — and
            // then it is somebody else's mount. See the port's documentation
            // for the case that is not, and for what a bench run found it
            // costs.
            Err(errno) if Errno(errno) == Errno::EBUSY && is_mount_point(at) => {
                Ok(Mounted::Adopted)
            }
            Err(errno) => Err(Errno(errno)),
        }
    }

    fn umount(&mut self, at: &str) -> Result<(), Errno> {
        crate::syscall::unmount(at).map_err(Errno)
    }

    fn lock_within(&mut self, path: &str, bound_ms: u64) -> Result<(), Errno> {
        let file = crate::syscall::open_lock_file(path).map_err(Errno)?;
        let started = self.now_ms();
        loop {
            if crate::syscall::try_lock(&file).map_err(Errno)? {
                self.locks.insert(path.to_string(), file);
                return Ok(());
            }
            if self.now_ms().saturating_sub(started) >= bound_ms {
                return Err(Errno::EAGAIN);
            }
            std::thread::sleep(std::time::Duration::from_millis(LOCK_POLL_MS));
        }
    }

    fn unlock(&mut self, path: &str) {
        self.locks.remove(path);
    }
}

/// Is anything mounted at `path`?
///
/// **Compares device numbers rather than reading `/proc/self/mountinfo`**, and
/// the reason is not taste: a mount point is by definition a directory whose
/// filesystem differs from its parent's, so `st_dev` answers it in two `stat(2)`
/// calls with nothing to parse. `/proc` would work on a board and is a
/// second-order dependency for a first-order fact — and this binary is the one
/// that has to be right about mounts before it writes 640 MiB.
///
/// A path that cannot be stat'd is **not** a mount point, which is the safe
/// direction: the caller then reports the `EBUSY` rather than writing into a
/// directory it believes is a card.
///
/// The root directory is its own parent, so it would answer `false` here. That
/// is correct for this caller — nothing mounts a boot partition at `/` — and it
/// is stated so nobody reuses the function expecting otherwise.
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

/// Off Linux there is no `mount(2)` to return `EBUSY` in the first place —
/// `crate::syscall::mount` refuses with `ENOSYS` — so this is unreachable and
/// answers the safe value.
#[cfg(not(target_os = "linux"))]
fn is_mount_point(_path: &str) -> bool {
    false
}

// ---------------------------------------------------------------------------
// The three filesystem operations that have no portable spelling
// ---------------------------------------------------------------------------
//
// `ADR-014` §2 keeps platform knowledge in one place, and this is the smallest
// version of it: three functions with a Linux body and a refusing one, so the
// crate still compiles on the development host and its tests still run there.
//
// **The non-Linux halves refuse rather than approximate.** `AGENTS.md` §3 —
// a stub that reported success would let a test pass for a sequence that cannot
// work on a board, which is worse than not building at all.

#[cfg(target_os = "linux")]
fn make_dir(path: &str, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new().mode(mode).create(path)
}

#[cfg(not(target_os = "linux"))]
fn make_dir(_path: &str, _mode: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a directory mode has no meaning off Linux",
    ))
}

#[cfg(target_os = "linux")]
fn set_mode(path: &str, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(target_os = "linux"))]
fn set_mode(_path: &str, _mode: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a Unix mode has no meaning off Linux",
    ))
}

#[cfg(target_os = "linux")]
fn make_symlink(target: &str, path: &str) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(not(target_os = "linux"))]
fn make_symlink(_target: &str, _path: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a symlink here would need privileges this is not asking for",
    ))
}
