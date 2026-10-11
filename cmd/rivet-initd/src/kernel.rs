//! The seam between PID 1's decisions and the kernel.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! The split exists because of what PID 1 is: an untested branch here is a
//! kernel panic with no console and no stack trace, and the branches that
//! matter most — a read-only `/system` (`ADR-018` §3), `/proc/self/mounts`
//! missing on every first boot, `signalfd(2)` refused, `tryboot` (which
//! `ADR-026` §4 says QEMU cannot emulate **at all**) — cannot be reached from a
//! test that needs root, a kernel and an emulator.
//!
//! **[`LibcKernel`] must stay thin, and should get thinner rather than
//! smarter.** Whatever logic lives in it is logic no test can drive. Its job is
//! to make a syscall, translate the result into one of this module's types, and
//! stop. It holds every `unsafe` block in the crate.
//!
//! One deliberate exception to "no logic here": [`LibcKernel::kill`] refuses a
//! non-positive pid. `kill(-1, …)` from PID 1 signals *every process on the
//! system*, and an interlock against an argument that catastrophic is not
//! logic — it is a fuse. The caller is separately tested never to ask.
//!
//! `ADR-014` §3 is why the `MS_*` translation is here and not in
//! `rivet-core`: core names what the kernel must do, the adapter knows what
//! the kernel calls it.

use rivet_core::mount::MountFlag;

/// Where the kernel publishes the mount table.
///
/// `/proc/self/mounts` rather than `/proc/mounts`: the former is this process's
/// own mount namespace, which is the one whose state we are converging.
pub const MOUNTS_PATH: &str = "/proc/self/mounts";

/// A raw OS error number, kept as data so a test can produce any of them.
///
/// `AGENTS.md` §5 — report what the system said. [`Display`](core::fmt::Display)
/// asks the platform for the message rather than carrying a table of strings
/// this repository would have to keep true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub const EPERM: Self = Self(libc::EPERM);
    pub const ENOENT: Self = Self(libc::ENOENT);
    pub const EINTR: Self = Self(libc::EINTR);
    pub const ECHILD: Self = Self(libc::ECHILD);
    pub const EACCES: Self = Self(libc::EACCES);
    pub const EEXIST: Self = Self(libc::EEXIST);
    pub const ENODEV: Self = Self(libc::ENODEV);
    pub const ENOTDIR: Self = Self(libc::ENOTDIR);
    pub const EBUSY: Self = Self(libc::EBUSY);
    pub const EROFS: Self = Self(libc::EROFS);
    pub const EINVAL: Self = Self(libc::EINVAL);
    /// An advisory lock another process holds — what [`Kernel::try_lock`]
    /// answers rather than waiting.
    pub const EAGAIN: Self = Self(libc::EAGAIN);
    /// The filesystem is full. `/state` is the only one a Rivet machine writes
    /// to (`ADR-018` §4), so this is `/state` and nothing else. Named so a test
    /// can drive `crate::seed`'s failure path; PID 1 never constructs it.
    #[allow(dead_code)]
    pub const ENOSPC: Self = Self(libc::ENOSPC);
    /// A descriptor that has gone bad under a reader. Named so a test can
    /// produce the state that drives the degraded reaper; PID 1 never
    /// constructs it.
    #[allow(dead_code)]
    pub const EIO: Self = Self(libc::EIO);

    /// The errno the last syscall set.
    pub fn last() -> Self {
        Self::from_io(&std::io::Error::last_os_error())
    }

    /// An `io::Error` may carry no OS number at all (a Rust-side error). Zero is
    /// not a valid errno, so it is unambiguous as "the platform did not say".
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

/// A `mount(2)` flag word, in the platform's own bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MountFlags(pub u64);

impl MountFlags {
    /// No bits at all. Named so a test can assert on the empty case.
    #[allow(dead_code)]
    pub const NONE: Self = Self(0);

    // `MS_*` is `c_ulong`, which is 64-bit on aarch64 and 32-bit on the armv6
    // the Pi Zero W would need — `AGENTS.md` §10.2's deferred compatibility
    // target. The cast is redundant on the target Rivet builds for today and
    // load-bearing on the one it might build for tomorrow, so it stays.
    #[allow(clippy::unnecessary_cast)]
    pub const NOEXEC: Self = Self(libc::MS_NOEXEC as u64);
    #[allow(clippy::unnecessary_cast)]
    pub const NOSUID: Self = Self(libc::MS_NOSUID as u64);
    #[allow(clippy::unnecessary_cast)]
    pub const NODEV: Self = Self(libc::MS_NODEV as u64);
    /// Not reachable from [`MountFlag`], because `stage0_plan()` has no
    /// read-only member. The boot partition is mounted with it — see
    /// [`crate::provision`].
    #[allow(clippy::unnecessary_cast)]
    pub const RDONLY: Self = Self(libc::MS_RDONLY as u64);
    /// `/state`'s, and only `/state`'s — `ADR-018` §7, where "every read
    /// otherwise becomes a write" on the one partition that takes all of an SD
    /// card's wear. See `rivet_core::state::STATE_FLAGS`.
    #[allow(clippy::unnecessary_cast)]
    pub const NOATIME: Self = Self(libc::MS_NOATIME as u64);

    /// Translate core's intent-named flags into this platform's `MS_*` bits.
    ///
    /// `ADR-014` §3 puts this mapping here and nowhere else. The match is
    /// deliberately exhaustive, so adding a [`MountFlag`] breaks this build
    /// rather than being silently dropped on the floor.
    pub fn from_core(flags: &[MountFlag]) -> Self {
        let mut bits = 0;
        for flag in flags {
            bits |= match flag {
                MountFlag::NoExec => Self::NOEXEC.0,
                MountFlag::NoSuid => Self::NOSUID.0,
                MountFlag::NoDev => Self::NODEV.0,
                MountFlag::NoAtime => Self::NOATIME.0,
            };
        }
        Self(bits)
    }

    /// Used by the tests to assert on what actually reached `mount(2)`; PID 1
    /// itself only ever passes the whole word through.
    #[allow(dead_code)]
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// How a child ended, decoded away from the platform's status word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Exited(i32),
    Signalled(i32),
    /// Neither exited nor signalled — stopped or continued, which PID 1 does
    /// not act on but should not silently mislabel either.
    Other(i32),
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Exited(code) => write!(f, "exit {code}"),
            Self::Signalled(sig) => write!(f, "killed by signal {sig}"),
            Self::Other(raw) => write!(f, "status {raw:#x}"),
        }
    }
}

/// The result of one non-blocking `waitpid(-1, …)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaped {
    Child {
        pid: i32,
        status: ExitStatus,
    },
    /// Children exist; none of them has exited.
    NoneReady,
}

/// The signals PID 1 acts on, named for what they mean here.
///
/// `ADR-017` §3.1.5 item 6 names `SIGTERM` only. [`Interrupt`](Self::Interrupt)
/// is included because the kernel delivers `SIGINT` to pid 1 for
/// Ctrl-Alt-Delete: ignoring it does not make the key combination safe, it
/// makes it silent. [`User1`](Self::User1) is `ADR-017` §3.1.10's addition of
/// 2026-08-23 — see its own documentation, which is the definition of what that
/// signal means on a Rivet machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sig {
    /// A child changed state.
    Child,
    /// Shut down and power off.
    Term,
    /// Shut down and reboot (Ctrl-Alt-Delete).
    Interrupt,
    /// **Shut down and restart into the firmware's one-shot trial slot.**
    ///
    /// # This is the whole contract, and nothing else may send it
    ///
    /// On a Rivet machine `SIGUSR1` to pid 1 means *"reboot into the slot that
    /// `rivet apply trial-boot` armed"*, and it means nothing else, ever. It
    /// goes through the same [`crate::pid1::Pid1`] shutdown path as the other
    /// two — stop the supervisor, sync, then ask the kernel — because a trial
    /// boot is a reboot, and a reboot that skipped stopping services and
    /// flushing the card would be a worse shutdown than the ordinary one.
    ///
    /// **Why this signal.** `SIGTERM` and `SIGINT` are taken (power off and
    /// Ctrl-Alt-Delete). `SIGHUP` conventionally means *reload configuration*,
    /// which this emphatically is not, and a system that overloaded it would
    /// eventually meet something that sends `SIGHUP` meaning the conventional
    /// thing. `SIGUSR1` has no convention to violate, which is the entire reason
    /// it exists in POSIX.
    ///
    /// **`SIGUSR1`'s default disposition is to terminate the process**, so the
    /// blocking in [`LibcKernel::arm_signals`] is not a nicety — it is what
    /// stops the signal that asks for a trial boot from killing pid 1 instead.
    /// It is blocked because it is in [`HANDLED_SIGNALS`], which
    /// `arm_signals` adds to the mask *before* it creates the descriptor; the
    /// tests below check the mask directly and then deliver a real one to a
    /// process that has no special protection. Linux additionally discards a
    /// signal sent to pid 1 whose disposition is `SIG_DFL`, but that is a
    /// backstop nobody should be relying on and it is **not** what makes this
    /// safe: it does not apply to a process that is not pid 1, which is what
    /// this binary is under QEMU's `init=` harness and in every test.
    User1,
    /// Only ever sent, never received: the escalation after `Term`.
    Kill,
    /// Something in the set arrived that this build does not act on.
    Other(i32),
}

impl Sig {
    pub fn raw(self) -> i32 {
        match self {
            Self::Child => libc::SIGCHLD,
            Self::Term => libc::SIGTERM,
            Self::Interrupt => libc::SIGINT,
            Self::User1 => libc::SIGUSR1,
            Self::Kill => libc::SIGKILL,
            Self::Other(n) => n,
        }
    }

    pub fn from_raw(n: i32) -> Self {
        match n {
            libc::SIGCHLD => Self::Child,
            libc::SIGTERM => Self::Term,
            libc::SIGINT => Self::Interrupt,
            libc::SIGUSR1 => Self::User1,
            libc::SIGKILL => Self::Kill,
            other => Self::Other(other),
        }
    }
}

