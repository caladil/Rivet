//! The buffer, the cursor, and every edit that can be made to them.
//!
//! **Nothing here knows there is a terminal.** That is the same split
//! `cmd/rivet-sysd` makes between its decisions and its `Kernel`, and it
//! is here for the same reason: the interesting cases in an editor are *joining
//! the line above with backspace at column 0*, *cutting the last line of the
//! buffer*, and *a search that wraps past the cursor it started at*, and none of
//! them should need a pty to reach. Every one of them is a test in this file.
//!
//! # Two honesty rules, and both are `AGENTS.md` §5 rather than taste
//!
//! 1. **A file that is not text is refused, not mangled.** [`Text::from_bytes`]
//!    returns [`NotText`] for a NUL byte or invalid UTF-8, naming the offset.
//!    An editor that loaded a binary lossily would *succeed* and write back
//!    something that is not what it read — `AGENTS.md` §5's "a wrong guess does
//!    not fail, it succeeds".
//! 2. **The bytes written back are the bytes read, unless the operator changed
//!    them.** The line terminator is detected on open and reused on write, and
//!    a file that did not end in a newline does not acquire one. This is a
//!    deliberate difference from `nano`, which appends a final newline on write
//!    by default: on this machine an editor is reached for to fix one line of
//!    somebody else's file, and silently adding a byte is the same class of act
//!    as silently discarding one.
//!
//! # Columns are characters, not bytes
//!
//! [`Cursor::col`] indexes `char`s. `String` indexes bytes, so every operation
//! here converts at the point of use through [`byte_index`] — which is O(col)
//! and is the right trade for a line-oriented editor whose lines are short.
//! **It is also not a display column**: a tab or a wide CJK character occupies
//! one `col` and more than one cell on screen. `crate::screen` owns that
//! distinction and this module deliberately does not, because the moment the
//! buffer knows how wide a glyph is drawn it needs to know which terminal is
//! drawing it.

/// Where the operator is, in lines and characters.
///
/// Ordered, so that `crate::text`'s search can say *before* and *after* without
/// each caller writing the comparison out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cursor {
    /// Index into [`Text::lines`], always in range for the buffer it came from.
    pub line: usize,
    /// Index in **characters** into that line. May equal the line's length,
    /// which is the position after the last character.
    pub col: usize,
}

impl Cursor {
    /// The start of the buffer.
    pub const START: Cursor = Cursor { line: 0, col: 0 };
}

/// Why a file could not be opened as text.
///
/// Carries the offset rather than a bare verdict, because the first question
/// after "it is not text" is "where", and an operator who knows the answer is 4
/// bytes in has learnt something about the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotText {
    /// A NUL byte. No text file contains one, and an editor that loaded it
    /// would write it back as something else.
    Nul { at: usize },
    /// The bytes stop being valid UTF-8 at this offset.
    NotUtf8 { at: usize },
}

impl core::fmt::Display for NotText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NotText::Nul { at } => write!(
                f,
                "there is a NUL byte at offset {at}, so this is not a text file"
            ),
            NotText::NotUtf8 { at } => write!(
                f,
                "the bytes stop being valid UTF-8 at offset {at}, so this is not a text file"
            ),
        }
    }
}

/// How the lines of this file are separated, as read from the file itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Terminator {
    /// `\n`. Every file Rivet itself writes.
    #[default]
    Unix,
    /// `\r\n`. Reachable here in one real way: the boot partition is FAT and is
    /// `ADR-002` §4.1's file-transfer channel, so a file on it may have been
    /// written by the Windows development host.
    Dos,
}

impl Terminator {
    const fn as_str(self) -> &'static str {
        match self {
            Terminator::Unix => "\n",
            Terminator::Dos => "\r\n",
        }
    }
}

/// The buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// **Never empty.** A buffer with no lines has no position for a cursor,
    /// so an empty file is one empty line and [`Text::cut_line`] blanks the
    /// last line rather than removing it.
    lines: Vec<String>,
    terminator: Terminator,
    /// Whether the file read ended with a terminator. Preserved rather than
    /// normalised — see this module's honesty rule 2.
    final_newline: bool,
    /// Whether anything has been changed since the last read or write. This is
    /// what makes `^X` ask.
    dirty: bool,
    /// Whether the file that was read used a mix of terminators, so that
    /// writing it back will normalise them. The status line says so on open,
    /// exactly as `nano`'s "Converted from DOS format" does, because a write
    /// that changes bytes the operator did not type has to be announced.
    converted: bool,
}

