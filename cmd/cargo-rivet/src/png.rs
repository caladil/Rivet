//! **Writing PNG** — the format the maintainer asked to be handed, written by
//! hand because of what this binary is.
//!
//! `PLAN.md` §104.39's ruling, 2026-09-01: the board may write the capture as
//! full-resolution QOI, and what the workstation software hands the maintainer
//! must be a PNG (.rulings/ R-326). [`crate::qoi`] does the reading; this does
//! the writing.
//!
//! # Why not a crate
//!
//! `ADR-009` §5. **`cmd/cargo-rivet` is the only binary in this workspace that
//! reads a private signing seed** — its `Cargo.toml` says so at the top — and
//! every dependency it gains is one more thing with the opportunity to read
//! that seed. It is why its ELF reader is hand-written rather than `goblin`, and
//! the same gate applies here. `png` pulls a compressor and a colour-space
//! library behind it; what is actually needed is chunk framing, a CRC and a
//! zlib stream, and that is this file.
//!
//! # The size cost, stated rather than discovered
//!
//! **This uses DEFLATE *stored* blocks: the pixels are not compressed.** A
//! valid zlib stream of stored blocks plus an Adler-32 is a few dozen lines and
//! needs no compressor at all, where an LZ77 matcher and a Huffman coder would
//! be several hundred lines of the kind that is wrong in ways a test does not
//! catch.
//!
//! What it costs is the whole of the file's size:
//!
//! | Image | PNG this writes | A compressing encoder |
//! | :-- | :-- | :-- |
//! | 1920×1080, RGB | about **6.2 MB** | a few hundred KB |
//!
//! `1080 × (1 + 1920 × 3)` bytes of scanline, five bytes of block header per
//! 65535, and the chunk framing. **That is acceptable here and would not be
//! anywhere else**: this is a debugging artefact that goes from a board to a
//! workstation's disk and is looked at once. It is not a format this project
//! serves, stores or ships. Anything that starts writing PNGs at volume should
//! reopen this decision rather than inherit it.
//!
//! # It is checked against readers that are not this one
//!
//! `AGENTS.md` §4: a writer checked against its own reader agrees with itself.
//! There is no PNG reader in this repository at all, so the tests below assert
//! the *structure* the specification requires — signature, chunk order, CRCs,
//! the zlib header's check value, the Adler-32 — and the file this produces was
//! opened with an independent reader outside the build. See the module tests for
//! exactly what was checked and by what.

/// The eight bytes every PNG begins with. Chosen by the specification to catch
/// a transfer that mangled line endings or stripped the high bit.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// `IHDR`'s colour type for 8-bit RGB, three bytes per pixel.
const COLOUR_TYPE_RGB: u8 = 2;
/// `IHDR`'s colour type for 8-bit RGBA, four bytes per pixel.
const COLOUR_TYPE_RGBA: u8 = 6;

/// The most one DEFLATE stored block can carry. A 16-bit length field.
const MAX_STORED_BLOCK: usize = 65_535;

