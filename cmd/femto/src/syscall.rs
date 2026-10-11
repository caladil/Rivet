//! **The wall around `libc`.** `ADR-009` §3.4.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! The same split `cmd/rivet`'s `syscall` module and
//! `cmd/rivet-sysd`'s `kernel` module make, and the same rule their
//! manifests state: *no `libc` type appears outside this module.* What leaves
//! here is `(u16, u16)`, `()` and a bare `i32` errno.
//!
//! # Three calls, and only three
//!
//! `tcgetattr(3)`, `tcsetattr(3)` and `ioctl(TIOCGWINSZ)`. Reading input and
//! writing output are `std::io`'s — `read(2)` and `write(2)` have perfectly
//! good `std` equivalents and `ADR-009` §2's reuse posture is about reaching
//! for the implementation that already exists.
//!
//! What has no `std` equivalent is *turning the line discipline off*, and
//! without it there is no editor at all: the terminal would still be buffering
//! a line at a time and echoing it, so no arrow key would ever arrive and every
//! character would appear twice. The three calls are what
//! [`RawMode`] is made of.
//!
//! # `RawMode` restores on drop, and that is the load-bearing part
//!
//! A program that leaves a terminal in raw mode leaves the operator with a
//! session that does not echo and does not respond to `^C` — on a machine
//! reached remotely, that costs the session. So the original `termios` is
//! captured, held, and written back by `Drop`, which runs on every ordinary
//! path out of `main` including `?`.
//!
//! **It does not run on a panic that aborts**, and `panic = "abort"` is set for
//! the release profile (root `Cargo.toml`). That is the reason this crate sets
//! `unwrap_used`, `expect_used` and `panic` to `deny` rather than to the
//! workspace's `warn`: with `abort`, an unwind-based cleanup is not available
//! and *not panicking* is the mechanism.
//!
//! # Linux only, and it refuses elsewhere rather than pretending
//!
//! This module is `#[cfg(target_os = "linux")]`. The development host is
//! Windows (`AGENTS.md` §10.5), and on any other platform `crate::host`
//! supplies a terminal that **refuses** — never one that succeeds
//! (`AGENTS.md` §3). Every test in this crate runs on both.

#![cfg(target_os = "linux")]

/// The errno the last failing call set, or `0` if the platform did not give
/// one. `0` is never a real errno.
fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// The terminal's settings, put back when this is dropped.
///
/// Held as an opaque `libc::termios` so that nothing outside this module can
/// name one — the wall this module's documentation describes.
pub struct RawMode {
    original: libc::termios,
}

/// The descriptor every call here uses.
///
/// Standard **input**, for both the line discipline and the window size. Not
/// stdout, because an editor whose keyboard is a terminal and whose output has
/// been redirected is a program with nothing to draw on, and asking the same
/// descriptor both questions means the two answers cannot be about different
/// terminals. A bare `0` rather than `libc::STDIN_FILENO` outside this module
/// is the wall this file is: no `libc` name leaves it.
const TTY: i32 = 0;

