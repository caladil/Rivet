//! **`aura-background-pack`** — the default multipack: **seven backgrounds
//! and twenty-two themes.**
//!
//! Digital Rain, Boxes and Solid were moved constant for constant out of
//! `examples/aura/aura-renderer` (`.rulings/` R-199 D1,
//! `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T6). Starfield, Aurora,
//! Contours and Conway's Game of Life were designed here, to §6 of
//! `docs/design-candidates/Background-Addons-2026-09-19.md` (T8).
//!
//! # What it is
//!
//! A `cdylib` carrying `provides-class = "aura-background"`. Two kinds of
//! host accept that class, each for its own reason
//! (`docs/design-candidates/Background-Addons-2026-09-19.md` §2): a renderer
//! loads it to **draw**, through [`rivetos_addon::frame`]'s binary entry
//! point (`.rulings/` R-199 D2); a controller loads it to **describe**,
//! through the ordinary JSON `Describe`, for a portal's two pickers. Both are
//! ordinary applications accepting one class — nothing new was needed on the
//! platform side to make a multipack possible, only `Description::backgrounds`
//! (`rivet-addon/src/protocol.rs`), which this pack's [`catalogue`] fills in.
//!
//! # The shape of this crate
//!
//! * [`background`] — the three ported backgrounds, unchanged. Read its
//!   module doc comment first: *"there is no design in it."* It also holds
//!   the `Background` enum and `Screensaver`, which dispatch to all seven.
//! * [`designed`] — the four T8 added: Starfield, Aurora, Contours and
//!   Conway's Game of Life. Separate from `background` on purpose, so the
//!   sentence above keeps being true of that file.
//! * [`rng`] and [`typeface`] — what the port needs, also ported (`rng`
//!   unchanged; `typeface` trimmed to what a background actually calls, and
//!   rebuilt on [`embedded_face`] rather than on `rivet_sdk::text::Face`
//!   directly — see that module for why).
//! * [`catalogue`] — this pack's own new code: the table an id crosses the
//!   wire as, in both directions.
//! * [`render`] — this pack's other new code: the seam between the ported
//!   drawing code's flat canvas and the addon boundary's [`Surface`].
//! * [`BackgroundPack`] (below) — the object that ties them to
//!   [`rivetos_addon::abi::Addon`] and [`Drawing`].
//!
//! `#[cfg(test)] mod evidence;` is T6's proof and ships in no build: see its
//! own doc comment.

//! # `unsafe` in this crate: one module, and the reason it stopped being none
//!
//! This crate was `#![forbid(unsafe_code)]` from the day it was split out, and
//! that was worth having: a pack is `dlopen`ed into the renderer holding the
//! display, and `rivetos_addon::frame`'s whole design — `Surface`, *"the host
//! owns the surface"* — is about a pack being unable to corrupt its host even
//! by accident.
//!
//! **`.rulings/` R-206 D1 makes that impossible to keep in full.** A pack that
//! draws with the GPU issues GL calls, and there is no safe way to call a C
//! library. So the guarantee narrows rather than disappearing:
//!
//! * **`deny`, not `forbid`**, so exactly one module can opt out and every
//!   other one still cannot — including the CPU backgrounds, which is where
//!   all five of them still live.
//! * **[`gles`] and [`gpu`] are the only modules that opt out** — the
//!   resolver, and the GL calls made through what it resolved. Everything in
//!   them is either `dlsym` or a GLES entry point transcribed from the
//!   specification. No pointer from the host is dereferenced in either,
//!   because the GPU boundary hands over integers rather than memory — which
//!   is `rivetos_addon::gl`'s own argument for having fewer checks than
//!   `frame`.
//!
//! The property that actually protected the host is therefore unchanged: this
//! pack still cannot write outside a buffer the host lent it, because on the
//! GPU path it is never lent one.

#![deny(unsafe_code)]

mod background;
mod catalogue;
mod designed;
mod embedded_face;
mod gles;
mod gpu;
mod render;
mod rng;
mod typeface;

#[cfg(test)]
mod evidence;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use rivetos_addon::abi::Addon;
use rivetos_addon::{
    Description, Drawing, DrawingGl, Polled, Refusal, Scene, Status, Surface,
};

use background::Screensaver;
use typeface::Typeface;

