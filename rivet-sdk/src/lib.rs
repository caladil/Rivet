//! **What a Rivet hands an application** — `ADR-036`, the application's side.
//!
//! # The first article
//!
//! > An application declares what it needs and is **handed** it. It does not go
//! > and take one.
//!
//! `ADR-036` §2. A program that binds its own port, opens `/dev/dri/card0`
//! by path, or guesses `/state/app/<name>/data` is doing the thing this platform
//! arranges not to need. This crate is the other half of that arrangement: it
//! reads what was handed over and gives it a name.
//!
//! # It links nothing of Rivet's
//!
//! **This said `[dependencies]` is empty, and it has not been true for some
//! time** — the audit of 2026-09-03 (`PLAN.md` §123). It is `libc`, plus
//! `fontdue` behind the optional `text` feature. What `ADR-036` §5 actually
//! rules is that an application is a **foreign program** and linking
//! `rivet-core` would drag an operating system into something that wanted a
//! temperature reading, and that is intact: nothing here depends on any other
//! crate in this workspace.
//!
//! So the constants here are **re-declared**, and `tests/agreement.rs` proves
//! they still match — including that a journal record this crate encodes parses
//! back through `rivet-core` identically, which is the failure that would
//! otherwise be silent (`ADR-036` §8.3).
//!
//! # What is here, and what is not yet
//!
//! | Module | Declared in the manifest as | State |
//! | :-- | :-- | :-- |
//! | [`app`] | nothing — always handed | **built** |
//! | [`listen`] | `listen = ["tcp:8080"]` | **built** |
//! | [`journal`] | nothing — always available | **built** |
//! | [`display`] | selected by the launcher | **built** 2026-09-01 — the channel; see `PLAN.md` §106 |
//! | `serve` | `listen`, plus the `serve` feature | **built** 2026-09-04 — `ADR-040` |
//! | [`secrets`] | nothing — `apply install-secret` names the application | **built** 2026-09-13 — `ADR-038` §5, `.rulings/` R-169 |
//! | introspection | `system-info`, `log = "own"` | ruled, unbuilt — `ADR-036` §8.2 |
//!
//! The last two are `ADR-036` §9's steps 4–6 and are not stubbed here: a module
//! that compiled and did nothing would be worse than one that is absent, because
//! absent is legible.
//!
//! # What this crate cannot do for you
//!
//! `SPEC-01` §5.2 is the list of things a Rivet does not have — no shell, no
//! `$HOME`, no environment a manifest can set, no D-Bus, no `/usr/share/zoneinfo`,
//! no `pkg-config` for the target. **Read it before choosing a dependency.**
//! Nothing here works around any of them.
//!
//! # It compiles on the development host, and that is deliberate
//!
//! **A Rivet application runs on Linux and its author does not.** Until
//! 2026-09-05 this crate declared [`display`] and [`schedule`] unconditionally,
//! so `cargo check --workspace` failed on Windows for **any** workspace that
//! depended on it — the maintainer's own, and both ports that adopted it. One
//! port worked around it by putting the whole dependency behind
//! `[target.'cfg(unix)'.dependencies]`, which cost it the ability to run its own
//! pure tests on the machine it was written on.
//!
//! So the rule now is: **a module is `cfg(unix)` only where the platform is
//! genuinely what it is about.**
//!
//! | Module | On Windows |
//! | :-- | :-- |
//! | [`app`], [`settings`], [`secrets`], [`schedule`], `serve`, `bluetooth`'s decoder | compiles, and its tests run |
//! | [`journal`] | compiles; [`journal::Journal::emit`] falls back to stderr, as it already did wherever there is no ingest socket |
//! | [`listen`] | compiles; `into_tcp_listener` and `into_unix_listener` are absent, because a handed descriptor is a Unix concept |
//! | [`display`] | **absent.** It is `recvmsg` and `SCM_RIGHTS` and nothing else — there is no part of it that is not descriptor passing |
//! | `bluetooth::advertisements` | **absent.** The stream is an `AF_UNIX` socket |
//! | `draw`, `text` | compiles behind the `draw` feature; both are pure Rust |
//! | `screen` | **absent** wherever `libdrm` is not linkable, which is the cross sysroot only |
//!
//! **`cfg(unix)` and not `cfg(target_os = "linux")`**, so a macOS author gets the
//! same surface. Nothing here is Linux-specific below the `libc` call level;
//! `SCM_RIGHTS`, `localtime_r` and `AF_UNIX` are all POSIX.

