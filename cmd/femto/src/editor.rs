//! Every key, and what it does. **The whole of this file is testable without a
//! terminal**, which is the point of it being a file.
//!
//! `crate::text` owns what an edit *is*; this owns what a keystroke *means* —
//! the cursor, the viewport, the cut buffer, the prompts, and the one rule that
//! makes this Rivet's editor: **a write to a view is refused, with the
//! generator and the `change-with`, at the moment it is attempted.**
//!
//! # Modeless, because `nano` is
//!
//! There is no command mode and no insert mode. Arrows move, typing inserts,
//! and control keys act — and there is exactly one transient state, the prompt
//! on the status line, which `nano` has too and which is always visible and
//! always cancelled with `^C` or `Escape`. A `nano` user who has to think about
//! which mode they are in has been failed.
//!
//! `Editor::press` and `Editor::handles` are `cfg(test)` helpers, and
//! `Editor::handles` is not a second list of keys: it runs the same
//! [`Editor::dispatch`] the real loop runs and reports whether the arm existed.
//! `crate::screen`'s `every_advertised_key_is_handled` uses it to keep the help
//! lines honest, which is `ADR-006` §7's "a catalogue that names something that
//! does not exist" applied to two rows of a screen.

use crate::key::Key;
use crate::port::Files;
use crate::screen::{Size, display_col, gutter_width};
use crate::text::{Cursor, Text};
use crate::view::{self, Opening};

/// Whether the program keeps going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    Quit,
}

/// The one transient state, on the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Prompt {
    /// `^O`. The answer starts as the current path, exactly as `nano`'s does.
    Write(String),
    /// `^W`.
    Search(String),
    /// `^_`.
    Goto(String),
    /// `^X` with unsaved changes. The one question this program insists on.
    SaveBeforeQuit,
}

/// **One write that happened**, for `crate::journal` to describe.
///
/// It is a value rather than a call because this module performs no I/O of its
/// own beyond `crate::port::Files` — the record goes to a socket, which is a
/// third seam, and threading it through `Editor::key` would put it on the path
/// of every keystroke to serve one of them. So the write is *reported* and
/// `crate::run` files it, which is where the machine already lives.
///
/// Everything in it is something this program knows for certain. There is no
/// `created` field: `crate::open` knows whether the file existed when it was
/// opened, and a `^O` to a third name several minutes later is a different
/// question — inventing an answer to it would be `AGENTS.md` §5's guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrote {
    /// What was written.
    pub path: String,
    /// The name the buffer was opened under, which is usually the same.
    pub opened_as: String,
    /// Whether those two differ — a `^O` to a new name.
    pub renamed: bool,
    pub bytes: usize,
    pub lines: usize,
}

/// The editor.
#[derive(Debug)]
pub struct Editor {
    path: String,
    text: Text,
    /// What `crate::view` said about [`Editor::path`] when it was opened, and
    /// what a successful write to a different name replaces.
    opening: Opening,
    cursor: Cursor,
    /// First visible line, and first visible **display column** — cells, not
    /// characters, because a tab is one of the first and eight of the second.
    top: usize,
    left: usize,
    /// `^K`'s accumulator. Consecutive cuts append, exactly as `nano`'s do, so
    /// that `^K ^K ^K ^U` moves three lines rather than one.
    cut: Vec<String>,
    cutting: bool,
    /// `(text, is_alert)`. The bool picks the colour, and it is a bool rather
    /// than a `Style` so that this module never names a colour.
    message: Option<(String, bool)>,
    prompt: Option<Prompt>,
    /// What `^W` searches for when the answer is left empty — `nano`'s
    /// `Search [last]:`.
    last_search: String,
    help: bool,
    /// The last write, until `crate::run` takes it. `None` the rest of the time,
    /// which is nearly always.
    wrote: Option<Wrote>,
}

impl Editor {
    /// Open a buffer. The classification is passed in rather than computed here
    /// so that `crate::main` does it once, before any I/O, and so that every
    /// test can open a view without one existing.
    pub fn open(path: String, text: Text, opening: Opening) -> Editor {
        let message = match &opening {
            // `ADR-007` §3.6.1 item 2 is the argument for saying this now: the
            // header is the highest-value mechanism "because it is read by
            // exactly the person about to make the mistake — they have the file
            // open". Four of the six generated files cannot carry one — the
            // three colon-separated databases and, since 2026-08-26, /etc/motd
            // — so for those this line is the only provenance an operator ever
            // sees.
            Opening::ReadOnly(refusal) => Some((refusal.headline.clone(), false)),
            // `nano`'s own words, and it says them for the same reason: a write
            // that changes bytes the operator did not type has to be announced
            // before the write, not after.
            Opening::Editable if text.was_converted() => {
                Some(("Converted from DOS format".to_owned(), false))
            }
            Opening::Editable => None,
        };
        Editor {
            path,
            text,
            opening,
            cursor: Cursor::START,
            top: 0,
            left: 0,
            cut: Vec::new(),
            cutting: false,
            message,
            prompt: None,
            last_search: String::new(),
            help: false,
            wrote: None,
        }
    }

