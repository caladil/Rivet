//! **A bitmap font, built in** — because the image has no font stack.
//!
//! Five pixels wide, seven tall, and only the glyphs the splash actually draws:
//! `Rivet OS`, `Version`, the digits, a dot and a dash. A full ASCII font would
//! be four times the data for characters nothing renders, and `ADR-001`'s
//! minimalism is about surface rather than bytes.
//!
//! **An unknown character draws nothing and does not panic.** A version string
//! is composed from a git description, and a build somebody tags with a letter
//! this table lacks must produce a slightly gappy line rather than a boot with
//! no splash and a dead child in the journal.

/// A glyph's width in source pixels, plus one column of spacing.
pub const GLYPH_WIDTH: usize = 6;

/// A glyph's height in source pixels.
pub const GLYPH_HEIGHT: usize = 7;

/// The baseline-to-baseline distance the splash lays text out on.
pub const LINE_HEIGHT: usize = GLYPH_HEIGHT + 3;

/// Where a glyph is drawn into.
pub struct Target {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bytes_per_pixel: usize,
    /// How one pixel is written, read from the driver -- `crate::pixel::Pixel`.
    /// It was three byte offsets until 2026-08-31, which could not express the
    /// 16-bit framebuffer VC4's KMS driver provides. `PLAN.md` §104.13.
    pub pixel: crate::pixel::Pixel,
}

