//! **What the screen says**, and nothing about how it gets there.
//!
//! The maintainer specified this layout (.rulings/ R-333): the `hostname.local`
//! and the control plane's address top left, the power button top right, the two
//! application cards, with Aura's appIcon shown on its card, and a HH:MM clock
//! in the bottom left corner.
//!
//! # It draws pixels and knows no protocol
//!
//! `crate::surface` hands this a canvas and puts the result on the screen;
//! nothing here has heard of Wayland, and nothing here had heard of DRM
//! before. That seam is why `crate::ui` and `crate::text` survived `ADR-058`
//! untouched while `drm.rs` was deleted whole.
//!
//! # The glass is real and is not a shader
//!
//! A card is a translucent fill over a gradient, with a soft shadow under it —
//! [`shadow`] and [`glass`] below. The maintainer asked for drop shadows or
//! glass textures in a simple, basic form (.rulings/ R-333), and this is the
//! honest version of that on a CPU: about 3 ms for a 1920×1080 frame, drawn when the
//! menu changes rather than every vblank.
//!
//! **The GPU version belongs in `rivet-displayd`, not here.** `ADR-058` §3's
//! whole argument is that the compositor owns the GPU so a client does not
//! have to: a launcher that wanted a real blur would need EGL, `wayland-egl`
//! and Mesa, which is every one of §1.1's five confinement grants back again,
//! in the program that was supposed to prove they were unnecessary. The right
//! shape is a compositor-side effect the client *asks* for, and that is a
//! protocol extension rather than a `#version 100` string in this file.

use crate::text::{Face, Target};
use crate::ui;

// The palette, carried over from the DRM menu unchanged — these were chosen
// to read at television distance and nothing about `ADR-058` changes that.
const BACKGROUND: [u8; 3] = [0x0d, 0x11, 0x17];
const BACKGROUND_LIFT: [u8; 3] = [0x14, 0x1b, 0x25];
const CARD: [u8; 3] = [0x16, 0x1b, 0x22];
const CARD_EDGE: [u8; 3] = [0x24, 0x2b, 0x33];
const FOCUS: [u8; 3] = [0x3f, 0xb9, 0x8c];
const INK: [u8; 3] = [0xe6, 0xed, 0xf3];
const INK_DIM: [u8; 3] = [0x7d, 0x8a, 0x97];


/// **The family mark: three concentric circles** — `(radius, stroke, opacity)`
/// in the 24-unit box the SVG is drawn in.
///
/// Copied from `examples/aura/aura-controller/src/portal/assets/index.html`
/// line 55, which is Aura's mark and its favicon. The maintainer, 2026-09-20:
/// the mark is the app icon and the favicon (.rulings/ R-333).
///
/// **Copied as NUMBERS rather than as a file**, and that is the whole reason
/// this works today. The alternative was an asset — which needs a format, a
/// decoder this tree does not have, a manifest key to declare it by, and a
/// packaging step to carry it. Three circles are three circles; drawing them
/// from the same radii the browser draws them from is exact rather than an
/// approximation of a picture.
///
/// **It is drawn for EVERY card and that is temporary.** This is the mark the
/// three portals share (`family-web-design-language`), not a per-application
/// icon, and a launcher cannot have one until an application can declare one.
/// A stroke of `0.0` means filled.
const MARK: [(f32, f32, f32); 3] = [(3.0, 0.0, 1.0), (6.5, 1.5, 0.6), (10.5, 1.0, 0.3)];

/// The box [`MARK`]'s radii are measured in — the SVG's `viewBox`.
const MARK_BOX: f32 = 24.0;

