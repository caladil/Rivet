//! **The generic composition** — what an addon's card looks like in a slot Aura
//! has no widget for.
//!
//! `SPEC-04` §5, and the answer to §11 (12) / `PLAN.md` §110.7 item 6. Until
//! 2026-09-02 this file drew nothing at all and nothing referenced it, so a
//! card in `tertiary` or `quaternary` was held, journalled and thrown away. A
//! shape a third party can send that nothing knows how to draw is a vocabulary
//! in name only, and that is the defect this closes.
//!
//! # What is generic here and what is not
//!
//! **The composition is generic. The appearance is Aura's.** Those are separate
//! claims and keeping them separate is the whole design:
//!
//! * *Generic* — the stack. A card is an ordered sequence of blocks (§5's own
//!   words) and this file draws them in that order, one under another, with no
//!   opinion about which shape may follow which. Nothing here knows what a bus,
//!   a forecast or a river gauge is.
//! * *Aura's* — every number. The type sizes, the gaps, the card padding, the
//!   glyphs and the 20% descender trim are `crate::widgets`' constants, which
//!   are `cpp/src/widgets.hpp`'s constants. A column is measured by
//!   `widgets::measure_column` and drawn by `widgets::draw_column` — the same
//!   two functions the weather panel uses, called rather than re-derived.
//!
//! That is what makes this **not** the composition the maintainer struck on
//! 2026-09-02. The old one sized its type as fractions of a quadrant, laid the
//! four slots out on a grid and left-aligned every line — a second visual
//! language competing with Aura's. This one places a content-sized block in a
//! corner, exactly as `DrawClockWidget` and `DrawWeatherPanel` do, so an
//! addon's card sits on the wall as a member of the same family.
//!
//! **R-125 is not touched.** Aura's own two cards keep their hard-coded
//! widgets: `primary` is `widgets::clock` and `secondary` is
//! `widgets::weather_panel`, because the cards that occupy them are Aura's and
//! their appearance is ruled fixed. This path is reached only by a slot Aura
//! has no widget for, where there is no fixed appearance to be wrong about.
//!
//! # The marks, drawn since 2026-09-02
//!
//! **Trend arrows and connection marks now draw** — `crate::icons`, and the
//! face is `MaterialSymbols-Aura.ttf`, which Aura has shipped all along. §11 (5)
//! listed *which font* as an open question; it never was one, and asking it
//! kept these marks off the screen for longer than building them would have.
//!
//! A reading that cannot be fully believed carries its mark to the **left** of
//! the value, and a reading with a trend carries its arrow to the **right** —
//! both vertically centred on the value's band. The original renderer's reading
//! card puts the mark left of the reading too; the arrow's placement is this
//! composition's own,
//! because the C++ stacks it over a unit column this composition does not have.
//!
//! **A missing face costs the marks and nothing else.** The C++'s own rule,
//! kept verbatim in behaviour: the reading still draws. That is `SPEC-04` §7
//! rule 5's lesser failure, and Aura has the scar that made it a rule — a
//! codepoint missing from the C++ atlas drew the atlas's *first* glyph instead,
//! so a reading of `6.5` appeared on the wall as `605` and nothing logged. A
//! mark that is absent is diagnosable; a mark that is wrong is not.
//!
//! **`Glyph::Clock` and `Glyph::Bus`** are in the vocabulary and in no Aura
//! source. `widgets::icon_is_drawn` says so and a column carrying one reserves
//! no width for it.

use aura_card::{Block, Card, List, Reading, Role, Statement};
use rivet_sdk::text::Target;

use crate::icons;
use crate::typeface::{GlyphInk, Typeface};
use crate::widgets::{
    self, CARD_HPAD, CARD_PAD, CardTheme, COL_GAP, Column, Corner, DESCENT_FRAC, ICON_GAP,
    LINE_GAP, Measured, STMT_GAP, STMT_SIZE, TEMP_SIZE, VAL_GAP,
};

/// **The narrowest a statement is wrapped to**, and the floor under a card's
/// whole width.
///
/// `DrawWeatherPanel`'s own `content_w.max(480.f)`. A card whose only block is
/// a sentence has nothing else to take a width from, and wrapping it to the
/// width of its longest word would produce a column one word wide.
const MIN_WRAP: f32 = 480.0;

/// **The widest a corner widget may be**, and the width prose and rows are laid
/// out to.
///
/// The panel takes its width from its own two columns and needs no ceiling; a
/// list does, because a label is elastic and would otherwise run to whatever
/// the longest one happens to be. The bound is not a taste: **two widgets share
/// a row**, each `pad` from its outer edge, and they must not meet in the
/// middle — so with `pad` between them as well, each may have half of
/// `screen_w - 3 * pad`.
///
/// The floor matters as much as the ceiling. On a small panel this goes
/// negative before it goes narrow, and a `usize` cast of a negative float is
/// the wrapping bug [`widgets::clamp_usize`] exists to catch further down.
fn room_for_a_corner(screen_w: f32, pad: f32) -> f32 {
    (3.0f32.mul_add(-pad, screen_w) / 2.0).max(MIN_WRAP)
}

/// Between two blocks in the stack.
///
/// `kStmtGap` — the gap `DrawWeatherPanel` puts between its columns and the
/// statement under them, which is the same relationship: one block to the next.
/// Reused rather than chosen, so there is no number in this file that Aura did
/// not already pick.
const BLOCK_GAP: f32 = STMT_GAP;

/// A list row, and a reading's caption.
///
/// **A `list` has no C++ counterpart at all** — Aura draws three cards and none
/// of them is one — so unlike everything else here these are a judgement rather
/// than a port, and it is made by reusing Aura's nearest existing size rather
/// than by picking a new one.
///
/// **`kStmtSize` and not `kSecSize`, and the first attempt had it wrong.** A
/// row looks like a small value, so 62 px was the obvious choice; drawn, it was
/// plainly not. A row is three things across — lead, label, trail — where a
/// reading's second line is one, so at 62 px the label is the only elastic
/// column and it collapses: `"Ann Arbor via State"` came out as `"Ann A…"` with
/// half the card empty. A row is read the way the statement under the panel is
/// read, and it is set the same size.
const ROW_SIZE: f32 = STMT_SIZE;
const CAPTION_SIZE: f32 = STMT_SIZE;

/// **Aura's corner reading card — the headline — ported constant for
/// constant** from the original renderer's reading card in
/// `cpp/src/widgets.hpp`.
///
/// A reading that carries BOTH a unit and a trend is that card: one number read
/// from across the room, with the arrow stacked over the unit in a column to
/// its right. It is kept separate from the ordinary reading above for the
/// reason the C++ keeps its reading card separate from `DrawWeatherPanel` —
/// the geometry is not the panel's, and folding them would make one of the two
/// wrong. The maintainer ruled it 2026-09-02: *match the original Aura.*
///
/// The comment beside `kValueSize` says why it is not `TEMP_SIZE`: *"The
/// reading is half again the weather panel's temperature: this card carries one
/// number and is read from across the room."*
const HEADLINE_VALUE_SIZE: f32 = 144.0;
/// `kUnitSize` — *"matches the forecast statement"*.
const HEADLINE_UNIT_SIZE: f32 = 44.0;
/// `kGapCol` — reading to the arrow/unit column.
const HEADLINE_GAP_COL: f32 = 20.0;
/// `kGapStack` — arrow INK to unit INK, not box to box. The two boxes together
/// carry about a third of their height in padding, which is the gap that was
/// actually being seen.
const HEADLINE_GAP_STACK: f32 = 4.0;

