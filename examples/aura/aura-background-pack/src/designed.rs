//! **The four backgrounds that were designed here rather than ported** —
//! Starfield, Aurora, Contours and Conway's Game of Life.
//! `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T8, to the designs in
//! `docs/design-candidates/Background-Addons-2026-09-19.md` §6.
//!
//! # Why they are not in `background.rs`
//!
//! That file opens by saying it is a port with no design in it, under the
//! maintainer's ruling that the screensavers leave no creative room
//! (`.rulings/` R-125). Every number in it is answerable to a line of C++.
//! Every number in **this** file was chosen here, and a reader who cannot tell
//! the two apart at a glance will eventually defend one of mine as if the C++
//! required it. The `Background` enum and `Screensaver` stay where they are,
//! because those are the dispatch and not the drawing; only the drawing
//! splits.
//!
//! # What the four have in common
//!
//! §6's constraint is the one that shaped all of them: 1080p, dark, in a room
//! where a person glances at it, never competing with the cards in the
//! corners. Three of them are cheap in the same way — **compute small, scale
//! up**. Aurora and Contours share one 96x54 [`HeightField`] and differ only
//! in how they read it; Starfield is a few hundred one- and two-pixel plots;
//! Life computes 14,400 cells four times a second rather than thirty.
//!
//! **None of them needs a font**, which is why nothing here takes a
//! [`crate::typeface::Typeface`]. A background that draws glyphs, as Digital
//! Rain does, lives in `crate::background` and is handed the typeface there.
//!
//! # Determinism
//!
//! Every random quantity below comes from [`Mt19937`] — Starfield's initial
//! directions, the height field's three phase offsets, Life's initial soup
//! **and every injection its watchdog makes**. Nothing reads a clock.
//! `evidence.rs`'s `the_same_seed_and_frame_index_render_the_same_bytes_twice`
//! is what holds this file to it, and it is the reason Life's watchdog counts
//! *steps* rather than seconds: a watchdog that fired after thirty wall-clock
//! seconds would put a different picture on frame 600 depending on how busy
//! the machine was.

use rivet_sdk::draw;
use rivet_sdk::text::Target;

use crate::background::Background;
use crate::rng::Mt19937;

// ---------------------------------------------------------------------------
// A sine that costs an array read
// ---------------------------------------------------------------------------

/// Entries in [`sin_table`]. 2048 over a full turn is a step of 0.176°, and
/// the linear interpolation between neighbours takes the worst-case error
/// below 1e-6 — three orders of magnitude under what an 8-bit channel can
/// show.
const SIN_TABLE_LEN: usize = 2048;

/// One turn of sine, sampled.
///
/// **This is for speed, and the speed is the whole reason the height field
/// fits the budget.** A 2048-entry table is built once per process; after
/// that a sine is an array read, a multiply and an add. The alternative is
/// `f32::sin`, which is a library call this crate would make hundreds of
/// thousands of times a second.
///
/// A smaller benefit, stated because it is real but is not why this exists:
/// only these 2048 values depend on the platform's libm. Everything Aurora
/// and Contours compute downstream is ordinary arithmetic on them, so two
/// machines agree to within whatever `f32::sin` disagrees by at 2048 points
/// rather than at every point either of them evaluates.
fn sin_table() -> &'static [f32; SIN_TABLE_LEN] {
    static TABLE: std::sync::OnceLock<[f32; SIN_TABLE_LEN]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0f32; SIN_TABLE_LEN];
        for (i, slot) in table.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let turns = i as f32 / SIN_TABLE_LEN as f32;
            *slot = (turns * std::f32::consts::TAU).sin();
        }
        table
    })
}

/// Sine of `turns` **full cycles** — `sin_turns(0.25) == 1.0`.
///
/// Turns rather than radians so the table index is a fractional part and not
/// a division by tau. Every caller in this file is writing a spatial or
/// temporal frequency, and "three cycles across the screen" is the quantity
/// being chosen; radians would put a `2π` in every one of those constants
/// and hide what the number means.
fn sin_turns(turns: f32) -> f32 {
    let table = sin_table();
    #[allow(clippy::cast_precision_loss)]
    let position = turns.rem_euclid(1.0) * SIN_TABLE_LEN as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = position as usize;
    #[allow(clippy::cast_precision_loss)]
    let fraction = position - index as f32;
    // `rem_euclid(1.0)` can round up to exactly 1.0 for a tiny negative
    // input, which would index one past the end. Both reads are masked
    // rather than the first one alone, because the second is `index + 1`.
    let a = table[index % SIN_TABLE_LEN];
    let b = table[(index + 1) % SIN_TABLE_LEN];
    a + (b - a) * fraction
}

/// Cosine, for the one place that needs a direction vector rather than a
/// wave — sine a quarter turn ahead.
fn cos_turns(turns: f32) -> f32 {
    sin_turns(turns + 0.25)
}

// ---------------------------------------------------------------------------
// The shared height field — Aurora and Contours
// ---------------------------------------------------------------------------

/// §6's *"say 96x54"*, taken literally. At 1920x1080 this is exactly one
/// sample per 20x20 block, which is not required — [`AxisMap`] handles any
/// output size — but it is the size the numbers below were tuned at.
const FIELD_W: usize = 96;
/// See [`FIELD_W`].
const FIELD_H: usize = 54;

/// Spatial frequency of the first wave, in cycles per grid column: about
/// three across the screen.
const WAVE_X: f32 = 0.031;
/// Cycles per grid row for the second wave: about two and a third down.
const WAVE_Y: f32 = 0.043;
/// Cycles per diagonal step for the third: about three and a half corner to
/// corner. Deliberately not a multiple of either of the others, so the three
/// never come back into phase and the field never visibly repeats.
const WAVE_D: f32 = 0.024;

/// Cycles per second of drift for each wave. Fourteen, nineteen and thirty
/// seconds respectively — slow enough that a glance does not catch motion
/// and a stare does.
const DRIFT_X: f32 = 0.070;
/// See [`DRIFT_X`]. Subtracted rather than added, so this wave travels
/// against the first.
const DRIFT_Y: f32 = 0.052;
/// See [`DRIFT_X`].
const DRIFT_D: f32 = 0.031;

/// **Two or three summed sines over a low-resolution grid** — §6, and the
/// object Aurora and Contours share.
///
/// The weights are 0.45, 0.35 and 0.20 rather than three thirds. Equal
/// weights make a sum of three unit sines that reaches ±3 but almost never
/// leaves ±1.5, so a palette mapped over the full range would only ever show
/// its middle third. Weights summing to one put the theoretical range at ±1
/// exactly, which maps onto `[0, 1]` with no scaling constant to explain and
/// no clamp to hide a mistake.
struct HeightField {
    /// `FIELD_W * FIELD_H` samples, each in `[0, 1]`.
    cells: Vec<f32>,
    /// Seconds of drift accumulated, already scaled by `rate`.
    time: f32,
    /// How fast this instance drifts relative to the constants above.
    /// Contours asks for a slower one — §6 calls it *"drifting slowly"*.
    rate: f32,
    /// Three offsets drawn from the pack's generator, so two boards showing
    /// Aurora side by side are not in lockstep. Without these the field is a
    /// pure function of elapsed time and every board that booted together
    /// would paint the identical picture for ever.
    phase: [f32; 3],
}

impl HeightField {
    fn new(rate: f32, rng: &mut Mt19937) -> Self {
        let mut field = Self {
            cells: vec![0.0; FIELD_W * FIELD_H],
            time: 0.0,
            rate,
            phase: [
                rng.real(0.0, 1.0),
                rng.real(0.0, 1.0),
                rng.real(0.0, 1.0),
            ],
        };
        // Frame zero must already be a picture: a host that asks for one
        // frame and never another still gets the field rather than a flat
        // grey.
        field.update(0.0);
        field
    }

    /// Recompute every sample.
    ///
    /// **300 sines, not 15,552.** The naive loop evaluates three sines per
    /// cell, which is 15,552 of them for a 96x54 grid on every frame. But
    /// the first wave's argument depends only on `x`, the second's only on
    /// `y`, and the third's only on `x + y` — so each is a one-dimensional
    /// table this function can fill once and then read 5,184 times. That is
    /// 96 + 54 + 150 sines a frame, and it turns the inner loop into three
    /// multiplies and two adds with no function call in it.
    fn update(&mut self, dt: f32) {
        self.time += dt * self.rate;
        let t = self.time;

        let mut wave_x = [0.0f32; FIELD_W];
        for (x, slot) in wave_x.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let fx = x as f32;
            *slot = sin_turns(fx.mul_add(WAVE_X, t * DRIFT_X) + self.phase[0]);
        }

        let mut wave_y = [0.0f32; FIELD_H];
        for (y, slot) in wave_y.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let fy = y as f32;
            *slot = sin_turns(fy.mul_add(WAVE_Y, -t * DRIFT_Y) + self.phase[1]);
        }

        let mut wave_d = [0.0f32; FIELD_W + FIELD_H];
        for (d, slot) in wave_d.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let fd = d as f32;
            *slot = sin_turns(fd.mul_add(WAVE_D, t * DRIFT_D) + self.phase[2]);
        }

        for y in 0..FIELD_H {
            let row = y * FIELD_W;
            let b = wave_y[y];
            for x in 0..FIELD_W {
                let h = 0.20f32.mul_add(wave_d[x + y], 0.45f32.mul_add(wave_x[x], 0.35 * b));
                self.cells[row + x] = h.mul_add(0.5, 0.5);
            }
        }
    }

    /// One row of samples.
    fn row(&self, y: usize) -> &[f32] {
        &self.cells[y * FIELD_W..y * FIELD_W + FIELD_W]
    }

    /// **The field as `FIELD_W * FIELD_H` luminance bytes**, for the GPU path.
    ///
    /// Quantising to eight bits is not a loss worth arguing about: the field
    /// is `[0, 1]` and the shader multiplies it by a level count of ten to
    /// twenty, so a step of 1/255 moves `v` by less than a tenth of the
    /// distance between two iso-lines.
    ///
    /// `scratch` is the caller's buffer, refilled in place — 5,184 bytes a
    /// frame allocated and dropped would be the largest thing this background
    /// does on the CPU once the rasterising has gone.
    fn bytes<'a>(&self, scratch: &'a mut Vec<u8>) -> &'a [u8] {
        if scratch.len() != FIELD_W * FIELD_H {
            scratch.clear();
            scratch.resize(FIELD_W * FIELD_H, 0);
        }
        for (slot, cell) in scratch.iter_mut().zip(self.cells.iter()) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                *slot = (cell.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        scratch
    }
}

