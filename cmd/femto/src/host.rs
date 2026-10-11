//! The real terminal and the real filesystem — `crate::port`'s two traits, over
//! the machine this is running on.
//!
//! This is the composition root's other half: `crate::main` decides *what* to
//! do and this decides *how it reaches the world*, which is the same
//! arrangement `cmd/rivet`'s `boot` module has over its `syscall` module.
//!
//! **On any platform that is not Linux, [`terminal`] refuses.** It does not
//! return a terminal that quietly does nothing — `AGENTS.md` §3: *"an
//! unsupported-platform implementation returns an error. Never `Ok(())` — a
//! `run` that succeeded without running reports a state that is not true."*
//! The development host is Windows (`AGENTS.md` §10.5) and every test in this
//! crate runs there, which is only true because nothing above this file knows
//! which platform it is on.

use std::io::Write as _;

use crate::port::{Files, Terminal};
use crate::screen::{Frame, Size};
#[cfg(target_os = "linux")]
use crate::screen::{Colour, Style};

/// A terminal for this machine, or the errno that says why not.
///
/// The returned value owns the raw-mode guard, so dropping the terminal
/// restores the line discipline. That is why this is one object rather than a
/// `setup`/`teardown` pair: there is no path out of `main` that skips it.
#[cfg(target_os = "linux")]
pub fn terminal() -> std::io::Result<AnsiTerminal> {
    let raw = crate::syscall::RawMode::enable().map_err(std::io::Error::from_raw_os_error)?;
    let mut terminal = AnsiTerminal {
        _raw: raw,
        out: std::io::stdout(),
        pending: Vec::new(),
    };
    // The alternate screen, so that quitting puts back whatever the operator
    // was looking at. A terminal that does not implement it ignores the
    // sequence, which is why it is safe to send unconditionally.
    terminal.emit("\x1b[?1049h\x1b[2J")?;
    Ok(terminal)
}

/// The refusal, on every platform that is not Linux.
#[cfg(not(target_os = "linux"))]
pub fn terminal() -> std::io::Result<AnsiTerminal> {
    Err(std::io::Error::other(
        "femto puts the terminal into raw mode with tcsetattr(3), which this platform does \
         not have. It is built for aarch64-unknown-linux-musl (ADR-003 §4); on a development \
         host, `cargo test` drives the whole editor without one.",
    ))
}

/// A terminal that speaks ANSI. **The only place in this crate that emits an
/// escape sequence**, which is why `crate::screen` can be tested on text.
#[cfg(target_os = "linux")]
pub struct AnsiTerminal {
    _raw: crate::syscall::RawMode,
    out: std::io::Stdout,
    /// The frame under construction. One `write(2)` per frame rather than one
    /// per span: a serial console at 115200 baud draws a torn screen otherwise,
    /// and this program is expected to run on one.
    pending: Vec<u8>,
}

/// The stand-in the non-Linux build compiles, so that `crate::main` has one
/// shape on every platform.
///
/// **Nothing constructs it** — [`terminal`] refuses first — and its methods
/// refuse anyway, because a stand-in whose `present` returned `Ok(())` would be
/// a screen that reported having been drawn (`AGENTS.md` §3).
#[cfg(not(target_os = "linux"))]
pub struct AnsiTerminal;

#[cfg(not(target_os = "linux"))]
impl Terminal for AnsiTerminal {
    fn size(&mut self) -> Size {
        Size::FALLBACK
    }

    fn read_byte(&mut self) -> std::io::Result<Option<u8>> {
        Err(std::io::Error::other("this platform has no raw-mode terminal"))
    }

    fn present(&mut self, _frame: &Frame) -> std::io::Result<()> {
        Err(std::io::Error::other("this platform has no raw-mode terminal"))
    }
}

#[cfg(target_os = "linux")]
impl AnsiTerminal {
    fn emit(&mut self, text: &str) -> std::io::Result<()> {
        self.out.write_all(text.as_bytes())?;
        self.out.flush()
    }
}

#[cfg(target_os = "linux")]
impl Drop for AnsiTerminal {
    fn drop(&mut self) {
        // Leave the alternate screen and make the cursor visible again. Errors
        // are ignored for the reason `crate::syscall`'s `Drop` gives: there is
        // nowhere to report them and failing loudly here is worse.
        let _ = self.out.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = self.out.flush();
    }
}

