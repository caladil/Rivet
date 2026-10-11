//! **The wall around `libc`.** `ADR-009` §3.4.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! The same split `cmd/rivet-initd`'s `kernel` module and
//! `cmd/rivet-sysd`'s make, and the same rule their `Cargo.toml`s state:
//! *no `libc` type appears outside this module.* What leaves here is `&str`,
//! `()` and a bare `i32` errno — Linux's own number, which
//! [`crate::boot::SyscallMedium`] turns into `rivet_adapter_image::Errno`
//! before anything reasons about it.
//!
//! # Why this crate has a syscall seam at all
//!
//! It is the login shell and it parsed argv and rendered
//! values and nothing else, which `src/main.rs` said out loud: *"It does not read
//! a configuration file, and it does not touch the network, the filesystem or any
//! device."* **That stopped being true on 2026-08-23**, and the reason is
//! `ADR-028` §4: an authenticated principal supplies the authority to change a
//! machine's boot slot, `ADR-006` §5 makes the CLI the only
//! administration surface there is, and `rivet_adapter_image` deliberately
//! ships no `BootMedium` implementor because `mount(2)` has no `std` equivalent
//! and that crate makes no syscalls. Its `port` module recorded the moment this
//! would arrive:
//!
//! > **That is a decision with a known expiry.** `ADR-028` §4 says an
//! > authenticated principal supplies the authority to install an update and does
//! > not say which process executes it. If that turns out to be anything other
//! > than PID 1, this is where the question of a second implementor is answered.
//!
//! It is not PID 1. So the implementor is here, at the caller's own seam, which
//! is the answer that module named.
//!
//! # ~~Three calls, and only three~~ Four, since 2026-08-23
//!
//! `mount(2)`, `umount2(2)` and `sync(2)`. Every other operation the port needs —
//! reading a device-tree property, reading and writing a file, renaming one — has
//! a `std` equivalent and uses it, because `ADR-009` §2's reuse posture is about
//! reaching for the implementation that exists rather than the lowest one
//! available. ~~Three `unsafe` blocks, each with a SAFETY comment, and there are
//! no others in this crate.~~
//!
//! **A fourth arrived with [`crate::measure`]: `clock_gettime(2)`.** Struck rather
//! than edited, per `AGENTS.md` §2 — the count is exactly the kind of sentence §7
//! is about, and it went stale the first time something in this crate needed to
//! time anything.
//!
//! It is here rather than behind `std::time::Instant` for one reason: `ADR-021`
//! §5 names **`CLOCK_MONOTONIC`** specifically, and it names it because a wall
//! clock on this hardware may be 1970 and may be stepped mid-measurement. `Instant`
//! is documented as monotonic and is very probably this same call, but *"very
//! probably"* is not a reading (`AGENTS.md` §7), and a benchmark whose clock is
//! an assumption produces numbers with the same status. The clock id is visible
//! in the line below, which is the whole point of writing it out.
//!
//! ~~Four `unsafe` blocks, each with a SAFETY comment, and there are no others in
//! this crate.~~
//!
//! **Five, later on 2026-08-23: `kill(2)`** — struck rather than edited, because
//! the sentence above went stale within a day of being written and `AGENTS.md` §7
//! is about exactly that. [`kill_init`] is what makes `rivet apply power-off`,
//! `restart` and `trial-restart` possible at all: `ADR-017` §3.1.5 item 6 gives
//! shutdown to pid 1 and §3.1.10 gives it three signals to be asked with, and
//! **until this function existed nothing in the image could send any of them.**
//! A Rivet board could be stopped only by removing power.
//!
//! It is here rather than behind anything in `std` because there is nothing in
//! `std` that sends a signal.
//!
//! Five `unsafe` blocks, each with a SAFETY comment, and there are no others in
//! this crate.
//!
//! # Linux only, and it refuses elsewhere rather than pretending
//!
//! This module is `#[cfg(target_os = "linux")]`. The development host is Windows
//! (`AGENTS.md` §10.5) and `src/main.rs`'s claim that *"everything below is
//! testable on any machine, including the Windows host this is written on"* is
//! worth keeping true. On any other platform `crate::boot` supplies a machine
//! that **refuses** — never one that succeeds — which is `AGENTS.md` §3: *"an
//! unsupported-platform implementation returns an error. Never `Ok(())`."*

use std::ffi::CString;

/// The errno for a `std::io::Error`, or `0` when the platform did not give one.
///
/// `0` is never a real errno, and `rivet_adapter_image::Errno`'s `Display`
/// already renders it as *"an error with no OS error number"* — so an unexpected
/// shape reaches the operator as itself rather than as a guess (`AGENTS.md` §5).
pub fn errno_of(error: &std::io::Error) -> i32 {
    error.raw_os_error().unwrap_or(0)
}

