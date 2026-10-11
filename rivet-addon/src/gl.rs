//! **The drawing boundary for a pack that draws with the GPU** — `.rulings/`
//! R-206 D1.
//!
//! [`crate::frame`] hands a pack a buffer of pixels. This hands it a **bound
//! GL framebuffer** instead, and the difference is the whole of R-206:
//!
//! > *"The host creates the texture and binds the framebuffer; the pack is
//! > handed a bound target and draws into it. The host owns the EGL context,
//! > the GL state and the compositing."*
//!
//! # THE HOST OWNS THE CONTEXT
//!
//! **The pack never creates a context, never makes one current, never calls
//! `eglSwapBuffers`, and never presents anything.** By the time
//! [`DrawingGl::draw_gl`] is entered the host has already made its context
//! current, created the target, bound it, and set the viewport to
//! [`GlTarget::width`] by [`GlTarget::height`]. The pack issues drawing
//! commands and returns.
//!
//! That is the sentence [`crate::frame`]'s header spells as *"the host owns
//! the surface"*, said about a context instead of a buffer, and it is the
//! reason option A was rejected: a pack that owned an EGL context would put a
//! driver state machine across a `dlopen` boundary, where a version skew
//! between the host's Mesa and the pack's is not a compile error but a
//! deadlock on a board with no console.
//!
//! # How the pack reaches GL at all
//!
//! **It links `libGLESv2` itself and calls it directly.** No function-pointer
//! table crosses this boundary and none needs to: a pack is `dlopen`ed into
//! the host's own address space, so the context the host made current *is*
//! the pack's current context, and `glDrawArrays` in the pack is the same
//! `glDrawArrays` the host would call. A table would be a second ABI to
//! version for no property the process boundary does not already give.
//!
//! # GL state is NOT restored, deliberately
//!
//! A pack may bind what it likes, change blend modes, leave a program
//! current. The host re-establishes every piece of state it depends on at the
//! top of its own drawing, because it has to do that anyway — it cannot trust
//! a third-party object to unwind state correctly, and a contract that says
//! *"put it back"* is one whose violations are invisible until a different
//! background is selected.
//!
//! **What the pack may NOT do** is delete or unbind the framebuffer it was
//! given, or keep [`GlTarget::framebuffer`] past the call. The host owns that
//! object and may recreate it between frames — on a mode change, on a new
//! surface, or because it felt like it.
//!
//! # Why this is a separate symbol and not a flag on the old one
//!
//! Same reason [`crate::export_drawing_addon!`] is a separate macro from
//! [`crate::export_addon!`], given in that macro's own note: the presence of
//! a symbol is how a host learns what an object can do, and an entry point
//! that exists but refuses every call is a lie told in the one place a host
//! has to trust. A pack that exports [`DRAW_GL_SYMBOL`] can draw with the
//! GPU. A pack that does not, cannot, and [`crate::host::Loaded::draws_gl`]
//! is exactly that question.
//!
//! **Which of its backgrounds it draws that way is a different question**,
//! answered in the catalogue rather than here — see
//! [`crate::protocol::Drawn`]. R-207 ships one pack whose Contours, Life and
//! Boxes go through this module and whose Digital Rain and Starfield go
//! through [`crate::frame`], so the capability cannot be a property of the
//! object alone.

use crate::frame::{
    id_str, Refusal, Scene, DREW, ID_BYTES, PANICKED, REFUSED_DESCRIPTOR, REFUSED_VERSION,
};

/// The GL frame descriptor's own version, carried in [`GlFrame::abi`] and
/// answered by [`GL_VERSION_SYMBOL`].
///
/// **Its own number, separate from [`crate::frame::FRAME_VERSION`] and from
/// [`crate::abi::VERSION`]**, for the reason `frame` gives about the first
/// two: these three layouts change for different reasons, and folding them
/// together makes every pack rebuild when any one of them moves.
pub const GL_VERSION: u32 = 1;

/// `unsafe extern "C" fn() -> u32`
///
/// The GL half's version, read through its own symbol at load time — before
/// any descriptor is composed, exactly as [`crate::abi::VERSION_SYMBOL`] is.
pub const GL_VERSION_SYMBOL: &[u8] = b"rivet_addon_gl_abi\0";