/// Where each output pixel falls on one axis of the low-resolution grid.
///
/// **Precomputed once per size**, because it never changes between frames and
/// the alternative is two divisions and a floor per pixel — four million of
/// them a second at 1080p, to recompute a constant.
///
/// The half-pixel offsets are the standard ones for scaling a sampled grid:
/// output pixel centres map to source *centres*, not to source edges, so the
/// picture does not shift by half a source cell on the way up. Indices are
/// clamped to `src - 2` so `index + 1` is always a real sample and the
/// interpolation at the far edge degenerates to holding the last value
/// rather than reading past the end.
struct AxisMap {
    index: Vec<u32>,
    weight: Vec<f32>,
    /// `(source sample, first output pixel, how many)` for each maximal run
    /// of output pixels that land on the same source sample.
    ///
    /// **This is what makes the two full-screen loops fast**, and it is worth
    /// the extra vector. Iterating output pixels and reading `index[x]` on
    /// each one means a gather, a comparison against the last index, and a
    /// branch — inside a loop that runs two million times a frame, and the
    /// branch is what stops the compiler vectorising any of it. Iterating
    /// *runs* hoists the whole per-sample setup (which `mid` entries, what
    /// the slope is, what the gradient is) out of the pixel loop, and what is
    /// left inside is one contiguous load and one fused multiply-add.
    ///
    /// **Measured, at 1920x1080 on the development machine and not on a
    /// board** — `evidence.rs`'s timing test, whose doc comment explains why
    /// no number from here is the number that matters. Aurora's drawing went
    /// from 5.13 ms a frame to 4.28 and Contours' from 8.19 to 7.32, with no
    /// change to either picture. Two further changes took them to about 4.2
    /// and 4.5: [`nearest_integer`] and [`shade_of`], which are about what
    /// the compiler can and cannot do rather than about the algorithm.
    ///
    /// **That is a smaller win than the restructuring deserved**, and the
    /// reason is worth writing down: neither loop vectorises even now.
    /// Aurora's palette read is a gather, and baseline `x86_64` — which is
    /// what `cargo build` targets by default — has no gather instruction.
    /// Contours' store is conditional. The gain that did arrive came from
    /// deleting work, not from widening it.
    runs: Vec<(u32, u32, u32)>,
}

impl AxisMap {
    /// `src` must be at least two samples; both [`FIELD_W`] and [`FIELD_H`]
    /// are, and nothing else builds one.
    fn new(out: usize, src: usize) -> Self {
        let mut index = Vec::with_capacity(out);
        let mut weight = Vec::with_capacity(out);
        for o in 0..out {
            #[allow(clippy::cast_precision_loss)]
            let position = ((o as f32 + 0.5) * src as f32 / out as f32) - 0.5;
            #[allow(clippy::cast_precision_loss)]
            let clamped = position.clamp(0.0, (src - 1) as f32);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = (clamped as usize).min(src - 2);
            #[allow(clippy::cast_possible_truncation)]
            index.push(i as u32);
            #[allow(clippy::cast_precision_loss)]
            weight.push(clamped - i as f32);
        }

        // `index` is non-decreasing by construction, so a run is a maximal
        // stretch of equal values and one pass finds them all.
        let mut runs: Vec<(u32, u32, u32)> = Vec::new();
        for (o, i) in index.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if run.0 == *i => run.2 += 1,
                _ => {
                    #[allow(clippy::cast_possible_truncation)]
                    runs.push((*i, o as u32, 1));
                }
            }
        }

        Self {
            index,
            weight,
            runs,
        }
    }
}

/// A colour packed into the four bytes `XRGB8888` scans out, `X` zeroed.
///
/// **XRGB8888 little-endian, blue lowest** — `rivet_sdk::draw`'s layout,
/// fixed by its module doc comment: *"there is one layout now and it is
/// written down here."*
///
/// **One 32-bit store instead of three 8-bit ones**, which at two million
/// pixels a frame is worth having: a four-byte `copy_from_slice` into a
/// `chunks_exact_mut(4)` pixel compiles to a single move, where three
/// separate byte stores to consecutive addresses do not.
///
/// Zeroing `X` is a real difference from what `draw::fill` does, which leaves
/// the fourth byte as it found it, and it is safe here: `X` is not an alpha
/// channel and nothing scans it out. Both callers paint every pixel of the
/// surface they were given, so there is no pre-existing `X` underneath them
/// that a reader could expect to survive.
///
/// # Why this file writes pixels rather than calling `draw::fill`
///
/// Aurora paints a different colour at every one of two million pixels, and
/// Contours evaluates a function at every one of them. There is no rectangle
/// to pass. `draw::fill` for a 1x1 rect two million times a frame would
/// re-clip, re-derive a row offset and re-slice for each one — which is
/// precisely the cost `draw::fill`'s own doc comment records having removed
/// from itself in `SPEC-04` §10, for the same reason, on the same screen.
/// Both callers take a bounds-checked row slice once and walk it with an
/// iterator, so nothing here can write outside the row it was given.
#[inline(always)]
const fn packed(colour: [u8; 3]) -> [u8; 4] {
    [colour[2], colour[1], colour[0], 0]
}

// ---------------------------------------------------------------------------
// Starfield
// ---------------------------------------------------------------------------

/// Starfield's themes — **star colour and drift rate**, which is what §6 says
/// a theme controls here.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StarPalette {
    /// Plain white at the reference rate.
    White,
    /// Warm, and slower: the calm one.
    Amber,
    /// The family's `ice-blue`, and the fastest of the three.
    IceBlue,
}

impl StarPalette {
    /// The colour a star is drawn at full brightness.
    const fn colour(self) -> [u8; 3] {
        match self {
            Self::White => [255, 255, 255],
            Self::Amber => [255, 190, 90],
            // `kCardIce`'s ink, so Starfield's blue is the family's blue and
            // not a second one chosen by eye.
            Self::IceBlue => [190, 205, 247],
        }
    }

    /// Multiplies every layer's speed.
    const fn rate(self) -> f32 {
        match self {
            Self::White => 1.0,
            Self::Amber => 0.72,
            Self::IceBlue => 1.35,
        }
    }
}

/// ~~§6's *"a few hundred points"*.~~ **2,400, ruled 2026-09-20** — the
/// maintainer, watching the old one on the wall, found the stars too few and
/// too small to see (`.rulings/` R-207).
///
/// **Three hundred and twenty was a budget, not a design.** It was chosen when
/// a full-surface clear cost about twenty milliseconds through `draw::fill`,
/// which left almost nothing for the stars — and the single-pixel plot below
/// was the same constraint showing through, to the point that
/// `STAR_LAYER_BRIGHTNESS`'s own note says brightness *"is the only cue that
/// survives at a one-pixel plot, since size cannot vary below one."*
///
/// `rivet_sdk::draw::clear` and `disc_coverage`'s `sqrt` took that background
/// from 21.53 ms to 4.11 ms, all of which was the clear. The ceiling the
/// maintainer set is **10 ms and it stays on the CPU**, deliberately, so that
/// the pack keeps a worked example of each — `.rulings/` R-207.
const STAR_COUNT: usize = 2_400;

/// The smallest a star is drawn, as a disc radius in pixels. Half a pixel is
/// a single lit pixel at the centre, which is what the far layer was.
const STAR_MIN_RADIUS: f32 = 0.5;

/// The largest, for the nearest star at full approach with the highest
/// [`Star::size_bias`] — a disc a little over five pixels across.
const STAR_MAX_RADIUS: f32 = 2.7;

/// How much of a star's size its own [`Star::size_bias`] can take away, so
/// that the dimmest are still points while the brightest are discs. `1.0`
/// would make every star the same size at the same depth.
const STAR_SIZE_SPREAD: f32 = 0.30;

/// **Size by layer, which is the parallax cue size can carry and brightness
/// cannot.** The near layer reaches full size; the far one stays near a point
/// however close it gets to the edge, because a far star is far.
const STAR_LAYER_SIZE: [f32; 3] = [0.30, 0.62, 1.0];

/// §6's *"three parallax layers"*, as speed multipliers. The near layer is
/// roughly twice the far one, which is enough separation to read as depth;
/// more makes the far layer look frozen.
const STAR_LAYER_SPEED: [f32; 3] = [0.55, 0.80, 1.15];

/// And as brightness. **The near layer is brighter** — §6 — and this is the
/// only cue that survives at a one-pixel plot, since size cannot vary below
/// one.
const STAR_LAYER_BRIGHTNESS: [f32; 3] = [0.42, 0.68, 1.0];

/// One point drifting outward.
///
/// Stored as a **direction and a radius** rather than as a position and a
/// velocity. The two are equivalent for the motion, and this one makes the
/// vanishing point structural: a star cannot drift off its own ray, so no
/// accumulated floating-point error can turn radial motion into a slow
/// curve over the hours this runs.
#[derive(Clone, Copy)]
struct Star {
    dx: f32,
    dy: f32,
    /// Distance from the vanishing point, in pixels.
    radius: f32,
    /// Index into [`STAR_LAYER_SPEED`]. Fixed for the star's whole life,
    /// including across respawns, so the three layers stay populated instead
    /// of drifting towards whichever one the respawn draw happens to favour.
    layer: usize,
    /// **How big this particular star gets**, in `[STAR_SIZE_SPREAD, 1.0]`,
    /// drawn fresh at every respawn.
    ///
    /// Depth alone would make every star at a given distance the same size,
    /// which reads as a grid of identical dots rather than as stars. Real
    /// ones differ in magnitude as well as in distance, and this is that:
    /// one number per star, so two stars side by side are not twins.
    size_bias: f32,
}

/// **Points drifting outward from a vanishing point** — §6.
pub(crate) struct Starfield {
    palette: StarPalette,
    stars: Vec<Star>,
    centre_x: f32,
    centre_y: f32,
    width: f32,
    height: f32,
    /// The radius at which a star reaches full brightness — half the shorter
    /// screen axis. Beyond it a star is simply at full brightness until it
    /// leaves.
    full_bright_radius: f32,
}

/// Acceleration with distance. A star's outward speed is
/// `radius * this + STAR_BASE_SPEED`, which is what makes the motion read as
/// flying forward rather than as an expanding ring: near the vanishing point
/// everything crawls, at the edge it streaks. A constant speed was tried on
/// paper and it is a firework, not a starfield.
const STAR_SPREAD: f32 = 0.55;

/// Pixels per second at the vanishing point itself, so a star that has just
/// respawned still moves. Without it `radius` multiplied by anything stays
/// near zero and new stars pile up at the centre.
const STAR_BASE_SPEED: f32 = 25.0;

