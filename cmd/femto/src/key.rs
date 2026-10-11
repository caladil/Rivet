//! Bytes off the terminal, turned into keys.
//!
//! A terminal in raw mode delivers a stream of bytes and nothing else. `Up` is
//! three of them — `ESC [ A` — and there is no framing, so a decoder is a state
//! machine fed one byte at a time. This module is that machine, and it is
//! separate from `crate::terminal` for the reason `AGENTS.md` §4 gives: with the
//! decode on this side of the seam, **every escape sequence is a test that needs
//! no terminal at all.**
//!
//! It is also the second of the three things `crossterm` would have supplied,
//! and the manifest's argument for not depending on it rests on this file being
//! small. It is about a hundred lines.
//!
//! # What is decoded, and what deliberately is not
//!
//! The sequences below are the ones `xterm`, `screen`, the Linux console and
//! every terminal emulator that talks to a Raspberry Pi agree on:
//! `ESC [` and `ESC O` cursor keys, `ESC [ n ~` for the editing block, and
//! C0 control characters. **There is no terminfo lookup**, which is a decision
//! rather than an omission: terminfo is a database *file* on the target as well
//! as a library, and `ADR-008` has no place to put one. A key this decoder does
//! not know arrives as [`Key::Unknown`] and the editor says so rather than
//! doing something.
//!
//! # `ESC` on its own
//!
//! Cannot be distinguished from the start of a sequence by looking at bytes
//! alone — that needs a timeout, which needs the read to be non-blocking, which
//! this program does not otherwise need. So a lone `ESC` is resolved by what
//! follows it: an unrecognised second byte emits [`Key::Escape`] **and** the
//! byte that disambiguated it, which is why [`Decoded`] can carry two keys.
//! The practical consequence is that pressing `ESC` alone does nothing until
//! the next keystroke, which is `nano`'s behaviour too.

/// One keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character to insert.
    Char(char),
    /// `^A` … `^Z`, `^\` and `^_`, held as the character the caret is written
    /// with — so `^K` is `Ctrl('K')` and Go To Line is `Ctrl('_')`.
    /// `crate::screen` prints them back by putting the caret in front.
    Ctrl(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Escape,
    /// A sequence this decoder does not know. Reported rather than swallowed,
    /// so an operator whose terminal sends something unexpected is told.
    Unknown,
}

/// What one byte produced.
///
/// Two keys is not a theoretical case: it is `ESC` followed by an ordinary
/// character — recognising a lone `ESC` at all depends on that second byte
/// ruling out the start of an escape sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded {
    /// The byte was consumed and the sequence is not finished.
    Nothing,
    One(Key),
    Two(Key, Key),
}

/// How far into a sequence the decoder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// `ESC` seen.
    Escape,
    /// `ESC [` or `ESC O` seen; `params` is what has arrived since.
    Sequence { ss3: bool, params: [u8; 4], len: usize },
    /// Inside a UTF-8 scalar. `pending` holds the bytes so far.
    Utf8 { buf: [u8; 4], len: usize, want: usize },
}

/// The state machine.
#[derive(Debug, Clone, Copy)]
pub struct Decoder {
    state: State,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder {
            state: State::Ground,
        }
    }
}

impl Decoder {
    /// Feed one byte.
    pub fn feed(&mut self, byte: u8) -> Decoded {
        match self.state {
            State::Ground => self.ground(byte),
            State::Escape => self.after_escape(byte),
            State::Sequence { ss3, params, len } => self.in_sequence(byte, ss3, params, len),
            State::Utf8 { buf, len, want } => self.in_utf8(byte, buf, len, want),
        }
    }

    fn ground(&mut self, byte: u8) -> Decoded {
        match byte {
            0x1b => {
                self.state = State::Escape;
                Decoded::Nothing
            }
            // `\r` is what a terminal in raw mode sends for Return; `\n` is what
            // arrives when input is piped. Both are Enter, which is what makes
            // this program drivable from a file as well as from a keyboard.
            b'\r' | b'\n' => Decoded::One(Key::Enter),
            b'\t' => Decoded::One(Key::Tab),
            // 0x7f is DEL, which is what almost every terminal sends for the
            // Backspace key; 0x08 is `^H`, which some send instead. Both mean
            // *delete backwards* to the person pressing them, and an editor
            // that treated `^H` as a command would be wrong on those terminals.
            0x08 | 0x7f => Decoded::One(Key::Backspace),
            // The remaining C0 controls. `b'a' - 1 + n` is the inverse of the
            // terminal's own encoding: `^A` is 1.
            0x00..=0x1f => Decoded::One(Key::Ctrl(control_letter(byte))),
            0x20..=0x7f => Decoded::One(Key::Char(char::from(byte))),
            _ => {
                let want = utf8_length(byte);
                if want <= 1 {
                    // A continuation byte with no lead, or an invalid lead.
                    return Decoded::One(Key::Unknown);
                }
                let mut buf = [0u8; 4];
                buf[0] = byte;
                self.state = State::Utf8 { buf, len: 1, want };
                Decoded::Nothing
            }
        }
    }

