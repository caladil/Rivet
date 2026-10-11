//! **Reading QOI** — the other half of the split `rivet-sdk/src/qoi.rs` names.
//!
//! That module opens with the ruling this one exists to honour, 2026-09-01:
//! the capture may be full-resolution QOI, and what the workstation software
//! hands the maintainer is a PNG (.rulings/ R-326). The board writes QOI
//! because an encoder is sixty lines where a PNG encoder is a compressor;
//! **the workstation is where the conversion was always supposed to happen**,
//! and until this module there was nothing anywhere in the repository that
//! could read one back. A capture taken by an operator or an agent was 67 KB of
//! base64 that nobody could look at.
//!
//! # Why here and not in `tools/`
//!
//! A side script would be a second implementation living outside the tool that
//! already talks to the board — a second place to install, a second thing to
//! keep in step with the encoder, and one that could not be reached from
//! `cargo rivet`. `ADR-011` in the small.
//!
//! # Why hand-written rather than a crate
//!
//! `ADR-009` §5: **this is the binary that reads a signing seed**, which is why
//! its ELF reader is hand-written rather than `goblin`. Every dependency it
//! gains is one more thing with the opportunity to read that seed. A QOI decoder
//! is a hundred lines and its correctness is checked here against the encoder
//! that produced the bytes — see the tests, which compile
//! `rivet-sdk/src/qoi.rs` itself rather than a copy of it.
//!
//! # The format, and the two opcodes the board never writes
//!
//! A 14-byte header, chunks, then an 8-byte end marker. Each pixel is a repeat
//! of the previous one, an index into a 64-entry table of recently seen colours,
//! a small difference, a difference biased on green, or the colour in full.
//!
//! **`QOI_OP_RGBA` and `channels = 4` are implemented even though `Screen`
//! never emits either.** A decoder is a reader of the FORMAT, not of one
//! encoder's output, and the alternative is a tool that refuses a perfectly
//! valid file for a reason that reads as a defect. Where alpha is genuinely
//! used, [`crate::png`] writes an RGBA image rather than dropping it.
//!
//! # Everything here refuses rather than guesses
//!
//! `AGENTS.md` §5. A stream that runs out mid-chunk, a run that would overrun
//! the image, a header whose dimensions the file cannot possibly hold — each is
//! an error naming what was found. **A wrong guess does not fail: it succeeds,
//! and hands back a plausible picture of nothing.**

/// `qoif`, the first four bytes of every QOI file.
const MAGIC: [u8; 4] = *b"qoif";

/// The header: magic, width, height, channels, colourspace.
const HEADER_BYTES: usize = 14;

/// Zero seven times then one. A reader that finds it knows it has the lot.
const END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

// The five opcodes, spelled as `rivet-sdk/src/qoi.rs` spells them. The two-bit
// tags are compared after the two eight-bit ones, because `0xfe & 0xc0` is
// `OP_RUN` and a decoder that matched on the tag first would read every literal
// colour as a run of 63 pixels.
const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xc0;
const OP_RGB: u8 = 0xfe;
/// **The board never writes this** — a scanout buffer has no alpha, so
/// `Screen::capture` declares three channels and pads to 255. Implemented
/// anyway; see this module's documentation.
const OP_RGBA: u8 = 0xff;

/// **The four two-bit tags are the four values of `tag & 0xc0`, and nothing
/// else.** [`decode`] branches on three of them and treats the remaining case as
/// `QOI_OP_LUMA` by construction rather than testing for it — which is only
/// sound while these four numbers are these four numbers.
///
/// Asserted here, at compile time, rather than as a branch in the decoder that
/// could never run: `AGENTS.md` §4 — a check that cannot fail proves nothing,
/// and this one fails the moment somebody renumbers an opcode.
const _: () = assert!(OP_INDEX == 0x00 && OP_DIFF == 0x40 && OP_LUMA == 0x80 && OP_RUN == 0xc0);

