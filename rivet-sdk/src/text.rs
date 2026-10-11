//! **Real type, rasterised at runtime** — `PLAN.md` §104.38.
//!
//! The maintainer judged what was here before unacceptable (`.rulings/` R-336).
//! It was a hand-rolled bitmap with a partial alphabet — earlier the same
//! evening a screen had to be written in lowercase because the uppercase
//! letters did not exist — and it could not scale, which the
//! splash-to-launcher transition needs.
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
pub const SYSTEM_FONT: &str = "/system/share/fonts/Geist.ttf";

/// The system monospace, on the read-only slot — added 2026-09-04.
///
/// **Geist Mono**, the same family as [`SYSTEM_FONT`], under the same licence
/// and covered by the same `OFL.txt`.
///
/// Reach for it wherever a **number changes in place** — a clock, a
/// temperature, a byte count. Geist's digits are proportional, so a value
/// redrawn every second shifts sideways under itself; a monospaced digit does
/// not. `rivet_core::etc::SYSTEM_MONO` carries the longer argument, including
/// why Aura keeps Source Code Pro rather than adopting this.
pub const SYSTEM_MONO: &str = "/system/share/fonts/GeistMono.ttf";

/// The icon face — a **subset** of Material Icons, rounded style.
///
/// `assets/fonts/README.md` carries what is in it and why it is a subset: the
/// full family is 1.6 MB for five glyphs. Icons as GLYPHS rather than images is
/// what lets a launcher draw one at any size with the rasteriser it already
/// has, and needs no image decoder anywhere on the machine.
pub const ICON_FONT: &str = "/system/share/fonts/MaterialIcons.ttf";

/// A loaded typeface, ready to draw with.
pub struct Face {
    font: fontdue::Font,
    /// **Rasterised glyphs, kept** — `SPEC-04` §10.3, added 2026-09-01.
    ///
    /// `draw` called `font.rasterize(ch, px)` inside its own loop, so **every
    /// glyph was rebuilt from outlines on every frame**. Measured on an x86-64
    /// host: rasterisation alone is 58% of the cost of a full Digital Rain
    /// frame — 1.36 ms of 2.36 ms for 6360 cells — and 28% of a five-digit
    /// clock at 292px. A dashboard redrawing the same twelve characters thirty
    /// times a second was paying it thirty times a second, and so was every
    /// other kiosk application on this platform.
    ///
    /// Keyed on the character **and the size**, because the same letter at two
    /// sizes is two different bitmaps. The size is keyed by its bit pattern
    /// rather than its value: `f32` is not `Eq`, and two sizes that differ in
    /// the last bit are genuinely two entries rather than a rounding question.
    ///
    /// `RefCell` because `draw` takes `&self` and always has — changing that
    /// would change every call site for a cache nobody asked for. Nothing here
    /// is `Sync`, which it was not before either.
    glyphs: core::cell::RefCell<std::collections::HashMap<(char, u32), std::rc::Rc<Glyph>>>,
}

/// One rasterised glyph.
struct Glyph {
    metrics: fontdue::Metrics,
    coverage: Vec<u8>,
}

/// **The cache is bounded, and the bound is a whole clear rather than an
/// eviction.**
///
/// An application animating a size — a number growing as it changes, a
/// transition — would otherwise insert one entry per frame for ever, and an 8
/// MB canvas is not the only thing on a board with a gigabyte. A thousand
/// entries covers every size a card draws at several times over.
///
/// Clearing rather than evicting because the alternative is a recency list to
/// maintain per glyph, and the cost of being wrong is one frame of
/// rasterisation — the thing this used to do on every frame.
const GLYPH_CACHE_LIMIT: usize = 1024;