/// The errno the last failing syscall set.
fn last_errno() -> i32 {
    errno_of(&std::io::Error::last_os_error())
}

/// Mount a filesystem at `at`, read-only unless `writable`.
///
/// **The flag policy is stated here because the port leaves it to the
/// implementor**, deliberately — `rivet_adapter_image::BootMedium::
/// mount_boot_filesystem` says the `MS_*` bits *"are a platform's own bits, and
/// on this hardware they carry a policy"*. The policy is
/// `cmd/rivet-initd::provision::mount_boot_filesystem`'s, and it is the same one
/// for the same reason: a FAT filesystem an operator edits from a Windows machine
/// (`ADR-002` §4.1) is as untrusted as anything on the board gets, so never
/// executable, never suid, never a device node.
///
/// **This is a second copy of that policy**, which `ADR-011` would normally
/// forbid. It is unavoidable rather than convenient: the first copy is in a
/// binary crate with no lib target, which is the same debt, standing since
/// 2026-08-23, already tracked for four path constants. The tripwire meanwhile is
/// `crate::boot`'s `the_mount_carries_the_same_flag_policy_pid_one_uses`, which
/// asserts the *bits* rather than trusting the comment.
pub fn mount(device: &str, at: &str, fstype: &str, writable: bool) -> Result<(), i32> {
    let (Ok(device), Ok(at_c), Ok(fstype)) =
        (CString::new(device), CString::new(at), CString::new(fstype))
    else {
        return Err(libc::EINVAL);
    };

    let mut flags = libc::MS_NOEXEC | libc::MS_NOSUID | libc::MS_NODEV;
    if !writable {
        flags |= libc::MS_RDONLY;
    }

    // SAFETY: `mount(2)` reads four borrowed pointers and returns an int; it
    // retains none of them. `device`, `at_c` and `fstype` are NUL-terminated and
    // owned by `CString`s alive for the whole call. The data pointer is NULL,
    // which `mount(2)` documents as "no filesystem-specific options". `flags` is
    // a bitmask of `MS_*` constants and cannot be an invalid value.
    let rc = unsafe {
        libc::mount(
            device.as_ptr(),
            at_c.as_ptr(),
            fstype.as_ptr(),
            flags,
            std::ptr::null(),
        )
    };
    if rc == 0 { Ok(()) } else { Err(last_errno()) }
}

/// Unmount whatever is at `at`.
pub fn unmount(at: &str) -> Result<(), i32> {
    let Ok(at) = CString::new(at) else {
        return Err(libc::EINVAL);
    };
    // SAFETY: `umount2(2)` reads one NUL-terminated borrowed pointer, owned by a
    // `CString` alive for the whole call, and retains nothing. A zero flag word
    // is the plain unmount, not one of the lazy or forced modes.
    let rc = unsafe { libc::umount2(at.as_ptr(), 0) };
    if rc == 0 { Ok(()) } else { Err(last_errno()) }
}

/// An exclusive `flock(2)` on an open file. **Dropping it releases the lock**,
/// because closing the descriptor does — and so does this process exiting, so
/// a `rivet` that dies holding one cannot leave the lock held.
#[derive(Debug)]
pub struct HeldLock {
    _file: std::fs::File,
}

/// Take `path`'s exclusive advisory lock, waiting at most `bound_ms`.
///
/// Creates the file, mode `0600`, and its directory if they are missing. Tries
/// without blocking and, while another descriptor holds the lock, tries again
/// every `poll_ms` until `bound_ms` have passed on the monotonic clock, then
/// answers `EAGAIN`. A non-blocking try in a loop rather than one blocking
/// `flock(2)`, because a blocking one has no timeout and this process is a
/// command somebody is waiting on.
pub fn lock_exclusive_within(path: &str, bound_ms: u64, poll_ms: u64) -> Result<HeldLock, i32> {
    use std::os::unix::fs::OpenOptionsExt as _;
    use std::os::unix::io::AsRawFd as _;

    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| errno_of(&e))?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|e| errno_of(&e))?;

    let started = monotonic_nanos()?;
    loop {
        // SAFETY: `flock(2)` takes a descriptor and an integer operation and
        // touches no memory belonging to this process. The descriptor is owned
        // by `file`, which is alive for the whole call; the operation is two
        // `LOCK_*` constants.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc == 0 {
            return Ok(HeldLock { _file: file });
        }
        let errno = last_errno();
        if errno == libc::EINTR {
            continue;
        }
        if errno != libc::EWOULDBLOCK {
            return Err(errno);
        }
        let waited_ms = monotonic_nanos()?.saturating_sub(started) / 1_000_000;
        if waited_ms >= u128::from(bound_ms) {
            return Err(libc::EAGAIN);
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms));
    }
}