    fn after_escape(&mut self, byte: u8) -> Decoded {
        match byte {
            b'[' | b'O' => {
                self.state = State::Sequence {
                    ss3: byte == b'O',
                    params: [0; 4],
                    len: 0,
                };
                Decoded::Nothing
            }
            // Two escapes in a row: the first was a real `ESC`, the second
            // starts whatever comes next.
            0x1b => Decoded::One(Key::Escape),
            other => {
                // The lone-`ESC` case. Resolve both keys at once rather than
                // dropping either.
                self.state = State::Ground;
                match self.ground(other) {
                    Decoded::One(key) => Decoded::Two(Key::Escape, key),
                    Decoded::Nothing => Decoded::One(Key::Escape),
                    Decoded::Two(a, _) => Decoded::Two(Key::Escape, a),
                }
            }
        }
    }

    fn in_sequence(&mut self, byte: u8, ss3: bool, mut params: [u8; 4], len: usize) -> Decoded {
        // Parameter bytes accumulate; a final byte ends the sequence.
        if byte.is_ascii_digit() || byte == b';' {
            if len < params.len() {
                params[len] = byte;
                self.state = State::Sequence {
                    ss3,
                    params,
                    len: len + 1,
                };
            } else {
                // Longer than anything this decoder knows. Keep consuming until
                // the final byte so the tail does not arrive as text.
                self.state = State::Sequence { ss3, params, len };
            }
            return Decoded::Nothing;
        }

        self.state = State::Ground;
        let digits = params.get(..len).unwrap_or(&[]);
        Decoded::One(final_byte(byte, digits))
    }

    fn in_utf8(&mut self, byte: u8, mut buf: [u8; 4], len: usize, want: usize) -> Decoded {
        if byte & 0b1100_0000 != 0b1000_0000 {
            // Not a continuation byte: the scalar was truncated. Report it and
            // start again with this byte, which is a lead or a control.
            self.state = State::Ground;
            return match self.ground(byte) {
                Decoded::One(key) => Decoded::Two(Key::Unknown, key),
                other => other,
            };
        }
        if let Some(slot) = buf.get_mut(len) {
            *slot = byte;
        }
        let len = len + 1;
        if len < want {
            self.state = State::Utf8 { buf, len, want };
            return Decoded::Nothing;
        }
        self.state = State::Ground;
        match core::str::from_utf8(buf.get(..want).unwrap_or(&[])) {
            Ok(text) => text
                .chars()
                .next()
                .map_or(Decoded::One(Key::Unknown), |c| Decoded::One(Key::Char(c))),
            Err(_) => Decoded::One(Key::Unknown),
        }
    }
}

/// The character a C0 control code stands for. `0x0b` is `^K`, `0x1f` is `^_`.
///
/// Arithmetic rather than a table because the ASCII table already is the table:
/// a control code is its character with bit 6 cleared, so adding `@` back is
/// the exact inverse of what the terminal did. That it also covers `^[`, `^\`,
/// `^]`, `^^` and `^_` for free is the reason this is the right encoding —
/// `^_` is `nano`'s Go To Line, and a lower-case-letter scheme could not name
/// it at all.
fn control_letter(byte: u8) -> char {
    char::from(byte.wrapping_add(b'@'))
}

/// How many bytes the UTF-8 scalar starting with this byte occupies.
fn utf8_length(lead: u8) -> usize {
    match lead {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        // 0x80..=0xc1 and 0xf5.. are never a valid lead byte.
        _ => 0,
    }
}