    /// Take the last write, if there was one since this was last asked.
    ///
    /// Taken rather than read, so a write is journalled exactly once: a second
    /// caller gets `None`, and a keystroke that is not `^O` cannot re-file the
    /// one before it.
    pub fn take_wrote(&mut self) -> Option<Wrote> {
        self.wrote.take()
    }

    /// Say, on the status line, that the write is not in the journal.
    ///
    /// **At the moment of the write**, rather than only in the lines printed
    /// after the editor exits: an operator who saves ten files on a machine
    /// whose supervisor has died should see it on the first one, not on the way
    /// out.
    pub fn note_unjournalled(&mut self) {
        let said = match self.message.take() {
            Some((text, _)) => format!("{text} — NOT journalled"),
            None => "NOT journalled".to_owned(),
        };
        self.message = Some((said, true));
    }

    // -- what `crate::screen` asks -------------------------------------------

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn text(&self) -> &Text {
        &self.text
    }

    pub fn opening(&self) -> &Opening {
        &self.opening
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn left(&self) -> usize {
        self.left
    }

    pub fn help_is_open(&self) -> bool {
        self.help
    }

    /// `(text, is_alert)`, or nothing to say.
    pub fn message(&self) -> Option<(&str, bool)> {
        self.message
            .as_ref()
            .map(|(text, alert)| (text.as_str(), *alert))
    }

    /// `(label, what has been typed)`.
    pub fn prompt(&self) -> Option<(String, &str)> {
        match self.prompt.as_ref()? {
            Prompt::Write(answer) => Some(("File Name to Write:".to_owned(), answer.as_str())),
            Prompt::Search(answer) => Some((
                if self.last_search.is_empty() {
                    "Search:".to_owned()
                } else {
                    format!("Search [{}]:", self.last_search)
                },
                answer.as_str(),
            )),
            Prompt::Goto(answer) => Some(("Enter line number:".to_owned(), answer.as_str())),
            Prompt::SaveBeforeQuit => Some((
                "Save modified buffer?  Y Yes  N No  ^C Cancel".to_owned(),
                "",
            )),
        }
    }

    // -- the loop ------------------------------------------------------------

    /// Keep the cursor on screen. Called once per frame, before rendering.
    ///
    /// Horizontal scrolling is in **cells**, so a line of tabs scrolls by what
    /// is drawn rather than by what is stored.
    pub fn scroll(&mut self, size: Size) {
        let rows = size.text_rows();
        if self.cursor.line < self.top {
            self.top = self.cursor.line;
        }
        if self.cursor.line >= self.top + rows {
            self.top = self.cursor.line + 1 - rows;
        }
        let cols = size
            .text_cols()
            .saturating_sub(gutter_width(self))
            .max(1);
        let at = display_col(self.text.line(self.cursor.line), self.cursor.col);
        if at < self.left {
            self.left = at;
        }
        if at >= self.left + cols {
            self.left = at + 1 - cols;
        }
    }

    /// One keystroke.
    pub fn key(&mut self, key: Key, size: Size, files: &mut dyn Files) -> Step {
        self.dispatch(key, size, files).0
    }

    /// The dispatch, and the `bool` is *whether this key meant anything*.
    ///
    /// Returned rather than discarded so that the help lines can be checked
    /// against the real key map instead of against a copy of it.
    fn dispatch(&mut self, key: Key, size: Size, files: &mut dyn Files) -> (Step, bool) {
        // Help swallows the next key, whatever it is. `nano` does the same.
        if self.help {
            self.help = false;
            return (Step::Continue, true);
        }
        if self.prompt.is_some() {
            return self.in_prompt(key, files);
        }

        // `^K` accumulates only while it is the key being pressed. Any other
        // key ends the run, which is what makes a second cut somewhere else
        // replace the buffer rather than append to it.
        if !matches!(key, Key::Ctrl('K')) {
            self.cutting = false;
        }

        let handled = match key {
            // -- leaving ----------------------------------------------------
            Key::Ctrl('X') => {
                if self.text.is_dirty() {
                    self.prompt = Some(Prompt::SaveBeforeQuit);
                    self.message = None;
                } else {
                    return (Step::Quit, true);
                }
                true
            }

            // -- the file ---------------------------------------------------
            Key::Ctrl('O') => {
                self.prompt = Some(Prompt::Write(self.path.clone()));
                self.message = None;
                true
            }

            // -- finding ----------------------------------------------------
            Key::Ctrl('W') => {
                self.prompt = Some(Prompt::Search(String::new()));
                self.message = None;
                true
            }
            Key::Ctrl('_') => {
                self.prompt = Some(Prompt::Goto(String::new()));
                self.message = None;
                true
            }

            // -- saying where you are ---------------------------------------
            Key::Ctrl('C') => {
                self.report_position();
                true
            }
            Key::Ctrl('G') => {
                self.help = true;
                true
            }
            // `nano` redraws on `^L`. Here it also clears the status line,
            // which is the other half of what somebody pressing it wants.
            Key::Ctrl('L') => {
                self.message = None;
                true
            }

            // -- changing the buffer ----------------------------------------
            Key::Ctrl('K') => {
                self.cut_line();
                true
            }
            Key::Ctrl('U') => {
                self.uncut();
                true
            }
            Key::Char(c) => {
                self.edit(|text, at| text.insert(at, c));
                true
            }
            Key::Tab => {
                self.edit(|text, at| text.insert(at, '\t'));
                true
            }
            Key::Enter => {
                self.edit(Text::insert_newline);
                true
            }
            Key::Backspace => {
                self.edit(Text::delete_before);
                true
            }
            Key::Delete | Key::Ctrl('D') => {
                self.edit(Text::delete_at);
                true
            }

            // -- moving ------------------------------------------------------
            Key::Left => {
                self.cursor = self.step_left();
                true
            }
            Key::Right => {
                self.cursor = self.step_right();
                true
            }
            Key::Up => {
                self.move_line(-1);
                true
            }
            Key::Down => {
                self.move_line(1);
                true
            }
            Key::Home | Key::Ctrl('A') => {
                self.cursor.col = 0;
                true
            }
            Key::End | Key::Ctrl('E') => {
                self.cursor.col = self.text.line_len(self.cursor.line);
                true
            }
            Key::PageUp | Key::Ctrl('Y') => {
                self.move_line(-isize_of(size.text_rows()));
                true
            }
            Key::PageDown | Key::Ctrl('V') => {
                self.move_line(isize_of(size.text_rows()));
                true
            }

            Key::Escape => true,
            Key::Ctrl(_) | Key::Unknown => {
                self.message = Some((
                    "Unknown key. ^G lists the ones this editor has.".to_owned(),
                    false,
                ));
                false
            }
        };

        (Step::Continue, handled)
    }

    /// A keystroke while the status line is asking a question.
    fn in_prompt(&mut self, key: Key, files: &mut dyn Files) -> (Step, bool) {
        let Some(prompt) = self.prompt.clone() else {
            return (Step::Continue, false);
        };

        // `^C` and `Escape` cancel any prompt, always. A prompt that could be
        // entered and not left is the modal trap this editor does not have.
        if matches!(key, Key::Ctrl('C') | Key::Escape) {
            self.prompt = None;
            self.message = Some(("Cancelled".to_owned(), false));
            return (Step::Continue, true);
        }

        if let Prompt::SaveBeforeQuit = prompt {
            return match key {
                Key::Char('y' | 'Y') => {
                    self.prompt = Some(Prompt::Write(self.path.clone()));
                    (Step::Continue, true)
                }
                Key::Char('n' | 'N') => {
                    self.prompt = None;
                    (Step::Quit, true)
                }
                // Anything else leaves the question on screen, which is what
                // `nano` does and is the right answer for a question whose
                // wrong answer discards the operator's work.
                _ => (Step::Continue, true),
            };
        }

        let answer = match &prompt {
            Prompt::Write(a) | Prompt::Search(a) | Prompt::Goto(a) => a.clone(),
            Prompt::SaveBeforeQuit => String::new(),
        };

        match key {
            Key::Enter => {
                self.prompt = None;
                match prompt {
                    Prompt::Write(_) => self.write_out(&answer, files),
                    Prompt::Search(_) => self.search(&answer),
                    Prompt::Goto(_) => self.goto(&answer),
                    Prompt::SaveBeforeQuit => {}
                }
                (Step::Continue, true)
            }
            Key::Backspace => {
                let mut answer = answer;
                answer.pop();
                self.prompt = Some(refill(&prompt, answer));
                (Step::Continue, true)
            }
            Key::Char(c) => {
                let mut answer = answer;
                answer.push(c);
                self.prompt = Some(refill(&prompt, answer));
                (Step::Continue, true)
            }
            _ => (Step::Continue, true),
        }
    }

    // -- the acts -------------------------------------------------------------

    /// **The rule this whole program exists to enforce.**
    ///
    /// The target is classified at the moment of the write rather than at the
    /// moment of the open, and that is what makes `^O` useful in a view: the
    /// contents of `/etc/passwd` can be written to `/state/home/root/passwd.txt`
    /// — a copy of a view is not a view — while a write *back* to the view is
    /// refused with the generator and the `change-with`.
    ///
    /// `ADR-007` §3.6: "the write is refused. Correct, but only if the refusal
    /// explains itself." A bare `EROFS` from the kernel would be the useless
    /// half of a true answer, and this refusal happens before any syscall so
    /// the operator never sees it.
    fn write_out(&mut self, target: &str, files: &mut dyn Files) {
        if target.is_empty() {
            self.message = Some(("No file name given".to_owned(), true));
            return;
        }
        if let Opening::ReadOnly(refusal) = view::classify(target) {
            self.message = Some((format!("{} — ^G for all of it", refusal.headline), true));
            return;
        }
        let bytes = self.text.to_bytes();
        match files.save(target, &bytes) {
            Ok(()) => {
                self.text.mark_saved();
                let lines = self.text.line_count();
                // **Reported before the path is replaced**, so that a `^O` to a
                // new name records the name it was opened under as well as the
                // one it landed on — which is the whole of what makes a journal
                // record of an interactive editor worth reading.
                self.wrote = Some(Wrote {
                    path: target.to_owned(),
                    opened_as: self.path.clone(),
                    renamed: target != self.path,
                    bytes: bytes.len(),
                    lines,
                });
                if target != self.path {
                    self.path = target.to_owned();
                    // The buffer is now this file, so it is now this file's
                    // classification — which is how "save a copy of a view, then
                    // edit the copy" works without a second command.
                    self.opening = view::classify(target);
                }
                self.message = Some((
                    format!("Wrote {lines} line{}", if lines == 1 { "" } else { "s" }),
                    false,
                ));
            }
            // The kernel's own words, never a paraphrase — `AGENTS.md` §5.
            Err(error) => {
                self.message = Some((format!("Error writing {target}: {error}"), true));
            }
        }
    }

    fn search(&mut self, answer: &str) {
        let needle = if answer.is_empty() {
            self.last_search.clone()
        } else {
            answer.to_owned()
        };
        if needle.is_empty() {
            self.message = Some(("Nothing to search for".to_owned(), false));
            return;
        }
        match self.text.find(self.cursor, &needle) {
            Some(at) => {
                self.cursor = at;
                self.message = None;
            }
            None => self.message = Some((format!("\"{needle}\" not found"), false)),
        }
        self.last_search = needle;
    }

    fn goto(&mut self, answer: &str) {
        match answer.trim().parse::<usize>() {
            Ok(line) if line >= 1 => {
                self.cursor = self.text.clamp(Cursor {
                    line: line - 1,
                    col: 0,
                });
                self.message = None;
            }
            // Refuse rather than guess — `AGENTS.md` §5. Jumping somewhere
            // arbitrary because the number did not parse is the kind of quiet
            // wrong answer that gets an operator editing the wrong line.
            _ => self.message = Some((format!("\"{answer}\" is not a line number"), true)),
        }
    }

    fn cut_line(&mut self) {
        if self.refuse_if_view() {
            return;
        }
        match self.text.cut_line(self.cursor) {
            Some(line) => {
                if !self.cutting {
                    self.cut.clear();
                }
                self.cut.push(line);
                self.cutting = true;
                self.cursor = self.text.clamp(self.cursor);
                self.message = None;
            }
            None => self.message = Some(("Nothing to cut".to_owned(), false)),
        }
    }

    fn uncut(&mut self) {
        if self.refuse_if_view() {
            return;
        }
        if self.cut.is_empty() {
            self.message = Some(("The cut buffer is empty".to_owned(), false));
            return;
        }
        let cut = self.cut.clone();
        self.cursor = self.text.insert_lines(self.cursor, &cut);
        self.message = None;
    }

    /// Every act that would change the buffer goes through here, so the view
    /// refusal is in one place rather than at eight call sites.
    fn edit(&mut self, act: impl FnOnce(&mut Text, Cursor) -> Cursor) {
        if self.refuse_if_view() {
            return;
        }
        self.cursor = act(&mut self.text, self.cursor);
        self.message = None;
    }

    /// `true` when the act must not happen, having said why.
    fn refuse_if_view(&mut self) -> bool {
        let Opening::ReadOnly(refusal) = &self.opening else {
            return false;
        };
        self.message = Some((format!("{} — ^G for all of it", refusal.headline), true));
        true
    }

    fn report_position(&mut self) {
        let line = self.cursor.line + 1;
        let lines = self.text.line_count();
        let col = self.cursor.col + 1;
        let width = self.text.line_len(self.cursor.line);
        let bytes = self.text.to_bytes().len();
        self.message = Some((
            format!("line {line}/{lines}, col {col}/{}, {bytes} bytes", width + 1),
            false,
        ));
    }

    // -- movement -------------------------------------------------------------

    fn step_left(&self) -> Cursor {
        if self.cursor.col > 0 {
            return Cursor {
                col: self.cursor.col - 1,
                ..self.cursor
            };
        }
        if self.cursor.line == 0 {
            return self.cursor;
        }
        let line = self.cursor.line - 1;
        Cursor {
            line,
            col: self.text.line_len(line),
        }
    }

    fn step_right(&self) -> Cursor {
        if self.cursor.col < self.text.line_len(self.cursor.line) {
            return Cursor {
                col: self.cursor.col + 1,
                ..self.cursor
            };
        }
        if self.cursor.line + 1 >= self.text.line_count() {
            return self.cursor;
        }
        Cursor {
            line: self.cursor.line + 1,
            col: 0,
        }
    }

    /// Up and down, and the page keys, which are the same act by a bigger step.
    ///
    /// The column is **clamped rather than remembered**, which is `nano`'s
    /// behaviour: moving down through a short line and back up leaves the
    /// cursor at the short line's end. An editor that restored the column would
    /// be more clever and less like the thing it is imitating.
    fn move_line(&mut self, by: isize) {
        let count = isize_of(self.text.line_count());
        let line = (isize_of(self.cursor.line) + by).clamp(0, count.saturating_sub(1));
        self.cursor = self.text.clamp(Cursor {
            line: usize::try_from(line).unwrap_or(0),
            col: self.cursor.col,
        });
    }

    // -- test helpers ---------------------------------------------------------

    /// Press one key against a filesystem that refuses everything.
    ///
    /// For the tests that are about the buffer and the screen rather than about
    /// writing. A test that needs a write drives [`Editor::key`] with
    /// `crate::port::testing::MemoryFiles`, and the refusal here is
    /// `AGENTS.md` §3's *stubs refuse; they never fake success* — a helper that
    /// silently succeeded would let a test think it had saved.
    #[cfg(test)]
    pub fn press(&mut self, key: Key) -> Step {
        self.key(key, Size::FALLBACK, &mut Refusing)
    }

    /// Whether this key means anything. Runs the real dispatch.
    #[cfg(test)]
    pub fn handles(&mut self, key: Key) -> bool {
        self.dispatch(key, Size::FALLBACK, &mut Refusing).1
    }
}

/// Put an edited answer back into the prompt it came from.
fn refill(prompt: &Prompt, answer: String) -> Prompt {
    match prompt {
        Prompt::Write(_) => Prompt::Write(answer),
        Prompt::Search(_) => Prompt::Search(answer),
        Prompt::Goto(_) => Prompt::Goto(answer),
        Prompt::SaveBeforeQuit => Prompt::SaveBeforeQuit,
    }
}

/// `usize` to `isize`, saturating. A buffer with more lines than `isize::MAX`
/// does not fit in memory, so this cannot be reached — but it is written as a
/// clamp rather than a cast so that it cannot wrap if it ever is.
fn isize_of(value: usize) -> isize {
    isize::try_from(value).unwrap_or(isize::MAX)
}

/// A filesystem that refuses. `cfg(test)` only — see [`Editor::press`].
#[cfg(test)]
struct Refusing;

#[cfg(test)]
impl Files for Refusing {
    fn read(&mut self, _path: &str) -> std::io::Result<Option<Vec<u8>>> {
        Err(std::io::Error::from_raw_os_error(1))
    }