/// Everything the screen shows, gathered by `main` and handed over once.
///
/// **A struct rather than a dozen arguments**, because the drawing is a pure
/// function of it: the same `Frame` always produces the same pixels, which is
/// what makes the layout testable without a compositor.
pub struct Frame<'a> {
    /// `rivet-06f5c.local`, or `None` while the supervisor has not said.
    pub hostname: Option<&'a str>,
    /// `192.168.50.66:7777`, or `None` when this machine could not find its
    /// own address — which is worth drawing as absent rather than guessing.
    pub address: Option<&'a str>,
    /// What may be started, in menu order.
    pub offered: &'a [crate::offered::Offer],
    /// Which control has the focus.
    pub focus: Focus,
    /// `HH:MM`, already formatted — the layout does not own a clock.
    pub clock: &'a str,
}

// **THERE IS NO `fault` FIELD, and it is a ruling rather than an omission.**
//
// A first version drew a sentence across the bottom when something went wrong,
// under a comment arguing that *"the person who can fix it is standing in
// front of the screen"*. The maintainer, on seeing one, had it removed and
// ruled that errors belong in journals and not on his screen (.rulings/ R-205).
//
// The argument was wrong on its own terms. This screen is on a wall, in a
// building, in front of people who did not ask the machine anything — a lobby
// television is not a console, and an operator's diagnostic is not content.
// The journal is where a fault is looked for, `println!` already puts it there
// (`ADR-017` §7), and a machine that has something to say about itself says it
// to whoever asked.
//
// `crate::ui`'s `ALERT` colour went with it. The day a screen needs to show
// that something is wrong, it is a DESIGNED state and not an error string.

/// What the person is pointing at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The nth offered application.
    Card(usize),
    /// The power control, top right.
    Power,
}

impl Focus {
    /// Move one step right, wrapping through the power control.
    ///
    /// **Power is the last stop and not a separate axis.** A kiosk is
    /// navigated with four buttons and a hold; an control reachable only by a
    /// gesture nobody documented is a control nobody finds.
    #[must_use]
    pub fn next(self, cards: usize) -> Self {
        match self {
            Self::Card(i) if i + 1 < cards => Self::Card(i + 1),
            Self::Card(_) => Self::Power,
            Self::Power if cards > 0 => Self::Card(0),
            Self::Power => Self::Power,
        }
    }

    /// Move one step left, wrapping the same way.
    #[must_use]
    pub fn previous(self, cards: usize) -> Self {
        match self {
            Self::Card(0) => Self::Power,
            Self::Card(i) => Self::Card(i - 1),
            Self::Power if cards > 0 => Self::Card(cards - 1),
            Self::Power => Self::Power,
        }
    }
}

/// Draw the whole screen.
///
/// `face` is optional and the screen degrades rather than failing: the cards,
/// the mark and the power control are all geometry and still appear. `ADR-018`
/// §8 — a machine missing a font is still a machine somebody can use.
///
/// **There is no icon face any more.** It used to draw `apps` out of the
/// 1,844-byte Material subset; [`mark`] draws the family's own three circles
/// instead, which is a better picture and one fewer file to be missing.
pub fn draw(canvas: &mut [u8], t: &Target, face: Option<&Face>, frame: &Frame<'_>) {
    let unit = (t.height as f32 / 1080.0).max(0.4);
    let px = |n: f32| (n * unit).round() as usize;
    let margin = px(64.0);

    backdrop(canvas, t);

    if let Some(face) = face {
        header(canvas, t, face, frame, margin, unit);
        clock(canvas, t, face, frame.clock, margin, unit);
    }

    power(canvas, t, frame.focus == Focus::Power, margin, unit);
    cards(canvas, t, face, frame, unit);
}

/// A vertical gradient, dark at the top and a shade lighter at the bottom.
///
/// **The whole canvas is written every frame**, which is what makes the
/// translucent cards above it correct: an alpha blend reads what is already
/// there, so a partial repaint would blend a card over the previous card.
fn backdrop(canvas: &mut [u8], t: &Target) {
    let height = t.height.max(1) as f32;
    for y in 0..t.height {
        let k = y as f32 / height;
        let colour = [
            mix(BACKGROUND[0], BACKGROUND_LIFT[0], k),
            mix(BACKGROUND[1], BACKGROUND_LIFT[1], k),
            mix(BACKGROUND[2], BACKGROUND_LIFT[2], k),
        ];
        for x in 0..t.width {
            ui::blend(canvas, t, x, y, colour, 1.0);
        }
    }
}

