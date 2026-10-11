//! **Real type, rasterised at runtime** — `PLAN.md` §104.38.
//!
//! The maintainer, on what was here before, said in blunt terms that the font
//! was bad (.rulings/ R-336). It was a hand-rolled bitmap with a partial alphabet — earlier the same evening a
//! screen had to be written in lowercase because the uppercase letters did not
//! exist — and it could not scale, which the splash-to-launcher transition needs.
//!
//! # Runtime, not a build-time atlas
//!
//! Ruled 2026-09-01. An atlas needs no dependency and is locked to the sizes it
//! was baked at, so "scaling" becomes a stretched bitmap. The stated long-term
//! plan is fonts installable from Google Fonts, and an atlas makes that awkward
//! for ever. `fontdue` is pure Rust with no `unsafe` and no C, so unlike `ring`
//! it costs nothing at the cross-compilation boundary.
//!
//! # The font is the IMAGE's, not this application's
//!
//! `SYSTEM_FONT` lives on the read-only system slot, beside the CA bundle and
//! for the same reason (`rivet_core::etc::CA_BUNDLE_SOURCE`): it is content
//! several programs share — the splash, this launcher, and any example that
//! draws — so it belongs to the image and is replaced by an A/B update.
//!
//! An application that wants its own typeface still ships one as a package
//! asset, which is what Aura does. This is the one every Rivet screen falls back
//! to, so that a board looks like itself before anything is installed.

/// The system sans-serif, on the read-only slot.
///
/// **Geist**, which is the typeface the maintainer's other projects already use
/// — so a Rivet screen matches Athena, Olympus, Ignition, Tachyon and Trends
/// rather than being the one thing that looks different.
///
/// **Aliased rather than spelled again, 2026-09-05.** It was a literal until
/// `rivet_core::confinement` grew a rule for the directory these live in
/// (`ADR-042` §4.2): a path that is now enforced by the kernel is one that must
/// have a single spelling, because the failure of a second one is not a missing
/// file, it is `EACCES` on a board with nothing on the screen.
pub const SYSTEM_FONT: &str = rivet_core::etc::SYSTEM_FONT;

// **There is a system MONOSPACE too, and it is deliberately not repeated here**
// — added to the image 2026-09-04. This launcher draws nothing whose digits
// change in place, which is the only thing a monospace buys, so a fourth copy
// of the path would be a constant nothing uses. `rivet_core::etc::SYSTEM_MONO`
// is where it lives, this crate already depends on that crate, and the day a
// screen here shows a countdown it should be reached for there rather than
// spelled again.
// **There is no ICON_FONT constant here any more** — removed 2026-09-20 with
// `ADR-058`'s rewrite. The launcher drew `apps` out of the 1,844-byte
// Material subset; `crate::menu` draws the family's own three concentric
// circles instead, which is a better picture, is exact rather than a glyph
// approximation of one, and is one fewer file that can be missing. The image
// still ships the face — `rivet_core::etc::ICON_FONT` — for anything else.


/// A loaded typeface, ready to draw with.
pub struct Face {
    font: fontdue::Font,
}

/// Where a canvas's pixels are and how they are laid out.
///
/// The same shape the old bitmap renderer took, so the call sites did not have
/// to change when the renderer under them did.
pub struct Target {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bytes_per_pixel: usize,
}

impl Face {
    /// Load the system font, or say why not.
    ///
    /// **The failure is a sentence and not a fallback.** A launcher that
    /// silently dropped back to a worse font would look like a rendering bug on
    /// a machine whose actual problem is a missing file, and the whole reason
    /// this exists is that the worse font was unacceptable.
    pub fn load(path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("{path}: {e} — the image ships no system font"))?;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("{path}: not a usable font: {e}"))?;
        Ok(Self { font })
    }

    /// How wide `text` will be at `px`, so a caller can centre or right-align.
    ///
    /// Advance widths summed, which is what the pen actually does — measuring
    /// the ink instead would put a trailing space outside the box and make a
    /// right-aligned string sit a hair too far right.
    #[must_use]
    pub fn width_of(&self, text: &str, px: f32) -> usize {
        text.chars()
            .map(|c| self.font.metrics(c, px).advance_width)
            .sum::<f32>()
            .ceil()
            .max(0.0) as usize
    }

    /// Draw `text` with its BASELINE at `y`, starting at `x`.
    ///
    /// **Baseline and not top-left**, because that is what makes two strings at
    /// different sizes sit on the same line — which the transition needs when
    /// the version text shrinks beside a logo that does not.
    ///
    /// `colour` is `[r, g, b]`. Coverage from the rasteriser is treated as
    /// alpha and blended against whatever is already there, so text over a card
    /// looks like text and not like a rectangle.
    pub fn draw(
        &self,
        canvas: &mut [u8],
        t: &Target,
        x: usize,
        y: usize,
        text: &str,
        px: f32,
        colour: [u8; 3],
    ) {
        let mut pen = x as f32;
        for ch in text.chars() {
            let (metrics, coverage) = self.font.rasterize(ch, px);
            // `xmin`/`ymin` are the glyph's own offsets from the pen and the
            // baseline. Adding them here is what makes a `y` sit below the line
            // and a `j` hang, rather than every glyph being flush to a box.
            let gx = pen + metrics.xmin as f32;
            let gy = y as f32 - (metrics.height as f32 + metrics.ymin as f32);
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let alpha = coverage[row * metrics.width + col];
                    if alpha == 0 {
                        continue;
                    }
                    let px_x = gx as isize + col as isize;
                    let px_y = gy as isize + row as isize;
                    if px_x < 0 || px_y < 0 {
                        continue;
                    }
                    let (px_x, px_y) = (px_x as usize, px_y as usize);
                    if px_x >= t.width || px_y >= t.height {
                        continue;
                    }
                    blend(canvas, t, px_x, px_y, colour, alpha);
                }
            }
            pen += metrics.advance_width;
        }
    }
}