/// `unsafe extern "C" fn(*const GlFrame) -> i32`
///
/// Returns [`DREW`] or one of [`crate::frame`]'s refusal codes. **An object
/// that exports this must export [`GL_VERSION_SYMBOL`] too**, and the loader
/// refuses one without the other by name.
pub const DRAW_GL_SYMBOL: &[u8] = b"rivet_addon_draw_gl\0";

/// **What the host tells a pack about the frame it wants drawn on the GPU.**
///
/// `repr(C)`, and the same rule [`crate::frame::Frame`] states applies here:
/// every revision that ever ships keeps a `u32` version at offset zero, and
/// that is the one field a later revision may not move — [`dispatch_gl`]
/// reads those four bytes through a `*const u32` rather than forming a
/// reference to a struct whose size the two sides may not agree on.
///
/// # The fields are deliberately few
///
/// There is no pointer here and no stride, because there are no pixels: the
/// pack draws through GL into a target the host has already bound. What
/// remains is the geometry it must size its viewport-relative work against,
/// the identity of the object it is drawing into, and the same scene
/// [`crate::frame`] carries.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GlFrame {
    /// [`GL_VERSION`], as the host that composed this descriptor speaks it.
    /// **Read and compared before anything else in this struct.**
    pub abi: u32,
    /// Visible pixels across, and the width the viewport is already set to.
    pub width: u32,
    /// Visible pixel rows, and the height the viewport is already set to.
    pub height: u32,
    /// **The GL framebuffer object the host has ALREADY BOUND**, as
    /// `glBindFramebuffer(GL_FRAMEBUFFER, …)` took it.
    ///
    /// **Zero is a legal and ordinary value**: it names the default
    /// framebuffer, which is what a host that renders the background straight
    /// into its own EGL surface binds. R-206 D1 puts the host in charge of
    /// creating and binding the target, and a host whose target *is* the
    /// surface it will present has nothing to create — forcing it to render
    /// to a texture and blit would spend a full-screen pass on the V3D to
    /// satisfy a word in a ruling rather than the ruling's reason.
    ///
    /// It is carried at all so a pack can rebind it after using a framebuffer
    /// of its own for an intermediate pass, which is the one legitimate
    /// reason to need the name.
    pub framebuffer: u32,
    /// Zero. Named rather than left as the padding `repr(C)` would insert
    /// before [`GlFrame::time_ms`] anyway — [`crate::frame::Frame::reserved`]
    /// makes the argument, and this is one field a later revision can spend
    /// without moving anything.
    pub reserved: u32,
    /// Zero, on the same terms. Two are named here because the alignment of
    /// the `u64` below leaves room for exactly two.
    pub reserved2: u32,
    /// **The frame's own time, from the host's clock**, in milliseconds.
    /// [`crate::frame::Frame::time_ms`] argues why the pack does not read a
    /// clock, and every word of it applies here.
    pub time_ms: u64,
    /// The background's id, as [`crate::protocol::Background::id`] spells it —
    /// ASCII, NUL-padded, not NUL-terminated when it fills the array exactly.
    pub background: [u8; ID_BYTES],
    /// The theme's id, as [`crate::protocol::Theme::id`] spells it, on the
    /// same terms.
    pub theme: [u8; ID_BYTES],
}

/// **The bound target, as a pack sees it.**
///
/// [`crate::frame::Surface`] exists to make a dangling pointer unspellable.
/// This type has a smaller job, because there is no pointer to dangle: GL
/// object names are integers the driver validates, and a pack that keeps one
/// too long gets a GL error rather than the renderer's address space. What it
/// is for is the same as `Surface`'s other half — handing the pack the two
/// numbers it needs without handing it the descriptor, so that a field added
/// to [`GlFrame`] is not a field every pack has to learn about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlTarget {
    width: u32,
    height: u32,
    framebuffer: u32,
}

impl GlTarget {
    /// Describe a target the host has bound.
    ///
    /// Returns [`None`] for a geometry that is not one — the same refusal
    /// [`crate::frame::Surface::over`] makes, and made here rather than in
    /// [`dispatch_gl`] so that a host composing a descriptor in a test can
    /// find out without a `dlopen`.
    #[must_use]
    pub const fn over(width: u32, height: u32, framebuffer: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            width,
            height,
            framebuffer,
        })
    }

    /// Visible pixels across. The viewport is already this wide.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Visible pixel rows. The viewport is already this tall.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The framebuffer object the host bound. See [`GlFrame::framebuffer`] on
    /// why zero is ordinary, and this module's header on what a pack may not
    /// do with it.
    #[must_use]
    pub const fn framebuffer(&self) -> u32 {
        self.framebuffer
    }
}