/// A row's lead token to its label — `kIconGap`, the same gap a glyph gets from
/// the value beside it.
const LEAD_GAP: f32 = ICON_GAP;

/// A row's label to its trailing token — `kColGap`, which is what Aura puts
/// between two things that are read as separate columns.
const TRAIL_GAP: f32 = COL_GAP;

/// Between two rows of a list, and between a list's caption and its first row —
/// `kValGap`, the gap between a value and the second line under it, which is the
/// same relationship: two lines of one stack rather than two blocks.
const ROW_GAP: f32 = VAL_GAP;

/// **Draw one card, corner-anchored and content-sized.**
///
/// Returns `false` when the card produced no picture, which is not the same as
/// an empty card: a card whose every block is a shape this build cannot draw —
/// a lone `pair` with two blank sides, say — measures to nothing, and the
/// caller journals that rather than the caller assuming it drew.
///
/// # Two passes, and why it cannot be one
///
/// A statement is wrapped by the renderer, so its width is an output of the
/// layout and not an input to it. The panel has the same problem and solves it
/// the same way: measure everything that has an intrinsic width first, wrap the
/// prose to that, and only then is the block's own width known.
#[allow(clippy::too_many_arguments)]
pub fn card(
    canvas: &mut [u8],
    t: &Target,
    bold: &Typeface,
    regular: &Typeface,
    // **`Option`, and that is the whole of the missing-asset policy.** `None`
    // is a machine where the icon face did not land; every reading still draws
    // and only its mark is absent (`crate::icons`' module note).
    icon_face: Option<&Typeface>,
    card: &Card,
    corner: Corner,
    screen_w: f32,
    screen_h: f32,
    grey: [u8; 3],
    pad: f32,
    theme: Option<&CardTheme>,
) -> bool {
    let pieces = measure(bold, regular, icon_face, card, room_for_a_corner(screen_w, pad));
    if pieces.is_empty() {
        return false;
    }

    let block_w = pieces.iter().map(Piece::width).fold(0.0_f32, f32::max);
    let block_h = pieces.iter().map(Piece::height).sum::<f32>()
        + (pieces.len() as f32 - 1.0) * BLOCK_GAP;
    if block_w <= 0.0 || block_h <= 0.0 {
        return false;
    }

    let (ox, oy) = corner.origin(screen_w, screen_h, block_w, block_h, pad);

    if let Some(theme) = theme {
        // The rainbow card is a six-stop gradient with circular corners and is
        // **Including the rainbow one, since 2026-09-02.** It was skipped
        // while the SDK had no gradient, and `weather_panel` skipped it too —
        // identically, because two widgets disagreeing about whether there is
        // a card behind them is worse than neither having one. They agree
        // again now, and they draw it.
        let (cx, cy) = (ox - CARD_HPAD, oy - CARD_PAD);
        let (cw, ch) = (
            2.0f32.mul_add(CARD_HPAD, block_w),
            2.0f32.mul_add(CARD_PAD, block_h),
        );
        if theme.rainbow {
            widgets::rainbow_card(canvas, t, cx, cy, cw, ch, widgets::RAINBOW_ALPHA);
        } else {
            widgets::rounded_card(canvas, t, cx, cy, cw, ch, theme.bg, theme.bg_alpha);
        }
    }

    // Each block is centred in the card's width, which is `DrawWeatherPanel`'s
    // rule for its columns and its statement alike: a block narrower than its
    // neighbour sits in the middle rather than against an edge.
    let mut y = oy;
    for piece in &pieces {
        let x = (block_w - piece.width()).mul_add(0.5, ox);
        piece.draw(canvas, t, bold, regular, icon_face, x, y, grey, theme);
        y += piece.height() + BLOCK_GAP;
    }
    true
}

/// **The band the digits' INK actually occupies**, which is what an icon beside
/// them has to be centred on.
///
/// `widgets::draw_column` positions the value by its **line box**, because that
/// is what `DrawWeatherPanel` does. An icon centred on that box lands high, and
/// visibly so: a line box carries the font's ascent gap above the digits, and
/// digits have no descenders to balance it below. The first two builds of this
/// centred the box and both looked wrong in `target/generic-card.ppm`.
///
/// So the value's ink band is computed the way the original renderer's reading
/// card computes everything — from the glyph ink — and the icons are centred on
/// THAT.
/// Returns `(top, height)` in the same space the caller draws in.
fn value_ink_band(bold: &Typeface, column: &Column, m: Measured, y: f32) -> (f32, f32) {
    // The line box `draw_column` will put the value in.
    let line_top = (m.top_h - m.values_h).mul_add(0.5, y);
    let sample = column
        .value
        .as_deref()
        .and_then(|v| v.chars().find(char::is_ascii_digit))
        // A value that is not a number at all - a `pair` side spelled in words
        // - still has a band, and `0` is a fair stand-in for its height.
        .unwrap_or('0');
    let ink = bold.ink(sample, TEMP_SIZE);
    (line_top + ink.top, ink.height())
}

/// **Where to draw an icon so its INK is centred in a band** — and the C++'s
/// own formula, because the obvious one is visibly wrong.
///
/// The original renderer's reading card:
///
/// ```text
/// mark_y = origin.y + (block_h - mark_h) * 0.5f - mark_ink.top
/// arrow_x = x + (stack_w - arrow_w) * 0.5f - arrow_ink.left
/// ```
///
/// Centring the LINE BOX instead puts the glyph high, and by a lot: a Material
/// Symbols outline occupies rather less than its em, so the box carries the
/// difference as padding at the top. The first build of this centred the box,
/// and the mark and the arrow both floated above the digits they belong to —
/// visible immediately in `target/generic-card.ppm`, which is the whole reason
/// that file is written.
///
/// The horizontal pull-back is the same correction on the other axis: the ink
/// starts at the glyph's left bearing, so a draw origin that ignores it puts
/// the ink right of where it was measured to go.
fn icon_origin(face: &Typeface, glyph: char, px: f32, x: f32, band_top: f32, band: f32) -> (f32, f32) {
    let ink = face.ink(glyph, px);
    (
        x - ink.left,
        (band - ink.height()).mul_add(0.5, band_top) - ink.top,
    )
}