/// The glyphs, as rows of bits from the most significant bit.
const GLYPHS: &[(char, [u8; GLYPH_HEIGHT])] = &[
    (' ', [0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b00000000]),
    ('-', [0b00000000, 0b00000000, 0b00000000, 0b11111000, 0b00000000, 0b00000000, 0b00000000]),
    ('.', [0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b00000000, 0b01100000, 0b01100000]),
    ('0', [0b01110000, 0b10001000, 0b10011000, 0b10101000, 0b11001000, 0b10001000, 0b01110000]),
    ('1', [0b00100000, 0b01100000, 0b00100000, 0b00100000, 0b00100000, 0b00100000, 0b01110000]),
    ('2', [0b01110000, 0b10001000, 0b00001000, 0b00010000, 0b00100000, 0b01000000, 0b11111000]),
    ('3', [0b11111000, 0b00010000, 0b00100000, 0b00010000, 0b00001000, 0b10001000, 0b01110000]),
    ('4', [0b00010000, 0b00110000, 0b01010000, 0b10010000, 0b11111000, 0b00010000, 0b00010000]),
    ('5', [0b11111000, 0b10000000, 0b11110000, 0b00001000, 0b00001000, 0b10001000, 0b01110000]),
    ('6', [0b00110000, 0b01000000, 0b10000000, 0b11110000, 0b10001000, 0b10001000, 0b01110000]),
    ('7', [0b11111000, 0b00001000, 0b00010000, 0b00100000, 0b01000000, 0b01000000, 0b01000000]),
    ('8', [0b01110000, 0b10001000, 0b10001000, 0b01110000, 0b10001000, 0b10001000, 0b01110000]),
    ('9', [0b01110000, 0b10001000, 0b10001000, 0b01111000, 0b00001000, 0b00010000, 0b01100000]),
    // Added 2026-08-30. The endpoint line is `ADDRESS:PORT` and this was
    // missing, so the board drew `192.168.50.66 7777` — a separator
    // rendered as a SPACE, which reads as two values rather than one.
    (':', [0b00000000, 0b01100000, 0b01100000, 0b00000000, 0b01100000, 0b01100000, 0b00000000]),
    ('B', [0b11110000, 0b10001000, 0b10001000, 0b11110000, 0b10001000, 0b10001000, 0b11110000]),
    ('O', [0b01110000, 0b10001000, 0b10001000, 0b10001000, 0b10001000, 0b10001000, 0b01110000]),
    ('R', [0b11110000, 0b10001000, 0b10001000, 0b11110000, 0b10100000, 0b10010000, 0b10001000]),
    ('S', [0b01111000, 0b10000000, 0b10000000, 0b01110000, 0b00001000, 0b00001000, 0b11110000]),
    ('V', [0b10001000, 0b10001000, 0b10001000, 0b10001000, 0b10001000, 0b01010000, 0b00100000]),
    ('a', [0b00000000, 0b00000000, 0b01110000, 0b00001000, 0b01111000, 0b10001000, 0b01111000]),
    ('b', [0b10000000, 0b10000000, 0b10110000, 0b11001000, 0b10001000, 0b10001000, 0b11110000]),
    ('c', [0b00000000, 0b00000000, 0b01110000, 0b10000000, 0b10000000, 0b10001000, 0b01110000]),
    ('d', [0b00001000, 0b00001000, 0b01101000, 0b10011000, 0b10001000, 0b10001000, 0b01111000]),
    ('e', [0b00000000, 0b00000000, 0b01110000, 0b10001000, 0b11111000, 0b10000000, 0b01110000]),
    ('f', [0b00110000, 0b01001000, 0b01000000, 0b11100000, 0b01000000, 0b01000000, 0b01000000]),
    ('g', [0b00000000, 0b01111000, 0b10001000, 0b10001000, 0b01111000, 0b00001000, 0b01110000]),
    ('h', [0b10000000, 0b10000000, 0b10110000, 0b11001000, 0b10001000, 0b10001000, 0b10001000]),
    ('i', [0b00100000, 0b00000000, 0b01100000, 0b00100000, 0b00100000, 0b00100000, 0b01110000]),
    ('j', [0b00100000, 0b00000000, 0b00100000, 0b00100000, 0b00100000, 0b10100000, 0b01000000]),
    ('k', [0b10000000, 0b10000000, 0b10010000, 0b10100000, 0b11000000, 0b10100000, 0b10010000]),
    ('l', [0b01100000, 0b00100000, 0b00100000, 0b00100000, 0b00100000, 0b00100000, 0b01110000]),
    ('m', [0b00000000, 0b00000000, 0b11010000, 0b10101000, 0b10101000, 0b10101000, 0b10101000]),
    ('n', [0b00000000, 0b00000000, 0b10110000, 0b11001000, 0b10001000, 0b10001000, 0b10001000]),
    ('o', [0b00000000, 0b00000000, 0b01110000, 0b10001000, 0b10001000, 0b10001000, 0b01110000]),
    ('p', [0b00000000, 0b00000000, 0b11110000, 0b10001000, 0b11110000, 0b10000000, 0b10000000]),
    ('q', [0b00000000, 0b00000000, 0b01111000, 0b10001000, 0b01111000, 0b00001000, 0b00001000]),
    ('r', [0b00000000, 0b00000000, 0b10110000, 0b11001000, 0b10000000, 0b10000000, 0b10000000]),
    ('s', [0b00000000, 0b00000000, 0b01111000, 0b10000000, 0b01110000, 0b00001000, 0b11110000]),
    ('t', [0b01000000, 0b01000000, 0b11100000, 0b01000000, 0b01000000, 0b01001000, 0b00110000]),
    ('u', [0b00000000, 0b00000000, 0b10001000, 0b10001000, 0b10001000, 0b10011000, 0b01101000]),
    ('v', [0b00000000, 0b00000000, 0b10001000, 0b10001000, 0b10001000, 0b01010000, 0b00100000]),
    ('w', [0b00000000, 0b00000000, 0b10001000, 0b10001000, 0b10101000, 0b10101000, 0b01010000]),
    ('x', [0b00000000, 0b00000000, 0b10001000, 0b01010000, 0b00100000, 0b01010000, 0b10001000]),
    ('y', [0b00000000, 0b00000000, 0b10001000, 0b10001000, 0b01111000, 0b00001000, 0b01110000]),
    ('z', [0b00000000, 0b00000000, 0b11111000, 0b00010000, 0b00100000, 0b01000000, 0b11111000]),
];

/// How wide `text` is once drawn at `scale`.
#[must_use]
pub fn text_width(text: &str, scale: usize) -> usize {
    text.chars().count() * GLYPH_WIDTH * scale
}

