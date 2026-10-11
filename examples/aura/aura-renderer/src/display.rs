//! **The surface this renderer draws on, which is a GPU one when there is
//! one.**
//!
//! # Why there are two and not one
//!
//! `.rulings/` R-206 D3: the x86 machine in this project's long-term targets
//! has no V3D. A renderer that required a GPU surface would show nothing
//! there, and a renderer that never asked for one could never draw R-207's
//! GPU backgrounds. So it asks, and takes what it gets.
//!
//! **The choice is made once, at start-up, and never again.** Which surface
//! kind a machine can give does not change while it runs, and rebuilding a
//! surface mid-flight would mean rebuilding the overlay and the buffer pool
//! with it.
//!
//! # Why a CPU background does NOT get a `wl_shm` surface
//!
//! It is the obvious design and it is wrong. R-207 ships Digital Rain and
//! Starfield on the CPU and Contours, Life and Boxes on the GPU **from one
//! pack, switchable by an operator from the portal**. Picking the surface
//! kind per background would mean tearing down and rebuilding the surface,
//! the overlay and the pool every time somebody changed a picker — on a
//! machine whose whole job is to keep a picture on a wall.
//!
//! So a machine with a V3D takes the GPU surface for everything, and a CPU
//! background reaches it through `rivet_sdk::gpu::Screen::show_pixels` — an
//! upload, which is the cheap direction. The readback that
//! `GPU-Backgrounds-2026-09-20` §3 rules out is the other one.

use rivet_sdk::composited;
use rivet_sdk::gpu;

/// Whichever surface this machine could give.
///
/// **Both variants are boxed.** `composited::Screen` is several hundred bytes
/// and a boxed GPU screen is one pointer, so an unboxed `Shm` makes every
/// `Display` that large whichever kind it holds. There is one per program, so
/// the box costs one allocation at start-up and nothing after.
pub enum Display {
    /// A dma-buf surface with a current GL context. Backgrounds that declare
    /// a GPU path draw straight into it; the rest are uploaded.
    Gpu(Box<gpu::Screen>),
    /// A `wl_shm` surface. Every background draws on the CPU, which is what
    /// this renderer did before 2026-09-21 and what a machine with no render
    /// node still does.
    Shm(Box<composited::Screen>),
}

impl Display {
    /// **Ask for a GPU surface, fall back to a shared-memory one.**
    ///
    /// The fallback is silent in the sense that it is not a fault — but the
    /// caller is handed the reason so it can say which it got and why, which
    /// is the difference between a board that is slow and a board nobody can
    /// explain.
    ///
    /// # Errors
    /// Only when NEITHER can be taken, and then both refusals are carried:
    /// the GPU one usually names the real problem and the `wl_shm` one
    /// usually says the compositor is not running.
    pub fn take() -> Result<(Self, Option<String>), String> {
        match gpu::take() {
            Ok(screen) => Ok((Self::Gpu(Box::new(screen)), None)),
            Err(gpu_why) => match composited::take() {
                Ok(screen) => Ok((Self::Shm(Box::new(screen)), Some(gpu_why))),
                Err(shm_why) => Err(format!(
                    "no surface of either kind — GPU: {gpu_why}; shared memory: {shm_why}"
                )),
            },
        }
    }

    pub const fn width(&self) -> usize {
        match self {
            Self::Gpu(s) => s.width,
            Self::Shm(s) => s.width,
        }
    }

    pub const fn height(&self) -> usize {
        match self {
            Self::Gpu(s) => s.height,
            Self::Shm(s) => s.height,
        }
    }

    /// **`width * 4` on the GPU path and the compositor's choice on the other.**
    ///
    /// The GPU path's canvas is this program's own and
    /// `Screen::show_pixels` requires it tightly packed — GLES 2.0 has no
    /// `GL_UNPACK_ROW_LENGTH` to describe padding with.
    pub const fn stride(&self) -> usize {
        match self {
            Self::Gpu(s) => s.width * 4,
            Self::Shm(s) => s.stride,
        }
    }

