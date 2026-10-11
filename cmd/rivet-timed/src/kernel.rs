//! The seam between this binary's decisions and the operating system.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! `AGENTS.md` §3.1's table is the placement rule and this trait is squarely in
//! its right-hand column — **an adapter's own I/O seam, not a domain port**. Its
//! methods mention `Errno` and a monotonic clock, which `rivet-core` is
//! forbidden to name, and its purpose is to let *this* adapter be tested without
//! the platform it adapts. It therefore lives here and not in the core, and the
//! empty `[dependencies]` table of `rivet-core` is the gate that makes that
//! structural rather than aspirational (`AGENTS.md` §10.1).
//!
//! # Why the clock is behind a port at all
//!
//! Because `clock_settime` is the one call in this workspace that **cannot be
//! executed by a test**. A test that succeeded would set the clock of the
//! machine running the suite — the development host, or a CI runner, or the
//! board. `cmd/rivet-sysd/src/kernel.rs` says exactly this about its own
//! copy, and it is the whole reason [`crate::discipline`] is a set of pure
//! functions returning a *decision* rather than code that moves a clock: the
//! decision is what is worth testing, and it is the part that can be.
//!
//! # This trait is the third of its shape in this workspace
//!
//! `cmd/rivet-initd` has a `Kernel`, `cmd/rivet-sysd` has a `Kernel`,
//! and this is the third. `Errno` is *identical* in all three. `ADR-013` §7
//! ruled that the shared platform crate emerges at Stage 3 out of accumulated
//! evidence rather than being built speculatively, and this file adds a data
//! point with a shape worth recording:
//!
//! **The overlap between this port and the supervisor's is four methods —
//! `now_ms`, `wall_ms`, `set_wall_ms`, `emit` — and they are the four that
//! `ADR-021` and `ADR-022` between them define.** That is a *narrower* and more
//! specific claim than "the ports want sharing", which the supervisor's own note
//! says is false. It is not the process-management half that recurs; it is the
//! clock-and-record half, which is what `ADR-013` §7's crate would exist for.
//!
//! # What is deliberately not here
//!
//! * **No signal handling.** The supervisor spawns children with the signal
//!   mask cleared (measured, `cmd/rivet-sysd/src/kernel.rs`), so
//!   `SIGTERM` arrives with its default disposition and terminates this process
//!   during its sleep. A handler would buy the ability to finish a poll in
//!   flight, which is worth nothing: a poll produces a clock correction, and a
//!   machine that is shutting down does not need one.
//! * **No `/state` access of any kind.** See [`crate::run`] — this binary is
//!   deliberately not a second writer of `/state/clock`.

use std::io::Write as _;

use crate::journal::Record;

/// Where the kernel publishes the boot identifier — `ADR-022` §3.2's `boot`
/// field. The same path `cmd/rivet-sysd` reads, and a second spelling
/// of it rather than a shared constant for the reason this module opens with.
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// A raw OS error number, kept as data so a test can produce any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    /// `clock_settime` without `CAP_SYS_TIME`. Named for the fakes and for
    /// the tests that drive them: the real adapter never constructs one, it
    /// reports whatever the platform set (`AGENTS.md` §5).
    #[allow(dead_code)]
    pub const EPERM: Self = Self(libc::EPERM);
    /// Likewise — what `read_file` answers for an absent boot id.
    #[allow(dead_code)]
    pub const ENOENT: Self = Self(libc::ENOENT);
    /// What `set_wall_ms` returns for a time this platform cannot represent,
    /// which is the one errno this crate produces rather than reports.
    pub const EINVAL: Self = Self(libc::EINVAL);

    /// The errno the last syscall set.
    pub fn last() -> Self {
        Self::from_io(&std::io::Error::last_os_error())
    }

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

/// Every operating-system fact this binary depends on, except the one UDP
/// exchange — which is [`crate::sntp::Sntp`], because it is a different seam
/// over a different piece of the world and a fake for it has to answer with
/// protocol values rather than with `Errno`.
///
/// Object-safe on purpose: the logic takes `&mut dyn Kernel`, so there is one
/// compiled copy of the decisions and the fake drives exactly the code the board
/// runs.
pub trait Kernel {
    /// Milliseconds from a monotonic clock.
    ///
    /// **Every deadline in this crate is expressed in this and none in wall
    /// time**, which is `ADR-021` §5's rule and which matters here more than
    /// anywhere else in the workspace: this is the process that *moves* the wall
    /// clock, so a poll interval or a rate limit measured in wall time would be
    /// reset, doubled or skipped by its own correction.
    fn now_ms(&mut self) -> u64;