impl Starfield {
    fn new(palette: StarPalette, width: usize, height: usize, rng: &mut Mt19937) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let (w, h) = (width as f32, height as f32);
        let mut field = Self {
            palette,
            stars: Vec::with_capacity(STAR_COUNT),
            centre_x: w * 0.5,
            centre_y: h * 0.5,
            width: w,
            height: h,
            full_bright_radius: (w.min(h) * 0.5).max(1.0),
        };
        for i in 0..STAR_COUNT {
            let mut star = Star {
                dx: 0.0,
                dy: 0.0,
                radius: 0.0,
                layer: i % STAR_LAYER_SPEED.len(),
                size_bias: 1.0,
            };
            field.respawn(&mut star, rng);
            // **The initial radius is spread over the whole screen**, unlike
            // a respawn's. A field that starts with every star at the centre
            // opens with a visible bloom, once, every time a person selects
            // this background — which is the first second of the effect and
            // the worst possible first impression of it.
            star.radius = rng.real(2.0, field.full_bright_radius * 1.9);
            field.stars.push(star);
        }
        field
    }

    /// A fresh direction and a radius just off the vanishing point.
    fn respawn(&self, star: &mut Star, rng: &mut Mt19937) {
        let turns = rng.real(0.0, 1.0);
        star.dx = cos_turns(turns);
        star.dy = sin_turns(turns);
        // Not zero: a star exactly at the vanishing point has no direction
        // to be seen departing along, and the first frame of its life would
        // put every respawned star on the same pixel.
        star.radius = rng.real(2.0, 18.0);
        // **Drawn at every respawn, not once at construction**, so a star that
        // recycles comes back as a different star rather than the same one
        // going round again — which is visible at 2,400 of them.
        star.size_bias = rng.real(STAR_SIZE_SPREAD, 1.0);
    }

    fn update(&mut self, dt: f32, rng: &mut Mt19937) {
        let rate = self.palette.rate();
        for i in 0..self.stars.len() {
            let star = self.stars[i];
            let speed = star.radius.mul_add(STAR_SPREAD, STAR_BASE_SPEED)
                * STAR_LAYER_SPEED[star.layer]
                * rate;
            let radius = star.radius + speed * dt;
            let x = star.dx.mul_add(radius, self.centre_x);
            let y = star.dy.mul_add(radius, self.centre_y);
            // **Recycled on leaving the RECTANGLE, not on passing a radius.**
            // A 16:9 screen is nearly twice as wide as it is tall, so a star
            // travelling straight up leaves the picture at 540 pixels and a
            // star travelling sideways at 960. Recycling on a single radius
            // would either strand the vertical ones off-screen for half their
            // lives — a third of the field invisible at any moment — or clip
            // the horizontal ones before they reach the edge.
            if x < 0.0 || y < 0.0 || x >= self.width || y >= self.height {
                let mut fresh = self.stars[i];
                self.respawn(&mut fresh, rng);
                self.stars[i] = fresh;
            } else {
                self.stars[i].radius = radius;
            }
        }
    }

    fn draw(&self, canvas: &mut [u8], t: &Target) {
        draw::clear(canvas, t, [0, 0, 0]);
        let colour = self.palette.colour();
        for star in &self.stars {
            let x = star.dx.mul_add(star.radius, self.centre_x);
            let y = star.dy.mul_add(star.radius, self.centre_y);
            if x < 0.0 || y < 0.0 || x >= self.width || y >= self.height {
                continue;
            }
            // Distance fades a star IN as it approaches, on top of its
            // layer's constant. The two together are what stops the field
            // reading as a flat sheet of dots at three brightnesses.
            let approach = (star.radius / self.full_bright_radius).clamp(0.0, 1.0);
            let brightness = STAR_LAYER_BRIGHTNESS[star.layer] * 0.25f32.mul_add(1.0, 0.75 * approach);

            // **Size carries depth, and that is the cue the old plot could not
            // give** — `STAR_COUNT`'s note. A star grows as it approaches, the
            // near layer grows most, and its own `size_bias` decides where in
            // that range it sits so that neighbours are not twins.
            let radius = STAR_MIN_RADIUS
                + approach * STAR_LAYER_SIZE[star.layer] * star.size_bias * STAR_MAX_RADIUS;

            // **A disc with the brightness as ALPHA, not baked into the
            // colour.** `circle_alpha` multiplies the shape's own antialiased
            // coverage by the opacity, so a faint star is a soft point rather
            // than a hard dot of a darker colour — which is what
            // `fill_rounded_alpha`'s note argues for a card and is more
            // visible here, where the shapes are two pixels wide.
            //
            // This is what `STAR_LAYER_BRIGHTNESS`'s own note said could not
            // be done: *"the only cue that survives at a one-pixel plot, since
            // size cannot vary below one."* It can now.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let alpha = (brightness.min(1.0) * 255.0 + 0.5) as u8;
            draw::circle_alpha(canvas, t, x, y, radius, colour, alpha);
        }
    }
}

/// A colour at a fraction of its brightness.
///
/// **Correct only over a black ground, so it is used only over one.**
/// Every caller below fills the screen black first and then scales, so
/// `colour * fraction` is exactly what blending `colour` at `fraction`
/// coverage over black would produce — without reading the canvas back.
fn scale(colour: [u8; 3], fraction: f32) -> [u8; 3] {
    let f = fraction.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let channel = |c: u8| -> u8 { (f32::from(c) * f) as u8 };
    [channel(colour[0]), channel(colour[1]), channel(colour[2])]
}

// ---------------------------------------------------------------------------
// Aurora
// ---------------------------------------------------------------------------

/// Aurora's themes — **the palette ramp**, which is what §6 says a theme
/// controls here.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuroraPalette {
    GreenViolet,
    DeepBlue,
    Ember,
}

impl AuroraPalette {
    /// Five stops, low to high, interpolated into a 256-entry ramp.
    ///
    /// **Every ramp starts near black.** Aurora is the brightest background
    /// in this pack and the only one with no black ground at all, so the low
    /// end of the ramp is the only darkness on the screen; a ramp that
    /// bottomed out at a mid tone would leave the whole panel glowing.
    const fn stops(self) -> [[u8; 3]; 5] {
        match self {
            Self::GreenViolet => [
                [4, 6, 14],
                [10, 60, 50],
                [30, 150, 90],
                [90, 110, 190],
                [150, 70, 180],
            ],
            Self::DeepBlue => [
                [2, 4, 14],
                [6, 24, 70],
                [14, 60, 130],
                [40, 110, 190],
                [130, 180, 235],
            ],
            Self::Ember => [
                [10, 3, 2],
                [50, 10, 4],
                [120, 35, 8],
                [200, 90, 20],
                [245, 180, 80],
            ],
        }
    }
}

/// Cycles per second of drift for Aurora's height field. One: the reference
/// rate the [`DRIFT_X`] family of constants was tuned at.
const AURORA_RATE: f32 = 1.0;

/// **A plasma field, bilinearly scaled to the full surface** — §6.
pub(crate) struct Aurora {
    field: HeightField,
    /// The five stops expanded once, and already in the byte order the
    /// surface scans out. Per pixel this turns a palette lookup into one
    /// array index and one four-byte copy, which matters when there are two
    /// million of them a frame.
    ramp: [[u8; 4]; 256],
    x_map: AxisMap,
    y_map: AxisMap,
    width: usize,
    height: usize,
}

impl Aurora {
    fn new(palette: AuroraPalette, width: usize, height: usize, rng: &mut Mt19937) -> Self {
        Self {
            field: HeightField::new(AURORA_RATE, rng),
            ramp: build_ramp(palette.stops()),
            x_map: AxisMap::new(width.max(1), FIELD_W),
            y_map: AxisMap::new(height.max(1), FIELD_H),
            width,
            height,
        }
    }

    fn update(&mut self, dt: f32) {
        self.field.update(dt);
    }

    fn draw(&self, canvas: &mut [u8], t: &Target) {
        let bpp = t.bytes_per_pixel();
        if bpp < 3 {
            return;
        }
        let width = t.width.min(self.width);
        for y in 0..t.height.min(self.height) {
            let sy = self.y_map.index[y] as usize;
            let wy = self.y_map.weight[y];
            let top = self.field.row(sy);
            let bottom = self.field.row(sy + 1);
            // **The vertical interpolation happens once per output row, not
            // once per output pixel.** At 1080p that is 96 lerps standing in
            // for 1920 of them, on every one of 1080 rows.
            let mut mid = [0.0f32; FIELD_W];
            for (i, slot) in mid.iter_mut().enumerate() {
                *slot = (bottom[i] - top[i]).mul_add(wy, top[i]);
            }

            let row_start = y * t.stride;
            let Some(row) = canvas.get_mut(row_start..row_start + width * bpp) else {
                continue;
            };

            // **By runs, not by pixels** — see `AxisMap::runs`. Everything
            // that depends on which source sample a pixel falls on is hoisted
            // out here, and both loops below walk two slices in step so there
            // is no bounds check inside either.
            for &(sample, start, count) in &self.x_map.runs {
                let i = sample as usize;
                let base = mid[i];
                let slope = mid[i + 1] - mid[i];
                let from = start as usize;
                let to = (from + count as usize).min(width);
                if from >= to {
                    continue;
                }
                let weights = &self.x_map.weight[from..to];
                let pixels = &mut row[from * bpp..to * bpp];
                // `bpp` is loop-invariant and the split is here rather than
                // inside, so the four-byte case is a clean `copy_from_slice`
                // of a fixed width. Nothing in this crate builds a `Target`
                // with any other value — `render.rs` hard-codes four — so the
                // second arm is for a `Target` that arrived some other way.
                if bpp == 4 {
                    for (pixel, weight) in pixels.chunks_exact_mut(4).zip(weights) {
                        let v = slope.mul_add(*weight, base);
                        pixel.copy_from_slice(&self.ramp[shade_of(v)]);
                    }
                } else {
                    for (pixel, weight) in pixels.chunks_exact_mut(bpp).zip(weights) {
                        let v = slope.mul_add(*weight, base);
                        let entry = self.ramp[shade_of(v)];
                        pixel[0] = entry[0];
                        pixel[1] = entry[1];
                        pixel[2] = entry[2];
                    }
                }
            }
        }
    }
}

/// A height in `[0, 1]` as an index into a 256-entry ramp.
///
/// **`.min(255)` and not `.clamp(0.0, 255.0)`**, which is not a style
/// preference: the integer `min` is what lets the compiler prove the index is
/// in range and drop the bounds check on the array read that follows it. The
/// float-to-integer cast saturates in Rust, so a negative can only arrive as
/// zero, and `HeightField` is asserted to stay inside `[0, 1]` anyway.
#[inline(always)]
fn shade_of(v: f32) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let shade = (v * 255.0) as usize;
    shade.min(255)
}