    fn save(&mut self, _path: &str, _bytes: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(1))
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::testing::MemoryFiles;

    const SIZE: Size = Size { rows: 12, cols: 40 };

    fn open(path: &str, body: &str) -> Editor {
        Editor::open(
            path.to_owned(),
            Text::from_bytes(body.as_bytes()).expect("the fixture is text"),
            view::classify(path),
        )
    }

    fn type_text(editor: &mut Editor, text: &str) {
        for c in text.chars() {
            editor.press(Key::Char(c));
        }
    }

    fn message(editor: &Editor) -> String {
        editor.message().map(|(t, _)| t.to_owned()).unwrap_or_default()
    }

    fn body(editor: &Editor) -> String {
        String::from_utf8(editor.text().to_bytes()).expect("UTF-8 by construction")
    }

    // -- typing and moving ----------------------------------------------------

    #[test]
    fn typing_inserts_at_the_cursor_with_no_mode_to_enter_first() {
        let mut editor = open("/state/notes.txt", "");
        type_text(&mut editor, "hello");
        assert_eq!(body(&editor), "hello\n");
        assert_eq!(editor.cursor(), Cursor { line: 0, col: 5 });
    }

    #[test]
    fn left_at_the_start_of_a_line_goes_to_the_end_of_the_one_above() {
        let mut editor = open("/state/x", "one\ntwo\n");
        editor.press(Key::Down);
        editor.press(Key::Left);
        assert_eq!(editor.cursor(), Cursor { line: 0, col: 3 });
    }