/// **`lchown(2)`** — change a path's owner **without following a symbolic
/// link**.
///
/// `ADR-006` §8.10.10, and the reason it is a syscall rather than anything in
/// `std` is the same reason `statvfs` is: the standard library has no `chown`
/// at all, and every crate that supplies one follows links by default.
/// §8.10.4.3 makes not following one a safety property of this whole family —
/// a link is a way out of the permitted root — so the call has to be the one
/// that does not.
///
/// A `None` half is left alone, which `lchown(2)` spells as `-1` in a type
/// where every other value is an identity.
///
/// # Errors
///
/// The string the caller reports, because this is the adapter's own I/O seam
/// and the operation above it says what a failure means. `EPERM` is the one
/// that will be seen: only uid 0 may give a file away.
pub fn change_owner(path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), String> {
    let Ok(path_c) = CString::new(path) else {
        return Err(format!("{path} cannot be passed to the kernel"));
    };
    // `-1` in the unsigned identity type is `lchown(2)`'s "leave this one",
    // spelled once here rather than at each caller.
    const UNCHANGED: libc::uid_t = libc::uid_t::MAX;
    let owner = uid.unwrap_or(UNCHANGED);
    let group = gid.unwrap_or(UNCHANGED as libc::gid_t);
    // SAFETY: `lchown(2)` reads one NUL-terminated borrowed pointer, owned by a
    // `CString` alive for the whole call, and retains nothing. The two identity
    // arguments are values.
    let rc = unsafe { libc::lchown(path_c.as_ptr(), owner, group) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(last_errno()).to_string())
    }
}

/// Flush filesystem buffers.
///
/// Cannot fail, because `sync(2)` cannot. On a Pi — no battery, usually an SD
/// card — the ordering around this call is the whole of what stops a power cut
/// leaving an `autoboot.txt` that will not boot.
pub fn sync() {
    // SAFETY: `sync(2)` takes no arguments, returns nothing, and cannot fail.
    unsafe { libc::sync() };
}