/// The most pixels this build will decode, whatever a header claims.
///
/// **An 8K display is 33 megapixels**, so this admits every screen anybody will
/// attach to a Rivet and refuses a header claiming a picture whose buffer would
/// be gigabytes. It is the second of two bounds and the weaker one: the first,
/// in [`decode`], is derived from the file itself and is far tighter for any
/// real capture.
const MAX_PIXELS: u64 = 40_000_000;

/// The most pixels one `QOI_OP_RUN` byte can stand for. Six bits, biased by -1.
const MAX_RUN: usize = 62;

/// A decoded image: `width * height` pixels, RGBA, row-major, top row first.
///
/// **RGBA even for a three-channel file**, because the index table is keyed on
/// four components (see [`index_of`]) and carrying two pixel shapes through the
/// decoder to save one byte per pixel in a debugging artefact would be the wrong
/// trade. [`Image::is_opaque`] is what [`crate::png`] asks before deciding
/// whether the alpha channel is worth writing out.
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, `R,G,B,A` per pixel.
    pub pixels: Vec<u8>,
    /// What the header declared: 3 or 4. Reported rather than acted on — the
    /// pixels are what they are, and a caller diagnosing a capture wants to know
    /// what the writer *said* as well as what arrived.
    pub channels: u8,
}

/// **The shape, never the pixels.**
///
/// A derived `Debug` on this type would print several million bytes into a test
/// failure or a diagnostic — burying the sentence somebody needed under the
/// picture. What is worth seeing is the shape it claims to be.
impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Image {{ {}×{}, {} channels declared, {} bytes of RGBA }}",
            self.width,
            self.height,
            self.channels,
            self.pixels.len()
        )
    }
}

impl Image {
    /// Whether every pixel is fully opaque.
    ///
    /// True for everything `Screen::capture` produces, and the question
    /// [`crate::png`] asks to decide between a three- and a four-channel PNG.
    #[must_use]
    pub fn is_opaque(&self) -> bool {
        self.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255)
    }
}

/// Where a colour lands in the 64-entry table.
///
/// The constants are the specification's and are not a choice. **The same
/// expression as the encoder's**, and it has to be: the table is not
/// transmitted, so both ends derive every slot independently and a decoder that
/// hashed differently would read a valid file as noise from the first index hit.
const fn index_of(r: u8, g: u8, b: u8, a: u8) -> usize {
    (r as usize * 3 + g as usize * 5 + b as usize * 7 + a as usize * 11) % 64
}

