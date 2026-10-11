//! **The second entry point: a background draws into the host's surface, and
//! the payload is a C-layout struct rather than serialized bytes.**
//! `.rulings/` R-199 D2.
//!
//! # Why this exists at all, stated as arithmetic
//!
//! [`crate::abi`]'s boundary serializes a message, copies it on entry,
//! allocates an answer and frees it through a second symbol. That is right for
//! a reading source polled every few minutes: a temperature is forty bytes
//! and arrives twelve times an hour.
//!
//! A background draws **thirty frames a second into a 1920x1080 surface**.
//! One frame is 8.29 MB; thirty of them is 249 MB a second, before any
//! encoding, on a board whose whole drawing budget for a frame is 33.3 ms and
//! whose real background already spends 12.4 ms of it (`SPEC-04` §10). Routing
//! that through a JSON document is not a design to weigh against alternatives;
//! it is a number that does not fit. So the frame crosses as a pointer to the
//! host's own buffer, and the addon writes into it in place.
//!
//! # This is a deliberate exception to the protocol's first rule
//!
//! [`crate`]'s rule 1 is that **no type layout crosses** — Rust promises
//! nothing about the layout of its own types between separate compilations,
//! and a direct call bets the host's address space on two builds agreeing.
//! That bet fails as memory corruption rather than as an error.
//!
//! **The exception is narrow, and the narrowness is the whole argument.**
//! [`Frame`] is `#[repr(C)]` over fixed-width scalars, a raw pointer and two
//! byte arrays. `repr(C)` over those *is* the C ABI of the target, which is
//! the one layout two separately compiled Rust builds — and a C program, and a
//! program in any other language on that target — do agree on, because it is
//! the platform's rather than the language's. Nothing here has a `String`, a
//! `Vec`, an enum with a payload or a `bool`, each of which would put a Rust
//! layout decision on the wire.
//!
//! # The rules that carry over from the JSON boundary unchanged
//!
//! 1. **The version is checked before anything else is read.**
//!    [`crate::abi::VERSION_SYMBOL`] still gates the load, and
//!    [`FRAME_VERSION_SYMBOL`] is read at the same moment for the drawing
//!    half. Then [`Frame::abi`] is the descriptor's **first field**, and
//!    [`dispatch`] reads those four bytes and compares them before it forms a
//!    reference to anything else — see that function for why the order is
//!    load-bearing rather than tidy.
//! 2. **Nothing unwinds.** The generated entry wraps the call in
//!    [`std::panic::catch_unwind`], for the same reason [`crate::abi`]'s glue
//!    does: unwinding out of an `extern "C"` function is undefined behaviour.
//!    A background that panics costs its frame, not the renderer.
//! 3. **The unsafe lives here.** An addon crate carries
//!    `#![forbid(unsafe_code)]` and implements [`Drawing`], a safe trait over
//!    a safe [`Surface`]. [`crate::export_drawing_addon!`] writes the
//!    `extern "C"` half.
//!
//! # THE HOST OWNS THE SURFACE
//!
//! **The addon writes within `stride * height` bytes and returns; it never
//! allocates the surface, never frees it, and never keeps the pointer past the
//! call.** That sentence is in these words because it is the rule a future
//! addon author will be tempted to break — by stashing the pointer to avoid
//! re-deriving a row address, by keeping a `&mut [u8]` in `self` across
//! frames, by handing the buffer to a thread that outlives the call. Every one
//! of those is a use-after-free in the renderer's address space, and the
//! renderer is the process holding the display.
//!
//! [`Surface`] is what makes obeying it the path of least resistance: it
//! borrows for the length of one call and there is no way to name its lifetime
//! in a field of `self`.
//!
//! # What the host gets back, and why no sentence crosses
//!
//! The entry returns an `i32` from the small set below — [`DREW`],
//! [`REFUSED_VERSION`] and the rest — and **allocates nothing**. The JSON
//! boundary answers a fault with a sentence because a fault happens rarely and
//! a person reads it. A refused *frame* happens thirty times a second when it
//! happens at all, and an error path that allocates is an error path that can
//! itself fail; a host that logged a sentence per refused frame would fill a
//! journal in a minute. So the code crosses, and the host's own
//! [`crate::host::DrawFailure`] owns the prose — the same split
//! [`crate::host::OpenFailure`] already makes.

use crate::abi;

/// The frame descriptor's own version, carried in [`Frame::abi`] and answered
/// by [`FRAME_VERSION_SYMBOL`].
///
/// **Separate from [`crate::abi::VERSION`] on purpose.** The two halves of
/// this protocol change for different reasons: a new message or a new field in
/// the JSON vocabulary moves the first, and a change to the C layout below
/// moves this one. Folding them together would make every addon author who
/// added a settings field rebuild every pack, and would make a layout change
/// look like a message change in a journal line.
///
/// Bumped when a change would make an older host and a newer addon read the
/// same bytes differently — which, for a `repr(C)` struct, means **any** change
/// to the fields other than spending [`Frame::reserved`].
pub const FRAME_VERSION: u32 = 1;

/// `unsafe extern "C" fn() -> u32`
///
/// The drawing half's version, read through its own symbol at load time, for
/// [`crate::abi::VERSION_SYMBOL`]'s reason: a host that cannot speak an
/// addon's layout refuses it having parsed nothing the addon wrote.
pub const FRAME_VERSION_SYMBOL: &[u8] = b"rivet_addon_frame_abi\0";