impl Default for Text {
    fn default() -> Self {
        Text {
            lines: vec![String::new()],
            terminator: Terminator::Unix,
            // A file that does not exist yet is written as a POSIX text file.
            // `Text::to_bytes` still renders an untouched empty buffer as zero
            // bytes, so `^O` on a new empty file does not create a file
            // containing a newline.
            final_newline: true,
            dirty: false,
            converted: false,
        }
    }
}

impl Text {
    /// Read a file's bytes, or refuse them.
    pub fn from_bytes(bytes: &[u8]) -> Result<Text, NotText> {
        if let Some(at) = bytes.iter().position(|b| *b == 0) {
            return Err(NotText::Nul { at });
        }
        let text = core::str::from_utf8(bytes).map_err(|e| NotText::NotUtf8 {
            at: e.valid_up_to(),
        })?;
        Ok(Text::from_text(text))
    }

    /// The parse, once the bytes are known to be text.
    fn from_text(text: &str) -> Text {
        // A zero-byte file is indistinguishable from one that has never been
        // written, so there is no terminator to preserve and the default —
        // a POSIX text file — is the right guess rather than a guess at all.
        // `Text::to_bytes` still writes zero bytes for a buffer nothing was
        // typed into, so an empty file that is opened and saved unchanged does
        // not gain a newline.
        if text.is_empty() {
            return Text::default();
        }

        let final_newline = text.ends_with('\n');
        let body = text.strip_suffix('\n').unwrap_or(text);
        let raw: Vec<&str> = body.split('\n').collect();

        // The terminator is the *first* line's, because a file is overwhelmingly
        // likely to be uniform and the first ending is the one an operator would
        // name. `converted` is what makes a non-uniform file say so out loud.
        let terminator = match raw.first() {
            Some(first) if raw.len() > 1 && first.ends_with('\r') => Terminator::Dos,
            // A one-line file that ends `\r\n` has its `\r` in `body` too.
            Some(first) if raw.len() == 1 && final_newline && first.ends_with('\r') => {
                Terminator::Dos
            }
            _ => Terminator::Unix,
        };

        let mut converted = false;
        let lines: Vec<String> = raw
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let is_last = i + 1 == raw.len();
                let has_cr = line.ends_with('\r');
                // The last line only has a terminator if the file ended with
                // one; otherwise a trailing `\r` is content.
                let terminated = !is_last || final_newline;
                match (terminator, has_cr && terminated) {
                    (Terminator::Dos, true) => line.strip_suffix('\r').unwrap_or(line).to_owned(),
                    (Terminator::Dos, false) => {
                        if terminated {
                            converted = true;
                        }
                        (*line).to_owned()
                    }
                    (Terminator::Unix, true) => {
                        converted = true;
                        line.strip_suffix('\r').unwrap_or(line).to_owned()
                    }
                    (Terminator::Unix, false) => (*line).to_owned(),
                }
            })
            .collect();

        Text {
            lines,
            terminator,
            final_newline,
            dirty: false,
            converted,
        }
    }

    /// The bytes to write.
    pub fn to_bytes(&self) -> Vec<u8> {
        // One empty line is how an empty file is held; writing it back as a
        // lone terminator would put a byte in a file the operator left empty.
        if self.lines.len() == 1 && self.lines.first().is_some_and(String::is_empty) {
            return Vec::new();
        }
        let sep = self.terminator.as_str();
        let mut out = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push_str(sep);
            }
            out.push_str(line);
        }
        if self.final_newline {
            out.push_str(sep);
        }
        out.into_bytes()
    }

    /// How many lines the buffer holds. Never zero.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// One line, or `""` for an index past the end.
    pub fn line(&self, index: usize) -> &str {
        self.lines.get(index).map_or("", String::as_str)
    }

    /// The length of one line, in characters.
    pub fn line_len(&self, index: usize) -> usize {
        self.line(index).chars().count()
    }

    /// Whether the buffer differs from what was last read or written.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether writing this buffer back will normalise a mixed set of line
    /// terminators. See [`Text::converted`]'s field documentation.
    pub fn was_converted(&self) -> bool {
        self.converted
    }

    /// The terminator this file will be written with.
    #[cfg(test)]
    pub fn terminator(&self) -> Terminator {
        self.terminator
    }

    /// Mark the buffer as agreeing with the file on disk. Called after a write.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
        // Whatever was on disk before, what is there now is what this buffer
        // holds — so a later `^O` has nothing left to announce.
        self.converted = false;
    }

    /// Clamp a cursor into the buffer, which is what every movement returns
    /// through.
    pub fn clamp(&self, cursor: Cursor) -> Cursor {
        let line = cursor.line.min(self.lines.len().saturating_sub(1));
        Cursor {
            line,
            col: cursor.col.min(self.line_len(line)),
        }
    }

    // -- edits ---------------------------------------------------------------

    /// Insert one character at the cursor and return where the cursor lands.
    pub fn insert(&mut self, at: Cursor, c: char) -> Cursor {
        let at = self.clamp(at);
        let Some(line) = self.lines.get_mut(at.line) else {
            return at;
        };
        let index = byte_index(line, at.col);
        line.insert(index, c);
        self.dirty = true;
        Cursor {
            line: at.line,
            col: at.col + 1,
        }
    }

    /// Split the line at the cursor.
    pub fn insert_newline(&mut self, at: Cursor) -> Cursor {
        let at = self.clamp(at);
        let Some(line) = self.lines.get_mut(at.line) else {
            return at;
        };
        let index = byte_index(line, at.col);
        let tail = line.split_off(index);
        self.lines.insert(at.line + 1, tail);
        self.dirty = true;
        Cursor {
            line: at.line + 1,
            col: 0,
        }
    }

    /// Backspace. At column 0 this joins the line onto the one above, which is
    /// the case worth having a test for.
    pub fn delete_before(&mut self, at: Cursor) -> Cursor {
        let at = self.clamp(at);
        if at.col > 0 {
            let Some(line) = self.lines.get_mut(at.line) else {
                return at;
            };
            let index = byte_index(line, at.col - 1);
            line.remove(index);
            self.dirty = true;
            return Cursor {
                line: at.line,
                col: at.col - 1,
            };
        }
        if at.line == 0 {
            // The start of the buffer. Nothing happens, and nothing is dirtied
            // — a buffer that reports itself modified because somebody pressed
            // backspace at the top would make `^X` ask a question with no
            // answer.
            return at;
        }
        let tail = self.lines.remove(at.line);
        let above = at.line - 1;
        let join_at = self.line_len(above);
        if let Some(line) = self.lines.get_mut(above) {
            line.push_str(&tail);
        }
        self.dirty = true;
        Cursor {
            line: above,
            col: join_at,
        }
    }

    /// Forward delete. At the end of a line this pulls the next line up.
    pub fn delete_at(&mut self, at: Cursor) -> Cursor {
        let at = self.clamp(at);
        if at.col < self.line_len(at.line) {
            if let Some(line) = self.lines.get_mut(at.line) {
                let index = byte_index(line, at.col);
                line.remove(index);
                self.dirty = true;
            }
            return at;
        }
        if at.line + 1 < self.lines.len() {
            let next = self.lines.remove(at.line + 1);
            if let Some(line) = self.lines.get_mut(at.line) {
                line.push_str(&next);
            }
            self.dirty = true;
        }
        at
    }

    /// `^K`. Take a whole line out and hand it back to the caller, which is
    /// what makes `^U` possible.
    ///
    /// **The last line of a buffer is blanked rather than removed**, because
    /// [`Text::lines`] is never empty. `nano` does the same and the cursor
    /// stays where it is either way.
    pub fn cut_line(&mut self, at: Cursor) -> Option<String> {
        let at = self.clamp(at);
        if at.line >= self.lines.len() {
            return None;
        }
        let taken = if self.lines.len() == 1 {
            let line = self.lines.get_mut(0)?;
            if line.is_empty() {
                // Cutting an empty sole line would put an empty string in the
                // cut buffer and dirty a buffer nothing changed.
                return None;
            }
            core::mem::take(line)
        } else {
            self.lines.remove(at.line)
        };
        self.dirty = true;
        Some(taken)
    }

    /// `^U`. Put whole lines back, above the cursor's line.
    pub fn insert_lines(&mut self, at: Cursor, lines: &[String]) -> Cursor {
        if lines.is_empty() {
            return at;
        }
        let at = self.clamp(at);
        // A single empty line is the empty buffer; pasting into it should
        // replace it rather than leave a blank line hanging below.
        let replace_empty =
            self.lines.len() == 1 && self.lines.first().is_some_and(String::is_empty);
        let insert_at = if replace_empty {
            self.lines.clear();
            0
        } else {
            at.line
        };
        for (offset, line) in lines.iter().enumerate() {
            self.lines.insert(insert_at + offset, line.clone());
        }
        self.dirty = true;
        Cursor {
            line: (insert_at + lines.len()).min(self.lines.len().saturating_sub(1)),
            col: 0,
        }
    }

    // -- search --------------------------------------------------------------

    /// `^W`. The next occurrence **after** `from`, wrapping to the top.
    ///
    /// Wrapping rather than stopping is `nano`'s behaviour and it is the one an
    /// operator expects: the alternative is a search that reports "not found"
    /// for a string that is two lines above the cursor.
    ///
    /// Matching is ASCII-case-insensitive, which is `nano`'s default. It is
    /// *ASCII*-case-insensitive rather than Unicode-case-insensitive because
    /// full case folding changes the number of characters in a string — `ß`
    /// folds to `ss` — and a match whose length differs from the needle's
    /// cannot be reported as a cursor position without lying about where it
    /// ends.
    pub fn find(&self, from: Cursor, needle: &str) -> Option<Cursor> {
        if needle.is_empty() {
            return None;
        }
        let from = self.clamp(from);
        let count = self.lines.len();

        // After the cursor on its own line, then every line below.
        if let Some(col) = find_in_line(self.line(from.line), needle, from.col + 1) {
            return Some(Cursor {
                line: from.line,
                col,
            });
        }
        for line in from.line + 1..count {
            if let Some(col) = find_in_line(self.line(line), needle, 0) {
                return Some(Cursor { line, col });
            }
        }
        // Wrap: every line above, then the cursor's own line up to and
        // including the cursor.
        for line in 0..from.line {
            if let Some(col) = find_in_line(self.line(line), needle, 0) {
                return Some(Cursor { line, col });
            }
        }
        find_in_line(self.line(from.line), needle, 0)
            .filter(|col| *col <= from.col)
            .map(|col| Cursor {
                line: from.line,
                col,
            })
    }
}