    pub fn what(&self) -> &str {
        match self {
            Self::Gpu(s) => &s.what,
            Self::Shm(s) => &s.what,
        }
    }

    /// Whether a background that declares a GPU path can actually take it.
    pub const fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }

    pub fn closed(&self) -> bool {
        match self {
            Self::Gpu(s) => s.closed(),
            Self::Shm(s) => s.closed(),
        }
    }

    /// **Make this surface's context current**, where there is one.
    ///
    /// Called at the top of every frame on the GPU path and a no-op on the
    /// other. It is the host's half of `rivetos_addon::gl`'s contract: by the
    /// time a pack's `draw_gl` is entered the context must be current and the
    /// target bound.
    ///
    /// # Errors
    /// EGL's own refusal.
    pub fn begin(&mut self) -> Result<(), String> {
        match self {
            Self::Gpu(s) => s.make_current(),
            Self::Shm(_) => Ok(()),
        }
    }

    /// The target a pack draws a GPU frame into, or `None` on a `wl_shm`
    /// surface.
    ///
    /// **Framebuffer zero**, because this host's target IS the surface it is
    /// about to present — see `rivetos_addon::gl::GlFrame::framebuffer` on
    /// why that is legal and why forcing a render-to-texture here would cost
    /// a full-screen pass to satisfy a word rather than a reason.
    pub fn gl_target(&self) -> Option<rivetos_addon::GlTarget> {
        match self {
            Self::Gpu(s) => {
                let (w, h) = (u32::try_from(s.width).ok()?, u32::try_from(s.height).ok()?);
                rivetos_addon::GlTarget::over(w, h, 0)
            }
            Self::Shm(_) => None,
        }
    }

    /// **Put a CPU-drawn frame on the surface.**
    ///
    /// On `wl_shm` this is the whole presentation. On the GPU path it is an
    /// upload and a full-screen blit, and [`Display::finish`] presents.
    ///
    /// # Errors
    /// A canvas of the wrong size, or a shader the driver would not build.
    pub fn show_cpu(&mut self, canvas: &[u8]) -> Result<(), String> {
        match self {
            Self::Gpu(s) => {
                let (w, h) = (s.width, s.height);
                s.show_pixels(canvas, w, h)
            }
            Self::Shm(s) => {
                s.blit(canvas);
                Ok(())
            }
        }
    }

    /// **Hand the finished frame over.**
    ///
    /// A no-op on `wl_shm`, where [`Display::show_cpu`] already committed —
    /// `composited::Screen::blit` attaches, damages and commits in one call
    /// and there is nothing left to do.
    ///
    /// # Errors
    /// `eglSwapBuffers`, or a buffer the compositor would not take.
    pub fn finish(&mut self) -> Result<(), String> {
        match self {
            Self::Gpu(s) => s.present(),
            Self::Shm(_) => Ok(()),
        }
    }

    /// Ask for the widget layer. See `rivet_sdk::overlay`.
    ///
    /// # Errors
    /// A compositor with no `wl_subcompositor`, or a pool it would not make.
    pub fn take_overlay(&mut self) -> Result<(), String> {
        match self {
            Self::Gpu(s) => s.take_overlay(),
            Self::Shm(s) => s.take_overlay(),
        }
    }

    /// The widget layer's pixels — `ARGB8888`, premultiplied.
    ///
    /// # Errors
    /// If the layer was never taken, or the pool would give no buffer.
    pub fn overlay(&mut self) -> Result<&mut [u8], String> {
        match self {
            Self::Gpu(s) => s.overlay(),
            Self::Shm(s) => s.overlay(),
        }
    }

    /// Commit the widget layer.
    ///
    /// # Errors
    /// If the layer was never taken, or nothing has been drawn into it.
    pub fn present_overlay(&mut self) -> Result<(), String> {
        match self {
            Self::Gpu(s) => s.present_overlay(),
            Self::Shm(s) => s.present_overlay(),
        }
    }
}