    #[test]
    fn right_at_the_end_of_a_line_goes_to_the_start_of_the_next() {
        let mut editor = open("/state/x", "one\ntwo\n");
        editor.press(Key::End);
        editor.press(Key::Right);
        assert_eq!(editor.cursor(), Cursor { line: 1, col: 0 });
    }

    #[test]
    fn moving_down_through_a_short_line_clamps_the_column_the_way_nano_does() {
        let mut editor = open("/state/x", "longer line\nab\nlonger line\n");
        editor.press(Key::End);
        editor.press(Key::Down);
        assert_eq!(editor.cursor(), Cursor { line: 1, col: 2 });
        editor.press(Key::Down);
        assert_eq!(
            editor.cursor(),
            Cursor { line: 2, col: 2 },
            "the column is clamped, not remembered — nano does not restore it"
        );
    }

    #[test]
    fn page_down_moves_by_a_screen_and_stops_at_the_end() {
        let lines: String = (0..50).map(|i| format!("line {i}\n")).collect();
        let mut editor = open("/state/x", &lines);
        editor.key(Key::PageDown, SIZE, &mut MemoryFiles::default());
        assert_eq!(editor.cursor().line, SIZE.text_rows());
        for _ in 0..20 {
            editor.key(Key::PageDown, SIZE, &mut MemoryFiles::default());
        }
        assert_eq!(editor.cursor().line, 49);
    }