fn mix(a: u8, b: u8, k: f32) -> u8 {
    (f32::from(a).mul_add(1.0 - k, f32::from(b) * k) + 0.5) as u8
}

/// `hostname.local` big, and the address it answers on beside it, quieter.
fn header(
    canvas: &mut [u8],
    t: &Target,
    face: &Face,
    frame: &Frame<'_>,
    margin: usize,
    unit: f32,
) {
    // **Two lines, not one, and no parentheses** — the maintainer, on seeing
    // the first version, asked to lose the parentheses and put the ip:port below
    // the host name, with the host name bold and the ip:port normal
    // (.rulings/ R-333).
    //
    // **"Bold" here is size and brightness, not a weight**, and that is a
    // limitation stated rather than worked around: `Geist.ttf` is a variable
    // font and `fontdue` rasterises its default instance, so there is no
    // second weight to ask for. `assets/fonts/README.md` says the same thing —
    // *"naming a weight is a thing to do the day a screen needs two of them"* —
    // and this is that day, so it is worth doing properly rather than faking
    // heavier glyphs by drawing them twice.
    let name_px = 44.0 * unit;
    let addr_px = 24.0 * unit;
    let first = (74.0 * unit).round() as usize;
    let second = first + (34.0 * unit).round() as usize;

    // `.local` stays dim beside a bright name: the name is what a person is
    // looking for and the suffix is how the machine is reached. Two colours on
    // one baseline is what `Face::draw`'s baseline contract exists for.
    let name = frame.hostname.unwrap_or("this machine");
    face.draw(canvas, t, margin, first, name, name_px, INK);
    if frame.hostname.is_some() {
        let pen = margin + face.width_of(name, name_px);
        face.draw(canvas, t, pen, first, ".local", name_px, INK_DIM);
    }

    if let Some(address) = frame.address {
        face.draw(canvas, t, margin, second, address, addr_px, INK_DIM);
    }
}

/// `HH:MM`, bottom left.
fn clock(canvas: &mut [u8], t: &Target, face: &Face, now: &str, margin: usize, unit: f32) {
    let size = 72.0 * unit;
    let baseline = t.height.saturating_sub((64.0 * unit).round() as usize);
    face.draw(canvas, t, margin, baseline, now, size, INK);
}

/// The power control: a ring with a gap at the top and a bar through it.
///
/// **Drawn rather than set in a glyph**, and that is forced rather than
/// preferred: `assets/fonts/MaterialIcons.ttf` is a **1,844-byte subset**
/// carrying two codepoints, and `power_settings_new` is not one of them.
/// Asking for it would draw nothing at all — the silent-wrong-picture failure
/// `aura-renderer`'s `icons.rs` records — where thirty lines of signed
/// distance cannot be wrong about which symbol it is.
fn power(canvas: &mut [u8], t: &Target, focused: bool, margin: usize, unit: f32) {
    let radius = 30.0 * unit;
    let thickness = 4.0 * unit;
    let cx = t.width as f32 - margin as f32 - radius;
    let cy = 66.0 * unit;
    let colour = if focused { FOCUS } else { INK_DIM };

    let reach = (radius + thickness * 2.0).ceil() as usize;
    let (x0, y0) = (
        (cx - reach as f32).max(0.0) as usize,
        (cy - reach as f32).max(0.0) as usize,
    );

    if focused {
        // A soft halo, so the focused control reads from across a room even
        // where the ring itself is a few pixels wide.
        soft_disc(canvas, t, cx, cy, radius * 1.9, FOCUS, 0.16);
    }

    for y in y0..(y0 + reach * 2).min(t.height) {
        for x in x0..(x0 + reach * 2).min(t.width) {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);

            // The ring, minus a wedge at the top. `dy < 0` is above the
            // centre, and the wedge is where the bar comes through.
            let ring = (dx.hypot(dy) - radius).abs() - thickness / 2.0;
            let in_gap = dy < 0.0 && dx.abs() < thickness * 1.6;
            let ring_alpha = if in_gap { 0.0 } else { (0.5 - ring).clamp(0.0, 1.0) };

            // The bar: a vertical capsule from above the ring to its middle.
            let bar_top = cy - radius * 1.15;
            let bar_bottom = cy - radius * 0.1;
            let along = dy.max(bar_top - cy).min(bar_bottom - cy);
            let bar = (dx.hypot(dy - along) - thickness / 2.0)
                .max(bar_top - (y as f32 + 0.5))
                .max((y as f32 + 0.5) - bar_bottom);
            let bar_alpha = (0.5 - bar).clamp(0.0, 1.0);

            let alpha = ring_alpha.max(bar_alpha);
            ui::blend(canvas, t, x, y, colour, alpha);
        }
    }
}