impl RawMode {
    /// Put the terminal into raw mode, returning the guard that restores it.
    ///
    /// **`cfmakeraw` is not used**, deliberately. It clears `OPOST`, which
    /// turns off output post-processing, and then every `\n` this program
    /// writes moves down a line without returning to column 0. The renderer
    /// would have to emit `\r\n` everywhere, which is a rendering concern
    /// leaking out of `crate::screen` and into a decision about a terminal
    /// flag. Clearing exactly the input flags that matter and leaving `OPOST`
    /// alone is both smaller and easier to say why.
    pub fn enable() -> Result<RawMode, i32> {
        // SAFETY: `termios` is a plain-old-data struct with no invalid bit
        // patterns for the fields `tcgetattr` writes; zeroing it gives a valid
        // value to hand the kernel a pointer to. `tcgetattr` fills it or fails,
        // and the failure is checked before anything reads the contents.
        let mut original: libc::termios = unsafe { core::mem::zeroed() };
        // SAFETY: `TTY` is standard input, open for the life of the process, and
        // `&mut original` is a valid, uniquely borrowed, correctly typed and
        // correctly aligned pointer for the duration of the call. The kernel
        // writes only into it and retains nothing.
        if unsafe { libc::tcgetattr(TTY, &raw mut original) } != 0 {
            return Err(last_errno());
        }

        let mut raw = original;
        // ICRNL: stop the terminal turning `\r` into `\n`, so that Return can be
        //        told from `^J`.
        // IXON:  stop `^S`/`^Q` being eaten as flow control; `^S` is a key here.
        // ICANON: no line buffering — a byte arrives when it is pressed.
        // ECHO:  this program draws the screen; the terminal must not also.
        // ISIG:  `^C` is a key this editor binds (Cancel), not a signal.
        // IEXTEN: stop `^V` being taken as literal-next; `^V` is Page Down.
        raw.c_iflag &= !(libc::ICRNL | libc::IXON);
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        // One byte is enough to return, and never time out. A blocking read is
        // what this program wants: there is nothing to do between keystrokes,
        // and a poll loop would spin a CPU on a board that has four of them.
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        // SAFETY: as above. `TCSAFLUSH` waits for pending output and discards
        // pending input, so no byte typed before the switch is read as if it
        // had been typed after it.
        if unsafe { libc::tcsetattr(TTY, libc::TCSAFLUSH, &raw const raw) } != 0 {
            return Err(last_errno());
        }
        Ok(RawMode { original })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // SAFETY: `TTY` was open when `enable` succeeded and this struct is
        // not `Clone`, so this is the one restore. `&self.original` is a valid
        // pointer to a `termios` the kernel itself produced. A failure here
        // cannot be reported — `Drop` returns nothing — and is deliberately
        // ignored rather than panicked on: a panic in a destructor during
        // unwinding aborts, and aborting while restoring a terminal is strictly
        // worse than a terminal that was not restored.
        unsafe {
            libc::tcsetattr(TTY, libc::TCSAFLUSH, &raw const self.original);
        }
    }
}

/// Nanoseconds on `CLOCK_MONOTONIC` — `ADR-022` §3.2's `mono`.
///
/// **A fourth call, added 2026-08-24 with `ADR-022` §11's journalling.** The
/// module's *"three calls, and only three"* heading above is superseded by this
/// one rather than edited, because the count going stale is the shape of failure
/// `AGENTS.md` §7 is about and the reason is worth seeing.
///
/// It is here rather than `std::time::Instant` because `Instant` is an opaque
/// monotone point with no origin: it can measure a duration and cannot say
/// *seconds since boot*, which is the field `ADR-021` §5 names and the one that
/// makes a record orderable on a machine with no RTC. `cmd/rivet`'s
/// `syscall::monotonic_nanos` is the same call for the same reason, and the two
/// crates cannot share it — neither has a library target.
///
/// It reads a counter, writes one `timespec`, and changes nothing.
pub fn monotonic_nanos() -> Result<u128, i32> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime(2)` writes one `struct timespec` through the
    // pointer it is given and retains nothing beyond the call. `ts` is a live
    // local of exactly that type, and the pointer is derived from it directly.
    // `CLOCK_MONOTONIC` is a clock id every Linux implements.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    if rc != 0 {
        return Err(last_errno());
    }
    // Converted rather than cast, so an impossible value becomes zero instead of
    // a very large number: a duration of zero is visibly wrong and a wrapped one
    // looks like a plausible measurement.
    let seconds = u128::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u128::try_from(ts.tv_nsec).unwrap_or(0);
    Ok(seconds * 1_000_000_000 + nanos)
}

/// The terminal's size in cells, as `(rows, cols)`.
///
/// **Asked of the kernel every frame**, never cached — `AGENTS.md` §5's *the
/// kernel is the authority for system state; never cache its verdicts*, which
/// applies to a window an operator can drag as much as to a network interface.
pub fn window_size() -> Result<(u16, u16), i32> {
    // SAFETY: `winsize` is plain-old-data; zeroing it is a valid value.
    let mut size: libc::winsize = unsafe { core::mem::zeroed() };
    // SAFETY: `TTY` is standard input, open, and `TIOCGWINSZ` is documented to take
    // a pointer to a `winsize`, which is what is passed. The kernel writes only
    // into it and retains nothing.
    if unsafe { libc::ioctl(TTY, libc::TIOCGWINSZ, &raw mut size) } != 0 {
        return Err(last_errno());
    }
    Ok((size.ws_row, size.ws_col))
}