/// One glyph's bitmap, or `None` when the font has no such character.
fn glyph(ch: char) -> Option<&'static [u8; GLYPH_HEIGHT]> {
    GLYPHS.iter().find(|(c, _)| *c == ch).map(|(_, rows)| rows)
}

/// Draw `text` with its top-left at `x, y`.
pub fn draw(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    text: &str,
    scale: usize,
    rgb: [u8; 3],
) {
    for (i, ch) in text.chars().enumerate() {
        let Some(rows) = glyph(ch) else {
            continue;
        };
        let gx = x + i * GLYPH_WIDTH * scale;
        for (ry, bits) in rows.iter().enumerate() {
            for rx in 0..5 {
                if bits & (0b1000_0000 >> rx) == 0 {
                    continue;
                }
                // One source pixel becomes a `scale` by `scale` block.
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = gx + rx * scale + dx;
                        let py = y + ry * scale + dy;
                        if px >= t.width || py >= t.height {
                            continue;
                        }
                        // `put` drops a write that would run past the end,
                        // so the bound is its concern -- and it has to be, now
                        // that a pixel is not always four bytes wide.
                        t.pixel.put(canvas, py * t.stride + px * t.bytes_per_pixel, rgb);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **A character with no glyph draws NOTHING and reports nothing**, which is
    /// how the board came to show `192.168.50.66 7777` — the colon rendered as a
    /// space, so one value read as two. Every character the splash can put on a
    /// screen must be in the table.
    #[test]
    fn every_character_the_splash_can_draw_has_a_glyph() {
        // The three lines, spelled as the set of characters they can contain
        // rather than as the strings themselves: the version carries a package
        // version and a git describe, and neither is a constant.
        let name = "Rivet OS";
        let version_alphabet = "Version 0123456789.- abcdef";
        // **A HOSTNAME, not just an address.** This said "0123456789.:" until
        // 2026-08-30, which was right when line three could only be an IP —
        // and the day it could be `devpi.local` the board drew `dev i.local`,
        // because `p` had never been needed. RFC 1123 says a hostname is
        // letters, digits and hyphens, so that is what the test asserts rather
        // than the characters this week's hostname happens to use.
        let endpoint_alphabet = "abcdefghijklmnopqrstuvwxyz0123456789-.:";

        for text in [name, version_alphabet, endpoint_alphabet] {
            for ch in text.chars() {
                assert!(
                    glyph(ch).is_some(),
                    "no glyph for {ch:?} — it would be drawn as a blank, silently"
                );
            }
        }
    }

    /// **The whole lowercase alphabet, because a hostname is the operator's
    /// word and not ours.** Ten letters were missing until 2026-08-30 — `j k m
    /// p q u w x y z` — because nothing the splash had ever drawn used them.
    #[test]
    fn every_lowercase_letter_has_a_glyph() {
        for ch in 'a'..='z' {
            assert!(glyph(ch).is_some(), "no glyph for {ch:?} — a hostname may contain it");
        }
    }

    /// The one that was actually missing, named so a regression is obvious.
    #[test]
    fn the_colon_that_separates_an_address_from_its_port_exists() {
        let colon = glyph(':').expect("the endpoint line is ADDRESS:PORT");
        assert!(
            colon.iter().any(|row| *row != 0),
            "the colon is present and blank, which is the same defect with a table entry"
        );
    }

    /// A glyph that is entirely empty is a hole in the alphabet wearing a
    /// table entry. Space is the one legitimate exception.
    #[test]
    fn no_glyph_except_space_is_blank() {
        for (ch, rows) in GLYPHS {
            if *ch == ' ' {
                continue;
            }
            assert!(
                rows.iter().any(|row| *row != 0),
                "{ch:?} has a table entry and draws nothing"
            );
        }
    }

    /// The table is searched linearly by character, so order is not required —
    /// but it is kept sorted so a reader can find one, and a duplicate would
    /// mean the second is unreachable.
    #[test]
    fn the_table_has_no_duplicates() {
        let mut seen: Vec<char> = GLYPHS.iter().map(|(c, _)| *c).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(before, seen.len(), "a character appears twice");
    }
}