impl core::fmt::Display for Sig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Child => write!(f, "SIGCHLD"),
            Self::Term => write!(f, "SIGTERM"),
            Self::Interrupt => write!(f, "SIGINT"),
            Self::User1 => write!(f, "SIGUSR1"),
            Self::Kill => write!(f, "SIGKILL"),
            Self::Other(n) => write!(f, "signal {n}"),
        }
    }
}

/// The signals PID 1 blocks and reads, in the order they are reported.
///
/// **Blocking and handling are the same list on purpose.** A member of this set
/// that the loop does not dispatch on is a request nothing answers; a signal the
/// loop dispatches on that is not in this set is worse — with `SIGUSR1`'s
/// default disposition it is a handler for a signal that kills the process
/// before it can run, which reads as working right up until somebody sends one.
pub const HANDLED_SIGNALS: &[Sig] = &[Sig::Child, Sig::Term, Sig::Interrupt, Sig::User1];

/// The argument string that asks the Raspberry Pi firmware for a one-shot
/// trial boot. `ADR-018` §5.3.
///
/// **Raspberry Pi document this as `sudo reboot "0 tryboot"`, and the quotes
/// are not part of it.** That spelling is *systemd's* CLI — one argument string
/// handed to `reboot(2)` — and §5.3 records that systemd 257 rejects it outright
/// with "Too many arguments", which is how a documented incantation turns out to
/// be a property of the shutdown tool rather than of the firmware. Rivet has
/// no systemd and passes the string to the syscall itself, so what matters here
/// is only the string.
///
/// **Not confirmed by Rivet on a board.** §5.3's end-to-end proof was on
/// stock Raspberry Pi OS via `systemctl reboot --reboot-argument=…`. Nothing in
/// this repository has ever asked a board for a tryboot reboot, and `ADR-026`
/// §4 puts `tryboot` in the column QEMU cannot reach at all.
pub const TRYBOOT_ARGUMENT: &str = "0 tryboot";

/// What to ask the kernel to do with the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebootHow {
    PowerOff,
    Restart,
    /// Restart into the firmware's **one-shot** trial slot — `ADR-018` §5.1.
    ///
    /// ~~Not a variety of shutdown, which is why it does not appear anywhere in
    /// [`crate::pid1`]'s signal handling.~~ **Corrected 2026-08-23**, and struck
    /// rather than deleted because `AGENTS.md` §7 is about exactly this: it is
    /// the *arm* half of an A/B update, and the **restart** it asks for is a
    /// shutdown like any other. `ADR-017` §3.1.10 rules that pid 1 performs it
    /// on `SIGUSR1`, through the same `shut_down` path as `PowerOff` and
    /// `Restart` — stop the supervisor, sync, then ask the kernel. See
    /// [`Sig::User1`].
    ///
    /// The reason it lives on this port rather than beside the update code is
    /// that it is a syscall: everything below this seam is.
    ///
    /// The flag it sets is the firmware's and it clears itself. If the slot it
    /// boots never commits, the next reset returns to the previous one with no
    /// userspace involved at all — which is the whole of why `ADR-018` §5 is a
    /// good mechanism and the property the x86 boot-slot port's survey judged
    /// any substitute has to have.
    ///
    /// ~~**Constructed only by [`crate::slots::reboot_for`], which has no
    /// production caller yet**~~ — **stale since 2026-08-23; the allow is gone
    /// because [`crate::pid1`] constructs it on `SIGUSR1`.**
    /// [`crate::slots::reboot_for`] is still uncalled and keeps its own allow:
    /// it is the mapping from `rivet_adapter_image::ArmedReboot`, the port's
    /// vocabulary for *which reboot carries an arming*, and the caller for
    /// **that** is the update path, which is not built. The port deliberately
    /// does not name this enum, because `PowerOff` and `Restart` are PID 1's
    /// shutdown handling (`ADR-017` §3.1.5 item 6) and an image adapter has no
    /// business owning them.
    ///
    /// **It has still never executed on anything.** A board is what proves it:
    /// `ADR-026` §4 puts `tryboot` in the column QEMU does not emulate at all,
    /// so a green suite says the decisions are right given a kernel that answers
    /// as scripted, and nothing more.
    TrialRestart,
}

impl core::fmt::Display for RebootHow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PowerOff => write!(f, "power off"),
            Self::Restart => write!(f, "reboot"),
            Self::TrialRestart => write!(f, "reboot into the trial slot"),
        }
    }
}

/// Every syscall PID 1 makes, as a port.
///
/// Object-safe on purpose: the logic takes `&mut dyn Kernel`, so there is one
/// compiled copy of the decisions and the fake drives exactly the same code the
/// board runs.
/// What [`Kernel::quiet_console`] managed to take.
///
/// Three independent halts, reported independently: an operator reading the
/// journal after a boot with a blank-looking screen needs to know WHICH of them
/// worked, because the ones that did not are the ones still painting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConsoleQuiet {
    /// The `vtconsole` unbound from the framebuffer, named — `vtcon1` on this
    /// board. `None` means none was found or the unbind was refused, and then
    /// the console driver still owns the screen.
    pub fbcon_unbound: Option<String>,
    /// Whether the kernel's console loglevel was lowered, which is what stops
    /// kernel messages and pid 1's own `/dev/kmsg` lines being PRINTED. The
    /// ring buffer still receives them either way, so nothing is lost.
    pub printk_lowered: bool,
    /// Whether pid 1's stdout and stderr now go to `/dev/null`. This is the one
    /// that silences the SUPERVISOR, which inherits them across the spawn.
    pub output_discarded: bool,
}

impl core::fmt::Display for ConsoleQuiet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "fbcon={} printk={} output={}",
            self.fbcon_unbound.as_deref().unwrap_or("not unbound"),
            if self.printk_lowered {
                "lowered"
            } else {
                "unchanged"
            },
            if self.output_discarded {
                "discarded"
            } else {
                "still on the console"
            },
        )
    }
}

pub trait Kernel {
    /// This process's own pid. PID 1's reasoning only applies when this is 1.
    fn self_pid(&self) -> u32;

    /// Report a line to the console. PID 1 has nowhere to report a failed
    /// report to, so this cannot fail.
    fn say(&mut self, line: &str);

    /// Milliseconds on a monotonic clock with an arbitrary zero.
    ///
    /// Monotonic rather than wall-clock deliberately: every use is a *duration*
    /// — backoff, a settle time, a shutdown grace period — and `ADR-021` has
    /// the wall clock arriving late and jumping when it does.
    fn now_ms(&mut self) -> u64;

    /// Create a directory and every missing parent. Succeeds if it is already a
    /// directory; fails if the path exists as something else.
    fn mkdir_p(&mut self, path: &str) -> Result<(), Errno>;

    fn mount(
        &mut self,
        source: &str,
        target: &str,
        fstype: &str,
        flags: MountFlags,
        data: Option<&str>,
    ) -> Result<(), Errno>;

    fn umount(&mut self, target: &str) -> Result<(), Errno>;

    /// Take `path`'s exclusive advisory lock (`flock(2)`) **without waiting**,
    /// creating the file mode `0600` and its directory if they are missing.
    ///
    /// [`Errno::EAGAIN`] if another process holds it. **Never blocks**, because
    /// PID 1's loop is what reaps children and takes shutdown signals; a
    /// caller that wants the lock later asks again from the loop. Held until
    /// [`Self::unlock`].
    fn try_lock(&mut self, path: &str) -> Result<(), Errno>;

    /// Release a lock [`Self::try_lock`] took, at once — even while a child
    /// forked meanwhile still holds a copy of the descriptor. Cannot fail:
    /// the lock is released explicitly and the descriptor is then closed.
    fn unlock(&mut self, path: &str);

    fn read_file(&mut self, path: &str) -> Result<String, Errno>;

    /// Bytes, for files that are not text.
    ///
    /// `/proc/device-tree/*` properties are the reason: a device-tree property
    /// may be a NUL-terminated string or a big-endian cell, and deciding which
    /// is a decision, so it happens above this seam.
    fn read_bytes(&mut self, path: &str) -> Result<Vec<u8>, Errno>;

    /// How many bytes are in a file, without reading any of them.
    ///
    /// **Asked before a read that will be bounded by a cap**, so an oversized
    /// file on the boot partition is refused by `stat(2)` rather than
    /// allocated and then rejected. `Err(Errno::ENOENT)` if it is not there.
    fn file_size(&mut self, path: &str) -> Result<u64, Errno>;

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno>;

    /// Create a directory only its owner may enter — mode `0700`.
    ///
    /// **Added 2026-08-28 for [`crate::identity`]**, and it is the second
    /// method this port has grown for a single job (`remove_file` was the
    /// first, and says so). `mkdir_p` leaves the mode to whatever umask PID 1
    /// inherited from the kernel, and a directory holding a private key may not
    /// be left to that.
    ///
    /// `ADR-020` §9.2 rules the device TLS seed a secret and §9.2.1 closes the
    /// list of them, so `/state/secrets` is `0700` and is created here rather
    /// than inheriting a mode from whatever the kernel handed PID 1.
    fn mkdir_private(&mut self, path: &str) -> Result<(), Errno>;