/// One block, measured and ready to draw.
///
/// The measurement and the drawing are deliberately the same value: a block
/// that measured one way and drew another is how a stack drifts, and the panel
/// has the same shape for the same reason.
enum Piece {
    /// A `reading` — its mark, glyph, value, second line, trend arrow, and a
    /// caption under them.
    Reading {
        column: Column,
        m: Measured,
        caption: Vec<String>,
        caption_w: f32,
        role: Role,
        /// `SPEC-04` §7 rule 4's mark, and the role it is drawn in. `None`
        /// when the reading is `ok` or when no icon face landed.
        mark: Option<(char, Role)>,
        mark_w: f32,
        /// §5.5's arrow. `None` when the producer sent no trend, or no face.
        arrow: Option<char>,
        arrow_w: f32,
    },
    /// **Aura's corner reading card, the headline** — a big number with the
    /// trend arrow stacked over a much smaller unit, in a column to its right:
    /// the original renderer's reading card.
    ///
    /// Every measurement here is of INK rather than of a line box, because that
    /// is what the C++ does and for the reason it gives: a line box reserves
    /// ascender and descender room the digits never use, so a card sized by it
    /// carries a band of empty space above the reading that no padding can
    /// remove.
    Headline {
        value: String,
        value_w: f32,
        /// `'0'` stands in for the digits, which are all one height in this
        /// face.
        value_ink: GlyphInk,
        unit: String,
        unit_w: f32,
        /// `'d'` stands in for the unit: an ascender, so its top is the highest
        /// ink the string can reach and its bottom is the baseline.
        unit_ink: GlyphInk,
        arrow: Option<(char, GlyphInk)>,
        mark: Option<(char, Role, GlyphInk)>,
        stack_w: f32,
        block_w: f32,
        block_h: f32,
        /// The number's own role. `Role::Alarm` is how the producer carries
        /// `out_of_range`, and out of range recolours **the number and its
        /// arrow, and nothing else** — the unit stays in the text colour.
        role: Role,
    },
    /// A `pair` — two columns and the rule between them. §5.8's shape, at
    /// Aura's constants, without the panel's statement (which is its own block
    /// here, because in this vocabulary it always was).
    Pair {
        left: Column,
        ml: Measured,
        right: Column,
        mr: Measured,
        content_w: f32,
        content_h: f32,
    },
    /// A `statement` — wrapped prose, centred.
    Statement { lines: Vec<String>, w: f32, role: Role },
    /// A `list` — a caption over rows of lead / label / trail.
    List {
        caption: Option<String>,
        caption_w: f32,
        rows: Vec<LaidRow>,
        lead_w: f32,
        trail_w: f32,
        label_w: f32,
    },
}

/// One row of a list, with its label already cut to the room it has.
struct LaidRow {
    lead: Option<String>,
    label: String,
    label_w: f32,
    trail: Option<String>,
    trail_w: f32,
    role: Role,
}

impl Piece {
    fn width(&self) -> f32 {
        match self {
            Self::Reading {
                m,
                caption_w,
                mark_w,
                arrow_w,
                ..
            } => (mark_w + m.width + arrow_w).max(*caption_w),
            Self::Headline { block_w, .. } => *block_w,
            Self::Pair { content_w, .. } => *content_w,
            Self::Statement { w, .. } => *w,
            Self::List {
                caption_w,
                lead_w,
                trail_w,
                label_w,
                ..
            } => (lead_w + label_w + trail_w).max(*caption_w),
        }
    }