/// The application cards, centred as a row.
fn cards(
    canvas: &mut [u8],
    t: &Target,
    face: Option<&Face>,

    frame: &Frame<'_>,
    unit: f32,
) {
    if frame.offered.is_empty() {
        if let Some(face) = face {
            let size = 32.0 * unit;
            let text = "no application on this machine offers a screen";
            let w = face.width_of(text, size);
            face.draw(
                canvas,
                t,
                t.width.saturating_sub(w) / 2,
                t.height / 2,
                text,
                size,
                INK_DIM,
            );
        }
        return;
    }

    let card_w = (460.0 * unit).round() as usize;
    let card_h = (300.0 * unit).round() as usize;
    let gap = (56.0 * unit).round() as usize;
    let radius = (28.0 * unit).round() as usize;

    // **Laid out from the middle outward**, so two cards are centred and three
    // are still centred. The alternative — a fixed left margin — makes a
    // machine offering one application look like one that lost the others.
    let count = frame.offered.len();
    let row_w = card_w * count + gap * count.saturating_sub(1);
    let start = t.width.saturating_sub(row_w) / 2;
    let top = t.height.saturating_sub(card_h) / 2;

    for (index, offer) in frame.offered.iter().enumerate() {
        let x = start + index * (card_w + gap);
        let focused = frame.focus == Focus::Card(index);

        shadow(canvas, t, x, top, card_w, card_h, radius, unit);
        glass(canvas, t, x, top, card_w, card_h, radius);

        // **The focus ring is the same THICKNESS whether it has focus or not**,
        // and only its colour changes.
        //
        // The first version drew 3 px when focused and 1 px otherwise.
        // `stroke_rounded` insets correctly, so the card's outer edge did not
        // move — but the INNER radius did, 27 px against 25 px, and the corner
        // visibly changed shape as the selection moved. A card that changes
        // shape on focus is what the maintainer read as a wonky border, along
        // with the sheen bug `glass` records.
        //
        // What reads as *selected* is the halo below, which adds no geometry
        // at all. The power control has had one since it was written and does
        // not have this problem.
        let ring = (2.0 * unit).round().max(1.0) as usize;
        if focused {
            halo(canvas, t, x, top, card_w, card_h, radius, unit);
        }
        ui::stroke_rounded(
            canvas,
            t,
            x,
            top,
            card_w,
            card_h,
            radius,
            ring,
            if focused { FOCUS } else { CARD_EDGE },
        );

        // **The mark and the name are ONE GROUP, centred together**, and the
        // first version was not — the mark was placed from the top of the card
        // and the name from the bottom, so the space between them was whatever
        // was left over. The maintainer asked for a look at the vertical spacing
        // and centering of the logo and `aura-renderer`, suggesting the logo
        // could move down a little (.rulings/ R-333).
        //
        // Measuring the group and centring THAT is what makes the answer
        // correct at any card size rather than tuned for one. Everything below
        // is derived; nothing is a nudge.
        let mark_size = 118.0 * unit;
        let name_px = 34.0 * unit;
        // Geist's cap height, close enough for centring and honest about being
        // an approximation: `fontdue` reports per-glyph metrics and a line
        // height, and neither is the height of a word in capitals and
        // lowercase. Being a few pixels out here is invisible; being derived
        // from the wrong quantity would not be.
        let cap = name_px * 0.72;
        let gap = 26.0 * unit;

        let group = mark_size + gap + cap;
        let group_top = top as f32 + (card_h as f32 - group) / 2.0;

        mark(
            canvas,
            t,
            x as f32 + card_w as f32 / 2.0,
            group_top + mark_size / 2.0,
            mark_size,
            if focused { FOCUS } else { INK },
        );

        if let Some(face) = face {
            let w = face.width_of(&offer.name, name_px);
            // The baseline sits a cap height below the top of the text, which
            // is what `Face::draw`'s baseline contract means by a line of
            // capitals resting on it.
            let baseline = group_top + mark_size + gap + cap;
            face.draw(
                canvas,
                t,
                x + card_w.saturating_sub(w) / 2,
                baseline.max(0.0) as usize,
                &offer.name,
                name_px,
                if focused { INK } else { INK_DIM },
            );
        }
    }
}

