//! **Aura's composition, ported constant for constant** — `cpp/src/widgets.hpp`
//! and the block at `cpp/src/main.cpp` line 413.
//!
//! The maintainer's ruling governs every number in this file: the port is
//! pixel-exact, it has no creative room, and it reproduces the product rather
//! than correcting it (`.rulings/` R-125, R-350). Where the C++ picks a number,
//! that number is here. Where it computes a position in a way that is arguably
//! odd — trimming 20% off a line box because digits do not use their descender,
//! trimming a further 18% off the top of the clock card because Source Code
//! Pro's cap height sits low — the odd computation is here too, with the C++'s
//! own reason beside it.
//!
//! # The shape of Aura's screen, which is not four quadrants
//!
//! Three widgets, each anchored to a **corner** and sized by its own content;
//! not a grid. The clock's corner is chosen at random and moves every two to
//! eight minutes for burn-in, the weather panel takes the corner diagonally
//! opposite, and the corner reading card takes the clock's corner turned one
//! quarter clockwise. The fourth corner is always empty, and that is not an accident —
//! it is what leaves the composition room to breathe.
//!
//! # What could not be reproduced, reported rather than approximated
//!
//! 1. **The corner rasterisation.** `DrawRectangleRounded(rect, 0.10f, 8, bg)`
//!    draws each corner as an **eight-segment polygon with no antialiasing**.
//!    `rivet_sdk::draw::fill_rounded_alpha` computes an exact signed distance
//!    and antialiases the boundary. The radius is identical —
//!    `min(w, h) * 0.05`, which is raylib's `recSize * roundness / 2` — and the
//!    corner is smoother here than on the wall. That is a difference in Aura's
//!    favour and it is still a difference; it was not chosen, and closing it
//!    would mean writing an aliased polygon rasteriser.
//! 2. **The Born This Way card.** `DrawRainbowCard` is a six-stop gradient built
//!    from `DrawRectangleGradientH`/`V` and four `DrawCircle` corners, with a
//!    black edge fade over it. `rivet_sdk::draw` has neither a gradient fill nor
//!    a circle, so it is **not ported** and this file draws no card at all for
//!    that palette. It is unreachable in the shipped configuration — the
//!    background constant in `main.rs` selects Digital Rain / Phosphor Green,
//!    whose card is `kCardGrn`, a plain rounded rectangle — and it is listed in
//!    the report rather than being faked with a flat fill.
//! 3. **Glyph edges and subpixel placement.** `crate::typeface`'s note.

use aura_card::{Glyph, Role};
use rivet_sdk::draw;
use rivet_sdk::text::Target;

use crate::rng::Mt19937;
use crate::typeface::Typeface;

/// `enum class Corner`, in its own order — the order `RandomCorner`'s
/// `uniform_int_distribution(0, 3)` indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft = 0,
    TopRight = 1,
    BottomLeft = 2,
    BottomRight = 3,
}

impl Corner {
    const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    /// `CornerOrigin` — where a block of this size sits in this corner.
    #[must_use]
    pub fn origin(self, w: f32, h: f32, block_w: f32, block_h: f32, pad: f32) -> (f32, f32) {
        match self {
            Self::TopLeft => (pad, pad),
            Self::TopRight => (w - pad - block_w, pad),
            Self::BottomLeft => (pad, h - pad - block_h),
            Self::BottomRight => (w - pad - block_w, h - pad - block_h),
        }
    }

    /// `OppositeCorner` — where the weather panel goes.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::TopLeft => Self::BottomRight,
            Self::TopRight => Self::BottomLeft,
            Self::BottomLeft => Self::TopRight,
            Self::BottomRight => Self::TopLeft,
        }
    }

    #[allow(dead_code)] // as above, for the corner reading card
    /// **The reading card's corner** — the original renderer's corner rule for
    /// it: one quarter-turn clockwise from the clock.
    ///
    /// The C++'s own reason: the clock and the weather panel own one diagonal
    /// between them, so the other diagonal is always free, and turning the
    /// clock's corner by ninety degrees lands on it whichever corner the clock
    /// is in.
    #[must_use]
    pub const fn quarter_turn(self) -> Self {
        match self {
            Self::TopLeft => Self::TopRight,
            Self::TopRight => Self::BottomRight,
            Self::BottomRight => Self::BottomLeft,
            Self::BottomLeft => Self::TopLeft,
        }
    }

    /// `RandomCorner` — a different corner, never the one it is already in.
    #[must_use]
    pub fn moved(self, rng: &mut Mt19937) -> Self {
        let mut next = self;
        while next == self {
            next = Self::ALL[rng.int(0, 3) as usize];
        }
        next
    }

    /// `RandomInterval` — two to eight minutes, inclusive.
    #[must_use]
    pub fn interval(rng: &mut Mt19937) -> std::time::Duration {
        std::time::Duration::from_secs(rng.int(2, 8) as u64 * 60)
    }

    /// The corner a fresh start puts the clock in.
    #[must_use]
    pub fn random(rng: &mut Mt19937) -> Self {
        Self::ALL[rng.int(0, 3) as usize]
    }
}

/// `kDescentFrac` — the share of a line box that digits and capitals never use.
///
/// `MeasureTextEx(...).y` is the font size, which reserves descender room a
/// clock face has no descenders for. Every vertical measurement in the C++
/// trims this off, and the gaps written down beside them are the gaps that
/// result.
pub const DESCENT_FRAC: f32 = 0.20;

/// **The card drawn behind a widget, and the ink on it — now the addon
/// protocol's type.**
///
/// This was a struct here and a ten-way `card_theme(background)` table beside
/// it, until T7 made `aura-background-pack` the authority on what this program
/// draws. Both went with the backgrounds, for one reason: a renderer holding
/// that table is a renderer that has to be edited before the pack may grow an
/// eleventh background, and that is the duplication the pack exists to remove.
/// `rivetos_addon::Theme::cards` is where the ten live now, per theme, which
/// is the granularity the answer has.
///
/// **An alias and not a newtype**, because the fields were already identical —
/// `bg`, `bg_alpha`, `text`, `rainbow`, `alert`, `caution` — and the protocol
/// type was defined from this one. A wrapper would buy a conversion at every
/// call site and nothing else.
///
/// The alpha on `bg` is load-bearing: 155 of 255, so the background is visible
/// through the card and the type on it is still legible. See
/// `rivet_sdk::draw::fill_rounded_alpha`.
pub type CardTheme = rivetos_addon::CardAppearance;

/// The dark themes' signal colours — `CardTheme`'s in-class defaults in
/// `cpp/src/main.cpp`.
///
/// **These two stayed when the rest of the table left**, and the reason is
/// `role_ink`'s: with no card at all there is no palette to ask, and an alarm
/// drawn in the same grey as everything else would delete the distinction the
/// role carries. A pack's themes each name their own pair; these are what a
/// theme that asked for no card falls back to.
const ALERT_DARK: [u8; 3] = [255, 69, 58];
const CAUTION_DARK: [u8; 3] = [255, 197, 0];

/// **Three cards for tests, and only for tests.**
///
/// The widget code branches on three properties of a card — a visible fill, a
/// fully transparent one, and the rainbow — and every test that used to reach
/// for `card_theme(some_background)` wanted one of those three rather than
/// that particular background. Now that the real ten live in
/// `aura-background-pack`, naming the shapes here is honest about what the
/// tests were ever checking, and it keeps this crate's tests from depending on
/// a catalogue in another crate that a board's operator can replace.
///
/// **Not `pub`, and not compiled into the binary.** These must never become a
/// back door to the table that just left: if a shipped code path ever needs a
/// card, it comes from the pack.
#[cfg(test)]
pub(crate) mod sample {
    use super::{ALERT_DARK, CAUTION_DARK, CardTheme};

    /// `kCardGrn` — a card with a visible fill, which is what takes the 96 px
    /// padding.
    pub(crate) const GREEN: CardTheme = CardTheme {
        bg: [0, 8, 2],
        bg_alpha: 155,
        text: [180, 255, 180],
        rainbow: false,
        alert: ALERT_DARK,
        caution: CAUTION_DARK,
    };

    /// `kBoxesIce` — a card that exists and cannot be seen, which is the case
    /// `only_a_card_that_can_be_seen_pushes_the_widgets_inward` is about.
    pub(crate) const TRANSPARENT: CardTheme = CardTheme {
        bg: [0, 0, 0],
        bg_alpha: 0,
        text: [0, 0, 0],
        rainbow: false,
        alert: [186, 26, 26],
        caution: [150, 95, 0],
    };

    /// `kCardPride` — no fill, and visible anyway.
    pub(crate) const RAINBOW: CardTheme = CardTheme {
        bg: [0, 0, 0],
        bg_alpha: 0,
        text: [0, 0, 0],
        rainbow: true,
        alert: ALERT_DARK,
        caution: CAUTION_DARK,
    };
}

/// `const float widget_pad = has_visible_card ? 96.f : 48.f;`
///
/// `has_visible_card` is `card && (card->bg.a > 0 || card->rainbow)`, so a theme
/// whose card is fully transparent gets the SMALL padding even though it has a
/// theme — which is why `kBoxesIce` and `kSolidNavy` sit as close to the edge as
/// the plain black default does.
#[must_use]
pub fn widget_pad(card: Option<&CardTheme>) -> f32 {
    match card {
        Some(theme) if theme.bg_alpha > 0 || theme.rainbow => 96.0,
        _ => 48.0,
    }
}