/// The atlas size Digital Rain's glyphs were built at — `background.rs`'s
/// own `CELL_H`, kept private there (matching the renderer's copy exactly)
/// and repeated here as a literal, the same way `aura-renderer/src/main.rs`
/// repeats it at its own font-loading call site (`face(RAIN_FONT, 18.0)`)
/// rather than reaching into `crate::background` for a constant it does not
/// export.
const DIGITAL_RAIN_ATLAS_SIZE: f32 = 18.0;

/// One selected background, mid-animation, and when it was last asked to
/// advance.
struct Current {
    background_id: String,
    theme_id: String,
    width: u32,
    height: u32,
    saver: Screensaver,
    /// `None` for the very first frame after `saver` was built — see
    /// [`BackgroundPack::draw`] for what that does to `dt`.
    last_time_ms: Option<u64>,
}

/// **The object.** One instance answers both halves of the protocol — the
/// catalogue [`BackgroundPack::describe`] returns is the catalogue
/// [`BackgroundPack::draw`] draws from, because [`export_drawing_addon!`]
/// wires `Addon` and `Drawing` to the SAME static instance behind one
/// `Mutex` (`rivetos_addon::frame`'s own doc comment on
/// `export_drawing_addon!` says why that matters: describing and drawing
/// have to be one pack talking about one list, not two that happen to agree
/// today).
#[derive(Default)]
pub struct BackgroundPack {
    /// Loaded once, on first use, from `assets/AuraRain.ttf` embedded in
    /// this object — see `typeface.rs` and `embedded_face.rs` for why a
    /// shared library rasterises its own font from bytes rather than from a
    /// path.
    rain_face: OnceLock<Typeface>,
    current: Option<Current>,
    /// **The scratch canvas for a host whose surface has padding between
    /// rows**, kept here so it is grown once rather than allocated per frame.
    /// Empty and never touched on a host whose surface is tightly packed,
    /// which is the path `render::render_frame` takes when it can — see that
    /// module's header for what the two cost.
    ///
    /// **The GPU path reuses it** to hold a frame's texture bytes — 5,184,
    /// 28,800 or 192 of them, against the 8 MB this holds for the CPU path.
    /// One buffer because the two paths never run in the same frame: a host
    /// asks for a frame one way or the other, and the one it did not ask for
    /// is not resizing this behind it.
    canvas: Vec<u8>,
    /// **The GL objects, once a GPU frame has been asked for.**
    ///
    /// `Option<Option<_>>` and both layers mean something. The outer is *"has
    /// this been attempted"*; the inner is *"did it work"*. A machine with no
    /// V3D fails once and is remembered as having failed — without the inner
    /// `None` the pack would try to compile three shaders on every frame of a
    /// host that should never have asked, thirty times a second, for ever.
    gpu: Option<Option<gpu::Gpu>>,
}

impl Addon for BackgroundPack {
    fn describe(&mut self) -> Description {
        Description {
            name: "aura-background-pack".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            class: "aura-background".to_string(),
            fields: Vec::new(),
            material: Vec::new(),
            backgrounds: catalogue::describe(),
        }
    }

    fn configure(
        &mut self,
        _settings: &serde_json::Map<String, serde_json::Value>,
        _material: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        // No settings and no material — see `describe`. Nothing to validate.
        Ok(())
    }

    fn poll(&mut self, _now_ms: i64) -> Polled {
        // **A pack has no reading to give**, `examples/probes/frame-probe`'s
        // own `poll` carries the same answer for the same reason: `Status::
        // Unavailable` is what says so, and it is not a fault.
        Polled {
            reading: None,
            status: Status::Unavailable,
            // An hour. A pack is asked to describe itself once and then asked
            // to draw; there is nothing to poll for in between.
            call_again_in_ms: 3_600_000,
            complaint: "this addon draws rather than measures".to_string(),
            // `Polled::also` is for a source that measured more than one thing
            // in one poll. This measured nothing at all, so it is empty here
            // for the same reason `reading` is `None`.
            also: Vec::new(),
        }
    }
}