    /// Publish a file only its owner may read — mode `0600`, **set in the
    /// open** — and atomically: a reader of `path` sees either the whole of
    /// `contents` or nothing this call did, never a partial write.
    ///
    /// Set in the `open` and never by a later `chmod`, which is
    /// `cmd/cargo-rivet`'s `key.rs` argument and applies here word for word: a
    /// `chmod` after the fact leaves a window in which the file exists and is
    /// readable by anything.
    ///
    /// **Atomic because a Pi has no battery and usually an SD card**, so the
    /// process may be cut off between any two of its own steps. The bytes go
    /// to a temporary file beside `path`, created `O_EXCL` so a leftover
    /// temporary from an earlier interrupted call is never opened and written
    /// through, then flushed and renamed over `path`, then the directory is
    /// flushed so the rename itself survives a power cut. A caller that reads
    /// `path` back to decide whether it is already held therefore sees either
    /// the complete file this call published or none of it — never the
    /// fragment a plain create-and-truncate would leave at that path.
    ///
    /// **Not `write_file` with an extra argument.** A mode parameter on the
    /// ordinary writer would make every caller state a mode, and every caller
    /// but this one has no opinion about it — which is how a default gets
    /// chosen by whoever wrote the call rather than by anybody deciding.
    fn write_secret(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno>;

    /// Overwrite a file's existing bytes **without truncating it first**.
    ///
    /// # Why this is not `write_file`
    ///
    /// `write_file` is `std::fs::write`, which truncates. **On FAT, truncating
    /// frees the file's cluster chain**, and the write that follows is free to
    /// allocate a different cluster — leaving the original data intact and
    /// orphaned in free space. That is not a theory: `crate::identity` shipped
    /// a zero-fill built on `write_file` and the board still had its seed at
    /// offset 13647872 of the boot partition afterwards, with the zeros
    /// somewhere else entirely.
    ///
    /// Opening without truncation keeps the cluster chain, so the zeros land on
    /// the bytes that held the secret. `contents` must be the file's existing
    /// length; anything shorter leaves a tail.
    fn overwrite_in_place(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno>;

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno>;

    /// Remove a file.
    ///
    /// Added 2026-08-23 for [`crate::seed`]: `rivet-store` clears the
    /// temporary file of a save that failed, because on a full `/state` that
    /// file is the thing occupying the space. Nothing else in PID 1 deletes
    /// anything, and nothing should — this is not a general facility and the one
    /// caller is a best-effort cleanup of a path PID 1 itself just wrote.
    fn remove_file(&mut self, path: &str) -> Result<(), Errno>;

    /// Entry names only, not paths.
    fn read_dir(&mut self, path: &str) -> Result<Vec<String>, Errno>;

    /// **`len` bytes from `getrandom(2)`**, which blocks until the kernel's
    /// pool is ready rather than returning weak bytes — `ADR-063` §4, the
    /// one thing PID 1 draws them for being a fleet's root.
    ///
    /// **The default refuses**, so a test double that says nothing about
    /// randomness cannot hand [`crate::fleet_root`] bytes it would mistake for
    /// random ones. [`LibcKernel`] overrides it.
    fn random_bytes(&mut self, len: usize) -> Result<Vec<u8>, Errno> {
        let _ = len;
        Err(Errno(libc::ENOSYS))
    }

    /// Seconds since the epoch on the wall clock — **which on a first boot is
    /// the epoch**, because a Raspberry Pi has no real-time clock (`ADR-021`).
    /// Read for a certificate's validity window and nothing else; the
    /// minting crate floors it. The default is the epoch.
    fn unix_time_secs(&mut self) -> u64 {
        0
    }

    /// The kernel's mount table.
    ///
    /// A default method rather than a separate syscall, so the real and the
    /// fake reach it by the same route and this body is covered by the tests.
    /// **`Err` here is the normal state of a first boot**, before `/proc`
    /// itself is mounted — see [`crate::boot::perform_mount`].
    fn read_mounts(&mut self) -> Result<String, Errno> {
        self.read_file(MOUNTS_PATH)
    }

    /// Block [`HANDLED_SIGNALS`] and route them to a descriptor. Idempotent.
    fn arm_signals(&mut self) -> Result<(), Errno>;

    /// Wait up to `timeout_ms` for one of [`HANDLED_SIGNALS`].
    ///
    /// `Ok(None)` means the deadline passed with nothing delivered — which is
    /// how the loop wakes up to restart a supervisor whose backoff has expired.
    /// Only meaningful after [`Self::arm_signals`].
    fn await_signal(&mut self, timeout_ms: u64) -> Result<Option<Sig>, Errno>;

    /// **Read whatever the console has RIGHT NOW, without blocking** —
    /// `ADR-035` §8A.1.2's break-glass reader.
    ///
    /// `Ok(0)` means nothing was waiting, which is the answer on almost every
    /// tick and must be cheap: pid 1 asks this every pass, and a version that
    /// blocked would stop the machine's supervisor being restarted for as long
    /// as nobody typed.
    ///
    /// **The console is shared with [`Kernel::say`]**, which writes to it.
    /// That is deliberate and is what makes one serial cable enough.
    fn read_console(&mut self, into: &mut [u8]) -> Result<usize, Errno>;

    fn sleep_ms(&mut self, ms: u64);

    /// Non-blocking `waitpid(-1, …)`.
    fn waitpid_any(&mut self) -> Result<Reaped, Errno>;

    /// Start a program. Returns its pid; the child is **not** waited on here,
    /// because PID 1's own reaper owns every exit on the system.
    fn spawn(&mut self, path: &str) -> Result<i32, Errno>;

    /// Start a program **with arguments**.
    ///
    /// Added 2026-08-30 for the boot splash, which is the first thing pid 1
    /// starts that needs to be told anything. The supervisor takes none — its
    /// configuration is the document it reads for itself — so [`Kernel::spawn`]
    /// stays as it is rather than growing an empty slice at its one call site.
    fn spawn_with(&mut self, path: &str, args: &[String]) -> Result<i32, Errno>;

    /// **Take the framebuffer away from the kernel console.**
    ///
    /// Added 2026-08-30, and it exists because drawing the splash last was not
    /// enough. Three writers paint `/dev/fb0` on this machine and all three
    /// carry on long after the picture is drawn: the kernel's own console, pid
    /// 1 through [`Kernel::say`], and the supervisor's record echo on the
    /// stdout it inherits from here. A capture of the television at the first
    /// attempt showed the shield gone under `mcpd` connection records at eight
    /// seconds and `not a tryboot boot` at thirty-three.
    ///
    /// So the splash does not merely go last — it TAKES the screen. The three
    /// halts are complementary and none of them subsumes another, which is why
    /// all three are attempted and each is reported separately.
    ///
    /// **Best effort, by construction.** Every step here is a nicety on a
    /// machine whose job is to boot; a kernel that will not unbind its console
    /// is a plainer screen and never a reason to stop. What was achieved comes
    /// back so the caller can say so in the journal.
    fn quiet_console(&mut self) -> ConsoleQuiet;

    /// Signal one process. Refuses a non-positive pid — see the module docs.
    fn kill(&mut self, pid: i32, signal: Sig) -> Result<(), Errno>;

    /// Flush filesystem buffers. A Pi has no battery and usually an SD card.
    fn sync(&mut self);

    /// Ask the kernel to reboot or power off.
    ///
    /// On success the machine goes away and this never returns — but it is
    /// typed as returning, because the caller must handle the case where the
    /// kernel refuses, and because a test cannot make a machine go away.
    fn reboot(&mut self, how: RebootHow) -> Result<(), Errno>;

    /// Fork a child that exits immediately with `code`. Returns its pid.
    fn fork_child_exiting_with(&mut self, code: i32) -> Result<i32, Errno>;

    /// Fork a child that forks again and exits at once, so that *its* child is
    /// orphaned onto pid 1 while still alive. Returns the intermediate pid; the
    /// orphan's pid is deliberately unknown to the caller, which is the point.
    fn fork_orphan_maker(&mut self, delay_ms: u64, code: i32) -> Result<i32, Errno>;
}

// ---------------------------------------------------------------------------
// The real one. Thin by policy: nothing here is reachable from a test.
// ---------------------------------------------------------------------------

use std::ffi::CString;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _};

/// `reboot(2)` with `LINUX_REBOOT_CMD_RESTART2` and an argument string.
///
/// # Why this is not `libc::reboot`
///
/// **It cannot be.** The `libc` crate's Linux binding is
/// `reboot(how_to: c_int) -> c_int` — one argument — because that is what the
/// glibc *and* musl wrappers are. musl's is literally
/// `syscall(SYS_reboot, MAGIC1, MAGIC2, type)` with no fourth word, so there is
/// no C entry point on this target that can carry `"0 tryboot"`. Every
/// implementation of `reboot "<arg>"` therefore makes the syscall directly, and
/// so does this. Reported here rather than left as a surprise, because the
/// obvious reading of `ADR-018` §5.3 is that `libc::reboot` grows an argument.
///
/// A free function rather than a method: it takes nothing from `LibcKernel` and
/// the module docs ask this half of the crate to stay as thin as possible.
fn reboot_with_argument(arg: &str) -> Result<(), Errno> {
    let Ok(arg) = CString::new(arg) else {
        // A NUL inside the argument. `TRYBOOT_ARGUMENT` has none, so this is
        // unreachable today and is still not an `unwrap`: PID 1 does not panic.
        return Err(Errno::EINVAL);
    };
    // SAFETY: `syscall(2)` is variadic and the kernel reads exactly the four
    // words `SYS_reboot` documents: two magic `int`s, the command `int`, and a
    // pointer. `arg` is a NUL-terminated `CString` owned by this frame and
    // alive across the whole call, and the kernel copies the string in
    // (`copy_from_user`) before returning, so nothing of ours is retained. The
    // three constants are the kernel's own and cannot be invalid values. On
    // success the machine restarts and this does not return; a return means it
    // failed and set errno.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_reboot,
            libc::LINUX_REBOOT_MAGIC1,
            libc::LINUX_REBOOT_MAGIC2,
            libc::LINUX_REBOOT_CMD_RESTART2,
            arg.as_ptr(),
        )
    };
    if rc == 0 { Ok(()) } else { Err(Errno::last()) }
}