/// The overlay brightness, as a grey — `Color widget_color`.
///
/// Aura's default `Brightness` is **60** (`internal/config/config.go`'s
/// `defaults()`), so the grey is `(unsigned char)(0.60 * 255)` = 153 and the
/// factor every themed colour is multiplied by is `153 / 255`.
#[must_use]
pub fn widget_grey(brightness_percent: u8) -> [u8; 3] {
    let level = (f32::from(brightness_percent) / 100.0 * 255.0) as u8;
    [level, level, level]
}

/// The ink a widget draws with: the theme's text colour scaled by the
/// brightness grey, or the grey itself when there is no card.
#[must_use]
pub fn text_colour(card: Option<&CardTheme>, grey: [u8; 3]) -> [u8; 3] {
    match card {
        None => grey,
        Some(theme) => {
            let b = f32::from(grey[0]) / 255.0;
            [
                (f32::from(theme.text[0]) * b) as u8,
                (f32::from(theme.text[1]) * b) as u8,
                (f32::from(theme.text[2]) * b) as u8,
            ]
        }
    }
}

/// **What a role is drawn in** — `SPEC-04` §5.1, against Aura's real palette.
///
/// This is the function `crate::theme::Theme::ink` was invented to be, rebuilt
/// on the palette the product actually ships. `Theme`'s `night` and `day` were
/// chosen here rather than taken from `cpp/src/main.cpp`; [`CardTheme`] is the
/// port of the real ones, so the mapping belongs beside it and there is again
/// exactly one place a meaning becomes a colour (`ADR-011`).
///
/// Returns a colour **and an alpha**, because one of the five roles is an alpha
/// in the C++ rather than a hue.
///
/// | role | what it draws with | where that comes from |
/// | :-- | :-- | :-- |
/// | `Normal` | [`text_colour`] | the theme's ink, dimmed by brightness |
/// | `Muted` | the same, at **35% alpha** | `DrawWeatherPanel`'s rule is `text_col.a * 0.35f`, and §5.1 says in as many words that the divider *"is this role by another name"* |
/// | `Warn` | `CardTheme::caution` | `{255,197,0}` dark, `{150,95,0}` light |
/// | `Alarm` | `CardTheme::alert` | `{255,69,58}` dark, `{186,26,26}` light |
///
/// # One thing it does that is worth disagreeing with out loud
///
/// **With no card theme at all, `warn` and `alarm` still get their signal
/// colours.** The default black background is the C++'s `card = nullptr`, which
/// has no palette to ask — but `{255,69,58}` and `{255,197,0}` are
/// `CardTheme`'s *in-class defaults* in `cpp/src/main.cpp`, not one theme's
/// choice, so they are the answer the C++ itself would give. Drawing an alarm
/// in the same grey as everything else would delete the distinction the role
/// exists to carry, which is worse than either.
#[must_use]
pub fn role_ink(card: Option<&CardTheme>, grey: [u8; 3], role: Role) -> ([u8; 3], u8) {
    let ink = text_colour(card, grey);
    // Brightness reaches EVERY colour a card draws, signal colours included —
    // the original reading card's `dim` lambda, and the reason `Theme::dimmed`
    // existed.
    let dim = |c: [u8; 3]| {
        let b = f32::from(grey[0]) / 255.0;
        [
            (f32::from(c[0]) * b) as u8,
            (f32::from(c[1]) * b) as u8,
            (f32::from(c[2]) * b) as u8,
        ]
    };
    let signal = |pick: fn(&CardTheme) -> [u8; 3], fallback: [u8; 3]| {
        dim(card.map_or(fallback, pick))
    };
    match role {
        Role::Normal => (ink, 255),
        // `rule.a = (unsigned char)(text_col.a * 0.35f)`, and the text alpha is
        // 255 — so 89, the same number `weather_panel` computes for its rule.
        Role::Muted => (ink, (255.0 * 0.35) as u8),
        Role::Warn => (signal(|t| t.caution, CAUTION_DARK), 255),
        Role::Alarm => (signal(|t| t.alert, ALERT_DARK), 255),
    }
}

/// `CardTheme::rainbow_alpha` — the opacity `cpp/src/main.cpp` gives `kCardPride`.
///
/// **A constant rather than a field, because one theme has it.** The C++ carries
/// it on every `CardTheme` and only the rainbow one reads it; a field here would
/// be a value nine themes had to be given and none would use.
pub(crate) const RAINBOW_ALPHA: u8 = 190;

/// **`kStops`** — the six colours, `cpp/src/widgets.hpp:255`.
///
/// Carried over rather than re-chosen, like every other palette in this file.
const RAINBOW_STOPS: [[u8; 3]; 6] = [
    [228, 3, 3],     // red
    [255, 140, 0],   // orange
    [255, 237, 0],   // yellow
    [0, 128, 38],    // green
    [0, 77, 255],    // blue
    [117, 7, 135],   // purple
];

/// The colour at `along` through the six stops, `0.0..=1.0`.
///
/// Five segments between six stops, which is why the C++ multiplies by five and
/// clamps the segment index to four: at exactly `1.0` the index would be five
/// and there is no seventh stop to interpolate toward.
fn rainbow_at(along: f32) -> [u8; 3] {
    let scaled = (along * 5.0).clamp(0.0, 5.0);
    let segment = (scaled as usize).min(4);
    let t = scaled - segment as f32;
    let a = RAINBOW_STOPS[segment];
    let b = RAINBOW_STOPS[segment + 1];
    let channel = |i: usize| {
        let from = f32::from(a[i]);
        (f32::from(b[i]) - from).mul_add(t, from) as u8
    };
    [channel(0), channel(1), channel(2)]
}

/// **`DrawRainbowCard`**, whole — `cpp/src/widgets.hpp:254`.
///
/// ```text
///   ╭──────────────────────────╮   six stops across, five gradients
///   │ red  orange  …  purple   │   four corner discs sampled from the same ramp
///   ╰──────────────────────────╯   a black fade over the outer half-radius
/// ```
///
/// # It was unbuildable for a fortnight over one missing primitive
///
/// `SPEC-04` §11 (14) carried this as *"not built"* because *"`rivet_sdk::draw`
/// has neither a gradient fill nor a circle"*. **Half of that was stale**: the
/// circle arrived with the weather glyphs and nobody revisited the claim. The
/// gradient was the real gap and is now `rivet_sdk::draw::fill_gradient_h`.
///
/// Until then the Born This Way palette drew **no card at all** — the renderer
/// refused to substitute a flat fill, which was right (§11 (14): *"drawing a
/// flat fill in its place is not one"*) and left the palette unusable.
///
/// # The geometry is integer-snapped, and the C++ says why
///
/// > *Integer-snap all geometry — prevents float-rounding gaps between strips.*
///
/// Three strips and four discs have to meet exactly. A float edge that rounds
/// one way for the body and the other for the top leaves a hairline of
/// background through the card, which on a rainbow is very visible.
///
/// **The strips are drawn colour-mapped to the FULL card width**, not to their
/// own: the top and bottom are inset by the radius, and a gradient restarted at
/// their own left edge would put red under the corner disc's orange.
///
/// # The corner seam, which is the ORIGINAL's and is reproduced deliberately
///
/// **Look at `target/rainbow-card.ppm` at a corner and there is a visible step
/// along the disc's own centre lines.** It is not a porting mistake, and
/// finding that out was worth more than the fix would have been.
///
/// The card is drawn as translucent shapes that OVERLAP, in this order: body
/// strip, top and bottom strips, four discs, four fades. A disc at the
/// bottom-left spans a quarter that no strip reached and three quarters that
/// one did — so at `alpha = 190`:
///
/// | region | coats | opacity |
/// | :-- | --: | --: |
/// | the disc's outer quadrant | 1 | `190/255` = **0.745** |
/// | its other three, over a strip | 2 | `1 - (1 - 0.745)²` = **0.935** |
///
/// A step of 0.19 in opacity, along a straight edge, is exactly what the zoom
/// shows. `cpp/src/widgets.hpp:307-316` draws the same shapes in the same order
/// at the same alpha, so raylib produces it too.
///
/// **Reproduced rather than corrected**, because R-125 rules the appearance
/// fixed and the C++ is the only source for it — and because the fix is not
/// small: it would mean compositing the whole card opaque into a scratch buffer
/// and applying `alpha` once, which is a *different picture* and a decision
/// rather than a repair. `SPEC-04` §11 (16) carries it for the maintainer, who
/// is the one who can say whether the wall has ever looked wrong.
#[allow(clippy::too_many_arguments)] // a rectangle and an opacity
pub fn rainbow_card(canvas: &mut [u8], t: &Target, x: f32, y: f32, w: f32, h: f32, alpha: u8) {
    if w <= 0.0 || h <= 0.0 || alpha == 0 {
        return;
    }
    let (ix, iy) = (clamp_usize(x), clamp_usize(y));
    let (iw, ih) = (w as usize, h as usize);
    let radius = (w.min(h) * 0.10) as usize;
    if iw <= 2 * radius || ih <= 2 * radius {
        return;
    }

    // One strip, clipped to `[from, from + width)` but coloured as though it
    // spanned the whole card — the C++'s `drawStrip`.
    let strip = |canvas: &mut [u8], from: usize, top: usize, width: usize, height: usize| {
        if width == 0 || height == 0 {
            return;
        }
        let along = |px: usize| (px.saturating_sub(ix) as f32) / w;
        for column in 0..width {
            let colour = rainbow_at(along(from + column));
            rivet_sdk::draw::fill_gradient_h(
                canvas,
                t,
                from + column,
                top,
                1,
                height,
                (colour, alpha),
                (colour, alpha),
            );
        }
    };

    // The centre body spans the full width; the top and bottom are inset, so
    // the corner discs can round them.
    strip(canvas, ix, iy + radius, iw, ih - 2 * radius);
    strip(canvas, ix + radius, iy, iw - 2 * radius, radius);
    strip(canvas, ix + radius, iy + ih - radius, iw - 2 * radius, radius);

    // The four corners, each sampled from the ramp at its own centre so the
    // disc continues the strip beside it rather than restarting the gradient.
    let r = radius as f32;
    for (cx, cy) in [
        (ix + radius, iy + radius),
        (ix + iw - radius, iy + radius),
        (ix + radius, iy + ih - radius),
        (ix + iw - radius, iy + ih - radius),
    ] {
        let colour = rainbow_at((cx.saturating_sub(ix) as f32) / w);
        rivet_sdk::draw::circle_alpha(canvas, t, cx as f32, cy as f32, r, colour, alpha);
    }

    // **The soft edge**: the outermost half-radius fades from black inward.
    // Four gradients from opaque black to TRANSPARENT black, which is the pair
    // of alphas `fill_gradient_h` exists to carry.
    let fade = (radius / 2).max(1);
    let black = ([0, 0, 0], alpha);
    let clear = ([0, 0, 0], 0);
    rivet_sdk::draw::fill_gradient_v(
        canvas, t, ix + radius, iy, iw - 2 * radius, fade, black, clear,
    );
    rivet_sdk::draw::fill_gradient_v(
        canvas, t, ix + radius, iy + ih - fade, iw - 2 * radius, fade, clear, black,
    );
    rivet_sdk::draw::fill_gradient_h(
        canvas, t, ix, iy + radius, fade, ih - 2 * radius, black, clear,
    );
    rivet_sdk::draw::fill_gradient_h(
        canvas, t, ix + iw - fade, iy + radius, fade, ih - 2 * radius, clear, black,
    );
}