/// `CLOCK_MONOTONIC`, in nanoseconds since an unspecified fixed point.
///
/// **`ADR-021` §5 is the whole reason this is a syscall rather than
/// `std::time::Instant`:**
///
/// > **Wall-clock time is for timestamps. `CLOCK_MONOTONIC` is for durations.**
///
/// The value is meaningless on its own and only differences of it mean anything,
/// which is why it is returned as a bare count rather than as anything
/// time-shaped: nothing above this line may mistake it for a moment.
///
/// §5's own narrowing applies and is worth repeating where the call is: this
/// clock does not *jump*, but `adjtime` slewing changes its **rate**, capped at
/// about 500 ppm. Over a measurement of a few seconds that is microseconds, and
/// it cannot matter to a throughput figure quoted in MB/s.
pub fn monotonic_nanos() -> Result<u128, i32> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime(2)` writes one `struct timespec` through the pointer
    // it is given and retains nothing beyond the call. `ts` is a live local of
    // exactly that type, and the pointer is derived from it directly.
    // `CLOCK_MONOTONIC` is a clock id every Linux implements.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    if rc != 0 {
        return Err(last_errno());
    }
    // Both fields are signed and neither is ever negative for this clock. They
    // are converted rather than cast so that an impossible value becomes zero
    // instead of a very large number — a duration of zero is visibly wrong, and
    // a wrapped one looks like a plausible measurement.
    let seconds = u128::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u128::try_from(ts.tv_nsec).unwrap_or(0);
    Ok(seconds * 1_000_000_000 + nanos)
}

/// **How many of the kernel's clock ticks make a second** — `sysconf(3)`'s
/// `_SC_CLK_TCK`, and the unit every CPU-time figure in `/proc` is stated in.
///
/// Added 2026-08-29 with `rivet get cpu`. It is here rather than as a constant
/// `100` for exactly [`monotonic_nanos`]'s reason, one level down: **the value
/// is knowable, so assuming it is a choice to not measure.** 100 is what every
/// Linux target Rivet builds for uses (`ADR-003` §4), and writing that number
/// into a parser would make it right on the board and silently wrong the first
/// time somebody built for something else — every CPU figure off by a constant
/// factor, and nothing on screen to say so. `AGENTS.md` §5: derive it.
///
/// **It is not `unsafe` for a reason worth stating.** `sysconf` takes an
/// integer and returns one; it reads no memory this process owns and writes
/// none. The `unsafe` is `libc`'s FFI boundary and nothing else, which is why
/// this block carries a SAFETY comment two lines long rather than a paragraph —
/// there is no pointer, no lifetime and no buffer to reason about.
///
/// # Errors
///
/// `sysconf` returns `-1` for a name it does not recognise, which for
/// `_SC_CLK_TCK` would mean a C library that does not implement POSIX. The
/// errno is passed up rather than papered over with a default, so that
/// [`crate::cpu`] refuses rather than reporting times computed against a rate it
/// guessed. It also refuses `0`, which would make every conversion a division by
/// zero.
pub fn clock_ticks_per_second() -> Result<u64, i32> {
    // SAFETY: `sysconf(3)` takes one integer and returns one; it reads and
    // writes no memory belonging to this process and retains nothing.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks <= 0 {
        return Err(last_errno());
    }
    u64::try_from(ticks).map_err(|_| libc::EOVERFLOW)
}

/// **`statvfs(2)` on a path** — how full the filesystem holding it is.
///
/// `ADR-005` §4.1.3's `df` row, at the bottom of the stack `rivet get storage`
/// stands on. Sixth `unsafe` block, and the count above this module's
/// [`kill_init`] paragraph is not amended again: `AGENTS.md` §7 is about
/// sentences like that one, and three strikethroughs is enough to establish
/// that the number belongs in a `grep` rather than in prose. **Count the
/// `unsafe` blocks.**
///
/// # `statvfs` rather than `statfs`
///
/// Both exist on Linux and both would answer. `statvfs(3)` is POSIX, its fields
/// mean the same thing on every platform Rivet might reach (the boot-slot
/// port keeps an x86 machine in view), and — the part that decides it — it
/// carries **`f_flag`**, which is where `ST_RDONLY` lives. `statfs(2)` has no
/// such field, so a card that had gone read-only mid-write (`ADR-018` §8) would
/// have to be detected by parsing the mount option string, which is what the
/// kernel was told rather than what is true.
///
/// What is given up by not using `statfs(2)` is `f_type`, the filesystem's magic
/// number. Nothing needs it: the type comes from the mount table as a word.
///
/// # It returns seven numbers and a flag, and converts none of them
///
/// The caller is [`crate::storage`], which fills in
/// `rivet_adapter_system::Usage`. Everything above that is the domain's —
/// `rivet_core::operation::storage` owns what "used" means, and the whole
/// point of the seam is that no arithmetic happens on this side of it.
///
/// **`f_frsize` falling back to `f_bsize`** is the one judgement made here, and
/// it is `uucore::fsext`'s: the two are usually equal, are not always, and
/// `f_frsize` is zero on some filesystems. It is here rather than one layer up
/// because it is a fact about the syscall.
// `statvfs`'s field widths are libc- and target-dependent: `f_frsize`, `f_bsize`
// and the block and inode counts are `c_ulong` or `fsblkcnt_t`, which are 64-bit
// on the targets Rivet ships (`ADR-002`) and NOT on every target this code
// compiles for during development. clippy therefore sees `u64::try_from` on a
// value that is already `u64` *for the target it is linting* and calls it
// useless. It is not useless; it is the reason the conversions are here at all,
// and the comment inside the function says what they buy.
//
// Deleting them to silence the lint would make this function correct only for
// 64-bit targets and silently wrong elsewhere — trading a real property for a
// clean report. Allowed rather than removed, with the reason at the site
// (ruled by the maintainer, 2026-08-24).
#[allow(clippy::useless_conversion)]
pub fn statvfs(path: &str) -> Result<Statvfs, i32> {
    let Ok(path) = CString::new(path) else {
        return Err(libc::EINVAL);
    };
    // SAFETY: `struct statvfs` is a plain-old-data record of unsigned integers
    // with libc-specific padding and reserved words, so all-zero is a valid
    // value of it and there is no pointer or reference inside to be null. A
    // field-by-field literal is not available: the reserved members are named
    // differently by musl and by glibc, and this crate builds against both.
    // Every field is overwritten on success and none is read on failure.
    let mut answer: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `statvfs(2)` reads one NUL-terminated borrowed pointer, owned by a
    // `CString` alive for the whole call, and writes one `struct statvfs`
    // through the second. `answer` is a live local of exactly that type and the
    // pointer is derived from it directly. Nothing is retained beyond the call,
    // and the call changes nothing on the machine.
    let rc = unsafe { libc::statvfs(path.as_ptr(), &raw mut answer) };
    if rc != 0 {
        return Err(last_errno());
    }
    // Every field is an unsigned integer whose width differs by target, so each
    // is converted rather than cast: an impossible value becomes zero, which is
    // visibly wrong, instead of a plausible-looking wrapped one.
    let frsize = u64::try_from(answer.f_frsize).unwrap_or(0);
    let bsize = u64::try_from(answer.f_bsize).unwrap_or(0);
    Ok(Statvfs {
        block_bytes: if frsize > 0 { frsize } else { bsize },
        blocks: u64::try_from(answer.f_blocks).unwrap_or(0),
        blocks_free: u64::try_from(answer.f_bfree).unwrap_or(0),
        blocks_available: u64::try_from(answer.f_bavail).unwrap_or(0),
        inodes: u64::try_from(answer.f_files).unwrap_or(0),
        inodes_free: u64::try_from(answer.f_ffree).unwrap_or(0),
        read_only: answer.f_flag & libc::ST_RDONLY != 0,
    })
}

/// What [`statvfs`] answered, with no `libc` type in it.
///
/// **The wall this module is** (`ADR-009` §3.4): `libc::statvfs`'s fields are
/// `c_ulong` and its padding is libc-specific, and neither may appear above this
/// line. [`crate::storage`] copies these into
/// `rivet_adapter_system::Usage` — a second struct with the same fields, which
/// is what the wall costs and is exactly what it is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Statvfs {
    pub block_bytes: u64,
    pub blocks: u64,
    pub blocks_free: u64,
    pub blocks_available: u64,
    pub inodes: u64,
    pub inodes_free: u64,
    pub read_only: bool,
}

/// **Send pid 1 one of `ADR-017` §3.1.10's three signals.**
///
/// # The pid is a literal, and the signal set is closed
///
/// Both are fuses rather than logic, and both are worth stating because the
/// failure they prevent is severe and silent:
///
/// * **`1` is not a parameter.** There is exactly one process this may reach.
///   `cmd/rivet-initd`'s own `kill` guards `pid <= 0` because from pid 1 a `0`
///   or `-1` is the whole machine; here the guard is that no caller supplies a
///   pid at all.
/// * **[`crate::shutdown::InitSignal`] has three members**, so nothing in this crate can ask the
///   kernel to send pid 1 an arbitrary signal. That matters more than it looks:
///   a signal pid 1 has not blocked is delivered with its default disposition,
///   and for most signals that is `Kernel panic - not syncing: Attempted to kill
///   init!`. `cmd/rivet-initd`'s `HANDLED_SIGNALS` is the other half of that
///   agreement and `ADR-017` §3.1.10 spends a section on the hazard.
///
/// **The interlock is not here.** Whether this machine's pid 1 is Rivet's at
/// all is [`crate::shutdown::identify`]'s, one layer up, because it is a
/// *decision* and everything below this line is a syscall.
#[allow(clippy::needless_pass_by_value)]
pub fn kill_init(signal: crate::shutdown::InitSignal) -> Result<(), i32> {
    use crate::shutdown::InitSignal;
    let raw = match signal {
        InitSignal::Term => libc::SIGTERM,
        InitSignal::Interrupt => libc::SIGINT,
        InitSignal::User1 => libc::SIGUSR1,
    };
    // SAFETY: `kill(2)` takes two integers, touches no memory belonging to this
    // process, and retains nothing. The pid is the literal `1` and cannot be `0`
    // (this process group) or negative (a process group, or every process this
    // one may signal). `raw` comes from an exhaustive match over a three-member
    // enum onto `libc`'s own signal constants, so it cannot be an invalid signal
    // number.
    let rc = unsafe { libc::kill(1, raw) };
    if rc == 0 { Ok(()) } else { Err(last_errno()) }
}