/// `unsafe extern "C" fn(*const Frame) -> i32`
///
/// Returns [`DREW`], or one of the refusals below. **An object that exports
/// this must export [`FRAME_VERSION_SYMBOL`] too**, and one that exports
/// neither is an ordinary reading source rather than a broken pack —
/// [`crate::host::Loaded::draws`] is how a host tells those apart.
pub const DRAW_SYMBOL: &[u8] = b"rivet_addon_draw\0";

/// **How many bytes a background's id and a theme's id each get in the
/// descriptor.**
///
/// # Why an id and not an index, which is where this departs from the design
///
/// `docs/design-candidates/Background-Addons-2026-09-19.md` §4 sketches
/// `background: u32` and `theme: u32`, each *an index into what `Describe`
/// returned*. Two things ruled since make an index the wrong contract:
///
/// * **`.rulings/` R-201 D1** — *the order of a list is a weak contract* for
///   something two signed artifacts depend on, and it fails silently and much
///   later. It was ruled about a library naming its addon object, and the
///   reasoning transfers without alteration: a pack that reorders its
///   catalogue between builds emits a perfectly valid artifact that draws the
///   wrong background, and nothing anywhere reports it.
/// * **`.rulings/` R-202 D1** — a rebuilt pack *supersedes* its own older
///   build at install. So the catalogue a host read at start and the catalogue
///   the object now holds can be two different lists, with the host still
///   holding index 2. An id survives that; a position does not.
///
/// A `String` cannot cross a `repr(C)` boundary, so the id crosses as a
/// NUL-padded byte array of this width. Thirty-two bytes holds every id the
/// alphabet below can spell that anybody would type — `digital-rain` is
/// twelve — and costs 64 bytes in a struct handed over thirty times a second,
/// which is not a cost.
pub const ID_BYTES: usize = 32;

/// `XRGB8888`, little-endian, blue at the lowest byte — what `rivet_sdk::draw`
/// writes and what the dumb buffer scans out.
///
/// **One, rather than zero.** A zeroed descriptor — a `calloc`, a partially
/// initialised struct, a host that forgot a field — then names no format at
/// all and is refused, instead of being read as the one format that exists.
pub const FORMAT_XRGB8888: u32 = 1;

/// The frame was drawn.
pub const DREW: i32 = 0;
/// [`Frame::abi`] is not [`FRAME_VERSION`]. **Nothing past the first four
/// bytes was read and no pixel was written.**
pub const REFUSED_VERSION: i32 = 1;
/// The descriptor is this version and still cannot be used: a null pointer, a
/// zero dimension, a `stride` narrower than a row, or a `stride * height` that
/// does not fit a `usize`.
pub const REFUSED_DESCRIPTOR: i32 = 2;
/// [`Frame::format`] names a format this addon does not write.
pub const REFUSED_FORMAT: i32 = 3;
/// The id in [`Frame::background`] is not one this pack has, or is not
/// readable as an id at all.
pub const REFUSED_BACKGROUND: i32 = 4;
/// The id in [`Frame::theme`] is not one that background has.
pub const REFUSED_THEME: i32 = 5;
/// The addon panicked while drawing. Caught at the boundary; the frame is
/// lost and the host is not.
pub const PANICKED: i32 = 6;
/// **The addon will not draw into a surface of this shape** — too small for
/// what it has to say, or a geometry it has no code for.
///
/// Distinct from [`REFUSED_DESCRIPTOR`], which is the boundary refusing a
/// descriptor that does not describe a surface at all. This one is the pack's
/// own judgement about a surface that is perfectly well described.
pub const REFUSED_SURFACE: i32 = 7;

/// **The descriptor, and the exception `.rulings/` R-199 D2 confirmed.**
///
/// Built by the host, read by the addon, valid for exactly one call.
///
/// # The field order is part of the contract
///
/// [`Frame::abi`] is first so that a reader which cannot trust the rest of the
/// layout can still read the version — see [`dispatch`], which reads those four
/// bytes through a `*const u32` rather than forming a `&Frame`. Every version
/// of this struct that ever ships must keep a `u32` version at offset zero,
/// and that is the one field a future revision may not move.
///
/// [`Frame::reserved`] is the padding `repr(C)` inserts anyway between
/// [`Frame::format`] and [`Frame::pixels`] on a 64-bit target, named rather
/// than left implicit. Naming it costs nothing, makes the layout identical on
/// a target that would not have padded, and leaves one field a later revision
/// can spend without moving anything.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    /// [`FRAME_VERSION`], as the host that composed this descriptor speaks it.
    /// **Read and compared before anything else in this struct.**
    pub abi: u32,
    /// Visible pixels across.
    pub width: u32,
    /// Visible pixel rows.
    pub height: u32,
    /// **Bytes per row, which is not `width * 4`.** A scanout buffer is padded
    /// to the controller's pitch, and a background that computes a row address
    /// from the width corrupts every row after the first.
    pub stride: u32,
    /// [`FORMAT_XRGB8888`] today.
    pub format: u32,
    /// Zero. See this struct's own note on why it is named.
    pub reserved: u32,
    /// **The host's surface.** Valid for this call and no longer, writable for
    /// `stride * height` bytes, allocated and freed by the host. See this
    /// module's header, which states that rule in the words an addon author
    /// should have read.
    pub pixels: *mut u8,
    /// **The frame's own time, from the host's clock**, in milliseconds.
    ///
    /// The addon does not read a clock, for `ADR-021`'s reason and for a
    /// second one that matters more here: two backgrounds drawing from the
    /// same host must animate on the same time base, and a host testing a pack
    /// must be able to ask for the frame at an hour that is not now.
    pub time_ms: u64,
    /// The background's id, as [`crate::protocol::Background::id`] spells it —
    /// ASCII, NUL-padded, and not NUL-terminated when it fills the array
    /// exactly. [`ID_BYTES`] is where the choice of an id over an index is
    /// argued.
    pub background: [u8; ID_BYTES],
    /// The theme's id, as [`crate::protocol::Theme::id`] spells it, on the same
    /// terms.
    pub theme: [u8; ID_BYTES],
}