/// **The family mark**, centred on `(cx, cy)` and `size` pixels across.
///
/// [`MARK`]'s three circles, scaled out of the 24-unit box the SVG draws them
/// in. A stroke of `0.0` is a filled disc; anything else is a ring of that
/// width, centred on the radius exactly as SVG's `stroke-width` is.
///
/// Antialiased by the same half-pixel coverage band `crate::ui` uses on a
/// rounded rectangle — at this size a hard edge on a circle is the one thing
/// that would make it read as drawn-by-a-program rather than as a mark.
fn mark(canvas: &mut [u8], t: &Target, cx: f32, cy: f32, size: f32, colour: [u8; 3]) {
    let scale = size / MARK_BOX;
    let reach = (MARK_BOX / 2.0 * scale).ceil() + 2.0;
    let x0 = (cx - reach).max(0.0) as usize;
    let y0 = (cy - reach).max(0.0) as usize;
    let x1 = ((cx + reach) as usize).min(t.width);
    let y1 = ((cy + reach) as usize).min(t.height);

    for y in y0..y1 {
        for x in x0..x1 {
            let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
            let mut alpha: f32 = 0.0;
            for (radius, stroke, opacity) in MARK {
                let r = radius * scale;
                let coverage = if stroke <= 0.0 {
                    // A disc: inside is covered, and the boundary is the band.
                    (0.5 - (d - r)).clamp(0.0, 1.0)
                } else {
                    // A ring: distance from the circle, minus half the stroke.
                    let half = stroke * scale / 2.0;
                    (0.5 - ((d - r).abs() - half)).clamp(0.0, 1.0)
                };
                alpha = alpha.max(coverage * opacity);
            }
            ui::blend(canvas, t, x, y, colour, alpha);
        }
    }
}

/// **A soft glow around a focused card**, outside its edge and touching none
/// of its geometry.
///
/// The same falloff [`shadow`] uses, in the accent and without the downward
/// offset — a shadow says *this is above the background*, a halo says *this is
/// the one you are pointing at*, and drawing both from one expression is what
/// keeps them consistent as the card's radius changes.
fn halo(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    unit: f32,
) {
    let spread = 18.0 * unit;
    let (fx, fy) = (x as f32, y as f32);
    let (fw, fh) = (w as f32, h as f32);
    let r = radius as f32;

    let pad = spread.ceil() as usize;
    for py in y.saturating_sub(pad)..(y + h + pad).min(t.height) {
        for px in x.saturating_sub(pad)..(x + w + pad).min(t.width) {
            let dx = (px as f32 + 0.5 - (fx + fw / 2.0)).abs() - (fw / 2.0 - r);
            let dy = (py as f32 + 0.5 - (fy + fh / 2.0)).abs() - (fh / 2.0 - r);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) - r;
            if outside <= 0.0 {
                continue;
            }
            let k = (1.0 - outside / spread).clamp(0.0, 1.0);
            ui::blend(canvas, t, px, py, FOCUS, k * k * 0.30);
        }
    }
}