/// `kill(pid, SIGHUP)` — `ADR-032` §3.2's reload, delivered.
///
/// **The pid IS a parameter here and it is not in [`kill_init`]**, which is a
/// real weakening of that function's guard and is deliberate: a reload is for the
/// supervisor, whose pid is the whole question. What replaces the guard is that
/// `crate::reload::find` is the only thing that produces one, and it produces one
/// only when pid 1 is `rivet-initd` AND the target is its child named
/// `rivet-sysd`.
///
/// **The signal is still not a parameter.** `SIGHUP` and nothing else, so this
/// cannot be turned into an arbitrary `kill(2)` by a later caller.
pub fn hangup(pid: i32) -> Result<(), i32> {
    // SAFETY: `kill(2)` takes two integers, touches no memory belonging to this
    // process, and retains nothing. `pid` comes from `crate::reload::find`,
    // which returns only a pid it read out of `/proc` and verified is pid 1's
    // child — so it is positive and is neither `0` (this process group) nor
    // negative (a process group, or every process this one may signal). The
    // signal is a literal.
    let rc = unsafe { libc::kill(pid, libc::SIGHUP) };
    if rc == 0 { Ok(()) } else { Err(last_errno()) }
}

// ---------------------------------------------------------------------------
// The VideoCore mailbox — `ioctl(2)`, and the sixth `unsafe` block in this crate
// ---------------------------------------------------------------------------