    /// Milliseconds since the Unix epoch, or `None` if there is no readable
    /// wall clock.
    fn wall_ms(&mut self) -> Option<u64>;

    /// Set the wall clock — `ADR-021` §4.2.1's step, and the only call in this
    /// crate that moves it.
    ///
    /// **It moves time in both directions**, which is the difference from
    /// `cmd/rivet-sysd`'s identically named method: that one implements
    /// `ADR-021` §3's *floor* and is reached only when the stored value is
    /// later, so it only ever moves forwards. A correction from a time server
    /// is authoritative in a way a stored floor is not, so a machine whose clock
    /// is ahead of real time is moved **back**. See [`crate::discipline`], which
    /// is where the two guards on that live.
    ///
    /// A refusal is survivable and must be said: on Linux this needs
    /// `CAP_SYS_TIME`, so `EPERM` is what a process without it gets.
    fn set_wall_ms(&mut self, unix_ms: u64) -> Result<(), Errno>;

    /// The journal's sink. Cannot fail: there is nowhere to report a failed
    /// report to.
    fn emit(&mut self, record: &Record);

    fn read_file(&mut self, path: &str) -> Result<String, Errno>;

    /// Sleep for this many milliseconds.
    ///
    /// On the port so that a poll interval of seventeen minutes is tested in
    /// microseconds. `AGENTS.md` §4 — a suite that cannot be run rots, and a
    /// suite that takes seventeen minutes to reach its second assertion will
    /// not be run.
    fn sleep_ms(&mut self, ms: u64);
}

// ---------------------------------------------------------------------------
// The real one. Thin by policy: nothing here is reachable from a test.
// ---------------------------------------------------------------------------

/// Split Unix epoch milliseconds into a `timespec`'s two fields, or `None` if
/// this platform's `timespec` cannot hold them.
///
/// A free function rather than two lines inline, for the reason
/// `cmd/rivet-sysd` gives about its identical copy: `LibcKernel` is
/// where nothing is reachable from a test, and the **conversion** is the part
/// that can be wrong. Its correctness also varies by target — `time_t` is
/// 64-bit on `aarch64` and 32-bit on the armv6 of `AGENTS.md` §10.2's deferred
/// compatibility target.
///
/// Deliberately **not** tested by calling `clock_settime`: a test that succeeded
/// would have set the clock of the machine running the suite.
fn timespec_parts(unix_ms: u64) -> Option<(libc::time_t, libc::c_long)> {
    let secs = <libc::time_t>::try_from(unix_ms / 1000).ok()?;
    let nanos = <libc::c_long>::try_from((unix_ms % 1000) * 1_000_000).ok()?;
    Some((secs, nanos))
}

/// How this binary renders records to the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleForm {
    /// `ADR-022` §3 — the on-disk form, and the authoritative one.
    Jsonl,
    /// A *view* of it (`ADR-022` §2), for a human at the HDMI console.
    Text,
}

/// The operating system, via `libc` and `std`.
pub struct LibcKernel {
    form: ConsoleForm,
}

impl LibcKernel {
    pub const fn new(form: ConsoleForm) -> Self {
        Self { form }
    }
}

impl Kernel for LibcKernel {
    fn now_ms(&mut self) -> u64 {
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
        secs.saturating_mul(1000)
            .saturating_add(nanos / 1_000_000)
    }

    fn wall_ms(&mut self) -> Option<u64> {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
    }

    fn set_wall_ms(&mut self, unix_ms: u64) -> Result<(), Errno> {
        let Some((tv_sec, tv_nsec)) = timespec_parts(unix_ms) else {
            // A value this platform's `timespec` cannot hold. `EINVAL` is what
            // `clock_settime(2)` itself returns for an out-of-range time, so
            // reporting the same number keeps one vocabulary.
            return Err(Errno::EINVAL);
        };
        let ts = libc::timespec { tv_sec, tv_nsec };
        // SAFETY: `clock_settime` reads one `timespec` through a pointer to a
        // live local and retains nothing. `CLOCK_REALTIME` is a valid, settable
        // clock id on every Linux this targets, and `tv_nsec` is in range by
        // construction — `timespec_parts` returns `None` otherwise.
        let rc = unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &raw const ts) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn emit(&mut self, record: &Record) {
        let line = match self.form {
            ConsoleForm::Jsonl => record.to_jsonl(),
            ConsoleForm::Text => format!("{}\n", record.to_text()),
        };
        // A closed console must not become the reason this process dies, so both
        // results are dropped deliberately.
        let mut out = std::io::stdout();
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        std::fs::read_to_string(path).map_err(|e| Errno::from_io(&e))
    }