    #[test]
    fn the_viewport_follows_the_cursor_down_and_back_up() {
        let lines: String = (0..50).map(|i| format!("line {i}\n")).collect();
        let mut editor = open("/state/x", &lines);
        for _ in 0..20 {
            editor.press(Key::Down);
        }
        editor.scroll(SIZE);
        assert!(editor.top() > 0, "the screen never followed the cursor");
        assert!(editor.cursor().line < editor.top() + SIZE.text_rows());
        for _ in 0..20 {
            editor.press(Key::Up);
        }
        editor.scroll(SIZE);
        assert_eq!(editor.top(), 0);
    }

    // -- cut and paste --------------------------------------------------------

    #[test]
    fn cut_and_uncut_move_a_line() {
        let mut editor = open("/state/x", "one\ntwo\nthree\n");
        editor.press(Key::Ctrl('K'));
        assert_eq!(body(&editor), "two\nthree\n");
        editor.press(Key::Down);
        editor.press(Key::Ctrl('U'));
        assert_eq!(body(&editor), "two\none\nthree\n");
    }

    #[test]
    fn consecutive_cuts_accumulate_into_one_cut_buffer() {
        // `nano`'s behaviour, and the reason `^K ^K ^K ^U` is how a block moves
        // in an editor that has no selection.
        let mut editor = open("/state/x", "a\nb\nc\nd\n");
        editor.press(Key::Ctrl('K'));
        editor.press(Key::Ctrl('K'));
        editor.press(Key::Ctrl('K'));
        assert_eq!(body(&editor), "d\n");
        editor.press(Key::Ctrl('U'));
        assert_eq!(body(&editor), "a\nb\nc\nd\n");
    }