impl Drawing for BackgroundPack {
    fn draw(&mut self, surface: &mut Surface<'_>, scene: &Scene<'_>) -> Result<(), Refusal> {
        let background = catalogue::resolve(scene.background, scene.theme)?;
        let width = surface.width();
        let height = surface.height();

        // **Borrowed from the field directly, and BEFORE `self.current` is
        // borrowed below.** A `fn rain_face(&self)` helper method would look
        // tidier, but calling it after `self.current.as_mut()` borrows the
        // whole of `self` for the length of that call, which the borrow
        // checker cannot reconcile with the still-live mutable borrow of
        // `self.current` — even though the two fields are disjoint. A direct
        // field projection, taken first, lets the two borrows coexist.
        let rain_face = self.rain_face.get_or_init(|| {
            Typeface::load_embedded(DIGITAL_RAIN_ATLAS_SIZE)
                .expect("assets/AuraRain.ttf ships inside this crate and has always parsed")
        });

        let current = advance(&mut self.current, background, scene, width, height);
        render::render_frame(&current.saver, surface, rain_face, &mut self.canvas)
    }
}

/// **Select the right screensaver and step it one frame.**
///
/// A free function taking `&mut Option<Current>` rather than a method, for two
/// reasons. The borrow checker is the smaller one: [`Drawing::draw`] holds a
/// borrow of `self.rain_face` across this call, and a `&mut self` method would
/// borrow the whole object.
///
/// **The larger one is that there are now two drawing paths and this is the
/// only copy of the timing.** A `dt` clamp duplicated between the CPU and GPU
/// entry points is two things that must stay identical, held apart by nothing
/// but attention — and the symptom of them drifting is a background that
/// animates at a different speed depending on which path drew it, which is
/// exactly the kind of difference nobody attributes to a missing edit.
///
/// **`dt` from consecutive `time_ms`, clamped as `main.cpp` and
/// `aura-renderer/src/main.rs` clamp theirs**: `[0.001, 0.1]`, so a long stall
/// between frames cannot teleport every column down the screen. The very first
/// frame after a fresh [`Screensaver`] has no previous `time_ms` to diff
/// against and advances by one nominal 1/30s tick instead — the same "frame 1
/// is after one update call" reading `evidence.rs` and the renderer's baseline
/// both use, so a freshly selected background looks the same one frame in
/// whichever path it got there through.
fn advance<'a>(
    slot: &'a mut Option<Current>,
    background: background::Background,
    scene: &Scene<'_>,
    width: u32,
    height: u32,
) -> &'a mut Current {
    let needs_new = match &slot {
        Some(current) => {
            current.background_id != scene.background
                || current.theme_id != scene.theme
                || current.width != width
                || current.height != height
        }
        None => true,
    };
    if needs_new {
        *slot = Some(Current {
            background_id: scene.background.to_string(),
            theme_id: scene.theme.to_string(),
            width,
            height,
            saver: Screensaver::new(background, width as usize, height as usize),
            last_time_ms: None,
        });
    }
    // Set to `Some` just above whenever it was `None` or stale, so this is
    // always the screensaver that matches `scene`.
    let current = slot
        .as_mut()
        .expect("just set to Some above whenever it was None or stale");
    let dt = match current.last_time_ms {
        Some(last) => {
            let delta_ms = scene.time_ms.saturating_sub(last);
            #[allow(clippy::cast_precision_loss)]
            let seconds = delta_ms as f32 / 1000.0;
            seconds.clamp(0.001, 0.1)
        }
        None => 1.0 / 30.0,
    };
    current.last_time_ms = Some(scene.time_ms);
    current.saver.update(dt);
    current
}

impl DrawingGl for BackgroundPack {
    /// **Draw one frame with the V3D**, into the framebuffer the host bound.
    ///
    /// The shape is [`Drawing::draw`]'s and the difference is one line: the
    /// same catalogue resolves the ids, the same [`advance`] steps the same
    /// simulation by the same `dt`, and then the state goes to a shader
    /// instead of to `render_frame`.
    ///
    /// # What the refusals mean here
    ///
    /// [`Refusal::UnknownBackground`] covers two different situations on
    /// purpose. One is an id this pack does not have, which is what it means
    /// on the CPU path too. The other is an id it **has** but cannot draw this
    /// way — Digital Rain, Starfield, Aurora, Solid — and a host should never
    /// reach it, because [`rivetos_addon::Drawn`] in the catalogue told it
    /// which backgrounds have a GPU path before it asked. Reaching it means
    /// the host ignored the catalogue, and the honest answer to *"draw
    /// starfield on the GPU"* is the same refusal as for an unknown id: the
    /// pack offers no GPU drawing of it.
    fn draw_gl(
        &mut self,
        target: &rivetos_addon::GlTarget,
        scene: &Scene<'_>,
    ) -> Result<(), Refusal> {
        let background = catalogue::resolve(scene.background, scene.theme)?;
        let current = advance(
            &mut self.current,
            background,
            scene,
            target.width(),
            target.height(),
        );
        let Some(uploaded) = current.saver.uploaded(&mut self.canvas) else {
            return Err(Refusal::UnknownBackground);
        };
        // **A GL failure is `UnsupportedSurface`, and the sentence is lost.**
        // `rivetos_addon::frame`'s header is why: nothing allocated crosses a
        // boundary used thirty times a second, so a shader that would not
        // compile becomes a code here and the host composes the prose. The
        // driver's own log is the thing worth having and it is the thing that
        // cannot cross — which is an argument for a pack having built its
        // programs before a host is waiting on a frame, and a reason this is
        // the first thing to revisit if a board ever refuses silently.
        self.gpu
            .get_or_insert_with(|| gpu::Gpu::new().ok())
            .as_mut()
            .ok_or(Refusal::UnsupportedSurface)?
            .draw(&uploaded, target.width(), target.height())
            .map_err(|_| Refusal::UnsupportedSurface)
    }
}