/// A soft drop shadow under a rounded rectangle.
///
/// Not a gaussian: the alpha falls off with the distance outside the shape,
/// which for a rounded rectangle is what a blurred shadow looks like anyway
/// and costs one `hypot` per pixel instead of two passes over the canvas.
fn shadow(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    unit: f32,
) {
    let spread = 26.0 * unit;
    let drop = 10.0 * unit;
    let (fx, fy) = (x as f32, y as f32 + drop);
    let (fw, fh) = (w as f32, h as f32);
    let r = radius as f32;

    let pad = spread.ceil() as usize;
    let y0 = y.saturating_sub(pad);
    let x0 = x.saturating_sub(pad);
    for py in y0..(y + h + pad + drop as usize).min(t.height) {
        for px in x0..(x + w + pad).min(t.width) {
            let dx = (px as f32 + 0.5 - (fx + fw / 2.0)).abs() - (fw / 2.0 - r);
            let dy = (py as f32 + 0.5 - (fy + fh / 2.0)).abs() - (fh / 2.0 - r);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) - r;
            if outside <= 0.0 {
                continue;
            }
            let k = (1.0 - outside / spread).clamp(0.0, 1.0);
            // Cubed, so the falloff is soft near the edge and gone well before
            // the next card — a linear ramp reads as a grey halo.
            ui::blend(canvas, t, px, py, [0, 0, 0], k * k * k * 0.55);
        }
    }
}

/// A translucent card over whatever the backdrop put there, lighter at the top.
///
/// This is the "glass": the gradient behind shows through, and the highlight
/// across the top edge is what makes it read as a surface rather than a
/// rectangle of flat colour.
fn glass(canvas: &mut [u8], t: &Target, x: usize, y: usize, w: usize, h: usize, radius: usize) {
    ui::fill_rounded(canvas, t, x, y, w, h, radius, CARD);

    // **The sheen: a few rows of low-alpha white along the top, masked by the
    // CARD's own shape.**
    //
    // The first version drew each row as `fill_rounded(.., w, 1, radius, ..)`
    // — a rounded rectangle one pixel tall. A radius larger than half the
    // shorter side is clamped to a stadium, so `radius` became `0.5` and every
    // row was a **full-width straight bar** painted over the card's rounded top
    // corners. On a board it reads as a wonky highlight, which is what the
    // maintainer saw first.
    //
    // The mask has to be the card's coverage at that pixel, so the sheen ends
    // exactly where the card does.
    let sheen = (h / 6).max(1);
    let (fx, fy) = (x as f32, y as f32);
    let (fw, fh) = (w as f32, h as f32);
    let r = radius as f32;
    for row in 0..sheen {
        let k = 1.0 - row as f32 / sheen as f32;
        let py = fy + row as f32 + 0.5;
        for col in 0..w {
            let inside = ui::coverage(fx + col as f32 + 0.5, py, fx, fy, fw, fh, r);
            ui::blend(canvas, t, x + col, y + row, [0xff, 0xff, 0xff], inside * 0.10 * k);
        }
    }
}