/// Write an id into the fixed-width field the descriptor carries.
///
/// Returns `None` when the id is not one this boundary can carry — the
/// alphabet is [`crate::abi::class_is_nameable`]'s, reused rather than
/// invented for the reason that function gives, plus a length that fits
/// [`ID_BYTES`].
#[must_use]
pub fn id_bytes(id: &str) -> Option<[u8; ID_BYTES]> {
    if !abi::class_is_nameable(id) || id.len() > ID_BYTES {
        return None;
    }
    let mut out = [0u8; ID_BYTES];
    out[..id.len()].copy_from_slice(id.as_bytes());
    Some(out)
}

/// Read one back. `None` when the bytes are not an id — which a host cannot
/// produce through [`id_bytes`] and a host in another language can.
#[must_use]
pub fn id_str(bytes: &[u8; ID_BYTES]) -> Option<&str> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(ID_BYTES);
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    abi::class_is_nameable(text).then_some(text)
}

/// **The host's surface, as an addon sees it: safe, borrowed, and impossible
/// to keep.**
///
/// The lifetime is the call's, and that — rather than any privacy — is what
/// enforces the rule in this module's header. [`Drawing::draw`] takes
/// `&mut self` and a `Surface<'_>` whose lifetime is the call's, and the
/// implementing type has no lifetime parameter to store it under, so *keep the
/// pointer until the next frame* does not compile. Saying it in the header as
/// well is belt and braces for an author reaching for `unsafe` to get around
/// the type system.
///
/// # Padding is never written
///
/// [`Surface::row`] hands back a row's **visible** bytes, `width * 4`, and not
/// the `stride` the host allocated. The bytes between a row's last pixel and
/// the next row's first are the host's and are left exactly as they were —
/// which is also what makes a probe able to prove an addon stayed inside its
/// lines by filling the padding with a sentinel and reading it back.
pub struct Surface<'a> {
    width: u32,
    height: u32,
    stride: u32,
    bytes: &'a mut [u8],
}

impl<'a> Surface<'a> {
    /// **A surface over a buffer the caller already holds.**
    ///
    /// [`dispatch`] builds every surface through here, so the geometry an
    /// addon draws against is checked in one place rather than once for a real
    /// frame and again for a test. **A pack's own unit tests build one
    /// directly**, which is how a background is tested without a descriptor
    /// and without a host — see `examples/probes/frame-probe`.
    ///
    /// `None` when the numbers do not describe a surface inside `bytes`.
    #[must_use]
    pub fn over(bytes: &'a mut [u8], width: u32, height: u32, stride: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let row = (width as usize).checked_mul(4)?;
        if (stride as usize) < row {
            return None;
        }
        let span = (stride as usize).checked_mul(height as usize)?;
        if bytes.len() < span {
            return None;
        }
        Some(Self {
            width,
            height,
            stride,
            bytes,
        })
    }

    /// Visible pixels across.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Visible pixel rows.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per row, padding included. A background that iterates rows
    /// through [`Surface::row`] never needs this; one that has a reason to
    /// know the shape of the allocation can ask.
    #[must_use]
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// **Every visible byte at once, when there is no padding to step over** —
    /// `None` on a surface whose [`Surface::stride`] exceeds `width * 4`.
    ///
    /// # Why this exists
    ///
    /// A background ported from a program that owned its own framebuffer
    /// draws into one flat `&mut [u8]`. Handed a [`Surface`] instead, the
    /// straightforward adaptation is to draw into a scratch buffer and copy it
    /// out row by row — and at 1920x1080 that is an 8.29 MB allocation and an
    /// 8.29 MB copy **per frame**, 249 MB a second at 30 fps, on a board whose
    /// whole budget for a frame is 33.3 ms. The copy is pure overhead
    /// whenever the host's buffer is already the shape the background wanted,
    /// which a DRM dumb buffer at a standard width is.
    ///
    /// So: ask, and take the flat path when the answer is yes. A host with a
    /// padded buffer still works, one row at a time, through
    /// [`Surface::row`].
    ///
    /// **`None` is not a failure** and a caller must have the row-wise path
    /// anyway — a scanout buffer's stride is the driver's business and can be
    /// rounded up to any alignment it likes.
    pub fn packed_mut(&mut self) -> Option<&mut [u8]> {
        let row = (self.width as usize).checked_mul(4)?;
        if self.stride as usize != row {
            return None;
        }
        let span = row.checked_mul(self.height as usize)?;
        self.bytes.get_mut(..span)
    }