/// The key a completed `ESC [` / `ESC O` sequence stands for.
fn final_byte(byte: u8, digits: &[u8]) -> Key {
    match byte {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        // `ESC [ H` / `ESC [ F` and the `ESC O` forms. Both are in use: the
        // Linux console sends the first pair, `xterm` in application mode the
        // second.
        b'H' => Key::Home,
        b'F' => Key::End,
        b'~' => match digits {
            b"1" | b"7" => Key::Home,
            b"2" => Key::Unknown, // Insert. This editor has no overwrite mode.
            b"3" => Key::Delete,
            b"4" | b"8" => Key::End,
            b"5" => Key::PageUp,
            b"6" => Key::PageDown,
            _ => Key::Unknown,
        },
        _ => Key::Unknown,
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Feed a whole byte string and collect every key it produced.
    fn keys(bytes: &[u8]) -> Vec<Key> {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        for byte in bytes {
            match decoder.feed(*byte) {
                Decoded::Nothing => {}
                Decoded::One(k) => out.push(k),
                Decoded::Two(a, b) => {
                    out.push(a);
                    out.push(b);
                }
            }
        }
        out
    }

    #[test]
    fn plain_ascii_arrives_as_characters() {
        assert_eq!(
            keys(b"hi"),
            vec![Key::Char('h'), Key::Char('i')]
        );
    }

    #[test]
    fn the_four_arrow_keys_are_three_bytes_each() {
        assert_eq!(
            keys(b"\x1b[A\x1b[B\x1b[C\x1b[D"),
            vec![Key::Up, Key::Down, Key::Right, Key::Left]
        );
    }

    #[test]
    fn application_mode_cursor_keys_decode_the_same_way() {
        // `ESC O A` rather than `ESC [ A`. xterm sends these once an
        // application has turned the keypad on, and an editor that only knew
        // the CSI forms would be arrowless on exactly the terminals people use.
        assert_eq!(keys(b"\x1bOA\x1bOD"), vec![Key::Up, Key::Left]);
    }

    #[test]
    fn the_editing_block_decodes_from_its_numbers() {
        assert_eq!(
            keys(b"\x1b[3~\x1b[5~\x1b[6~\x1b[1~\x1b[4~"),
            vec![Key::Delete, Key::PageUp, Key::PageDown, Key::Home, Key::End]
        );
    }

    #[test]
    fn home_and_end_decode_in_both_of_the_forms_terminals_send() {
        assert_eq!(keys(b"\x1b[H\x1b[F"), vec![Key::Home, Key::End]);
        assert_eq!(keys(b"\x1bOH\x1bOF"), vec![Key::Home, Key::End]);
    }

    #[test]
    fn control_characters_arrive_as_their_letter() {
        assert_eq!(
            keys(b"\x0f\x0b\x15\x17\x18"),
            vec![
                Key::Ctrl('O'),
                Key::Ctrl('K'),
                Key::Ctrl('U'),
                Key::Ctrl('W'),
                Key::Ctrl('X')
            ]
        );
    }

    /// `nano` puts Go To Line on `^_`, which is `0x1f` — above the letters, and
    /// therefore the case a lower-case encoding of `Ctrl` gets wrong.
    #[test]
    fn the_control_codes_above_the_letters_name_themselves_correctly() {
        assert_eq!(
            keys(b"\x1f\x1c"),
            vec![Key::Ctrl('_'), Key::Ctrl('\\')]
        );
    }

    #[test]
    fn both_bytes_a_terminal_sends_for_backspace_mean_backspace() {
        // 0x7f is DEL and 0x08 is `^H`. Treating `^H` as a command would make
        // this editor wrong on the terminals that send it.
        assert_eq!(keys(b"\x7f\x08"), vec![Key::Backspace, Key::Backspace]);
    }

    #[test]
    fn carriage_return_and_line_feed_are_both_enter() {
        assert_eq!(keys(b"\r\n"), vec![Key::Enter, Key::Enter]);
    }

    #[test]
    fn tab_is_a_key_and_not_a_control_letter() {
        assert_eq!(keys(b"\t"), vec![Key::Tab]);
    }

    #[test]
    fn a_lone_escape_is_resolved_by_the_key_that_follows_it() {
        assert_eq!(keys(b"\x1bx"), vec![Key::Escape, Key::Char('x')]);
    }

    #[test]
    fn two_escapes_in_a_row_produce_one_escape_and_wait_for_the_second() {
        assert_eq!(keys(b"\x1b\x1b"), vec![Key::Escape]);
        assert_eq!(keys(b"\x1b\x1bx"), vec![Key::Escape, Key::Escape, Key::Char('x')]);
    }

    #[test]
    fn a_multibyte_character_is_reassembled_from_its_bytes() {
        assert_eq!(keys("é".as_bytes()), vec![Key::Char('é')]);
        assert_eq!(keys("→".as_bytes()), vec![Key::Char('→')]);
        assert_eq!(keys("🦀".as_bytes()), vec![Key::Char('🦀')]);
    }

    #[test]
    fn a_truncated_multibyte_character_is_reported_rather_than_swallowed() {
        // Lead byte of a two-byte scalar, then an ASCII `a`. The scalar is
        // broken and the `a` is real; both have to come out.
        assert_eq!(keys(b"\xc3a"), vec![Key::Unknown, Key::Char('a')]);
    }

    #[test]
    fn a_sequence_this_decoder_does_not_know_is_reported_not_typed() {
        // `ESC [ 2 0 0 ~` is the start of a bracketed paste. This editor does
        // not implement it; what it must not do is insert `[200~` into the
        // buffer, which is what a decoder that gave up mid-sequence would do.
        assert_eq!(keys(b"\x1b[200~"), vec![Key::Unknown]);
    }

    #[test]
    fn a_sequence_longer_than_the_parameter_buffer_still_ends_cleanly() {
        // Longer than `params`, so the tail is dropped — but the final byte
        // must still terminate the sequence rather than leaking as text.
        assert_eq!(keys(b"\x1b[1;2;3;4;5R"), vec![Key::Unknown]);
    }
}
