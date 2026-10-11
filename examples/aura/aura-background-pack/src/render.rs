//! **The seam between the ported drawing code and the addon boundary.**
//!
//! `crate::background::Screensaver::draw` still speaks the language it always
//! has — a flat canvas and a `rivet_sdk::text::Target` — because that is
//! `crate::background`'s whole point: constant for constant, unchanged.
//! [`render_frame`] is the new glue that hands it a canvas exactly as
//! `aura-renderer`'s own `main.rs` does, then copies the result into a
//! [`Surface`], which is what `rivetos_addon::frame` actually hands an addon
//! (`.rulings/` R-199 D2's `#[repr(C)]` descriptor).
//!
//! **`evidence.rs` calls this same function**, so the 40-frame comparison
//! against `examples/aura/aura-renderer/evidence/T6-background-baseline/`
//! exercises the exact code this crate's `Drawing::draw` impl uses in
//! production — not a second, parallel path that happens to agree with it.
//!
//! # The flat path, and the intermediate canvas that is still there behind it
//!
//! **Since 2026-09-20 the common case allocates and copies nothing.**
//! `rivetos_addon::frame::Surface::packed_mut` answers with the whole visible
//! buffer when the host's stride is exactly `width * 4`, which is the shape
//! `Screensaver::draw` already wants, so the drawing goes straight into the
//! host's surface. The paragraphs below describe the path taken when it does
//! not — a host whose buffer has padding between rows — and that path now
//! reuses a canvas the caller owns rather than allocating one per frame.
//!
//! This was not a speculative optimisation. T7 put this function on
//! `aura-renderer`'s hot path for the first time, and the cost it had been
//! carrying unmeasured is an 8.29 MB allocation and an 8.29 MB copy **per
//! frame** — 249 MB a second at 30 fps, against a 33.3 ms budget of which
//! Digital Rain's own drawing already spends 12.4 ms (`SPEC-04` §10).
//! `PLAN-Addon-Feature-Set` T8 says a background that misses the budget is a
//! defect and not a preference; the same standard applies to the glue.
//!
//! # Why an intermediate canvas is still needed for a padded surface
//!
//! [`Surface::row`] hands back a row's **visible** bytes only — `width * 4` —
//! and deliberately has no way to reach the padding between one row and the
//! next, which is what lets a probe prove an addon stayed inside its lines
//! (`rivet-addon/src/frame.rs`'s own module doc comment). `Screensaver::draw`
//! and everything under it addresses pixels through a `Target { stride, .. }`
//! that assumes it can reach a whole padded buffer — `rivet_sdk::draw::fill`
//! computes `row * stride + x * bytes_per_pixel` directly into the slice it
//! is given. Reconciling those two shapes by rewriting the ported drawing
//! code against `Surface`'s narrower API would be exactly the kind of
//! "improvement on the way across" T6's brief forbids. So instead: draw into
//! an intermediate buffer whose own stride is `width * 4` — tightly packed,
//! no padding of its own — then copy each of its rows into the `Surface`'s
//! corresponding row. The two are the same length by construction
//! (`width * 4` either way), so the copy can never write past what
//! `Surface::row` already bounds-checked.
//!
//! On a padded surface this still costs one straight per-row copy. It no
//! longer costs an allocation: the canvas is a `&mut Vec<u8>` the caller
//! keeps between frames and this function grows once, on the first frame of a
//! given size, and then reuses.

use rivetos_addon::frame::{Refusal, Surface};
use rivet_sdk::text::Target;

use crate::background::Screensaver;
use crate::typeface::Typeface;

