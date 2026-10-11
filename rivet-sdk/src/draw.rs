//! **Shapes** — rounded rectangles, and the arithmetic that makes their corners
//! not look like a mistake.
//!
//! `PLAN.md` §104.38. The maintainer's direction: each application's card is a
//! rounded rectangle, and the selected card is outlined in a color
//! (`.rulings/` R-336).
//!
//! # Why the corners are computed rather than drawn
//!
//! A rounded corner made of whole pixels has visible steps, and at the size a
//! television is viewed from those steps are what the eye lands on. Each pixel's
//! **coverage** is computed instead — how much of it lies inside the shape —
//! and blended as alpha, which is the same mechanism `crate::text` uses for
//! glyph edges. One idea, applied twice, so a card and the word on it are made
//! of the same stuff.
//!
//! # Everything here is `XRGB8888`
//!
//! Little-endian, blue at the lowest byte, which is what `crate::drm`'s dumb
//! buffer scans out. `§104.35.2` deleted the framebuffer path and with it the
//! channel-offset indirection that used to make this configurable; there is one
//! layout now and it is written down here.

use crate::text::Target;

/// Blend one colour into one pixel at a given coverage.
///
/// `crate::text::blend`'s twin, and deliberately a separate function rather
/// than a shared one: that blends a glyph's coverage, this blends a shape's, and
/// tying them together would mean a change for one silently altering the other.
#[inline(always)]
fn blend(canvas: &mut [u8], t: &Target, x: usize, y: usize, colour: [u8; 3], alpha: f32) {
    if x >= t.width || y >= t.height || alpha <= 0.0 {
        return;
    }
    let at = y * t.stride + x * t.bytes_per_pixel();
    if at + 3 >= canvas.len() {
        return;
    }
    // **Opaque is the common case, and at alpha 1.0 the answer is the colour.**
    // Added 2026-09-01 (`SPEC-04` §10.3). Everything below this line is three
    // `mul_add`s, three multiplies and three float-to-int conversions to
    // compute something already known — and the interior of every filled shape
    // goes through it.
    //
    // **It is exact, not an approximation.** With `a == 1.0` the expression
    // below is `(f32::from(fg) * 1.0 + f32::from(bg) * 0.0 + 0.5) as u8`, and
    // `fg + 0.5` truncates back to `fg` for every `u8`. `tests::the_fast_paths_
    // are_byte_identical_to_the_general_ones` holds it against the reference.
    if alpha >= 1.0 {
        canvas[at] = colour[2];
        canvas[at + 1] = colour[1];
        canvas[at + 2] = colour[0];
        if t.format.writes_alpha() {
            canvas[at + 3] = 255;
        }
        return;
    }
    let a = alpha.clamp(0.0, 1.0);
    let mix = |fg: u8, bg: u8| -> u8 {
        // `+ 0.5` is rounding. Without it every edge biases dark, which on a
        // border one pixel wide is the difference between a line and a smudge.
        (f32::from(fg).mul_add(a, f32::from(bg) * (1.0 - a)) + 0.5) as u8
    };
    canvas[at] = mix(colour[2], canvas[at]);
    canvas[at + 1] = mix(colour[1], canvas[at + 1]);
    canvas[at + 2] = mix(colour[0], canvas[at + 2]);
    // The shape's coverage becomes the fourth byte through the same `mix` with
    // an opaque source — `a + dst * (1 - a)`, the `over` operator. See
    // `crate::text::blend`, which does the identical thing for a glyph, and
    // `text::Format::Argb8888` for why the result is premultiplied for free.
    if t.format.writes_alpha() {
        canvas[at + 3] = mix(255, canvas[at + 3]);
    }
}

/// How much of the pixel at `(px, py)` lies inside a rounded rectangle.
///
/// `0.0` outside, `1.0` well inside, a fraction on the boundary — and the
/// fraction is what makes the corner smooth rather than stepped.
///
/// # The signed distance, and the version of it that was wrong
///
/// The first attempt measured the distance to the shape shrunk by `r` and
/// treated that as the edge. It is correct OUTSIDE the shape and useless
/// inside it: every interior point clamps to itself, so the distance is zero
/// whether the pixel is one step in or in the middle, and a radius of zero made
/// the whole shape half-covered. A test caught it, which is the argument for
/// having one about a corner nobody would think to look at.
///
/// This is the standard rounded-box signed distance instead. `d` is how far the
/// pixel is past the inner box on each axis; the positive part gives the
/// distance outside, the negative part gives the distance inside, and
/// subtracting `r` rounds the corners. Coverage is then the half-pixel band
/// either side of zero.
fn coverage(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (half_w, half_h) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + half_w, y + half_h);
    let dx = (px - cx).abs() - (half_w - r);
    let dy = (py - cy).abs() - (half_h - r);
    let outside = dx.max(0.0).hypot(dy.max(0.0));
    let inside = dx.max(dy).min(0.0);
    let distance = inside + outside - r;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Fill a rounded rectangle.
pub fn fill_rounded(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    colour: [u8; 3],
) {
    fill_rounded_alpha(canvas, t, x, y, w, h, radius, colour, 255);
}

/// [`fill_rounded`], **translucent**.
///
/// # Why a shape needs an opacity at all
///
/// A card over a moving background is the case, and it is not a decoration.
/// Aura draws its clock on a card whose fill is `{3, 5, 14}` at alpha 155 so
/// that the digital rain behind it is still visible through it and the type on
/// it is still legible — one shape doing both jobs. Filled opaque it is a black
/// hole punched in the animation; left out entirely the type sits on whatever
/// the rain happens to be doing and stops being readable.
///
/// The shape's own antialiased coverage is multiplied by `alpha`, so a corner
/// stays smooth at every opacity rather than growing a hard edge as it fades.
/// `alpha` of 255 is [`fill_rounded`] exactly, and `0` draws nothing.
#[allow(clippy::too_many_arguments)]
pub fn fill_rounded_alpha(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    colour: [u8; 3],
    alpha: u8,
) {
    if alpha == 0 {
        return;
    }
    let opacity = f32::from(alpha) / 255.0;
    let (fx, fy) = (x as f32, y as f32);
    let (fw, fh) = (w as f32, h as f32);
    // A radius larger than half the shorter side is not a rounded rectangle, it
    // is a stadium; clamped rather than refused, because a caller that asked for
    // one should get the nearest thing that exists.
    let r = (radius as f32).min(fw / 2.0).min(fh / 2.0);
    // **Only the four corner squares can be anything but fully covered.**
    // Added 2026-09-01 (`SPEC-04` §10.3): the signed distance, `hypot`
    // included, was being evaluated at every pixel of a shape whose interior is
    // by definition solid — measured at 2.44 ms for a 760x420 card, against
    // 0.21 ms with this band. **11.8x, for arithmetic whose answer was 1.0.**
    //
    // The band is `ceil(r)` rows and columns from each edge. Everything outside
    // it is coverage exactly 1.0, which is worth stating rather than trusting:
    // for a pixel in the top band but horizontally central, `dx` is far
    // negative and `dy` is at most `r - 0.5`, so `inside` is 0, `outside` is
    // `dy`, and the distance is `dy - r <= -0.5` — the clamp's upper end. With
    // `r < 0.5` there is no band at all, and the same arithmetic gives 1.0 at
    // the corner, which `a_radius_of_zero_fills_the_corners` already asserted.
    //
    // `the_fast_paths_are_byte_identical_to_the_general_ones` compares this
    // against the original at every radius that matters, on a canvas that was
    // not blank.
    let band = r.ceil() as usize;
    for row in 0..h {
        let vertical_band = row < band || row + band >= h;
        for col in 0..w {
            let a = if vertical_band && (col < band || col + band >= w) {
                coverage(
                    fx + col as f32 + 0.5,
                    fy + row as f32 + 0.5,
                    fx,
                    fy,
                    fw,
                    fh,
                    r,
                )
            } else {
                1.0
            };
            blend(canvas, t, x + col, y + row, colour, a * opacity);
        }
    }
}