    /// One row's visible bytes, `width * 4` of them, or `None` past the last
    /// row.
    ///
    /// **`None` rather than a panic or an empty slice.** A panic costs the
    /// frame, and an empty slice makes an off-by-one draw nothing and report
    /// success — the failure that is found on a television rather than in a
    /// test.
    pub fn row(&mut self, y: u32) -> Option<&mut [u8]> {
        if y >= self.height {
            return None;
        }
        let start = (y as usize) * (self.stride as usize);
        let end = start + (self.width as usize) * 4;
        self.bytes.get_mut(start..end)
    }

    /// Write one pixel, `0x00RR_GGBB`. Out of range is **clipped and
    /// silent**, exactly as `rivet_sdk::draw`'s primitives clip: a background
    /// plotting a particle near an edge is the ordinary case, not an error.
    pub fn put(&mut self, x: u32, y: u32, pixel: u32) {
        if x >= self.width {
            return;
        }
        let Some(row) = self.row(y) else {
            return;
        };
        let at = (x as usize) * 4;
        // XRGB8888, little-endian: blue lowest, then green, then red, then the
        // byte the scanout ignores. `rivet_sdk::draw` writes the same order and
        // this is the one place this crate restates it.
        row[at..at + 4].copy_from_slice(&pixel.to_le_bytes());
    }

    /// Fill every visible pixel with one colour, `0x00RR_GGBB`.
    ///
    /// Written as a row of four bytes repeated with `chunks_exact_mut` rather
    /// than through [`Surface::put`], because the compiler vectorises this and
    /// cannot vectorise a call with a bounds check per pixel — the same
    /// finding `SPEC-04` §10 records for `draw::fill`.
    pub fn fill(&mut self, pixel: u32) {
        let word = pixel.to_le_bytes();
        for y in 0..self.height {
            let Some(row) = self.row(y) else {
                break;
            };
            for cell in row.as_chunks_mut::<4>().0 {
                cell.copy_from_slice(&word);
            }
        }
    }
}

/// **What to draw, this frame.**
///
/// One type, used by both halves: a host builds it to say what it wants and
/// [`dispatch`] builds the identical thing out of the descriptor's bytes. Two
/// types would be two places for an id to be spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scene<'a> {
    /// The background's id, from [`crate::protocol::Background::id`].
    pub background: &'a str,
    /// The theme's id, from [`crate::protocol::Theme::id`].
    pub theme: &'a str,
    /// The frame's own time, from the host's clock. See [`Frame::time_ms`].
    pub time_ms: u64,
}

/// Why a pack would not draw a frame it was asked for.
///
/// **Three, and no sentence.** This module's header argues why nothing
/// allocated crosses a boundary used thirty times a second; each of these
/// becomes one of the `i32` codes above, and the host turns it back into
/// prose.
///
/// **Whether a pack MAY refuse a theme it does not know, or must draw
/// something, is the maintainer's** — it is the open question
/// `docs/design-candidates/Background-Addons-2026-09-19.md` §8 raises, and it
/// is a pack's policy either way. What the boundary owes that question is the
/// ability to express a refusal at all, which is what this enum is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The id in [`Frame::background`] is not one this pack has.
    UnknownBackground,
    /// The id in [`Frame::theme`] is not one that background has.
    UnknownTheme,
    /// **The surface is one this pack will not draw into** — a minimum size
    /// it needs, a geometry it has no code for.
    ///
    /// **There is no `UnsupportedFormat` here, and that is deliberate.**
    /// [`dispatch`] has to know the pixel width to make a [`Surface`] at all,
    /// so it refuses an unknown [`Frame::format`] itself, with
    /// [`REFUSED_FORMAT`], before a pack is called. A variant a pack could
    /// never be given the chance to return would be a piece of vocabulary
    /// describing a situation that does not arise.
    UnsupportedSurface,
}

impl Refusal {
    /// The code this crosses as.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            Self::UnknownBackground => REFUSED_BACKGROUND,
            Self::UnknownTheme => REFUSED_THEME,
            Self::UnsupportedSurface => REFUSED_SURFACE,
        }
    }
}

/// **What a pack's author implements**, and it contains no `unsafe`, no
/// pointer and no lifetime they have to name.
///
/// `&mut self` for [`crate::abi::Addon`]'s reason: a background holds state
/// between frames — a rain column's position, a cell grid, a phase — and the
/// host serializes calls.
///
/// A pack implements this **and** [`crate::abi::Addon`]: the first is how it
/// draws, the second is how it describes its catalogue
/// ([`crate::protocol::Description::backgrounds`]) and takes its settings.
/// [`crate::export_drawing_addon!`] is what exports both halves.
pub trait Drawing: Send + 'static {
    /// Draw one frame.
    ///
    /// # Errors
    /// [`Refusal`], when the pack will not draw what it was asked for. See
    /// that type on whose question that is.
    fn draw(&mut self, surface: &mut Surface<'_>, scene: &Scene<'_>) -> Result<(), Refusal>;
}

