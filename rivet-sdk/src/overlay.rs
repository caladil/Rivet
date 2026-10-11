//! **The widget layer: a `wl_shm` subsurface over whatever the background is.**
//!
//! # What it is for, and the trap it answers
//!
//! `GPU-Backgrounds-2026-09-20` §3 named the trap: *"a half-GPU renderer is
//! slower than an all-CPU one"*, because a frame drawn on the GPU has to come
//! **back** to main memory for the CPU to draw widgets on it, and the readback
//! of 8.29 MB at 1080p costs more than the time it was trying to save. Written
//! under a DRM lease, which leaves exactly two options — everything on the GPU
//! or nothing — §3 sized the first at 3,400 lines of `widgets.rs` and
//! `layout.rs` rewritten in GL.
//!
//! **A compositor supplies a third.** The background is the parent surface;
//! the widgets are a subsurface on top of it, drawn by exactly the CPU code
//! that draws them today. `rivet-displayd` blends the two, because blending
//! surfaces is what a compositor is. **Nothing is read back and nothing is
//! ported.**
//!
//! # It pays on a CPU background too, and that is not the obvious part
//!
//! The readback argument is about a GPU parent. The saving below is about
//! **cadence**, and it applies whatever the parent is:
//!
//! > Aura redraws its widgets thirty times a second because they are
//! > composited into the same buffer as an animated background. Its clock
//! > changes **once a minute**. `PLAN.md` §181's soak measures those widgets
//! > at 11.2 ms of a 19.5 ms drawing budget — more than the background — and
//! > all but one frame in eighteen hundred is drawing a picture identical to
//! > the last one.
//!
//! On its own surface the widget layer is committed when it CHANGES. The
//! compositor keeps showing the last buffer for free.
//!
//! # Why this module exists rather than a method on each `Screen`
//!
//! Both [`crate::composited::Screen`] and [`crate::gpu::Screen`] want it and
//! neither wants the other's parent. The first commit of this wrote it into
//! `gpu` alone; moving it here when the second caller appeared is the
//! maintainer's rule of unifying the two copies before adding an option to tell
//! them apart (`.rulings/` R-356) — the option is what an agent reaches for to
//! avoid reading the other copy.

use smithay_client_toolkit::reexports::client::protocol::{wl_shm, wl_subsurface, wl_surface};
use smithay_client_toolkit::reexports::client::QueueHandle;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::subcompositor::SubcompositorState;

/// The widgets' surface and the pool behind it.
pub(crate) struct Overlay {
    surface: wl_surface::WlSurface,
    /// Held so the subsurface lives as long as the surface does. Never read:
    /// everything it is configured with is set once, at creation.
    _subsurface: wl_subsurface::WlSubsurface,
    pool: SlotPool,
    buffer: Option<Buffer>,
    /// The width, height and stride `buffer` was created with.
    ///
    /// `canvas` compares this against what is asked for on every call. A
    /// released buffer the compositor no longer holds is otherwise
    /// indistinguishable from one that still matches the caller's request —
    /// `pool.canvas` succeeds on both — and only this says whether what it
    /// hands back still describes the requested surface.
    geometry: Option<(usize, usize, usize)>,
}

impl Overlay {
    /// Create a widget layer over `parent`.
    ///
    /// # Errors
    /// A pool the compositor would not create.
    pub(crate) fn create<D>(
        subcompositor: &SubcompositorState,
        parent: &wl_surface::WlSurface,
        shm: &Shm,
        handle: &QueueHandle<D>,
    ) -> Result<Self, String>
    where
        D: smithay_client_toolkit::reexports::client::Dispatch<
                wl_surface::WlSurface,
                smithay_client_toolkit::compositor::SurfaceData,
            > + smithay_client_toolkit::reexports::client::Dispatch<
                wl_subsurface::WlSubsurface,
                smithay_client_toolkit::subcompositor::SubsurfaceData,
            > + 'static,
    {
        let (subsurface, surface) = subcompositor.create_subsurface(parent.clone(), handle);
        // **Above the parent, explicitly.** A subsurface is BELOW its parent by
        // default, which would put the widgets behind the background and look
        // precisely like widgets that had stopped being drawn.
        subsurface.place_above(parent);
        subsurface.set_position(0, 0);
        // **Desynchronised.** Synchronised is the default and would make this
        // surface's commits wait for the parent's — tying a clock that changes
        // once a minute to a background that changes thirty times a second,
        // and throwing away the whole saving described in this module's header.
        subsurface.set_desync();
        let pool = SlotPool::new(4, shm)
            .map_err(|e| format!("the overlay's shared-memory pool could not be created: {e}"))?;
        Ok(Self {
            surface,
            _subsurface: subsurface,
            pool,
            buffer: None,
            geometry: None,
        })
    }