    #[test]
    fn a_cut_interrupted_by_another_key_starts_a_new_cut_buffer() {
        let mut editor = open("/state/x", "a\nb\nc\n");
        editor.press(Key::Ctrl('K')); // takes "a", leaving b, c
        editor.press(Key::Down); // ends the run
        editor.press(Key::Ctrl('K')); // takes "c", leaving b
        editor.press(Key::Ctrl('U'));
        assert_eq!(
            body(&editor),
            "c\nb\n",
            "the second cut replaces the buffer; appending would have put `a` back too"
        );
    }

    #[test]
    fn uncut_with_an_empty_cut_buffer_says_so_rather_than_doing_nothing() {
        let mut editor = open("/state/x", "a\n");
        editor.press(Key::Ctrl('U'));
        assert_eq!(message(&editor), "The cut buffer is empty");
    }

    // -- search ---------------------------------------------------------------

    #[test]
    fn search_moves_the_cursor_to_the_match() {
        let mut editor = open("/state/x", "alpha\nbeta\ngamma\n");
        editor.press(Key::Ctrl('W'));
        type_text(&mut editor, "gam");
        editor.press(Key::Enter);
        assert_eq!(editor.cursor(), Cursor { line: 2, col: 0 });
    }

    #[test]
    fn search_with_an_empty_answer_repeats_the_last_one() {
        let mut editor = open("/state/x", "x\nfind me\ny\nfind me\n");
        editor.press(Key::Ctrl('W'));
        type_text(&mut editor, "find");
        editor.press(Key::Enter);
        assert_eq!(editor.cursor().line, 1);
        editor.press(Key::Ctrl('W'));
        editor.press(Key::Enter);
        assert_eq!(editor.cursor().line, 3, "an empty answer means `again`");
    }

    #[test]
    fn a_search_that_finds_nothing_says_so_and_leaves_the_cursor_alone() {
        let mut editor = open("/state/x", "alpha\n");
        editor.press(Key::Down);
        let before = editor.cursor();
        editor.press(Key::Ctrl('W'));
        type_text(&mut editor, "omega");
        editor.press(Key::Enter);
        assert!(message(&editor).contains("not found"), "{}", message(&editor));
        assert_eq!(editor.cursor(), before);
    }

    #[test]
    fn a_prompt_is_always_cancellable() {
        for cancel in [Key::Ctrl('C'), Key::Escape] {
            let mut editor = open("/state/x", "alpha\n");
            editor.press(Key::Ctrl('W'));
            type_text(&mut editor, "alp");
            editor.press(cancel);
            assert!(editor.prompt().is_none(), "a prompt that cannot be left is a mode");
            assert_eq!(message(&editor), "Cancelled");
        }
    }