/// **Validate a descriptor and run one frame against a pack.**
///
/// Split out of [`crate::export_drawing_addon!`] for
/// [`crate::abi::dispatch`]'s reason: the macro's body is then a few lines of
/// glue, and everything with a decision in it is ordinary Rust a test can
/// reach.
///
/// # The order of the checks is the design, not housekeeping
///
/// 1. **A null pointer**, because everything after dereferences it.
/// 2. **[`Frame::abi`], read as four bytes through a `*const u32`.** It is
///    deliberately *not* read as `(*frame).abi`, which would form a reference
///    to a whole `Frame` as **this** build lays it out — and if the host's
///    descriptor is an older or newer revision, that struct may be smaller
///    than this build's and the reference would cover memory the host never
///    allocated. Reading the first `u32` is sound for every revision, because
///    every revision is required to keep a version there.
/// 3. **Everything else**, now that both sides have agreed what the bytes
///    mean.
///
/// That ordering is what lets [`REFUSED_VERSION`] be returned **before a pixel
/// is written** and before a single other field is believed.
///
/// # Safety
/// `frame` must be null or point at a `Frame` the caller keeps valid for this
/// call, whose `pixels` is writable for `stride * height` bytes. The host's
/// side of that contract is [`crate::host::Loaded::draw`], which checks the
/// buffer it was given is that large before it composes a descriptor at all.
#[must_use]
pub unsafe fn dispatch<D: Drawing>(drawing: &mut D, frame: *const Frame) -> i32 {
    if frame.is_null() {
        return REFUSED_DESCRIPTOR;
    }
    // STEP 2 above. Four bytes, at offset zero, with no reference formed to
    // anything larger.
    // SAFETY: `frame` is non-null and, by the caller's contract, points at a
    // descriptor of at least this crate's first field — which every revision
    // of `Frame` is required to begin with.
    let abi = unsafe { frame.cast::<u32>().read() };
    if abi != FRAME_VERSION {
        return REFUSED_VERSION;
    }

    // SAFETY: the versions agree, so the descriptor is laid out as this build
    // reads it, and the caller's contract makes it valid for this call.
    let frame = unsafe { &*frame };

    if frame.format != FORMAT_XRGB8888 {
        return REFUSED_FORMAT;
    }
    if frame.pixels.is_null() || frame.width == 0 || frame.height == 0 {
        return REFUSED_DESCRIPTOR;
    }
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    // A row must fit inside the stride, and the whole surface must fit inside
    // a `usize`. Both are the host lying about its own buffer rather than
    // anything the addon did, and both are refused rather than clamped: a
    // clamp would draw something into a buffer whose shape nobody agrees on.
    let Some(row_bytes) = width.checked_mul(4) else {
        return REFUSED_DESCRIPTOR;
    };
    if stride < row_bytes {
        return REFUSED_DESCRIPTOR;
    }
    let Some(span) = stride.checked_mul(height) else {
        return REFUSED_DESCRIPTOR;
    };
    // **`checked_mul` is not the whole bound, and a test found that the hard
    // way.** A `u32` stride times a `u32` height fits a 64-bit `usize`
    // comfortably and can still be larger than `isize::MAX`, which is the
    // limit a Rust slice may span — `from_raw_parts_mut` treats a larger one
    // as undefined behaviour rather than as an error. No real surface is
    // anywhere near it; a descriptor that claims one is a host that has lost
    // track of its own buffer, and it is refused here rather than believed.
    if span > isize::MAX as usize {
        return REFUSED_DESCRIPTOR;
    }

    let (Some(background), Some(theme)) = (id_str(&frame.background), id_str(&frame.theme)) else {
        return REFUSED_DESCRIPTOR;
    };

    // SAFETY: the host's contract is that `pixels` is writable for
    // `stride * height` bytes, and `span` is that product computed without
    // overflow. Nothing else aliases it for the length of this call, because
    // the host is blocked in it.
    let bytes = unsafe { std::slice::from_raw_parts_mut(frame.pixels, span) };
    let Some(mut surface) = Surface::over(bytes, frame.width, frame.height, frame.stride) else {
        return REFUSED_DESCRIPTOR;
    };
    let scene = Scene {
        background,
        theme,
        time_ms: frame.time_ms,
    };
    match drawing.draw(&mut surface, &scene) {
        Ok(()) => DREW,
        Err(refusal) => refusal.code(),
    }
}

