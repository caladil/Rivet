//! **Aura's typefaces, measured the way Aura measures them.**
//!
//! `SPEC-04` §6. This module exists for one reason and it is worth stating
//! before any of the arithmetic: **the maintainer's acceptance criterion for
//! this port is "pixel perfect", and a font drawn at "292 px" is not the same
//! size in two rasterisers.**
//!
//! # The 25.7% that made the font look wrong
//!
//! `fontdue` — what `rivet_sdk::text` rasterises through — treats `px` as a
//! fraction of the **em square**: its scale is `px / units_per_em`.
//!
//! `stb_truetype` — what raylib rasterises through, and therefore what every
//! number in `cpp/src/main.cpp` and `cpp/src/widgets.hpp` was chosen against —
//! treats `px` as the **ascent-to-descent height**: `stbtt_ScaleForPixelHeight`
//! is `px / (ascent - descent)`, both read from `hhea`.
//!
//! Those agree only when a font's `ascent - descent` happens to equal its em.
//! Measured with `fontTools` on the three faces this application ships:
//!
//! | face | `unitsPerEm` | `hhea` ascent | descent | ascent − descent |
//! | :-- | --: | --: | --: | --: |
//! | `SourceCodePro-Bold` | 1000 | 984 | −273 | **1257** |
//! | `SourceCodePro-Regular` | 1000 | 984 | −273 | **1257** |
//! | `AuraRain` | 2048 | 1802 | −246 | 2048 |
//!
//! So raylib's `kClockFontSize = 292` draws Source Code Pro at `292/1257` of an
//! em — 232.3 px per em — and `fontdue` asked for 292 draws it at 292 px per
//! em. **The Rust clock was 25.7% larger than Aura's**, which is the "the font
//! is wrong" the maintainer was looking at, on top of it being Geist and not
//! Source Code Pro at all.
//!
//! IPAGothic's two numbers are equal, so Digital Rain's 18 px cell needs no
//! conversion — which is worth knowing, because it means the rain would have
//! looked right while the clock did not.
//!
//! # The other three things raylib does that a naive port does not
//!
//! 1. **Advances are integers, fixed at the atlas size.** `LoadFontData` stores
//!    `(int)(advanceX * scale)` — truncated, once, at `Font::baseSize` — and
//!    `MeasureTextEx` then multiplies that integer by `fontSize / baseSize`. A
//!    float advance summed per glyph drifts from it; over `"22:15"` at 292 px
//!    the difference is about two pixels, and every centred string in
//!    `widgets.hpp` is placed from that width.
//! 2. **`MeasureTextEx(...).y` is always the font size**, not the line height.
//!    It starts at `font.baseSize` and only grows at a newline, then scales by
//!    `fontSize / baseSize`. Every `.y` in `widgets.hpp` — `time_sz.y`,
//!    `temp_sz.y` — is therefore exactly the size that was asked for, and
//!    `kDescentFrac`'s 20% trim is taken off *that*.
//! 3. **`DrawTextEx` positions by the LINE BOX TOP, not the baseline.** The
//!    baseline is `position.y + (int)(ascent * scaleAtBase) * fontSize/baseSize`
//!    — the same truncated integer, from `LoadFontData`'s
//!    `chars[i].offsetY += (int)(ascent*scaleFactor)`. `rivet_sdk::text::Face`
//!    draws from a baseline, so the conversion has to happen here and cannot be
//!    guessed at.
//!
//! # What CANNOT be reproduced, and is reported rather than approximated
//!
//! * **Glyph edges.** Two different rasterisers antialias differently. Nothing
//!   in this file changes that and nothing can.
//! * **Downscaling from the atlas.** raylib rasterises once at `baseSize` and
//!   draws every other size as a bilinear resample of that bitmap. The weather
//!   panel's 96 px temperature is a resampled 292 px glyph; here it is
//!   rasterised at 96 px, which is sharper. The clock (292 from 292), the date
//!   (60 from 60) and Digital Rain (18 from 18) are all drawn at their atlas
//!   size, so they are unaffected — the panel is the only place it bites.
//! * **Subpixel placement.** raylib draws at float coordinates through the GPU;
//!   `Face::draw` takes integers. A centred string can therefore sit up to half
//!   a pixel from where raylib put it.

use rivet_sdk::text::{Face, Target};