    #[test]
    fn backspace_in_a_prompt_edits_the_answer() {
        let mut editor = open("/state/x", "alpha\nbeta\n");
        editor.press(Key::Ctrl('W'));
        type_text(&mut editor, "betX");
        editor.press(Key::Backspace);
        type_text(&mut editor, "a");
        editor.press(Key::Enter);
        assert_eq!(editor.cursor(), Cursor { line: 1, col: 0 });
    }

    // -- go to line -----------------------------------------------------------

    #[test]
    fn go_to_line_goes_to_the_line() {
        let mut editor = open("/state/x", "a\nb\nc\nd\n");
        editor.press(Key::Ctrl('_'));
        type_text(&mut editor, "3");
        editor.press(Key::Enter);
        assert_eq!(editor.cursor(), Cursor { line: 2, col: 0 });
    }

    #[test]
    fn go_to_a_line_number_that_is_not_one_refuses_rather_than_guessing() {
        let mut editor = open("/state/x", "a\nb\n");
        editor.press(Key::Ctrl('_'));
        type_text(&mut editor, "banana");
        editor.press(Key::Enter);
        assert_eq!(editor.cursor(), Cursor::START);
        assert!(message(&editor).contains("not a line number"));
    }

    // -- writing --------------------------------------------------------------

    #[test]
    fn write_out_puts_the_buffer_on_disk_and_clears_modified() {
        let mut files = MemoryFiles::with("/state/notes.txt", "old\n");
        let mut editor = open("/state/notes.txt", "old\n");
        type_text(&mut editor, "new ");
        assert!(editor.text().is_dirty());
        editor.key(Key::Ctrl('O'), SIZE, &mut files);
        editor.key(Key::Enter, SIZE, &mut files);
        assert_eq!(files.text("/state/notes.txt").as_deref(), Some("new old\n"));
        assert!(!editor.text().is_dirty());
        assert_eq!(message(&editor), "Wrote 1 line");
    }

    #[test]
    fn write_out_to_a_different_name_writes_there_and_keeps_the_new_name() {
        let mut files = MemoryFiles::default();
        let mut editor = open("/state/a.txt", "body\n");
        type_text(&mut editor, "x");
        editor.key(Key::Ctrl('O'), SIZE, &mut files);
        for _ in 0.."/state/a.txt".len() {
            editor.key(Key::Backspace, SIZE, &mut files);
        }
        for c in "/state/b.txt".chars() {
            editor.key(Key::Char(c), SIZE, &mut files);
        }
        editor.key(Key::Enter, SIZE, &mut files);
        assert_eq!(files.text("/state/b.txt").as_deref(), Some("xbody\n"));
        assert_eq!(editor.path(), "/state/b.txt");
    }

    /// The kernel's own error, not a paraphrase of it — `AGENTS.md` §5.
    #[test]
    fn a_refused_write_reports_what_the_filesystem_said() {
        let mut files = MemoryFiles::default();
        files.read_only.push("/state/x".to_owned());
        let mut editor = open("/state/x", "body\n");
        type_text(&mut editor, "y");
        editor.key(Key::Ctrl('O'), SIZE, &mut files);
        editor.key(Key::Enter, SIZE, &mut files);
        assert!(message(&editor).starts_with("Error writing /state/x:"));
        assert!(
            editor.text().is_dirty(),
            "a failed write must not mark the buffer clean"
        );
    }

    // -- quitting -------------------------------------------------------------

    #[test]
    fn a_clean_buffer_quits_without_a_question() {
        let mut editor = open("/state/x", "body\n");
        assert_eq!(editor.press(Key::Ctrl('X')), Step::Quit);
    }

    #[test]
    fn a_modified_buffer_asks_before_quitting_and_no_discards_it() {
        let mut editor = open("/state/x", "body\n");
        type_text(&mut editor, "z");
        assert_eq!(editor.press(Key::Ctrl('X')), Step::Continue);
        assert!(editor.prompt().is_some());
        assert_eq!(editor.press(Key::Char('n')), Step::Quit);
    }

    #[test]
    fn answering_yes_to_the_quit_question_offers_the_write_prompt() {
        let mut files = MemoryFiles::default();
        let mut editor = open("/state/x", "body\n");
        type_text(&mut editor, "z");
        editor.key(Key::Ctrl('X'), SIZE, &mut files);
        editor.key(Key::Char('y'), SIZE, &mut files);
        let (label, answer) = editor.prompt().expect("the write prompt");
        assert!(label.contains("File Name"), "{label}");
        assert_eq!(answer, "/state/x");
    }

    #[test]
    fn a_key_that_is_neither_yes_nor_no_leaves_the_quit_question_on_screen() {
        let mut editor = open("/state/x", "body\n");
        type_text(&mut editor, "z");
        editor.press(Key::Ctrl('X'));
        editor.press(Key::Char('q'));
        assert!(
            editor.prompt().is_some(),
            "a wrong answer to this question must not discard the buffer"
        );
    }