    fn height(&self) -> f32 {
        match self {
            Self::Reading {
                m, caption, mark, ..
            } => {
                // A mark with no reading beside it still occupies a band, or a
                // `connecting` card would measure to nothing and be skipped.
                let band = if m.present {
                    m.top_h
                } else if mark.is_some() {
                    TEMP_SIZE * icons::MARK_FRACTION
                } else {
                    0.0
                };
                band + stacked(caption.len(), CAPTION_SIZE, true)
            }
            Self::Headline { block_h, .. } => *block_h,
            Self::Pair { content_h, .. } => *content_h,
            Self::Statement { lines, .. } => stacked(lines.len(), STMT_SIZE, false),
            Self::List { caption, rows, .. } => {
                let head = if caption.is_some() {
                    CAPTION_SIZE * (1.0 - DESCENT_FRAC) + ROW_GAP
                } else {
                    0.0
                };
                head + stacked(rows.len(), ROW_SIZE, false)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        canvas: &mut [u8],
        t: &Target,
        bold: &Typeface,
        regular: &Typeface,
        icon_face: Option<&Typeface>,
        x: f32,
        y: f32,
        grey: [u8; 3],
        theme: Option<&CardTheme>,
    ) {
        let ink = |role: Role| widgets::role_ink(theme, grey, role);
        match self {
            Self::Headline {
                value,
                value_w,
                value_ink,
                unit,
                unit_w,
                unit_ink,
                arrow,
                mark,
                stack_w,
                block_h,
                role,
                ..
            } => {
                let Some(face) = icon_face else {
                    return;
                };
                // Out of range recolours the number and its arrow, and nothing
                // else — the unit stays in the text colour.
                let (value_col, _) = ink(*role);
                let (text_col, _) = ink(Role::Normal);
                let mut x = x;

                if let Some((glyph, mark_role, m_ink)) = mark {
                    // The alert is red because something is wrong; the question
                    // is amber because nothing is, yet.
                    let (mark_col, _) = ink(*mark_role);
                    let mark_y = (block_h - m_ink.height()).mul_add(0.5, y) - m_ink.top;
                    let px = HEADLINE_VALUE_SIZE * icons::MARK_FRACTION;
                    face.draw(
                        canvas,
                        t,
                        x - m_ink.left,
                        mark_y,
                        &glyph.to_string(),
                        px,
                        mark_col,
                        255,
                    );
                    x += m_ink.width() + icons::MARK_GAP;
                }

                // `value_y` is where the digits' INK starts; the draw origin is
                // pulled back by the font's ascent so the two agree.
                let value_y = (block_h - value_ink.height()).mul_add(0.5, y);
                bold.draw(
                    canvas,
                    t,
                    x,
                    value_y - value_ink.top,
                    value,
                    HEADLINE_VALUE_SIZE,
                    value_col,
                    255,
                );
                x += value_w + HEADLINE_GAP_COL;

                // The unit sits on the digits' bottom edge; the arrow rides
                // directly above it, the two centred on one vertical axis.
                let unit_y = value_y + value_ink.height() - unit_ink.bottom;
                regular.draw(
                    canvas,
                    t,
                    (stack_w - unit_w).mul_add(0.5, x),
                    unit_y,
                    unit,
                    HEADLINE_UNIT_SIZE,
                    text_col,
                    255,
                );

                if let Some((glyph, a_ink)) = arrow {
                    // Vertically: the arrow's INK sits `HEADLINE_GAP_STACK` above the
                    // unit's ink, rather than its box above the unit's box.
                    // Horizontally: the ink is centred in the column, so the
                    // draw origin is pulled back by the glyph's left bearing.
                    let arrow_x = (stack_w - a_ink.width()).mul_add(0.5, x) - a_ink.left;
                    let arrow_y = unit_y + unit_ink.top - HEADLINE_GAP_STACK - a_ink.bottom;
                    face.draw(
                        canvas,
                        t,
                        arrow_x,
                        arrow_y,
                        &glyph.to_string(),
                        HEADLINE_VALUE_SIZE * icons::ARROW_FRACTION,
                        value_col,
                        255,
                    );
                }
            }
            Self::Reading {
                column,
                m,
                caption,
                caption_w,
                role,
                mark,
                mark_w,
                arrow,
                arrow_w,
            } => {
                let (colour, _) = ink(*role);
                // The mark, the reading and the arrow are one band, centred
                // together — so a reading that gains a mark does not shift on
                // the screen relative to a reading that has none.
                let band = mark_w + m.width + arrow_w;
                let left = (self.width() - band).mul_add(0.5, x);

                // **The mark, to the LEFT** — the original reading card's
                // arrangement.
                // Drawn in its OWN role rather than the reading's: amber for a
                // question, red for a fault, whatever colour the number is.
                if let (Some((glyph, mark_role)), Some(face)) = (mark, icon_face) {
                    let px = TEMP_SIZE * icons::MARK_FRACTION;
                    let (mark_colour, _) = ink(*mark_role);
                    let (band_top, band) = if m.present {
                        value_ink_band(bold, column, *m, y)
                    } else {
                        // Nothing to align to: the mark IS the reading, which
                        // is §7 rule 4's *instead of a value*.
                        (y, px)
                    };
                    let (gx, gy) = icon_origin(face, *glyph, px, left, band_top, band);
                    face.draw(canvas, t, gx, gy, &glyph.to_string(), px, mark_colour, 255);
                }

                // The glyph, the value and the second line, drawn by the same
                // function the weather panel's columns go through.
                let cx = left + mark_w;
                widgets::draw_column(canvas, t, bold, column, *m, cx, y, colour);

                // **The arrow, to the RIGHT**, in the reading's own colour —
                // a trend is a fact about the number, not a separate warning.
                if let (Some(glyph), Some(face)) = (arrow, icon_face) {
                    let px = TEMP_SIZE * icons::ARROW_FRACTION;
                    let (band_top, band) = value_ink_band(bold, column, *m, y);
                    let (gx, gy) = icon_origin(
                        face,
                        *glyph,
                        px,
                        cx + m.width + icons::MARK_GAP,
                        band_top,
                        band,
                    );
                    face.draw(canvas, t, gx, gy, &glyph.to_string(), px, colour, 255);
                }
                // The caption under them, centred on the reading, in the muted
                // role — `Reading::caption` is the clock's date and the
                // weather's condition text, both asides.
                let (cap_colour, cap_alpha) = ink(Role::Muted);
                let mut cy = y + m.top_h + BLOCK_GAP.min(STMT_GAP);
                for line in caption {
                    let lw = regular.measure(line, CAPTION_SIZE).0;
                    regular.draw(
                        canvas,
                        t,
                        (caption_w - lw).mul_add(0.5, (self.width() - caption_w).mul_add(0.5, x)),
                        cy,
                        line,
                        CAPTION_SIZE,
                        cap_colour,
                        cap_alpha,
                    );
                    cy += CAPTION_SIZE + LINE_GAP;
                }
            }
            Self::Pair {
                left,
                ml,
                right,
                mr,
                content_h,
                ..
            } => {
                let (colour, _) = ink(Role::Normal);
                let mut cx = x;
                if ml.present {
                    widgets::draw_column(canvas, t, bold, left, *ml, cx, y, colour);
                    cx += ml.width;
                }
                if ml.present && mr.present {
                    let dx = cx + widgets::COL_GAP;
                    // The rule, at the muted role — §5.1 says in as many words
                    // that `DrawWeatherPanel`'s divider is this role by another
                    // name, so it is drawn from the role rather than from a
                    // second copy of `text_col.a * 0.35f`.
                    let (rule, alpha) = ink(Role::Muted);
                    rivet_sdk::draw::fill_rounded_alpha(
                        canvas,
                        t,
                        widgets::clamp_usize(widgets::trunc(dx)),
                        widgets::clamp_usize(widgets::trunc(content_h.mul_add(0.06, y))),
                        widgets::clamp_usize(widgets::trunc(widgets::DIVIDER_W)),
                        widgets::clamp_usize(widgets::trunc(content_h * 0.88)),
                        0,
                        rule,
                        alpha,
                    );
                    cx = dx + widgets::DIVIDER_W + widgets::COL_GAP;
                }
                if mr.present {
                    widgets::draw_column(canvas, t, bold, right, *mr, cx, y, colour);
                }
            }
            Self::Statement { lines, w, role } => {
                let (colour, alpha) = ink(*role);
                let mut ly = y;
                for line in lines {
                    let lw = regular.measure(line, STMT_SIZE).0;
                    regular.draw(
                        canvas,
                        t,
                        (w - lw).mul_add(0.5, x),
                        ly,
                        line,
                        STMT_SIZE,
                        colour,
                        alpha,
                    );
                    ly += STMT_SIZE + LINE_GAP;
                }
            }
            Self::List {
                caption,
                rows,
                lead_w,
                trail_w,
                label_w,
                ..
            } => {
                let row_w = lead_w + label_w + trail_w;
                let mut ly = y;
                if let Some(head) = caption {
                    let (colour, alpha) = ink(Role::Muted);
                    regular.draw(canvas, t, x, ly, head, CAPTION_SIZE, colour, alpha);
                    ly += CAPTION_SIZE * (1.0 - DESCENT_FRAC) + ROW_GAP;
                }
                for row in rows {
                    let (colour, alpha) = ink(row.role);
                    if let Some(lead) = &row.lead {
                        regular.draw(canvas, t, x, ly, lead, ROW_SIZE, colour, alpha);
                    }
                    regular.draw(
                        canvas,
                        t,
                        x + lead_w,
                        ly,
                        &row.label,
                        ROW_SIZE,
                        colour,
                        alpha,
                    );
                    if let Some(trail) = &row.trail {
                        // Down the right edge of the row, so a column of
                        // countdowns lines up whatever the labels do.
                        regular.draw(
                            canvas,
                            t,
                            x + row_w - row.trail_w,
                            ly,
                            trail,
                            ROW_SIZE,
                            colour,
                            alpha,
                        );
                    }
                    ly += ROW_SIZE * (1.0 - DESCENT_FRAC) + ROW_GAP;
                }
            }
        }
    }
}

/// `n` lines of `px`, with `kLineGap` between them.
///
/// `trimmed` takes the 20% descender space off, which is right for a stack of
/// digits and wrong for prose — `kDescentFrac`'s own rule. A statement has
/// descenders and a caption under a reading is measured as part of a stack, so
/// the two callers differ.
fn stacked(n: usize, px: f32, trimmed: bool) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let line = if trimmed { px * (1.0 - DESCENT_FRAC) } else { px };
    let gap = if trimmed { BLOCK_GAP.min(STMT_GAP) } else { 0.0 };
    gap + (n as f32).mul_add(line, (n as f32 - 1.0) * LINE_GAP)
}

/// Measure every block, in two passes over the card.
///
/// `room` is what [`room_for_a_corner`] allows. It is a **ceiling on the
/// elastic blocks and not on the card**: a `pair` of two wide readings measures
/// wider than `room` and is drawn at its own width, because a column of digits
/// has an intrinsic size and clipping it would be worse than overhanging. Prose
/// and labels have no intrinsic size, so they get the ceiling.
fn measure(
    bold: &Typeface,
    regular: &Typeface,
    icon_face: Option<&Typeface>,
    card: &Card,
    room: f32,
) -> Vec<Piece> {
    // --- pass one: everything with an intrinsic width ----------------------
    let mut intrinsic = 0.0_f32;
    for block in &card.blocks {
        let w = match block {
            Block::Reading(r) => widgets::measure_column(bold, &Column::from_reading(r)).width,
            Block::Pair(p) => {
                let ml = widgets::measure_column(bold, &Column::from_reading(&p.left));
                let mr = widgets::measure_column(bold, &Column::from_reading(&p.right));
                pair_width(ml, mr)
            }
            // A list's labels are cut to the width rather than setting it, and
            // a statement is wrapped to it. Neither contributes.
            Block::List(_) | Block::Statement(_) => 0.0,
        };
        intrinsic = intrinsic.max(w);
    }
    // Prose is wrapped to the room a corner has, or to the widest thing that
    // was already going to set the card's width — whichever is larger, so a
    // wide `pair` does not leave the sentence under it wrapped to a narrower
    // column than the card it sits on.
    let wrap_w = intrinsic.max(room);

    // --- pass two: wrap, cut, and lay out ----------------------------------
    let mut pieces = Vec::new();
    for block in &card.blocks {
        match block {
            Block::Reading(r) => {
                if let Some(piece) = reading(bold, regular, icon_face, r, wrap_w) {
                    pieces.push(piece);
                }
            }
            Block::Pair(p) => {
                let left = Column::from_reading(&p.left);
                let right = Column::from_reading(&p.right);
                let ml = widgets::measure_column(bold, &left);
                let mr = widgets::measure_column(bold, &right);
                // **Both sides blank is a pair that says nothing**, and §5.8
                // admits it: a side with nothing to report spells it with its
                // `state`, and a `connecting` reading carries no value and no
                // glyph. Skipped rather than drawn as a lone rule.
                if ml.present || mr.present {
                    pieces.push(Piece::Pair {
                        content_w: pair_width(ml, mr),
                        content_h: ml.top_h.max(mr.top_h),
                        left,
                        ml,
                        right,
                        mr,
                    });
                }
            }
            Block::Statement(s) => {
                if let Some(piece) = statement(regular, s, wrap_w) {
                    pieces.push(piece);
                }
            }
            Block::List(l) => {
                if let Some(piece) = list(regular, l, wrap_w) {
                    pieces.push(piece);
                }
            }
        }
    }
    pieces
}

/// The panel's own content width: two columns, `kColGap` either side of the
/// rule, or one column alone.
fn pair_width(ml: Measured, mr: Measured) -> f32 {
    let both = ml.present && mr.present;
    (if ml.present { ml.width } else { 0.0 })
        + (if mr.present { mr.width } else { 0.0 })
        + if both {
            2.0f32.mul_add(widgets::COL_GAP, widgets::DIVIDER_W)
        } else {
            0.0
        }
}

/// **The corner reading card's measurement** — the original renderer's reading
/// card's `--- Measure ---` block.
///
/// `None` when this reading is not that card, which sends it to [`reading`] and
/// the ordinary composition instead.
fn headline(
    bold: &Typeface,
    regular: &Typeface,
    icon_face: Option<&Typeface>,
    r: &Reading,
) -> Option<Piece> {
    // **A value, a unit and a trend together are the card.** The weather
    // panel's readings carry a unit and no trend and live inside a `Pair`; the
    // vitals producers send no unit on a `reading` that has an arrow. So this
    // is the shape the original reading card draws, and nothing else reaches
    // it.
    let (value, unit, trend) = (r.value.as_ref()?, r.unit.as_ref()?, r.trend?);
    if value.is_empty() || unit.is_empty() {
        return None;
    }
    // No icon face is no arrow, and no arrow is not this card — the ordinary
    // composition still draws the number. `SPEC-04` §7 rule 5's lesser failure.
    let face = icon_face?;

    let value_ink = bold.ink('0', HEADLINE_VALUE_SIZE);
    let unit_ink = regular.ink('d', HEADLINE_UNIT_SIZE);
    let value_w = bold.measure(value, HEADLINE_VALUE_SIZE).0;
    let unit_w = regular.measure(unit, HEADLINE_UNIT_SIZE).0;

    let arrow_glyph = icons::arrow(trend);
    let arrow_ink = face.ink(arrow_glyph, HEADLINE_VALUE_SIZE * icons::ARROW_FRACTION);
    let arrow_w = arrow_ink.width();

    let mark = icons::mark(r.state)
        .map(|(glyph, role)| (glyph, role, face.ink(glyph, HEADLINE_VALUE_SIZE * icons::MARK_FRACTION)));
    let mark_w = mark.map_or(0.0, |(_, _, ink)| ink.width());

    // The second column: the arrow stacked over the unit, sharing a centre
    // line. It is the centre of their INK, not of their line boxes — the arrow
    // glyph carries more side bearing than an `m`, so centring the boxes would
    // leave the arrow visibly off-axis.
    let stack_w = arrow_w.max(unit_w);
    let stack_h = arrow_ink.height() + HEADLINE_GAP_STACK + unit_ink.height();

    let block_w = if mark.is_some() {
        mark_w + icons::MARK_GAP
    } else {
        0.0
    } + value_w
        + HEADLINE_GAP_COL
        + stack_w;
    let block_h = value_ink
        .height()
        .max(stack_h)
        .max(mark.map_or(0.0, |(_, _, ink)| ink.height()));

    Some(Piece::Headline {
        value: value.clone(),
        value_w,
        value_ink,
        unit: unit.clone(),
        unit_w,
        unit_ink,
        arrow: Some((arrow_glyph, arrow_ink)),
        mark,
        stack_w,
        block_w,
        block_h,
        role: r.role,
    })
}

fn reading(
    bold: &Typeface,
    regular: &Typeface,
    icon_face: Option<&Typeface>,
    r: &Reading,
    wrap_w: f32,
) -> Option<Piece> {
    // Aura's own card first; everything else takes the generic composition.
    if let Some(piece) = headline(bold, regular, icon_face, r) {
        return Some(piece);
    }
    let column = Column::from_reading(r);
    let m = widgets::measure_column(bold, &column);

    // **No face, no mark, and no width reserved for one** — the same rule
    // `measure_column` already applies to a glyph it cannot draw: reserving
    // room for something absent leaves the reading floating in space.
    // **The INK width, not the advance.** An icon's advance carries side
    // bearing that the drawing above pulls back out again, so measuring the
    // advance would reserve a band wider than the mark visibly occupies and
    // push the value off centre. The same reasoning `measure_column` applies to
    // a glyph it cannot draw, one step finer.
    let advance = |face: &Typeface, glyph: char, px: f32| face.ink(glyph, px).width();
    let mark = icon_face.and(icons::mark(r.state));
    let mark_w = match (mark, icon_face) {
        (Some((glyph, _)), Some(face)) => {
            advance(face, glyph, TEMP_SIZE * icons::MARK_FRACTION) + icons::MARK_GAP
        }
        _ => 0.0,
    };
    let arrow = icon_face.and(r.trend.map(icons::arrow));
    let arrow_w = match (arrow, icon_face) {
        (Some(glyph), Some(face)) => {
            icons::MARK_GAP + advance(face, glyph, TEMP_SIZE * icons::ARROW_FRACTION)
        }
        _ => 0.0,
    };
    let caption = r
        .caption
        .as_deref()
        .map(|text| widgets::wrap_text(regular, text, CAPTION_SIZE, wrap_w))
        .unwrap_or_default();
    // Nothing to draw and nothing to say about it: a reading whose state admits
    // no value, whose glyph is one of the two that are not ported, and which
    // carries no caption either.
    // A reading with a MARK has something to draw even with no value: that is
    // exactly what §7 rule 4 means by a mark drawn *instead of* a value for
    // `connecting` and `unavailable`.
    if !m.present && caption.is_empty() && mark.is_none() {
        return None;
    }
    let caption_w = caption
        .iter()
        .map(|line| regular.measure(line, CAPTION_SIZE).0)
        .fold(0.0_f32, f32::max);
    Some(Piece::Reading {
        column,
        m,
        caption,
        caption_w,
        role: r.role,
        mark,
        mark_w,
        arrow,
        arrow_w,
    })
}

fn statement(regular: &Typeface, s: &Statement, wrap_w: f32) -> Option<Piece> {
    if s.text.trim().is_empty() {
        return None;
    }
    let lines = widgets::wrap_text(regular, &s.text, STMT_SIZE, wrap_w);
    if lines.is_empty() {
        return None;
    }
    let w = lines
        .iter()
        .map(|line| regular.measure(line, STMT_SIZE).0)
        .fold(0.0_f32, f32::max);
    Some(Piece::Statement {
        lines,
        w,
        role: s.role,
    })
}

/// **Nothing here knows what a bus is.** Read this function and then read
/// `aura_card::Row`: neither says route, destination or minutes, which is the
/// property `aura_card::tests::a_list_is_not_bus_shaped` asserts from the other
/// side. The same code draws a departure board and a day's agenda.
fn list(regular: &Typeface, l: &List, wrap_w: f32) -> Option<Piece> {
    if l.rows.is_empty() && l.caption.is_none() {
        return None;
    }
    // One lead column as wide as the widest lead, so the labels line up; and
    // one trail column, for the same reason on the other edge. A list where no
    // row has a lead draws no left column at all — `Row::lead`'s own rule.
    let widest = |pick: fn(&aura_card::Row) -> Option<&str>, gap: f32| -> f32 {
        let w = l
            .rows
            .iter()
            .filter_map(pick)
            .map(|s| regular.measure(s, ROW_SIZE).0)
            .fold(0.0_f32, f32::max);
        if w > 0.0 { w + gap } else { 0.0 }
    };
    let lead_w = widest(|r| r.lead.as_deref(), LEAD_GAP);
    let trail_w = widest(|r| r.trail.as_deref(), TRAIL_GAP);

    // The room a label has is what is left of the wrap width once the two
    // token columns have taken theirs. **Cut here and not by the producer**,
    // for the reason `wrap_text` lives in the renderer: the width is the
    // renderer's fact and nobody else's.
    let room = (wrap_w - lead_w - trail_w).max(MIN_WRAP * 0.25);
    let rows: Vec<LaidRow> = l
        .rows
        .iter()
        .map(|row| {
            let label = truncate(regular, &row.label, ROW_SIZE, room);
            LaidRow {
                label_w: regular.measure(&label, ROW_SIZE).0,
                label,
                lead: row.lead.clone(),
                trail: row.trail.clone(),
                trail_w: row
                    .trail
                    .as_deref()
                    .map_or(0.0, |s| regular.measure(s, ROW_SIZE).0),
                role: row.role,
            }
        })
        .collect();

    let label_w = rows.iter().map(|r| r.label_w).fold(0.0_f32, f32::max);
    let caption_w = l
        .caption
        .as_deref()
        .map_or(0.0, |c| regular.measure(c, CAPTION_SIZE).0);
    Some(Piece::List {
        caption: l.caption.clone(),
        caption_w,
        rows,
        lead_w,
        trail_w,
        label_w,
    })
}

/// Cut a label to fit, with an ellipsis so a reader can see that it was cut.
///
/// `…` is U+2026 and Source Code Pro has it. If it did not, this would draw
/// nothing at all rather than the wrong glyph — which is the failure mode the
/// module note is about, and the cheaper half of it: an ellipsis that is
/// missing costs a reader the knowledge that a label was cut, and costs nothing
/// else.
fn truncate(face: &Typeface, text: &str, px: f32, max_w: f32) -> String {
    if face.measure(text, px).0 <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let candidate = format!("{out}{ch}…");
        if face.measure(&candidate, px).0 > max_w {
            break;
        }
        out.push(ch);
    }
    format!("{out}…")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use aura_card::{Glyph, Pair, Row, Slot, State};


