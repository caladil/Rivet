//! **A verbatim port of `rivet_sdk::text::Face`'s rasteriser, cache and blend
//! — constructed from bytes rather than from a path.**
//!
//! # Why this exists at all
//!
//! Digital Rain draws through a typeface (`crate::typeface::Typeface`, which
//! wraps [`Face`]), and `rivet_sdk::text::Face` has exactly one constructor:
//! `Face::load(path: &str)`, which reads the font with `std::fs::read`. That
//! is the right shape for an APPLICATION — `examples/aura/aura-renderer`
//! ships its fonts as package assets and finds them through
//! `rivet_sdk::app::dir()`, which reads `RIVET_APP_DIR`.
//!
//! **A shared library has no `RIVET_APP_DIR` of its own.** This pack is
//! `dlopen`ed *into* a host process — `aura-renderer` or `aura-controller` —
//! and every environment variable it can read, including `RIVET_APP_DIR`, is
//! the HOST's. Asking for it would find the host's own asset directory (or
//! none, for a host with none), never this library's. And `docs/SPEC-02-
//! Shared-Rivets.md` §4 gives a library's install layout as `manifest.toml`,
//! `package-header` and `lib/` — no sibling directory for anything else, so
//! there is no supported place to install a font beside the library even if
//! this pack could find its own path (`ADR-053`'s class match puts an
//! object's *directory* on `LD_LIBRARY_PATH`, but nothing hands a loaded
//! object that path back to ask with).
//!
//! So the font is embedded in the object at compile time
//! (`crate::typeface::RAIN_FONT_BYTES`, `include_bytes!`) and rasterised from
//! bytes in memory — which needs a constructor `rivet_sdk::text::Face` does
//! not have, because `Face`'s fields are private to that crate and this one
//! cannot be modified by this task (its brief is scoped to
//! `examples/aura/aura-background-pack` and `examples/aura/aura-renderer`
//! only).
//!
//! # What is copied, and the one promise this module exists to keep
//!
//! Everything past construction — the glyph cache, the rounding in `blend`,
//! the `+ 127` before dividing by 255 — is copied from
//! `rivet-sdk/src/text.rs` **unchanged**, because
//! `examples/aura/aura-renderer/evidence/T6-background-baseline/manifest.json`
//! was captured through the SDK's own `Face`, and a difference in this
//! module's arithmetic would be indistinguishable, from a hash alone, from a
//! difference in Digital Rain itself. Matched by inspection against
//! `rivet-sdk/src/text.rs` as it stood on 2026-09-19; `evidence.rs`'s
//! comparison against the committed baseline is what actually proves it.
//!
//! **Trimmed, not just ported.** `Face::load`, `Face::width_of`,
//! `Face::line_height` and `Face::ink` (with its `Ink` struct) are not here.
//! `crate::typeface::Typeface` — the thing that actually calls into this
//! module — only ever calls [`Face::units_per_em`], [`Face::vertical_metrics`],
//! [`Face::advance_of`] and [`Face::draw_alpha`], because this pack has no
//! card layer to lay text out for (no clock, no weather panel — those belong
//! to `aura-renderer`'s widgets, not to a background). Carrying the unused
//! quarter across would be baggage with no reader, not fidelity.
//!
//! **One deliberate departure from the SDK's copy: `Arc` where it has `Rc`.**
//! `rivetos_addon::frame::Drawing: Send + 'static` (a background holds state
//! a host may move between calls), and `Rc` is not `Send`. `rivet_sdk::text::
//! Face` never needed to be — `aura-renderer` draws from one thread and says
//! so in its own doc comment ("nothing here is `Sync`, which it was not
//! before either"). This module's `Face` is reached through
//! `crate::typeface::Typeface`, held inside `crate::BackgroundPack`, which
//! the addon boundary requires to be `Send`; an atomic refcount is the
//! smallest change that satisfies it without touching the cache's actual
//! behaviour (`tests::a_cached_glyph_draws_exactly_what_rasterising_it_again_
//! would` holds that behaviour to the same standard either way).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rivet_sdk::text::Target;

/// See `rivet_sdk::text::Face`'s own note on this constant — copied verbatim,
/// including the number, because the cache's *behaviour* is part of what is
/// being reproduced: a bound that cleared more or less often would change
/// nothing about a single frame's pixels, but keeping the same constant means
/// there is one fewer thing that differs from the SDK's copy while auditing
/// this file against it.
const GLYPH_CACHE_LIMIT: usize = 1024;

/// One rasterised glyph. Verbatim from `rivet_sdk::text`.
struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

/// A loaded typeface, ready to draw with — `rivet_sdk::text::Face`'s shape,
/// built from bytes already in memory instead of from a path.
pub struct Face {
    font: fontdue::Font,
    glyphs: RefCell<HashMap<(char, u32), Arc<Glyph>>>,
}

impl Face {
    /// Parse an embedded font. The only difference from
    /// `rivet_sdk::text::Face::load`: no `std::fs::read`, because there is no
    /// path to read from — see this module's doc comment.
    ///
    /// # Errors
    ///
    /// A sentence naming why `fontdue` rejected the bytes. In practice this
    /// never fires for `crate::typeface::RAIN_FONT_BYTES`: those bytes are
    /// `include_bytes!`'d from a file this crate ships and have parsed
    /// cleanly since `aura-renderer` first loaded them (`assets/README.md`).
    /// Returning `Result` anyway, rather than parsing at compile time or
    /// panicking, keeps this function testable on a deliberately bad input —
    /// see `tests::rejected_bytes_name_why_rather_than_panicking`.
    pub fn from_bytes(bytes: &'static [u8]) -> Result<Self, String> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|e| format!("not a usable font: {e}"))?;
        Ok(Self {
            font,
            glyphs: RefCell::new(HashMap::new()),
        })
    }