// **`deny`, not `forbid`, and the difference is one function.**
//
// `forbid` cannot be lifted, which was right while this crate only read
// environment variables. Turning a handed descriptor into a listener needs
// `FromRawFd`, and that is `unsafe` wherever it happens.
//
// It happens HERE so that it happens ONCE. The alternative is every application
// writing the same block, each making the same safety argument privately and
// none of them able to check it — and the argument is the platform's to make,
// not theirs: the supervisor opened the socket, called `listen`, and passed the
// descriptor. `rivet-sdk` is where that promise is written down, so it is where
// the `unsafe` belongs. See [`listen::Socket::into_tcp_listener`].
//
// [`display`] lifts it for its whole module, and says why at the top of that
// file: receiving a descriptor is four unsafe steps that only make sense
// together.
//
// **A third place since 2026-09-02**, and it is a function rather than a
// module: `schedule::local_now` calls `localtime_r(3)`, which is one C call
// with a contract that fits in its own SAFETY note (`ADR-039` §6).
//
// ~~Those three are the whole of it, and every other module is unsafe-free and
// must stay so.~~ **Corrected 2026-09-03.** There is a fourth and it is the
// largest: [`screen`], behind the optional `screen` feature, is a DRM driver
// written against `libdrm` and carries around thirty unsafe blocks. The
// sentence above was written before it existed and was left standing, which the
// audit of 2026-09-03 found (`PLAN.md` §123). It is not a small omission: a
// reader trusting it would conclude this crate had no `unsafe` worth reviewing.
//
// The rule that DOES hold, and is the one to keep: **every module that is not
// `display`, `listen::into_*`, `schedule::local_now` or `screen` is unsafe-free
// and must stay so** — which is `app`, `journal`, `settings`, `draw`, `text`,
// `qoi`, `flip` and `serve`, the whole of what an application touches on the
// ordinary path. (`serve` added 2026-09-04. It parses bytes off a socket, which
// is the module here with the most hostile input and the least business holding
// a raw pointer.)
#![deny(unsafe_code)]

pub mod app;
/// **The screen an application was handed**, and the one module here that is
/// entirely `cfg(unix)`.
///
/// Every line of it is `recvmsg`, the `CMSG_*` layout and an `OwnedFd`. There is
/// no half of it that is about anything else, so there is nothing to keep on a
/// host that has no descriptor passing — unlike [`schedule`], which is a parser
/// with one C call in it, or `bluetooth`, which is a line decoder with one
/// socket in it. Both of those keep their pure halves; this one has none.
#[cfg(unix)]
pub mod display;
// **What this machine is, handed over by its supervisor** — `ADR-057`,
// `.rulings/` R-199 D4. The module's own header is the documentation;
// **a `///` here would be MERGED with it and resolved in THIS file's scope**,
// which silently breaks every intra-doc link the module writes about its own
// types. Twenty-one rustdoc warnings, found by `tools/test-all.sh`'s second
// ratchet the first time this module existed.
pub mod facts;
pub mod health;
pub mod journal;
pub mod listen;
// **Who connected to a socket this application provides** — `SO_PEERCRED`,
// which is Linux's, so the module is Linux-only. A comment and not a doc
// comment, for `facts`' reason.
#[cfg(target_os = "linux")]
pub mod peer;
pub mod schedule;
pub mod secrets;
pub mod settings;
// **The one question the supervisor may not be asked about itself** —
// `ADR-057`, `.rulings/` R-199 D5. Read from `/proc`, in this process. A
// comment and not a doc comment, for [`facts`]'s reason.
pub mod supervisor;