/// **Write a drawing addon's five C symbols** — [`crate::export_addon!`]'s
/// three, plus [`FRAME_VERSION_SYMBOL`] and [`DRAW_SYMBOL`].
///
/// ```ignore
/// rivetos_addon::export_drawing_addon!(BackgroundPack);
/// ```
///
/// The type must implement [`crate::abi::Addon`], [`crate::frame::Drawing`]
/// and [`Default`]. **It is one object and one instance**: the pack a host
/// asks to describe itself is the same pack it asks to draw, behind the same
/// `Mutex`, which is what makes `Describe`'s catalogue and a frame's ids talk
/// about the same list.
///
/// **This expands [`crate::export_addon!`] and adds `rivet_addon_draw`
/// beside it, catch-unwind-guarded the same way — see that macro's own
/// section on why the `cdylib` must be built with `panic = "unwind"`.** It
/// binds here too: a background that panics while drawing must cost the
/// frame, not the renderer holding the display.
///
/// # Why a second macro rather than a flag on the first
///
/// An object that exports [`DRAW_SYMBOL`] is telling every host that it draws
/// — [`crate::host::Loaded::draws`] is exactly that symbol's presence. A
/// reading source that exported a drawing entry which refused every frame
/// would be lying in the one place a host has to trust, so the reading source
/// keeps [`crate::export_addon!`] and exports nothing it cannot do.
#[macro_export]
macro_rules! export_drawing_addon {
    ($addon:ty) => {
        $crate::export_addon!($addon);

        /// The frame descriptor's version this object was built against.
        #[unsafe(no_mangle)]
        pub extern "C" fn rivet_addon_frame_abi() -> u32 {
            $crate::frame::FRAME_VERSION
        }

        /// Draw one frame. See `rivetos_addon::frame` for the contract, and
        /// in particular for the rule that the host owns the surface.
        ///
        /// # Safety
        /// `frame` must point at a `rivetos_addon::frame::Frame` valid for the
        /// length of this call, whose `pixels` is writable for
        /// `stride * height` bytes. The host's side of this contract is
        /// `rivetos_addon::host::Loaded::draw`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rivet_addon_draw(
            frame: *const $crate::frame::Frame,
        ) -> ::std::primitive::i32 {
            // RULE 2: nothing unwinds out of an `extern "C"` function. A
            // panicking background costs one frame; the renderer holding the
            // display keeps running and the host learns which pack did it.
            ::std::panic::catch_unwind(|| {
                // **The instance `export_addon!` made, not a second one.** A
                // background holds state between frames and a catalogue
                // between calls, and two statics would make the object that
                // described itself a different object from the one that draws.
                match __rivet_addon_instance().lock() {
                    // SAFETY: forwarded verbatim from this function's own
                    // contract, which the host satisfies.
                    Ok(mut guard) => unsafe { $crate::frame::dispatch(&mut *guard, frame) },
                    // A pack that panicked while holding the lock is a pack
                    // whose state is unknown, and drawing from unknown state
                    // is how one bad frame becomes every frame.
                    Err(_) => $crate::frame::PANICKED,
                }
            })
            .unwrap_or($crate::frame::PANICKED)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A tightly packed surface hands out all its bytes, and a padded one
    /// refuses** — the whole of what `packed_mut` decides, and the decision a
    /// background takes its fast path on.
    #[test]
    fn a_packed_surface_hands_out_its_bytes_and_a_padded_one_does_not() {
        let mut tight = vec![0u8; 4 * 4 * 3];
        let mut surface = Surface::over(&mut tight, 4, 3, 16).expect("a surface");
        let bytes = surface.packed_mut().expect("4 * 4 == the stride");
        assert_eq!(bytes.len(), 4 * 4 * 3);

        let mut padded = vec![0u8; 20 * 3];
        let mut surface = Surface::over(&mut padded, 4, 3, 20).expect("a surface");
        assert!(
            surface.packed_mut().is_none(),
            "four bytes of padding a row is not a flat buffer"
        );
    }

    /// **The flat path and the row-wise path write the same picture.** A
    /// background choosing between them by stride must not draw two different
    /// things, and nothing else in this crate would notice if it did — the
    /// difference would appear on a television, on whichever board happened
    /// to have a padded buffer.
    #[test]
    fn writing_flat_and_writing_by_row_agree() {
        let paint = |bytes: &mut [u8], width: usize, y: usize, x: usize| {
            let at = (y * width + x) * 4;
            bytes[at..at + 4].copy_from_slice(&0x00AA_BBCCu32.to_le_bytes());
        };

        let mut flat = vec![0u8; 4 * 4 * 3];
        {
            let mut surface = Surface::over(&mut flat, 4, 3, 16).expect("a surface");
            let bytes = surface.packed_mut().expect("packed");
            paint(bytes, 4, 2, 1);
        }

        let mut by_row = vec![0u8; 4 * 4 * 3];
        {
            let mut surface = Surface::over(&mut by_row, 4, 3, 16).expect("a surface");
            surface.put(1, 2, 0x00AA_BBCC);
        }

        assert_eq!(flat, by_row);
    }

    /// A pack with two backgrounds, one of which refuses an unknown theme —
    /// enough to exercise every code [`dispatch`] can return.
    #[derive(Default)]
    struct Pack {
        frames: u32,
    }

    impl Drawing for Pack {
        fn draw(&mut self, surface: &mut Surface<'_>, scene: &Scene<'_>) -> Result<(), Refusal> {
            let base = match scene.background {
                "solid" => 0x0010_2030,
                "boxes" => 0x0040_5060,
                _ => return Err(Refusal::UnknownBackground),
            };
            let tint = match scene.theme {
                "ice-blue" => 0x0000_0001,
                "amber" => 0x0000_0002,
                _ => return Err(Refusal::UnknownTheme),
            };
            self.frames += 1;
            surface.fill(base | tint);
            Ok(())
        }
    }

    /// A descriptor over a buffer the test owns, with the padding set to a
    /// sentinel so that a write past a row's last pixel is visible.
    struct Bench {
        bytes: Vec<u8>,
        width: u32,
        height: u32,
        stride: u32,
    }

    const PAD: u8 = 0xAB;

    impl Bench {
        fn new(width: u32, height: u32, stride: u32) -> Self {
            Self {
                bytes: vec![PAD; (stride as usize) * (height as usize)],
                width,
                height,
                stride,
            }
        }

        fn frame(&mut self, background: &str, theme: &str) -> Frame {
            self.frame_claiming(FRAME_VERSION, background, theme)
        }

        fn frame_claiming(&mut self, abi: u32, background: &str, theme: &str) -> Frame {
            Frame {
                abi,
                width: self.width,
                height: self.height,
                stride: self.stride,
                format: FORMAT_XRGB8888,
                reserved: 0,
                pixels: self.bytes.as_mut_ptr(),
                time_ms: 1_789_000_000_000,
                background: id_bytes(background).expect("a nameable id"),
                theme: id_bytes(theme).expect("a nameable id"),
            }
        }

        fn pixel(&self, x: u32, y: u32) -> u32 {
            let at = (y as usize) * (self.stride as usize) + (x as usize) * 4;
            u32::from_le_bytes([
                self.bytes[at],
                self.bytes[at + 1],
                self.bytes[at + 2],
                self.bytes[at + 3],
            ])
        }

        fn padding_is_untouched(&self) -> bool {
            let row = (self.width as usize) * 4;
            (0..self.height as usize).all(|y| {
                let start = y * (self.stride as usize);
                self.bytes[start + row..start + self.stride as usize]
                    .iter()
                    .all(|b| *b == PAD)
            })
        }
    }

    fn run(pack: &mut Pack, frame: &Frame) -> i32 {
        // SAFETY: `frame` is a descriptor built by `Bench` over a buffer that
        // outlives this call, with `stride * height` writable bytes.
        unsafe { dispatch(pack, std::ptr::from_ref(frame)) }
    }

    #[test]
    fn a_frame_crosses_and_the_bytes_arrive() {
        let mut bench = Bench::new(8, 4, 40);
        let frame = bench.frame("solid", "ice-blue");
        assert_eq!(run(&mut Pack::default(), &frame), DREW);
        assert_eq!(bench.pixel(0, 0), 0x0010_2031);
        assert_eq!(bench.pixel(7, 3), 0x0010_2031, "the last visible pixel");
    }

    /// **The rule the whole `stride` field exists for.** A pack that computed
    /// a row address from the width would write over the padding of every row
    /// but the first, and the picture would shear.
    #[test]
    fn the_padding_between_rows_belongs_to_the_host_and_is_left_alone() {
        let mut bench = Bench::new(8, 4, 40);
        let frame = bench.frame("boxes", "amber");
        assert_eq!(run(&mut Pack::default(), &frame), DREW);
        assert!(bench.padding_is_untouched());
    }

    /// **The refusal that matters most, and it is proven by the bytes rather
    /// than by the code.** A descriptor from a version this build does not
    /// speak is refused with the surface exactly as the host left it.
    #[test]
    fn a_descriptor_from_another_version_is_refused_before_a_pixel_is_written() {
        let mut bench = Bench::new(8, 4, 40);
        let frame = bench.frame_claiming(FRAME_VERSION + 1, "solid", "ice-blue");
        assert_eq!(run(&mut Pack::default(), &frame), REFUSED_VERSION);
        assert!(
            bench.bytes.iter().all(|b| *b == PAD),
            "every byte of the surface is still the host's"
        );
    }

    #[test]
    fn a_pack_that_does_not_have_the_background_says_so_and_draws_nothing() {
        let mut bench = Bench::new(8, 4, 40);
        let frame = bench.frame("starfield", "ice-blue");
        assert_eq!(run(&mut Pack::default(), &frame), REFUSED_BACKGROUND);
        assert!(bench.bytes.iter().all(|b| *b == PAD));

        let frame = bench.frame("solid", "phosphor-green");
        assert_eq!(run(&mut Pack::default(), &frame), REFUSED_THEME);
        assert!(bench.bytes.iter().all(|b| *b == PAD));
    }

    #[test]
    fn a_descriptor_that_lies_about_its_own_shape_is_refused_rather_than_clamped() {
        let mut bench = Bench::new(8, 4, 40);
        let mut pack = Pack::default();

        let mut narrow = bench.frame("solid", "amber");
        narrow.stride = 8; // narrower than one row of eight pixels
        assert_eq!(run(&mut pack, &narrow), REFUSED_DESCRIPTOR);

        let mut empty = bench.frame("solid", "amber");
        empty.height = 0;
        assert_eq!(run(&mut pack, &empty), REFUSED_DESCRIPTOR);

        let mut nowhere = bench.frame("solid", "amber");
        nowhere.pixels = std::ptr::null_mut();
        assert_eq!(run(&mut pack, &nowhere), REFUSED_DESCRIPTOR);

        let mut huge = bench.frame("solid", "amber");
        huge.stride = u32::MAX;
        huge.height = u32::MAX;
        assert_eq!(
            run(&mut pack, &huge),
            REFUSED_DESCRIPTOR,
            "a surface that cannot be addressed is refused, not truncated"
        );

        let mut alien = bench.frame("solid", "amber");
        alien.format = 0;
        assert_eq!(run(&mut pack, &alien), REFUSED_FORMAT);

        // SAFETY: a null descriptor is the first thing `dispatch` checks, and
        // checking it is what this line is for.
        assert_eq!(
            unsafe { dispatch(&mut pack, std::ptr::null()) },
            REFUSED_DESCRIPTOR
        );
        assert!(bench.bytes.iter().all(|b| *b == PAD));
    }

    #[test]
    fn a_pack_holds_its_state_between_frames() {
        let mut bench = Bench::new(4, 2, 16);
        let frame = bench.frame("solid", "amber");
        let mut pack = Pack::default();
        assert_eq!(run(&mut pack, &frame), DREW);
        assert_eq!(run(&mut pack, &frame), DREW);
        assert_eq!(pack.frames, 2, "a background animates, so it remembers");
    }

    #[test]
    fn an_id_survives_the_crossing_and_an_unnameable_one_never_starts_it() {
        let there = id_bytes("digital-rain").expect("nameable");
        assert_eq!(id_str(&there), Some("digital-rain"));

        // Exactly full: no room for a terminator, and it still reads back.
        let long = "a".repeat(ID_BYTES);
        let full = id_bytes(&long).expect("exactly ID_BYTES fits");
        assert_eq!(id_str(&full), Some(long.as_str()));

        assert!(id_bytes(&"a".repeat(ID_BYTES + 1)).is_none());
        assert!(id_bytes("../etc/passwd").is_none());
        assert!(id_bytes("Digital Rain").is_none());
        assert!(id_bytes("").is_none());

        // What a host in another language could hand over.
        let mut rubbish = [0xFFu8; ID_BYTES];
        assert!(id_str(&rubbish).is_none());
        rubbish[0] = 0;
        assert!(id_str(&rubbish).is_none(), "an empty id is not an id");
    }

    /// **The codes are the contract too**, because the two sides of this
    /// boundary are two builds and a renumbering would turn one refusal into
    /// another silently. Every one of them is distinct and every `Refusal` a
    /// pack can return maps to its own.
    #[test]
    fn every_outcome_has_its_own_code_and_no_two_share_one() {
        let codes = [
            DREW,
            REFUSED_VERSION,
            REFUSED_DESCRIPTOR,
            REFUSED_FORMAT,
            REFUSED_BACKGROUND,
            REFUSED_THEME,
            PANICKED,
            REFUSED_SURFACE,
        ];
        let mut seen = codes.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), codes.len(), "two outcomes share a code");
        assert_eq!(DREW, 0, "success is zero, as every C entry's is");

        assert_eq!(Refusal::UnknownBackground.code(), REFUSED_BACKGROUND);
        assert_eq!(Refusal::UnknownTheme.code(), REFUSED_THEME);
        assert_eq!(Refusal::UnsupportedSurface.code(), REFUSED_SURFACE);
    }

    /// The struct is the contract, so its shape is asserted rather than
    /// assumed. A field added in the middle, or a `bool` where a `u32` was,
    /// fails here instead of on a board.
    #[test]
    fn the_descriptor_has_the_layout_two_builds_agreed_on() {
        let word = std::mem::align_of::<*mut u8>();
        assert_eq!(std::mem::align_of::<Frame>(), word);
        assert_eq!(std::mem::offset_of!(Frame, abi), 0, "the version is first");
        assert_eq!(std::mem::offset_of!(Frame, width), 4);
        assert_eq!(std::mem::offset_of!(Frame, height), 8);
        assert_eq!(std::mem::offset_of!(Frame, stride), 12);
        assert_eq!(std::mem::offset_of!(Frame, format), 16);
        assert_eq!(std::mem::offset_of!(Frame, reserved), 20);
        assert_eq!(std::mem::offset_of!(Frame, pixels), 24);
        assert_eq!(std::mem::offset_of!(Frame, time_ms), 32);
        assert_eq!(std::mem::offset_of!(Frame, background), 40);
        assert_eq!(std::mem::offset_of!(Frame, theme), 40 + ID_BYTES);
        assert_eq!(std::mem::size_of::<Frame>(), 40 + ID_BYTES * 2);
    }

    #[test]
    fn a_surface_hands_back_rows_and_refuses_one_past_the_end() {
        let mut bytes = vec![0u8; 3 * 16];
        let mut surface = Surface::over(&mut bytes, 2, 3, 16).expect("a surface");
        assert_eq!(surface.row(0).expect("a first row").len(), 8);
        assert_eq!(surface.row(2).expect("a last row").len(), 8);
        assert!(surface.row(3).is_none());
        assert!(
            Surface::over(&mut [0u8; 8], 4, 2, 16).is_none(),
            "a buffer smaller than its own geometry is not a surface"
        );
        assert!(
            Surface::over(&mut [0u8; 64], 4, 2, 8).is_none(),
            "a stride narrower than a row is not a surface"
        );

        surface.put(0, 0, 0x00AA_BBCC);
        surface.put(99, 0, 0x0011_2233);
        surface.put(0, 99, 0x0011_2233);
        assert_eq!(&bytes[0..4], &0x00AA_BBCCu32.to_le_bytes());
        assert!(
            bytes[4..].iter().all(|b| *b == 0),
            "an out-of-range plot is clipped rather than wrapped"
        );
    }
}