/// Draw one frame of `saver` into `surface`, through the exact same
/// `Screensaver::draw` every host — real or `evidence.rs` — reaches.
///
/// # Errors
///
/// [`Refusal::UnsupportedSurface`] when `surface`'s own height disagrees with
/// itself between one call and the next in a way that leaves a row
/// unaddressable. Not reachable through `rivetos_addon::frame::dispatch` in
/// practice — that function builds every `Surface` through
/// [`Surface::over`], which already refused a geometry that could not
/// address its own rows — so this arm exists for a caller that built a
/// `Surface` some other way, such as a future test.
pub fn render_frame(
    saver: &Screensaver,
    surface: &mut Surface<'_>,
    rain_face: &Typeface,
    canvas: &mut Vec<u8>,
) -> Result<(), Refusal> {
    let width = surface.width() as usize;
    let height = surface.height() as usize;
    // Tightly packed — no padding of its own — which is both what the flat
    // path below requires of the host's buffer and what makes the copy in the
    // fallback a per-row slice of matching length rather than a strided one.
    let target = Target::xrgb(width, height, width * 4);

    // **The flat path: the host's buffer is already the shape we wanted.**
    // Nothing is allocated, nothing is copied, and the picture is identical —
    // `rivet-addon`'s `writing_flat_and_writing_by_row_agree` is what holds
    // the two paths to that.
    if let Some(bytes) = surface.packed_mut() {
        saver.draw(bytes, &target, rain_face);
        return Ok(());
    }

    // A padded surface. Draw into the caller's canvas and copy the visible
    // bytes of each row across, leaving the host's padding untouched.
    let span = target.stride * target.height;
    if canvas.len() != span {
        canvas.clear();
        canvas.resize(span, 0);
    }
    saver.draw(canvas, &target, rain_face);
    for y in 0..height {
        let row = surface
            .row(u32::try_from(y).unwrap_or(u32::MAX))
            .ok_or(Refusal::UnsupportedSurface)?;
        let start = y * target.stride;
        row.copy_from_slice(&canvas[start..start + target.stride]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use crate::background::{Background, Solid};
    use crate::rng::Mt19937;

    /// A frame lands in the `Surface` at all, and the padding a host left
    /// beside a narrower-than-visible buffer is untouched — the same
    /// property `rivet-addon/src/frame.rs`'s own tests hold `dispatch` to.
    #[test]
    fn a_frame_reaches_the_surface_and_leaves_padding_alone() {
        let saver = Screensaver::new_with_rng(
            Background::Solid(Solid::Navy),
            4,
            2,
            Mt19937::new(1),
        );
        let rain_face = Typeface::load_embedded(18.0).expect("the embedded rain face");
        // Stride wider than the visible width, padded with a sentinel.
        let stride = 4 * 4 + 8;
        let mut bytes = vec![0xABu8; stride * 2];
        let mut surface = Surface::over(&mut bytes, 4, 2, stride as u32).expect("a surface");
        // A padded surface, so this exercises the fallback path and the
        // canvas is really used.
        let mut canvas = Vec::new();
        render_frame(&saver, &mut surface, &rain_face, &mut canvas)
            .expect("solid draws unconditionally");
        assert_eq!(canvas.len(), 4 * 4 * 2, "the scratch canvas was grown once");

        // Navy is [10, 18, 40] — XRGB8888 little-endian puts blue lowest.
        assert_eq!(&bytes[0..4], &[40, 18, 10, 0]);
        // The padding after each row's visible bytes is the host's sentinel,
        // never written.
        for row in 0..2usize {
            let start = row * stride + 4 * 4;
            assert!(
                bytes[start..start + 8].iter().all(|b| *b == 0xAB),
                "row {row}'s padding was written"
            );
        }
    }

    /// **The flat path and the padded path draw the same picture**, and the
    /// flat one never touches the scratch canvas.
    ///
    /// This is the assertion that lets `render_frame` branch on the host's
    /// stride at all. The two paths are chosen by a property of somebody
    /// else's buffer — a DRM driver's alignment — so a difference between
    /// them would not be a bug this crate could reproduce on demand. It would
    /// be a bug that appears on one board and not another.
    #[test]
    fn the_flat_path_draws_what_the_padded_path_draws() {
        let saver = || Screensaver::new_with_rng(Background::Solid(Solid::Navy), 4, 2, Mt19937::new(1));
        let rain_face = Typeface::load_embedded(18.0).expect("the embedded rain face");

        // Tightly packed: `packed_mut` answers and nothing is copied.
        let mut flat = vec![0u8; 4 * 4 * 2];
        let mut canvas = Vec::new();
        {
            let mut surface = Surface::over(&mut flat, 4, 2, 4 * 4).expect("a surface");
            render_frame(&saver(), &mut surface, &rain_face, &mut canvas).expect("draws");
        }
        assert!(
            canvas.is_empty(),
            "the flat path allocated a canvas it did not need"
        );

        // Padded by eight bytes a row: the fallback runs.
        let stride = 4 * 4 + 8;
        let mut padded = vec![0u8; stride * 2];
        {
            let mut surface = Surface::over(&mut padded, 4, 2, stride as u32).expect("a surface");
            render_frame(&saver(), &mut surface, &rain_face, &mut canvas).expect("draws");
        }

        for row in 0..2usize {
            let visible = row * stride;
            assert_eq!(
                &padded[visible..visible + 4 * 4],
                &flat[row * 4 * 4..row * 4 * 4 + 4 * 4],
                "row {row} differs between the flat and the padded path"
            );
        }
    }
}