/// **Drawing on the screen a Rivet handed you** — `ADR-036` §5.4A, behind the
/// `draw` feature.
///
/// Extracted from `cmd/rivet-launcher` and `examples/probes/kiosk-demo` rather
/// than written fresh, which is `ADR-036` §5.4A's whole argument: a library
/// extracted from the launcher itself keeps the operating system and the SDK
/// moving together, because changing one means changing the other
/// (.rulings/ R-286).
///
/// Before this, every kiosk application had to bring its own modeset, its own
/// framebuffer and its own glyph rasteriser — about eight hundred lines that
/// have nothing to do with what the application is for.
#[cfg(feature = "draw")]
pub mod draw;

/// **The widget layer both surface kinds can carry** — a `wl_shm` subsurface
/// over the background, so widgets are committed when they CHANGE rather than
/// every frame. See the module for what that saves and why it is not only
/// about the GPU.
///
/// Behind the same features as the two `Screen`s that use it, because it is
/// the toolkit's subcompositor and shm and nothing else.
#[cfg(feature = "composited")]
pub(crate) mod overlay;
/// **Serving HTTP on a socket a Rivet handed you** — `ADR-040`, behind the
/// optional `serve` feature.
///
/// The pieces three programs in this repository each wrote for themselves: a
/// bounded head parser, a body bound, a connection cap, a per-connection
/// deadline and a `rustls::ServerConfig` built from the files the platform
/// handed the application. **No accept loop and no `spawn`**, because
/// `rivet-fleetd` is OS threads and `rivet-proxyd` is tokio and a module that
/// shipped a loop could be adopted by one of them.
///
/// # `cfg(test)` as well, and that is the point of the split
///
/// Everything but `serve::Tls` is pure Rust, so the module compiles into this
/// crate's own test binary with the feature off — which is what makes
/// `cargo test --workspace` run every bound and every refusal below. Only the
/// TLS half needs `rustls`. Same argument as `flip.rs` below, and the same one
/// the `Cargo.toml` note makes about `draw` against `screen`.
#[cfg(any(feature = "serve", test))]
pub mod serve;

/// **Reading BLE advertisements** — `ADR-041`, behind the optional `bluetooth`
/// feature.
///
/// An application declares `connects = ["rivet-bluetoothd:advertisements"]` and
/// reads a line per advertisement off the socket the platform grants it. It
/// does not open a Bluetooth socket, because an HCI user channel is exclusive
/// and the platform holds it.
///
/// # A weaker case for a feature than `draw` and `serve`, and it is worth
/// saying so
///
/// Those two gate real weight: `fontdue` and `libdrm` for one, `rustls` and
/// `ring` for the other, and an application that does not draw has no business
/// linking a glyph rasteriser. **This gates no dependency at all** — it is
/// `std` and a line parser — so the honest claim is smaller: what opting in
/// buys is that the surface is absent from an application that never asked for
/// it, and that the Cargo feature and the manifest key travel together in the
/// author's head. `.rulings/` R-144's acceptance criteria asked for it as an
/// opt-in and that is the shape it takes; pretending the argument is `draw`'s
/// would be overstating it.
///
/// `cfg(test)` for `serve`'s reason: the whole module is pure enough to compile
/// into this crate's test binary with the feature off, so
/// `cargo test --workspace` runs every refusal in it.
#[cfg(any(feature = "bluetooth", test))]
pub mod bluetooth;

/// The QOI encoder behind [`screen::Screen::capture`].
#[cfg(feature = "screen")]
mod qoi;
/// **The page-flip bookkeeping**, with no C in it.
///
/// `cfg(test)` as well as `cfg(feature = "screen")`, and that is the point of
/// the file existing at all: `screen` links `libdrm`, which lives only in the
/// aarch64 sysroot, so a test binary that enables it does not link on the
/// development host. This module is where the parts with judgement in them go,
/// so that `cargo test -p rivetos-sdk` reaches them anywhere. `AGENTS.md` §4 —
/// *a test suite that cannot be run will rot silently.*
///
/// **It was written against `draw` and re-gated onto `screen` at the merge**,
/// 2026-09-01: the two changes were built in parallel and the other one had
/// already split the display out of the drawing primitives for this exact
/// reason. The argument is unchanged and the feature it names is not.
#[cfg(any(feature = "screen", test))]
mod flip;