/// **Where a glyph's ink actually is**, in pixels, relative to the pen and the
/// baseline.
///
/// A glyph box is mostly air. An arrow occupies the middle of its em and a
/// digit leaves room above for ascenders it does not have, so two glyphs
/// stacked by their line boxes sit much further apart than the gap anybody
/// wrote down. Aura's C++ renderer learned this the hard way and has a
/// `GlyphInkBounds` of its own for it (`cpp/src/widgets.hpp`); this is the same
/// measurement, taken from the rasteriser rather than from an atlas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ink {
    /// Distance right of the pen to the first ink column. Negative for a glyph
    /// that leans left of its origin.
    pub left: i32,
    /// Distance **above** the baseline to the bottom ink row. Negative for a
    /// descender.
    pub bottom: i32,
    pub width: usize,
    pub height: usize,
}

/// What the fourth byte of each pixel means.
///
/// # Why this is a type and not a `bool`
///
/// Everything in this crate draws thirty-two bits a pixel with blue at the
/// lowest address, and for most of this project's life the fourth byte was
/// padding that nothing was allowed to touch. `draw::clear` wrote `255` into it
/// on 2026-09-20 and broke twenty-four baseline hashes and a test named
/// `a_frame_reaches_the_surface_and_leaves_padding_alone` — which is the
/// property that byte carries, stated as a test.
///
/// Then the widgets became a `wl_shm` subsurface over a GPU-rendered
/// background (`GPU-Backgrounds` §3), and *that* buffer's fourth byte is alpha:
/// a layer drawn with no alpha is a layer that is either a black rectangle or
/// invisible, depending which way the compositor reads it. So both meanings are
/// now live, in the same process, through the same primitives.
///
/// A `bool` beside `bytes_per_pixel` would let a caller write
/// `bytes_per_pixel: 3, alpha: true` — a pixel with no fourth byte whose
/// fourth byte is alpha. The stride arithmetic is **derived** from the format
/// here instead, so that state cannot be spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Blue, green, red, **padding**. The fourth byte is never written.
    ///
    /// What a scanout buffer is: `crate::drm`'s dumb buffer, the compositor's
    /// opaque background surface, and every `T6` byte-identity baseline.
    #[default]
    Xrgb8888,
    /// Blue, green, red, **alpha**, and the colour is *premultiplied*.
    ///
    /// What `wl_shm` means by `ARGB8888` and therefore what an overlay
    /// subsurface must contain. Premultiplication is not an extra step here: it
    /// is what blending a colour into a zero-cleared buffer already produces,
    /// because `fg * a + 0 * (1 - a)` **is** `fg * a`. Start from
    /// [`erase`](crate::draw::erase) and the invariant holds by induction.
    Argb8888,
}

impl Format {
    /// Bytes per pixel. Four, for both — but read it from here rather than
    /// writing `4`, so that the day a sixteen-bit format arrives there is one
    /// place to put it.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Xrgb8888 | Self::Argb8888 => 4,
        }
    }

    /// Whether a primitive should write the fourth byte.
    #[must_use]
    pub const fn writes_alpha(self) -> bool {
        matches!(self, Self::Argb8888)
    }
}

/// Where a canvas's pixels are and how they are laid out.
///
/// The same shape the old bitmap renderer took, so the call sites did not have
/// to change when the renderer under them did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    /// What the fourth byte means. See [`Format`] — this is load-bearing, not
    /// decorative.
    pub format: Format,
}

impl Target {
    /// An opaque scanout target: the fourth byte is padding and stays as it was.
    #[must_use]
    pub const fn xrgb(width: usize, height: usize, stride: usize) -> Self {
        Self { width, height, stride, format: Format::Xrgb8888 }
    }

    /// A translucent overlay target: the fourth byte is premultiplied alpha and
    /// every primitive maintains it.
    #[must_use]
    pub const fn argb(width: usize, height: usize, stride: usize) -> Self {
        Self { width, height, stride, format: Format::Argb8888 }
    }

    /// Bytes per pixel, from the format.
    #[must_use]
    pub const fn bytes_per_pixel(&self) -> usize {
        self.format.bytes_per_pixel()
    }
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
        Ok(Self {
            font,
            glyphs: core::cell::RefCell::new(std::collections::HashMap::new()),
        })
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