/// `DrawRectangleRounded(rect, 0.10f, 8, colour)`.
///
/// raylib's radius for `roundness` is `min(w, h) * roundness / 2`, so 0.10
/// means five percent of the shorter side. See the module note for what is
/// different about the corner itself.
#[allow(clippy::too_many_arguments)] // a rectangle, a colour and an opacity
pub(crate) fn rounded_card(
    canvas: &mut [u8],
    t: &Target,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    colour: [u8; 3],
    alpha: u8,
) {
    if w <= 0.0 || h <= 0.0 || alpha == 0 {
        return;
    }
    let radius = (w.min(h) * 0.05) as usize;
    draw::fill_rounded_alpha(
        canvas,
        t,
        clamp_usize(x),
        clamp_usize(y),
        w.max(0.0) as usize,
        h.max(0.0) as usize,
        radius,
        colour,
        alpha,
    );
}

pub(crate) fn clamp_usize(v: f32) -> usize {
    if v <= 0.0 { 0 } else { v as usize }
}

/// `kClockFontSize`, and `DrawClockWidget`'s `kTimeSize`.
pub const CLOCK_FONT_SIZE: f32 = 292.0;
/// `kRegularFontSize`, and `DrawClockWidget`'s `kDateSize`.
pub const REGULAR_FONT_SIZE: f32 = 60.0;

/// **`DrawClockWidget`**, whole.
///
/// ```text
///   ┌───────────────────────┐   card, inset by kCardPad and trimmed at the top
///   │  9:05                 │   time, 292px bold, at the block's left edge
///   │   Monday, September 1 │   date, 60px regular, CENTRED on the block
///   └───────────────────────┘
/// ```
///
/// The block is as wide as the wider of the two strings and as tall as the
/// time's *visible* height plus a 16 px gap plus the date's box. The card behind
/// it is that block plus 20 px on every side, **less an 18% trim off the top** —
/// the C++'s comment says why: Source Code Pro Bold carries about 18% internal
/// whitespace above its cap height at large sizes, so without the trim the gap
/// above the digits looks far bigger than the gap below the date.
#[allow(clippy::too_many_arguments)]
pub fn clock(
    canvas: &mut [u8],
    t: &Target,
    bold: &Typeface,
    regular: &Typeface,
    time: &str,
    date: &str,
    corner: Corner,
    screen_w: f32,
    screen_h: f32,
    grey: [u8; 3],
    pad: f32,
    card: Option<&CardTheme>,
) {
    const GAP: f32 = 16.0;
    const CARD_PAD: f32 = 20.0;

    let (time_w, time_h) = bold.measure(time, CLOCK_FONT_SIZE);
    let (date_w, date_h) = regular.measure(date, REGULAR_FONT_SIZE);

    let visible_h = time_h * (1.0 - DESCENT_FRAC);
    let block_w = time_w.max(date_w);
    let block_h = visible_h + GAP + date_h;

    let (ox, oy) = corner.origin(screen_w, screen_h, block_w, block_h, pad);
    let ink = text_colour(card, grey);

    if let Some(theme) = card {
        let top_trim = time_h * 0.18;
        // **The card behind the widget, and since 2026-09-02 that includes
        // the rainbow one.** It was skipped entirely while
        // `rivet_sdk::draw` had no gradient - `SPEC-04` §11 (14) - and
        // skipping was right: drawing a flat fill instead would have been
        // a different picture presented as this one.
        let (cx, cy) = (ox - CARD_PAD, oy - CARD_PAD + top_trim);
        let (cw, ch) = (
            block_w + 2.0 * CARD_PAD,
            block_h + 2.0 * CARD_PAD - top_trim,
        );
        if theme.rainbow {
            rainbow_card(canvas, t, cx, cy, cw, ch, RAINBOW_ALPHA);
        } else {
            rounded_card(canvas, t, cx, cy, cw, ch, theme.bg, theme.bg_alpha);
        }
    }

    bold.draw(canvas, t, ox, oy, time, CLOCK_FONT_SIZE, ink, 255);

    let date_x = ox + (block_w - date_w) * 0.5;
    regular.draw(
        canvas,
        t,
        date_x,
        oy + visible_h + GAP,
        date,
        REGULAR_FONT_SIZE,
        ink,
        255,
    );
}


// ---------------------------------------------------------------------------
// The weather panel — `DrawWeatherPanel`, `DrawWeatherIcon`, `DrawIndoorIcon`,
// `WrapText`, and `DrawCloudBody`. `SPEC-04` §5.8.
// ---------------------------------------------------------------------------
//
// **Every constant, offset and stacking order below is out of
// `cpp/src/widgets.hpp`.** Where the C++ truncates a float to an `int` before
// handing it to a primitive, so does this — `trunc` is that cast and it is
// called at exactly the places the C++ calls it, because a disc whose centre
// rounds instead of truncating sits half a pixel from where Aura puts it and
// the cloud stops lining up with its own body.

/// `(int)` — C's conversion, which truncates toward zero rather than rounding.
pub(crate) fn trunc(v: f32) -> f32 {
    v as i32 as f32
}

/// `kTempSize` — the first value.
pub(crate) const TEMP_SIZE: f32 = 96.0;
/// `kSecSize` — the second line, deliberately smaller than the value above it.
pub(crate) const SEC_SIZE: f32 = 62.0;
/// `kStmtSize` — the statement under the pair.
pub(crate) const STMT_SIZE: f32 = 44.0;
/// `kIconSize` — the glyph, square.
pub(crate) const ICON_SIZE: f32 = 88.0;
/// `kIconGap` — glyph to the value stack.
pub(crate) const ICON_GAP: f32 = 26.0;
/// `kValGap` — value to second line.
pub(crate) const VAL_GAP: f32 = 4.0;
/// `kColGap` — each column to the rule, so the rule sits 48 either side.
pub(crate) const COL_GAP: f32 = 48.0;
/// `kStmtGap` — the pair to the statement.
pub(crate) const STMT_GAP: f32 = 26.0;
/// `kLineGap` — between wrapped statement lines.
pub(crate) const LINE_GAP: f32 = 6.0;
/// `kCardPad` / `kCardHPad` — the card, vertically and horizontally. **Not the
/// clock's**: the clock card is 20 on every side and this one is wider than it
/// is tall.
pub(crate) const CARD_PAD: f32 = 20.0;
pub(crate) const CARD_HPAD: f32 = 30.0;
/// The rule between the columns: two pixels, at 35% of the text alpha.
pub(crate) const DIVIDER_W: f32 = 2.0;

/// **Whether this glyph has a drawing.**
///
/// Eleven of `SPEC-04` §5.6's thirteen are branches of `DrawWeatherIcon` or
/// `DrawIndoorIcon` and are ported below. `clock` and `bus` are not — they
/// exist in the vocabulary and in no Aura source, so there is nothing to port
/// and inventing one would be inventing a picture (§7 rule 5, and the `605`
/// failure it names).
///
/// A column whose glyph cannot be drawn **reserves no width for one**, which is
/// §5.8.3 item 4's rule reached from the other direction: the numbers are
/// centred against what is actually there rather than against a hole.
#[must_use]
pub const fn icon_is_drawn(glyph: Glyph) -> bool {
    !matches!(glyph, Glyph::Clock | Glyph::Bus)
}

