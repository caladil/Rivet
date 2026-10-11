//! **Aura's Digital Rain face, measured the way Aura measures it.**
//!
//! Ported from `examples/aura/aura-renderer/src/typeface.rs`, trimmed to what
//! `crate::background`'s `DigitalRain` actually calls (`Typeface::draw` and
//! nothing else — this pack has no clock and no weather panel, so
//! `Typeface::measure`, `Typeface::ink` and the `GlyphInk` struct the
//! renderer's copy carries for THOSE have no caller here and are not ported;
//! see `embedded_face.rs`'s doc comment for the same trim one layer down).
//! Every number that survives the trim is unchanged.
//!
//! `SPEC-04` §6's whole argument applies unchanged: **the maintainer's
//! acceptance criterion is "pixel perfect", and a font drawn at "18px" is not
//! the same size in two rasterisers.** `fontdue` — what [`Face`] rasterises
//! through — treats `px` as a fraction of the **em square**; `stb_truetype` —
//! what raylib rasterises through, and therefore what `ss_matrix.hpp`'s
//! `kCellW`/`kCellH` were chosen against — treats `px` as the
//! **ascent-to-descent height**. IPAGothic's two numbers are equal (both
//! 2048), so Digital Rain's 18px cell happens to need no conversion — but the
//! conversion is still computed rather than assumed, because `AuraRain.ttf`
//! is a subset of the same face and nothing here should depend on that
//! coincidence holding by construction.
//!
//! # Where the font bytes come from
//!
//! `RAIN_FONT_BYTES` is `include_bytes!("../assets/AuraRain.ttf")` — the same
//! file, copied byte for byte from `examples/aura/aura-renderer/assets/`,
//! with its licence (`assets/IPAFont-LICENSE.txt`, also copied) beside it, as
//! IPA Font License 1.0 Article 3.1(3) requires. See `embedded_face.rs` for
//! why this pack embeds the font instead of shipping it as a loose asset the
//! way an application does.

use crate::embedded_face::Face;
use rivet_sdk::text::Target;

/// `AuraRain.ttf`, embedded at compile time. See this module's doc comment
/// and `embedded_face.rs` for why: a shared library has no `RIVET_APP_DIR` of
/// its own to load a loose asset from.
const RAIN_FONT_BYTES: &[u8] = include_bytes!("../assets/AuraRain.ttf");

/// One of Aura's faces, with raylib's metrics over it.
pub struct Typeface {
    face: Face,
    /// raylib's `Font::baseSize` — the size its atlas was rasterised at.
    ///
    /// It is not a detail. Every integer advance and the baseline offset are
    /// computed at this size and then scaled, so a face declared with the wrong
    /// one measures wrongly at every other size.
    base_size: f32,
    units_per_em: f32,
    ascent: f32,
    descent: f32,
}

impl Typeface {
    /// Load `AuraRain.ttf` from the bytes this crate embeds and declare the
    /// atlas size the C++ built it at (18, for Digital Rain — `CELL_H` in
    /// `background.rs`).
    ///
    /// # Errors
    ///
    /// [`Face::from_bytes`]'s sentence, unchanged — see that function for why
    /// this realistically never fires against `RAIN_FONT_BYTES`.
    pub fn load_embedded(base_size: f32) -> Result<Self, String> {
        let face = Face::from_bytes(RAIN_FONT_BYTES)?;
        let units_per_em = face.units_per_em();
        let (ascent, descent, _) = face.vertical_metrics(units_per_em).ok_or_else(|| {
            "AuraRain.ttf: no horizontal line metrics, so nothing can be laid out on it"
                .to_string()
        })?;
        Ok(Self {
            face,
            base_size,
            units_per_em,
            ascent,
            descent,
        })
    }

    /// What `stbtt_ScaleForPixelHeight` returns for `px`: font units to pixels.
    fn stb_scale(&self, px: f32) -> f32 {
        px / (self.ascent - self.descent)
    }