/// **What a descriptor's connectors amount to**, with no C in it.
///
/// [`flip`]'s argument applied to the other half of `screen.rs` that has
/// judgement in it: which of four different things went wrong when no output
/// could be driven, and what to tell an operator about each. Gated the same way
/// and for the same reason — a message nothing can compile is a message nothing
/// can test, and this one sent a session looking at a cable that had not moved
/// (`ADR-042` §8.4).
#[cfg(any(feature = "screen", test))]
mod outputs;

/// The display itself: two pages, a mode, and one call to put a frame on the
/// screen.
///
/// **Behind its own `screen` feature since 2026-09-01**, and `draw` is now
/// pure Rust without it. This module is the only thing here that links
/// `libdrm`; while it shared a feature with the drawing primitives, enabling
/// them on a host with no cross-compiled `libdrm` failed to link, so
/// `draw.rs`'s tests could not be run anywhere. See the `Cargo.toml` note.
#[cfg(feature = "screen")]
pub mod screen;
/// **A screen handed over by the compositor** — `ADR-058` §5, ruled
/// 2026-09-20.
///
/// [`screen`] above is the DRM lease that `ADR-058` retired; this is its
/// replacement, and it is deliberately the same five things — `width`,
/// `height`, `stride`, `what` and `blit` — so an application moves between
/// them by changing which one it asks for and nothing else.
///
/// **Behind its own feature and not `screen`'s**, because the two are
/// alternatives rather than layers: `screen` links `libdrm` and this links
/// nothing at all. An application declaring `connects =
/// ["rivet-displayd:wayland"]` uses this and gives up `devices = ["display"]`
/// with every confinement grant that came with it.
#[cfg(feature = "composited")]
pub mod composited;
/// **The same surface, drawn on the GPU** — `.rulings/` R-206, ruled
/// 2026-09-20.
///
/// [`composited`] above hands the compositor PIXELS; this hands it a
/// DESCRIPTOR. An application renders with GL into a buffer on its own render
/// node and the frame never leaves the GPU, which is the only arrangement that
/// is faster than drawing on the CPU — `GPU-Backgrounds-2026-09-20` §3
/// measured the readback that makes the half-way version slower than either.
///
/// Behind its own feature because it links `libgbm`, `libEGL` and `libGLESv2`,
/// which [`composited`] does not, and because an application wanting a CPU
/// surface must not pay for Mesa.
#[cfg(feature = "composited-gpu")]
pub mod gpu;
/// **One `wl_buffer` per GPU buffer object, for that object's life**, the rule
/// `gpu` presents by, with no C in it. Gated like `flip` and for its reason:
/// `gpu` links Mesa, which the development host cannot, so the rule lives where
/// `cargo test -p rivetos-sdk` reaches it anywhere.
#[cfg(any(feature = "composited-gpu", test))]
mod attached;
/// Type, measured and rasterised.
#[cfg(feature = "draw")]
pub mod text;

pub use journal::Severity;

/// An environment variable the supervisor did not set.
///
/// **Named rather than defaulted.** Every variable this crate reads is one a
/// Rivet promises; its absence means the program is running somewhere that made
/// no such promise — a development host, a plain Linux box, a container — and a
/// silent fallback would put an application's data somewhere nobody is looking.
/// `ADR-036` §5.2 is the rule; this type is how it is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotHanded {
    /// The variable that was not set.
    pub variable: &'static str,
}

impl std::fmt::Display for NotHanded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is not set, so this process was not started by a Rivet supervisor. \
             See ADR-036 §3 for what a Rivet hands an application.",
            self.variable
        )
    }
}

impl std::error::Error for NotHanded {}