    // -- the view rule, which is the point of this crate -----------------------

    /// `ADR-007` §3.6: the write is refused, **and the refusal explains
    /// itself**. The second half is the whole difference from `EROFS`.
    #[test]
    fn writing_a_view_back_over_itself_is_refused_with_the_change_with() {
        let mut files = MemoryFiles::with("/etc/hosts", "127.0.0.1 localhost\n");
        let mut editor = open("/etc/hosts", "127.0.0.1 localhost\n");
        editor.key(Key::Ctrl('O'), SIZE, &mut files);
        editor.key(Key::Enter, SIZE, &mut files);
        let said = message(&editor);
        assert!(said.contains("/etc/hosts"), "{said}");
        assert!(said.contains("loopback"), "{said}");
        assert!(
            files.attempts.is_empty(),
            "the refusal must happen before the syscall, not after EROFS"
        );
    }

    #[test]
    fn typing_into_a_view_changes_nothing_and_says_why() {
        let mut editor = open("/etc/passwd", "root:x:0:0:root:/state/home/root:/system/bin/rivet\n");
        let before = body(&editor);
        type_text(&mut editor, "hello");
        editor.press(Key::Enter);
        editor.press(Key::Backspace);
        editor.press(Key::Delete);
        assert_eq!(body(&editor), before);
        assert!(!editor.text().is_dirty());
        assert!(message(&editor).contains("/etc/passwd"), "{}", message(&editor));
    }

    #[test]
    fn cutting_and_pasting_in_a_view_are_refused_too() {
        let mut editor = open("/etc/shells", "/system/bin/rivet\n");
        editor.press(Key::Ctrl('K'));
        assert_eq!(body(&editor), "/system/bin/rivet\n");
        assert!(message(&editor).contains("/etc/shells"));
    }

    /// The half of the ruling that makes read-only better than a flat refusal:
    /// an operator can still *keep* what they are looking at.
    #[test]
    fn a_copy_of_a_view_can_be_written_somewhere_writable_and_is_then_editable() {
        let mut files = MemoryFiles::default();
        let mut editor = open("/etc/hosts", "127.0.0.1 localhost\n");
        editor.key(Key::Ctrl('O'), SIZE, &mut files);
        for _ in 0.."/etc/hosts".len() {
            editor.key(Key::Backspace, SIZE, &mut files);
        }
        for c in "/state/home/root/hosts.txt".chars() {
            editor.key(Key::Char(c), SIZE, &mut files);
        }
        editor.key(Key::Enter, SIZE, &mut files);
        assert_eq!(
            files.text("/state/home/root/hosts.txt").as_deref(),
            Some("127.0.0.1 localhost\n")
        );
        assert!(matches!(editor.opening(), Opening::Editable));
        editor.key(Key::Char('#'), SIZE, &mut files);
        assert!(
            editor.text().is_dirty(),
            "the copy is not a view and must be editable"
        );
    }

    #[test]
    fn a_view_opens_saying_what_it_is_before_anything_is_typed() {
        let editor = open("/etc/shadow", "root:!:20000:::::\n");
        assert!(
            message(&editor).contains("/etc/shadow"),
            "four of the six generated files can carry no provenance header, so this \
             line is the only place their provenance is readable: {}",
            message(&editor)
        );
    }

    #[test]
    fn a_view_still_moves_and_searches() {
        let mut editor = open("/etc/hosts", "127.0.0.1 localhost\n::1 localhost\n");
        editor.press(Key::Ctrl('W'));
        type_text(&mut editor, "::1");
        editor.press(Key::Enter);
        assert_eq!(editor.cursor().line, 1, "a view is read-only, not inert");
    }

    // -- odds and ends --------------------------------------------------------

    #[test]
    fn the_position_report_says_where_the_cursor_is() {
        let mut editor = open("/state/x", "one\ntwo\n");
        editor.press(Key::Down);
        editor.press(Key::Ctrl('C'));
        assert!(message(&editor).starts_with("line 2/2, col 1/4"), "{}", message(&editor));
    }

    #[test]
    fn help_opens_and_the_next_key_closes_it() {
        let mut editor = open("/state/x", "one\n");
        editor.press(Key::Ctrl('G'));
        assert!(editor.help_is_open());
        editor.press(Key::Char('q'));
        assert!(!editor.help_is_open());
        assert_eq!(body(&editor), "one\n", "the key that closed help must not be typed");
    }

    #[test]
    fn a_key_this_editor_does_not_bind_says_so_rather_than_doing_something() {
        let mut editor = open("/state/x", "one\n");
        assert!(!editor.handles(Key::Ctrl('Z')));
        assert!(message(&editor).contains("^G"));
        assert_eq!(body(&editor), "one\n");
    }

    #[test]
    fn a_file_with_mixed_line_endings_says_it_will_be_converted_on_open() {
        let editor = open("/state/x", "one\r\ntwo\nthree\r\n");
        assert_eq!(message(&editor), "Converted from DOS format");
    }
}