/// Stroke a rounded rectangle, `thickness` pixels inside its own bounds.
///
/// **Inside, not centred on the edge.** A border drawn centred would make a
/// selected card a few pixels bigger than an unselected one, so the whole row
/// would shift as the selection moved — which is exactly the kind of motion the
/// eye reads as a bug.
pub fn stroke_rounded(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    thickness: usize,
    colour: [u8; 3],
) {
    let (fx, fy) = (x as f32, y as f32);
    let (fw, fh) = (w as f32, h as f32);
    let r = (radius as f32).min(fw / 2.0).min(fh / 2.0);
    let th = thickness as f32;
    for row in 0..h {
        for col in 0..w {
            let (px, py) = (fx + col as f32 + 0.5, fy + row as f32 + 0.5);
            let outer = coverage(px, py, fx, fy, fw, fh, r);
            // The inner edge is the same shape inset by the thickness. The
            // stroke is what is inside the outer and outside the inner, which
            // keeps both edges antialiased instead of only the outside one.
            let inner = coverage(
                px,
                py,
                fx + th,
                fy + th,
                (fw - th * 2.0).max(0.0),
                (fh - th * 2.0).max(0.0),
                (r - th).max(0.0),
            );
            blend(canvas, t, x + col, y + row, colour, outer - inner);
        }
    }
}

/// Fill a plain rectangle. No rounding, no antialiasing, because there is
/// nothing to antialias.
///
/// # Clipped once and written a row at a time
///
/// Rewritten 2026-09-01 (`SPEC-04` §10.3). It called [`blend`] per pixel, which
/// meant recomputing `y * stride + x * bytes_per_pixel` and taking four
/// branches for every one of a screen's two million pixels — and it is the
/// first call of every frame, on the whole screen.
///
/// The rectangle is clipped once instead, and each row is written as a slice.
/// **The result is byte-for-byte what the per-pixel version produced**, which
/// `tests::the_fast_paths_are_byte_identical_to_the_general_ones` asserts
/// against the original, including the clipped and past-the-end cases.
pub fn fill(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    colour: [u8; 3],
) {
    let bpp = t.bytes_per_pixel();
    if x >= t.width || y >= t.height {
        return;
    }
    let opaque = t.format.writes_alpha();
    let x_end = x.saturating_add(w).min(t.width);
    let y_end = y.saturating_add(h).min(t.height);
    // The greatest byte offset `blend` would have written at: it required
    // `at + 3 < canvas.len()`, and that bound is preserved exactly rather than
    // tightened, so a caller drawing into a short canvas sees what it always saw.
    let last_start = canvas.len().saturating_sub(4);
    for row in y..y_end {
        let row_start = row * t.stride;
        let begin = row_start + x * bpp;
        if begin > last_start {
            continue;
        }
        let end = (row_start + x_end * bpp).min(last_start + bpp).min(canvas.len());
        for pixel in canvas[begin..end].chunks_exact_mut(bpp) {
            pixel[0] = colour[2];
            pixel[1] = colour[1];
            pixel[2] = colour[0];
            // A fill is opaque coverage, so on an alpha target it makes the
            // region opaque. `blend`'s `alpha >= 1.0` path, reached the fast
            // way; the branch is hoisted out of the loop above it.
            if opaque {
                pixel[3] = 255;
            }
        }
    }
}


/// **Paint the WHOLE surface one colour, at memory speed.**
///
/// # Why this exists beside [`fill`], which can already do it
///
/// It is eighteen milliseconds a frame, measured on `rivet-01` 2026-09-20.
///
/// Every animated background in `aura-background-pack` opens by clearing the
/// surface, and every one of them was doing it through [`fill`]:
///
/// ```text
/// starfield/white     background 21.53 ms    95% of the 33.3 ms budget
/// life/pulse          background 26.15 ms   105%
/// contours/ice-blue   background 34.29 ms   132%
/// ```
///
/// **Starfield is the measurement that gives it away.** It draws 320 stars of
/// **one pixel each** and still cost 21.53 ms. The stars are free; the clear
/// is the whole cost. The three numbers above differ only by what each spends
/// after it, and all three were paying about twenty milliseconds first.
///
/// # Why [`fill`] cannot simply be made fast
///
/// [`fill`] is **`blend` at full opacity**, which is the identity its own test
/// asserts byte for byte — so it writes three bytes per pixel and leaves the
/// fourth exactly as it found it. That is a real contract and worth keeping:
/// it is what lets a caller reason about `fill` and `blend` together.
///
/// Preserving every fourth byte is also precisely what makes it slow. There is
/// no way to memset three bytes in four; the write has to be byte-granular,
/// which is two million iterations of three indexed stores to clear a 1080p
/// screen. A Pi that should move 8.3 MB in about two milliseconds takes ten
/// times longer.
///
/// So this is a **different operation with a different contract**, not a fast
/// path bolted onto that one: it writes all four bytes, which lets a whole row
/// go out as one `copy_from_slice`.
///
/// # What it writes, and why that is safe
///
/// `bpp == 4` on this platform means **XRGB8888** — `aura-background-pack`'s
/// `render.rs` hard-codes it, and [`crate::composited`] and `crate::gpu` both
/// declare that fourcc to the compositor — where the fourth byte is `X`,
/// padding the scanout ignores.
///
/// **Zero, and that is not arbitrary.** [`fill`] never writes this byte, so on
/// every buffer this project has it has always been whatever the buffer was
/// allocated as — zero. `aura-background-pack` reuses one scratch canvas
/// across frames, so "never written" and "zero" have been the same thing there
/// since it existed, and its `T6` baseline hashes record it.
///
/// `255` was tried first, on the reasoning that a target which *does* read the
/// byte should see an opaque pixel. It broke
/// `render::a_frame_reaches_the_surface_and_leaves_padding_alone` and 24 of
/// the `T6` hashes — a change invisible on a screen and very visible to the
/// two things whose job is to notice that the port stopped being faithful.
/// **A byte nothing reads is not worth a baseline**, so this matches what was
/// already there.
///
/// **On an `Argb8888` target something does read it**, and there the same
/// reasoning gives the opposite answer: a clear is full coverage, so the byte
/// is `255`. That is not a special case bolted on — it is
/// [`Format`](crate::text::Format) being asked what the byte means instead of
/// the function assuming. To clear an overlay to *nothing*, use [`erase`].
///
/// **A caller that needs the fourth byte preserved wants [`fill`]**, and
/// should keep using it. This one is for the case that is actually a clear.
pub fn clear(canvas: &mut [u8], t: &Target, colour: [u8; 3]) {
    // The fourth byte: padding stays zero on `Xrgb8888` — writing `255` there
    // is what broke twenty-four baseline hashes on 2026-09-20 — and a cleared
    // `Argb8888` surface is opaque, because a clear is full coverage.
    let pattern = [colour[2], colour[1], colour[0], u8::from(t.format.writes_alpha()) * 255];
    for row in 0..t.height {
        let begin = row * t.stride;
        let end = begin + t.width * 4;
        let Some(row_bytes) = canvas.get_mut(begin..end) else {
            // A short canvas is drawn as far as it goes, which is `fill`'s
            // position and the reason given there: a kiosk that went black
            // because one frame was miscounted is worse than one that looks
            // wrong.
            return;
        };
        for pixel in row_bytes.chunks_exact_mut(4) {
            pixel.copy_from_slice(&pattern);
        }
    }
}