/// The kernel, via `libc`.
pub struct LibcKernel {
    sigfd: Option<File>,
    started: std::time::Instant,
    /// The serial line break-glass reads — `ADR-035` §8A.1.
    ///
    /// **Opened once and kept**, because opening a tty per tick would be a
    /// syscall storm in pid 1's loop, and because the open is where the
    /// decision about WHICH device lives.
    breakglass: Option<BreakGlassLine>,
    /// Advisory locks held, by path — `Kernel::try_lock`. Removing an entry
    /// closes its descriptor, which is what releases the lock.
    locks: std::collections::BTreeMap<String, File>,
}

impl LibcKernel {
    pub fn new() -> Self {
        Self {
            sigfd: None,
            started: std::time::Instant::now(),
            breakglass: None,
            locks: std::collections::BTreeMap::new(),
        }
    }
}

/// Which line break-glass listens on, resolved once.
///
/// # Why this is not descriptor 0
///
/// **Because descriptor 0 is the wrong console, and the board said so.**
/// `ADR-035` §8A.1 describes Tier 1 as *"MCP over the serial console … with a
/// physical cable"*, and the first implementation read fd 0 — which is
/// `/dev/console`. On 2026-08-30 the board was asked what that is:
///
/// ```text
/// tty1                 -WU (EC  p  )    4:1
/// ttyS0                -W- (E   p a)    4:64
/// ```
///
/// `tty1` carries `C`, so `/dev/console` is **HDMI**. Break-glass was listening
/// to a USB keyboard and a serial cable would have found nothing — the security
/// premise survived (a keyboard at the machine is physical presence too) and
/// the description did not.
///
/// # Why it is resolved rather than hard-coded
///
/// `crate::boot::cmdline_txt` writes `console=serial0`, a **device-tree alias**
/// the kernel resolves — which is what keeps that line correct whether the port
/// is the PL011 or the mini-UART. Hard-coding `/dev/ttyS0` here would throw that
/// away and be wrong on the first board that resolves the alias differently.
///
/// So this asks `/proc/consoles`, which is the kernel reporting what it
/// actually enabled, and takes the first entry that is not a virtual terminal.
enum BreakGlassLine {
    /// The serial device, open.
    Open(File),
    /// No serial console was found and the reason was said once. Break-glass is
    /// unavailable; `ADR-035` §8A's other tiers are what remain.
    Unavailable,
}

/// What the kernel says it enabled — `/proc/consoles`, one console per line.
const CONSOLES: &str = "/proc/consoles";

/// The serial console's device path, or `None` when the kernel enabled none.
///
/// A virtual terminal (`tty0`, `tty1`, …) is not a serial line: it is the
/// screen, and reading it would make break-glass a keyboard interface again.
/// Unbind whichever `vtconsole` is driving the framebuffer, and name it.
///
/// **The console is found by ASKING, never by assuming `vtcon1`.** The index
/// depends on which console drivers registered, and a board that gained or lost
/// one would have this write to the wrong device — `AGENTS.md` §5.
/// `/sys/class/vtconsole/vtcon*/name` reads `frame buffer device` for the one
/// that matters and `dummy device` for the placeholder.
///
/// This is the halt that covers everything at once: with fbcon unbound, nothing
/// writing to a terminal reaches the screen at all, whatever it is and whoever
/// writes it.
fn unbind_framebuffer_console() -> Option<String> {
    const VTCONSOLE: &str = "/sys/class/vtconsole";
    let entries = std::fs::read_dir(VTCONSOLE).ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let Ok(kind) = std::fs::read_to_string(dir.join("name")) else {
            continue;
        };
        if !kind.contains("frame buffer") {
            continue;
        }
        // A `0` here is the documented unbind. It can be refused — a kernel
        // built without the binding sysfs attribute has no such file — and a
        // refusal is answered by leaving the console where it is.
        if std::fs::write(dir.join("bind"), b"0").is_ok() {
            return Some(name);
        }
    }
    None
}

/// Lower the loglevel at which the kernel PRINTS to the console.
///
/// The four numbers are console, default, minimum and boot-time default. Only
/// the first is being changed, and it is set to 1 so that `KERN_EMERG` and
/// `KERN_ALERT` still reach a screen — a machine dying should say so over a
/// logo.
///
/// **Nothing is lost by this.** The kernel's ring buffer receives every message
/// regardless of the console loglevel, so pid 1's `/dev/kmsg` writes still land
/// and the supervisor still ingests them (`ADR-022` §4.1). This changes what is
/// PRINTED and not what is RECORDED.
fn lower_console_loglevel() -> bool {
    std::fs::write(
        "/proc/sys/kernel/printk",
        b"1	4	1	7
",
    )
    .is_ok()
}

/// Point pid 1's own stdout and stderr at `/dev/null`.
///
/// **This is the one that silences the supervisor**, which is the loudest of
/// the three writers: `cmd/rivet-sysd/src/kernel.rs`'s `emit` writes a JSON
/// record to stdout for every journal entry, and it inherits that stdout from
/// here across `spawn`. Redirecting the descriptors rather than asking the
/// supervisor to be quiet keeps the decision in the one process that knows
/// whether this boot has a splash on it.
///
/// [`Kernel::say`] keeps working and keeps recording: its `/dev/kmsg` write is
/// a separate descriptor and is not touched.
fn discard_own_output() -> bool {
    let Ok(null) = std::fs::File::options().write(true).open("/dev/null") else {
        return false;
    };
    // Anything buffered belongs on the console it was written for, before the
    // descriptor underneath it changes.
    let _ = std::io::stdout().flush();
    let fd = null.as_raw_fd();
    // SAFETY: `fd` is open for writing for the lifetime of `null`, and 1 and 2
    // are pid 1's own descriptors. `dup2` closes the old target itself.
    let ok = unsafe { libc::dup2(fd, 1) >= 0 && libc::dup2(fd, 2) >= 0 };
    ok
}

fn serial_console_device() -> Option<String> {
    let text = std::fs::read_to_string(CONSOLES).ok()?;
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .find(|name| {
            name.starts_with("tty")
                && !name["tty".len()..].starts_with(|c: char| c.is_ascii_digit())
        })
        .map(|name| format!("/dev/{name}"))
}

impl Kernel for LibcKernel {
    fn self_pid(&self) -> u32 {
        std::process::id()
    }

    fn say(&mut self, line: &str) {
        // `writeln!` on a closed console must not become the reason the kernel
        // panics, so both results are dropped deliberately.
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();

        // **And into the kernel's ring buffer, so the line is not lost with the
        // console** — added 2026-08-30, `PLAN.md` §102.17.
        //
        // `cmd/rivet-sysd/src/kmsg.rs` said pid 1's lines *"have nowhere to go
        // by construction"*, and a real update proved the cost: a trial boot
        // declined to commit, `crate::pid1::maybe_confirm` said why through
        // this function, and on a headless board over TLS that answer existed
        // only on a serial cable. Two reboots learned one bit.
        //
        // **This does NOT break `ADR-017` §8 item 2**, which is why it is
        // `/dev/kmsg` and not the journal. That item rules the journal is part
        // of the SUPERVISOR, so pid 1 must never depend on the supervisor to
        // record anything — otherwise the message *the supervisor failed* is
        // the one that cannot be written. The kernel's ring buffer is not the
        // supervisor: this write needs nothing running, and the supervisor
        // INGESTS it afterwards if and when it starts (`ADR-022` §4.1).
        //
        // **Best effort, and every failure is dropped.** `/dev/kmsg` does not
        // exist in the QEMU harness at all (`PLAN.md`'s three defects from
        // assuming it does), a read-only or absent `/dev` is an ordinary state
        // early in boot, and none of that is a reason for pid 1 to stop or to
        // stop saying things on the console.
        if let Ok(mut kmsg) = std::fs::OpenOptions::new()
            .write(true)
            .open(rivet_core::state::KMSG)
        {
            // One `write` per record: the interface is message-oriented and a
            // line split across two writes becomes two records.
            let _ = kmsg.write_all(
                format!(
                    "{line}
"
                )
                .as_bytes(),
            );
        }
    }