    /// The `px` to hand `fontdue` so that it draws what `stb_truetype` would
    /// draw at `px`.
    #[must_use]
    pub fn raster_px(&self, px: f32) -> f32 {
        self.units_per_em * self.stb_scale(px)
    }

    /// `LoadFontData`'s `(int)(ascent * scaleFactor)`, at the atlas size.
    ///
    /// Truncated, not rounded, and truncated once — a fresh truncation at each
    /// draw size would step differently and put the clock a pixel off itself
    /// between two sizes that should agree.
    fn ascent_offset(&self) -> f32 {
        (self.ascent * self.stb_scale(self.base_size)) as i32 as f32
    }

    /// `LoadFontData`'s `chars[i].advanceX`, at the atlas size, in whole pixels.
    fn advance_at_base(&self, ch: char) -> f32 {
        // `advance_of` at the em square is the raw `hmtx` value, because
        // `fontdue`'s scale there is exactly one.
        let units = self.face.advance_of(ch, self.units_per_em);
        (units * self.stb_scale(self.base_size)) as i32 as f32
    }

    /// Where the baseline falls for text whose line box top is at `y`.
    #[must_use]
    pub fn baseline(&self, y: f32, px: f32) -> f32 {
        y + self.ascent_offset() * (px / self.base_size)
    }

    /// `DrawTextEx(font, text, {x, y}, px, 0, colour)`, at `alpha`.
    ///
    /// `x` and `y` are the line box's top-left, as raylib's are. They are
    /// rounded to whole pixels on the way in — see the module note about
    /// subpixel placement, which is one of the differences that cannot be
    /// closed.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        canvas: &mut [u8],
        t: &Target,
        x: f32,
        y: f32,
        text: &str,
        px: f32,
        colour: [u8; 3],
        alpha: u8,
    ) {
        let scale = px / self.base_size;
        let raster = self.raster_px(px);
        let baseline = self.baseline(y, px);
        let mut pen = x;
        let mut buffer = [0u8; 4];
        for ch in text.chars() {
            // Negative coordinates are off the top or the left of the screen.
            // Dropped rather than clamped: a glyph clamped to zero would appear
            // stacked at the edge, which reads as a drawing bug rather than as
            // something being off-screen.
            if pen >= 0.0 && baseline >= 0.0 {
                self.face.draw_alpha(
                    canvas,
                    t,
                    round_to_usize(pen),
                    round_to_usize(baseline),
                    ch.encode_utf8(&mut buffer),
                    raster,
                    colour,
                    alpha,
                );
            }
            pen += self.advance_at_base(ch) * scale;
        }
    }
}

/// Nearest whole pixel, never below zero.
fn round_to_usize(v: f32) -> usize {
    if v <= 0.0 { 0 } else { (v + 0.5) as usize }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The conversion this module exists for**, checked against the number
    /// `fontTools` reports for the shipped file — the same check
    /// `aura-renderer/src/typeface.rs`'s own test makes, against the same font.
    #[test]
    fn a_raylib_size_converts_to_the_fontdue_size_that_draws_the_same_glyph() {
        let rain = Typeface::load_embedded(18.0).expect("the embedded rain face");
        assert!(
            (rain.raster_px(18.0) - 18.0).abs() < 0.001,
            "IPAGothic's ascent-to-descent is its em, so 18 is 18"
        );
    }

    /// `Typeface::draw` actually paints something, into a canvas that was not
    /// already blank — the guard `aura-renderer`'s own typeface tests use.
    #[test]
    fn drawing_a_glyph_changes_a_textured_canvas() {
        let rain = Typeface::load_embedded(18.0).expect("the embedded rain face");
        let t = Target::xrgb(20, 20, 20 * 4);
        let textured: Vec<u8> = (0..t.stride * t.height).map(|i| (i * 37 % 251) as u8).collect();
        let mut canvas = textured.clone();
        rain.draw(&mut canvas, &t, 0.0, 18.0, "0", 18.0, [180, 255, 180], 255);
        assert_ne!(canvas, textured, "drawing '0' changed nothing");
    }
}