/// **Erase the whole surface to nothing** — transparent black.
///
/// # Why an overlay needs this and [`clear`] will not do
///
/// [`clear`] paints a *colour*. An overlay that Aura's widgets are drawn into
/// must begin as no colour at all, so that the GPU-rendered background beneath
/// it shows through everywhere the widgets are not — which is most of the
/// screen.
///
/// It is also the base case that makes premultiplied alpha come out right
/// without anybody multiplying anything. Blending a colour at coverage `a`
/// into zero gives `fg * a + 0 * (1 - a)`, which **is** the premultiplied
/// value, and every later blend preserves the property. Start from a
/// non-zero surface and that induction has no base; see
/// [`Format::Argb8888`](crate::text::Format::Argb8888).
///
/// On an `Xrgb8888` target this is [`clear`] to black and the fourth byte is
/// zero either way, so it is safe — just misnamed for what that caller wants.
pub fn erase(canvas: &mut [u8], t: &Target) {
    for row in 0..t.height {
        let begin = row * t.stride;
        let end = begin + t.width * t.bytes_per_pixel();
        let Some(row_bytes) = canvas.get_mut(begin..end) else {
            // Short canvas, same position as `clear` and `fill`: draw as far as
            // it goes rather than refuse the frame.
            return;
        };
        row_bytes.fill(0);
    }
}

// ---------------------------------------------------------------------------
// Discs and segments — added 2026-09-02 for `SPEC-04` §5.8's weather panel
// ---------------------------------------------------------------------------
//
// **Why these are here and not in the application that wanted them.**
//
// `ADR-036` §5.4 puts the kiosk surface in this crate, and a kiosk surface with
// rectangles and glyphs and no circle is one that every drawing application has
// to complete for itself. `SPEC-04` §11 (14) already named the absence — *"
// `rivet_sdk::draw` has no gradient and no circle"* — as the reason a card could
// not be ported. This closes the circle half of it. The gradient half is still
// open.
//
// **What they are NOT: a general vector renderer.** There is no path, no join,
// no cap and no fill rule. A caller that wants a joined polyline draws a disc at
// each vertex, which is what `cpp/src/widgets.hpp` does and says why:
// *"raylib has no line joins, so a disc is drawn at each vertex; without them
// the roof peak and eaves show notches at this stroke width."*
//
// # These are ANTIALIASED and raylib's are not
//
// Stated here rather than discovered. raylib draws a circle as a **36-sided
// polygon** through the GPU with no multisampling, and a thick line as one
// untriangulated quad; both have hard, stepped edges. These compute exact
// coverage, like every other shape in this module and like a glyph edge in
// `crate::text`.
//
// The difference is the same one `fill_rounded_alpha` already carries against
// `DrawRectangleRounded`, in the same direction: **smoother here than on the
// wall**. It was not chosen, it is not correctable without writing an aliased
// polygon rasteriser, and it is written down at both ends so that a port
// claiming pixel fidelity says where it is not.