/// The byte offset of character `col` in `line`, or the line's length.
fn byte_index(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map_or_else(|| line.len(), |(index, _)| index)
}

/// The character index of the first match at or after `start`.
fn find_in_line(line: &str, needle: &str, start: usize) -> Option<usize> {
    let haystack: Vec<char> = line.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    (start..=last).find(|&at| {
        haystack
            .iter()
            .skip(at)
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn text(body: &str) -> Text {
        Text::from_bytes(body.as_bytes()).expect("the fixture is text")
    }

    fn rendered(text: &Text) -> String {
        String::from_utf8(text.to_bytes()).expect("the buffer is UTF-8 by construction")
    }

    // -- reading and writing bytes -------------------------------------------

    #[test]
    fn a_file_of_three_lines_reads_as_three_lines() {
        let text = text("one\ntwo\nthree\n");
        assert_eq!(text.line_count(), 3);
        assert_eq!(text.line(1), "two");
    }

    #[test]
    fn a_file_that_did_not_end_in_a_newline_does_not_acquire_one() {
        let text = text("no trailing newline");
        assert_eq!(rendered(&text), "no trailing newline");
    }

    #[test]
    fn a_file_that_ended_in_a_newline_keeps_it() {
        assert_eq!(rendered(&text("one\n")), "one\n");
    }

    #[test]
    fn an_empty_file_reads_as_one_empty_line_and_writes_as_no_bytes() {
        let text = text("");
        assert_eq!(text.line_count(), 1);
        assert!(
            text.to_bytes().is_empty(),
            "opening an empty file and saving it unchanged must not add a byte"
        );
    }

    #[test]
    fn typing_into_an_empty_file_produces_a_posix_text_file() {
        let mut text = text("");
        for (index, c) in "hi".chars().enumerate() {
            text.insert(Cursor { line: 0, col: index }, c);
        }
        assert_eq!(
            rendered(&text),
            "hi\n",
            "a zero-byte file has no terminator to preserve, so the default applies"
        );
    }

    #[test]
    fn a_new_file_that_was_never_typed_into_writes_as_no_bytes() {
        assert!(Text::default().to_bytes().is_empty());
    }

    #[test]
    fn a_dos_file_round_trips_as_a_dos_file() {
        let text = text("one\r\ntwo\r\n");
        assert_eq!(text.terminator(), Terminator::Dos);
        assert_eq!(text.line(0), "one");
        assert_eq!(rendered(&text), "one\r\ntwo\r\n");
        assert!(!text.was_converted());
    }

    #[test]
    fn a_file_with_mixed_terminators_says_it_will_be_converted() {
        let text = text("one\r\ntwo\nthree\r\n");
        assert!(
            text.was_converted(),
            "a write that normalises the operator's line endings has to announce it"
        );
    }

    #[test]
    fn a_carriage_return_that_is_not_a_terminator_is_content() {
        // No trailing newline, so the `\r` is the last byte of the line rather
        // than half of a line ending.
        let text = text("one\rtwo");
        assert_eq!(text.line(0), "one\rtwo");
        assert_eq!(rendered(&text), "one\rtwo");
    }

    #[test]
    fn a_nul_byte_is_refused_with_its_offset() {
        assert_eq!(
            Text::from_bytes(b"abc\0def"),
            Err(NotText::Nul { at: 3 }),
            "an editor that loaded a binary would write back something else"
        );
    }

    #[test]
    fn bytes_that_are_not_utf8_are_refused_with_the_offset_they_stop_at() {
        assert_eq!(
            Text::from_bytes(b"ok\xff"),
            Err(NotText::NotUtf8 { at: 2 })
        );
    }

    // -- inserting -----------------------------------------------------------

    #[test]
    fn typing_a_character_puts_it_at_the_cursor_and_moves_past_it() {
        let mut text = text("ac\n");
        let at = text.insert(Cursor { line: 0, col: 1 }, 'b');
        assert_eq!(text.line(0), "abc");
        assert_eq!(at, Cursor { line: 0, col: 2 });
        assert!(text.is_dirty());
    }

    #[test]
    fn a_multibyte_character_counts_as_one_column() {
        let mut text = text("aé c\n");
        let at = text.insert(Cursor { line: 0, col: 3 }, 'X');
        assert_eq!(text.line(0), "aé Xc");
        assert_eq!(at, Cursor { line: 0, col: 4 });
    }

    #[test]
    fn enter_in_the_middle_of_a_line_splits_it() {
        let mut text = text("abcd\n");
        let at = text.insert_newline(Cursor { line: 0, col: 2 });
        assert_eq!(text.line(0), "ab");
        assert_eq!(text.line(1), "cd");
        assert_eq!(at, Cursor { line: 1, col: 0 });
    }

    // -- deleting ------------------------------------------------------------

    #[test]
    fn backspace_at_column_zero_joins_the_line_onto_the_one_above() {
        let mut text = text("one\ntwo\n");
        let at = text.delete_before(Cursor { line: 1, col: 0 });
        assert_eq!(text.line_count(), 1);
        assert_eq!(text.line(0), "onetwo");
        assert_eq!(at, Cursor { line: 0, col: 3 });
    }

    #[test]
    fn backspace_at_the_start_of_the_buffer_changes_nothing_and_dirties_nothing() {
        let mut text = text("one\n");
        let at = text.delete_before(Cursor::START);
        assert_eq!(at, Cursor::START);
        assert_eq!(text.line(0), "one");
        assert!(!text.is_dirty());
    }

    #[test]
    fn forward_delete_at_the_end_of_a_line_pulls_the_next_line_up() {
        let mut text = text("one\ntwo\n");
        let at = text.delete_at(Cursor { line: 0, col: 3 });
        assert_eq!(text.line(0), "onetwo");
        assert_eq!(at, Cursor { line: 0, col: 3 });
    }

    #[test]
    fn forward_delete_on_the_last_line_of_the_buffer_does_nothing() {
        let mut text = text("one");
        let at = text.delete_at(Cursor { line: 0, col: 3 });
        assert_eq!(text.line(0), "one");
        assert_eq!(at, Cursor { line: 0, col: 3 });
        assert!(!text.is_dirty());
    }

    // -- cut and paste -------------------------------------------------------

    #[test]
    fn cutting_a_line_takes_it_out_and_hands_it_back() {
        let mut text = text("one\ntwo\nthree\n");
        assert_eq!(text.cut_line(Cursor { line: 1, col: 0 }).as_deref(), Some("two"));
        assert_eq!(text.line_count(), 2);
        assert_eq!(text.line(1), "three");
    }

    #[test]
    fn cutting_the_only_line_blanks_it_rather_than_emptying_the_buffer() {
        let mut text = text("only");
        assert_eq!(text.cut_line(Cursor::START).as_deref(), Some("only"));
        assert_eq!(text.line_count(), 1, "a buffer with no lines has no cursor");
        assert_eq!(text.line(0), "");
    }

    #[test]
    fn cutting_an_empty_sole_line_is_refused_rather_than_filling_the_cut_buffer() {
        let mut text = text("");
        assert_eq!(text.cut_line(Cursor::START), None);
        assert!(!text.is_dirty());
    }

    #[test]
    fn cut_then_uncut_somewhere_else_moves_the_line() {
        let mut text = text("one\ntwo\nthree\n");
        let taken = text.cut_line(Cursor { line: 0, col: 0 }).expect("a line");
        let at = text.insert_lines(Cursor { line: 1, col: 0 }, &[taken]);
        assert_eq!(rendered(&text), "two\none\nthree\n");
        assert_eq!(at, Cursor { line: 2, col: 0 });
    }

    #[test]
    fn uncutting_into_an_empty_buffer_replaces_the_empty_line() {
        let mut text = text("");
        text.insert_lines(Cursor::START, &["pasted".to_owned()]);
        assert_eq!(text.line_count(), 1, "the empty line was left hanging below");
        assert_eq!(rendered(&text), "pasted\n");
    }

    // -- search --------------------------------------------------------------

    #[test]
    fn search_finds_the_next_occurrence_after_the_cursor() {
        let text = text("alpha\nbeta\nalpha\n");
        assert_eq!(
            text.find(Cursor::START, "alpha"),
            Some(Cursor { line: 2, col: 0 })
        );
    }

    #[test]
    fn search_wraps_past_the_end_of_the_buffer() {
        let text = text("alpha\nbeta\n");
        assert_eq!(
            text.find(Cursor { line: 1, col: 0 }, "alpha"),
            Some(Cursor { line: 0, col: 0 }),
            "a search that stopped at the end would report `not found` for a match two lines up"
        );
    }

    #[test]
    fn search_ignores_ascii_case_the_way_nano_does() {
        let text = text("The Quick Fox\n");
        assert_eq!(
            text.find(Cursor::START, "quick"),
            Some(Cursor { line: 0, col: 4 })
        );
    }

    #[test]
    fn search_for_a_string_that_is_not_there_finds_nothing() {
        assert_eq!(text("alpha\n").find(Cursor::START, "omega"), None);
    }

    #[test]
    fn search_finds_the_match_the_cursor_is_sitting_on_only_by_wrapping() {
        let text = text("alpha\n");
        // Starting at column 0 the search begins at column 1, so the only way
        // back to column 0 is the wrap — which is the branch this exercises.
        assert_eq!(text.find(Cursor::START, "alpha"), Some(Cursor::START));
    }

    #[test]
    fn a_search_column_is_a_character_index_not_a_byte_offset() {
        let text = text("héllo wörld\n");
        assert_eq!(
            text.find(Cursor::START, "wörld"),
            Some(Cursor { line: 0, col: 6 })
        );
    }

    // -- the dirty flag ------------------------------------------------------

    #[test]
    fn a_freshly_read_buffer_is_not_dirty_and_a_saved_one_stops_being() {
        let mut text = text("one\n");
        assert!(!text.is_dirty());
        text.insert(Cursor::START, 'x');
        assert!(text.is_dirty());
        text.mark_saved();
        assert!(!text.is_dirty());
    }

    #[test]
    fn a_cursor_past_the_end_of_the_buffer_is_clamped_into_it() {
        let text = text("one\ntwo\n");
        assert_eq!(
            text.clamp(Cursor {
                line: 99,
                col: 99
            }),
            Cursor { line: 1, col: 3 }
        );
    }
}