/// **What a pack's author implements to draw with the GPU**, and — exactly as
/// [`crate::frame::Drawing`] — it contains no `unsafe`, no pointer and no
/// lifetime they have to name.
///
/// A pack may implement this, [`crate::frame::Drawing`], or both. R-206 D3
/// makes both the interesting case: the x86 machine in this project's
/// long-term targets has no V3D, and a pack with no CPU path leaves it with no
/// backgrounds at all.
pub trait DrawingGl: Send + 'static {
    /// Draw one frame into the bound target.
    ///
    /// # Errors
    /// [`Refusal`], when the pack will not draw what it was asked for. The
    /// vocabulary is [`crate::frame`]'s, unchanged: a GPU pack refuses an
    /// unknown background for the same reasons and says so in the same words.
    fn draw_gl(&mut self, target: &GlTarget, scene: &Scene<'_>) -> Result<(), Refusal>;
}

/// **Validate a GL descriptor and run one frame against a pack.**
///
/// [`crate::frame::dispatch`]'s twin, and the check order is the same design
/// rather than the same housekeeping: a null pointer first, then
/// [`GlFrame::abi`] read as four bytes through a `*const u32` — never as
/// `(*frame).abi`, which would form a reference to a whole `GlFrame` as
/// **this** build lays it out, over memory an older or newer host may not have
/// allocated — and only then anything else.
///
/// That ordering is what lets [`REFUSED_VERSION`] come back before a single
/// GL call is made and before one other field is believed.
///
/// # There is no format check here, and no surface-size check
///
/// [`crate::frame::dispatch`] makes both, because it is about to build a
/// `&mut [u8]` over memory the host described and a wrong number there is a
/// buffer overrun in the renderer. Nothing here forms a slice: the target is
/// an integer the driver owns, and a wrong one is a GL error the driver
/// raises. The checks that remain are the ones that are still about meaning —
/// a zero dimension, and an id the boundary cannot carry.
///
/// # Safety
/// `frame` must be null or point at a [`GlFrame`] the caller keeps valid for
/// this call, with the host's context current and the target bound. The
/// host's side of that contract is [`crate::host::Loaded::draw_gl`].
#[must_use]
pub unsafe fn dispatch_gl<D: DrawingGl>(drawing: &mut D, frame: *const GlFrame) -> i32 {
    if frame.is_null() {
        return REFUSED_DESCRIPTOR;
    }
    // Four bytes, at offset zero, with no reference formed to anything larger.
    // SAFETY: `frame` is non-null and, by the caller's contract, points at a
    // descriptor beginning with the one field every revision must keep there.
    let abi = unsafe { frame.cast::<u32>().read() };
    if abi != GL_VERSION {
        return REFUSED_VERSION;
    }

    // SAFETY: the versions agree, so the descriptor is laid out as this build
    // reads it, and the caller's contract makes it valid for this call.
    let frame = unsafe { &*frame };

    let Some(target) = GlTarget::over(frame.width, frame.height, frame.framebuffer) else {
        return REFUSED_DESCRIPTOR;
    };
    let (Some(background), Some(theme)) = (id_str(&frame.background), id_str(&frame.theme)) else {
        return REFUSED_DESCRIPTOR;
    };
    let scene = Scene {
        background,
        theme,
        time_ms: frame.time_ms,
    };
    match drawing.draw_gl(&target, &scene) {
        Ok(()) => DREW,
        Err(refusal) => refusal.code(),
    }
}