    fn sleep_ms(&mut self, ms: u64) {
        // `std::thread::sleep` is `nanosleep(2)` on Linux and is **interrupted
        // by a signal**, which is the behaviour wanted: `SIGTERM` from the
        // supervisor terminates the process out of this call rather than after
        // seventeen minutes. `std` restarts the sleep on `EINTR` only for a
        // signal whose handler returns, and this process installs none.
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn errno_zero_says_so_rather_than_inventing_a_message() {
        assert_eq!(Errno(0).to_string(), "error with no OS error number");
    }

    #[test]
    fn errno_display_carries_the_number_the_platform_used() {
        assert!(
            Errno::EPERM
                .to_string()
                .contains(&Errno::EPERM.0.to_string())
        );
    }

    /// The split `clock_settime` is handed. Whole seconds down, the remainder in
    /// nanoseconds, and never a `tv_nsec` outside `0..1_000_000_000` — which the
    /// kernel rejects with `EINVAL`.
    #[test]
    fn epoch_milliseconds_split_into_whole_seconds_and_a_nanosecond_remainder() {
        assert_eq!(timespec_parts(0), Some((0, 0)));
        assert_eq!(timespec_parts(1), Some((0, 1_000_000)));
        assert_eq!(timespec_parts(999), Some((0, 999_000_000)));
        assert_eq!(timespec_parts(1_000), Some((1, 0)));
        assert_eq!(
            timespec_parts(1_700_000_000_123),
            Some((1_700_000_000, 123_000_000))
        );
        for ms in [0, 1, 999, 1_000, 1_001, 1_700_000_000_123, u64::from(u32::MAX)] {
            let Some((secs, nanos)) = timespec_parts(ms) else {
                unreachable!("{ms} is representable and was refused")
            };
            assert!(
                (0..1_000_000_000).contains(&nanos),
                "{ms} gave tv_nsec {nanos}"
            );
            assert!(secs >= 0);
        }
    }

    /// A value this platform cannot represent is **refused**, not truncated into
    /// some other year. Written against the platform's own limit rather than a
    /// literal, because on every target Rivet builds for today `time_t` is
    /// 64-bit and the limit cannot be reached at all.
    #[test]
    fn a_time_this_platform_cannot_hold_is_refused_rather_than_wrapped() {
        let most_seconds = u64::MAX / 1000;
        let representable = <libc::time_t>::try_from(most_seconds).is_ok();
        eprintln!(
            "this platform's time_t is {} bytes and {} hold {most_seconds} seconds",
            std::mem::size_of::<libc::time_t>(),
            if representable { "can" } else { "cannot" }
        );
        assert_eq!(
            timespec_parts(u64::MAX).is_some(),
            representable,
            "the refusal must follow the platform's own limit rather than a guess"
        );
    }

    /// The monotonic clock is monotonic, read through the real adapter. It
    /// proves nothing about the wall clock and is not meant to — it is the one
    /// assertion about `LibcKernel` that can be made without setting anything.
    #[test]
    fn the_real_monotonic_clock_does_not_go_backwards() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let first = k.now_ms();
        k.sleep_ms(2);
        let second = k.now_ms();
        assert!(second >= first, "{first} then {second}");
    }

    /// The boot identifier really is where this crate says it is, on whatever
    /// Linux the suite runs on — and the value is never asserted, only its
    /// readability, because `AGENTS.md` §8.3 keeps machine identifiers out of
    /// tracked files.
    #[test]
    fn the_boot_id_path_is_readable_on_a_linux_host() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        match k.read_file(BOOT_ID_PATH) {
            Ok(text) => assert_eq!(
                text.trim().len(),
                36,
                "a boot id is a 36-character UUID; this host gave {} characters",
                text.trim().len()
            ),
            Err(e) => {
                // Printed rather than asserted: a host without procfs is not
                // this test's failure to report, and a silent skip is what
                // `AGENTS.md` §7 warns about.
                eprintln!("this host could not read {BOOT_ID_PATH}: {e}");
            }
        }
    }
}