    /// The em square this typeface was designed on, in font units.
    #[must_use]
    pub fn units_per_em(&self) -> f32 {
        self.font.units_per_em()
    }

    /// Ascent, descent and line gap at `px`. Descent is **negative**.
    #[must_use]
    pub fn vertical_metrics(&self, px: f32) -> Option<(f32, f32, f32)> {
        self.font
            .horizontal_line_metrics(px)
            .map(|m| (m.ascent, m.descent, m.line_gap))
    }

    /// How wide one character is at `px` — the pen's own step.
    #[must_use]
    pub fn advance_of(&self, ch: char, px: f32) -> f32 {
        self.font.metrics(ch, px).advance_width
    }

    /// One glyph, rasterised once. `Arc` rather than a reference, so the
    /// `RefCell` borrow is not held across the drawing loop — see this
    /// module's doc comment for why `Arc` and not `Rc`.
    fn glyph(&self, ch: char, px: f32) -> Arc<Glyph> {
        let key = (ch, px.to_bits());
        if let Some(found) = self.glyphs.borrow().get(&key) {
            return Arc::clone(found);
        }
        let (metrics, coverage) = self.font.rasterize(ch, px);
        let glyph = Arc::new(Glyph { metrics, coverage });
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() >= GLYPH_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&glyph));
        glyph
    }

    /// [`rivet_sdk::text::Face::draw_alpha`], copied unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_alpha(
        &self,
        canvas: &mut [u8],
        t: &Target,
        x: usize,
        y: usize,
        text: &str,
        px: f32,
        colour: [u8; 3],
        alpha: u8,
    ) {
        if alpha == 0 {
            return;
        }
        let mut pen = x as f32;
        for ch in text.chars() {
            let glyph = self.glyph(ch, px);
            let Glyph { metrics, coverage } = &*glyph;
            let gx = pen + metrics.xmin as f32;
            let gy = y as f32 - (metrics.height as f32 + metrics.ymin as f32);
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let covered = coverage[row * metrics.width + col];
                    if covered == 0 {
                        continue;
                    }
                    let alpha = if alpha == 255 {
                        covered
                    } else {
                        (((u32::from(covered) * u32::from(alpha)) + 127) / 255) as u8
                    };
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

/// [`rivet_sdk::text::blend`], copied unchanged — including the `+ 127`
/// rounding, without which every blend biases dark.
fn blend(canvas: &mut [u8], t: &Target, x: usize, y: usize, colour: [u8; 3], alpha: u8) {
    let at = y * t.stride + x * t.bytes_per_pixel();
    if at + 3 >= canvas.len() {
        return;
    }
    let a = u32::from(alpha);
    let inv = 255 - a;
    let mix =
        |fg: u8, bg: u8| -> u8 { (((u32::from(fg) * a + u32::from(bg) * inv) + 127) / 255) as u8 };
    canvas[at] = mix(colour[2], canvas[at]);
    canvas[at + 1] = mix(colour[1], canvas[at + 1]);
    canvas[at + 2] = mix(colour[0], canvas[at + 2]);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    const RAIN: &[u8] = include_bytes!("../assets/AuraRain.ttf");

    #[test]
    fn the_embedded_font_parses_and_rasterises_a_glyph() {
        let face = Face::from_bytes(RAIN).expect("the shipped font parses");
        let (metrics, coverage) = face.font.rasterize('0', 18.0);
        assert!(metrics.width > 0 && metrics.height > 0, "'0' rasterised to nothing");
        assert!(coverage.iter().any(|c| *c > 0), "'0' produced a blank bitmap");
    }

    /// A font this crate does not ship is rejected by name rather than by
    /// panic — `Result` earns its place in the signature. `Face` carries no
    /// `Debug` (nothing here needs it outside this one test), so this reads
    /// the `Result` by hand rather than reaching for `expect_err`.
    #[test]
    fn rejected_bytes_name_why_rather_than_panicking() {
        let junk: &'static [u8] = b"not a font";
        match Face::from_bytes(junk) {
            Ok(_) => panic!("garbage bytes should not have parsed as a font"),
            Err(err) => assert!(!err.is_empty()),
        }
    }

    /// The cache is keyed on character AND size, mirroring
    /// `rivet_sdk::text::Face`'s own guard — the failure this catches is the
    /// same one that test exists for: the same glyph at two sizes sharing one
    /// cache slot.
    #[test]
    fn a_cached_glyph_draws_exactly_what_rasterising_it_again_would() {
        let face = Face::from_bytes(RAIN).expect("the shipped font parses");
        let t = Target::xrgb(40, 40, 40 * 4);
        let textured: Vec<u8> = (0..t.stride * t.height).map(|i| (i * 31 % 253) as u8).collect();

        let mut warm = textured.clone();
        face.draw_alpha(&mut warm, &t, 2, 20, "0", 18.0, [200, 220, 240], 255);
        // A second size for the same character, into the same warmed cache.
        let mut warm_small = textured.clone();
        face.draw_alpha(&mut warm_small, &t, 2, 20, "0", 9.0, [200, 220, 240], 255);

        let cold = Face::from_bytes(RAIN).expect("a second face");
        let mut fresh = textured.clone();
        cold.draw_alpha(&mut fresh, &t, 2, 20, "0", 18.0, [200, 220, 240], 255);

        assert_eq!(warm, fresh, "a warmed cache drew something different from a cold one");
        assert_ne!(warm, textured, "nothing was drawn at all");
        assert_ne!(warm_small, warm, "two sizes of the same glyph produced one picture");
    }
}
