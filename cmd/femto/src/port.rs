//! The two seams, and the fakes that stand in for them.
//!
//! **Both are the second kind of seam `AGENTS.md` §3.1 describes** — an
//! adapter's own I/O seam, not a domain port. Their vocabulary is bytes, cells
//! and `std::io::Error`, which is exactly the test that section gives: if a
//! trait's signatures mention things `rivet-core` is forbidden to name, it
//! belongs in the crate that adapts them and not in the core. Neither of these
//! is anywhere near `rivet-core`, and neither should ever move there.
//!
//! They exist for the reason `cmd/rivet-sysd`'s `Kernel` does: the
//! branches that matter cannot be reached from a test otherwise. A pty, a
//! terminal that resizes mid-edit, and a filesystem that refuses a write with
//! `EROFS` are three states a developer's machine does not reach on demand —
//! and the last of those is the single most important behaviour in this
//! program (`crate::view`), so a suite that could not drive it would be testing
//! everything except the point.

use crate::screen::{Frame, Size};

/// Everything this program needs from a terminal.
///
/// Three methods, which is the whole of what `crossterm` would have been
/// depended on for — see the manifest.
pub trait Terminal {
    /// How big the screen is **now**. Asked once per frame rather than cached,
    /// because a window that is resized between two keystrokes is the ordinary
    /// case and `AGENTS.md` §5's *never cache the kernel's verdicts* is the
    /// same rule pointed at a terminal.
    fn size(&mut self) -> Size;

    /// The next byte of input, or `None` at end of input.
    ///
    /// **Bytes rather than keys**, so that `crate::key`'s escape-sequence
    /// decoding sits above the seam and is driven by every test that drives the
    /// loop. A port that handed back keys would put the decoder on the
    /// untestable side of the wall for no gain.
    fn read_byte(&mut self) -> std::io::Result<Option<u8>>;

    /// Draw a frame.
    fn present(&mut self, frame: &Frame) -> std::io::Result<()>;
}

/// Everything this program needs from a filesystem.
pub trait Files {
    /// The file's bytes, or `None` if it does not exist.
    ///
    /// **`None` rather than an error**, because opening a name that is not
    /// there yet is how a file gets created and is not a failure. Every other
    /// way a read can fail — a directory, a permission, an I/O error — stays an
    /// error, so "there is no such file" and "I could not read the file" do not
    /// arrive as the same answer (`AGENTS.md` §5).
    fn read(&mut self, path: &str) -> std::io::Result<Option<Vec<u8>>>;

    /// Replace the file's contents.
    fn save(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()>;
}

#[cfg(test)]
pub mod testing {
    //! Fakes. Compiled only under `cfg(test)`.

    use std::collections::BTreeMap;

    use super::{Files, Terminal};
    use crate::screen::{Frame, Size};

    /// A filesystem in a `BTreeMap`, which can also be told to refuse.
    #[derive(Debug, Default)]
    pub struct MemoryFiles {
        pub files: BTreeMap<String, Vec<u8>>,
        /// Paths whose writes fail with `EROFS`, so that the read-only case is
        /// driven rather than described.
        pub read_only: Vec<String>,
        /// Every `save` that was attempted, in order — including the refused
        /// ones, because "it did not try to write" and "it tried and was
        /// refused" are different bugs.
        pub attempts: Vec<String>,
    }

    impl MemoryFiles {
        pub fn with(path: &str, body: &str) -> MemoryFiles {
            let mut files = MemoryFiles::default();
            files.files.insert(path.to_owned(), body.as_bytes().to_vec());
            files
        }

        /// What is on disk now, as text.
        pub fn text(&self, path: &str) -> Option<String> {
            self.files
                .get(path)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        }
    }

    impl Files for MemoryFiles {
        fn read(&mut self, path: &str) -> std::io::Result<Option<Vec<u8>>> {
            Ok(self.files.get(path).cloned())
        }

        fn save(&mut self, path: &str, bytes: &[u8]) -> std::io::Result<()> {
            self.attempts.push(path.to_owned());
            if self.read_only.iter().any(|p| p == path) {
                return Err(std::io::Error::from_raw_os_error(30)); // EROFS
            }
            self.files.insert(path.to_owned(), bytes.to_vec());
            Ok(())
        }
    }

    /// A terminal with a scripted keyboard, which keeps every frame it was
    /// asked to draw.
    ///
    /// Frames are kept whole rather than reduced to the last one, because the
    /// interesting assertion is often about a frame in the middle — the one
    /// where the prompt was on screen, or the one that said `VIEW` before
    /// anything was typed.
    #[derive(Debug)]
    pub struct ScriptedTerminal {
        input: std::collections::VecDeque<u8>,
        pub frames: Vec<Frame>,
        pub size: Size,
    }

    impl ScriptedTerminal {
        /// Type these bytes, then end the input.
        ///
        /// **Ending the input rather than looping forever is what makes the
        /// driver testable at all**: `crate::run` returns when the terminal has
        /// nothing left to say, so a script that never presses `^X` still
        /// terminates and its last frame can be asserted on.
        pub fn typing(bytes: &[u8]) -> ScriptedTerminal {
            ScriptedTerminal {
                input: bytes.iter().copied().collect(),
                frames: Vec::new(),
                size: Size { rows: 12, cols: 60 },
            }
        }

        /// The frame drawn last, which is what the operator would be looking at.
        pub fn last(&self) -> Frame {
            self.frames.last().cloned().unwrap_or_default()
        }

        /// Every line of text that appeared on screen at any point.
        pub fn everything_seen(&self) -> String {
            self.frames
                .iter()
                .map(Frame::text)
                .collect::<Vec<String>>()
                .join("\n")
        }
    }

    impl Terminal for ScriptedTerminal {
        fn size(&mut self) -> Size {
            self.size
        }

        fn read_byte(&mut self) -> std::io::Result<Option<u8>> {
            Ok(self.input.pop_front())
        }

        fn present(&mut self, frame: &Frame) -> std::io::Result<()> {
            self.frames.push(frame.clone());
            Ok(())
        }
    }
}