/// How much of the pixel at `(px, py)` lies inside a disc.
///
/// # `sqrt`, not `hypot`, and it is not a micro-optimisation
///
/// `f32::hypot` is **an out-of-line libm call** on musl/aarch64, not an
/// instruction. It does scaling work to avoid overflowing in the squares — a
/// real concern for `hypot`'s general contract and a non-concern here, where
/// both operands are a pixel offset from a disc centre and bounded by the
/// bounding box the caller is already iterating. `sqrt` is a single `fsqrt`.
///
/// **This runs once per pixel of a `(2r+2)²` box, inside or out**, for every
/// disc and every rounded corner this crate draws. `aura-background-pack`'s
/// Conway's Game of Life is the case that makes it visible: it draws a disc
/// per **warm** cell, and `LIFE_FADE_SECONDS = 0.9` at four steps a second
/// means a cell keeps drawing for 3.6 steps after it dies — so the fading
/// trail, which is the whole point of that background, multiplies the disc
/// count several-fold over the live population.
///
/// Found 2026-09-20 while accounting for Life's 26.15 ms, and it is the second
/// finding of the same shape as [`clear`]'s: the cost was in a shared
/// primitive rather than in the thing that looked expensive.
fn disc_coverage(px: f32, py: f32, cx: f32, cy: f32, radius: f32) -> f32 {
    let (dx, dy) = (px - cx, py - cy);
    let distance = dx.mul_add(dx, dy * dy).sqrt() - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// **A filled disc**, centred on `(cx, cy)`.
///
/// **A rectangle whose colour changes across it** — raylib's
/// `DrawRectangleGradientH(int x, int y, int w, int h, Color left, Color right)`.
///
/// # Why this exists, and why it was the last thing missing
///
/// `SPEC-04` §11 (14) recorded `DrawRainbowCard` as unportable because *"`rivet_sdk::draw`
/// has neither a gradient fill nor a circle"*. Half of that went stale the day
/// the weather glyphs landed and brought [`circle`] with them; the gradient was
/// the real gap, and this is it.
///
/// **The alpha interpolates too**, which is not decoration: the rainbow card's
/// soft edge is four gradients from opaque black to *transparent* black, and a
/// primitive that carried one alpha for the whole rectangle could not draw it.
///
/// # The sampling, which is the only place two implementations can differ
///
/// raylib interpolates between vertex colours on the GPU, so the two ends are
/// at the rectangle's EDGES and a pixel takes the value at its own centre. This
/// samples at `(i + 0.5) / w` for the same reason — which also means a
/// one-pixel-wide rectangle takes the midpoint rather than needing a special
/// case for dividing by zero.
#[allow(clippy::too_many_arguments)] // a rectangle, two colours and two opacities
pub fn fill_gradient_h(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    from: ([u8; 3], u8),
    to: ([u8; 3], u8),
) {
    for column in 0..w {
        let along = (column as f32 + 0.5) / w as f32;
        let (colour, alpha) = lerp_stop(from, to, along);
        if alpha <= 0.0 {
            continue;
        }
        for row in 0..h {
            blend(canvas, t, x + column, y + row, colour, alpha);
        }
    }
}

/// [`fill_gradient_h`], down the rectangle instead of across it — raylib's
/// `DrawRectangleGradientV`.
#[allow(clippy::too_many_arguments)] // as above
pub fn fill_gradient_v(
    canvas: &mut [u8],
    t: &Target,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    from: ([u8; 3], u8),
    to: ([u8; 3], u8),
) {
    for row in 0..h {
        let along = (row as f32 + 0.5) / h as f32;
        let (colour, alpha) = lerp_stop(from, to, along);
        if alpha <= 0.0 {
            continue;
        }
        for column in 0..w {
            blend(canvas, t, x + column, y + row, colour, alpha);
        }
    }
}

/// One point on the way between two colours, alpha included.
///
/// Returns the alpha as the `0.0..=1.0` [`blend`] takes rather than as a byte,
/// so nothing rounds twice on the way to a pixel.
fn lerp_stop(from: ([u8; 3], u8), to: ([u8; 3], u8), along: f32) -> ([u8; 3], f32) {
    let along = along.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| {
        let a = f32::from(a);
        (b as f32 - a).mul_add(along, a) as u8
    };
    (
        [
            channel(from.0[0], to.0[0]),
            channel(from.0[1], to.0[1]),
            channel(from.0[2], to.0[2]),
        ],
        f32::from(channel(from.1, to.1)) / 255.0,
    )
}

/// raylib's `DrawCircle(int centerX, int centerY, float radius, Color)`. The
/// centre is a float here because the caller has one and truncating it is the
/// caller's decision to make: `cpp/src/widgets.hpp` casts some centres to `int`
/// and passes others through, and reproducing that means reproducing which.
///
/// A radius of zero or less draws nothing, rather than one pixel — a disc with
/// no radius is not a dot, and a caller that wanted a dot asked for the wrong
/// primitive.
pub fn circle(canvas: &mut [u8], t: &Target, cx: f32, cy: f32, radius: f32, colour: [u8; 3]) {
    circle_alpha(canvas, t, cx, cy, radius, colour, 255);
}

/// [`circle`], translucent. The shape's own coverage is multiplied by `alpha`,
/// so the edge stays smooth at every opacity instead of hardening as it fades —
/// [`fill_rounded_alpha`]'s rule, applied to a curve.
pub fn circle_alpha(
    canvas: &mut [u8],
    t: &Target,
    cx: f32,
    cy: f32,
    radius: f32,
    colour: [u8; 3],
    alpha: u8,
) {
    if radius <= 0.0 || alpha == 0 {
        return;
    }
    let opacity = f32::from(alpha) / 255.0;
    // The bounding box, clipped to the target once. Half a pixel of slack on
    // each side, because a pixel whose centre is outside the radius can still
    // be partly covered.
    let (x0, y0, x1, y1) = clipped_box(t, cx - radius - 1.0, cy - radius - 1.0, cx + radius + 1.0, cy + radius + 1.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let coverage = disc_coverage(x as f32 + 0.5, y as f32 + 0.5, cx, cy, radius);
            blend(canvas, t, x, y, colour, coverage * opacity);
        }
    }
}

/// **A straight segment of a given thickness, with SQUARE ends.**
///
/// raylib's `DrawLineEx`, which builds one quad of width `thick` centred on the
/// segment and nothing else — no cap, no join. Square ends are not a
/// simplification: `DrawIndoorIcon` draws a disc at every vertex *because* the
/// ends are square, and rounding them here would double every joint and change
/// the shape of the house.
///
/// A thickness at or below zero draws nothing. A zero-length segment draws
/// nothing, rather than a dot, for [`circle`]'s reason.
#[allow(clippy::too_many_arguments)]
pub fn line(
    canvas: &mut [u8],
    t: &Target,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    colour: [u8; 3],
) {
    line_alpha(canvas, t, x0, y0, x1, y1, thickness, colour, 255);
}

/// [`line`], translucent.
#[allow(clippy::too_many_arguments)]
pub fn line_alpha(
    canvas: &mut [u8],
    t: &Target,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    colour: [u8; 3],
    alpha: u8,
) {
    if thickness <= 0.0 || alpha == 0 {
        return;
    }
    let (dx, dy) = (x1 - x0, y1 - y0);
    let length = dx.hypot(dy);
    if length <= 0.0 {
        return;
    }
    // The segment's own axes: `ux` along it, `vx` across it. A pixel is inside
    // when its projection lies within the length and within half the thickness,
    // which is the same signed-distance idea `coverage` uses for a box, in a
    // rotated frame.
    let (ux, uy) = (dx / length, dy / length);
    let half = thickness / 2.0;
    let opacity = f32::from(alpha) / 255.0;
    let pad = half + 1.0;
    let (bx0, by0, bx1, by1) = clipped_box(
        t,
        x0.min(x1) - pad,
        y0.min(y1) - pad,
        x0.max(x1) + pad,
        y0.max(y1) + pad,
    );
    for y in by0..by1 {
        for x in bx0..bx1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (rx, ry) = (px - x0, py - y0);
            let along = rx.mul_add(ux, ry * uy);
            let across = rx.mul_add(-uy, ry * ux);
            // Signed distance on each axis of the rotated box: negative inside,
            // positive outside. **Both have to stay signed** — clamping the
            // along-axis one to zero before combining makes every interior
            // pixel come out at distance zero, which is half coverage
            // everywhere and a segment that looks like a smear.
            let da = (along - length).max(-along);
            let db = across.abs() - half;
            let outside = da.max(0.0).hypot(db.max(0.0));
            let inside = da.max(db).min(0.0);
            let distance = inside + outside;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            blend(canvas, t, x, y, colour, coverage * opacity);
        }
    }
}