    /// The bytes to draw the widgets into — **`ARGB8888`, premultiplied**.
    ///
    /// The fourth byte is alpha and a caller must maintain it:
    /// `rivet_sdk::text::Format::Argb8888` and a `Target` built with
    /// [`crate::text::Target::argb`] are what make the drawing primitives do
    /// that. Bytes left at zero are fully transparent, which is what most of
    /// this surface should be.
    ///
    /// # Errors
    /// A pool that would give no buffer.
    pub(crate) fn canvas(&mut self, width: usize, height: usize) -> Result<&mut [u8], String> {
        let (w, h) = (width.max(1), height.max(1));
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let (wi, hi, stride) = (w as i32, h as i32, (w * 4) as i32);
        #[allow(clippy::cast_sign_loss)]
        let wanted = (w, h, stride as usize);

        if needs_fresh_buffer(self.geometry, wanted) {
            // **Unconditional, independent of whether the compositor still
            // holds the OLD buffer.** A resize is a reason to replace on its
            // own — reusing a released buffer of the wrong size would hand a
            // caller a slice and a stride that no longer describe what
            // `present` is about to attach, exactly as `composited::paint`
            // treats its own resize as a reason to reallocate rather than
            // asking `pool.canvas` first.
            let (buffer, _) = self
                .pool
                .create_buffer(wi, hi, stride, wl_shm::Format::Argb8888)
                .map_err(|e| format!("a {w}x{h} overlay buffer could not be created: {e}"))?;
            self.buffer = Some(buffer);
            self.geometry = Some(wanted);
        } else if let Some(buffer) = self.buffer.as_mut() {
            // The geometry is unchanged, so the only question left is whether
            // the compositor still holds this buffer; a second of the SAME
            // size is made rather than waiting, exactly as
            // `composited::paint` does. Written as two statements because the
            // borrow checker will not let one `match` hold `pool` while its
            // arm asks `pool` for another buffer.
            if self.pool.canvas(buffer).is_none() {
                let (second, _) = self
                    .pool
                    .create_buffer(wi, hi, stride, wl_shm::Format::Argb8888)
                    .map_err(|e| format!("a second {w}x{h} overlay buffer failed: {e}"))?;
                *buffer = second;
            }
        }

        self.pool
            .canvas(self.buffer.as_mut().ok_or("no overlay buffer")?)
            .ok_or_else(|| "the overlay buffer is still held by the compositor".to_string())
    }

    /// Attach, damage and commit the widget layer.
    ///
    /// # Errors
    /// If nothing has been drawn into it yet, or the attach is refused.
    pub(crate) fn present(&mut self, width: usize, height: usize) -> Result<(), String> {
        let (w, h) = (
            i32::try_from(width).unwrap_or(0),
            i32::try_from(height).unwrap_or(0),
        );
        let Some(buffer) = self.buffer.as_ref() else {
            return Err("nothing has been drawn into the overlay".to_string());
        };
        self.surface.damage_buffer(0, 0, w, h);
        buffer
            .attach_to(&self.surface)
            .map_err(|e| format!("the overlay buffer could not be attached: {e}"))?;
        self.surface.commit();
        Ok(())
    }
}

/// Whether `canvas` must replace the buffer outright, decided without asking
/// the compositor anything.
///
/// **Independent of whether the compositor still holds the current buffer.**
/// A resize is a reason to replace on its own: reusing a released buffer of
/// the wrong size would hand a caller a slice and a stride that no longer
/// describe the surface `present` is about to attach. When the geometry is
/// unchanged, what remains to decide is whether the compositor still holds
/// the buffer, and `canvas` asks that directly rather than through this
/// function.
fn needs_fresh_buffer(current: Option<(usize, usize, usize)>, wanted: (usize, usize, usize)) -> bool {
    current != Some(wanted)
}

#[cfg(test)]
mod tests {
    use super::needs_fresh_buffer;

    /// No buffer yet is always a reason to make one.
    #[test]
    fn no_buffer_yet_needs_a_fresh_one() {
        assert!(needs_fresh_buffer(None, (100, 100, 400)));
    }

    /// **A grow is a reason to replace, regardless of the held-buffer
    /// question.** This is the regression: a released buffer of the OLD size
    /// otherwise looks perfectly reusable to `pool.canvas`.
    #[test]
    fn a_grow_needs_a_fresh_buffer() {
        assert!(needs_fresh_buffer(Some((100, 100, 400)), (200, 100, 800)));
    }

    /// A shrink is the same decision as a grow.
    #[test]
    fn a_shrink_needs_a_fresh_buffer() {
        assert!(needs_fresh_buffer(Some((200, 200, 800)), (100, 100, 400)));
    }

    /// **The same total pixel count at a different width is still a
    /// resize.** Byte count alone cannot stand in for geometry: a caller
    /// indexing by row would read the wrong pixel from every row but the
    /// first if width and stride were not both checked.
    #[test]
    fn a_same_area_different_width_needs_a_fresh_buffer() {
        assert!(needs_fresh_buffer(Some((100, 200, 400)), (200, 100, 800)));
    }

    /// **The branch this fix must leave alone.** Unchanged geometry is not by
    /// itself a reason to replace anything — whether the compositor still
    /// holds the buffer is `canvas`'s own next question, asked only once this
    /// is `false`.
    #[test]
    fn unchanged_geometry_does_not_need_a_fresh_buffer() {
        assert!(!needs_fresh_buffer(Some((100, 100, 400)), (100, 100, 400)));
    }
}