/// `DrawCloudBody` — bumps on top, a flat body under them.
///
/// The C++'s own note: *"the rectangle is deliberately narrower than the side
/// circles so both left and right edges appear rounded rather than cut
/// straight."*
fn cloud_body(canvas: &mut [u8], t: &Target, x: f32, y: f32, sz: f32, col: [u8; 3]) {
    let c = |canvas: &mut [u8], cx: f32, cy: f32, r: f32| {
        draw::circle(canvas, t, trunc(cx), trunc(cy), trunc(r), col);
    };
    c(canvas, x + sz * 0.22, y + sz * 0.52, sz * 0.22); // left bump
    c(canvas, x + sz * 0.50, y + sz * 0.34, sz * 0.28); // centre, tallest
    c(canvas, x + sz * 0.76, y + sz * 0.54, sz * 0.20); // right bump
    draw::fill(
        canvas,
        t,
        clamp_usize(trunc(x + sz * 0.14)),
        clamp_usize(trunc(y + sz * 0.52)),
        clamp_usize(trunc(sz * 0.72)),
        clamp_usize(trunc(sz * 0.27)),
        col,
    );
}

/// `DrawIndoorIcon` — a house in outline with a filled thermometer inside.
///
/// The C++'s reason for the outline: *"stroked rather than filled, to read as a
/// place rather than as another weather condition"*. And for the discs at the
/// vertices: *"raylib has no line joins, so a disc is drawn at each vertex;
/// without them the roof peak and eaves show notches at this stroke width."*
/// `rivet_sdk::draw::line` has no joins either, for the same reason, so the
/// discs are load-bearing here rather than decorative.
fn indoor_icon(canvas: &mut [u8], t: &Target, x: f32, y: f32, sz: f32, col: [u8; 3]) {
    let thickness = 2.0_f32.max(sz * 0.055);
    let joint = thickness * 0.5;

    let apex = (x + sz * 0.50, y + sz * 0.07);
    let eave_l = (x + sz * 0.07, y + sz * 0.44);
    let eave_r = (x + sz * 0.93, y + sz * 0.44);
    let foot_l = (x + sz * 0.07, y + sz * 0.93);
    let foot_r = (x + sz * 0.93, y + sz * 0.93);

    for (a, b) in [
        (apex, eave_l),   // roof, left slope
        (apex, eave_r),   // roof, right slope
        (eave_l, foot_l), // left wall
        (eave_r, foot_r), // right wall
        (foot_l, foot_r), // floor
    ] {
        draw::line(canvas, t, a.0, a.1, b.0, b.1, thickness, col);
    }
    for v in [apex, eave_l, eave_r, foot_l, foot_r] {
        draw::circle(canvas, t, trunc(v.0), trunc(v.1), joint, col);
    }

    // The thermometer: a filled stem rising from a filled bulb, both centred on
    // the house. The bulb is wider than the stem, which is what makes it read
    // as a thermometer rather than a pin.
    let cx = x + sz * 0.50;
    let bulb_y = y + sz * 0.77;
    let bulb_r = sz * 0.105;
    let stem_w = sz * 0.075;
    let stem_top = y + sz * 0.34;

    draw::line(canvas, t, cx, stem_top, cx, bulb_y, stem_w, col);
    draw::circle(canvas, t, trunc(cx), trunc(stem_top), stem_w * 0.5, col); // rounded cap
    draw::circle(canvas, t, trunc(cx), trunc(bulb_y), bulb_r, col);
}

/// `DrawWeatherIcon` — one branch per condition, drawn with primitives.
///
/// The C++ matches on NWS slug substrings; here the producer has already
/// collapsed the slug space to `SPEC-04` §5.6's closed set, which is what §5.6
/// says it always was — *"mapping a slug to a glyph is a producer's job"*. Each
/// arm below is the branch its slug reached.
///
/// **There is no `else`.** The C++ has one — a dotted circle for a slug it does
/// not recognise — and it is unreachable from a vocabulary of thirteen names.
/// A glyph with no drawing is not drawn at all ([`icon_is_drawn`]).
fn weather_icon(canvas: &mut [u8], t: &Target, glyph: Glyph, x: f32, y: f32, sz: f32, col: [u8; 3]) {
    let cx = x + sz * 0.5;
    let cy = y + sz * 0.5;
    match glyph {
        // `skc`, `hot` — sun disc and eight rays.
        Glyph::Clear => {
            draw::circle(canvas, t, trunc(cx), trunc(cy), trunc(sz * 0.26), col);
            for i in 0..8 {
                let a = i as f32 * core::f32::consts::PI / 4.0;
                draw::line(
                    canvas,
                    t,
                    cx + sz * 0.33 * a.cos(),
                    cy + sz * 0.33 * a.sin(),
                    cx + sz * 0.46 * a.cos(),
                    cy + sz * 0.46 * a.sin(),
                    2.5,
                    col,
                );
            }
        }
        // `few`, `sct` — a small sun upper right, a cloud below and left.
        Glyph::PartlyCloudy => {
            let (sx, sy) = (x + sz * 0.65, y + sz * 0.24);
            draw::circle(canvas, t, trunc(sx), trunc(sy), trunc(sz * 0.17), col);
            for i in 0..6 {
                let a = i as f32 * core::f32::consts::PI / 3.0;
                draw::line(
                    canvas,
                    t,
                    sx + sz * 0.22 * a.cos(),
                    sy + sz * 0.22 * a.sin(),
                    sx + sz * 0.32 * a.cos(),
                    sy + sz * 0.32 * a.sin(),
                    2.0,
                    col,
                );
            }
            cloud_body(canvas, t, x, y + sz * 0.24, sz * 0.82, col);
        }
        // `bkn`, `ovc`.
        Glyph::Cloudy => cloud_body(canvas, t, x, y + sz * 0.06, sz, col),
        // `rain`, `shower`, `drizzle`. The C++'s note: the cloud is in the upper
        // portion and the drops in the lower so that the icon's visual centre
        // still lands near `sz*0.50`.
        Glyph::Rain => {
            cloud_body(canvas, t, x, y + sz * 0.10, sz * 0.68, col);
            for i in 0..3 {
                let rx = x + sz * (0.18 + i as f32 * 0.26);
                draw::line(
                    canvas,
                    t,
                    rx,
                    y + sz * 0.66,
                    rx - sz * 0.06,
                    y + sz * 0.89,
                    2.0,
                    col,
                );
            }
        }
        // `tsra`, `thunder` — a cloud and a bolt. `DrawLineStrip` draws
        // one-pixel lines, not thick ones, which is why this is a strip and the
        // rain above is three `DrawLineEx`.
        Glyph::Thunder => {
            cloud_body(canvas, t, x, y + sz * 0.08, sz * 0.68, col);
            let (lx, ly) = (x + sz * 0.44, y + sz * 0.62);
            draw::line_strip(
                canvas,
                t,
                &[
                    (lx + sz * 0.12, ly),
                    (lx - sz * 0.04, ly + sz * 0.18),
                    (lx + sz * 0.07, ly + sz * 0.18),
                    (lx - sz * 0.12, ly + sz * 0.38),
                ],
                col,
            );
        }
        // `snow`, `blizzard`, `flurr` — a cloud and four dots.
        Glyph::Snow => {
            cloud_body(canvas, t, x, y, sz * 0.78, col);
            for i in 0..4 {
                draw::circle(
                    canvas,
                    t,
                    trunc(x + sz * (0.12 + i as f32 * 0.24)),
                    trunc(y + sz * 0.83),
                    trunc(sz * 0.04),
                    col,
                );
            }
        }
        // `sleet`, `fzra`, `ice` — a cloud and alternating drops and dots.
        Glyph::Sleet => {
            cloud_body(canvas, t, x, y, sz * 0.78, col);
            for i in 0..3 {
                let rx = x + sz * (0.18 + i as f32 * 0.28);
                let ry = y + sz * 0.70;
                if i % 2 == 0 {
                    draw::circle(canvas, t, trunc(rx), trunc(ry + sz * 0.12), trunc(sz * 0.04), col);
                } else {
                    draw::line(canvas, t, rx, ry, rx - sz * 0.05, ry + sz * 0.22, 2.0, col);
                }
            }
        }
        // `fog`, `haze`, `smoke`, `dust` — horizontal dashes of decreasing
        // length, centred.
        Glyph::Fog => {
            for i in 0..4 {
                let lw = sz * (0.86 - i as f32 * 0.08);
                let ly = y + sz * (0.22 + i as f32 * 0.20);
                draw::line(canvas, t, x + (sz - lw) * 0.5, ly, x + (sz - lw) * 0.5 + lw, ly, 3.0, col);
            }
        }
        // `wind` — horizontal lines, left aligned rather than centred.
        Glyph::Wind => {
            for i in 0..3 {
                let lw = sz * (0.80 - i as f32 * 0.10);
                let ly = y + sz * (0.30 + i as f32 * 0.22);
                draw::line(canvas, t, x + sz * 0.10, ly, x + sz * 0.10 + lw, ly, 3.0, col);
            }
        }
        // `unavailable` — a cloud with a cross under it. **This is a condition
        // and not a state**: the weather service said nothing, which is itself
        // worth drawing (`SPEC-04` §5.6).
        Glyph::Unavailable => {
            cloud_body(canvas, t, x, y + sz * 0.10, sz * 0.68, col);
            let (lx, ly) = (x + sz * 0.28, y + sz * 0.66);
            let (rw, rh) = (sz * 0.44, sz * 0.26);
            draw::line(canvas, t, lx, ly, lx + rw, ly + rh, 3.0, col);
            draw::line(canvas, t, lx + rw, ly, lx, ly + rh, 3.0, col);
        }
        Glyph::Indoor => indoor_icon(canvas, t, x, y, sz, col),
        // No drawing exists — see `icon_is_drawn`, which stops the caller
        // reserving room for one.
        Glyph::Clock | Glyph::Bus => {}
    }
}