/// A radial glow, used behind a focused control.
fn soft_disc(
    canvas: &mut [u8],
    t: &Target,
    cx: f32,
    cy: f32,
    radius: f32,
    colour: [u8; 3],
    peak: f32,
) {
    let reach = radius.ceil() as usize;
    let x0 = (cx - radius).max(0.0) as usize;
    let y0 = (cy - radius).max(0.0) as usize;
    for y in y0..(y0 + reach * 2).min(t.height) {
        for x in x0..(x0 + reach * 2).min(t.width) {
            let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
            let k = (1.0 - d / radius).clamp(0.0, 1.0);
            ui::blend(canvas, t, x, y, colour, k * k * peak);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn offers(n: usize) -> Vec<crate::offered::Offer> {
        (0..n)
            .map(|i| crate::offered::Offer {
                name: format!("app-{i}"),
            })
            .collect()
    }

    /// **The power control is reachable from the last card and back again.**
    /// A kiosk is navigated with four buttons; a control that can be moved
    /// past but not onto is a control nobody can press.
    #[test]
    fn the_focus_walks_the_cards_and_then_the_power_control() {
        let cards = 2;
        let mut focus = Focus::Card(0);
        focus = focus.next(cards);
        assert_eq!(focus, Focus::Card(1));
        focus = focus.next(cards);
        assert_eq!(focus, Focus::Power);
        focus = focus.next(cards);
        assert_eq!(focus, Focus::Card(0), "it must wrap rather than stop");
    }

    /// Backwards from the first card reaches power directly, which is what
    /// makes "one step left from the start" useful rather than a dead press.
    #[test]
    fn the_focus_walks_backwards_through_power() {
        assert_eq!(Focus::Card(0).previous(2), Focus::Power);
        assert_eq!(Focus::Power.previous(2), Focus::Card(1));
        assert_eq!(Focus::Card(1).previous(2), Focus::Card(0));
    }

    /// **A machine offering nothing still has a power control.** The focus
    /// cannot land on a card that is not there, and a `next` that returned
    /// `Card(0)` would index an empty list on the next draw.
    #[test]
    fn a_machine_with_nothing_to_offer_keeps_the_focus_on_power() {
        assert_eq!(Focus::Power.next(0), Focus::Power);
        assert_eq!(Focus::Power.previous(0), Focus::Power);
    }

    fn canvas(w: usize, h: usize) -> (Vec<u8>, Target) {
        (
            vec![0u8; w * h * 4],
            Target {
                width: w,
                height: h,
                stride: w * 4,
                bytes_per_pixel: 4,
            },
        )
    }

    /// **It draws without a font**, which is `ADR-018` §8: a machine missing
    /// an asset is still a machine somebody can look at and repair. This is
    /// the test that would have caught an unwrap on `Face::load`.
    #[test]
    fn the_screen_draws_with_no_typeface_at_all() {
        let (mut c, t) = canvas(320, 180);
        let offered = offers(2);
        draw(
            &mut c,
            &t,
            None,
            &Frame {
                hostname: Some("rivet-test"),
                address: Some("192.168.0.2:7777"),
                offered: &offered,
                focus: Focus::Card(0),
                clock: "09:41",
            },
        );
        assert!(
            c.iter().any(|&b| b != 0),
            "nothing was drawn at all, so a fontless machine shows a black screen"
        );
    }

    /// The backdrop covers every pixel, which is what makes the translucent
    /// cards correct — an alpha blend reads what is already there, so a frame
    /// that left gaps would blend this frame's card over the last one's.
    #[test]
    fn every_pixel_is_written_so_a_translucent_card_cannot_accumulate() {
        let (mut c, t) = canvas(64, 64);
        backdrop(&mut c, &t);
        for y in 0..t.height {
            for x in 0..t.width {
                let at = y * t.stride + x * 4;
                assert_ne!(
                    (c[at], c[at + 1], c[at + 2]),
                    (0, 0, 0),
                    "({x}, {y}) was left untouched"
                );
            }
        }
    }

    /// A surface far smaller than 1080 lines still produces a layout rather
    /// than a panic — every size below is scaled from the height, and
    /// `usize` subtraction is where that goes wrong.
    #[test]
    fn a_tiny_surface_does_not_underflow_the_layout() {
        let (mut c, t) = canvas(96, 64);
        let offered = offers(3);
        draw(
            &mut c,
            &t,
            None,
            &Frame {
                hostname: None,
                address: None,
                offered: &offered,
                focus: Focus::Power,
                clock: "00:00",
            },
        );
    }
}