    /// How wide one character is at `px` — the pen's own step.
    ///
    /// [`Self::width_of`] for one character, exposed because a caller that lays
    /// glyphs out itself (a grid, a column of cells) needs the step without
    /// building a `String` per glyph.
    #[must_use]
    pub fn advance_of(&self, ch: char, px: f32) -> f32 {
        self.font.metrics(ch, px).advance_width
    }

    /// The em square this typeface was designed on, in font units.
    ///
    /// **Needed by anybody reproducing another rasteriser's sizing.** `px` here
    /// is a fraction of the em (`fontdue`'s scale is `px / units_per_em`), and
    /// `stb_truetype` — which is what raylib, SDL_ttf and a good deal of the C
    /// world rasterise through — scales by `px / (ascent - descent)` instead. A
    /// caller porting a layout from one of those has to convert, and cannot
    /// without this number.
    #[must_use]
    pub fn units_per_em(&self) -> f32 {
        self.font.units_per_em()
    }

    /// Ascent, descent and line gap at `px`. Descent is **negative**.
    ///
    /// `None` for a face with no horizontal line metrics, which is a face
    /// nothing here can lay out.
    #[must_use]
    pub fn vertical_metrics(&self, px: f32) -> Option<(f32, f32, f32)> {
        self.font
            .horizontal_line_metrics(px)
            .map(|m| (m.ascent, m.descent, m.line_gap))
    }

    /// Where one glyph's ink sits, at `px`.
    ///
    /// See [`Ink`]. A glyph with no ink at all — a space — is zero-sized, and
    /// the offsets are then meaningless rather than wrong.
    #[must_use]
    pub fn ink(&self, ch: char, px: f32) -> Ink {
        let m = self.glyph(ch, px).metrics;
        Ink {
            left: m.xmin,
            bottom: m.ymin,
            width: m.width,
            height: m.height,
        }
    }

    /// The distance from one baseline to the next at `px`.
    #[must_use]
    pub fn line_height(&self, px: f32) -> usize {
        self.font
            .horizontal_line_metrics(px)
            .map_or((px * 1.4) as usize, |m| {
                (m.ascent - m.descent + m.line_gap).ceil().max(1.0) as usize
            })
    }

    /// One glyph, rasterised once.
    ///
    /// `Rc` rather than a reference, so the `RefCell` borrow is not held across
    /// the drawing loop — the drawing loop takes `&mut canvas`, and a borrow
    /// living that long would be a runtime panic waiting for the first
    /// re-entrant call. The clone is a refcount bump.
    fn glyph(&self, ch: char, px: f32) -> std::rc::Rc<Glyph> {
        let key = (ch, px.to_bits());
        if let Some(found) = self.glyphs.borrow().get(&key) {
            return std::rc::Rc::clone(found);
        }
        let (metrics, coverage) = self.font.rasterize(ch, px);
        let glyph = std::rc::Rc::new(Glyph { metrics, coverage });
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() >= GLYPH_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, std::rc::Rc::clone(&glyph));
        glyph
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
        self.draw_alpha(canvas, t, x, y, text, px, colour, 255);
    }

    /// [`Self::draw`], with the whole string at one opacity.
    ///
    /// **Coverage times `alpha`**, which is what a tinted glyph means in every
    /// renderer that has one: the rasteriser says how much of the pixel the
    /// letter covers, and this says how strongly the letter is present at all.
    /// A caller fading text in, or drawing a trail whose tail is dimmer than its
    /// head, needs the two multiplied rather than the colour scaled — scaling
    /// the colour fades toward black instead of toward whatever is behind it,
    /// and over a picture that is visibly wrong.
    ///
    /// `alpha` of 255 is [`Self::draw`] exactly, and `0` draws nothing.
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
            // **From the cache, or rasterised once and kept** — see the note on
            // `Face::glyphs`. `rasterize` is deterministic, so what comes back
            // is what it would have produced; `tests::a_cached_glyph_draws_
            // exactly_what_rasterising_it_again_would` asserts that against a
            // face that has never seen the character.
            let glyph = self.glyph(ch, px);
            let Glyph { metrics, coverage } = &*glyph;
            // `xmin`/`ymin` are the glyph's own offsets from the pen and the
            // baseline. Adding them here is what makes a `y` sit below the line
            // and a `j` hang, rather than every glyph being flush to a box.
            let gx = pen + metrics.xmin as f32;
            let gy = y as f32 - (metrics.height as f32 + metrics.ymin as f32);
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let covered = coverage[row * metrics.width + col];
                    if covered == 0 {
                        continue;
                    }
                    // `cov * a / 255`, rounded. The rounding matters at the
                    // dim end of a fade: without it a trail's last few steps
                    // all floor to the same value and the fade visibly stops
                    // before it reaches nothing.
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