/// A connected run of thin segments — raylib's `DrawLineStrip`, which draws
/// **one-pixel** lines and not thick ones.
///
/// Kept separate from [`line`] because the width is the whole difference: a
/// caller reaching for this wants the hairline that `DrawLineStrip` gives, and
/// spelling it as a loop of [`line`] calls at thickness 1 would be the same
/// thing said less clearly. Fewer than two points draws nothing.
pub fn line_strip(canvas: &mut [u8], t: &Target, points: &[(f32, f32)], colour: [u8; 3]) {
    for pair in points.windows(2) {
        let [(x0, y0), (x1, y1)] = pair else { continue };
        line(canvas, t, *x0, *y0, *x1, *y1, 1.0, colour);
    }
}

/// A pixel range clipped to the target, as a half-open box.
fn clipped_box(t: &Target, x0: f32, y0: f32, x1: f32, y1: f32) -> (usize, usize, usize, usize) {
    let lo = |v: f32| if v <= 0.0 { 0 } else { v.floor() as usize };
    let hi = |v: f32, bound: usize| {
        if v <= 0.0 {
            0
        } else {
            ((v.ceil() as usize) + 1).min(bound)
        }
    };
    (lo(x0), lo(y0), hi(x1, t.width), hi(y1, t.height))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use crate::text::Format;

    fn target(w: usize, h: usize) -> (Vec<u8>, Target) {
        (
            vec![0u8; w * h * 4],
            Target {
                width: w,
                height: h,
                stride: w * 4,
                format: Format::Xrgb8888,
            },
        )
    }

    fn red_at(canvas: &[u8], t: &Target, x: usize, y: usize) -> u8 {
        canvas[y * t.stride + x * t.bytes_per_pixel() + 2]
    }

    /// The middle is solid and the corner is not — which is the whole point of
    /// a rounded rectangle and the one thing worth asserting about the shape.
    #[test]
    fn a_rounded_rectangle_is_solid_inside_and_clipped_at_the_corner() {
        let (mut canvas, t) = target(40, 40);
        fill_rounded(&mut canvas, &t, 0, 0, 40, 40, 10, [255, 0, 0]);
        assert_eq!(red_at(&canvas, &t, 20, 20), 255, "the middle is not solid");
        assert_eq!(red_at(&canvas, &t, 0, 0), 0, "the corner was filled");
        assert_eq!(red_at(&canvas, &t, 39, 0), 0, "the corner was filled");
    }

    /// A radius of zero is a plain rectangle, corners included. The failure this
    /// catches is a `coverage` that never reaches 1.0 at an exact corner.
    #[test]
    fn a_radius_of_zero_fills_the_corners() {
        let (mut canvas, t) = target(8, 8);
        fill_rounded(&mut canvas, &t, 0, 0, 8, 8, 0, [255, 0, 0]);
        assert_eq!(red_at(&canvas, &t, 0, 0), 255);
        assert_eq!(red_at(&canvas, &t, 7, 7), 255);
    }

    /// **The corner is antialiased**, not stepped: somewhere along the arc there
    /// is a pixel that is neither empty nor full. A shape drawn by whole-pixel
    /// tests would have none, and would look like stairs on a television.
    #[test]
    fn the_corner_has_partial_pixels() {
        let (mut canvas, t) = target(40, 40);
        fill_rounded(&mut canvas, &t, 0, 0, 40, 40, 12, [255, 0, 0]);
        let partial = (0..14)
            .flat_map(|y| (0..14).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let v = red_at(&canvas, &t, *x, *y);
                v > 0 && v < 255
            })
            .count();
        assert!(partial > 0, "the corner is stepped, not antialiased");
    }

    /// A stroke leaves the middle alone. The failure this catches is an inner
    /// coverage that is wrong, which fills the shape instead of outlining it.
    #[test]
    fn a_stroke_does_not_fill_the_middle() {
        let (mut canvas, t) = target(40, 40);
        stroke_rounded(&mut canvas, &t, 0, 0, 40, 40, 10, 3, [255, 0, 0]);
        assert_eq!(red_at(&canvas, &t, 20, 20), 0, "the stroke filled the shape");
        assert!(red_at(&canvas, &t, 20, 1) > 0, "the top edge is missing");
    }

    // -----------------------------------------------------------------------
    // The reference implementations, and the test that holds the fast paths to
    // them
    // -----------------------------------------------------------------------
    //
    // **These are the functions as they were before 2026-09-01**, kept verbatim
    // and compiled only for tests. `SPEC-04` §10.3 is the reason the fast paths
    // exist; this is the reason they are safe to have.
    //
    // The failure they guard against is the one that is discovered on a
    // television: a fast path that is right to within one bit per channel, on
    // an edge pixel, in one theme. Nobody notices until a photograph is
    // compared with another photograph. A byte comparison notices immediately.

    /// **The two ends are the two colours**, and the middle is between them.
    #[test]
    fn a_horizontal_gradient_runs_from_one_colour_to_the_other() {
        let (mut c, t) = target(64, 8);
        fill_gradient_h(&mut c, &t, 0, 0, 64, 8, ([255, 0, 0], 255), ([0, 0, 255], 255));
        let at = |x: usize| {
            let i = 4 * x;
            [c[i + 2], c[i + 1], c[i]]
        };
        // Sampled at pixel centres, so neither end is exactly the stop - but
        // the first is overwhelmingly red and the last overwhelmingly blue.
        assert!(at(0)[0] > 240 && at(0)[2] < 15, "{:?}", at(0));
        assert!(at(63)[2] > 240 && at(63)[0] < 15, "{:?}", at(63));
        // And it is monotonic: red only falls, blue only rises.
        for x in 1..64 {
            assert!(at(x)[0] <= at(x - 1)[0], "red rose at {x}");
            assert!(at(x)[2] >= at(x - 1)[2], "blue fell at {x}");
        }
    }

    /// The vertical one is the same gradient turned ninety degrees, and every
    /// row is uniform across its width.
    #[test]
    fn a_vertical_gradient_is_uniform_across_each_row() {
        let (mut c, t) = target(8, 64);
        fill_gradient_v(&mut c, &t, 0, 0, 8, 64, ([255, 0, 0], 255), ([0, 0, 255], 255));
        for y in 0..64 {
            let first = y * t.stride;
            for x in 1..8 {
                let at = first + 4 * x;
                assert_eq!(
                    (c[at], c[at + 1], c[at + 2]),
                    (c[first], c[first + 1], c[first + 2]),
                    "row {y} is not uniform at {x}"
                );
            }
        }
    }

    /// **The ALPHA interpolates**, which is what the rainbow card's soft edge
    /// is made of: opaque black fading to transparent black. A primitive that
    /// carried one opacity for the whole rectangle could not draw it.
    #[test]
    fn a_gradient_can_fade_to_transparent() {
        let (mut c, t) = target(32, 4);
        // A white canvas, so a black fade is visible as a rise back to white.
        for byte in c.iter_mut() {
            *byte = 255;
        }
        fill_gradient_h(&mut c, &t, 0, 0, 32, 4, ([0, 0, 0], 255), ([0, 0, 0], 0));
        let at = |x: usize| c[4 * x];
        assert!(at(0) < 10, "the opaque end did not paint: {}", at(0));
        assert!(at(31) > 245, "the transparent end painted: {}", at(31));
        for x in 1..32 {
            assert!(at(x) >= at(x - 1), "the fade reversed at {x}");
        }
    }

    /// A one-pixel rectangle takes the midpoint rather than dividing by zero.
    #[test]
    fn a_gradient_one_pixel_wide_is_the_middle_of_the_two() {
        let (mut c, t) = target(4, 4);
        fill_gradient_h(&mut c, &t, 0, 0, 1, 4, ([0, 0, 0], 255), ([255, 255, 255], 255));
        assert!((120..=135).contains(&c[0]), "not the midpoint: {}", c[0]);
    }

    /// Nothing is drawn outside the rectangle, and a zero-sized one draws
    /// nothing at all rather than panicking on an empty range.
    #[test]
    fn a_gradient_stays_inside_its_rectangle() {
        let (mut c, t) = target(16, 16);
        fill_gradient_h(&mut c, &t, 4, 4, 8, 8, ([255, 0, 0], 255), ([255, 0, 0], 255));
        for y in 0..16 {
            for x in 0..16 {
                let inside = (4..12).contains(&x) && (4..12).contains(&y);
                let painted = c[y * t.stride + 4 * x + 2] > 0;
                assert_eq!(painted, inside, "at {x},{y}");
            }
        }
        // Zero width and zero height are both no-ops.
        fill_gradient_h(&mut c, &t, 0, 0, 0, 8, ([0, 255, 0], 255), ([0, 255, 0], 255));
        fill_gradient_v(&mut c, &t, 0, 0, 8, 0, ([0, 255, 0], 255), ([0, 255, 0], 255));
        assert!(c.iter().step_by(4).all(|g| *g == 0), "a zero-sized gradient painted");
    }

    fn blend_reference(
        canvas: &mut [u8],
        t: &Target,
        x: usize,
        y: usize,
        colour: [u8; 3],
        alpha: f32,
    ) {
        if x >= t.width || y >= t.height || alpha <= 0.0 {
            return;
        }
        let at = y * t.stride + x * t.bytes_per_pixel();
        if at + 3 >= canvas.len() {
            return;
        }
        let a = alpha.clamp(0.0, 1.0);
        let mix = |fg: u8, bg: u8| -> u8 {
            (f32::from(fg).mul_add(a, f32::from(bg) * (1.0 - a)) + 0.5) as u8
        };
        canvas[at] = mix(colour[2], canvas[at]);
        canvas[at + 1] = mix(colour[1], canvas[at + 1]);
        canvas[at + 2] = mix(colour[0], canvas[at + 2]);
    }

    fn fill_reference(
        canvas: &mut [u8],
        t: &Target,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        colour: [u8; 3],
    ) {
        for row in 0..h {
            for col in 0..w {
                blend_reference(canvas, t, x + col, y + row, colour, 1.0);
            }
        }
    }

    fn fill_rounded_reference(
        canvas: &mut [u8],
        t: &Target,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        radius: usize,
        colour: [u8; 3],
    ) {
        let (fx, fy) = (x as f32, y as f32);
        let (fw, fh) = (w as f32, h as f32);
        let r = (radius as f32).min(fw / 2.0).min(fh / 2.0);
        for row in 0..h {
            for col in 0..w {
                let a = coverage(
                    fx + col as f32 + 0.5,
                    fy + row as f32 + 0.5,
                    fx,
                    fy,
                    fw,
                    fh,
                    r,
                );
                blend_reference(canvas, t, x + col, y + row, colour, a);
            }
        }
    }

    fn dim_reference(canvas: &mut [u8], strength: u8) {
        let keep = u32::from(255 - strength);
        for byte in canvas.iter_mut() {
            *byte = u8::try_from(u32::from(*byte) * keep / 255).unwrap_or(*byte);
        }
    }

    /// A canvas that is **not blank**, so a fast path that forgot to blend
    /// against what was there cannot pass by writing over zeroes.
    fn textured(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 37 % 251) as u8).collect()
    }

    /// **The guard on `SPEC-04` §10.3's three fast paths.**
    ///
    /// Every case runs twice — once through the code that ships, once through
    /// the code it replaced — and the two buffers must be identical byte for
    /// byte. Not *close*: identical.
    #[test]
    fn the_fast_paths_are_byte_identical_to_the_general_ones() {
        let sizes = [(40usize, 40usize), (7, 3), (16, 16), (64, 41)];
        // The radii that matter: none, sub-pixel, ordinary, and one large
        // enough to clamp to a stadium.
        let radii = [0usize, 1, 3, 12, 24, 100];

        for (cw, ch) in sizes {
            let (_, t) = target(cw, ch);
            for &(x, y, w, h) in &[
                (0usize, 0usize, cw, ch),
                (2, 2, cw / 2, ch / 2),
                // Past every edge, so the clipping is compared too.
                (cw.saturating_sub(3), ch.saturating_sub(3), 40, 40),
                (0, 0, cw + 20, ch + 20),
            ] {
                let mut mine = textured(cw * ch * 4);
                let mut reference = mine.clone();
                fill(&mut mine, &t, x, y, w, h, [200, 100, 50]);
                fill_reference(&mut reference, &t, x, y, w, h, [200, 100, 50]);
                assert_eq!(mine, reference, "fill {cw}x{ch} at {x},{y} {w}x{h}");

                for radius in radii {
                    let mut mine = textured(cw * ch * 4);
                    let mut reference = mine.clone();
                    fill_rounded(&mut mine, &t, x, y, w, h, radius, [30, 220, 90]);
                    fill_rounded_reference(&mut reference, &t, x, y, w, h, radius, [30, 220, 90]);
                    assert_eq!(
                        mine, reference,
                        "fill_rounded {cw}x{ch} at {x},{y} {w}x{h} r={radius}"
                    );

                    // The stroke goes through the same `coverage` and the same
                    // `blend`, so it is compared as well — its inner edge is
                    // where an alpha fast path would show up first.
                    let mut mine = textured(cw * ch * 4);
                    let mut reference = mine.clone();
                    stroke_rounded(&mut mine, &t, x, y, w, h, radius, 3, [30, 220, 90]);
                    {
                        let (fx, fy) = (x as f32, y as f32);
                        let (fw, fh) = (w as f32, h as f32);
                        let r = (radius as f32).min(fw / 2.0).min(fh / 2.0);
                        let th = 3.0f32;
                        for row in 0..h {
                            for col in 0..w {
                                let (px, py) = (fx + col as f32 + 0.5, fy + row as f32 + 0.5);
                                let outer = coverage(px, py, fx, fy, fw, fh, r);
                                let inner = coverage(
                                    px,
                                    py,
                                    fx + th,
                                    fy + th,
                                    (fw - th * 2.0).max(0.0),
                                    (fh - th * 2.0).max(0.0),
                                    (r - th).max(0.0),
                                );
                                blend_reference(
                                    &mut reference,
                                    &t,
                                    x + col,
                                    y + row,
                                    [30, 220, 90],
                                    outer - inner,
                                );
                            }
                        }
                    }
                    assert_eq!(
                        mine, reference,
                        "stroke_rounded {cw}x{ch} at {x},{y} {w}x{h} r={radius}"
                    );
                }
            }

            // `dim` at every strength that could round differently.
            for strength in [0u8, 1, 63, 64, 127, 128, 129, 200, 254, 255] {
                let mut mine = textured(cw * ch * 4);
                let mut reference = mine.clone();
                dim(&mut mine, &t, strength);
                dim_reference(&mut reference, strength);
                assert_eq!(mine, reference, "dim strength={strength}");
            }
        }
    }

    /// The opaque fast path in [`blend`], on its own and at the exact boundary.
    /// `1.0` must take it and anything below must not.
    #[test]
    fn the_opaque_blend_matches_the_general_one_at_every_alpha() {
        let (_, t) = target(4, 4);
        for alpha in [
            0.0f32, 0.001, 0.25, 0.5, 0.75, 0.999, 0.999_999, 1.0, 1.5, -1.0,
        ] {
            let mut mine = textured(4 * 4 * 4);
            let mut reference = mine.clone();
            for y in 0..4 {
                for x in 0..4 {
                    blend(&mut mine, &t, x, y, [11, 222, 133], alpha);
                    blend_reference(&mut reference, &t, x, y, [11, 222, 133], alpha);
                }
            }
            assert_eq!(mine, reference, "alpha={alpha}");
        }
    }

    /// A shape drawn off the edge is clipped rather than wrapped. The failure
    /// this prevents is a card near the right edge writing into the next row.
    #[test]
    fn a_shape_past_the_edge_is_clipped() {
        let (mut canvas, t) = target(16, 16);
        fill_rounded(&mut canvas, &t, 12, 12, 40, 40, 4, [255, 0, 0]);
        // Nothing outside the canvas, and the canvas is intact where the shape
        // is not: row 0 was never touched.
        assert_eq!(red_at(&canvas, &t, 0, 0), 0);
    }
    // -----------------------------------------------------------------------
    // Discs and segments
    // -----------------------------------------------------------------------

    /// A disc is solid at its centre, empty outside its radius, and **round** —
    /// the four points on the axes at the radius are covered and the four
    /// diagonal corners of the same bounding box are not. A square drawn by a
    /// bug in the bounding box would pass the first two and fail this.
    #[test]
    fn a_disc_is_solid_in_the_middle_and_round_at_its_edge() {
        let (mut canvas, t) = target(41, 41);
        circle(&mut canvas, &t, 20.5, 20.5, 15.0, [255, 0, 0]);
        assert_eq!(red_at(&canvas, &t, 20, 20), 255, "the centre is not solid");
        for (x, y) in [(20, 8), (20, 32), (8, 20), (32, 20)] {
            assert!(red_at(&canvas, &t, x, y) > 0, "({x},{y}) is inside the disc");
        }
        // 15 / sqrt(2) is 10.6, so a corner 15 away on both axes is outside.
        for (x, y) in [(9, 9), (31, 9), (9, 31), (31, 31)] {
            assert_eq!(red_at(&canvas, &t, x, y), 0, "({x},{y}) is outside the disc");
        }
    }

    /// **The edge is antialiased**, like every other curve in this module.
    /// Stated as a test because it is the difference from raylib that
    /// `SPEC-04` reports, and a later change that made it hard-edged would be
    /// closing that gap without anybody deciding to.
    #[test]
    fn a_discs_edge_has_partial_pixels() {
        let (mut canvas, t) = target(41, 41);
        circle(&mut canvas, &t, 20.5, 20.5, 15.0, [255, 0, 0]);
        let partial = (0..41)
            .flat_map(|y| (0..41).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let v = red_at(&canvas, &t, *x, *y);
                v > 0 && v < 255
            })
            .count();
        assert!(partial > 8, "the disc is stepped, not antialiased");
    }

    /// A disc with no radius draws nothing — not one pixel. A caller that
    /// wanted a dot asked for the wrong primitive, and a stray pixel at a
    /// vertex is exactly the kind of mark that reads as a rendering fault.
    #[test]
    fn a_disc_with_no_radius_draws_nothing() {
        let (mut canvas, t) = target(9, 9);
        circle(&mut canvas, &t, 4.5, 4.5, 0.0, [255, 0, 0]);
        circle(&mut canvas, &t, 4.5, 4.5, -3.0, [255, 0, 0]);
        assert!(canvas.iter().all(|b| *b == 0), "something was drawn");
    }

    /// A disc partly off the target is clipped, not wrapped and not a panic.
    /// The negative-coordinate case is the one that matters: every coordinate
    /// this module indexes with is a `usize`, so the clip has to happen before
    /// the conversion.
    #[test]
    fn a_disc_over_an_edge_is_clipped_rather_than_wrapped() {
        let (mut canvas, t) = target(20, 20);
        circle(&mut canvas, &t, -5.0, -5.0, 12.0, [255, 0, 0]);
        assert!(red_at(&canvas, &t, 0, 0) > 0, "the visible part is missing");
        assert_eq!(red_at(&canvas, &t, 19, 19), 0, "it wrapped to the far corner");
    }

    /// **A thick line is a rectangle of the width asked for, with square ends.**
    ///
    /// Horizontal, so the arithmetic is checkable by hand: a 6px line down the
    /// middle covers rows 7 to 12 and leaves 6 and 13 alone, and nothing is
    /// drawn past either end. Square ends are not a simplification — the icons
    /// draw a disc at each joint *because* the ends are square.
    #[test]
    fn a_thick_line_is_as_wide_as_it_was_asked_for_and_ends_square() {
        let (mut canvas, t) = target(30, 20);
        line(&mut canvas, &t, 5.0, 10.0, 25.0, 10.0, 6.0, [255, 0, 0]);
        for y in 7..13 {
            assert_eq!(red_at(&canvas, &t, 15, y), 255, "row {y} should be solid");
        }
        assert_eq!(red_at(&canvas, &t, 15, 6), 0, "the line is too thick");
        assert_eq!(red_at(&canvas, &t, 15, 13), 0, "the line is too thick");
        assert_eq!(red_at(&canvas, &t, 3, 10), 0, "drawn before its start");
        assert_eq!(red_at(&canvas, &t, 27, 10), 0, "drawn past its end");
    }

    /// **The interior is opaque.** The failure this catches is a signed
    /// distance whose along-axis term was clamped to zero before being
    /// combined, which puts every interior pixel at coverage 0.5 — a line that
    /// is the right shape, the right width, and half a smear.
    #[test]
    fn a_diagonal_lines_interior_is_solid() {
        let (mut canvas, t) = target(40, 40);
        line(&mut canvas, &t, 5.0, 5.0, 35.0, 35.0, 8.0, [255, 0, 0]);
        assert_eq!(red_at(&canvas, &t, 20, 20), 255, "the middle is not solid");
        assert_eq!(red_at(&canvas, &t, 10, 10), 255, "near the start is not solid");
        assert_eq!(red_at(&canvas, &t, 30, 20), 0, "it is not a rectangle");
    }

    /// A zero-length or zero-thickness segment draws nothing rather than
    /// dividing by its own length.
    #[test]
    fn a_degenerate_segment_draws_nothing() {
        let (mut canvas, t) = target(9, 9);
        line(&mut canvas, &t, 4.0, 4.0, 4.0, 4.0, 3.0, [255, 0, 0]);
        line(&mut canvas, &t, 1.0, 4.0, 8.0, 4.0, 0.0, [255, 0, 0]);
        assert!(canvas.iter().all(|b| *b == 0), "something was drawn");
    }

    /// A strip joins its points in order and draws nothing for fewer than two.
    #[test]
    fn a_strip_joins_its_points_and_needs_two_of_them() {
        let (mut canvas, t) = target(20, 20);
        line_strip(&mut canvas, &t, &[(2.5, 2.5), (2.5, 17.5), (17.5, 17.5)], [255, 0, 0]);
        assert!(red_at(&canvas, &t, 2, 10) > 0, "the first segment is missing");
        assert!(red_at(&canvas, &t, 10, 17) > 0, "the second segment is missing");
        assert_eq!(red_at(&canvas, &t, 10, 5), 0, "it closed a shape nobody asked for");

        let (mut empty, t) = target(9, 9);
        line_strip(&mut empty, &t, &[(1.0, 1.0)], [255, 0, 0]);
        line_strip(&mut empty, &t, &[], [255, 0, 0]);
        assert!(empty.iter().all(|b| *b == 0));
    }

    /// Opacity multiplies the shape's own coverage, so a translucent disc is
    /// still a disc — the rule [`fill_rounded_alpha`] states for a corner,
    /// applied to a curve. Alpha zero draws nothing at all.
    #[test]
    fn opacity_multiplies_coverage_rather_than_hardening_the_edge() {
        let (mut canvas, t) = target(41, 41);
        circle_alpha(&mut canvas, &t, 20.5, 20.5, 15.0, [255, 0, 0], 128);
        let middle = red_at(&canvas, &t, 20, 20);
        assert!((120..=136).contains(&middle), "half opacity gave {middle}");

        let (mut none, t) = target(41, 41);
        circle_alpha(&mut none, &t, 20.5, 20.5, 15.0, [255, 0, 0], 0);
        line_alpha(&mut none, &t, 2.0, 2.0, 38.0, 38.0, 5.0, [255, 0, 0], 0);
        assert!(none.iter().all(|b| *b == 0), "alpha zero drew something");
    }
}