#[allow(dead_code)] // used by the weather panel and the reading card — SPEC-04 §5.4
/// Where a glyph's ink sits inside its line box, in pixels at a given size.
///
/// `GlyphInkBounds` from `cpp/src/widgets.hpp`, and it exists there for the
/// reason written beside it: a glyph box is mostly air, so stacking two of them
/// by their boxes leaves a gap far larger than the number anybody wrote.
///
/// `top` and `bottom` are measured **down from the line box top** — the `y`
/// that would be handed to [`Typeface::draw`] — because that is the frame the
/// C++ works in and re-basing it would make every offset in the port need
/// re-deriving.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GlyphInk {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[allow(dead_code)] // as above
impl GlyphInk {
    #[must_use]
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
    #[must_use]
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
}

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
    /// Load a face and declare the atlas size the C++ built it at.
    ///
    /// # Errors
    ///
    /// The sentence [`Face::load`] produced, unchanged: it already names the
    /// path and the reason, which is what a journal wants.
    pub fn load(path: &str, base_size: f32) -> Result<Self, String> {
        let face = Face::load(path)?;
        let units_per_em = face.units_per_em();
        let (ascent, descent, _) = face
            .vertical_metrics(units_per_em)
            .ok_or_else(|| format!("{path}: no horizontal line metrics, so nothing can be laid out on it"))?;
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

    /// `MeasureTextEx(font, text, px, 0)`.
    ///
    /// The `y` is `px` and not a line height; see the module note. Returned as a
    /// pair rather than a `Vector2` because the caller uses the two halves for
    /// unrelated things and naming them here is clearer than `.0`.
    #[must_use]
    pub fn measure(&self, text: &str, px: f32) -> (f32, f32) {
        let scale = px / self.base_size;
        let width: f32 = text.chars().map(|ch| self.advance_at_base(ch)).sum();
        (width * scale, px)
    }

    /// Where the baseline falls for text whose line box top is at `y`.
    #[must_use]
    pub fn baseline(&self, y: f32, px: f32) -> f32 {
        y + self.ascent_offset() * (px / self.base_size)
    }

    #[allow(dead_code)] // as above
    /// Where one glyph's ink sits, relative to the line box top and the pen.
    #[must_use]
    pub fn ink(&self, ch: char, px: f32) -> GlyphInk {
        let ink = self.face.ink(ch, self.raster_px(px));
        // `Ink::bottom` is the distance from the baseline UP to the bottom ink
        // row, so the top of the ink is `height + bottom` above the baseline.
        let top = self.ascent_offset() * (px / self.base_size)
            - (ink.height as f32 + ink.bottom as f32);
        GlyphInk {
            top,
            bottom: top + ink.height as f32,
            left: ink.left as f32,
            right: ink.left as f32 + ink.width as f32,
        }
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

    const BOLD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SourceCodePro-Bold.ttf");
    const REGULAR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SourceCodePro-Regular.ttf");

    /// **The guard on every other test here.** Each one below has to skip when
    /// the file is missing, and `rivet_sdk::text`'s own comment records what
    /// that costs: a font test that skips itself passes while proving nothing.
    /// These files are tracked, so their absence is a defect.
    #[test]
    fn the_shipped_typefaces_are_where_this_crate_says_they_are() {
        // `AuraRain.ttf` was the third entry here until 2026-09-20. It is
        // `aura-background-pack`'s now, along with Digital Rain and the IPA
        // licence that has to travel with it — `assets/README.md`.
        for path in [BOLD, REGULAR] {
            assert!(
                std::path::Path::new(path).is_file(),
                "{path} is not a file — the package would ship without it"
            );
        }
    }

    /// **The conversion this module exists for**, checked against the numbers
    /// `fontTools` reports for the shipped files.
    ///
    /// Source Code Pro: `1000 / 1257`, so raylib's 292 is 232.30 px per em.
    ///
    /// **The IPAGothic half of this test went to the pack** with the face it
    /// was about — `2048 / 2048`, so raylib's 18 is 18 and Digital Rain needed
    /// no conversion at all. That the identity case also holds is worth
    /// asserting where the font is, and asserting it here would mean this
    /// crate carrying a 19 KB font for one test.
    #[test]
    fn a_raylib_size_converts_to_the_fontdue_size_that_draws_the_same_glyph() {
        let bold = Typeface::load(BOLD, 292.0).expect("the bold face");
        let converted = bold.raster_px(292.0);
        assert!(
            (converted - 292.0 * 1000.0 / 1257.0).abs() < 0.01,
            "292px Source Code Pro should rasterise at 232.3, not {converted}"
        );
    }

    /// `MeasureTextEx(...).y` is the size asked for. Every `.y` in
    /// `widgets.hpp` depends on it, and `kDescentFrac` trims 20% off it.
    #[test]
    fn measuring_returns_the_font_size_as_its_height() {
        let bold = Typeface::load(BOLD, 292.0).expect("the bold face");
        for px in [292.0_f32, 144.0, 96.0, 62.0] {
            assert!((bold.measure("22:15", px).1 - px).abs() < 0.001);
        }
    }

    /// The advance is an integer fixed at the atlas size, then scaled.
    ///
    /// Source Code Pro is monospaced at 600/1000 em, so at 292 px every glyph
    /// steps `(int)(600 * 292/1257)` = 139 px and `"22:15"` is 695 px wide —
    /// **not** the 696.9 a float advance would sum to. Aura centres its date
    /// against that width, so the two pixels are a real placement difference.
    #[test]
    fn advances_are_truncated_at_the_atlas_size_the_way_raylib_stores_them() {
        let bold = Typeface::load(BOLD, 292.0).expect("the bold face");
        let (width, _) = bold.measure("22:15", 292.0);
        assert!(
            (width - 695.0).abs() < 0.001,
            "five monospaced glyphs at 292px should be 695px, not {width}"
        );
        // At half the size the same integers scale, rather than being
        // re-truncated: 695 * (146/292).
        let (half, _) = bold.measure("22:15", 146.0);
        assert!((half - 347.5).abs() < 0.001, "{half}");
    }

    /// A glyph's ink is inside its line box, and a digit's ink is nowhere near
    /// the full height — which is the whole reason `kDescentFrac` and
    /// `GlyphInkBounds` exist in the C++.
    #[test]
    fn a_digits_ink_sits_inside_its_line_box_and_is_shorter_than_it() {
        let bold = Typeface::load(BOLD, 292.0).expect("the bold face");
        let ink = bold.ink('0', 292.0);
        assert!(ink.top > 0.0, "ink starts below the line box top");
        assert!(ink.height() > 0.0, "a zero rasterised to nothing");
        assert!(
            ink.height() < 292.0 * 0.8,
            "a digit fills {} of a 292px box, which would mean the box is wrong",
            ink.height()
        );
    }
}