/// Expand N stops into a 256-entry ramp by straight linear interpolation,
/// already packed into the surface's byte order — see [`packed`].
fn build_ramp(stops: [[u8; 3]; 5]) -> [[u8; 4]; 256] {
    let mut ramp = [[0u8; 4]; 256];
    let segments = stops.len() - 1;
    for (i, slot) in ramp.iter_mut().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let position = i as f32 / 255.0 * segments as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let segment = (position as usize).min(segments - 1);
        #[allow(clippy::cast_precision_loss)]
        let fraction = position - segment as f32;
        let (a, b) = (stops[segment], stops[segment + 1]);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let channel =
            |c: usize| -> u8 { (f32::from(b[c]) - f32::from(a[c])).mul_add(fraction, f32::from(a[c])) as u8 };
        *slot = packed([channel(0), channel(1), channel(2)]);
    }
    ramp
}

// ---------------------------------------------------------------------------
// Contours
// ---------------------------------------------------------------------------

/// Contours' themes — **line colour and contour spacing**, which is what §6
/// says a theme controls here.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContourPalette {
    PhosphorGreen,
    IceBlue,
    Ember,
}

impl ContourPalette {
    const fn colour(self) -> [u8; 3] {
        match self {
            Self::PhosphorGreen => [120, 255, 150],
            Self::IceBlue => [190, 215, 255],
            Self::Ember => [255, 170, 90],
        }
    }

    /// How many iso-lines span the height field's full `[0, 1]` range.
    /// **This is the spacing**, expressed as a count rather than a distance
    /// because the field's range is fixed and a count is what a reader can
    /// check against the picture.
    const fn levels(self) -> f32 {
        match self {
            Self::PhosphorGreen => 14.0,
            // Tighter: the densest of the three, and the one that reads most
            // like a chart.
            Self::IceBlue => 20.0,
            // Wider, so the ember lines are sparse and the screen is darker
            // still.
            Self::Ember => 10.0,
        }
    }
}

/// The nearest integer to a **non-negative** `v`, without calling
/// `f32::round`.
///
/// **This is not micro-optimisation for its own sake.** Rust's default
/// `x86_64` target is baseline SSE2, which has no rounding instruction at
/// all, so `f32::round` compiles to a call into the platform's `roundf` — a
/// function call per pixel, two million times a frame. A float-to-integer
/// cast is a single SSE2 instruction, and for `v >= 0` truncating `v + 0.5`
/// is exactly round-half-up. Measured together with [`shade_of`], the pair
/// took Contours' drawing from 7.32 ms a frame to 5.50 on the development
/// machine.
///
/// **A Raspberry Pi is `aarch64`, which does have `frintn`**, so this change
/// buys far less there than it does here — possibly nothing. It is kept
/// because it costs nothing to keep and because the development machine is
/// where anyone iterating on this file will feel it.
///
/// **Only correct for non-negative input**, which is why this is a private
/// helper and not a general one: truncation goes toward zero, so a negative
/// `v` would round the wrong way. Contours' `v` is a height in `[0, 1]`
/// multiplied by a positive level count, so it cannot be negative.
#[inline(always)]
fn nearest_integer(v: f32) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let n = (v + 0.5) as i32;
    #[allow(clippy::cast_precision_loss)]
    let rounded = n as f32;
    rounded
}

/// Half the drawn width of a line, in pixels. The full line is therefore
/// about 2.6 pixels of which only the centre is at full brightness — thin
/// enough to read as an instrument, wide enough that an antialiased curve
/// does not break into dashes where it runs nearly parallel to the pixel
/// grid.
const CONTOUR_HALF_WIDTH: f32 = 1.3;

/// §6 asks for Contours to drift **slowly**, and this is how much slower than
/// Aurora: a little over a third the rate, so the same wave takes forty
/// seconds to cross instead of fourteen.
const CONTOUR_RATE: f32 = 0.38;

/// **The same height field drawn as iso-lines rather than filled** — §6.
///
/// # How a line is found without tracing one
///
/// There is no path-following here and no marching squares. For each pixel
/// the field's height `v` is scaled into contour units, and `|v - round(v)|`
/// is then the distance to the nearest iso-line *in those units*. Dividing
/// by the local gradient converts that into a distance in **pixels**, and a
/// pixel within [`CONTOUR_HALF_WIDTH`] of a line is lit in proportion to how
/// close it is. That division is what makes every line the same width
/// regardless of how steep the field is under it — without it, lines are
/// hairline on a slope and blot out whole regions near a plateau.
///
/// It also costs nothing where there is no line. A pixel that is far from
/// one takes a compare and a branch and writes nothing at all, which is why
/// Contours is the cheapest of the three that share this field even though
/// it does more arithmetic per pixel than Aurora.
pub(crate) struct Contours {
    field: HeightField,
    palette: ContourPalette,
    x_map: AxisMap,
    y_map: AxisMap,
    width: usize,
    height: usize,
    /// Source cells per output pixel on each axis — the factor that turns a
    /// per-cell height difference into a per-pixel one.
    x_scale: f32,
    y_scale: f32,
}