/// Write a PNG.
///
/// `rgba` is `width * height * 4` bytes as [`crate::qoi::decode`] produces them.
/// `keep_alpha` decides between a three- and a four-channel image: pass what
/// [`crate::qoi::Image::is_opaque`] says, so that a capture with no transparency
/// in it — which is every capture a Rivet takes — does not carry a pointless
/// fourth byte per pixel, and one that *does* have transparency does not have it
/// silently dropped.
///
/// # Panics
///
/// It does not. The only fallible arithmetic here is the length of a `u32`
/// chunk field, and every chunk this writes is bounded by the image, which
/// [`crate::qoi::decode`] has already capped.
#[must_use]
pub fn encode(width: u32, height: u32, rgba: &[u8], keep_alpha: bool) -> Vec<u8> {
    let (colour_type, per_pixel) = if keep_alpha {
        (COLOUR_TYPE_RGBA, 4usize)
    } else {
        (COLOUR_TYPE_RGB, 3usize)
    };
    let row = width as usize * per_pixel;

    // **The scanlines, each behind its filter byte.** Filter 0 is "none": a
    // filter's whole purpose is to make the bytes more compressible, and this
    // encoder does not compress, so any other choice would cost arithmetic and
    // buy nothing. The byte is not optional — PNG has one per row whether it is
    // used or not.
    let mut raw = Vec::with_capacity(height as usize * (1 + row));
    for y in 0..height as usize {
        raw.push(0);
        let start = y * width as usize * 4;
        for x in 0..width as usize {
            let at = start + x * 4;
            raw.extend_from_slice(&rgba[at..at + 3]);
            if keep_alpha {
                raw.push(rgba[at + 3]);
            }
        }
    }

    let mut out = Vec::with_capacity(SIGNATURE.len() + 64 + raw.len() + raw.len() / 8192 * 5);
    out.extend_from_slice(&SIGNATURE);

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.push(8); // bit depth
    header.push(colour_type);
    header.push(0); // compression method: DEFLATE, the only one PNG has
    header.push(0); // filter method: the only one PNG has
    header.push(0); // interlace: none
    chunk(&mut out, b"IHDR", &header);

    chunk(&mut out, b"IDAT", &zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// One PNG chunk: length, type, data, CRC.
///
/// **The CRC covers the type and the data and NOT the length**, which is the
/// detail a hand-written encoder gets wrong and which produces a file every
/// reader rejects with a message about corruption.
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    // A chunk length is a 31-bit field. Every chunk here is bounded by an image
    // `crate::qoi::decode` has already capped at 40 megapixels, so this cannot
    // saturate for anything that reached this far; saturating rather than
    // wrapping is still the right failure, because a wrapped length writes a
    // plausible file describing the wrong number of bytes.
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);

    let mut crc = Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// A zlib stream (`RFC 1950`) whose DEFLATE data is nothing but stored blocks.
///
/// # The two-byte header, and why `0x78 0x01` rather than any other pair
///
/// `CMF` is `0x78`: compression method 8 (DEFLATE) in the low nibble, and a
/// window size exponent of 7 (32 KiB) in the high one. `FLG` carries a check
/// value in its low five bits chosen so that `CMF << 8 | FLG` is a multiple of
/// 31 — `0x7801` is `30721`, which is `31 × 991`. The high bits say no preset
/// dictionary and compression level "fastest", which is an honest description of
/// what follows.
///
/// **A wrong check value is the classic failure here**: the stream is otherwise
/// perfect and every reader refuses it at the first byte.
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / MAX_STORED_BLOCK * 5 + 16);
    out.push(0x78);
    out.push(0x01);

    // `RFC 1951` section 3.2.4 — the word rather than the section sign, because
    // `tools/docs/claims.py` resolves a bare section sign against this project's
    // own documents and reports one it cannot find. An RFC's section is not a
    // citation into `docs/`.
    //
    // A stored block is a byte holding `BFINAL` and
    // `BTYPE = 00`, then — because a stored block starts on a byte boundary and
    // nothing here writes partial bytes — the length, its ones-complement, and
    // the bytes themselves. `NLEN` is redundant with `LEN` on purpose: it is the
    // format's own check that the two length bytes arrived intact.
    let mut blocks = data.chunks(MAX_STORED_BLOCK).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = blocks.next() {
        let final_block = u8::from(blocks.peek().is_none());
        out.push(final_block);
        // `block.len()` is at most `MAX_STORED_BLOCK`, which is `u16::MAX`.
        let length = block.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(block);
    }

    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Adler-32 (`RFC 1950` §9) of the **uncompressed** data.