/// `WrapText` — greedy, on spaces, measured at the size it will be drawn at.
///
/// **The renderer wraps and the producer never does** (`SPEC-04` §5.3): the
/// width is the renderer's fact and nobody else's, which is the same reason
/// `WrapText` lives in the C++ renderer and not in the Go controller.
///
/// A word wider than `max_w` on its own becomes its own line rather than being
/// broken. That is the C++'s behaviour and it is a choice: a broken word in a
/// forecast reads as a fault, and a line that overruns reads as a long word.
#[must_use]
pub fn wrap_text(font: &Typeface, text: &str, size: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    // `SplitWords` splits on the space and drops empties — not on any
    // whitespace. A tab in a forecast would stay inside a word, which is what
    // the C++ does and is one fewer thing for two implementations to disagree
    // about.
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if font.measure(&candidate, size).0 <= max_w {
            current = candidate;
        } else {
            if !current.is_empty() {
                lines.push(core::mem::take(&mut current));
            }
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// One side of the panel — `struct PanelColumn`, in this vocabulary.
///
/// The C++ carries `std::optional<double>` temperatures and formats them here;
/// a card carries strings the producer already formatted (`SPEC-04` §5.2), so
/// this holds the two composed strings instead. `"70" + "°F"` is the same
/// string `"%.0f\xC2\xB0%s"` produced, and `"54" + "%"` is the same string its
/// percent branch produced — the concatenation covers both cases exactly,
/// which is why the vocabulary needs no unit flag where the C++ needed one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Column {
    /// The mark at the left of the column, if it has one that can be drawn.
    pub glyph: Option<Glyph>,
    /// The large first line: value and unit, already joined.
    pub value: Option<String>,
    /// The smaller second line: secondary and its unit, already joined.
    pub second: Option<String>,
}

impl Column {
    /// One side of a [`aura_card::Pair`], as a column.
    ///
    /// **`SPEC-04` §5.8.1's renderer obligation**: a column is drawn when its
    /// reading has a value, a secondary, or a glyph. A reading whose state
    /// admits no value carries none, so `connecting` and `unavailable` arrive
    /// here as a glyph or as nothing, which is `PanelColumn::drawable()`.
    #[must_use]
    pub fn from_reading(reading: &aura_card::Reading) -> Self {
        let join = |value: &Option<String>, unit: Option<&str>| -> Option<String> {
            value
                .as_ref()
                .map(|v| unit.map_or_else(|| v.clone(), |u| format!("{v}{u}")))
        };
        Self {
            glyph: reading.glyph.filter(|g| icon_is_drawn(*g)),
            value: join(&reading.value, reading.unit.as_deref()),
            // The C++'s `second_is_percent` flag, expressed as data: a second
            // unit when it differs, the first when it does not.
            second: join(
                &reading.secondary,
                reading
                    .secondary_unit
                    .as_deref()
                    .or(reading.unit.as_deref()),
            ),
        }
    }

    /// `PanelColumn::drawable()`.
    fn drawable(&self) -> bool {
        self.value.is_some() || self.second.is_some() || self.glyph.is_some()
    }
}

/// `struct Measured` — one column, sized.
#[derive(Default, Clone, Copy)]
pub(crate) struct Measured {
    pub(crate) present: bool,
    pub(crate) has_icon: bool,
    pub(crate) value_w: f32,
    pub(crate) second_w: f32,
    pub(crate) values_w: f32,
    pub(crate) values_h: f32,
    pub(crate) width: f32,
    pub(crate) top_h: f32,
}

pub(crate) fn measure_column(bold: &Typeface, c: &Column) -> Measured {
    let mut m = Measured::default();
    if !c.drawable() {
        return m;
    }
    m.present = true;
    // `MeasureTextEx(...).y` is the font size, always — see `crate::typeface`.
    // So the two heights below are `kTempSize` and `kSecSize`, trimmed.
    if let Some(v) = &c.value {
        m.value_w = bold.measure(v, TEMP_SIZE).0;
    }
    if let Some(s) = &c.second {
        m.second_w = bold.measure(s, SEC_SIZE).0;
    }
    // Trim the descender space the digits never occupy, so the gap between the
    // two numbers is the gap that is actually seen.
    let value_h = if c.value.is_some() {
        TEMP_SIZE * (1.0 - DESCENT_FRAC)
    } else {
        0.0
    };
    let second_h = if c.second.is_some() {
        SEC_SIZE * (1.0 - DESCENT_FRAC)
    } else {
        0.0
    };
    m.values_w = m.value_w.max(m.second_w);
    m.values_h = value_h
        + second_h
        + if value_h > 0.0 && second_h > 0.0 {
            VAL_GAP
        } else {
            0.0
        };
    // **No icon, no icon-sized hole.** The C++'s own note: NWS's older
    // observations carry a temperature with an empty slug, the recovery path
    // reaches for exactly those, and reserving the width anyway left the
    // readings floating in space.
    m.has_icon = c.glyph.is_some();
    let icon_w = if m.has_icon { ICON_SIZE } else { 0.0 };
    m.top_h = icon_w.max(m.values_h);
    m.width = icon_w
        + if m.values_w > 0.0 {
            (if icon_w > 0.0 { ICON_GAP } else { 0.0 }) + m.values_w
        } else {
            0.0
        };
    m
}

/// **`DrawWeatherPanel`**, whole.
///
/// ```text
///   ┌──────────────────────────────────────────┐
///   │   ☁  70°F   │   ⌂  76°F                  │   two columns, a rule between
///   │      62°F   │      54%                   │   second line centred under the first
///   │        Chance Rain Showers               │   the statement, wrapped and centred
///   └──────────────────────────────────────────┘
/// ```
///
/// It takes the corner **diagonally opposite the clock**, so the two own one
/// diagonal between them and the corner reading card has the other.
///
/// # The four placements an impression gets wrong
///
/// 1. The glyph is at the **left of its column** and the value stack is centred
///    against it, on the taller of the two.
/// 2. The second line is centred under the first **on the column's own value
///    width**, not sharing its left edge. That is what makes the two read as
///    one stacked reading.
/// 3. The rule spans **88% of the taller column, starting 6% down**.
/// 4. The columns are centred **as a group**, so a column that is not drawn
///    leaves the other in the middle rather than against an edge.
#[allow(clippy::too_many_arguments)]
pub fn weather_panel(
    canvas: &mut [u8],
    t: &Target,
    bold: &Typeface,
    regular: &Typeface,
    left: &Column,
    right: &Column,
    statement: &str,
    clock_corner: Corner,
    screen_w: f32,
    screen_h: f32,
    grey: [u8; 3],
    pad: f32,
    card: Option<&CardTheme>,
) {
    let ml = measure_column(bold, left);
    let mr = measure_column(bold, right);
    if !ml.present && !mr.present && statement.is_empty() {
        return;
    }

    let both = ml.present && mr.present;
    let content_w = (if ml.present { ml.width } else { 0.0 })
        + (if mr.present { mr.width } else { 0.0 })
        + if both {
            2.0f32.mul_add(COL_GAP, DIVIDER_W)
        } else {
            0.0
        };
    let content_h = ml.top_h.max(mr.top_h);

    // --- the statement -----------------------------------------------------
    let lines = if statement.is_empty() {
        Vec::new()
    } else {
        wrap_text(regular, statement, STMT_SIZE, content_w.max(480.0))
    };
    // `MeasureTextEx(regular, "Ag", kStmtSize, 0).y` is `kStmtSize` — the
    // measurement is the font size and not a line height, so the C++'s "Ag" is
    // measuring nothing the constant does not already say. Ported as the
    // constant, with the reason, rather than as a call that looks meaningful.
    let line_h = STMT_SIZE;
    let stmt_w = lines
        .iter()
        .map(|l| regular.measure(l, STMT_SIZE).0)
        .fold(0.0_f32, f32::max);
    let stmt_h = if lines.is_empty() {
        0.0
    } else {
        (lines.len() as f32).mul_add(line_h, (lines.len() as f32 - 1.0) * LINE_GAP)
    };

    let block_w = content_w.max(stmt_w);
    let block_h = content_h + if lines.is_empty() { 0.0 } else { STMT_GAP + stmt_h };

    // --- placement ---------------------------------------------------------
    let corner = clock_corner.opposite();
    let (ox, oy) = corner.origin(screen_w, screen_h, block_w, block_h, pad);
    let ink = text_colour(card, grey);

    if let Some(theme) = card {
        // **The card behind the widget, and since 2026-09-02 that includes
        // the rainbow one.** It was skipped entirely while
        // `rivet_sdk::draw` had no gradient - `SPEC-04` §11 (14) - and
        // skipping was right: drawing a flat fill instead would have been
        // a different picture presented as this one.
        let (cx, cy) = (ox - CARD_HPAD, oy - CARD_PAD);
        let (cw, ch) = (
            2.0f32.mul_add(CARD_HPAD, block_w),
            2.0f32.mul_add(CARD_PAD, block_h),
        );
        if theme.rainbow {
            rainbow_card(canvas, t, cx, cy, cw, ch, RAINBOW_ALPHA);
        } else {
            rounded_card(canvas, t, cx, cy, cw, ch, theme.bg, theme.bg_alpha);
        }
    }

    // --- the columns -------------------------------------------------------
    let mut x = (block_w - content_w).mul_add(0.5, ox);
    if ml.present {
        draw_column(canvas, t, bold, left, ml, x, oy, ink);
        x += ml.width;
    }
    if both {
        let dx = x + COL_GAP;
        // `rule.a = (unsigned char)(text_col.a * 0.35f)` — the text alpha is
        // 255, so 89. `SPEC-04` §5.1: this is the `muted` role, and it is the
        // only place on this card that has one.
        let alpha = (255.0 * 0.35) as u8;
        draw::fill_rounded_alpha(
            canvas,
            t,
            clamp_usize(trunc(dx)),
            clamp_usize(trunc(content_h.mul_add(0.06, oy))),
            clamp_usize(trunc(DIVIDER_W)),
            clamp_usize(trunc(ml.top_h.max(mr.top_h) * 0.88)),
            0,
            ink,
            alpha,
        );
        x = dx + DIVIDER_W + COL_GAP;
    }
    if mr.present {
        draw_column(canvas, t, bold, right, mr, x, oy, ink);
    }

    // --- the statement -----------------------------------------------------
    let mut sy = oy + content_h + STMT_GAP;
    for line in &lines {
        let lw = regular.measure(line, STMT_SIZE).0;
        regular.draw(
            canvas,
            t,
            (block_w - lw).mul_add(0.5, ox),
            sy,
            line,
            STMT_SIZE,
            ink,
            255,
        );
        sy += line_h + LINE_GAP;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_column(
    canvas: &mut [u8],
    t: &Target,
    bold: &Typeface,
    c: &Column,
    m: Measured,
    cx: f32,
    origin_y: f32,
    ink: [u8; 3],
) {
    if !m.present {
        return;
    }
    let ix = cx;
    if let Some(glyph) = c.glyph {
        let icon_y = (m.top_h - ICON_SIZE).mul_add(0.5, origin_y);
        weather_icon(canvas, t, glyph, ix, icon_y, ICON_SIZE, ink);
    }
    let vx = if m.has_icon { ix + ICON_SIZE + ICON_GAP } else { ix };
    let mut vy = (m.top_h - m.values_h).mul_add(0.5, origin_y);

    if let Some(v) = &c.value {
        bold.draw(
            canvas,
            t,
            (m.values_w - m.value_w).mul_add(0.5, vx),
            vy,
            v,
            TEMP_SIZE,
            ink,
            255,
        );
        vy += TEMP_SIZE * (1.0 - DESCENT_FRAC) + VAL_GAP;
    }
    if let Some(s) = &c.second {
        bold.draw(
            canvas,
            t,
            (m.values_w - m.second_w).mul_add(0.5, vx),
            vy,
            s,
            SEC_SIZE,
            ink,
            255,
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// `CornerOrigin`, at every corner, for a block that is not square.
    #[test]
    fn a_block_lands_pad_from_the_two_edges_of_its_corner() {
        let (w, h, bw, bh, pad) = (1920.0, 1080.0, 700.0, 310.0, 48.0);
        assert_eq!(Corner::TopLeft.origin(w, h, bw, bh, pad), (48.0, 48.0));
        assert_eq!(
            Corner::TopRight.origin(w, h, bw, bh, pad),
            (1920.0 - 48.0 - 700.0, 48.0)
        );
        assert_eq!(
            Corner::BottomLeft.origin(w, h, bw, bh, pad),
            (48.0, 1080.0 - 48.0 - 310.0)
        );
        assert_eq!(
            Corner::BottomRight.origin(w, h, bw, bh, pad),
            (1920.0 - 48.0 - 700.0, 1080.0 - 48.0 - 310.0)
        );
    }

    /// The three widgets never share a corner, whichever one the clock is in —
    /// which is what `OppositeCorner` and the reading card's quarter-turn rule
    /// exist to guarantee, and the property that leaves exactly one corner
    /// free.
    #[test]
    fn the_three_widgets_occupy_three_different_corners_at_every_rotation() {
        for clock in Corner::ALL {
            let weather = clock.opposite();
            let reading = clock.quarter_turn();
            assert_ne!(clock, weather, "clock and weather collided");
            assert_ne!(clock, reading, "clock and the reading card collided");
            assert_ne!(weather, reading, "weather and the reading card collided");
        }
    }

    /// **All four corners are occupied and no two widgets share one**, at every
    /// rotation — which is what `main.rs` relies on now that `tertiary` and
    /// `quaternary` both draw through `crate::layout`.
    ///
    /// The fourth is `quarter_turn().opposite()`, and the property that makes it
    /// the right one is not obvious from the name: the clock and the panel own
    /// one diagonal, the quarter turn lands on the other, and its opposite is
    /// the one seat left. A fourth widget placed by any other rule would sooner
    /// or later draw on top of one of the first three.
    #[test]
    fn the_four_widgets_fill_the_four_corners_at_every_rotation() {
        for clock in Corner::ALL {
            let seats = [
                clock,
                clock.opposite(),
                clock.quarter_turn(),
                clock.quarter_turn().opposite(),
            ];
            for (i, a) in seats.iter().enumerate() {
                for b in seats.iter().skip(i + 1) {
                    assert_ne!(a, b, "clock at {clock:?}: two widgets share {a:?}");
                }
            }
            for corner in Corner::ALL {
                assert!(
                    seats.contains(&corner),
                    "clock at {clock:?}: {corner:?} is left empty, so the four seats are not the \
                     four corners"
                );
            }
        }
    }

    /// A rotation always moves the clock. `RandomCorner` loops until it draws a
    /// different one, and a version that did not would leave the panel to burn
    /// in — which is the only reason the rotation exists.
    #[test]
    fn a_rotation_never_leaves_the_clock_where_it_was() {
        let mut rng = Mt19937::new(11);
        let mut corner = Corner::TopLeft;
        for _ in 0..200 {
            let next = corner.moved(&mut rng);
            assert_ne!(next, corner);
            corner = next;
        }
        for _ in 0..50 {
            let interval = Corner::interval(&mut rng).as_secs();
            assert!((120..=480).contains(&interval), "{interval}s");
        }
    }

    /// **The padding follows the card's ALPHA, not the card's existence.**
    /// `kBoxesIce` and `kSolidNavy` both have a theme and a fully transparent
    /// background, so they take the 48 px padding that the themeless default
    /// takes. A port that keyed on `card.is_some()` would push both of them 48
    /// pixels further in than Aura does.
    #[test]
    fn only_a_card_that_can_be_seen_pushes_the_widgets_inward() {
        assert_eq!(widget_pad(Some(&sample::GREEN)), 96.0);
        assert_eq!(widget_pad(Some(&sample::TRANSPARENT)), 48.0);
        // No card at all takes the same padding a transparent one does, which
        // is the whole point: the test is on the alpha, not on `is_some()`.
        assert_eq!(widget_pad(None), 48.0);
        // The rainbow card has no fill and is still visible.
        assert_eq!(widget_pad(Some(&sample::RAINBOW)), 96.0);
    }

    /// Brightness reaches the card's ink, and 60 is Aura's default.
    ///
    /// `{180,255,180}` at 153/255 is `{108,153,108}` — truncated, as the C++'s
    /// `(unsigned char)(card->text.r * b)` truncates.
    #[test]
    fn brightness_scales_the_themes_ink_and_not_only_the_grey() {
        assert_eq!(widget_grey(60), [153, 153, 153]);
        let green = sample::GREEN;
        assert_eq!(text_colour(Some(&green), widget_grey(60)), [108, 153, 108]);
        // With no card the grey is used raw.
        assert_eq!(text_colour(None, widget_grey(60)), [153, 153, 153]);
        // And at full brightness the theme's own colour survives untouched.
        assert_eq!(text_colour(Some(&green), widget_grey(100)), [180, 255, 180]);
    }

    /// **Drawing with no card at all still works**, which is what Solid Black
    /// asks for — the C++'s `card = nullptr`.
    ///
    /// This used to assert that `card_theme(Solid(Black))` was `None`. That
    /// fact belongs to the pack now and is tested there
    /// (`catalogue.rs`'s `every_card_appearance_is_the_renderers_own`). What
    /// remains this crate's to hold is that `None` is a case its widget code
    /// handles rather than a case it never sees.
    #[test]
    fn no_card_at_all_is_a_case_the_widgets_handle() {
        assert_eq!(widget_pad(None), 48.0);
        assert_eq!(text_colour(None, widget_grey(60)), [153, 153, 153]);
        // And an alarm still gets its signal colour with no palette to ask —
        // `role_ink`'s deliberate disagreement, stated in its own doc comment.
        let (ink, _) = role_ink(None, widget_grey(100), Role::Alarm);
        assert_eq!(ink, ALERT_DARK);
    }

    // -----------------------------------------------------------------------
    // The weather panel — SPEC-04 §5.8
    // -----------------------------------------------------------------------

    const BOLD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SourceCodePro-Bold.ttf");
    const REGULAR: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/SourceCodePro-Regular.ttf"
    );

    fn faces() -> (Typeface, Typeface) {
        (
            Typeface::load(BOLD, CLOCK_FONT_SIZE).expect("the bold face"),
            Typeface::load(REGULAR, REGULAR_FONT_SIZE).expect("the regular face"),
        )
    }

    fn canvas(w: usize, h: usize) -> (Vec<u8>, Target) {
        (
            vec![0u8; w * h * 4],
            Target::xrgb(w, h, w * 4),
        )
    }

    /// The bounding box of everything that is not the untouched background.
    fn ink_bounds(canvas: &[u8], t: &Target) -> Option<(usize, usize, usize, usize)> {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for y in 0..t.height {
            for x in 0..t.width {
                let at = y * t.stride + x * t.bytes_per_pixel();
                if canvas[at] != 0 || canvas[at + 1] != 0 || canvas[at + 2] != 0 {
                    bounds = Some(match bounds {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        bounds
    }

    /// The ink bounds of a horizontal band of the canvas.
    fn ink_bounds_between(
        canvas: &[u8],
        t: &Target,
        from: usize,
        to: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for y in from..to.min(t.height) {
            for x in 0..t.width {
                let at = y * t.stride + x * t.bytes_per_pixel();
                if canvas[at] != 0 || canvas[at + 1] != 0 || canvas[at + 2] != 0 {
                    bounds = Some(match bounds {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        bounds
    }

    /// Split the drawn rows at their widest empty run, which is the gap between
    /// the columns and the statement — `kStmtGap`, and the only gap on this card
    /// that no glyph spans.
    fn split_at_the_widest_gap(
        canvas: &[u8],
        t: &Target,
    ) -> ((usize, usize), (usize, usize)) {
        let inked: Vec<bool> = (0..t.height)
            .map(|y| {
                (0..t.width).any(|x| {
                    let at = y * t.stride + x * t.bytes_per_pixel();
                    canvas[at] != 0 || canvas[at + 1] != 0 || canvas[at + 2] != 0
                })
            })
            .collect();
        let first = inked.iter().position(|v| *v).expect("nothing was drawn");
        let last = inked.iter().rposition(|v| *v).expect("nothing was drawn");
        let (mut best, mut best_len, mut run_start, mut run) = (first, 0usize, first, 0usize);
        for (y, has_ink) in inked.iter().enumerate().take(last + 1).skip(first) {
            if *has_ink {
                if run > best_len {
                    best_len = run;
                    best = run_start;
                }
                run = 0;
            } else {
                if run == 0 {
                    run_start = y;
                }
                run += 1;
            }
        }
        assert!(best_len > 0, "the panel has no gap between its two halves");
        ((first, best), (best + best_len, last + 1))
    }

    fn outdoor() -> Column {
        Column {
            glyph: Some(Glyph::Cloudy),
            value: Some("70°F".to_string()),
            second: Some("62°F".to_string()),
        }
    }

    fn indoor() -> Column {
        Column {
            glyph: Some(Glyph::Indoor),
            value: Some("76°F".to_string()),
            second: Some("54%".to_string()),
        }
    }

    /// `WrapText` — greedy on spaces, and a word wider than the line becomes its
    /// own line rather than being broken. A broken word in a forecast reads as a
    /// fault; a long line reads as a long word.
    #[test]
    fn a_statement_wraps_on_spaces_and_never_breaks_a_word() {
        let (_, regular) = faces();
        let text = "Chance Rain Showers then Slight Chance Rain Showers";
        let one_line = wrap_text(&regular, text, 44.0, 4000.0);
        assert_eq!(one_line, vec![text.to_string()]);

        let wrapped = wrap_text(&regular, text, 44.0, 480.0);
        assert!(wrapped.len() > 1, "480px should not hold that in one line");
        assert_eq!(wrapped.join(" "), text, "wrapping lost or reordered a word");
        for line in &wrapped {
            assert!(!line.starts_with(' ') && !line.ends_with(' '), "{line:?}");
        }

        // Narrower than any single word: every word gets a line of its own and
        // none of them is cut.
        let each = wrap_text(&regular, text, 44.0, 1.0);
        assert_eq!(each.len(), text.split(' ').count());
        assert!(wrap_text(&regular, "", 44.0, 480.0).is_empty());
    }

    /// **A column with no glyph reserves no width for one.** The C++ added this
    /// after readings with no condition slug left the numbers floating in an
    /// icon-sized hole, and it is the rule that makes the outdoor recovery path
    /// look right.
    #[test]
    fn a_column_with_no_glyph_reserves_no_room_for_one() {
        let (bold, _) = faces();
        let with = measure_column(&bold, &outdoor());
        let without = measure_column(
            &bold,
            &Column {
                glyph: None,
                ..outdoor()
            },
        );
        assert!(with.has_icon && !without.has_icon);
        assert!(
            (with.width - without.width - (88.0 + 26.0)).abs() < 0.001,
            "the glyph should cost exactly kIconSize + kIconGap: {} vs {}",
            with.width,
            without.width
        );
        // And the column is as tall as the taller of the glyph and the stack.
        assert!((with.top_h - 88.0_f32.max(with.values_h)).abs() < 0.001);
        assert!((without.top_h - without.values_h).abs() < 0.001);
    }

    /// `PanelColumn::drawable()` — a column with no value, no second line and no
    /// glyph is not measured, not drawn, and takes no width.
    #[test]
    fn a_column_with_nothing_to_report_is_not_present_at_all() {
        let (bold, _) = faces();
        let empty = measure_column(&bold, &Column::default());
        assert!(!empty.present);
        assert_eq!(empty.width, 0.0);
        // A glyph alone is enough, which is how the outdoor column reports that
        // the weather service said nothing (`SPEC-04` §5.6).
        let glyph_only = measure_column(
            &bold,
            &Column {
                glyph: Some(Glyph::Unavailable),
                ..Column::default()
            },
        );
        assert!(glyph_only.present);
        assert!((glyph_only.width - 88.0).abs() < 0.001, "{}", glyph_only.width);
    }

    /// A reading becomes a column by **joining the value to its unit**, and the
    /// second line takes its own unit when it has one. `"70" + "°F"` is the
    /// string `"%.0f\xC2\xB0%s"` produced and `"54" + "%"` is the string its
    /// percent branch produced, which is why the vocabulary needs no unit flag.
    #[test]
    fn a_reading_becomes_a_column_by_joining_each_value_to_its_unit() {
        let room = aura_card::Reading {
            glyph: Some(Glyph::Indoor),
            value: Some("76".to_string()),
            unit: Some("°F".to_string()),
            secondary: Some("54".to_string()),
            secondary_unit: Some("%".to_string()),
            ..aura_card::Reading::default()
        };
        let column = Column::from_reading(&room);
        assert_eq!(column.value.as_deref(), Some("76°F"));
        assert_eq!(column.second.as_deref(), Some("54%"));

        // No second unit: the first one is used, which is the outdoor case.
        let outside = aura_card::Reading {
            secondary_unit: None,
            ..room.clone()
        };
        assert_eq!(Column::from_reading(&outside).second.as_deref(), Some("54°F"));

        // A glyph with no drawing is dropped, so no width is reserved for it —
        // `icon_is_drawn`, and `SPEC-04` §7 rule 5.
        let clocklike = aura_card::Reading {
            glyph: Some(Glyph::Clock),
            ..room
        };
        assert_eq!(Column::from_reading(&clocklike).glyph, None);
    }

    /// **The panel takes the corner diagonally opposite the clock**, at every
    /// rotation. The clock and the panel own one diagonal between them, which is
    /// what leaves the other free for the corner reading card.
    #[test]
    fn the_panel_lands_in_the_corner_opposite_the_clock() {
        let (bold, regular) = faces();
        let (w, h) = (1920, 1080);
        for (clock, expect_right, expect_bottom) in [
            (Corner::TopLeft, true, true),
            (Corner::TopRight, false, true),
            (Corner::BottomLeft, true, false),
            (Corner::BottomRight, false, false),
        ] {
            let (mut c, t) = canvas(w, h);
            weather_panel(
                &mut c,
                &t,
                &bold,
                &regular,
                &outdoor(),
                &indoor(),
                "Chance Rain Showers",
                clock,
                w as f32,
                h as f32,
                widget_grey(60),
                96.0,
                Some(&sample::GREEN),
            );
            let (x0, y0, x1, y1) = ink_bounds(&c, &t).expect("the panel drew nothing");
            let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
            assert_eq!(cx > w / 2, expect_right, "clock {clock:?}: horizontal");
            assert_eq!(cy > h / 2, expect_bottom, "clock {clock:?}: vertical");
        }
    }

    /// **The columns are centred as a group**, on the same block the statement
    /// is centred on — so a column that goes away moves the other to the middle
    /// instead of leaving it against an edge. `SPEC-04` §5.8.3 item 6, and it is
    /// what a contributor going quiet looks like on the wall.
    ///
    /// The block itself is anchored to its **corner**, so its left edge moves
    /// when its width does; comparing screen positions between the two renders
    /// would be comparing two different blocks. What is compared instead is the
    /// columns' centre against the statement's, inside each render.
    #[test]
    fn the_columns_are_centred_on_the_same_block_the_statement_is() {
        let (bold, regular) = faces();
        let (w, h) = (1920, 1080);
        let render = |left: &Column, right: &Column| {
            let (mut c, t) = canvas(w, h);
            weather_panel(
                &mut c,
                &t,
                &bold,
                &regular,
                left,
                right,
                "Chance Rain Showers",
                Corner::TopLeft,
                w as f32,
                h as f32,
                widget_grey(60),
                96.0,
                // No card, so the ink is the type, the icons and the rule —
                // a filled rectangle behind them would make every bound the
                // card's.
                None,
            );
            (c, t)
        };

        for (name, left, right) in [
            ("two columns", outdoor(), indoor()),
            ("one column", outdoor(), Column::default()),
            ("the other one", Column::default(), indoor()),
        ] {
            let (c, t) = render(&left, &right);
            let (top, bottom) = split_at_the_widest_gap(&c, &t);
            let columns = ink_bounds_between(&c, &t, top.0, top.1).expect("no columns drawn");
            let statement = ink_bounds_between(&c, &t, bottom.0, bottom.1).expect("no statement");
            let centre = |b: (usize, usize, usize, usize)| (b.0 + b.2) as i64 / 2;
            assert!(
                (centre(columns) - centre(statement)).abs() <= 4,
                "{name}: columns centred at {} and the statement at {}",
                centre(columns),
                centre(statement)
            );
        }
    }

    /// One column is narrower than two, and the rule between them is only there
    /// when there are two of them to separate.
    #[test]
    fn the_rule_exists_only_when_there_are_two_columns_to_separate() {
        let (bold, regular) = faces();
        let (w, h) = (1920, 1080);
        let width_of = |left: &Column, right: &Column| {
            let (mut c, t) = canvas(w, h);
            weather_panel(
                &mut c, &t, &bold, &regular, left, right, "", Corner::TopLeft,
                w as f32, h as f32, widget_grey(60), 96.0, None,
            );
            let (x0, _, x1, _) = ink_bounds(&c, &t).expect("nothing drawn");
            x1 - x0
        };
        let both = width_of(&outdoor(), &indoor());
        let one = width_of(&outdoor(), &Column::default());
        // kColGap either side of a 2px rule is 98px of separation on top of the
        // second column's own width.
        assert!(
            both > one + 98,
            "two columns should carry the rule and its gaps: {both} vs {one}"
        );
    }

    /// **The corner seam is pinned, because it is a known difference rather
    /// than a defect** — see `rainbow_card`'s note.
    ///
    /// A translucent disc over a translucent strip double-blends, so the disc's
    /// outer quadrant gets one coat and its inner three get two. This measures
    /// the step so that a future change to how the card is composited is a
    /// DELIBERATE one and not a silent repaint.
    #[test]
    fn the_rainbow_corner_carries_the_originals_double_blend() {
        let (mut c, t) = canvas(400, 400);
        rainbow_card(&mut c, &t, 100.0, 100.0, 200.0, 200.0, RAINBOW_ALPHA);
        // radius = min(200,200) * 0.10 = 20, so the bottom-left disc is centred
        // at (120, 280) with r = 20.
        let brightness = |x: usize, y: usize| {
            let at = y * t.stride + x * t.bytes_per_pixel();
            u32::from(c[at]) + u32::from(c[at + 1]) + u32::from(c[at + 2])
        };
        // Just inside the disc, below-left of its centre: ONE coat.
        let outer = brightness(112, 288);
        // Just inside the disc, above-right of its centre: over the body, TWO.
        let inner = brightness(128, 272);
        assert!(
            inner > outer,
            "the double-blend is gone: outer {outer}, inner {inner}. If that was deliberate, \
             this test and `rainbow_card`'s note are what to change - SPEC-04 §11 (16)"
        );
    }

    /// **The rainbow card writes a picture of itself into `target/`**, because
    /// the only judgement that matters on a six-stop gradient with four sampled
    /// corner discs is a person looking at it.
    ///
    /// It asserts as well as writing — a file produced while drawing nothing
    /// would be worse than no file — and what it asserts is the property the
    /// C++ comment is about: **no gaps between the strips**. Every row through
    /// the middle of the card must be painted edge to edge, because a float
    /// edge rounding one way for the body and the other for the top leaves a
    /// hairline of background through it, which on a rainbow is very visible.
    #[test]
    fn the_rainbow_card_draws_itself_into_a_file_that_can_be_looked_at() {
        let (bold, regular) = faces();
        let (w, h) = (1920usize, 1080usize);
        let (mut c, t) = canvas(w, h);

        // On its own first, so the assertions below are about the card and not
        // about what was drawn over it.
        rainbow_card(&mut c, &t, 300.0, 300.0, 1000.0, 400.0, RAINBOW_ALPHA);

        // **No gaps.** A horizontal line through the middle is painted at every
        // pixel between the card's edges.
        let painted = |x: usize, y: usize| {
            let at = y * t.stride + x * t.bytes_per_pixel();
            c[at] != 0 || c[at + 1] != 0 || c[at + 2] != 0
        };
        for x in 300..1300 {
            assert!(painted(x, 500), "a gap in the body at x={x}");
        }
        // And a vertical one, which is where the three strips meet.
        for y in 300..700 {
            assert!(painted(800, y), "a gap between strips at y={y}");
        }
        // The ramp actually runs: red at the left, purple at the right.
        let rgb = |x: usize, y: usize| {
            let at = y * t.stride + x * t.bytes_per_pixel();
            [c[at + 2], c[at + 1], c[at]]
        };
        let left = rgb(340, 500);
        let right = rgb(1260, 500);
        assert!(left[0] > left[2], "the left end is not red: {left:?}");
        assert!(right[2] > right[1], "the right end is not purple: {right:?}");

        // Then the clock over it, which is what a wall actually shows.
        widgets_clock_over_rainbow(&mut c, &t, &bold, &regular, w as f32, h as f32);

        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let at = y * t.stride + x * t.bytes_per_pixel();
                ppm.extend_from_slice(&[c[at + 2], c[at + 1], c[at]]);
            }
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/target/rainbow-card.ppm");
        // **Created, not assumed.** `CARGO_MANIFEST_DIR/target` exists only if
        // somebody has built this crate natively in its own tree; under
        // `tools/test-all.sh`, `CARGO_TARGET_DIR` points elsewhere and this
        // directory is never made. These three tests therefore passed in a tree
        // with a leftover `target/` and failed on a clean checkout — a green
        // run resting on an untracked build artifact, which is `AGENTS.md` §7's
        // shape: the probe could not tell its own answer from the tree's.
        std::fs::create_dir_all(std::path::Path::new(path).parent().expect("a parent"))
            .expect("the directory for a rendered fixture should be creatable");
        std::fs::write(path, ppm).expect("target/ should be writable");
    }

    /// The clock, drawn on the Born This Way theme — the composition a wall
    /// running that palette actually shows.
    fn widgets_clock_over_rainbow(
        c: &mut [u8],
        t: &Target,
        bold: &Typeface,
        regular: &Typeface,
        w: f32,
        h: f32,
    ) {
        clock(
            c,
            t,
            bold,
            regular,
            "9:05",
            "Monday, September 2",
            Corner::TopLeft,
            w,
            h,
            widget_grey(60),
            widget_pad(Some(&sample::RAINBOW)),
            Some(&sample::RAINBOW),
        );
    }

    /// **The panel writes a picture of itself into `target/`** so that the one
    /// judgement that matters — a person looking at it — can be made without a
    /// board.
    ///
    /// It is a test rather than an example because a binary crate cannot carry
    /// one, and it asserts rather than only writing: a file this test produced
    /// while drawing nothing would be worse than no file.
    #[test]
    fn the_panel_draws_itself_into_a_file_that_can_be_looked_at() {
        let (bold, regular) = faces();
        let (w, h) = (1920, 1080);
        let (mut c, t) = canvas(w, h);
        // The ground the wall actually shows: the phosphor-green card sits on
        // Digital Rain, so a flat black here is the closest honest stand-in.
        weather_panel(
            &mut c,
            &t,
            &bold,
            &regular,
            &outdoor(),
            &indoor(),
            "Chance Rain Showers then Slight Chance Rain Showers",
            Corner::TopLeft,
            w as f32,
            h as f32,
            widget_grey(60),
            96.0,
            Some(&sample::GREEN),
        );
        let (x0, y0, x1, y1) = ink_bounds(&c, &t).expect("the panel drew nothing");
        assert!(x1 - x0 > 400 && y1 - y0 > 200, "the panel is implausibly small");

        // A binary PPM: three bytes a pixel, a header a person can read, and no
        // encoder to be wrong about. `target/` is not tracked.
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let at = y * t.stride + x * t.bytes_per_pixel();
                ppm.extend_from_slice(&[c[at + 2], c[at + 1], c[at]]);
            }
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/target/weather-panel.ppm");
        // **Created, not assumed.** `CARGO_MANIFEST_DIR/target` exists only if
        // somebody has built this crate natively in its own tree; under
        // `tools/test-all.sh`, `CARGO_TARGET_DIR` points elsewhere and this
        // directory is never made. These three tests therefore passed in a tree
        // with a leftover `target/` and failed on a clean checkout — a green
        // run resting on an untracked build artifact, which is `AGENTS.md` §7's
        // shape: the probe could not tell its own answer from the tree's.
        std::fs::create_dir_all(std::path::Path::new(path).parent().expect("a parent"))
            .expect("the directory for a rendered fixture should be creatable");
        std::fs::write(path, ppm).expect("target/ should be writable");
    }
}
