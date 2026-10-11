//! **QOI** — the *Quite OK Image* format, written out so a screen can leave the
//! machine.
//!
//! `PLAN.md` §104.39. Ruled 2026-09-01: the capture may be full-resolution QOI,
//! and what the workstation software hands the maintainer is a PNG
//! (.rulings/ R-326). So the board writes QOI and `cargo rivet` turns it into a
//! PNG on the workstation — the split matters, because it keeps the board free
//! of a compressor it would otherwise need a dependency for.
//!
//! # Why not PNG here
//!
//! PNG is DEFLATE, and DEFLATE on the board means either a dependency or a
//! hand-written compressor. QOI is a single pass with no entropy coding, no
//! tables and no allocation beyond the output, and it is about fifty lines. On a
//! launcher screen — flat backgrounds, a few cards, some text — it does well,
//! because those are exactly the runs and repeats it encodes.
//!
//! # Why not raw
//!
//! 1920×1080×4 is 8 MB, and base64 over the control plane makes that 11 MB.
//! `rivet get file` has a read cap far below it. A screen that cannot be
//! retrieved is not a screenshot.
//!
//! # The format, in one paragraph
//!
//! A 14-byte header, then chunks, then an 8-byte end marker. Each pixel is
//! encoded as whichever is smallest: a repeat of the previous one, an index into
//! a 64-entry table of recently seen colours, a small difference from the
//! previous colour, a larger difference biased on green, or the colour in full.
//! The specification is one page and this implements it whole, minus the alpha
//! channel — a scanout buffer has none.

/// `qoif`.
const MAGIC: [u8; 4] = *b"qoif";
/// Zero seven times then one, which is how a reader knows it has the lot.
const END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xc0;
const OP_RGB: u8 = 0xfe;

/// Where a colour lands in the 64-entry table. The constants are the
/// specification's and are not a choice.
const fn index_of(r: u8, g: u8, b: u8, a: u8) -> usize {
    (r as usize * 3 + g as usize * 5 + b as usize * 7 + a as usize * 11) % 64
}

