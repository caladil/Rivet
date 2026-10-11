//! **Shapes** — rounded rectangles, and the arithmetic that makes their corners
//! not look like a mistake.
//!
//! `PLAN.md` §104.38. The maintainer's direction: each application's card is a
//! rounded rectangle, with a nice colored border around the selected card
//! (.rulings/ R-336).
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
pub fn blend(canvas: &mut [u8], t: &Target, x: usize, y: usize, colour: [u8; 3], alpha: f32) {
    if x >= t.width || y >= t.height || alpha <= 0.0 {
        return;
    }
    let at = y * t.stride + x * t.bytes_per_pixel;
    if at + 3 >= canvas.len() {
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
pub fn coverage(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
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
    let (fx, fy) = (x as f32, y as f32);
    let (fw, fh) = (w as f32, h as f32);
    // A radius larger than half the shorter side is not a rounded rectangle, it
    // is a stadium; clamped rather than refused, because a caller that asked for
    // one should get the nearest thing that exists.
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
            blend(canvas, t, x + col, y + row, colour, a);
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


#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn target(w: usize, h: usize) -> (Vec<u8>, Target) {
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

    fn red_at(canvas: &[u8], t: &Target, x: usize, y: usize) -> u8 {
        canvas[y * t.stride + x * t.bytes_per_pixel + 2]
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
}