/// Decode a QOI file.
///
/// # Errors
///
/// A file too short to hold a header and an end marker; wrong magic; a
/// channel or colourspace byte the format does not define; zero dimensions; a
/// header claiming more pixels than the bytes present could encode or more than
/// [`MAX_PIXELS`]; a chunk that runs off the end of the file; a run that would
/// overrun the image; or a stream that does not end with the end marker.
///
/// Every one of them names what was found, because the caller is holding bytes
/// that came off a machine over a control plane and the useful question is
/// *which of the two ends is wrong*.
pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    if bytes.len() < HEADER_BYTES + END.len() {
        return Err(format!(
            "this is {} bytes, and the smallest possible QOI file is {} — a {HEADER_BYTES}-byte \
             header and an {}-byte end marker, with no pixels between them. Nothing was decoded.",
            bytes.len(),
            HEADER_BYTES + END.len(),
            END.len(),
        ));
    }
    if bytes[0..4] != MAGIC {
        return Err(format!(
            "this does not begin with `qoif`; the first four bytes are {:02x?}. A Rivet writes \
             QOI and nothing else to `screen.qoi`, so either this is a different file or it \
             did not arrive whole.",
            &bytes[0..4],
        ));
    }

    let width = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let height = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let channels = bytes[12];
    let colourspace = bytes[13];

    if width == 0 || height == 0 {
        return Err(format!(
            "the header says this image is {width}×{height}, and a picture with no pixels in it \
             is not one. A capture taken before anything was drawn is the likely cause."
        ));
    }
    if channels != 3 && channels != 4 {
        return Err(format!(
            "the header says {channels} channels, and QOI has 3 (RGB) or 4 (RGBA). A Rivet \
             writes 3 — a scanout buffer's fourth byte is padding rather than transparency."
        ));
    }
    if colourspace > 1 {
        return Err(format!(
            "the header's colourspace byte is {colourspace}, and QOI defines 0 (sRGB with a \
             linear alpha channel) and 1 (all channels linear). This build refuses rather than \
             assuming which was meant."
        ));
    }

    let pixel_count = u64::from(width) * u64::from(height);
    // **Two bounds, and the absolute one is checked first** so that an absurd
    // header is called absurd rather than merely unsupported by its own body.
    if pixel_count > MAX_PIXELS {
        return Err(format!(
            "the header claims {width}×{height} = {pixel_count} pixels and this build decodes at \
             most {MAX_PIXELS}. An 8K display is about 33 million, so this is not describing a \
             screen — it is a header asking for a buffer of several gigabytes."
        ));
    }
    // **The tighter bound, and it comes from the file rather than from a
    // constant.** One byte can stand for at most `MAX_RUN` pixels, and only a
    // run chunk manages that; every other opcode produces one pixel and some of
    // them cost four bytes. So a payload of N bytes cannot encode more than
    // `N * MAX_RUN` pixels, and a header claiming more is describing an image
    // these bytes do not contain — worth catching HERE, because the alternative
    // is allocating for the claim before reading the first chunk.
    let payload = bytes.len() - HEADER_BYTES - END.len();
    let most_the_bytes_could_hold = payload as u64 * MAX_RUN as u64;
    if pixel_count > most_the_bytes_could_hold {
        return Err(format!(
            "the header claims {width}×{height} = {pixel_count} pixels, and only {payload} bytes \
             of chunks follow it. One byte encodes at most {MAX_RUN} pixels, so these bytes \
             could hold {most_the_bytes_could_hold} at the very most: this file is TRUNCATED, \
             or its header is not describing its body. Nothing was allocated for the claim."
        ));
    }
    // `pixel_count` is bounded by `MAX_PIXELS` above, so this cannot truncate.
    let pixel_count = pixel_count as usize;

    let mut table = [[0u8; 4]; 64];
    // The specification's starting colour, which the encoder also assumes. It is
    // opaque black rather than zero: an image whose first pixel is black encodes
    // as a run, and a decoder starting from transparent black would render that
    // run invisible.
    let mut px = [0u8, 0, 0, 255];
    let mut out: Vec<u8> = Vec::with_capacity(pixel_count * 4);
    let mut at = HEADER_BYTES;

    // The last chunk in the file is followed by the end marker, so a stream that
    // is short announces itself as a chunk reading past the end rather than as
    // silence.
    let byte = |at: usize| -> Result<u8, String> {
        bytes.get(at).copied().ok_or_else(|| {
            format!(
                "a chunk starting at byte {at} reads past the end of the file, which is {} bytes \
                 long. What arrived is a TRUNCATED capture and not a short one; nothing is \
                 decoded from it.",
                bytes.len(),
            )
        })
    };

    while out.len() < pixel_count * 4 {
        let tag = byte(at)?;
        at += 1;

        // The two eight-bit opcodes first — see the comment at `OP_RGB`.
        if tag == OP_RGB {
            px = [byte(at)?, byte(at + 1)?, byte(at + 2)?, px[3]];
            at += 3;
        } else if tag == OP_RGBA {
            px = [byte(at)?, byte(at + 1)?, byte(at + 2)?, byte(at + 3)?];
            at += 4;
        } else if tag & 0xc0 == OP_RUN {
            // Six bits with a bias of -1, so the byte says 0..=61 and means
            // 1..=62. 62 and 63 are the two opcodes above and never reach here.
            let run = (tag & 0x3f) as usize + 1;
            let have = out.len() / 4;
            if have + run > pixel_count {
                return Err(format!(
                    "a run of {run} pixels starts at pixel {have} of {pixel_count}, so it runs \
                     off the end of the image. This file's header and its body disagree about \
                     how big the picture is."
                ));
            }
            for _ in 0..run {
                out.extend_from_slice(&px);
            }
            continue;
        } else if tag & 0xc0 == OP_INDEX {
            px = table[(tag & 0x3f) as usize];
        } else if tag & 0xc0 == OP_DIFF {
            // Wrapping, which is what lets a colour stepping across zero encode
            // small instead of huge — and is why the encoder computes it the
            // same way.
            px[0] = px[0].wrapping_add(((tag >> 4) & 0x03).wrapping_sub(2));
            px[1] = px[1].wrapping_add(((tag >> 2) & 0x03).wrapping_sub(2));
            px[2] = px[2].wrapping_add((tag & 0x03).wrapping_sub(2));
        } else {
            // `OP_LUMA` — the fourth and last two-bit tag; see the compile-time
            // assertion above for why no test for it is needed here. Green's
            // difference is in this byte, and red's and blue's difference FROM
            // GREEN's in the next.
            let dg = (tag & 0x3f).wrapping_sub(32);
            let second = byte(at)?;
            at += 1;
            px[0] = px[0].wrapping_add(dg.wrapping_add(((second >> 4) & 0x0f).wrapping_sub(8)));
            px[1] = px[1].wrapping_add(dg);
            px[2] = px[2].wrapping_add(dg.wrapping_add((second & 0x0f).wrapping_sub(8)));
        }

        table[index_of(px[0], px[1], px[2], px[3])] = px;
        out.extend_from_slice(&px);
    }

    // **The end marker is checked, and its absence is a refusal.** Everything
    // above could succeed on a file whose header understates its size — the
    // pixels would decode, the picture would look right, and the operator would
    // never learn that most of the capture was thrown away.
    let tail = bytes.get(at..).unwrap_or_default();
    if tail != END {
        return Err(format!(
            "the last pixel is decoded at byte {at} and the {} bytes after it are not QOI's end \
             marker. Found {:02x?}, expected {:02x?}. Either this file carries more image than \
             its header admits, or something appended to it — and this build will not decide \
             which by guessing.",
            tail.len(),
            &tail[..tail.len().min(16)],
            END,
        ));
    }

    Ok(Image {
        width,
        height,
        pixels: out,
        channels,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The encoder that writes the files this module reads — the real one,
    /// compiled from its own source file.**
    ///
    /// `AGENTS.md` §4: *"a test that round-trips a reader against its own writer
    /// proves the two agree with each other and nothing about the format"*, and
    /// *"where a format crosses an implementation boundary, test against bytes
    /// the OTHER side actually wrote"*. This is the other side. The decoder
    /// above was written from the specification and the encoder was written from
    /// the specification; neither is derived from the other, and the tests below
    /// are the only thing that says so.
    ///
    /// # Why `#[path]` rather than a dependency
    ///
    /// `rivetos-sdk` declares `mod qoi` **private and behind `feature =
    /// "screen"`**, and that feature links `libdrm` — which this Windows host
    /// does not have and which a host tool has no business linking. So a
    /// dev-dependency cannot reach the encoder today.
    ///
    /// Including the source file is not a copy: **it is the file the board
    /// compiles.** An edit to the encoder changes what this test compiles, in
    /// the same commit, which is the property that matters — a vendored copy
    /// would agree with itself for ever. The proper fix is one line in
    /// `rivet-sdk/src/lib.rs` making the module `pub` and ungated, and it is
    /// outside this crate; until it is taken, this is the honest arrangement
    /// rather than a weaker test.
    /// **The encoder is reached through `crate::device_qoi`**, declared in
    /// `main.rs`, and the reason is a corrected mistake: `#[path]` on a module
    /// inside `mod tests` resolves against `src/qoi/tests/`, a directory that
    /// does not exist — so the `..` components have to be walked through
    /// nothing, and whether that resolves is the host's business rather than
    /// the compiler's. It worked on Windows and failed in the build container,
    /// which meant `cargo test --workspace` had been failing there at this
    /// crate: the standing check, silently not standing.
    use crate::device_qoi as device;

    /// An `XRGB8888` scanout buffer, built the way the kernel hands one over:
    /// blue lowest, then green, then red, then a padding byte, and a `stride`
    /// wider than the row.
    fn scanout(
        width: usize,
        height: usize,
        stride: usize,
        f: impl Fn(usize, usize) -> [u8; 3],
    ) -> Vec<u8> {
        let mut buffer = vec![0u8; stride * height];
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = f(x, y);
                let at = y * stride + x * 4;
                buffer[at] = b;
                buffer[at + 1] = g;
                buffer[at + 2] = r;
                // Deliberately NOT 255. It is padding, and an encoder that
                // passed it through would produce an image that fades out
                // wherever the kernel happened to leave a low byte.
                buffer[at + 3] = 0;
            }
        }
        buffer
    }

    /// Encode with the device's encoder, decode with this one, and compare every
    /// pixel against the source buffer.
    ///
    /// Returns the encoded length so a caller can assert on it.
    fn round_trip(
        width: usize,
        height: usize,
        stride: usize,
        f: impl Fn(usize, usize) -> [u8; 3],
    ) -> usize {
        let source = scanout(width, height, stride, &f);
        let encoded = device::encode(&source, width, height, stride)
            .expect("the buffer holds the image");
        let image = decode(&encoded).expect("the device's own encoder writes readable QOI");

        assert_eq!((image.width as usize, image.height as usize), (width, height));
        assert_eq!(image.channels, 3, "a scanout buffer has no alpha");
        assert_eq!(image.pixels.len(), width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = f(x, y);
                let at = (y * width + x) * 4;
                assert_eq!(
                    &image.pixels[at..at + 4],
                    &[r, g, b, 255],
                    "pixel {x},{y} came back wrong"
                );
            }
        }
        encoded.len()
    }

    /// **Every opcode, in one image, checked pixel for pixel.**
    ///
    /// This is the test the whole module exists for, and the shape of the image
    /// is chosen rather than arbitrary — *"a decoder that only handles runs will
    /// decode a screenshot of a mostly-black screen perfectly and fail on
    /// anything real"*. Each band below is there to force one chunk type out of
    /// the encoder, and [`every_opcode_is_actually_exercised`] proves they do
    /// rather than assuming it.
    fn every_opcode(x: usize, y: usize) -> [u8; 3] {
        match y % 5 {
            // A flat band: the second pixel onward is QOI_OP_RUN.
            0 => [0x0d, 0x11, 0x17],
            // Steps of one: QOI_OP_DIFF, whose range is -2..=1 per channel.
            1 => [(x % 4) as u8, (x % 3) as u8, (x % 2) as u8],
            // Steps of about ten in green with red and blue following it:
            // QOI_OP_LUMA, whose range is ±32 on green and ±8 on the other two
            // relative to green.
            2 => [(x * 10) as u8, (x * 10 + 3) as u8, (x * 10 + 250) as u8],
            // Colours that jump: QOI_OP_RGB, and then the repeats of those same
            // colours a few pixels later become QOI_OP_INDEX hits.
            3 => [(x * 71) as u8, (x * 131) as u8, (x * 197) as u8],
            _ => [((x / 3) * 64) as u8, 0x80, ((x % 7) * 36) as u8],
        }
    }

    #[test]
    fn an_image_using_every_opcode_round_trips_through_the_device_encoder() {
        round_trip(64, 40, 64 * 4 + 128, every_opcode);
    }

    /// **The band test is worth nothing if the bands do not produce the chunks
    /// they were written for.** A decoder that only handles runs passes a
    /// round-trip over a flat screen, and this is what stops that from being
    /// what the test above quietly became.
    ///
    /// It walks the encoded stream and counts each opcode, which is a second
    /// reader of the format — deliberately, because it must not share a bug with
    /// [`decode`]. It counts tags without interpreting them; the only thing it
    /// has to know is how many bytes each chunk occupies.
    #[test]
    fn every_opcode_is_actually_exercised() {
        let source = scanout(64, 40, 64 * 4 + 128, every_opcode);
        let encoded = device::encode(&source, 64, 40, 64 * 4 + 128)
            .expect("the buffer holds the image");

        let (mut index, mut diff, mut luma, mut run, mut rgb) = (0, 0, 0, 0, 0);
        let mut at = HEADER_BYTES;
        while at < encoded.len() - END.len() {
            let tag = encoded[at];
            let width = if tag == OP_RGB {
                rgb += 1;
                4
            } else if tag == OP_RGBA {
                4 + 1
            } else if tag & 0xc0 == OP_RUN {
                run += 1;
                1
            } else if tag & 0xc0 == OP_INDEX {
                index += 1;
                1
            } else if tag & 0xc0 == OP_DIFF {
                diff += 1;
                1
            } else {
                luma += 1;
                2
            };
            at += width;
        }

        for (name, count) in [
            ("QOI_OP_INDEX", index),
            ("QOI_OP_DIFF", diff),
            ("QOI_OP_LUMA", luma),
            ("QOI_OP_RUN", run),
            ("QOI_OP_RGB", rgb),
        ] {
            assert!(
                count > 0,
                "the fixture produced no {name}, so the round-trip above proves nothing about it"
            );
        }
    }

    /// **A flat screen is one long run**, which is what a launcher background
    /// actually is and the case a run-only decoder would pass. It is here as the
    /// boundary of the run flush: 62 is the most one chunk holds, and an image
    /// wider than that forces the encoder to break a run in the middle.
    #[test]
    fn a_flat_screen_is_runs_and_still_comes_back_whole() {
        let bytes = round_trip(200, 3, 200 * 4, |_, _| [0x0d, 0x11, 0x17]);
        assert!(
            bytes < 200 * 3 * 3 / 20,
            "a flat screen encoded to {bytes} bytes, which is not a run"
        );
    }

    /// **The stride is honoured.** Encoding `width * 4` per row instead would
    /// shear the image, and every pixel would still be *a* valid colour — so
    /// only a comparison against the source catches it. The decoder has no
    /// stride at all, which is the point: a QOI file's rows are packed, and this
    /// asserts the decoder does not invent one.
    #[test]
    fn a_padded_stride_on_the_encoding_side_does_not_shear_the_decoded_image() {
        round_trip(17, 5, 17 * 4 + 37, |x, y| [x as u8, y as u8, 0xff]);
    }

    /// One pixel is a legal image, and the smallest one that exists.
    #[test]
    fn a_single_pixel_image_round_trips() {
        round_trip(1, 1, 4, |_, _| [1, 2, 3]);
    }

    /// A run that ends exactly at the last pixel of the image — the case where
    /// the encoder's post-loop flush is the last chunk in the file, and where an
    /// off-by-one in the end-marker check would fire.
    #[test]
    fn an_image_ending_in_a_run_decodes_to_the_last_pixel() {
        round_trip(8, 8, 32, |x, y| {
            if y == 7 { [9, 9, 9] } else { [x as u8, y as u8, 0] }
        });
    }

    // -- Hand-composed vectors -------------------------------------------
    //
    // Every one of these is HAND-COMPOSED from the specification rather than
    // captured, and `AGENTS.md` §4 asks that the difference be visible in the
    // file. They prove one opcode each in isolation, which the round-trips above
    // cannot: an encoder never emits a chunk it thinks is wasteful, so the only
    // way to check a decoder against, say, `QOI_OP_RGBA` is to write one by
    // hand.

    /// A QOI file with the given dimensions and chunk bytes.
    fn compose(width: u32, height: u32, channels: u8, chunks: &[u8]) -> Vec<u8> {
        let mut out = Vec::from(MAGIC);
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.push(channels);
        out.push(0);
        out.extend_from_slice(chunks);
        out.extend_from_slice(&END);
        out
    }

    #[test]
    fn a_literal_rgb_chunk_is_the_colour_it_carries() {
        let file = compose(1, 1, 3, &[OP_RGB, 0x11, 0x22, 0x33]);
        let image = decode(&file).expect("one literal pixel");
        assert_eq!(image.pixels, vec![0x11, 0x22, 0x33, 255]);
    }

    /// **`QOI_OP_RGBA` and its alpha survive.** `Screen::capture` never writes
    /// one; the format has it, so this reads it — and the alpha is what
    /// [`crate::png`] uses to decide the PNG's colour type.
    #[test]
    fn a_literal_rgba_chunk_keeps_its_alpha() {
        let file = compose(1, 1, 4, &[OP_RGBA, 0x11, 0x22, 0x33, 0x44]);
        let image = decode(&file).expect("one literal pixel with alpha");
        assert_eq!(image.pixels, vec![0x11, 0x22, 0x33, 0x44]);
        assert!(!image.is_opaque());
        assert_eq!(image.channels, 4);
    }

    /// A diff is signed and it wraps. `0` minus `2` is `254`, and a decoder
    /// using saturating arithmetic would answer `0` — a plausible colour, in a
    /// picture that would look almost right.
    #[test]
    fn a_diff_chunk_wraps_rather_than_saturating() {
        // dr = -2, dg = -2, db = -2 against the starting colour (0,0,0,255).
        let file = compose(1, 1, 3, &[OP_DIFF]);
        let image = decode(&file).expect("one diff");
        assert_eq!(image.pixels, vec![254, 254, 254, 255]);
    }

    /// The luma chunk's second byte is red and blue **relative to green's own
    /// difference**, which is the part of the format easiest to get subtly
    /// wrong: dropping the `dg` term produces an image with the right shape and
    /// the wrong colours.
    #[test]
    fn a_luma_chunks_second_byte_is_relative_to_green() {
        // dg = +10; dr - dg = +1; db - dg = -1.
        let file = compose(1, 1, 3, &[OP_LUMA | (10 + 32), ((1 + 8) << 4) | (8 - 1)]);
        let image = decode(&file).expect("one luma");
        assert_eq!(image.pixels, vec![11, 10, 9, 255]);
    }

    /// **An index hit reads the table both ends built independently.** The
    /// table is never transmitted, so this is the opcode where a decoder that
    /// hashed differently produces noise — and it produces it only after the
    /// first repeat, which is why a small fixture can miss it entirely.
    #[test]
    fn an_index_chunk_reads_the_slot_the_encoder_would_have_written() {
        let colour = [0x11u8, 0x22, 0x33];
        let slot = index_of(colour[0], colour[1], colour[2], 255) as u8;
        let file = compose(
            1,
            3,
            3,
            &[
                OP_RGB,
                colour[0],
                colour[1],
                colour[2],
                // Something else, so the index below is not simply the previous
                // pixel repeating.
                OP_RGB,
                0x99,
                0x88,
                0x77,
                OP_INDEX | slot,
            ],
        );
        let image = decode(&file).expect("three pixels");
        assert_eq!(&image.pixels[8..12], &[0x11, 0x22, 0x33, 255]);
    }

    /// A run says one less than it means, so the byte `0xc0` is one pixel and
    /// not zero.
    #[test]
    fn a_run_chunk_is_biased_by_one() {
        let file = compose(1, 3, 3, &[OP_RUN | 2]);
        let image = decode(&file).expect("a run of three");
        assert_eq!(image.pixels, [0u8, 0, 0, 255].repeat(3));
    }

    // -- Refusals ---------------------------------------------------------

    #[test]
    fn a_file_that_is_not_qoi_is_refused_by_its_magic() {
        let png = [0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let why = decode(&png).expect_err("a PNG is not a QOI");
        assert!(why.contains("qoif"), "{why}");
    }

    #[test]
    fn a_file_too_short_to_be_one_says_so_rather_than_indexing_past_the_end() {
        for length in 0..HEADER_BYTES + END.len() {
            let short = vec![0u8; length];
            let why = decode(&short).expect_err("too short");
            assert!(why.contains("smallest possible"), "at {length}: {why}");
        }
    }

    /// **The bound that comes from the file rather than from a constant.** A
    /// header claiming a large picture with nothing behind it must be refused
    /// BEFORE anything is allocated for the claim.
    #[test]
    fn a_header_claiming_more_pixels_than_the_bytes_could_hold_is_refused() {
        let file = compose(4096, 4096, 3, &[OP_RUN | 61]);
        let why = decode(&file).expect_err("16 megapixels in one byte");
        assert!(why.contains("TRUNCATED"), "{why}");
        assert!(why.contains("16777216"), "it names the claim: {why}");
    }

    /// And the absolute cap, which is checked first so that an absurd header is
    /// called absurd rather than merely unsupported by its own body.
    #[test]
    fn a_header_past_the_absolute_cap_is_refused() {
        let file = compose(60_000, 60_000, 3, &[OP_RUN | 61]);
        let why = decode(&file).expect_err("3.6 gigapixels");
        assert!(why.contains("decodes at most"), "{why}");
    }

    #[test]
    fn a_zero_dimension_is_refused() {
        let why = decode(&compose(0, 4, 3, &[])).expect_err("no width");
        assert!(why.contains("0×4"), "{why}");
    }

    #[test]
    fn a_channel_count_the_format_does_not_have_is_refused() {
        let why = decode(&compose(1, 1, 2, &[OP_RGB, 1, 2, 3])).expect_err("two channels");
        assert!(why.contains("2 channels"), "{why}");
    }

    /// A chunk that starts before the end of the file and reads past it. The
    /// distinction the message draws — truncated, not short — is the useful one:
    /// the transfer lost bytes, and re-fetching is the fix.
    #[test]
    fn a_chunk_that_reads_past_the_end_is_refused_as_truncated() {
        // Three pixels claimed, two literals present, and then an `OP_RGB` tag
        // with nothing behind it and no end marker — a transfer that stopped.
        let mut file = Vec::from(MAGIC);
        file.extend_from_slice(&3u32.to_be_bytes());
        file.extend_from_slice(&1u32.to_be_bytes());
        file.push(3);
        file.push(0);
        file.extend_from_slice(&[OP_RGB, 1, 2, 3, OP_RGB, 4, 5, 6, OP_RGB]);
        let why = decode(&file).expect_err("the chunk is cut off");
        assert!(why.contains("TRUNCATED"), "{why}");
    }

    /// A run overrunning the image is a header and a body that disagree, and
    /// decoding it would either panic or silently produce a taller picture.
    #[test]
    fn a_run_that_overruns_the_image_is_refused() {
        let file = compose(2, 1, 3, &[OP_RUN | 9]);
        let why = decode(&file).expect_err("ten pixels into a two-pixel image");
        assert!(why.contains("runs off the end"), "{why}");
    }

    /// **A file with more image than its header admits decodes cleanly and is
    /// still wrong.** Without the end-marker check the operator would get a
    /// picture that looked right and was missing most of the capture.
    #[test]
    fn a_stream_that_does_not_end_at_the_marker_is_refused() {
        let mut file = compose(1, 1, 3, &[OP_RGB, 1, 2, 3]);
        // More chunks after the marker: exactly what a header understating the
        // height would leave behind.
        file.extend_from_slice(&[OP_RGB, 4, 5, 6]);
        let why = decode(&file).expect_err("there is more after the marker");
        assert!(why.contains("end marker"), "{why}");
    }
}