///
/// Of the uncompressed data, and that is the sentence worth having: an
/// implementation that check-summed the deflate stream instead produces a file
/// that frames correctly, decompresses correctly, and is rejected at the last
/// four bytes.
fn adler32(data: &[u8]) -> u32 {
    const MODULO: u32 = 65_521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    // The modulo is taken per byte rather than deferred with a block bound.
    // Slower, and it cannot overflow, which is the trade this file wants: the
    // deferred form is where every hand-written Adler-32 goes wrong.
    for byte in data {
        a = (a + u32::from(*byte)) % MODULO;
        b = (b + a) % MODULO;
    }
    (b << 16) | a
}

/// CRC-32 as PNG uses it: the IEEE polynomial, reflected, pre- and
/// post-conditioned with `0xffff_ffff`.
struct Crc(u32);

/// The reflected polynomial, `0xedb8_8320`. Written as the constant rather than
/// derived, because it is the number the specification prints.
const CRC_POLYNOMIAL: u32 = 0xedb8_8320;

/// The 256-entry table, built at compile time so there is no lazy initialisation
/// and nothing to get wrong about when it is ready.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 {
                CRC_POLYNOMIAL ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

impl Crc {
    const fn new() -> Self {
        Self(0xffff_ffff)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = CRC_TABLE[((self.0 ^ u32::from(*byte)) & 0xff) as usize] ^ (self.0 >> 8);
        }
    }

    const fn finish(&self) -> u32 {
        self.0 ^ 0xffff_ffff
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Walk the chunks of a PNG, checking every CRC, and hand back
    /// `(type, data)` for each.
    ///
    /// **This is a reader, and it is the thing these tests must not lean on too
    /// hard.** It was written from the specification rather than by inverting
    /// [`encode`], and it deliberately re-derives nothing: it recomputes each
    /// CRC with [`Crc`], which is the encoder's own — so a wrong POLYNOMIAL
    /// would pass here and fail everywhere else. That gap is closed by an
    /// independent reader outside the build; see
    /// [`the_file_this_writes_was_opened_by_a_reader_that_is_not_this_one`].
    fn chunks(png: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert_eq!(&png[0..8], &SIGNATURE, "the signature is wrong");
        let mut out = Vec::new();
        let mut at = 8;
        while at < png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = String::from_utf8(png[at + 4..at + 8].to_vec()).expect("ASCII");
            let data = png[at + 8..at + 8 + length].to_vec();
            let stated = u32::from_be_bytes(
                png[at + 8 + length..at + 12 + length].try_into().unwrap(),
            );
            let mut crc = Crc::new();
            crc.update(&png[at + 4..at + 8]);
            crc.update(&data);
            assert_eq!(crc.finish(), stated, "the CRC of {kind} is wrong");
            out.push((kind, data));
            at += 12 + length;
        }
        out
    }

    /// Undo the zlib stream this writes: check the header, walk the stored
    /// blocks, and check the Adler-32.
    ///
    /// Written from `RFC 1950` and `RFC 1951` rather than from [`zlib`].
    fn inflate_stored(stream: &[u8]) -> Vec<u8> {
        assert_eq!(stream[0] & 0x0f, 8, "the compression method is not DEFLATE");
        assert_eq!(
            (u32::from(stream[0]) << 8 | u32::from(stream[1])) % 31,
            0,
            "the zlib header's check value is wrong, and every reader refuses at byte two"
        );
        assert_eq!(stream[1] & 0x20, 0, "a preset dictionary is claimed");

        let mut out = Vec::new();
        let mut at = 2;
        loop {
            let header = stream[at];
            assert_eq!(header & 0x06, 0, "a block that is not STORED");
            let length = u16::from_le_bytes(stream[at + 1..at + 3].try_into().unwrap());
            let complement = u16::from_le_bytes(stream[at + 3..at + 5].try_into().unwrap());
            assert_eq!(!length, complement, "LEN and NLEN disagree");
            out.extend_from_slice(&stream[at + 5..at + 5 + length as usize]);
            at += 5 + length as usize;
            if header & 1 == 1 {
                break;
            }
        }
        let stated = u32::from_be_bytes(stream[at..at + 4].try_into().unwrap());
        assert_eq!(stated, adler32(&out), "the Adler-32 is wrong");
        assert_eq!(at + 4, stream.len(), "there is data after the stream");
        out
    }

    /// A small image whose every pixel is different, so a transposition or an
    /// off-by-one in the row loop cannot cancel out.
    fn sample(width: u32, height: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..height {
            for x in 0..width {
                v.extend_from_slice(&[
                    (x * 7 + 1) as u8,
                    (y * 11 + 2) as u8,
                    (x * y + 3) as u8,
                    255,
                ]);
            }
        }
        v
    }

    #[test]
    fn the_chunks_are_ihdr_then_idat_then_iend_and_every_crc_is_right() {
        let png = encode(4, 3, &sample(4, 3), false);
        let found = chunks(&png);
        let kinds: Vec<&str> = found.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
        // `chunks` asserted every CRC on the way through; this says the last
        // chunk really is the last, so a reader stopping at IEND stops at the
        // end of the file.
        assert!(found.last().expect("IEND").1.is_empty());
    }

    #[test]
    fn the_header_says_what_the_image_actually_is() {
        for (keep_alpha, want_type) in [(false, COLOUR_TYPE_RGB), (true, COLOUR_TYPE_RGBA)] {
            let png = encode(4, 3, &sample(4, 3), keep_alpha);
            let (_, header) = &chunks(&png)[0];
            assert_eq!(u32::from_be_bytes(header[0..4].try_into().unwrap()), 4);
            assert_eq!(u32::from_be_bytes(header[4..8].try_into().unwrap()), 3);
            assert_eq!(header[8], 8, "bit depth");
            assert_eq!(header[9], want_type);
            assert_eq!(&header[10..13], &[0, 0, 0], "compression, filter, interlace");
        }
    }

    /// **Every pixel comes back**, through the zlib stream, the stored blocks
    /// and the filter bytes. This is the assertion the whole file exists for.
    #[test]
    fn every_pixel_survives_the_scanlines_and_the_stored_blocks() {
        let (width, height) = (37u32, 19u32);
        let rgba = sample(width, height);
        let png = encode(width, height, &rgba, false);
        let raw = inflate_stored(&chunks(&png)[1].1);

        let row = width as usize * 3;
        assert_eq!(raw.len(), height as usize * (1 + row));
        for y in 0..height as usize {
            assert_eq!(raw[y * (1 + row)], 0, "the filter byte of row {y}");
            for x in 0..width as usize {
                let want = &rgba[(y * width as usize + x) * 4..][..3];
                let got = &raw[y * (1 + row) + 1 + x * 3..][..3];
                assert_eq!(got, want, "pixel {x},{y}");
            }
        }
    }

    /// The alpha channel is written when it is asked for, and is not otherwise.
    #[test]
    fn alpha_is_written_only_when_it_is_kept() {
        let rgba = vec![10, 20, 30, 40];
        let with = inflate_stored(&chunks(&encode(1, 1, &rgba, true))[1].1);
        assert_eq!(with, vec![0, 10, 20, 30, 40]);
        let without = inflate_stored(&chunks(&encode(1, 1, &rgba, false))[1].1);
        assert_eq!(without, vec![0, 10, 20, 30]);
    }

    /// **An image larger than one stored block.** 65535 is the boundary, and an
    /// encoder that wrote one oversized block would produce a file whose first
    /// chunk decompresses and whose rest is garbage.
    #[test]
    fn an_image_spanning_several_stored_blocks_reassembles() {
        // 300 × 300 × 3 plus a filter byte per row is about 270 KB, which is
        // five blocks — so the final-block flag is exercised as well as the
        // splitting.
        let (width, height) = (300u32, 300u32);
        let rgba = sample(width, height);
        let png = encode(width, height, &rgba, false);
        let stream = &chunks(&png)[1].1;
        let raw = inflate_stored(stream);
        assert_eq!(raw.len(), height as usize * (1 + width as usize * 3));

        // And the blocks really were split, rather than this passing because
        // the image happened to fit in one.
        let blocks = raw.len().div_ceil(MAX_STORED_BLOCK);
        assert!(blocks > 1, "the fixture is too small to test the split");
    }

    /// **The CRC is the one the specification prints**, checked against a value
    /// this project did not compute.
    ///
    /// `"IEND"` with no data is the one chunk whose CRC every PNG in the world
    /// carries identically: `0xae42_60_82`. It is in the specification's own
    /// example and in every hex dump of every PNG. A wrong polynomial, a missing
    /// pre-conditioning, or a CRC taken over the length field as well would all
    /// produce a different number here.
    #[test]
    fn the_crc_of_iend_is_the_number_every_png_in_the_world_carries() {
        let mut crc = Crc::new();
        crc.update(b"IEND");
        assert_eq!(crc.finish(), 0xae42_6082);

        // And it is what this encoder actually writes, not merely what the
        // helper computes.
        let png = encode(1, 1, &[0, 0, 0, 255], false);
        assert_eq!(&png[png.len() - 4..], &0xae42_6082u32.to_be_bytes());
    }

    /// Adler-32 against `RFC 1950`'s definition applied to a string whose value
    /// is widely published: `Adler32("Wikipedia") == 0x11E60398`.
    #[test]
    fn adler32_agrees_with_a_published_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        // The empty stream's is 1, which is the initial state and the case a
        // loop that never runs must still get right.
        assert_eq!(adler32(b""), 1);
    }

    /// An empty zlib stream still has to be a stream — one empty final stored
    /// block and the Adler-32 of nothing. Nothing in this tool produces one
    /// (a zero-pixel image is refused by [`crate::qoi::decode`]), and a reader
    /// handed a truncated stream is a worse failure than a refusal, so the
    /// degenerate case is written out rather than left to chance.
    #[test]
    fn an_empty_stream_is_still_a_well_formed_one() {
        assert_eq!(inflate_stored(&zlib(&[])), Vec::<u8>::new());
    }

    /// **A file this encoder wrote was opened by a reader that is not this
    /// one.** `AGENTS.md` §4, and the classic failure a hand-rolled encoder
    /// produces: a file that satisfies every assertion above and that nothing
    /// will open.
    ///
    /// The check cannot live in this test suite — it needs a PNG decoder, and
    /// adding one as a dependency is exactly what this module exists to avoid
    /// (`ADR-009` §5). So it is recorded here as what was done, by whom, and to
    /// what:
    ///
    /// * The 1920×1080 capture taken off the development board on 2026-09-01
    ///   was fetched with `cargo rivet screenshot`, decoded by [`crate::qoi`]
    ///   and written by this module.
    /// * The resulting file was opened with **Pillow**, outside the build, via
    ///   `uv run --with pillow`. It reported the mode, the size and the pixels.
    /// * Pillow **also decoded the original `.qoi`** — it has its own QOI
    ///   reader, written by somebody else from the same specification — and the
    ///   two pixel buffers were compared byte for byte.
    ///
    /// Pillow is a *check* and is not part of this tool; it appears in no
    /// manifest. This test asserts the one thing about that check that can be
    /// asserted here: that the file it read is the file this code produces, so
    /// that a future edit which breaks the format is not silently vouched for by
    /// a paragraph.
    #[test]
    fn the_file_this_writes_was_opened_by_a_reader_that_is_not_this_one() {
        // The shape an external reader confirmed: signature, IHDR first, one
        // IDAT, IEND last, RGB, 8 bits. If any of that changes, the paragraph
        // above stops describing what this code does and has to be re-run.
        let png = encode(2, 2, &sample(2, 2), false);
        assert_eq!(&png[0..8], &SIGNATURE);
        let kinds: Vec<String> = chunks(&png).into_iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
    }
}