/// **Write the two C symbols of a pack's GL half**, on top of
/// [`crate::export_drawing_addon!`]'s five.
///
/// ```ignore
/// rivetos_addon::export_drawing_addon_with_gl!(BackgroundPack);
/// ```
///
/// The type must implement [`crate::abi::Addon`], [`crate::frame::Drawing`],
/// [`DrawingGl`] and [`Default`]. **It is one object and one instance** —
/// `export_addon!`'s, behind the same `Mutex` — so the catalogue that names a
/// background, the CPU path that draws it and the GPU path that draws it are
/// all the same object's, which is what makes
/// [`crate::protocol::Drawn::Either`] mean anything.
///
/// **The GL entry this adds is catch-unwind-guarded the same way as the
/// other three** — [`crate::export_addon!`]'s section on why the `cdylib`
/// must be built with `panic = "unwind"` binds here too.
///
/// # Why this requires the CPU half rather than replacing it
///
/// R-206 D3. A pack exporting only a GL entry would be refused by every host
/// on a machine without a GPU, which is one of the machines this project is
/// aimed at. A pack that genuinely has no CPU path says so per background in
/// its catalogue, where a host can read it and skip that background — not by
/// omitting an entry point, where a host can only discover it by failing.
#[macro_export]
macro_rules! export_drawing_addon_with_gl {
    ($addon:ty) => {
        $crate::export_drawing_addon!($addon);

        /// The GL frame descriptor's version this object was built against.
        #[unsafe(no_mangle)]
        pub extern "C" fn rivet_addon_gl_abi() -> u32 {
            $crate::gl::GL_VERSION
        }

        /// Draw one frame into the host's bound framebuffer. See
        /// `rivetos_addon::gl` for the contract, and in particular for the
        /// rule that the host owns the context.
        ///
        /// # Safety
        /// `frame` must point at a `rivetos_addon::gl::GlFrame` valid for the
        /// length of this call, with the host's EGL context current and its
        /// target bound. The host's side is
        /// `rivetos_addon::host::Loaded::draw_gl`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rivet_addon_draw_gl(
            frame: *const $crate::gl::GlFrame,
        ) -> ::std::primitive::i32 {
            // RULE 2: nothing unwinds out of an `extern "C"` function. A
            // panicking background costs one frame; the renderer holding the
            // display keeps running and the host learns which pack did it.
            ::std::panic::catch_unwind(|| {
                match __rivet_addon_instance().lock() {
                    // SAFETY: forwarded verbatim from this function's own
                    // contract, which the host satisfies.
                    Ok(mut guard) => unsafe { $crate::gl::dispatch_gl(&mut *guard, frame) },
                    // A pack that panicked while holding the lock is a pack
                    // whose state is unknown, and drawing from unknown state
                    // is how one bad frame becomes every frame.
                    Err(_) => $crate::gl::PANICKED_GL,
                }
            })
            .unwrap_or($crate::gl::PANICKED_GL)
        }
    };
}