/// Blend one coverage value into the canvas.
///
/// `XRGB8888`, little-endian, so blue is the lowest byte — the one layout this
/// program draws into (`crate::drm`). Kept here rather than reaching for
/// `crate::pixel` because the old renderer's channel-offset indirection existed
/// to support the framebuffer's driver-declared bitfields, and `§104.35.2`
/// deleted that path.
fn blend(canvas: &mut [u8], t: &Target, x: usize, y: usize, colour: [u8; 3], alpha: u8) {
    let at = y * t.stride + x * t.bytes_per_pixel();
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
    // **The glyph's own alpha is 255 — its coverage is `a`.** So the fourth
    // byte takes the identical `mix` with an opaque source, which expands to
    // `a + dst * (1 - a)`: the `over` operator, and the companion of the three
    // lines above rather than a different idea. On `Xrgb8888` it does not run
    // and the padding byte keeps whatever the buffer handed us.
    if t.format.writes_alpha() {
        canvas[at + 3] = mix(255, canvas[at + 3]);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The fonts in this repository, addressed from the crate rather than
    /// from a working directory.**
    ///
    /// Corrected 2026-09-01. These were `"../../assets/fonts/Geist.ttf"`, which
    /// resolves from `cargo test`'s working directory — the *package* root —
    /// and therefore pointed one level above the repository at a path that has
    /// never existed. `Face::load` returned `Err`, the test's *"a tree without
    /// them is a tree this test has nothing to say about"* branch took over,
    /// and **`the_shipped_faces_rasterise_a_glyph_each` passed by doing
    /// nothing.**
    ///
    /// It went unnoticed because it could not be run at all: `draw` linked
    /// `libdrm` until this same change split `screen` out, so `cargo test
    /// --workspace` never compiled this module. Two defects hiding each other,
    /// which is the argument for the split rather than a footnote to it.
    ///
    /// `CARGO_MANIFEST_DIR` is fixed at compile time and does not depend on
    /// where anybody was standing.
    const GEIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/fonts/Geist.ttf");
    const GEIST_MONO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/fonts/GeistMono.ttf");
    const ICONS: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/fonts/MaterialIcons.ttf"
    );

    /// **The guard on the guard.** Every font test below has a *"the file is
    /// not here, so there is nothing to say"* branch, and that branch is how
    /// the wrong path went unnoticed for as long as it did. This one has no
    /// such branch: the files are tracked in this repository, so their absence
    /// is a defect and not a circumstance.
    #[test]
    fn the_font_fixtures_are_where_the_tests_look_for_them() {
        for path in [GEIST, GEIST_MONO, ICONS] {
            assert!(
                std::path::Path::new(path).is_file(),
                "{path} is not a file — every font test below would silently \
                 pass by skipping itself"
            );
        }
    }

    /// **Every shipped face actually rasterises.** Geist is TrueType outlines
    /// and the icon font is a CFF subset -- two different things inside a font
    /// file, and `fontdue` handling one does not prove it handles the other.
    ///
    /// Read from the repository rather than from an image, so `cargo test`
    /// catches a bad subset before a card is built and shipped.
    ///
    /// **Geist Mono joined on 2026-09-04**, the day it was added to the image,
    /// and it is the case this test exists for: those bytes were fetched from
    /// upstream and had never been through a rasteriser here. A font file that
    /// downloads cleanly and then produces no ink is a card that boots to a
    /// screen with a blank status block on it.
    #[test]
    fn the_shipped_faces_rasterise_a_glyph_each() {
        for (path, ch, what) in [
            (GEIST, 'R', "the system typeface"),
            (GEIST_MONO, '0', "the system monospace"),
            (
                ICONS,
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

    /// **The monospace is monospaced, and the sans is not.**
    ///
    /// Added 2026-09-04 with [`SYSTEM_MONO`]. The entire reason a second face is
    /// in the image is that a value redrawn in place must not shift sideways as
    /// its digits change, and **nothing else in this repository would notice if
    /// a proportional font were shipped at that path.** Every screen would draw,
    /// every test would pass, and the clock would jitter exactly as it does
    /// today — a defect visible only to somebody standing in front of a panel.
    ///
    /// Both halves are asserted. The sans failing to be proportional would mean
    /// the two constants had been crossed over, which is the other way to get
    /// this wrong and produces no error anywhere else either.
    #[test]
    fn the_system_monospace_is_monospaced_and_the_system_sans_is_not() {
        // A deliberately hostile set: the narrowest and widest characters a
        // status line actually contains. Two digits alone would pass on a font
        // with tabular figures and proportional letters.
        const SAMPLE: [char; 6] = ['0', '1', 'W', 'i', '.', 'm'];
        const PX: f32 = 32.0;

        let Ok(mono) = Face::load(GEIST_MONO) else {
            panic!("{GEIST_MONO} did not load — the fixture guard should have caught this")
        };
        let widths: Vec<f32> = SAMPLE.iter().map(|c| mono.advance_of(*c, PX)).collect();
        let first = widths[0];
        for (ch, w) in SAMPLE.iter().zip(&widths) {
            assert!(
                (w - first).abs() < 0.01,
                "the system monospace is not monospaced: {ch:?} advances {w} where '0' \
                 advances {first} at {PX}px. A number redrawn in place will shift."
            );
        }

        let Ok(sans) = Face::load(GEIST) else {
            panic!("{GEIST} did not load — the fixture guard should have caught this")
        };
        let narrow = sans.advance_of('i', PX);
        let wide = sans.advance_of('W', PX);
        assert!(
            wide > narrow + 1.0,
            "the system SANS measures 'i' and 'W' the same ({narrow} vs {wide}) — the two \
             font constants are probably crossed over"
        );
    }

    /// **The guard on the glyph cache** — `SPEC-04` §10.3.
    ///
    /// A cache that returned a *nearly* right bitmap — the wrong size, a stale
    /// entry after a key collision, an off-by-one in the metrics — would change
    /// type by a fraction of a pixel, which nobody sees until two photographs
    /// of a television are put side by side. So the comparison is byte for
    /// byte, between a face that has drawn the string before and one that has
    /// never seen it.
    ///
    /// The second half is the case a naive key gets wrong: the **same
    /// character at two sizes** is two glyphs, and a cache keyed on the
    /// character alone would draw the second at the first one's size.
    #[test]
    fn a_cached_glyph_draws_exactly_what_rasterising_it_again_would() {
        let Ok(warm) = Face::load(GEIST) else {
            // As the test above: a tree without the font is a tree this has
            // nothing to say about.
            return;
        };
        let t = Target {
            width: 300,
            height: 120,
            stride: 300 * 4,
            format: Format::Xrgb8888,
        };
        // Not a blank canvas — text is blended, so drawing over zeroes would
        // hide a bug in what the coverage is blended against.
        let textured: Vec<u8> = (0..t.stride * t.height).map(|i| (i * 31 % 253) as u8).collect();

        for (text, px) in [
            ("22:15", 96.0f32),
            ("22:15", 42.0),
            ("Chance Rain Showers", 24.0),
            ("°F 6.5 kPa", 36.0),
        ] {
            // The warm face has already drawn at least one of these, so by the
            // second iteration its cache is populated and being exercised.
            let mut from_cache = textured.clone();
            warm.draw(&mut from_cache, &t, 10, 90, text, px, [230, 240, 250]);

            let cold = Face::load(GEIST).expect("a second face");
            let mut from_scratch = textured.clone();
            cold.draw(&mut from_scratch, &t, 10, 90, text, px, [230, 240, 250]);

            assert_eq!(from_cache, from_scratch, "{text:?} at {px}px");
            assert!(
                from_cache != textured,
                "{text:?} at {px}px drew nothing at all, so this proved nothing"
            );

            // And again, now that the warm face is certainly warm.
            let mut again = textured.clone();
            warm.draw(&mut again, &t, 10, 90, text, px, [230, 240, 250]);
            assert_eq!(again, from_scratch, "{text:?} at {px}px, second pass");
        }
    }

    /// The bound exists so an application animating a size cannot grow the
    /// cache for ever. Clearing rather than evicting is the documented choice;
    /// this asserts that it happens at all and that drawing still works after.
    #[test]
    fn the_glyph_cache_is_bounded_and_survives_being_cleared() {
        let Ok(face) = Face::load(GEIST) else {
            return;
        };
        let t = Target {
            width: 64,
            height: 64,
            stride: 64 * 4,
            format: Format::Xrgb8888,
        };
        let mut canvas = vec![0u8; t.stride * t.height];
        // One entry per size, past the limit.
        for i in 0..=GLYPH_CACHE_LIMIT {
            face.draw(&mut canvas, &t, 0, 40, "8", 8.0 + i as f32 * 0.01, [255, 255, 255]);
        }
        assert!(
            face.glyphs.borrow().len() <= GLYPH_CACHE_LIMIT,
            "the cache grew past its bound"
        );
        // Still draws, and still draws the right thing.
        let cold = Face::load(GEIST).expect("a second face");
        let mut mine = vec![0u8; t.stride * t.height];
        let mut reference = vec![0u8; t.stride * t.height];
        face.draw(&mut mine, &t, 0, 40, "8", 32.0, [255, 255, 255]);
        cold.draw(&mut reference, &t, 0, 40, "8", 32.0, [255, 255, 255]);
        assert_eq!(mine, reference);
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
            format: Format::Xrgb8888,
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
            format: Format::Xrgb8888,
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
            format: Format::Xrgb8888,
        };
        let mut canvas = vec![0u8; 4];
        blend(&mut canvas, &t, 99, 99, [255, 255, 255], 255);
        assert_eq!(canvas, vec![0u8; 4], "an out-of-range blend wrote something");
    }

    /// **The fourth byte, both ways round, in one test.**
    ///
    /// `Xrgb8888` must leave it exactly as found — that is the property
    /// `render::a_frame_reaches_the_surface_and_leaves_padding_alone` and 24
    /// `T6` baseline hashes encode, and it has been broken once already.
    /// `Argb8888` must accumulate coverage into it, or an overlay subsurface
    /// is invisible.
    #[test]
    fn the_fourth_byte_is_padding_or_alpha_according_to_the_format() {
        let opaque = Target::xrgb(1, 1, 4);
        let mut canvas = vec![0x77u8; 4];
        blend(&mut canvas, &opaque, 0, 0, [10, 20, 30], 128);
        assert_eq!(canvas[3], 0x77, "XRGB blended into the padding byte");

        let overlay = Target::argb(1, 1, 4);
        let mut canvas = vec![0u8; 4];
        blend(&mut canvas, &overlay, 0, 0, [255, 255, 255], 128);
        assert_eq!(canvas[3], 128, "ARGB did not accumulate coverage as alpha");
        // Premultiplied, and nobody multiplied: 255 * 128/255 == 128. A
        // straight-alpha buffer would hold 255 here.
        assert_eq!(canvas[0], 128, "the colour came out un-premultiplied");

        // `over` is idempotent towards opaque, not additive past it.
        blend(&mut canvas, &overlay, 0, 0, [255, 255, 255], 128);
        assert_eq!(canvas[3], 192, "a second blend must be a + dst*(1-a), not a sum");
    }
}