/// **Darken the whole canvas**, for something drawn in front of it.
///
/// `PLAN.md` §108.1. `strength` is out of 255: 0 leaves the frame alone and 255
/// is black. Multiplicative rather than a blend toward a colour, so a dimmed
/// frame keeps its hues and reads as *the same picture, further back* — which
/// is the whole job. Blending toward grey would make it look like a different
/// screen rather than the one underneath.
///
/// # ~~A table, not arithmetic~~ — TRIED, MEASURED, AND REVERTED 2026-09-01
///
/// This looks like the most obvious of the three hot paths `SPEC-04` §10.3
/// names: a multiply, a constant divide and a **fallible** `u8::try_from` per
/// byte across eight megabytes, with the fallible branch dead because the
/// product cannot exceed 255. There are 256 possible inputs and one `strength`,
/// so a 256-entry table built once replaces all of it.
///
/// **The table was 3.2 times SLOWER.** Measured on an x86-64 host, same build,
/// three runs each, stable to within 2%: **0.36 ms as written below, 1.17 ms
/// through a lookup.** The arithmetic auto-vectorises — the compiler lowers the
/// constant divide to a multiply and a shift and does sixteen bytes at a time —
/// and **a table lookup cannot be vectorised at all**, so the "optimisation"
/// traded a SIMD stream for a gather.
///
/// At 0.36 ms this is already moving 16.6 MB in and out, which is close to that
/// machine's bandwidth, so it was bandwidth-bound rather than arithmetic-bound
/// before anybody touched it. Recorded here rather than deleted, because the
/// next reader will have the same idea.
///
/// A quarter is genuinely available and is deliberately not taken: this touches
/// the fourth byte of every pixel, which `XRGB8888` does not scan out. Skipping
/// it would change the buffer without changing the picture — but a caller
/// keeping an alpha there (a cached card layer, for instance) would see a
/// different result, and that is a behaviour change rather than an
/// optimisation.
pub fn dim(canvas: &mut [u8], t: &Target, strength: u8) {
    let keep = u32::from(255 - strength);
    for byte in canvas.iter_mut() {
        *byte = u8::try_from(u32::from(*byte) * keep / 255).unwrap_or(*byte);
    }
    // `t` is taken for the signature every other helper here has, so a caller
    // does not have to remember that this one is the exception.
    let _ = t;
}

#[cfg(test)]
mod dim_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The two ends, because a dim that never darkens and one that always
    /// blacks out are both silently useless.
    #[test]
    fn dimming_spans_from_untouched_to_black() {
        let t = Target::xrgb(2, 1, 8);
        let mut none = vec![200u8; 8];
        dim(&mut none, &t, 0);
        assert!(none.iter().all(|b| *b == 200), "0 must change nothing");

        let mut all = vec![200u8; 8];
        dim(&mut all, &t, 255);
        assert!(all.iter().all(|b| *b == 0), "255 must reach black");

        let mut half = vec![200u8; 8];
        dim(&mut half, &t, 128);
        assert!(half.iter().all(|b| *b < 200 && *b > 0), "{half:?}");
    }

}
