//! **What a host application and an addon say to each other.**
//! `PLAN-Feature-Set-2026-09-12` §6.3; `.rulings/` R-196, R-197, R-198.
//!
//! `ADR-053` decides *which* library joins *which* application — a class both
//! sides sign — and stops there on purpose. This crate is the other half: the
//! calls that cross once a host has opened an addon, and the rules that keep
//! that crossing from being the most dangerous line in the system.
//!
//! # The shape, in one paragraph
//!
//! An addon is a `cdylib` with three C symbols. The host asks it to
//! [`Request::Describe`] itself, hands it its settings with
//! [`Request::Configure`], and then calls [`Request::Poll`] on a cadence the
//! addon itself chooses, one answer at a time. Requests and answers cross as
//! **serialized bytes**, never as Rust types — so nothing depends on the two
//! sides having been compiled by the same compiler, which is the whole reason
//! this is not a trait object.
//!
//! # Five rules, and every one of them is a defect that did not happen
//!
//! 1. **No type layout crosses.** Bytes do. Rust promises nothing about layout
//!    between separate compilations, and a direct call would bet the host's
//!    address space on the two builds agreeing. That bet fails as memory
//!    corruption rather than as an error, which is `ADR-053` §6's argument and
//!    this crate's first rule. **[`frame`] is the one exception, confirmed by
//!    the maintainer knowing it is one** (`.rulings/` R-199 D2): a background
//!    drawing thirty frames a second into a 1080p surface cannot serialize
//!    them, and what crosses instead is `#[repr(C)]` over fixed-width scalars
//!    and a pointer — the C ABI, which is the one layout two separately
//!    compiled Rust builds do agree on. That module's header is the whole
//!    argument, including why the exception stops there.
//! 2. **Versions agree before any bytes move.** [`abi::VERSION`] is read
//!    through its own symbol, so a host refuses an addon it cannot speak to
//!    without having parsed anything the addon wrote.
//! 3. **The allocating side frees.** An answer is allocated by the addon and
//!    returned to the addon's own [`abi::FREE_SYMBOL`]. Calling `free` across
//!    two Rust allocators is heap corruption with a delay on it.
//! 4. **Nothing unwinds across the boundary.** Unwinding out of an
//!    `extern "C"` function is undefined behaviour, so the generated glue
//!    catches it and returns a fault. A panicking addon costs its answer, not
//!    the host — **but only when the addon `cdylib` is itself built with
//!    `panic = "unwind"`.** [`std::panic::catch_unwind`] has nothing to catch
//!    when the crate compiling it was built with `panic = "abort"`: the
//!    addon's first panic then aborts the whole host process that loaded it
//!    — the kiosk renderer holding the display, or whatever else joined it —
//!    instead of returning the fault this crate exists to produce. See
//!    [`export_addon!`]'s own section on this.
//! 5. **The unsafe lives here.** An addon crate carries
//!    `#![forbid(unsafe_code)]`; its `extern "C"` entry is written by
//!    [`export_addon!`], and the host's side by [`host`]. Two files in this
//!    crate are the whole surface, and they are the ones to read twice.
//!
//! # What this cannot do, stated plainly
//!
//! A loaded addon runs **in the host's process**, so it inherits the host's
//! Landlock window, its uid, its network permission and its data directory.
//! Those are kernel-enforced and an addon cannot exceed them — it can do
//! nothing the host could not do. What is lost is memory isolation *inside*
//! those privileges: an addon that writes through a wild pointer corrupts the
//! host, and no rule in this crate can prevent that. The guards are the five
//! above, plus [`trial`], which makes such a crash cost one restart rather
//! than a crash loop. An addon that links C is outside all of them, and is a
//! policy question rather than a technical one (R-198 D3).
//!
//! # Where the pieces are
//!
//! * [`protocol`] — the messages and the typed field vocabulary an addon's
//!   settings are described in.
//! * [`abi`] — the three symbols, the calling convention, and
//!   [`export_addon!`], which is how an addon author writes none of it.
//! * [`frame`] — the drawing entry: a versioned `#[repr(C)]` descriptor, a
//!   safe [`frame::Surface`] the addon writes into, and
//!   [`export_drawing_addon!`].
//! * [`host`] — opening an addon, the handshake, the call and the frame.
//!   `cfg(unix)`.
//! * [`trial`] — the note a host writes before it loads an addon, so that an
//!   addon which killed it last time is skipped and named.
//! * [`gl`] — [`frame`]'s twin for a pack that draws with the GPU. The host
//!   owns the EGL context and binds the target; the pack is handed the bound
//!   framebuffer and draws into it (`.rulings/` R-206 D1). A pack may export
//!   both halves, and R-207's does.

pub mod abi;
pub mod frame;
pub mod gl;
#[cfg(unix)]
pub mod host;
pub mod protocol;
pub mod trial;

pub use frame::{Drawing, Refusal, Scene, Surface};
pub use gl::{DrawingGl, GlTarget};
pub use protocol::{
    Background, CardAppearance, Cards, Description, Drawn, Failure, Field, FieldKind, Measured, Polled,
    Reading, Request, Response, Status, Theme, Trend,
};