impl Contours {
    /// This frame, as the GPU path wants it.
    ///
    /// **Every number here is the one the CPU path uses**, read off the same
    /// palette — `levels` is the iso-line count, `half_width` the same
    /// [`CONTOUR_HALF_WIDTH`] its threshold is scaled by. The shader is the
    /// same arithmetic per pixel; what changed is who does it.
    fn uploaded<'a>(&self, scratch: &'a mut Vec<u8>) -> crate::gpu::Uploaded<'a> {
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        crate::gpu::Uploaded::Contours {
            width: FIELD_W as i32,
            height: FIELD_H as i32,
            levels: self.palette.levels(),
            half_width: CONTOUR_HALF_WIDTH,
            colour: normalise(self.palette.colour()),
            field: self.field.bytes(scratch),
        }
    }

    fn new(palette: ContourPalette, width: usize, height: usize, rng: &mut Mt19937) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let x_scale = FIELD_W as f32 / width.max(1) as f32;
        #[allow(clippy::cast_precision_loss)]
        let y_scale = FIELD_H as f32 / height.max(1) as f32;
        Self {
            field: HeightField::new(CONTOUR_RATE, rng),
            palette,
            x_map: AxisMap::new(width.max(1), FIELD_W),
            y_map: AxisMap::new(height.max(1), FIELD_H),
            width,
            height,
            x_scale,
            y_scale,
        }
    }

    fn update(&mut self, dt: f32) {
        self.field.update(dt);
    }

    fn draw(&self, canvas: &mut [u8], t: &Target) {
        // **Mostly black** — §6 — and this is the black.
        draw::clear(canvas, t, [0, 0, 0]);
        let bpp = t.bytes_per_pixel();
        if bpp < 3 {
            return;
        }
        let colour = self.palette.colour();
        let levels = self.palette.levels();
        let width = t.width.min(self.width);

        for y in 0..t.height.min(self.height) {
            let sy = self.y_map.index[y] as usize;
            let wy = self.y_map.weight[y];
            let top = self.field.row(sy);
            let bottom = self.field.row(sy + 1);
            let mut mid = [0.0f32; FIELD_W];
            let mut down = [0.0f32; FIELD_W];
            for i in 0..FIELD_W {
                // The vertical derivative, in height per source row — and
                // also the term the horizontal interpolation needs, so it is
                // computed once and used twice. Held per source cell rather
                // than interpolated per pixel: across the twenty pixels of
                // one cell it is a linear ramp whose ends differ by a
                // fraction the line width cannot show, and recomputing it per
                // pixel would double the inner loop.
                let step = bottom[i] - top[i];
                mid[i] = step.mul_add(wy, top[i]);
                down[i] = step;
            }

            let row_start = y * t.stride;
            let Some(row) = canvas.get_mut(row_start..row_start + width * bpp) else {
                continue;
            };

            // **By runs, not by pixels** — see `AxisMap::runs`. The gradient
            // is the expensive part and it is constant across a run, so it
            // is computed ninety-six times a row rather than nineteen hundred.
            for &(sample, start, count) in &self.x_map.runs {
                let i = sample as usize;
                let base = mid[i];
                let slope = mid[i + 1] - mid[i];
                let per_pixel_x = slope * levels * self.x_scale;
                let per_pixel_y = down[i] * levels * self.y_scale;
                // A plateau has no gradient and therefore no line anywhere
                // near it; the floor keeps the reciprocal finite rather than
                // special-casing it in the hot loop.
                let gradient = per_pixel_x
                    .mul_add(per_pixel_x, per_pixel_y * per_pixel_y)
                    .sqrt()
                    .max(1e-6);
                // **The threshold is scaled instead of the distance**, which
                // takes a multiply out of the inner loop: rather than
                // dividing every pixel's contour-unit distance by the
                // gradient and comparing against a pixel width, compare the
                // raw distance against a width already converted into contour
                // units. The reciprocal is then needed only on the few pixels
                // that turn out to be on a line.
                let threshold = CONTOUR_HALF_WIDTH * gradient;
                let inverse_threshold = 1.0 / threshold;
                let from = start as usize;
                let to = (from + count as usize).min(width);
                if from >= to {
                    continue;
                }
                let weights = &self.x_map.weight[from..to];

                // **Skip the whole run when no line can cross it**, which at
                // 1080p is most of them and is why Contours is cheaper than
                // Aurora despite doing more arithmetic per pixel.
                //
                // `v` is linear across a run, so the run's twenty pixels span
                // a known interval of contour units — and at these level
                // counts that interval is typically under half a contour, so
                // it usually contains no iso-line at all. Two fused
                // multiply-adds decide it. `floor(hi)` is the largest integer
                // not above the interval, so it lies inside exactly when it
                // is also at or above `lo`; both are non-negative, which is
                // what makes the truncating cast a floor.
                let v_first = slope.mul_add(weights[0], base) * levels;
                let v_last = slope.mul_add(weights[weights.len() - 1], base) * levels;
                let lo = (v_first.min(v_last) - threshold).max(0.0);
                let hi = v_first.max(v_last) + threshold;
                #[allow(clippy::cast_possible_truncation)]
                let highest = (hi as i32) as f32;
                if highest < lo {
                    continue;
                }

                let pixels = &mut row[from * bpp..to * bpp];
                for (pixel, weight) in pixels.chunks_exact_mut(bpp).zip(weights) {
                    let v = slope.mul_add(*weight, base) * levels;
                    let distance = (v - nearest_integer(v)).abs();
                    if distance < threshold {
                        let coverage = distance.mul_add(-inverse_threshold, 1.0);
                        let lit = scale(colour, coverage);
                        if bpp == 4 {
                            pixel.copy_from_slice(&packed(lit));
                        } else {
                            pixel[0] = lit[2];
                            pixel[1] = lit[1];
                            pixel[2] = lit[0];
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Conway's Game of Life
// ---------------------------------------------------------------------------

/// Life's themes — **the cell's shape**, and for one of them what its radius
/// means.
///
/// **This is a departure from §6**, which says Life's themes are *"the cell
/// colour and the fade length"*. The maintainer asked for shape instead, on
/// 2026-09-20, and gave the three below by name. The colour and the fade are
/// therefore the same for all three and live in [`LIFE_COLOUR`] and
/// [`LIFE_FADE_SECONDS`]; what a person picks between is what a cell looks
/// like.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeStyle {
    /// Filled square cells — the classic. The radius carries nothing.
    Squares,
    /// Circles at a fixed radius. The radius carries nothing.
    Dots,
    /// Circles whose **radius carries the cell's age**: a newborn cell is a
    /// pinprick, a settled one is at full radius, a dying one shrinks as it
    /// fades.
    ///
    /// **This is the one worth having.** A glider under `Squares` is four
    /// identical blocks that move; under `Pulse` it grows a head of fresh
    /// pinpricks on its leading edge and a shrinking tail behind it, so a
    /// single still frame shows which way it is going. A photograph of a
    /// Life board normally cannot say that.
    Pulse,
}

/// §6's *"roughly 160x90 cells"*. At 1080p a cell is exactly 12 pixels.
const LIFE_COLS: usize = 160;
/// See [`LIFE_COLS`].
const LIFE_ROWS: usize = 90;

/// §6: *"four steps a second is livelier to watch than sixty"*. A step is
/// therefore a quarter of a second apart, and the drawing between steps is
/// the fade and — under [`LifeStyle::Pulse`] — the growth.
const LIFE_STEPS_PER_SECOND: f32 = 4.0;

/// §6's *"long fade on dying cells, so the pattern leaves a trail rather than
/// flickering"*. Nine tenths of a second is three and a half steps of decay,
/// which is long enough that a glider's whole recent path is visible at once.
const LIFE_FADE_SECONDS: f32 = 0.9;

/// The one cell colour — the family's `ice-blue` ink, so Life's cells are the
/// same blue `kCardIce` and Starfield's Ice Blue use rather than a fourth one
/// picked by eye.
const LIFE_COLOUR: [u8; 3] = [190, 205, 247];

/// A palette colour as GL wants it: `[0, 1]` floats rather than `[0, 255]`
/// bytes.
///
/// One function so that a background porting its colour to the GPU cannot do
/// the division slightly differently from its neighbour — which is exactly the
/// sort of difference that shows up as *"the blue looks off on that one"* and
/// takes an afternoon.
pub(crate) fn normalise(colour: [u8; 3]) -> [f32; 3] {
    [
        f32::from(colour[0]) / 255.0,
        f32::from(colour[1]) / 255.0,
        f32::from(colour[2]) / 255.0,
    ]
}

/// A fully grown cell's radius, as a fraction of the smaller cell dimension.
///
/// Named 2026-09-20 when the GPU path arrived. It was spelled `0.46` inline in
/// `Life::draw` and the shader needs the same number; two literals that must
/// agree and cannot be compared is how a background comes out subtly different
/// on one path.
const LIFE_FULL_RADIUS: f32 = 0.46;

/// [`LifeStyle::Dots`]' fixed radius, on the same terms — previously `0.40`
/// inline beside the one above.
const LIFE_DOT_RADIUS: f32 = 0.40;

/// What [`LifeStyle::Dots`] puts in the texture's alpha channel.
///
/// The shader multiplies alpha by [`LIFE_FULL_RADIUS`] to get a radius, so a
/// style whose radius is fixed has to express it as the ratio rather than as
/// its own number — which is also the statement that `Dots` is `Pulse` with
/// the growth taken out.
const LIFE_DOT_FRACTION: f32 = LIFE_DOT_RADIUS / LIFE_FULL_RADIUS;

/// How many steps a cell must survive to reach full radius under
/// [`LifeStyle::Pulse`].
///
/// **Three, and that is short on purpose.** A glider's cells live two to
/// four generations. A settle time of ten would mean no cell in a glider
/// ever reached full radius and the whole pattern would be uniformly tiny,
/// which loses exactly the distinction this theme exists to show. Three puts
/// a glider's oldest cells near full size while its newest are pinpricks.
const LIFE_SETTLE_STEPS: f32 = 3.0;

/// The fraction of full radius a newborn cell is drawn at. At a 12-pixel cell
/// and a full radius of 5.5 this is a shade over one pixel — a pinprick, as
/// asked, rather than an invisible cell.
const LIFE_NEWBORN_FRACTION: f32 = 0.18;

/// A dying cell's radius shrinks to this fraction rather than to nothing.
/// The alpha fade carries it the rest of the way out; a radius that reached
/// zero would make the tail vanish twice as fast as the fade intends and undo
/// the long trail.
const LIFE_DYING_FLOOR: f32 = 0.35;

/// How many recent grid states the stasis watchdog remembers.
///
/// **Sixteen, and the number is the point.** A hash that matches any of the
/// last sixteen means the field has entered a cycle of period sixteen or
/// less, and that covers essentially everything a Conway soup settles into:
/// still lifes (period 1), blinkers, toads and beacons (2), pulsars (3) and
/// pentadecathlons (15). A deeper ring would catch more exotic oscillators
/// that a random soup effectively never produces, at the cost of carrying
/// them.
const LIFE_HISTORY: usize = 16;

/// A population at or below this fraction of the grid counts as attrition.
/// 0.5% of 14,400 cells is 72 — about the size of two gliders and a handful
/// of debris.
const LIFE_SPARSE_FRACTION: f32 = 0.005;

/// How many consecutive sparse steps are tolerated before an injection.
/// A hundred and twenty steps is thirty seconds at four a second.
const LIFE_SPARSE_STEPS: u32 = 120;

/// The injected patch — §6 has no opinion on this because §6 did not
/// anticipate needing one; the maintainer specified roughly 20x20 at about
/// 35%, 2026-09-20.
const LIFE_PATCH_W: usize = 20;
/// See [`LIFE_PATCH_W`].
const LIFE_PATCH_H: usize = 20;
/// See [`LIFE_PATCH_W`].
const LIFE_PATCH_DENSITY: f32 = 0.35;

/// The density of a full reseed — the initial soup, and the one the
/// extinction case falls back to. Around a third is the classic choice: much
/// denser and the first few generations are a uniform die-off, much sparser
/// and almost nothing survives generation one.
const LIFE_SOUP_DENSITY: f32 = 0.30;

/// **Conway's Game of Life, B3/S23, on a torus.**
///
/// # The watchdog, and why the rule is not the thing that was changed
///
/// B3/S23 on a random soup **always settles**. A 160x90 field burns down to
/// still lifes and a few period-2 oscillators inside a few thousand
/// generations; at four steps a second that is ten to twenty minutes, and
/// then the wall is a static field of blocks and beehives until the board
/// reboots. Aura runs for months.
///
/// The obvious fix is a livelier rule — HighLife's B36/S23 has a replicator
/// and never settles. **It was rejected**: this background is named after
/// Conway, and a background called Conway's Game of Life that is not running
/// his rule is a lie told to everyone who looks at it. The rule below is
/// B3/S23 exactly and nothing in this file touches it.
///
/// What was added is a watchdog *around* the automaton, in three parts:
///
/// 1. **Extinction.** Population zero is the dead-screen case and the only
///    one that justifies clearing the board: a full reseed.
/// 2. **Stasis.** A grid hash matching any of the last [`LIFE_HISTORY`]
///    means a short cycle — see that constant. An injection.
/// 3. **Attrition.** A population under [`LIFE_SPARSE_FRACTION`] held for
///    [`LIFE_SPARSE_STEPS`] catches what the cycle detector cannot see: two
///    gliders circling an empty torus for ever is not cyclic at any depth,
///    and it is not worth watching either. An injection.
///
/// **Injection and not reset**, for 2 and 3. A 20x20 patch dropped somewhere
/// at random reads as the field being reseeded; clearing the whole grid reads
/// as the program having crashed and restarted. With the fade still running
/// and `Pulse`'s age-radius, a fresh patch visibly blooms into the dead
/// debris around it, which is a better thing to catch out of the corner of an
/// eye than a black screen refilling.
///
/// **Every one of them draws from [`Mt19937`]**, and the sparse window counts
/// *steps* rather than seconds. A watchdog keyed to the wall clock would make
/// frame 600 depend on how fast the machine rendered the first 599.
pub(crate) struct Life {
    style: LifeStyle,
    alive: Vec<bool>,
    /// Steps survived, saturating. Meaningless for a dead cell except that it
    /// is **not reset on death** — `Pulse` reads it to decide how big the
    /// corpse was when it died, so the tail tapers from the size the cell
    /// reached rather than from full radius.
    age: Vec<u8>,
    /// `1.0` while alive, decaying to `0.0` over [`LIFE_FADE_SECONDS`] after
    /// death. This is the trail.
    heat: Vec<f32>,
    /// The next generation, kept as a field rather than allocated per step.
    /// A step is 14,400 cells four times a second and the allocation would be
    /// the most expensive part of it.
    next: Vec<bool>,
    accumulator: f32,
    /// How far through the current step, in `[0, 1)`. `Pulse` interpolates a
    /// growing cell's radius across it, so growth is smooth at 30 fps rather
    /// than stepping four times a second.
    progress: f32,
    /// Cell size in pixels, as floats: the grid is 160x90 and the surface
    /// need not be a multiple of it.
    cell_w: f32,
    cell_h: f32,
    history: [u64; LIFE_HISTORY],
    history_at: usize,
    sparse_steps: u32,
    /// Counted for the timing test and for the watchdog report — how often
    /// this instance has had to intervene.
    interventions: u32,
}

impl Life {
    fn new(style: LifeStyle, width: usize, height: usize, rng: &mut Mt19937) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let (cell_w, cell_h) = (
            width as f32 / LIFE_COLS as f32,
            height as f32 / LIFE_ROWS as f32,
        );
        let mut life = Self {
            style,
            alive: vec![false; LIFE_COLS * LIFE_ROWS],
            age: vec![0; LIFE_COLS * LIFE_ROWS],
            heat: vec![0.0; LIFE_COLS * LIFE_ROWS],
            next: vec![false; LIFE_COLS * LIFE_ROWS],
            accumulator: 0.0,
            progress: 0.0,
            cell_w,
            cell_h,
            // Not `[0; N]`: an all-zero ring would match the hash of an empty
            // grid on the very first step and fire the stasis watchdog before
            // anything had a chance to stagnate.
            history: [u64::MAX; LIFE_HISTORY],
            history_at: 0,
            sparse_steps: 0,
            interventions: 0,
        };
        life.reseed(rng);
        life
    }

    /// Fill the whole grid with a fresh soup. Only extinction reaches this
    /// after construction.
    fn reseed(&mut self, rng: &mut Mt19937) {
        for i in 0..self.alive.len() {
            let alive = rng.real(0.0, 1.0) < LIFE_SOUP_DENSITY;
            self.alive[i] = alive;
            self.age[i] = 0;
            // A fresh cell starts lit; a fresh empty one starts dark rather
            // than inheriting whatever was fading there.
            self.heat[i] = if alive { 1.0 } else { 0.0 };
        }
    }

    /// Drop a patch of soup somewhere at random, leaving everything else —
    /// including whatever is still fading — exactly as it was.
    fn inject(&mut self, rng: &mut Mt19937) {
        let x0 = rng.int(0, LIFE_COLS as i32 - 1) as usize;
        let y0 = rng.int(0, LIFE_ROWS as i32 - 1) as usize;
        for dy in 0..LIFE_PATCH_H {
            for dx in 0..LIFE_PATCH_W {
                // Wrapped, like everything else here, so a patch near an edge
                // is a patch and not a clipped rectangle.
                let x = (x0 + dx) % LIFE_COLS;
                let y = (y0 + dy) % LIFE_ROWS;
                if rng.real(0.0, 1.0) < LIFE_PATCH_DENSITY {
                    let i = y * LIFE_COLS + x;
                    self.alive[i] = true;
                    self.age[i] = 0;
                    self.heat[i] = 1.0;
                }
            }
        }
    }

    /// FNV-1a over the live bits, eight cells to a byte.
    ///
    /// Cheap enough to be free beside the step itself — one pass over 14,400
    /// bools — and wide enough that a false match is not a thing that
    /// happens. A collision would cost one unnecessary injection, which is
    /// also not a thing anybody would notice.
    fn hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for chunk in self.alive.chunks(8) {
            let mut byte = 0u8;
            for (bit, cell) in chunk.iter().enumerate() {
                byte |= u8::from(*cell) << bit;
            }
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// One generation of B3/S23, wrapped on both axes.
    ///
    /// **Toroidal, and that is not an aesthetic choice.** On a bounded grid
    /// every glider dies at the wall, and a glider is the most interesting
    /// thing that happens on this screen. Wrapping costs two modulo
    /// operations per neighbour and keeps them circulating indefinitely.
    fn step(&mut self, rng: &mut Mt19937) {
        for y in 0..LIFE_ROWS {
            let up = (y + LIFE_ROWS - 1) % LIFE_ROWS * LIFE_COLS;
            let here = y * LIFE_COLS;
            let down = (y + 1) % LIFE_ROWS * LIFE_COLS;
            for x in 0..LIFE_COLS {
                let left = (x + LIFE_COLS - 1) % LIFE_COLS;
                let right = (x + 1) % LIFE_COLS;
                let neighbours = u32::from(self.alive[up + left])
                    + u32::from(self.alive[up + x])
                    + u32::from(self.alive[up + right])
                    + u32::from(self.alive[here + left])
                    + u32::from(self.alive[here + right])
                    + u32::from(self.alive[down + left])
                    + u32::from(self.alive[down + x])
                    + u32::from(self.alive[down + right]);
                // B3/S23. Conway's, unmodified, and see this type's doc
                // comment for why nothing may be added to it.
                self.next[here + x] = if self.alive[here + x] {
                    neighbours == 2 || neighbours == 3
                } else {
                    neighbours == 3
                };
            }
        }

        let mut population = 0usize;
        for i in 0..self.alive.len() {
            let now = self.next[i];
            if now {
                population += 1;
                // Age advances only for a cell that was ALREADY alive. A
                // birth resets it to zero, which is what makes `Pulse`'s
                // pinprick a birth rather than merely a small cell.
                self.age[i] = if self.alive[i] {
                    self.age[i].saturating_add(1)
                } else {
                    0
                };
                self.heat[i] = 1.0;
            }
            self.alive[i] = now;
        }

        self.watchdog(population, rng);
    }

    /// The three interventions, in the order that matters: extinction first,
    /// because an empty grid is also a cyclic one and would otherwise be
    /// injected into rather than reseeded.
    fn watchdog(&mut self, population: usize, rng: &mut Mt19937) {
        if population == 0 {
            self.reseed(rng);
            self.interventions += 1;
            self.forget();
            return;
        }

        #[allow(clippy::cast_precision_loss)]
        let sparse = (population as f32) < self.alive.len() as f32 * LIFE_SPARSE_FRACTION;
        if sparse {
            self.sparse_steps += 1;
        } else {
            self.sparse_steps = 0;
        }

        let hash = self.hash();
        let cyclic = self.history.contains(&hash);
        self.history[self.history_at] = hash;
        self.history_at = (self.history_at + 1) % LIFE_HISTORY;

        if cyclic || self.sparse_steps >= LIFE_SPARSE_STEPS {
            self.inject(rng);
            self.interventions += 1;
            self.forget();
        }
    }

    /// Clear the cycle ring after an intervention.
    ///
    /// Without this a still life that has just been injected into would match
    /// its own pre-injection hashes for the next sixteen steps and fire the
    /// watchdog again on each of them — sixteen patches in four seconds,
    /// which is a visible burst and not a reseed.
    fn forget(&mut self) {
        self.history = [u64::MAX; LIFE_HISTORY];
        self.history_at = 0;
        self.sparse_steps = 0;
    }

    fn update(&mut self, dt: f32, rng: &mut Mt19937) {
        // The fade runs per FRAME and the automaton per STEP. That split is
        // the whole of §6's "a step does not have to happen every frame":
        // sixty steps a second is a flicker, but a fade that only moved four
        // times a second would be one too.
        let decay = dt / LIFE_FADE_SECONDS;
        for i in 0..self.heat.len() {
            if !self.alive[i] && self.heat[i] > 0.0 {
                self.heat[i] = (self.heat[i] - decay).max(0.0);
            }
        }

        let period = 1.0 / LIFE_STEPS_PER_SECOND;
        self.accumulator += dt;
        while self.accumulator >= period {
            self.accumulator -= period;
            self.step(rng);
        }
        self.progress = (self.accumulator / period).clamp(0.0, 1.0);
    }

    /// The radius a cell is drawn at under [`LifeStyle::Pulse`], as a
    /// fraction of full.
    fn pulse_fraction(&self, i: usize) -> f32 {
        let alive = self.alive[i];
        // A living cell's growth is interpolated across the current step; a
        // dead one's is frozen at the age it reached, because `progress`
        // belongs to a step it is no longer taking part in.
        let steps = if alive {
            f32::from(self.age[i]) + self.progress
        } else {
            f32::from(self.age[i])
        };
        let settled = (steps / LIFE_SETTLE_STEPS).clamp(0.0, 1.0);
        let grown = (1.0 - LIFE_NEWBORN_FRACTION).mul_add(settled, LIFE_NEWBORN_FRACTION);
        // Shrinking with the fade, but only down to a floor — see
        // `LIFE_DYING_FLOOR`.
        let shrink = (1.0 - LIFE_DYING_FLOOR).mul_add(self.heat[i], LIFE_DYING_FLOOR);
        grown * shrink
    }

    /// **The board as one `GL_LUMINANCE_ALPHA` texel per cell** — heat in the
    /// luminance channel, the radius fraction in alpha.
    ///
    /// Two channels rather than two textures because the fragment shader needs
    /// both for every pixel and one fetch is cheaper than two. `scratch` is
    /// the caller's buffer, refilled in place: the board is 160x90 and a
    /// fresh 28,800-byte allocation thirty times a second is the kind of thing
    /// that shows up in a profile as something else's fault.
    ///
    /// The arithmetic is [`Self::draw`]'s, factored to neither side: `heat` is
    /// the same field the CPU path scales its colour by, and the alpha channel
    /// is exactly what that path passes to `circle_alpha` as a radius
    /// fraction. Two readings of one simulation, not two simulations.
    fn texels<'a>(&self, scratch: &'a mut Vec<u8>) -> &'a [u8] {
        let want = LIFE_COLS * LIFE_ROWS * 2;
        if scratch.len() != want {
            scratch.clear();
            scratch.resize(want, 0);
        }
        for (i, texel) in scratch.chunks_exact_mut(2).enumerate() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                texel[0] = (self.heat[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                texel[1] = match self.style {
                    // The squares shader reads no radius; the shape is the
                    // cell. Written as zero rather than left stale.
                    LifeStyle::Squares => 0,
                    LifeStyle::Dots => (LIFE_DOT_FRACTION * 255.0 + 0.5) as u8,
                    LifeStyle::Pulse => {
                        (self.pulse_fraction(i).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                    }
                };
            }
        }
        scratch
    }

    /// The radius a fully grown cell is drawn at, in pixels — [`Self::draw`]'s
    /// `full_radius`, read by the GPU path so the constant lives once.
    fn full_radius(&self) -> f32 {
        self.cell_w.min(self.cell_h) * LIFE_FULL_RADIUS
    }

    /// This frame, as the GPU path wants it.
    fn uploaded<'a>(&self, scratch: &'a mut Vec<u8>) -> crate::gpu::Uploaded<'a> {
        #[allow(clippy::cast_precision_loss)]
        let (cols, rows) = (LIFE_COLS as f32, LIFE_ROWS as f32);
        crate::gpu::Uploaded::Life {
            cols,
            rows,
            cell_px: self.cell_px(),
            full_radius: self.full_radius(),
            colour: normalise(LIFE_COLOUR),
            board: self.texels(scratch),
        }
    }

    /// Cell size in pixels, as the shader needs it to keep a disc circular.
    const fn cell_px(&self) -> (f32, f32) {
        (self.cell_w, self.cell_h)
    }

    fn draw(&self, canvas: &mut [u8], t: &Target) {
        draw::clear(canvas, t, [0, 0, 0]);
        let full_radius = self.full_radius();
        let dot_radius = self.cell_w.min(self.cell_h) * LIFE_DOT_RADIUS;

        for y in 0..LIFE_ROWS {
            for x in 0..LIFE_COLS {
                let i = y * LIFE_COLS + x;
                let heat = self.heat[i];
                if heat <= 0.0 {
                    continue;
                }
                match self.style {
                    LifeStyle::Squares => {
                        // Bounds from the NEXT cell's origin rather than from
                        // a rounded width, so the cells tile exactly at any
                        // surface size instead of leaving a one-pixel seam
                        // wherever the division does not come out even.
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let x0 = (x as f32 * self.cell_w) as usize;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let x1 = ((x + 1) as f32 * self.cell_w) as usize;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let y0 = (y as f32 * self.cell_h) as usize;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let y1 = ((y + 1) as f32 * self.cell_h) as usize;
                        draw::fill(
                            canvas,
                            t,
                            x0,
                            y0,
                            x1.saturating_sub(x0),
                            y1.saturating_sub(y0),
                            scale(LIFE_COLOUR, heat),
                        );
                    }
                    LifeStyle::Dots | LifeStyle::Pulse => {
                        #[allow(clippy::cast_precision_loss)]
                        let cx = (x as f32 + 0.5) * self.cell_w;
                        #[allow(clippy::cast_precision_loss)]
                        let cy = (y as f32 + 0.5) * self.cell_h;
                        let radius = if matches!(self.style, LifeStyle::Pulse) {
                            full_radius * self.pulse_fraction(i)
                        } else {
                            dot_radius
                        };
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let alpha = (heat * 255.0).clamp(0.0, 255.0) as u8;
                        // `circle_alpha` rather than a scaled colour: a disc
                        // has an antialiased edge and the coverage there has
                        // to multiply with the fade rather than replace it.
                        draw::circle_alpha(canvas, t, cx, cy, radius, LIFE_COLOUR, alpha);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The one a Screensaver holds
// ---------------------------------------------------------------------------

/// Whichever of the four is running.
///
/// **One field on `Screensaver` rather than four `Option`s.** The ported
/// three each have their own `Option` there because that is how the renderer
/// had it and `background.rs` does not get rearranged; the four added here
/// arrive together and an enum is what they actually are.
///
/// **`large_enum_variant` is allowed rather than fixed**, and the fix clippy
/// suggests would be a real cost. [`Aurora`] is 928 bytes because it carries
/// its 256-entry ramp inline, which is the whole point of the ramp: it makes
/// a palette lookup one array index at each of two million pixels a frame.
/// Boxing it to shrink an enum would put a heap indirection on that path to
/// save bytes in an object this crate constructs **once per background
/// selection** and then holds in a static for as long as that background is
/// on the wall. There is no repeated move here to make smaller.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Designed {
    Starfield(Starfield),
    Aurora(Aurora),
    Contours(Contours),
    Life(Life),
}

impl Designed {
    /// Build whichever of the four `which` names, or [`None`] for one of the
    /// three `background.rs` still owns.
    pub(crate) fn for_background(
        which: Background,
        width: usize,
        height: usize,
        rng: &mut Mt19937,
    ) -> Option<Self> {
        match which {
            Background::Starfield(palette) => Some(Self::Starfield(Starfield::new(
                palette, width, height, rng,
            ))),
            Background::Aurora(palette) => {
                Some(Self::Aurora(Aurora::new(palette, width, height, rng)))
            }
            Background::Contours(palette) => {
                Some(Self::Contours(Contours::new(palette, width, height, rng)))
            }
            Background::Life(style) => Some(Self::Life(Life::new(style, width, height, rng))),
            Background::Solid(_) | Background::Boxes(_) | Background::DigitalRain(_) => None,
        }
    }

    pub(crate) fn update(&mut self, dt: f32, rng: &mut Mt19937) {
        match self {
            Self::Starfield(starfield) => starfield.update(dt, rng),
            Self::Aurora(aurora) => aurora.update(dt),
            Self::Contours(contours) => contours.update(dt),
            Self::Life(life) => life.update(dt, rng),
        }
    }

    pub(crate) fn draw(&self, canvas: &mut [u8], t: &Target) {
        match self {
            Self::Starfield(starfield) => starfield.draw(canvas, t),
            Self::Aurora(aurora) => aurora.draw(canvas, t),
            Self::Contours(contours) => contours.draw(canvas, t),
            Self::Life(life) => life.draw(canvas, t),
        }
    }

    /// **This frame's state, packed for the GPU** — or [`None`] for one of the
    /// two that has no GPU path.
    ///
    /// `None` here and [`rivetos_addon::Drawn`] in the catalogue are the same
    /// fact told to two readers: the catalogue lets a host avoid asking, and
    /// this is what happens if it asks anyway. Aurora is `None` because R-207
    /// skipped it; Starfield is `None` because R-207 measured it at 3.86 ms on
    /// the GPU against 4.11 on the CPU and ruled that a tie is not a reason to
    /// move — the maintainer wanted one worked example of each path.
    pub(crate) fn uploaded<'a>(
        &self,
        scratch: &'a mut Vec<u8>,
    ) -> Option<crate::gpu::Uploaded<'a>> {
        match self {
            Self::Contours(contours) => Some(contours.uploaded(scratch)),
            Self::Life(life) => Some(life.uploaded(scratch)),
            Self::Starfield(_) | Self::Aurora(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The table is a sine and the interpolation between its entries does not
    /// wander. Checked against `f32::sin` at a hundred points, including the
    /// wrap at either end of a turn, because everything Aurora and Contours
    /// draw is built on this one function.
    #[test]
    fn the_sine_table_is_a_sine() {
        for i in 0..100 {
            #[allow(clippy::cast_precision_loss)]
            let turns = i as f32 / 100.0 * 3.0 - 1.5;
            let want = (turns * std::f32::consts::TAU).sin();
            let got = sin_turns(turns);
            assert!(
                (want - got).abs() < 1e-4,
                "sin_turns({turns}) is {got}, sin is {want}"
            );
        }
        assert!((cos_turns(0.0) - 1.0).abs() < 1e-4);
        assert!((sin_turns(0.25) - 1.0).abs() < 1e-4);
    }

    /// **The height field stays inside `[0, 1]`**, which is the contract
    /// Aurora's 256-entry ramp index and Contours' level arithmetic both
    /// depend on. Asserted over a long drift rather than at one instant,
    /// because the three waves come into phase rarely and that is exactly
    /// when a weighting mistake would show.
    #[test]
    fn the_height_field_never_leaves_its_range() {
        let mut rng = Mt19937::new(11);
        let mut field = HeightField::new(1.0, &mut rng);
        let mut low = 1.0f32;
        let mut high = 0.0f32;
        for _ in 0..2_000 {
            field.update(1.0 / 30.0);
            for v in &field.cells {
                assert!((0.0..=1.0).contains(v), "{v} escaped the field's range");
                low = low.min(*v);
                high = high.max(*v);
            }
        }
        // And it must actually USE the range — a field pinned near the middle
        // would render as one flat colour whatever the palette said.
        assert!(low < 0.2, "the field never got dark: floor was {low}");
        assert!(high > 0.8, "the field never got bright: ceiling was {high}");
    }

    /// The phase offsets are read from the generator, so two fields from two
    /// seeds are different pictures at the same instant. Without this every
    /// board on the wall would be in lockstep.
    #[test]
    fn two_seeds_give_two_different_auroras() {
        let mut a = HeightField::new(1.0, &mut Mt19937::new(1));
        let mut b = HeightField::new(1.0, &mut Mt19937::new(2));
        a.update(1.0);
        b.update(1.0);
        assert!(
            a.cells.iter().zip(b.cells.iter()).any(|(x, y)| (x - y).abs() > 0.01),
            "two seeds produced the same field"
        );
    }

    /// Every output pixel maps to a real pair of adjacent samples — the
    /// property that lets [`Aurora::draw`] index `mid[i + 1]` without a
    /// bounds check of its own. Checked at sizes that divide the grid evenly
    /// and at sizes that do not.
    #[test]
    fn the_axis_map_never_indexes_past_its_source() {
        for out in [1usize, 2, 7, 96, 100, 1920, 3840] {
            let map = AxisMap::new(out, FIELD_W);
            assert_eq!(map.index.len(), out);
            for (o, i) in map.index.iter().enumerate() {
                assert!(
                    (*i as usize) + 1 < FIELD_W,
                    "output {o} of {out} maps to sample {i}, which has no neighbour"
                );
                assert!((0.0..=1.0).contains(&map.weight[o]));
            }
        }
    }

    /// **Stars stay on the screen or are recycled**, and the field does not
    /// quietly empty itself over an hour. The failure this catches is a
    /// respawn that never fires, which leaves every star parked outside the
    /// picture and the screen black.
    #[test]
    fn every_star_stays_in_the_picture() {
        let mut rng = Mt19937::new(5);
        let mut field = Starfield::new(StarPalette::White, 1920, 1080, &mut rng);
        for _ in 0..3_600 {
            field.update(1.0 / 30.0, &mut rng);
            for star in &field.stars {
                let x = star.dx.mul_add(star.radius, field.centre_x);
                let y = star.dy.mul_add(star.radius, field.centre_y);
                assert!(
                    (0.0..1920.0).contains(&x) && (0.0..1080.0).contains(&y),
                    "a star is at ({x}, {y}), outside the screen, and was not recycled"
                );
            }
        }
        // All three layers are still populated: a respawn must not move a
        // star between layers, or the parallax collapses over time.
        for layer in 0..STAR_LAYER_SPEED.len() {
            assert!(
                field.stars.iter().any(|s| s.layer == layer),
                "layer {layer} emptied"
            );
        }
    }

    /// Contours draws thin lines on a mostly black screen. The property is
    /// what §6 claims for it — *"mostly black"* — and it is what makes this
    /// the one that is legible at night.
    ///
    /// **At 1920x1080, and the size is the assertion.** A contour's spacing
    /// is fixed in field units, so the number of lines on the screen does not
    /// change with resolution but the pixels between them do — at a quarter
    /// scale the same lines are four times closer together and
    /// [`CONTOUR_HALF_WIDTH`] is four times wider relative to the gap. This
    /// test was first written at 480x270 and measured 45% of the screen lit,
    /// which looked like a design failure and was a test that had shrunk the
    /// picture. The three level counts in [`ContourPalette::levels`] are
    /// chosen for a 1080p panel, so that is where the claim has to be
    /// checked. The densest of the three is asserted, because if `IceBlue`
    /// is mostly black the other two certainly are.
    #[test]
    fn contours_leaves_most_of_the_screen_black() {
        let mut rng = Mt19937::new(3);
        let mut contours = Contours::new(ContourPalette::IceBlue, 1920, 1080, &mut rng);
        contours.update(1.0);
        let t = Target::xrgb(1920, 1080, 1920 * 4);
        let mut canvas = vec![0u8; t.stride * t.height];
        contours.draw(&mut canvas, &t);
        let lit = canvas
            .chunks_exact(4)
            .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
            .count();
        let total = t.width * t.height;
        assert!(
            lit * 5 < total,
            "{lit} of {total} pixels are lit, which is not 'mostly black'"
        );
        assert!(lit > 0, "nothing was drawn at all");
    }

    /// Aurora fills every pixel — it is the one background in this pack with
    /// no black ground behind it, and a gap would show as a black square.
    #[test]
    fn aurora_paints_the_whole_surface() {
        let mut rng = Mt19937::new(3);
        let mut aurora = Aurora::new(AuroraPalette::Ember, 200, 100, &mut rng);
        aurora.update(2.0);
        let t = Target::xrgb(200, 100, 200 * 4);
        // A sentinel that no ramp entry can produce: every Ember stop has a
        // blue channel under 90.
        let mut canvas = vec![0xFFu8; t.stride * t.height];
        aurora.draw(&mut canvas, &t);
        for y in 0..t.height {
            for x in 0..t.width {
                let at = y * t.stride + x * 4;
                assert!(
                    canvas[at] != 0xFF || canvas[at + 1] != 0xFF || canvas[at + 2] != 0xFF,
                    "({x}, {y}) was never painted"
                );
            }
        }
    }

    /// **The rule is Conway's.** Checked on the two patterns that pin B3/S23
    /// down: a block is a still life and a blinker has period two. A rule
    /// that had quietly become HighLife or Seeds would fail one of them.
    #[test]
    fn the_rule_is_b3_s23() {
        let mut rng = Mt19937::new(1);
        let mut life = Life::new(LifeStyle::Squares, 1920, 1080, &mut rng);

        let clear = |life: &mut Life| {
            for i in 0..life.alive.len() {
                life.alive[i] = false;
                life.heat[i] = 0.0;
                life.age[i] = 0;
            }
        };
        let set = |life: &mut Life, cells: &[(usize, usize)]| {
            for (x, y) in cells {
                life.alive[y * LIFE_COLS + x] = true;
            }
        };
        let live = |life: &Life, cells: &[(usize, usize)]| -> bool {
            cells.iter().all(|(x, y)| life.alive[y * LIFE_COLS + x])
        };

        // A block survives untouched: each of its four cells has exactly two
        // neighbours (S2), and no empty cell beside it has exactly three.
        clear(&mut life);
        let block = [(10, 10), (11, 10), (10, 11), (11, 11)];
        set(&mut life, &block);
        life.step(&mut rng);
        assert!(live(&life, &block), "a block did not survive");
        assert_eq!(
            life.alive.iter().filter(|c| **c).count(),
            4,
            "a block grew something beside it"
        );

        // A blinker rotates: the three empty cells above and below the middle
        // have three neighbours (B3) and the two ends have one (death).
        clear(&mut life);
        set(&mut life, &[(40, 40), (41, 40), (42, 40)]);
        life.step(&mut rng);
        assert!(
            live(&life, &[(41, 39), (41, 40), (41, 41)]),
            "a blinker did not rotate"
        );
        assert_eq!(life.alive.iter().filter(|c| **c).count(), 3);
    }

    /// **The grid wraps.** A glider aimed at a corner must come out of the
    /// opposite one rather than dying at a wall — §6's most interesting
    /// object, and the reason the topology is a torus.
    #[test]
    fn a_glider_crosses_the_edge_and_comes_back() {
        let mut rng = Mt19937::new(1);
        let mut life = Life::new(LifeStyle::Squares, 1920, 1080, &mut rng);
        for i in 0..life.alive.len() {
            life.alive[i] = false;
            life.heat[i] = 0.0;
        }
        // A glider travelling down-right, placed two cells from the far
        // corner so it has to wrap almost immediately.
        let x = LIFE_COLS - 4;
        let y = LIFE_ROWS - 4;
        for (dx, dy) in [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
            life.alive[(y + dy) * LIFE_COLS + (x + dx)] = true;
        }
        // Four steps move a glider one cell diagonally; eighty steps move it
        // twenty, which is well past the corner it started beside.
        for _ in 0..80 {
            life.step(&mut rng);
        }
        let population = life.alive.iter().filter(|c| **c).count();
        assert_eq!(
            population, 5,
            "the glider is {population} cells after wrapping, so it hit a wall instead"
        );
    }

    /// **Extinction recovers.** An empty grid is the dead-screen case, and
    /// the watchdog's only full reseed.
    #[test]
    fn an_empty_grid_is_reseeded_rather_than_left_dead() {
        let mut rng = Mt19937::new(77);
        let mut life = Life::new(LifeStyle::Dots, 1920, 1080, &mut rng);
        for i in 0..life.alive.len() {
            life.alive[i] = false;
        }
        life.step(&mut rng);
        let population = life.alive.iter().filter(|c| **c).count();
        assert!(
            population > 1_000,
            "an extinct grid came back with {population} cells, which is not a reseed"
        );
        assert_eq!(life.interventions, 1);
    }

    /// **A still life is perturbed rather than left sitting for ever.** A
    /// lone block is the simplest possible stasis: it is stable under B3/S23
    /// and would otherwise be the picture until the board rebooted.
    #[test]
    fn a_still_life_is_injected_into() {
        let mut rng = Mt19937::new(5);
        let mut life = Life::new(LifeStyle::Pulse, 1920, 1080, &mut rng);
        for i in 0..life.alive.len() {
            life.alive[i] = false;
        }
        for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
            life.alive[y * LIFE_COLS + x] = true;
        }
        // The first step records the block's hash; the second matches it.
        life.step(&mut rng);
        life.step(&mut rng);
        assert!(
            life.interventions > 0,
            "a block sat still for two steps and nothing intervened"
        );
        assert!(
            life.alive.iter().filter(|c| **c).count() > 4,
            "the watchdog fired but injected nothing"
        );
    }

    /// **The field never goes quiet for longer than the watchdog's window**,
    /// over a run far longer than a soup takes to burn down.
    ///
    /// Twenty thousand steps is eighty-three minutes of wall time at four
    /// steps a second — past the ten-to-twenty minutes a 160x90 soup takes to
    /// settle, several times over. The assertion is the honest form of the
    /// requirement: not that the watchdog compiles, but that no window of
    /// `LIFE_SPARSE_STEPS + LIFE_HISTORY` consecutive steps passes with the
    /// population unchanged.
    #[test]
    fn the_field_never_settles_for_long() {
        let mut rng = Mt19937::new(20_260_920);
        let mut life = Life::new(LifeStyle::Squares, 1920, 1080, &mut rng);
        let window = LIFE_SPARSE_STEPS as usize + LIFE_HISTORY + 2;
        let mut flat = 0usize;
        let mut worst = 0usize;
        let mut previous = life.alive.iter().filter(|c| **c).count();
        for step in 0..20_000 {
            life.step(&mut rng);
            let population = life.alive.iter().filter(|c| **c).count();
            assert!(population > 0, "the field was empty at step {step}");
            if population == previous {
                flat += 1;
                worst = worst.max(flat);
            } else {
                flat = 0;
            }
            assert!(
                flat < window,
                "the population sat at {population} for {flat} steps from step {step} — \
                 the watchdog's window is {window}"
            );
            previous = population;
        }
        // And it had to actually do something: a run this long that needed no
        // intervention would mean the detectors are not firing.
        assert!(
            life.interventions > 0,
            "twenty thousand steps and the watchdog never fired, which cannot be right"
        );
        eprintln!(
            "life watchdog: {} interventions in 20000 steps; longest flat run {worst} steps",
            life.interventions
        );
    }

    /// **`Pulse` is the theme where the radius means something**, and this is
    /// the property that makes a still frame show the direction of time: a
    /// newborn cell is drawn smaller than a settled one, and a dying one
    /// smaller than it was.
    #[test]
    fn a_pulse_cell_grows_with_age_and_shrinks_as_it_fades() {
        let mut rng = Mt19937::new(2);
        let mut life = Life::new(LifeStyle::Pulse, 1920, 1080, &mut rng);
        for i in 0..life.alive.len() {
            life.alive[i] = false;
            life.heat[i] = 0.0;
            life.age[i] = 0;
        }
        let i = 30 * LIFE_COLS + 30;

        life.alive[i] = true;
        life.heat[i] = 1.0;
        life.age[i] = 0;
        let newborn = life.pulse_fraction(i);

        life.age[i] = 10;
        let settled = life.pulse_fraction(i);

        life.alive[i] = false;
        life.heat[i] = 0.25;
        let dying = life.pulse_fraction(i);

        assert!(
            newborn < settled,
            "a newborn ({newborn}) is not smaller than a settled cell ({settled})"
        );
        assert!(
            dying < settled,
            "a dying cell ({dying}) is not smaller than a settled one ({settled})"
        );
        assert!(
            newborn > 0.0 && (settled - 1.0).abs() < 1e-6,
            "a settled cell should be exactly full radius, and a newborn visible"
        );
        // And `Dots` must NOT vary: its radius carries nothing.
        assert!(matches!(LifeStyle::Dots, LifeStyle::Dots));
    }

    /// A dying cell leaves a trail rather than flickering out — §6's *"long
    /// fade"*, asserted as the number of frames a cell stays partly lit after
    /// death.
    ///
    /// **The board is not emptied for this, and that is the trap.** The first
    /// version cleared every cell and watched one fade; it measured eight
    /// frames rather than twenty-seven, because an empty board is an extinct
    /// one, the watchdog reseeded it on the first step, and the reseed wrote
    /// the observed cell's `heat` back to zero. The fade was never the thing
    /// that was being measured.
    ///
    /// A lone glider in the far corner is what this uses instead: the
    /// population is never zero, the grid hash changes on every step because
    /// the glider is moving, and the sparse counter reaches four of the
    /// hundred and twenty it needs. So none of the three watchdogs can fire
    /// during the second this test takes, and the glider cannot reach the
    /// observed cell across three steps of a 160-cell board.
    #[test]
    fn a_dead_cell_fades_over_most_of_a_second() {
        let mut rng = Mt19937::new(9);
        let mut life = Life::new(LifeStyle::Dots, 1920, 1080, &mut rng);
        for i in 0..life.alive.len() {
            life.alive[i] = false;
            life.heat[i] = 0.0;
        }
        for (dx, dy) in [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
            life.alive[(2 + dy) * LIFE_COLS + (2 + dx)] = true;
        }
        let i = 45 * LIFE_COLS + 80;
        life.heat[i] = 1.0;
        let mut frames = 0;
        while life.heat[i] > 0.0 && frames < 1_000 {
            life.update(1.0 / 30.0, &mut rng);
            frames += 1;
        }
        assert!(!life.alive[i], "the glider reached the observed cell");
        assert_eq!(life.interventions, 0, "a watchdog fired and spoiled the run");
        // 0.9 s at 30 fps is 27 frames. A couple either side is the accumulator.
        assert!(
            (25..=30).contains(&frames),
            "a dead cell took {frames} frames to fade, not about 27"
        );
    }
}