#[cfg(target_os = "linux")]
impl Terminal for AnsiTerminal {
    fn size(&mut self) -> Size {
        match crate::syscall::window_size() {
            // A kernel that answers `0` for either dimension has answered
            // "I do not know" — which several pseudo-terminals do at the
            // moment they are created. Taking it literally would divide the
            // screen into nothing.
            Ok((rows, cols)) if rows > 0 && cols > 0 => Size { rows, cols },
            _ => Size::FALLBACK,
        }
    }

    fn read_byte(&mut self) -> std::io::Result<Option<u8>> {
        use std::io::Read as _;
        let mut byte = [0u8; 1];
        loop {
            return match std::io::stdin().read(&mut byte) {
                Ok(0) => Ok(None),
                Ok(_) => Ok(Some(byte[0])),
                // `EINTR` is not end-of-input and must not be reported as one:
                // a signal that interrupts the read would otherwise close the
                // editor and take the buffer with it. Go round again.
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => Err(error),
            };
        }
    }

    fn present(&mut self, frame: &Frame) -> std::io::Result<()> {
        self.pending.clear();
        // Hide the cursor for the duration of the draw, so it is not seen
        // sweeping across the screen a row at a time.
        self.pending.extend_from_slice(b"\x1b[?25l");
        for (index, row) in frame.rows.iter().enumerate() {
            // Absolute positioning per row rather than newlines: it is
            // independent of `OPOST`, it cannot scroll the screen by writing
            // past the last line, and it is one sequence either way.
            self.pending
                .extend_from_slice(format!("\x1b[{};1H", index + 1).as_bytes());
            for span in row {
                self.pending.extend_from_slice(sgr(span.style).as_bytes());
                self.pending.extend_from_slice(span.text.as_bytes());
            }
            // Reset, then clear to the end of the line — in that order, or the
            // clear paints the rest of the row in the span's background colour.
            self.pending.extend_from_slice(b"\x1b[0m\x1b[K");
        }
        self.pending.extend_from_slice(
            format!("\x1b[{};{}H\x1b[?25h", frame.cursor.0 + 1, frame.cursor.1 + 1).as_bytes(),
        );
        self.out.write_all(&self.pending)?;
        self.out.flush()
    }
}

/// One `Style` as a Select Graphic Rendition sequence.
///
/// Always begins with a reset, so a style is absolute rather than relative to
/// whatever the previous span left behind — a renderer whose colours depend on
/// draw order is one that eventually paints a title bar red.
#[cfg(target_os = "linux")]
fn sgr(style: Style) -> String {
    let mut codes = vec!["0".to_owned()];
    if style.bold {
        codes.push("1".to_owned());
    }
    if let Some(fg) = style.fg {
        codes.push(ansi(fg, false).to_string());
    }
    if let Some(bg) = style.bg {
        codes.push(ansi(bg, true).to_string());
    }
    format!("\x1b[{}m", codes.join(";"))
}

/// The SGR number for a colour, as foreground or background.
#[cfg(target_os = "linux")]
fn ansi(colour: Colour, background: bool) -> u8 {
    let base = match colour {
        Colour::Black => 30,
        Colour::Red => 31,
        Colour::Yellow => 33,
        Colour::Blue => 34,
        Colour::BrightBlack => 90,
        Colour::BrightYellow => 93,
        Colour::BrightWhite => 97,
    };
    if background { base + 10 } else { base }
}

/// The filesystem.
pub struct HostFiles;

impl Files for HostFiles {
    fn read(&mut self, path: &str) -> std::io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Write **in place**, then `fsync`.
    ///
    /// # Why not write a temporary file and rename it
    ///
    /// The atomic-rename idiom is the usual advice and it is the wrong choice
    /// *here*, for a reason specific to this machine rather than a general one:
    /// a rename replaces the inode, so the new file has the creating process's
    /// ownership and a fresh mode derived from the umask. On Rivet the files
    /// an operator edits include key files a daemon refuses to honour if their
    /// mode is wrong — so the tidy idiom would lock a remote machine out.
    ///
    /// The cost, said plainly rather than left to be discovered: **a power cut
    /// between the truncate and the write leaves a short file.** That is a real
    /// risk on a board with no battery, and it is the smaller one — a mangled
    /// file is visible and recoverable, a key file silently ignored is neither.
    /// `nano` makes the same trade by default and for the same reason.
    ///
    /// The `fsync` is what makes the write reach the card rather than the page
    /// cache, which matters on a machine whose usual way of stopping is having
    /// its power removed.
    fn save(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
}