/// [`crate::frame::PANICKED`], re-exported so that
/// [`crate::export_drawing_addon_with_gl!`] can name it through this module
/// rather than reaching into a sibling the pack author never used.
pub const PANICKED_GL: i32 = PANICKED;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::id_bytes;

    fn descriptor(width: u32, height: u32) -> GlFrame {
        GlFrame {
            abi: GL_VERSION,
            width,
            height,
            framebuffer: 0,
            reserved: 0,
            reserved2: 0,
            time_ms: 1_234,
            background: id_bytes("contours").expect("a carryable id"),
            theme: id_bytes("ice-blue").expect("a carryable id"),
        }
    }

    #[derive(Default)]
    struct Spy {
        seen: Option<(u32, u32, u32, u64, String, String)>,
        refuse: Option<Refusal>,
    }

    impl DrawingGl for Spy {
        fn draw_gl(&mut self, target: &GlTarget, scene: &Scene<'_>) -> Result<(), Refusal> {
            self.seen = Some((
                target.width(),
                target.height(),
                target.framebuffer(),
                scene.time_ms,
                scene.background.to_string(),
                scene.theme.to_string(),
            ));
            self.refuse.map_or(Ok(()), Err)
        }
    }

    /// **The version is read before anything else is believed**, which is the
    /// one property the whole check order exists for.
    #[test]
    fn a_descriptor_from_another_layout_is_refused_before_a_field_is_read() {
        let mut frame = descriptor(1920, 1080);
        frame.abi = GL_VERSION + 1;
        // Poisoned past the version: if any of it were read, the ids would not
        // parse and the code would be `REFUSED_DESCRIPTOR` instead.
        frame.width = 0;
        frame.background = [0xFF; ID_BYTES];

        let mut spy = Spy::default();
        let code = unsafe { dispatch_gl(&mut spy, &raw const frame) };
        assert_eq!(code, REFUSED_VERSION, "the version was not read first");
        assert!(spy.seen.is_none(), "a pack was called with a foreign layout");
    }

    #[test]
    fn a_null_descriptor_is_refused_rather_than_dereferenced() {
        let mut spy = Spy::default();
        let code = unsafe { dispatch_gl(&mut spy, std::ptr::null()) };
        assert_eq!(code, REFUSED_DESCRIPTOR);
        assert!(spy.seen.is_none());
    }

    /// A zero dimension is the one geometry check left, since nothing here
    /// forms a slice. It must still refuse: a viewport of no height is a pack
    /// about to divide by it.
    #[test]
    fn a_surface_with_no_area_is_refused() {
        let mut spy = Spy::default();
        for (w, h) in [(0, 1080), (1920, 0), (0, 0)] {
            let frame = descriptor(w, h);
            let code = unsafe { dispatch_gl(&mut spy, &raw const frame) };
            assert_eq!(code, REFUSED_DESCRIPTOR, "{w}x{h} was accepted");
        }
        assert!(spy.seen.is_none());
    }

    /// **Zero is a real framebuffer name and must reach the pack**, because a
    /// host drawing straight into its own EGL surface binds exactly that —
    /// see [`GlFrame::framebuffer`]. Treating it as "absent" would make the
    /// zero-copy host the one case that does not work.
    #[test]
    fn the_default_framebuffer_reaches_the_pack_as_itself() {
        let mut spy = Spy::default();
        let frame = descriptor(1920, 1080);
        let code = unsafe { dispatch_gl(&mut spy, &raw const frame) };
        assert_eq!(code, DREW);
        let seen = spy.seen.expect("the pack was called");
        assert_eq!(seen.0, 1920);
        assert_eq!(seen.1, 1080);
        assert_eq!(seen.2, 0, "the default framebuffer did not cross");
        assert_eq!(seen.3, 1_234);
        assert_eq!(seen.4, "contours");
        assert_eq!(seen.5, "ice-blue");
    }

    #[test]
    fn a_framebuffer_the_host_created_crosses_unchanged() {
        let mut spy = Spy::default();
        let mut frame = descriptor(640, 480);
        frame.framebuffer = 7;
        let code = unsafe { dispatch_gl(&mut spy, &raw const frame) };
        assert_eq!(code, DREW);
        assert_eq!(spy.seen.expect("called").2, 7);
    }

    /// The refusal vocabulary is `frame`'s, and it crosses as `frame`'s codes.
    #[test]
    fn a_pack_that_refuses_answers_in_the_shared_vocabulary() {
        for refusal in [
            Refusal::UnknownBackground,
            Refusal::UnknownTheme,
            Refusal::UnsupportedSurface,
        ] {
            let mut spy = Spy {
                seen: None,
                refuse: Some(refusal),
            };
            let frame = descriptor(64, 64);
            let code = unsafe { dispatch_gl(&mut spy, &raw const frame) };
            assert_eq!(code, refusal.code());
        }
    }

    /// **The layout has no implicit padding**, which is what makes the
    /// `repr(C)` promise checkable rather than merely stated: six `u32`s put
    /// the `u64` at offset 24, already aligned.
    #[test]
    fn the_descriptor_is_laid_out_with_nothing_the_compiler_chose() {
        assert_eq!(core::mem::align_of::<GlFrame>(), 8);
        assert_eq!(core::mem::size_of::<GlFrame>(), 24 + 8 + ID_BYTES * 2);
        let frame = descriptor(1, 1);
        let base = (&raw const frame).cast::<u8>();
        let time = (&raw const frame.time_ms).cast::<u8>();
        // SAFETY: both pointers are into the same object.
        assert_eq!(unsafe { time.offset_from(base) }, 24);
    }

    /// The version symbol is the one a host reads before it composes
    /// anything, so its spelling is part of the ABI.
    #[test]
    fn the_symbols_are_spelled_as_the_loader_looks_them_up() {
        assert!(GL_VERSION_SYMBOL.ends_with(b"\0"));
        assert!(DRAW_GL_SYMBOL.ends_with(b"\0"));
        assert_eq!(GL_VERSION_SYMBOL, b"rivet_addon_gl_abi\0");
        assert_eq!(DRAW_GL_SYMBOL, b"rivet_addon_draw_gl\0");
        // **Distinct from the CPU half's**, which is what lets a host load one
        // object and find out which halves it has rather than which one it is.
        assert_ne!(GL_VERSION_SYMBOL, crate::frame::FRAME_VERSION_SYMBOL);
        assert_ne!(DRAW_GL_SYMBOL, crate::frame::DRAW_SYMBOL);
    }
}