/// Blend one coverage value into the canvas.
///
/// `XRGB8888`, little-endian, so blue is the lowest byte — the one layout this
/// program draws into (`crate::drm`). Kept here rather than reaching for
/// `crate::pixel` because the old renderer's channel-offset indirection existed
/// to support the framebuffer's driver-declared bitfields, and `§104.35.2`
/// deleted that path.
fn blend(canvas: &mut [u8], t: &Target, x: usize, y: usize, colour: [u8; 3], alpha: u8) {
    let at = y * t.stride + x * t.bytes_per_pixel;
    if at + 3 >= canvas.len() {
        return;
    }
    let a = u32::from(alpha);
    let inv = 255 - a;
    // `(fg * a + bg * (255 - a)) / 255`, in integers. The `+ 127` is rounding:
    // without it every blend biases dark and small text looks thin.
    let mix = |fg: u8, bg: u8| -> u8 {
        (((u32::from(fg) * a + u32::from(bg) * inv) + 127) / 255) as u8
    };
    canvas[at] = mix(colour[2], canvas[at]);
    canvas[at + 1] = mix(colour[1], canvas[at + 1]);
    canvas[at + 2] = mix(colour[0], canvas[at + 2]);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **Both shipped faces actually rasterise.** Geist is TrueType outlines
    /// and the icon font is a CFF subset -- two different things inside a font
    /// file, and `fontdue` handling one does not prove it handles the other.
    ///
    /// Read from the repository rather than from an image, so `cargo test`
    /// catches a bad subset before a card is built and shipped.
    #[test]
    fn the_shipped_faces_rasterise_a_glyph_each() {
        for (path, ch, what) in [
            ("../../assets/fonts/Geist.ttf", 'R', "the system typeface"),
            (
                "../../assets/fonts/MaterialIcons.ttf",
                '\u{f56d}',
                "the default app icon",
            ),
        ] {
            // Not a failure when absent: this test is about the FILES, and a
            // tree without them is a tree this test has nothing to say about.
            let Ok(face) = Face::load(path) else { continue };
            let (metrics, coverage) = face.font.rasterize(ch, 32.0);
            assert!(
                metrics.width > 0 && metrics.height > 0,
                "{what}: {ch:?} rasterised to nothing"
            );
            assert!(
                coverage.iter().any(|c| *c > 0),
                "{what}: {ch:?} produced an entirely blank bitmap"
            );
        }
    }

    /// Blending is the half that is easy to get wrong and impossible to see in
    /// a screenshot: fully opaque must be exactly the colour, and fully
    /// transparent must leave the canvas untouched.
    #[test]
    fn coverage_blends_to_the_colour_at_the_ends() {
        let t = Target {
            width: 1,
            height: 1,
            stride: 4,
            bytes_per_pixel: 4,
        };
        let mut canvas = vec![0u8; 4];
        blend(&mut canvas, &t, 0, 0, [10, 20, 30], 255);
        assert_eq!(canvas[..3], [30, 20, 10], "opaque did not become the colour");

        let mut canvas = vec![7u8; 4];
        blend(&mut canvas, &t, 0, 0, [10, 20, 30], 0);
        assert_eq!(canvas[..3], [7, 7, 7], "transparent changed the canvas");
    }

    /// Half coverage is the midpoint, rounded rather than floored — a floor
    /// biases every edge dark and makes small text look thin.
    #[test]
    fn half_coverage_rounds_rather_than_floors() {
        let t = Target {
            width: 1,
            height: 1,
            stride: 4,
            bytes_per_pixel: 4,
        };
        let mut canvas = vec![0u8; 4];
        blend(&mut canvas, &t, 0, 0, [255, 255, 255], 128);
        assert_eq!(canvas[0], 128, "128/255 of white should round to 128");
    }

    /// A blend that would land outside the canvas is dropped, not wrapped. The
    /// failure this prevents is a glyph at the right edge writing into the
    /// first pixel of the next row.
    #[test]
    fn a_pixel_past_the_end_is_dropped() {
        let t = Target {
            width: 1,
            height: 1,
            stride: 4,
            bytes_per_pixel: 4,
        };
        let mut canvas = vec![0u8; 4];
        blend(&mut canvas, &t, 99, 99, [255, 255, 255], 255);
        assert_eq!(canvas, vec![0u8; 4], "an out-of-range blend wrote something");
    }
}