    fn now_ms(&mut self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn read_console(&mut self, into: &mut [u8]) -> Result<usize, Errno> {
        // Resolve the line on the first call. Doing it here rather than in
        // `new` keeps pid 1's construction free of I/O, and `/proc` is not
        // mounted yet when the kernel is built.
        if self.breakglass.is_none() {
            self.breakglass = Some(match serial_console_device() {
                Some(path) => match std::fs::OpenOptions::new().read(true).open(&path) {
                    Ok(file) => {
                        self.say(&format!(
                            "rivet-initd: break-glass listening on {path} (ADR-035 §8A.1)"
                        ));
                        BreakGlassLine::Open(file)
                    }
                    Err(e) => {
                        self.say(&format!(
                            "rivet-initd: break-glass is UNAVAILABLE: {path} could not be opened                              ({e}). ADR-035 §8A's other tiers are the boot partition and \
                             rewriting the card"
                        ));
                        BreakGlassLine::Unavailable
                    }
                },
                None => {
                    self.say(
                        "rivet-initd: break-glass is UNAVAILABLE: the kernel enabled no serial \
                         console, so there is no line to listen on. ADR-035 §8A's other tiers \
                         are the boot partition and rewriting the card",
                    );
                    BreakGlassLine::Unavailable
                }
            });
        }

        let Some(BreakGlassLine::Open(file)) = self.breakglass.as_ref() else {
            // No line. Not an error: a machine without a serial console is a
            // machine with two break-glass tiers instead of three, and pid 1
            // has already said so once.
            return Ok(0);
        };

        // **`poll` with a zero timeout, then read.** The alternative is setting
        // `O_NONBLOCK` on the descriptor, which is a lasting change to a device
        // shared with the kernel's own console output. Asking whether a read
        // would block is a question; changing the device is a side effect.
        let mut pfd = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(file),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pfd, 1, 0) };
        if ready < 0 {
            return Err(Errno::last());
        }
        if ready == 0 || pfd.revents & libc::POLLIN == 0 {
            return Ok(0);
        }
        let got = unsafe {
            libc::read(
                std::os::fd::AsRawFd::as_raw_fd(file),
                into.as_mut_ptr().cast(),
                into.len(),
            )
        };
        if got < 0 {
            return Err(Errno::last());
        }
        Ok(usize::try_from(got).unwrap_or(0))
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), Errno> {
        // `create_dir_all` is already idempotent for a directory that exists;
        // a path that exists as a *file* is an error, and which error it is is
        // the platform's answer, not ours to assume.
        std::fs::create_dir_all(path).map_err(|e| Errno::from_io(&e))
    }

    fn mount(
        &mut self,
        source: &str,
        target: &str,
        fstype: &str,
        flags: MountFlags,
        data: Option<&str>,
    ) -> Result<(), Errno> {
        let (Ok(source), Ok(target), Ok(fstype)) = (
            CString::new(source),
            CString::new(target),
            CString::new(fstype),
        ) else {
            return Err(Errno::EINVAL);
        };
        let data = match data.map(CString::new) {
            None => None,
            Some(Ok(c)) => Some(c),
            Some(Err(_)) => return Err(Errno::EINVAL),
        };
        let data_ptr = data
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr())
            .cast::<libc::c_void>();

        // SAFETY: `mount(2)` reads four borrowed pointers and returns an int; it
        // retains none of them. `source`, `target` and `fstype` are
        // NUL-terminated and owned by `CString`s alive for the whole call.
        // `data_ptr` is either NULL — which mount(2) documents as "no
        // filesystem-specific options" — or borrowed from `data`, which is
        // likewise alive here. `flags` is a bitmask of MS_* constants and
        // cannot be an invalid value.
        let rc = unsafe {
            libc::mount(
                source.as_ptr(),
                target.as_ptr(),
                fstype.as_ptr(),
                flags.0 as libc::c_ulong,
                data_ptr,
            )
        };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn umount(&mut self, target: &str) -> Result<(), Errno> {
        let Ok(target) = CString::new(target) else {
            return Err(Errno::EINVAL);
        };
        // SAFETY: `umount2(2)` reads one NUL-terminated borrowed pointer, owned
        // by a `CString` alive for the whole call, and retains nothing. A zero
        // flag word is the plain unmount, not one of the lazy or forced modes.
        let rc = unsafe { libc::umount2(target.as_ptr(), 0) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn try_lock(&mut self, path: &str) -> Result<(), Errno> {
        use std::os::unix::fs::OpenOptionsExt as _;
        use std::os::unix::io::AsRawFd as _;

        if self.locks.contains_key(path) {
            return Ok(());
        }
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| Errno::from_io(&e))?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|e| Errno::from_io(&e))?;
        loop {
            // SAFETY: `flock(2)` takes a descriptor and an integer operation and
            // touches no memory belonging to this process. The descriptor is
            // owned by `file`, alive for the whole call; the operation is two
            // `LOCK_*` constants, and `LOCK_NB` is what makes it never block.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                self.locks.insert(path.to_string(), file);
                return Ok(());
            }
            match Errno::last() {
                Errno::EINTR => {}
                // `EWOULDBLOCK` is `EAGAIN` on Linux; named as the one this
                // port's callers match on.
                Errno(e) if e == libc::EWOULDBLOCK => return Err(Errno::EAGAIN),
                other => return Err(other),
            }
        }
    }

    fn unlock(&mut self, path: &str) {
        use std::os::unix::io::AsRawFd as _;
        // **`LOCK_UN` before the close, not the close alone.** `flock(2)`
        // belongs to the open file description, and a `fork` by any thread of
        // this process duplicates the descriptor onto it: closing this copy
        // would leave the lock held until the child closed its own at `exec`.
        // `LOCK_UN` releases it on the description, whoever else holds a copy.
        if let Some(file) = self.locks.remove(path) {
            // SAFETY: `flock(2)` takes a descriptor owned by `file`, alive for
            // the whole call, and an integer operation; it touches no memory.
            // A failure leaves the close below to release it as before.
            let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        std::fs::read_to_string(path).map_err(|e| Errno::from_io(&e))
    }

    fn read_bytes(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        std::fs::read(path).map_err(|e| Errno::from_io(&e))
    }

    fn file_size(&mut self, path: &str) -> Result<u64, Errno> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| Errno::from_io(&e))
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        std::fs::write(path, contents).map_err(|e| Errno::from_io(&e))
    }

    fn mkdir_private(&mut self, path: &str) -> Result<(), Errno> {
        use std::os::unix::fs::DirBuilderExt as _;
        match std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
        {
            Ok(()) => Ok(()),
            Err(e) => Err(Errno::from_io(&e)),
        }
    }

    fn overwrite_in_place(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        use std::io::Write as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|e| Errno::from_io(&e))?;
        file.write_all(contents).map_err(|e| Errno::from_io(&e))?;
        file.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn write_secret(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        // Atomic, private, durable publication -- B12. `path` must never show
        // a partial file, so the bytes are written beside it first and made
        // visible in one `rename(2)`.
        let temp = format!("{path}.new");

        // O_EXCL: a leftover temporary from an interrupted earlier call is
        // never opened and written through -- it is removed once and this
        // call starts clean, so a stale temp can never be mistaken for the
        // bytes this call is about to publish.
        let open_temp = || {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)
        };
        let mut file = match open_temp() {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&temp).map_err(|e| Errno::from_io(&e))?;
                open_temp().map_err(|e| Errno::from_io(&e))?
            }
            Err(e) => return Err(Errno::from_io(&e)),
        };
        file.write_all(contents).map_err(|e| Errno::from_io(&e))?;
        // Flushed before the rename that makes it visible under `path` — an
        // un-flushed rename can point a directory entry at blocks whose write
        // is still pending, and a power cut then shows a file that is
        // present and the right length but not these bytes.
        file.sync_all().map_err(|e| Errno::from_io(&e))?;
        drop(file);

        std::fs::rename(&temp, path).map_err(|e| Errno::from_io(&e))?;

        // The rename is a directory-entry change; fsync the DIRECTORY so the
        // new name survives a power cut and not only the bytes it points at.
        let dir = std::path::Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let dir_handle = std::fs::File::open(dir).map_err(|e| Errno::from_io(&e))?;
        dir_handle.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        std::fs::rename(from, to).map_err(|e| Errno::from_io(&e))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), Errno> {
        std::fs::remove_file(path).map_err(|e| Errno::from_io(&e))
    }

    fn read_dir(&mut self, path: &str) -> Result<Vec<String>, Errno> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| Errno::from_io(&e))? {
            let entry = entry.map_err(|e| Errno::from_io(&e))?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }

    /// `getrandom(2)` with no flags: it blocks until the pool is initialised.
    /// A syscall rather than `/dev/urandom`, because it needs no device node.
    fn random_bytes(&mut self, len: usize) -> Result<Vec<u8>, Errno> {
        let mut out = vec![0u8; len];
        let mut filled = 0;
        while filled < len {
            let rest = &mut out[filled..];
            // SAFETY: `rest` is a live, writable buffer of exactly `rest.len()`
            // bytes, and `getrandom` writes at most that many.
            let got = unsafe { libc::getrandom(rest.as_mut_ptr().cast(), rest.len(), 0) };
            if got < 0 {
                let errno = Errno::last();
                if errno == Errno::EINTR {
                    continue;
                }
                return Err(errno);
            }
            filled += usize::try_from(got).unwrap_or(0);
        }
        Ok(out)
    }

    fn unix_time_secs(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn arm_signals(&mut self) -> Result<(), Errno> {
        if self.sigfd.is_some() {
            return Ok(());
        }
        // SAFETY: `set` is a `sigset_t` owned by this frame and lives across
        // every call that borrows it. `sigemptyset` and `sigaddset` only write
        // through that pointer. `pthread_sigmask` reads it and writes nothing
        // (the old-set pointer is NULL, which it documents as "do not report
        // the old mask"). `signalfd(-1, …)` reads the set and creates a new
        // descriptor rather than modifying an existing one; the returned fd is
        // owned by nothing else, so the `File` is its sole owner and cannot
        // double-close. Zeroed is a valid empty `sigset_t` on Linux, and
        // `sigemptyset` re-establishes that regardless.
        let fd = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            if libc::sigemptyset(&mut set) != 0 {
                return Err(Errno::last());
            }
            for sig in HANDLED_SIGNALS {
                if libc::sigaddset(&mut set, sig.raw()) != 0 {
                    return Err(Errno::last());
                }
            }
            if libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
                return Err(Errno::last());
            }
            let fd = libc::signalfd(-1, &set, libc::SFD_CLOEXEC);
            if fd < 0 {
                return Err(Errno::last());
            }
            File::from_raw_fd(fd)
        };
        self.sigfd = Some(fd);
        Ok(())
    }

    fn await_signal(&mut self, timeout_ms: u64) -> Result<Option<Sig>, Errno> {
        let Some(file) = self.sigfd.as_mut() else {
            return Err(Errno::EINVAL);
        };
        let raw_fd = file.as_raw_fd();
        let timeout = i32::try_from(timeout_ms).unwrap_or(i32::MAX);
        let mut pfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll(2)` reads and writes exactly the one `pollfd` this
        // frame owns, for the `1` we tell it, and retains nothing. The fd is
        // borrowed from `self.sigfd`, which outlives the call.
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout) };
        if rc < 0 {
            return Err(Errno::last());
        }
        if rc == 0 {
            return Ok(None);
        }
        // `poll` said readable, so this read does not block.
        let mut buf = [0u8; std::mem::size_of::<libc::signalfd_siginfo>()];
        let read = file.read(&mut buf).map_err(|e| Errno::from_io(&e))?;
        if read < buf.len() {
            // A short read from a signalfd is not something the interface
            // permits; refuse rather than decode a partial record.
            return Err(Errno::EINVAL);
        }
        // `ssi_signo` is the first field: a native-endian u32.
        let signo = u32::from_ne_bytes(
            buf.get(..4)
                .and_then(|s| s.try_into().ok())
                .unwrap_or([0; 4]),
        );
        Ok(Some(Sig::from_raw(i32::try_from(signo).unwrap_or(0))))
    }

    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn waitpid_any(&mut self) -> Result<Reaped, Errno> {
        let mut raw: libc::c_int = 0;
        // SAFETY: `waitpid` writes at most one `c_int` through the pointer,
        // which borrows a live local. `-1` means "any child" and `WNOHANG`
        // means it returns rather than blocking, so this cannot wedge PID 1.
        let pid = unsafe { libc::waitpid(-1, &mut raw, libc::WNOHANG) };
        if pid > 0 {
            let status = if libc::WIFEXITED(raw) {
                ExitStatus::Exited(libc::WEXITSTATUS(raw))
            } else if libc::WIFSIGNALED(raw) {
                ExitStatus::Signalled(libc::WTERMSIG(raw))
            } else {
                ExitStatus::Other(raw)
            };
            return Ok(Reaped::Child { pid, status });
        }
        if pid == 0 {
            return Ok(Reaped::NoneReady);
        }
        Err(Errno::last())
    }

    fn quiet_console(&mut self) -> ConsoleQuiet {
        ConsoleQuiet {
            fbcon_unbound: unbind_framebuffer_console(),
            printk_lowered: lower_console_loglevel(),
            output_discarded: discard_own_output(),
        }
    }

    fn spawn(&mut self, path: &str) -> Result<i32, Errno> {
        // `Command` rather than fork+exec by hand. ~~`std` resets the child's
        // signal mask and SIGPIPE disposition, which PID 1 has changed and a
        // child must not inherit.~~
        //
        // **Half of that was false, and it was measured on 2026-08-23.** `std`
        // resets the SIGPIPE *disposition* and **does not touch the signal
        // mask**: a child of this process comes up with `SigBlk` identical to
        // its parent's, bit for bit — see
        // `a_spawned_child_inherits_this_processs_blocked_signals` below, which
        // exists so this claim cannot go stale a second time.
        // `cmd/rivet-sysd/src/kernel.rs` had recorded the same
        // measurement all along and carries a `pre_exec` that clears it; this
        // crate carried the opposite claim, and two comments in one repository
        // disagreeing about a platform fact is what `AGENTS.md` §5 says to
        // settle by asking the system.
        //
        // **PID 1 deliberately does NOT add that `pre_exec`, and the reason is
        // not symmetry with the supervisor — it is the opposite of it.** The
        // supervisor clears the mask because a *service* that inherited a
        // blocked `SIGTERM` could not be stopped by the supervisor's own
        // shutdown. PID 1's only child is `rivet-sysd`, which
        // **re-blocks the same signals itself** and reads them from its own
        // `signalfd(2)`. Inheriting the block therefore covers the window
        // between `exec` and that arming — the supervisor's start-up reads
        // `/state/clock` and the desired-state document first, so the window is
        // milliseconds on a slow card rather than instants — and a `SIGTERM`
        // arriving in it is left **pending** and handled, instead of killing a
        // supervisor that has not armed yet. That is the same race-free
        // property `WakeMode::Signals` exists for, extended across the exec.
        //
        // The one asymmetry it leaves is `SIGUSR1` (`ADR-017` §3.1.10), which
        // the supervisor blocks by inheritance and never reads. Nothing sends
        // it to the supervisor; if anything ever does, it will be pending and
        // ignored rather than fatal, which is the safe direction.
        //
        // The returned `Child` is dropped without being waited on — `std` does
        // not reap on drop — because every exit on this system belongs to PID
        // 1's own reaper.
        match std::process::Command::new(path).spawn() {
            Ok(child) => i32::try_from(child.id()).map_err(|_| Errno::EINVAL),
            Err(e) => Err(Errno::from_io(&e)),
        }
    }

    fn spawn_with(&mut self, path: &str, args: &[String]) -> Result<i32, Errno> {
        // Everything `spawn` says about signal masks and reaping applies here
        // unchanged; the only difference is the argument vector.
        match std::process::Command::new(path).args(args).spawn() {
            Ok(child) => i32::try_from(child.id()).map_err(|_| Errno::EINVAL),
            Err(e) => Err(Errno::from_io(&e)),
        }
    }

    fn kill(&mut self, pid: i32, signal: Sig) -> Result<(), Errno> {
        // The fuse, not logic: 0 means "my whole process group" and -1 means
        // "every process I may signal", which from pid 1 is the entire machine.
        // Neither is ever what this program means.
        if pid <= 0 {
            return Err(Errno::EINVAL);
        }
        // SAFETY: `kill(2)` takes two integers, touches no memory of ours, and
        // the pid is guarded positive immediately above.
        let rc = unsafe { libc::kill(pid, signal.raw()) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn sync(&mut self) {
        // SAFETY: `sync(2)` takes no arguments, returns nothing, and cannot
        // fail. It touches no memory belonging to this process.
        unsafe { libc::sync() };
    }

    fn reboot(&mut self, how: RebootHow) -> Result<(), Errno> {
        let cmd = match how {
            RebootHow::PowerOff => libc::RB_POWER_OFF,
            RebootHow::Restart => libc::RB_AUTOBOOT,
            // The one command that carries an argument, so it does not go
            // through the same call at all. See `reboot_with_argument`.
            RebootHow::TrialRestart => return reboot_with_argument(TRYBOOT_ARGUMENT),
        };
        // SAFETY: `reboot(2)` takes one of the kernel's own command constants
        // and touches no memory of ours. On success it does not return; if it
        // does return, it failed and set errno.
        let rc = unsafe { libc::reboot(cmd) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn fork_child_exiting_with(&mut self, code: i32) -> Result<i32, Errno> {
        // SAFETY: this process is single-threaded — nothing spawns a thread —
        // so the child cannot inherit a lock held by a thread that no longer
        // exists. The child branch calls only `_exit`, which is
        // async-signal-safe and diverges, so no Rust destructor, allocator or
        // buffered writer runs after the fork. `say` flushes stdout, so there
        // are no buffered bytes to duplicate.
        let pid = unsafe {
            let pid = libc::fork();
            if pid == 0 {
                libc::_exit(code);
            }
            pid
        };
        if pid < 0 { Err(Errno::last()) } else { Ok(pid) }
    }

    fn fork_orphan_maker(&mut self, delay_ms: u64, code: i32) -> Result<i32, Errno> {
        let nsec = i64::try_from(delay_ms.saturating_mul(1_000_000)).unwrap_or(0);
        // SAFETY: as `fork_child_exiting_with`. The intermediate child forks
        // once more and `_exit`s at once, so its own child is reparented to
        // pid 1 while still alive. `nanosleep` reads a `timespec` that lives in
        // the child's frame for the duration of the call and writes nothing
        // (the remainder pointer is NULL). Both `nanosleep` and `_exit` are
        // async-signal-safe, and `fork` in a single-threaded process is.
        let pid = unsafe {
            let pid = libc::fork();
            if pid == 0 {
                let grandchild = libc::fork();
                if grandchild == 0 {
                    let delay = libc::timespec {
                        tv_sec: 0,
                        tv_nsec: nsec,
                    };
                    libc::nanosleep(&delay, std::ptr::null_mut());
                    libc::_exit(code);
                }
                libc::_exit(0);
            }
            pid
        };
        if pid < 0 { Err(Errno::last()) } else { Ok(pid) }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn core_flags_translate_to_the_platform_bits_they_name() {
        assert_eq!(
            MountFlags::from_core(&[MountFlag::NoExec]),
            MountFlags::NOEXEC
        );
        assert_eq!(
            MountFlags::from_core(&[MountFlag::NoSuid]),
            MountFlags::NOSUID
        );
        assert_eq!(
            MountFlags::from_core(&[MountFlag::NoDev]),
            MountFlags::NODEV
        );
    }

    #[test]
    fn no_flags_is_no_bits() {
        assert_eq!(MountFlags::from_core(&[]), MountFlags::NONE);
    }

    #[test]
    fn flags_combine_rather_than_replace() {
        let all = MountFlags::from_core(&[MountFlag::NoExec, MountFlag::NoSuid, MountFlag::NoDev]);
        assert!(all.contains(MountFlags::NOEXEC));
        assert!(all.contains(MountFlags::NOSUID));
        assert!(all.contains(MountFlags::NODEV));
        assert_eq!(
            all.0,
            MountFlags::NOEXEC.0 | MountFlags::NOSUID.0 | MountFlags::NODEV.0
        );
    }

    #[test]
    fn the_four_platform_bits_are_distinct() {
        // A translation table that mapped two intents onto one bit would pass
        // every test above and still be wrong.
        let bits = [
            MountFlags::NOEXEC,
            MountFlags::NOSUID,
            MountFlags::NODEV,
            MountFlags::RDONLY,
        ];
        for (i, a) in bits.iter().enumerate() {
            for (j, b) in bits.iter().enumerate() {
                assert!(i == j || a != b, "two mount flags share a bit");
            }
        }
    }

    #[test]
    fn union_adds_bits_without_dropping_any() {
        let both = MountFlags::NOEXEC.union(MountFlags::RDONLY);
        assert!(both.contains(MountFlags::NOEXEC));
        assert!(both.contains(MountFlags::RDONLY));
    }

    #[test]
    fn run_carries_no_noexec_bit_because_services_are_started_from_it() {
        // rivet-core tests the *intent*; this tests the translation of it.
        let run = rivet_core::mount::stage0_plan()
            .iter()
            .find(|s| s.target == "/run");
        assert!(run.is_some_and(|s| !MountFlags::from_core(s.flags).contains(MountFlags::NOEXEC)));
    }

    #[test]
    fn errno_zero_says_so_rather_than_inventing_a_message() {
        assert_eq!(Errno(0).to_string(), "error with no OS error number");
    }

    #[test]
    fn errno_display_carries_the_number_the_platform_used() {
        assert!(
            Errno::EBUSY
                .to_string()
                .contains(&Errno::EBUSY.0.to_string())
        );
    }

    #[test]
    fn signals_round_trip_through_their_platform_numbers() {
        for sig in [Sig::Child, Sig::Term, Sig::Interrupt, Sig::User1, Sig::Kill] {
            assert_eq!(Sig::from_raw(sig.raw()), sig);
        }
    }

    /// Four distinct numbers. A table that mapped `User1` onto `SIGTERM` would
    /// power the board off when it was asked for a trial boot — an update that
    /// silently did the wrong thing rather than one that failed.
    #[test]
    fn the_signals_pid_one_receives_are_four_different_numbers() {
        let raws = [
            Sig::Child.raw(),
            Sig::Term.raw(),
            Sig::Interrupt.raw(),
            Sig::User1.raw(),
        ];
        for (i, a) in raws.iter().enumerate() {
            for (j, b) in raws.iter().enumerate() {
                assert!(i == j || a != b, "two received signals share a number");
            }
        }
        assert_eq!(Sig::User1.raw(), libc::SIGUSR1);
    }

    #[test]
    fn an_unknown_signal_number_is_carried_rather_than_discarded() {
        // SIGWINCH is not in HANDLED_SIGNALS. If one ever arrives, PID 1 must
        // be able to say which it was rather than reporting a wrong name.
        let sig = Sig::from_raw(libc::SIGWINCH);
        assert_eq!(sig, Sig::Other(libc::SIGWINCH));
        assert!(sig.to_string().contains(&libc::SIGWINCH.to_string()));
    }

    #[test]
    fn the_handled_set_is_the_four_the_loop_dispatches_on() {
        assert_eq!(
            HANDLED_SIGNALS,
            [Sig::Child, Sig::Term, Sig::Interrupt, Sig::User1]
        );
        // SIGKILL is in the enum because PID 1 *sends* it; blocking it is not
        // possible and asking to would make `arm_signals` fail outright.
        assert!(!HANDLED_SIGNALS.contains(&Sig::Kill));
    }

    // -- the signal mask, which is what stops SIGUSR1 killing pid 1 -----------
    //
    // `SIGUSR1`'s default disposition is **terminate**. `ADR-017` §3.1.10 gives
    // it a meaning on this system, so a mask that did not actually carry it
    // would produce a pid 1 that dies the first time an operator asks for a
    // trial boot — and every test above would still pass, because the fake
    // kernel has no signals in it at all. These two run against `LibcKernel`.

    /// The mask is read back **out of the kernel** rather than inferred from
    /// having called `sigaddset` — `AGENTS.md` §5: report what the system said.
    #[test]
    fn arming_blocks_every_signal_the_loop_dispatches_on() {
        let mut k = LibcKernel::new();
        assert_eq!(k.arm_signals(), Ok(()), "signalfd was refused on this host");

        // SAFETY: `set` is a `sigset_t` owned by this frame. `pthread_sigmask`
        // with a NULL new-set pointer changes nothing and only writes the
        // current mask through the second pointer; `sigismember` reads it.
        let blocked = |sig: i32| -> bool {
            unsafe {
                let mut set: libc::sigset_t = std::mem::zeroed();
                if libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut set) != 0 {
                    return false;
                }
                libc::sigismember(&set, sig) == 1
            }
        };

        for sig in HANDLED_SIGNALS {
            assert!(
                blocked(sig.raw()),
                "{sig} is dispatched on but is NOT blocked: it would take its default action"
            );
        }
        // The control: a signal nobody armed is not blocked, so the assertions
        // above are reading a real mask rather than a function that says yes.
        assert!(
            !blocked(libc::SIGWINCH),
            "the mask check answers yes to anything"
        );
    }

    /// **The one that would actually die.** A real `SIGUSR1` is delivered to
    /// this process — which is *not* pid 1, so Linux's "ignore `SIG_DFL`
    /// signals sent to init" rule does not apply and cannot be what saves it.
    /// If the mask were wrong, the test binary would terminate here rather than
    /// fail, which is the loudest possible failure.
    ///
    /// `pthread_kill` rather than `kill`: a process-directed signal may be taken
    /// by any thread that has it unblocked, and cargo's harness is
    /// multi-threaded. Directed at this thread it becomes pending on this
    /// thread, and `signalfd(2)` reports signals pending for the reading thread
    /// as well as for the process.
    #[test]
    fn a_real_sigusr1_is_read_from_the_descriptor_instead_of_killing_the_process() {
        let mut k = LibcKernel::new();
        assert_eq!(k.arm_signals(), Ok(()), "signalfd was refused on this host");

        // SAFETY: `pthread_kill` takes a thread id and a signal number and
        // touches no memory of ours. `pthread_self()` is this thread, whose
        // mask the line above has just had `SIGUSR1` added to, so the signal
        // becomes pending rather than acted on.
        let rc = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGUSR1) };
        assert_eq!(rc, 0, "could not deliver the signal");

        // Reaching this line at all is the assertion: an unblocked SIGUSR1
        // would have terminated the process on the line above.
        assert_eq!(k.await_signal(1_000), Ok(Some(Sig::User1)));
    }

    /// The fuse described in the module docs. `kill(-1, SIGKILL)` from pid 1
    /// would signal every process on the machine.
    #[test]
    fn kill_refuses_a_non_positive_pid() {
        let mut k = LibcKernel::new();
        assert_eq!(k.kill(0, Sig::Term), Err(Errno::EINVAL));
        assert_eq!(k.kill(-1, Sig::Term), Err(Errno::EINVAL));
    }

    /// **A lock another holder has is refused at once, never waited on** —
    /// against the real `flock(2)`, the holder being a second `LibcKernel`
    /// and so a second open file description, which `flock` treats as another
    /// process. The directory and the `0600` file are created by the first
    /// taker; once released, the lock can be taken.
    #[test]
    fn a_held_lock_is_refused_at_once_and_a_released_one_is_taken() {
        use std::os::unix::fs::PermissionsExt as _;
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("rivet").join("autoboot.lock");
        let path = path.to_string_lossy().into_owned();

        let mut holder = LibcKernel::new();
        let mut pid1 = LibcKernel::new();
        assert_eq!(holder.try_lock(&path), Ok(()));
        let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode.ok(), Some(0o600));

        let started = std::time::Instant::now();
        assert_eq!(pid1.try_lock(&path), Err(Errno::EAGAIN));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(100),
            "try_lock waited {:?}",
            started.elapsed()
        );

        holder.unlock(&path);
        assert_eq!(pid1.try_lock(&path), Ok(()));
        assert_eq!(holder.try_lock(&path), Err(Errno::EAGAIN));
        pid1.unlock(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A released lock is released even while a forked child still holds a
    /// copy of its descriptor.** `flock(2)` locks the open file description,
    /// and a `fork` duplicates the descriptor onto it; closing this process's
    /// copy alone leaves the lock held until the child closes its own — at
    /// its `exec`, or never. The child here is exactly that window, held
    /// open: it inherits the descriptor and waits on a pipe. Any thread of a
    /// multi-threaded process (PID 1 forking the supervisor, or a test
    /// harness running another test that spawns) opens the same window.
    #[test]
    fn a_released_lock_is_free_while_a_forked_child_holds_its_descriptor() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-lock-fork-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("autoboot.lock").to_string_lossy().into_owned();

        let mut holder = LibcKernel::new();
        let mut pid1 = LibcKernel::new();
        assert_eq!(holder.try_lock(&path), Ok(()));

        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `pipe2(2)` writes two descriptors into the two-element array
        // it is handed, which lives for the whole call.
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0, "pipe2");
        let [read_end, write_end] = fds;
        // SAFETY: the child calls only `close`, `read` and `_exit`, each
        // async-signal-safe, so forking a multi-threaded process is sound.
        let child = unsafe { libc::fork() };
        if child == 0 {
            // SAFETY: as above — descriptors and a one-byte stack buffer.
            unsafe {
                libc::close(write_end);
                let mut byte = 0u8;
                // Returns when every write end is closed: the parent's signal.
                libc::read(read_end, std::ptr::addr_of_mut!(byte).cast(), 1);
                libc::_exit(0);
            }
        }
        assert!(child > 0, "fork failed");
        // SAFETY: closing a descriptor this process owns.
        unsafe { libc::close(read_end) };

        holder.unlock(&path);
        let taken = pid1.try_lock(&path);

        // SAFETY: closing the last write end releases the child; `waitpid`
        // writes one status integer into a local.
        unsafe {
            libc::close(write_end);
            let mut status = 0;
            libc::waitpid(child, &mut status, 0);
        }
        assert_eq!(taken, Ok(()), "the lock stayed held by the child's copy of the descriptor");
        pid1.unlock(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_monotonic_clock_does_not_go_backwards() {
        let mut k = LibcKernel::new();
        let a = k.now_ms();
        let b = k.now_ms();
        assert!(b >= a);
    }

    /// The one place a real filesystem is used: what errno the platform gives
    /// for `mkdir -p` over an existing *file* is the platform's answer, and
    /// `AGENTS.md` §5 says read it rather than reproduce it from documentation.
    #[test]
    fn mkdir_p_over_a_file_reports_what_the_platform_actually_said() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut file = dir.clone();
        file.push("not-a-directory");
        let wrote = std::fs::write(&file, b"x").is_ok();
        assert!(wrote, "could not create the fixture file");

        let mut k = LibcKernel::new();
        let err = k.mkdir_p(&file.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);

        // Printed rather than only asserted, so `cargo test -- --nocapture`
        // shows what this platform actually answered instead of what this
        // repository believes it answers.
        eprintln!("mkdir_p over an existing file answered: {err:?}");

        // Not asserted as EEXIST from memory: assert it is one of the two the
        // kernel can give here, both of which `boot::mkdir_detail` classifies
        // as "the path exists and is not a directory".
        assert!(
            err == Err(Errno::EEXIST) || err == Err(Errno::ENOTDIR),
            "mkdir_p over a file gave {err:?}, which boot::mkdir_detail does not expect"
        );
    }

    #[test]
    fn mkdir_p_is_idempotent_over_a_directory_that_exists() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-idem-{}", std::process::id()));
        let mut k = LibcKernel::new();
        let first = k.mkdir_p(&dir.to_string_lossy());
        let second = k.mkdir_p(&dir.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
    }

    /// Write-then-rename is how `crate::confirm` commits `autoboot.txt`; this
    /// checks the two primitives it is built from against a real filesystem.
    #[test]
    fn write_then_rename_replaces_a_file_in_one_step() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-rename-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("autoboot.txt").to_string_lossy().into_owned();
        let temp = dir.join("autoboot.txt.new").to_string_lossy().into_owned();

        let mut k = LibcKernel::new();
        let seeded = k.write_file(&target, b"old");
        let wrote = k.write_file(&temp, b"new");
        let renamed = k.rename(&temp, &target);
        let read_back = k.read_bytes(&target);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(seeded, Ok(()));
        assert_eq!(wrote, Ok(()));
        assert_eq!(renamed, Ok(()));
        assert_eq!(read_back, Ok(b"new".to_vec()));
    }

    // -- write_secret's atomic recipe -----------------------------------------
    //
    // B12: a partial `write_secret` used to be visible at its own destination.
    // These drive the REAL recipe against a real filesystem, because mode,
    // fsync ordering and O_EXCL are exactly the branches a fake cannot answer.

    #[test]
    fn write_secret_publishes_mode_0600_and_no_temporary_survives_it() {
        use std::os::unix::fs::PermissionsExt as _;

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-secret-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("device.seed").to_string_lossy().into_owned();

        let mut k = LibcKernel::new();
        let wrote = k.write_secret(&target, b"thirty-two bytes of seed material");
        let meta = std::fs::metadata(&target);
        let temp_left_behind = dir.join("device.seed.new").exists();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(wrote, Ok(()));
        let mode = meta.expect("the destination was not created").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "write_secret must set the mode in the open, not leave the umask's"
        );
        assert!(
            !temp_left_behind,
            "the temporary was not renamed away, so a directory listing would show it"
        );
    }

    #[test]
    fn write_secret_replaces_its_own_content_in_full_rather_than_appending() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-secret-replace-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("device.seed").to_string_lossy().into_owned();

        let mut k = LibcKernel::new();
        let first = k.write_secret(&target, b"the first, longer set of bytes");
        let second = k.write_secret(&target, b"shorter");
        let read_back = k.read_bytes(&target);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
        assert_eq!(
            read_back,
            Ok(b"shorter".to_vec()),
            "a second, shorter publish must leave exactly its own bytes, never the \
             first write's tail"
        );
    }

    /// **The leftover a crash between `O_CREAT|O_EXCL` and the rename leaves
    /// behind.** The next call must not be refused by it, and must not publish
    /// through it — B12's "ignored or removed, never mistaken for the real
    /// file".
    #[test]
    fn write_secret_recovers_from_a_stale_temporary_left_by_an_earlier_call() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-secret-stale-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let target = dir.join("device.seed").to_string_lossy().into_owned();
        let temp = dir.join("device.seed.new");
        // What an interrupted PREVIOUS call left: the temporary exists, with
        // whatever partial bytes it had managed before the power cut.
        let planted = std::fs::write(&temp, b"garbage from an interrupted call").is_ok();
        assert!(planted, "could not plant the fixture");

        let mut k = LibcKernel::new();
        let wrote = k.write_secret(&target, b"the real seed");
        let read_back = k.read_bytes(&target);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(wrote, Ok(()), "a stale temporary must not refuse the next call");
        assert_eq!(
            read_back,
            Ok(b"the real seed".to_vec()),
            "the stale temporary's bytes must never reach the destination"
        );
    }

    /// A destination whose directory is absent answers `ENOENT`, which is
    /// every first boot's ordinary case rather than a fault (`AGENTS.md` §5).
    #[test]
    fn write_secret_leaves_no_destination_at_all_when_the_directory_does_not_exist() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-initd-test-secret-nodir-{}", std::process::id()));
        // Deliberately not created.
        let target = dir.join("device.seed").to_string_lossy().into_owned();

        let mut k = LibcKernel::new();
        let wrote = k.write_secret(&target, b"x");
        let read_back = k.read_bytes(&target);

        assert!(wrote.is_err(), "a missing directory must be reported, not silently made");
        assert_eq!(read_back, Err(Errno::ENOENT));
    }

    // -- the tryboot reboot ---------------------------------------------------
    //
    // None of this can be *exercised*: `LibcKernel::reboot(TrialRestart)` stops
    // the machine on success, and `ADR-026` §4 says QEMU does not emulate
    // `tryboot` at all. What is checkable is the data the call is built from,
    // which is where the two plausible mistakes live — a wrong string and a
    // wrong command constant.

    /// `ADR-018` §5.3 records the string. The quotes in Raspberry Pi's
    /// `sudo reboot "0 tryboot"` are the shell's; the argument is two words.
    #[test]
    fn the_tryboot_argument_is_the_string_adr_018_5_3_records() {
        assert_eq!(TRYBOOT_ARGUMENT, "0 tryboot");
        assert!(
            !TRYBOOT_ARGUMENT.contains('"'),
            "the quotes are the shell's"
        );
        assert!(
            !TRYBOOT_ARGUMENT.contains('\0'),
            "a NUL would make this unrepresentable as a C string"
        );
    }

    /// The three reboot commands must be three different numbers. A table that
    /// mapped `TrialRestart` onto `RB_AUTOBOOT` would reboot the board, look
    /// entirely healthy, and never trial anything.
    #[test]
    fn the_restart2_command_is_not_the_plain_restart_or_the_power_off() {
        let cmds = [
            libc::RB_AUTOBOOT,
            libc::RB_POWER_OFF,
            libc::LINUX_REBOOT_CMD_RESTART2,
        ];
        for (i, a) in cmds.iter().enumerate() {
            for (j, b) in cmds.iter().enumerate() {
                assert!(i == j || a != b, "two reboot commands share a value");
            }
        }
        // And RB_AUTOBOOT really is LINUX_REBOOT_CMD_RESTART — the *argumentless*
        // one — which is what makes RESTART2 a different call rather than a flag.
        assert_eq!(libc::RB_AUTOBOOT, libc::LINUX_REBOOT_CMD_RESTART);
    }

    #[test]
    fn every_reboot_request_says_which_one_it_is_on_the_console() {
        let all = [
            RebootHow::PowerOff,
            RebootHow::Restart,
            RebootHow::TrialRestart,
        ];
        for how in all {
            assert!(!how.to_string().is_empty(), "{how:?} has no console form");
        }
        // The trial restart must not read as an ordinary reboot: PID 1's
        // shutdown lines are the only diagnostic a board gives here.
        assert_ne!(
            RebootHow::TrialRestart.to_string(),
            RebootHow::Restart.to_string()
        );
    }

    /// **The measurement, kept as a test so the claim cannot go stale again.**
    ///
    /// `Kernel::spawn`'s comment said that `std` resets the child's signal
    /// mask. It does not — it resets the SIGPIPE *disposition* — and
    /// `cmd/rivet-sysd/src/kernel.rs` had recorded the opposite
    /// measurement all along. Two comments in one repository disagreeing about
    /// a platform fact is exactly what `AGENTS.md` §5 says to settle by asking
    /// the system.
    ///
    /// **This asserts the inheritance rather than mourning it.**
    /// `Kernel::spawn` carries the argument for why PID 1 leaves it alone: the
    /// supervisor re-blocks the same signals, so inheriting the block covers
    /// the window before it has armed. If a future `std` starts clearing the
    /// mask, this test fails — which is the point, because that would silently
    /// open the window again.
    #[test]
    fn a_spawned_child_inherits_this_processs_blocked_signals() {
        // SAFETY: `set` is a `sigset_t` owned by this frame; the three calls
        // only read or write through that pointer and retain nothing. This
        // blocks SIGUSR1 in *this test thread* only, which is what makes the
        // child's inherited mask observable.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGUSR1);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }

        // Exactly what `LibcKernel::spawn` does, plus the arguments needed to
        // make the child say what its mask is.
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("grep ^SigBlk /proc/self/status")
            .output();
        let Ok(out) = out else {
            // No `/bin/sh` on this host, which is not this test's subject.
            eprintln!("skipping: /bin/sh could not be run");
            return;
        };
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // Printed as well as asserted, so `--nocapture` shows what this platform
        // actually said rather than what this repository believes it says.
        eprintln!("a spawned child's mask: {line}");

        let Some(mask) = line
            .split_whitespace()
            .nth(1)
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        else {
            eprintln!("skipping: no SigBlk line to read ({line})");
            return;
        };

        // SIGUSR1 is signal 10, so it is bit 9 of the mask word: 0x200.
        let usr1_bit = 1_u64 << (libc::SIGUSR1 - 1);
        assert_ne!(
            mask & usr1_bit,
            0,
            "std::process::Command cleared the child's signal mask. That is the opposite of what \
             was measured on 2026-08-23, and `Kernel::spawn`'s argument for letting the supervisor \
             inherit PID 1's block no longer holds — read it before changing this assertion"
        );
    }

    #[test]
    fn reading_a_file_that_is_not_there_says_enoent() {
        let mut k = LibcKernel::new();
        assert_eq!(
            k.read_bytes("/no/such/path/rivet-initd-test"),
            Err(Errno::ENOENT)
        );
    }
}