    const BOLD: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/SourceCodePro-Bold.ttf"
    );
    const REGULAR: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/SourceCodePro-Regular.ttf"
    );

    /// The icon face, or `None` on a tree where it did not land — which is a
    /// state these tests must tolerate, because it is a state a MACHINE is
    /// allowed to be in (`crate::icons`' missing-asset rule).
    fn icon_face() -> Option<Typeface> {
        Typeface::load(
            concat!(env!("CARGO_MANIFEST_DIR"), "/assets/MaterialSymbols-Aura.ttf"),
            crate::icons::BASE_SIZE,
        )
        .ok()
    }

    fn faces() -> (Typeface, Typeface) {
        (
            Typeface::load(BOLD, widgets::CLOCK_FONT_SIZE).expect("the bold face"),
            Typeface::load(REGULAR, widgets::REGULAR_FONT_SIZE).expect("the regular face"),
        )
    }

    fn canvas(w: usize, h: usize) -> (Vec<u8>, Target) {
        (
            vec![0u8; w * h * 4],
            Target::xrgb(w, h, w * 4),
        )
    }

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

    fn reading_block(value: &str, unit: &str, caption: Option<&str>) -> Block {
        Block::Reading(Reading {
            glyph: None,
            value: Some(value.to_string()),
            unit: Some(unit.to_string()),
            caption: caption.map(str::to_string),
            ..Reading::default()
        })
    }

    /// A departure board, which is the case §5.3 names and which Aura has no
    /// widget for at all.
    fn departures() -> Card {
        Card {
            slot: Slot::Tertiary,
            ttl_seconds: 120,
            blocks: vec![Block::List(List {
                caption: Some("State & Packard".to_string()),
                glyph: None,
                rows: vec![
                    Row {
                        lead: Some("N4".to_string()),
                        label: "Ann Arbor via State".to_string(),
                        trail: Some("2 min".to_string()),
                        role: Role::Warn,
                    },
                    Row {
                        lead: Some("N6".to_string()),
                        label: "Ypsilanti Transit Center".to_string(),
                        trail: Some("11 min".to_string()),
                        role: Role::Normal,
                    },
                ],
            })],
        }
    }

    fn drew(card: &Card) -> Option<(usize, usize, usize, usize)> {
        let (bold, regular) = faces();
        let (mut c, t) = canvas(1920, 1080);
        let drawn = super::card(
            &mut c,
            &t,
            &bold,
            &regular,
            icon_face().as_ref(),
            card,
            Corner::TopLeft,
            1920.0,
            1080.0,
            widgets::widget_grey(60),
            96.0,
            None,
        );
        let bounds = ink_bounds(&c, &t);
        assert_eq!(
            drawn,
            bounds.is_some(),
            "the return value disagreed with the canvas: said {drawn}, drew {}",
            bounds.is_some()
        );
        bounds
    }

    /// **The defect this file was rewritten to close.** A card in a slot Aura
    /// has no widget for used to be held, journalled and thrown away; it has to
    /// put ink on the screen.
    #[test]
    fn an_addons_card_in_a_slot_aura_has_no_widget_for_is_actually_drawn() {
        let bounds = drew(&departures()).expect("a departure board drew nothing at all");
        let (x0, y0, x1, y1) = bounds;
        assert!(
            x1 - x0 > 300 && y1 - y0 > 100,
            "implausibly small for a two-row board with a caption: {bounds:?}"
        );
    }

    /// Every one of the four shapes draws on its own, so no shape is a silent
    /// hole in the vocabulary. `pair` is the one that used to return without
    /// moving the pen.
    #[test]
    fn each_of_the_four_shapes_draws_something_by_itself() {
        let shapes: [(&str, Block); 4] = [
            ("reading", reading_block("6.5", " m", Some("ten minutes ago"))),
            (
                "statement",
                Block::Statement(Statement {
                    text: "The sensor has not reported since 14:02".to_string(),
                    role: Role::Warn,
                }),
            ),
            ("list", departures().blocks.into_iter().next().unwrap()),
            (
                "pair",
                Block::Pair(Pair {
                    left: Reading {
                        glyph: Some(Glyph::Cloudy),
                        value: Some("70".to_string()),
                        unit: Some("°F".to_string()),
                        ..Reading::default()
                    },
                    right: Reading {
                        glyph: Some(Glyph::Indoor),
                        value: Some("76".to_string()),
                        unit: Some("°F".to_string()),
                        ..Reading::default()
                    },
                }),
            ),
        ];
        for (name, block) in shapes {
            let card = Card {
                slot: Slot::Tertiary,
                ttl_seconds: 60,
                blocks: vec![block],
            };
            assert!(
                drew(&card).is_some(),
                "`{name}` alone on a card drew nothing, so it is a hole in the vocabulary"
            );
        }
    }

    /// **A `pair` moves the pen now.** The old file's `Block::Pair` arm
    /// returned the pen unchanged, so a card with a pair above a statement drew
    /// the statement on top of nothing and the two occupied one line's worth of
    /// stack. The pair must add its own height.
    #[test]
    fn a_pair_takes_room_in_the_stack_instead_of_being_skipped() {
        let statement = Block::Statement(Statement {
            text: "Chance Rain Showers".to_string(),
            role: Role::Normal,
        });
        let pair = Block::Pair(Pair {
            left: Reading {
                value: Some("70".to_string()),
                unit: Some("°F".to_string()),
                ..Reading::default()
            },
            right: Reading {
                value: Some("76".to_string()),
                unit: Some("°F".to_string()),
                ..Reading::default()
            },
        });
        let alone = Card {
            slot: Slot::Tertiary,
            ttl_seconds: 60,
            blocks: vec![statement.clone()],
        };
        let stacked = Card {
            slot: Slot::Tertiary,
            ttl_seconds: 60,
            blocks: vec![pair, statement],
        };
        let (_, y0, _, y1) = drew(&alone).expect("the statement alone");
        let (_, py0, _, py1) = drew(&stacked).expect("the pair and the statement");
        assert!(
            (py1 - py0) > (y1 - y0) + 80,
            "the pair added no height: {} vs {}",
            py1 - py0,
            y1 - y0
        );
    }

    /// A reading with genuinely nothing on it is not a card that quietly draws
    /// an empty rectangle.
    ///
    /// **`state` is `Ok` here, and that is the point of the fixture.** Before
    /// 2026-09-02 this test used `Connecting`, because a state that admits no
    /// value drew nothing at all. It draws its MARK now — `SPEC-04` §7 rule 4's
    /// *a mark instead of a value* — so `Connecting` is no longer an example of
    /// nothing, and the test that used it would have passed for a reason that
    /// had stopped being true.
    #[test]
    fn a_card_with_nothing_drawable_on_it_reports_that_it_drew_nothing() {
        let card = Card {
            slot: Slot::Quaternary,
            ttl_seconds: 60,
            blocks: vec![Block::Reading(Reading {
                // `clock` is in the vocabulary and in no Aura source, so
                // `icon_is_drawn` is false and the column reserves no width.
                glyph: Some(Glyph::Clock),
                state: State::Ok,
                ..Reading::default()
            })],
        };
        assert!(
            drew(&card).is_none(),
            "a card with no drawable content claimed to have drawn"
        );
    }

    /// **A reading that cannot be believed draws its MARK where its value would
    /// have been** — `SPEC-04` §7 rule 4, built 2026-09-02.
    ///
    /// This is the behaviour the renderer had been journalling an apology for
    /// since the day the vocabulary admitted `state`: *"this build has no icon
    /// face, so the mark is OMITTED"*. The face was in Aura's repository the
    /// whole time.
    #[test]
    fn a_reading_that_cannot_be_believed_draws_its_mark_instead_of_a_value() {
        if icon_face().is_none() {
            // A tree without the asset is a tree this test has nothing to say
            // about — but the asset is TRACKED, so say which state that is
            // rather than passing quietly. `SPEC-04` §12.1's scar.
            panic!("the icon face is tracked in this repository and did not load");
        }
        for state in [State::Connecting, State::Unavailable] {
            let card = Card {
                slot: Slot::Quaternary,
                ttl_seconds: 60,
                blocks: vec![Block::Reading(Reading {
                    state,
                    ..Reading::default()
                })],
            };
            assert!(
                drew(&card).is_some(),
                "a `{state:?}` reading drew nothing at all, so a person looking at the slot \
                 cannot tell it apart from an empty one"
            );
        }
    }

    /// **A trend arrow widens the reading**, which is how a value with a
    /// direction is told apart from one without at a glance.
    #[test]
    fn a_trend_arrow_is_drawn_beside_the_value_it_belongs_to() {
        if icon_face().is_none() {
            panic!("the icon face is tracked in this repository and did not load");
        }
        let with = |trend| Card {
            slot: Slot::Quaternary,
            ttl_seconds: 60,
            blocks: vec![Block::Reading(Reading {
                value: Some("6.5".to_string()),
                trend,
                ..Reading::default()
            })],
        };
        let (px0, _, px1, _) = drew(&with(None)).expect("a plain reading draws");
        let (ax0, _, ax1, _) = drew(&with(Some(aura_card::Trend::Rising))).expect("with an arrow");
        assert!(
            (ax1 - ax0) > (px1 - px0),
            "the arrow added no width: {} vs {}",
            ax1 - ax0,
            px1 - px0
        );
    }

    /// The block never leaves the screen, at any corner, and a card too big for
    /// the panel is clipped by the drawing primitives rather than wrapping
    /// through a `usize` subtraction — which is what `clamp_usize` is for.
    #[test]
    fn a_card_stays_on_the_screen_in_every_corner() {
        let (bold, regular) = faces();
        for corner in [
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomLeft,
            Corner::BottomRight,
        ] {
            let (mut c, t) = canvas(1920, 1080);
            assert!(super::card(
                &mut c,
                &t,
                &bold,
                &regular,
                icon_face().as_ref(),
                &departures(),
                corner,
                1920.0,
                1080.0,
                widgets::widget_grey(60),
                96.0,
                None,
            ));
            let (x0, y0, x1, y1) = ink_bounds(&c, &t).expect("nothing drew at {corner:?}");
            assert!(
                x1 < 1920 && y1 < 1080,
                "{corner:?}: ink at {x1},{y1} is off a 1920x1080 screen"
            );
            // Pad is 96 with a card theme and this call passes none, so the
            // block sits 96 from its two edges either way; the assertion is
            // only that it is inside, not where.
            assert!(x0 < 1920 && y0 < 1080, "{corner:?}: origin off screen");
        }
    }

    /// **A role reaches the screen as a different colour.** The old
    /// `theme::Theme::ink` did this against an invented palette; this asserts it
    /// against Aura's own, through `widgets::role_ink`.
    #[test]
    fn the_five_roles_are_not_all_the_same_colour() {
        let theme = Some(widgets::sample::GREEN);
        let grey = widgets::widget_grey(60);
        let normal = widgets::role_ink(theme.as_ref(), grey, Role::Normal);
        let muted = widgets::role_ink(theme.as_ref(), grey, Role::Muted);
        let warn = widgets::role_ink(theme.as_ref(), grey, Role::Warn);
        let alarm = widgets::role_ink(theme.as_ref(), grey, Role::Alarm);
        assert_ne!(normal, muted, "muted must differ from normal — it is 35% alpha");
        assert_ne!(warn.0, normal.0, "warn must not be the ordinary ink");
        assert_ne!(alarm.0, normal.0, "alarm must not be the ordinary ink");
        assert_ne!(alarm.0, warn.0, "alarm and warn must be distinguishable");
        // Brightness reaches the signal colours, which is the original reading
        // card's `dim` and the behavior `Theme::dimmed` was tested for.
        let bright = widgets::role_ink(theme.as_ref(), widgets::widget_grey(100), Role::Alarm);
        assert!(
            alarm.0[0] < bright.0[0],
            "brightness did not reach the alarm colour"
        );
    }

    /// A label too long for its row is cut with an ellipsis rather than
    /// overrunning the card, and the two token columns keep their room.
    #[test]
    fn a_long_label_is_cut_and_the_trailing_token_still_fits() {
        let (_, regular) = faces();
        let long = "Ypsilanti Transit Center via Washtenaw and Golfside".repeat(3);
        let cut = truncate(&regular, &long, ROW_SIZE, 600.0);
        assert!(cut.ends_with('…'), "a cut label must say it was cut: {cut:?}");
        assert!(
            regular.measure(&cut, ROW_SIZE).0 <= 600.0,
            "the cut label is still too wide"
        );
        assert!(long.starts_with(cut.trim_end_matches('…')), "the cut changed the text");
    }

    /// **The generic card writes a picture of itself into `target/`** so that
    /// the one judgement that matters — a person looking at it — can be made
    /// without a board. `widgets::tests` does the same for the weather panel and
    /// this follows it deliberately.
    ///
    /// It asserts as well as writing: a file this test produced while drawing
    /// nothing would be worse than no file.
    #[test]
    fn the_generic_card_draws_itself_into_a_file_that_can_be_looked_at() {
        let (bold, regular) = faces();
        let (w, h) = (1920usize, 1080usize);
        let (mut c, t) = canvas(w, h);
        // An addon's card of the kind Aura has no widget for at all: a reading
        // with a caption, a departure board, and a sentence explaining itself.
        let card = Card {
            slot: Slot::Tertiary,
            ttl_seconds: 120,
            blocks: vec![
                // **A reading that is stale AND rising** — both marks at once,
                // which is the arrangement a person has to judge: the amber
                // question to the left of the number, the arrow to its right,
                // and neither shoving the value off centre.
                Block::Reading(Reading {
                    value: Some("6.5".to_string()),
                    unit: Some(" m".to_string()),
                    caption: Some("ten minutes ago".to_string()),
                    trend: Some(aura_card::Trend::Rising),
                    state: State::Stale,
                    ..Reading::default()
                }),
                departures().blocks.into_iter().next().unwrap(),
                Block::Statement(Statement {
                    text: "The sensor has not reported since 14:02".to_string(),
                    role: Role::Warn,
                }),
            ],
        };
        assert!(super::card(
            &mut c,
            &t,
            &bold,
            &regular,
            icon_face().as_ref(),
            &card,
            Corner::TopLeft,
            w as f32,
            h as f32,
            widgets::widget_grey(60),
            96.0,
            Some(&widgets::sample::GREEN),
        ));
        let (x0, y0, x1, y1) = ink_bounds(&c, &t).expect("the card drew nothing");
        assert!(
            x1 - x0 > 400 && y1 - y0 > 200,
            "the card is implausibly small"
        );

        // A binary PPM: three bytes a pixel, a header a person can read, and no
        // encoder to be wrong about. `target/` is not tracked.
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let at = y * t.stride + x * t.bytes_per_pixel();
                ppm.extend_from_slice(&[c[at + 2], c[at + 1], c[at]]);
            }
        }
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/target/generic-card.ppm");
        // Created rather than assumed — see the note at the identical write in
        // `crate::widgets`. Under `tools/test-all.sh` this directory does not
        // exist, and these tests were passing only in a tree that had been
        // built natively.
        std::fs::create_dir_all(std::path::Path::new(path).parent().expect("a parent"))
            .expect("the directory for a rendered fixture should be creatable");
        std::fs::write(path, ppm).expect("target/ should be writable");
    }
}