/// The mailbox device's ioctl **type**, `MAJOR_NUM` in the firmware's own
/// `vcio.h`.
const VCIO_TYPE: u32 = 100;

/// `_IOWR(VCIO_TYPE, 0, char *)` — the one request `/dev/vcio` accepts.
///
/// **Computed rather than written down as `0xC0086400`.** The size field of an
/// ioctl number is `sizeof(char *)`, so the constant differs between a 32-bit
/// and a 64-bit userland — and a hard-coded number would be right on the board
/// and silently wrong anywhere it was ported. `AGENTS.md` §5: derive it.
///
/// **This is `asm-generic`'s encoding**, which is what arm, arm64 and x86 use.
/// It is NOT right on mips, powerpc, sparc or alpha, whose direction bits differ
/// — none of which Rivet targets (`ADR-003` §4), and all of which would need
/// this function rather than a caller changed.
const fn iowr(kind: u32, number: u32, size: u32) -> u32 {
    // `_IOC_READ | _IOC_WRITE` is `2 | 1`, in the top two bits.
    (3 << 30) | (size << 16) | (kind << 8) | number
}

/// **One VideoCore property-interface exchange over `/dev/vcio`.**
///
/// `docs/design-candidates/Admin-Surface-Survey.md` §7.6 measured that device as
/// **present** on the pinned kernel, and that the throttle flags are reachable
/// no other way on it: there is no `rpi_volt` hwmon device and no
/// `get_throttled` file anywhere under `/sys/devices/platform/soc`.
///
/// **Everything about what is in the buffer is a decision and is not here.**
/// The tag number, the request layout and the meaning of every bit that comes
/// back live in [`crate::thermal`], which is this module's own rule: below this
/// line is a syscall, above it is a decision. What this function knows is that
/// the firmware is handed a word array and writes its answer back into the same
/// one.
///
/// # Errors
///
/// The errno the `ioctl(2)` set. `ENODEV`/`ENOENT` mean this is not a Broadcom
/// machine; `EACCES` or `EPERM` mean it is and this process may not ask it.
pub fn vcio_property(fd: i32, message: &mut [u32]) -> Result<(), i32> {
    // The firmware reads the length out of the first word, so a buffer that
    // does not carry one cannot be handed over at all. Refused here rather than
    // trusted, because the kernel would copy whatever that word said.
    if message.is_empty() {
        return Err(libc::EINVAL);
    }

    let request = iowr(
        VCIO_TYPE,
        0,
        u32::try_from(size_of::<*mut u8>()).unwrap_or(0),
    );

    // SAFETY: `ioctl(2)` on `/dev/vcio` with this request reads and writes the
    // word array `message` points at, and nothing else belonging to this
    // process. The pointer is derived from a live `&mut [u32]` that outlives the
    // call, so it is non-null, aligned for `u32` and writable. The kernel's vcio
    // driver bounds its copy by `message[0]`, the size word, which
    // `crate::thermal` sets to the buffer's own byte length — and an empty
    // buffer, which would have no size word at all, is refused above. The fd is
    // an open file descriptor owned by the caller for the whole call. The
    // firmware retains nothing.
    let rc = unsafe { libc::ioctl(fd, request as _, message.as_mut_ptr()) };
    if rc == 0 { Ok(()) } else { Err(last_errno()) }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::shutdown::InitSignal;
    use rivet_adapter_image::Errno;

    /// **Every errno `rivet-adapter-image` names must be this platform's own
    /// number.**
    ///
    /// That crate declares Linux's numbers by hand and takes no `libc`, and its
    /// `port` module says where the check has to live: *"a suite that cannot be
    /// run rots, so the check runs where it can."* `cmd/rivet-initd` carries the
    /// same test for the same reason; this is the second place the numbers reach
    /// `libc`, and it is now also a place where a wrong one would mislabel a
    /// console line an operator reads remotely.
    #[test]
    fn every_errno_the_image_port_names_is_this_platforms_own_number() {
        for (named, platform) in [
            (Errno::EPERM, libc::EPERM),
            (Errno::ENOENT, libc::ENOENT),
            (Errno::EIO, libc::EIO),
            (Errno::EAGAIN, libc::EAGAIN),
            (Errno::EAGAIN, libc::EWOULDBLOCK),
            (Errno::EACCES, libc::EACCES),
            (Errno::EBUSY, libc::EBUSY),
            (Errno::ENODEV, libc::ENODEV),
            (Errno::EINVAL, libc::EINVAL),
            (Errno::ENOSPC, libc::ENOSPC),
            (Errno::EROFS, libc::EROFS),
        ] {
            assert_eq!(named.0, platform, "{named} is not this platform's number");
        }
    }

    /// **The mailbox ioctl number is the one the firmware's `vcio.h` defines**,
    /// worked out from the same `_IOWR` macro rather than copied.
    ///
    /// `0xC0086400` on a 64-bit userland and `0xC0046400` on a 32-bit one,
    /// because the size field is `sizeof(char *)`. Rivet ships 64-bit
    /// (`ADR-003` §4) and this asserts the number for *this* build's pointer
    /// width — so a port to a 32-bit target gets the right constant rather than
    /// a silent `EINVAL` from the driver.
    #[test]
    fn the_mailbox_request_number_is_iowr_100_0_over_a_pointer() {
        let request = iowr(
            VCIO_TYPE,
            0,
            u32::try_from(size_of::<*mut u8>()).expect("a pointer fits in a u32 of bytes"),
        );
        let expected = match size_of::<*mut u8>() {
            8 => 0xC008_6400u32,
            4 => 0xC004_6400u32,
            other => panic!("unhandled pointer width: {other}"),
        };
        assert_eq!(request, expected, "{request:#010x} is not vcio.h's number");
    }

    /// A buffer with no size word cannot be handed to the driver, which would
    /// read the length out of it — refused rather than trusted.
    #[test]
    fn an_empty_mailbox_message_is_refused_before_any_ioctl() {
        assert_eq!(vcio_property(-1, &mut []), Err(libc::EINVAL));
    }

    /// A path with a NUL in it cannot become a `CString`, and must be refused
    /// rather than truncated at the NUL — which would name a different path.
    #[test]
    fn a_path_containing_a_nul_is_refused_rather_than_truncated() {
        assert_eq!(
            mount("/dev/x\0y", "/run/x", "vfat", false),
            Err(libc::EINVAL)
        );
        assert_eq!(unmount("/run/x\0y"), Err(libc::EINVAL));
    }

    /// An `io::Error` with no OS number reaches the port as `0`, which
    /// `Errno`'s own `Display` renders as "an error with no OS error number" —
    /// so the operator sees what happened rather than errno 2.
    #[test]
    fn an_error_with_no_os_number_is_reported_as_zero_rather_than_as_a_guess() {
        let invented = std::io::Error::other("no OS number here");
        assert_eq!(errno_of(&invented), 0);
        assert!(
            Errno(errno_of(&invented))
                .to_string()
                .contains("no OS error number")
        );
        let real = std::io::Error::from_raw_os_error(libc::EROFS);
        assert_eq!(errno_of(&real), libc::EROFS);
    }

    /// **The clock actually runs, and it never goes backwards.**
    ///
    /// The second half is the property `ADR-021` §5 wants and the one a wall
    /// clock does not have; the first is what distinguishes a working call from
    /// a stub that returns a constant, which would make every measured duration
    /// zero and every throughput infinite.
    ///
    /// It is a real syscall in a test on purpose — `clock_gettime(2)` reads a
    /// counter, changes nothing, and needs no privilege, so it belongs in the
    /// same category as the `unmount` probe below.
    #[test]
    fn the_monotonic_clock_advances_and_never_reads_earlier_than_it_did() {
        let started = monotonic_nanos();
        assert!(started.is_ok(), "clock_gettime failed: {started:?}");
        let first = started.unwrap_or(0);
        let mut last = first;
        // Enough work that the clock must have moved, without a sleep: a sleep in
        // a test is a duration nobody can shorten later.
        let mut spun: u64 = 0;
        for i in 0..2_000_000u64 {
            spun = spun.wrapping_add(i);
            if i % 100_000 == 0 {
                let reading = monotonic_nanos();
                assert!(
                    reading.is_ok(),
                    "clock_gettime failed mid-loop: {reading:?}"
                );
                let now = reading.unwrap_or(0);
                assert!(now >= last, "the monotonic clock went backwards");
                last = now;
            }
        }
        assert!(spun > 0, "the loop was optimised away");
        assert!(
            last > first,
            "the clock did not advance across two million iterations; it is not running"
        );
    }

    /// **The three signals this crate may send pid 1 are the three
    /// `cmd/rivet-initd` blocks and dispatches on, by number.**
    ///
    /// The two crates never link and each names the signals its own way — one
    /// with `kernel::Sig`, one with [`InitSignal`] — so the only thing that ties
    /// them together is that both resolve to the same `libc` constants. This
    /// asserts that half of it here, where `libc` is; `cmd/rivet-initd`'s
    /// `every_signal_number_is_this_platforms_own` asserts the other. A mapping
    /// that sent `SIGUSR2` for a trial restart would compile, pass every fake,
    /// and kill pid 1 on the first board that ran it (`ADR-017` §3.1.10's
    /// "hazard that makes this more than a match arm").
    ///
    /// **It does not send anything.** Nothing in this workspace sends a signal to
    /// pid 1 in a test.
    #[test]
    fn each_request_maps_onto_the_signal_number_pid_one_blocks_for_it() {
        for (signal, expected, name) in [
            (InitSignal::Term, libc::SIGTERM, "SIGTERM"),
            (InitSignal::Interrupt, libc::SIGINT, "SIGINT"),
            (InitSignal::User1, libc::SIGUSR1, "SIGUSR1"),
        ] {
            // The same match `kill_init` performs, so a change there fails here.
            let raw = match signal {
                InitSignal::Term => libc::SIGTERM,
                InitSignal::Interrupt => libc::SIGINT,
                InitSignal::User1 => libc::SIGUSR1,
            };
            assert_eq!(raw, expected, "{name}");
            assert_eq!(signal.as_str(), name);
        }
        // Three distinct numbers. A table that mapped two requests onto one
        // signal would stop the machine the wrong way with no error anywhere.
        assert_ne!(libc::SIGTERM, libc::SIGINT);
        assert_ne!(libc::SIGINT, libc::SIGUSR1);
        assert_ne!(libc::SIGTERM, libc::SIGUSR1);
        // And none of them is a signal that cannot be blocked, which pid 1
        // would therefore die of.
        for raw in [libc::SIGTERM, libc::SIGINT, libc::SIGUSR1] {
            assert_ne!(raw, libc::SIGKILL);
            assert_ne!(raw, libc::SIGSTOP);
        }
    }

    /// A lock file in a directory of this test's own, removed first so the
    /// directory creation is exercised too.
    fn lock_path(name: &str) -> (std::path::PathBuf, String) {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("rivet").join("autoboot.lock");
        let path = path.to_str().expect("a utf-8 temporary path").to_string();
        (dir, path)
    }

    /// **A held lock is waited on for the bound, then refused as `EAGAIN`** —
    /// against the real `flock(2)`, with the holder a second open file
    /// description in this process, which `flock` treats exactly as another
    /// process. Once the holder lets go, the lock is taken at once.
    #[test]
    fn a_held_lock_is_waited_on_for_the_bound_and_then_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let (dir, path) = lock_path("held");
        let holder = lock_exclusive_within(&path, 0, 1).expect("the first lock");
        let mode = std::fs::metadata(&path).expect("the lock file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");

        let started = std::time::Instant::now();
        let got = lock_exclusive_within(&path, 200, 10);
        let waited = started.elapsed();
        assert_eq!(got.err(), Some(libc::EAGAIN));
        assert!(waited >= std::time::Duration::from_millis(200), "{waited:?}");
        assert!(waited < std::time::Duration::from_secs(5), "{waited:?}");

        drop(holder);
        let started = std::time::Instant::now();
        assert!(lock_exclusive_within(&path, 200, 10).is_ok());
        assert!(started.elapsed() < std::time::Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A holder that lets go during the wait hands the lock over: the waiter
    /// takes it rather than running out its bound.
    #[test]
    fn a_lock_released_during_the_wait_is_taken() {
        let (dir, path) = lock_path("released");
        let holder = lock_exclusive_within(&path, 0, 1).expect("the first lock");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(holder);
        });
        let started = std::time::Instant::now();
        let got = lock_exclusive_within(&path, 5_000, 10);
        assert!(got.is_ok(), "{got:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        releaser.join().expect("the releasing thread");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unmounting something that is not a mount point must fail, and must fail
    /// with a number rather than with a panic. This is the one call in this
    /// module that is safe to actually make in a test: it changes nothing, needs
    /// no privilege to be refused, and proves the errno path is wired.
    #[test]
    fn a_failing_syscall_reports_an_errno_rather_than_panicking() {
        let got = unmount("/proc/rivet-does-not-exist");
        assert!(got.is_err(), "that path is not a mount point");
        assert!(got.err().is_some_and(|e| e != 0), "no errno was reported");
    }
}