rivetos_addon::export_drawing_addon_with_gl!(BackgroundPack);

#[cfg(test)]
mod tests {
    use super::*;
    use rivetos_addon::frame::id_bytes;

    /// The catalogue `describe()` returns is exactly the one `resolve()`
    /// answers against — the same "one table, two readers" property
    /// `catalogue.rs`'s own doc comment states, checked from the object a
    /// host actually gets rather than from the table directly.
    #[test]
    fn every_described_id_pair_resolves_and_every_id_fits_a_frame() {
        let mut pack = BackgroundPack::default();
        let description = pack.describe();
        assert_eq!(description.class, "aura-background");
        assert_eq!(description.backgrounds.len(), 7);
        for background in &description.backgrounds {
            assert!(id_bytes(&background.id).is_some());
            for theme in &background.themes {
                assert!(id_bytes(&theme.id).is_some());
                assert!(
                    catalogue::resolve(&background.id, &theme.id).is_ok(),
                    "{}/{} is described but does not resolve",
                    background.id,
                    theme.id
                );
            }
        }
    }

    /// **One instance describes and draws.** Constructing a fresh
    /// `BackgroundPack` and drawing through it must not panic and must
    /// change a buffer that started as a sentinel — the object end-to-end,
    /// not just its parts.
    #[test]
    fn a_fresh_pack_draws_a_described_background_without_panicking() {
        let mut pack = BackgroundPack::default();
        let description = pack.describe();
        let solid = description
            .backgrounds
            .iter()
            .find(|b| b.id == "solid")
            .expect("solid is in the catalogue");
        let theme = &solid.themes[0];

        let (width, height, stride) = (8u32, 4u32, 32u32);
        let mut bytes = vec![0xABu8; (stride * height) as usize];
        let mut surface = Surface::over(&mut bytes, width, height, stride).expect("a surface");
        let scene = Scene {
            background: &solid.id,
            theme: &theme.id,
            time_ms: 1_000,
        };
        pack.draw(&mut surface, &scene).expect("a described pair draws");
        assert!(bytes.iter().any(|b| *b != 0xAB), "nothing was drawn");
    }

    /// An id the catalogue does not have is refused, not guessed at.
    ///
    /// **`starfield` was this test's unknown background until T8 built one.**
    /// The stand-in has to be something this pack genuinely does not have,
    /// and a test written against an absence acquires a maintenance cost the
    /// day the absence is filled — which is the whole of why this note is
    /// here rather than a silently swapped string.
    #[test]
    fn an_unknown_background_or_theme_is_refused() {
        let mut pack = BackgroundPack::default();
        let mut bytes = vec![0u8; 64];
        let mut surface = Surface::over(&mut bytes, 4, 4, 16).expect("a surface");
        assert_eq!(
            pack.draw(
                &mut surface,
                &Scene {
                    background: "mandelbrot",
                    theme: "ice-blue",
                    time_ms: 0
                }
            ),
            Err(Refusal::UnknownBackground)
        );
        assert_eq!(
            pack.draw(
                &mut surface,
                &Scene {
                    background: "solid",
                    theme: "phosphor-green",
                    time_ms: 0
                }
            ),
            Err(Refusal::UnknownTheme)
        );
    }
}