/// Encode an `XRGB8888` scanout buffer as QOI.
///
/// `stride` is bytes per row, which is **not** `width * 4` — the kernel picks it
/// (`crate::drm`), and reading it as if it were would shear the image.
///
/// Alpha is written as `255` throughout and the file declares three channels: a
/// scanout buffer's fourth byte is padding, not transparency, and passing it
/// through would make a viewer show a screen that fades out where the padding
/// happened to be low.
///
/// # `None` when the buffer does not hold the image it was asked to describe
///
/// **This used to `continue` past a pixel it could not read** — the audit of
/// 2026-09-03's LOW list, *"`qoi::encode` skips missing pixels"*. The header is
/// written first and says `width` × `height`; skipping pixels after that
/// produces a file whose header and body disagree, and every decoder in this
/// repository reads until it has `w * h` pixels. So a short buffer did not
/// fail — it **succeeded**, and the reader either ran off the end of the chunks
/// or, worse, resynchronised on the end marker and showed a sheared screen with
/// nothing anywhere saying so.
///
/// `AGENTS.md` §5: refuse rather than guess. Both production callers already
/// return a sentence for a journal, so there is somewhere for the refusal to go.
///
/// Not reachable from a correct caller — a mapped scanout buffer is
/// `height * stride` bytes and `w * 4 <= stride` — which is exactly why it had
/// to be a refusal and not a comment.
#[must_use]
pub fn encode(pixels: &[u8], width: usize, height: usize, stride: usize) -> Option<Vec<u8>> {
    // The last byte the loop below will read, `(height - 1) * stride + width *
    // 4`, checked so that dimensions from a caller that has miscomputed them
    // cannot wrap into a small number and pass.
    let last_row = height.checked_sub(1)?.checked_mul(stride)?;
    let needed = last_row.checked_add(width.checked_mul(4)?)?;
    if width == 0 || pixels.len() < needed {
        return None;
    }
    let mut out = Vec::with_capacity(width * height + 1024);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&(width as u32).to_be_bytes());
    out.extend_from_slice(&(height as u32).to_be_bytes());
    out.push(3); // channels: RGB
    out.push(0); // colourspace: sRGB with linear alpha

    let mut table = [[0u8; 4]; 64];
    // The specification's starting colour, which a decoder also assumes.
    let mut prev = [0u8, 0, 0, 255];
    let mut run = 0u8;

    for y in 0..height {
        for x in 0..width {
            let at = y * stride + x * 4;
            // The bound above already settles this. Asked again, and answered
            // with `?` rather than `continue`, so that neither route can emit a
            // file whose header claims pixels the body does not carry.
            let chunk = pixels.get(at..at + 4)?;
            // XRGB8888 little-endian: blue lowest, then green, then red.
            let px = [chunk[2], chunk[1], chunk[0], 255u8];

            if px == prev {
                run += 1;
                // A run is six bits with a bias of -1, so 62 is the most that
                // fits before it has to be flushed.
                if run == 62 {
                    out.push(OP_RUN | (run - 1));
                    run = 0;
                }
                continue;
            }
            if run > 0 {
                out.push(OP_RUN | (run - 1));
                run = 0;
            }

            let slot = index_of(px[0], px[1], px[2], px[3]);
            if table[slot] == px {
                out.push(OP_INDEX | slot as u8);
                prev = px;
                continue;
            }
            table[slot] = px;

            // Differences are computed with wrapping arithmetic, which is what
            // makes a colour that steps across zero encode small instead of
            // huge.
            let dr = px[0].wrapping_sub(prev[0]) as i8;
            let dg = px[1].wrapping_sub(prev[1]) as i8;
            let db = px[2].wrapping_sub(prev[2]) as i8;
            let dr_dg = dr.wrapping_sub(dg);
            let db_dg = db.wrapping_sub(dg);

            if (-2..=1).contains(&dr) && (-2..=1).contains(&dg) && (-2..=1).contains(&db) {
                out.push(
                    OP_DIFF
                        | ((dr + 2) as u8) << 4
                        | ((dg + 2) as u8) << 2
                        | ((db + 2) as u8),
                );
            } else if (-32..=31).contains(&dg)
                && (-8..=7).contains(&dr_dg)
                && (-8..=7).contains(&db_dg)
            {
                out.push(OP_LUMA | ((dg + 32) as u8));
                out.push(((dr_dg + 8) as u8) << 4 | ((db_dg + 8) as u8));
            } else {
                out.push(OP_RGB);
                out.push(px[0]);
                out.push(px[1]);
                out.push(px[2]);
            }
            prev = px;
        }
    }
    if run > 0 {
        out.push(OP_RUN | (run - 1));
    }
    out.extend_from_slice(&END);
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// A minimal decoder, for the tests only.
    ///
    /// **Written independently of the encoder**, from the format description
    /// rather than by inverting the code above — an encoder checked against its
    /// own mirror image agrees with itself and proves nothing.
    fn decode(bytes: &[u8]) -> (usize, usize, Vec<[u8; 3]>) {
        assert_eq!(&bytes[0..4], b"qoif");
        let w = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let h = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let mut table = [[0u8; 4]; 64];
        let mut px = [0u8, 0, 0, 255];
        let mut out = Vec::with_capacity(w * h);
        let mut i = 14;
        while out.len() < w * h {
            let b = bytes[i];
            i += 1;
            if b == OP_RGB {
                px = [bytes[i], bytes[i + 1], bytes[i + 2], 255];
                i += 3;
            } else if b & 0xc0 == OP_INDEX {
                px = table[(b & 0x3f) as usize];
            } else if b & 0xc0 == OP_DIFF {
                px[0] = px[0].wrapping_add(((b >> 4) & 0x03).wrapping_sub(2));
                px[1] = px[1].wrapping_add(((b >> 2) & 0x03).wrapping_sub(2));
                px[2] = px[2].wrapping_add((b & 0x03).wrapping_sub(2));
            } else if b & 0xc0 == OP_LUMA {
                let dg = (b & 0x3f).wrapping_sub(32);
                let second = bytes[i];
                i += 1;
                px[0] = px[0].wrapping_add(dg.wrapping_add(((second >> 4) & 0x0f).wrapping_sub(8)));
                px[1] = px[1].wrapping_add(dg);
                px[2] = px[2].wrapping_add(dg.wrapping_add((second & 0x0f).wrapping_sub(8)));
            } else {
                let run = (b & 0x3f) + 1;
                for _ in 0..run {
                    out.push([px[0], px[1], px[2]]);
                }
                continue;
            }
            table[index_of(px[0], px[1], px[2], px[3])] = px;
            out.push([px[0], px[1], px[2]]);
        }
        (w, h, out)
    }

    /// Build a scanout buffer with a stride wider than the image, which is what
    /// a real one has.
    fn buffer(width: usize, height: usize, stride: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut v = vec![0u8; stride * height];
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = f(x, y);
                let at = y * stride + x * 4;
                v[at] = b;
                v[at + 1] = g;
                v[at + 2] = r;
                v[at + 3] = 0;
            }
        }
        v
    }

    /// **Every pixel survives.** A gradient exercises every chunk type: runs at
    /// the flat left edge, small diffs along it, and full colours at the jumps.
    #[test]
    fn a_gradient_round_trips_exactly() {
        let (w, h, stride) = (64, 32, 64 * 4 + 128);
        let src = buffer(w, h, stride, |x, y| [(x * 4) as u8, (y * 8) as u8, 0x40]);
        let (dw, dh, got) = decode(&encode(&src, w, h, stride).expect("the buffer holds the image"));
        assert_eq!((dw, dh), (w, h));
        for y in 0..h {
            for x in 0..w {
                let want = [(x * 4) as u8, (y * 8) as u8, 0x40];
                assert_eq!(got[y * w + x], want, "pixel {x},{y}");
            }
        }
    }

    /// **The stride is honoured**, which is the one thing a scanout buffer will
    /// punish. Encoding `width * 4` per row instead would shear the image, and
    /// every pixel would still be *a* valid colour — so only a comparison
    /// against the source catches it.
    #[test]
    fn a_padded_stride_does_not_shear_the_image() {
        let (w, h) = (17, 5);
        let stride = w * 4 + 37;
        let src = buffer(w, h, stride, |x, y| [x as u8, y as u8, 0xff]);
        let (_, _, got) = decode(&encode(&src, w, h, stride).expect("the buffer holds the image"));
        for y in 0..h {
            for x in 0..w {
                assert_eq!(got[y * w + x], [x as u8, y as u8, 0xff], "at {x},{y}");
            }
        }
    }

    /// A flat screen is mostly one long run, which is the case a launcher
    /// background actually is — and the reason this is worth doing at all.
    #[test]
    fn a_flat_screen_compresses_hard() {
        let (w, h) = (256, 256);
        let stride = w * 4;
        let src = buffer(w, h, stride, |_, _| [0x0d, 0x11, 0x17]);
        let encoded = encode(&src, w, h, stride).expect("the buffer holds the image");
        let raw = w * h * 3;
        assert!(
            encoded.len() < raw / 50,
            "a flat screen encoded to {} bytes of a possible {raw}",
            encoded.len()
        );
        let (_, _, got) = decode(&encoded);
        assert!(got.iter().all(|p| *p == [0x0d, 0x11, 0x17]));
    }

    /// **A buffer too small for the image is REFUSED, not encoded short** —
    /// the audit of 2026-09-03's LOW list, *"`qoi::encode` skips missing
    /// pixels"*.
    ///
    /// The header goes out first and says `width` × `height`. Skipping the
    /// pixels that are not there produced a file every decoder in this
    /// repository reads by counting to `w * h` — so it did not fail, it
    /// succeeded, and the operator got a sheared screen with nothing saying
    /// why.
    ///
    /// The short case is one byte short on purpose: an obviously tiny buffer
    /// would be refused by almost any bound, and the defect was about the last
    /// row.
    #[test]
    fn a_buffer_that_does_not_hold_the_image_is_refused() {
        let (w, h, stride) = (8, 4, 8 * 4);
        let full = buffer(w, h, stride, |x, y| [x as u8, y as u8, 9]);
        assert!(encode(&full, w, h, stride).is_some(), "the whole buffer must encode");

        let short = &full[..full.len() - 1];
        assert!(
            encode(short, w, h, stride).is_none(),
            "one byte short of the image and it still produced a file"
        );
        // And the same buffer described as taller than it is.
        assert!(
            encode(&full, w, h + 1, stride).is_none(),
            "a height the buffer cannot cover still produced a file"
        );
        // Nothing to encode is not an image either.
        assert!(encode(&full, 0, h, stride).is_none());
        assert!(encode(&full, w, 0, stride).is_none());
    }

    /// The end marker is present and is the specification's, so a decoder that
    /// looks for it finds it.
    #[test]
    fn the_file_ends_the_way_a_reader_expects() {
        let src = buffer(2, 2, 8, |_, _| [1, 2, 3]);
        let encoded = encode(&src, 2, 2, 8).expect("the buffer holds the image");
        assert_eq!(&encoded[encoded.len() - 8..], &END);
        assert_eq!(&encoded[0..4], b"qoif");
    }
}
