//! **Contours, drawn by the V3D instead of by the ARM cores.**
//!
//! `GPU-Backgrounds-2026-09-20` §1 measured the CPU version on `rivet-01` and
//! it does not fit:
//!
//! ```text
//! background 34.29 ms, DRAWING 44.00 ms of the 33.3 ms budget (132%)
//! ```
//!
//! §1 also says why it is the right thing to move: *"a 96x54 scalar field
//! sampled per pixel and scaled to two megapixels. On the V3D that is one
//! textured quad with the texture unit doing the bilinear filtering in
//! hardware — the single most GPU-shaped workload in this repository."*
//!
//! This is that sentence written out. It exists to answer one question with a
//! number — **does GPU Contours come in under the 33.3 ms budget?** — and it is
//! deliberately a faithful port rather than an improved one.
//!
//! # What is the same, and it has to be
//!
//! Every constant is taken from `aura-background-pack`'s `designed.rs`, not
//! re-tuned: `WAVE_*`, `DRIFT_*`, the 96x54 field, `CONTOUR_RATE`,
//! `CONTOUR_HALF_WIDTH`, and Ice Blue's colour and level count. A version that
//! drew a nicer picture at a different cost would answer a question nobody
//! asked.
//!
//! # What moved, and where each half now runs
//!
//! | | CPU version | here |
//! | :-- | :-- | :-- |
//! | the 96x54 field | 300 sines a frame | **unchanged, still the CPU** — 5,184 cells is nothing, and it is uploaded as a 5 KB texture |
//! | bilinear interpolation to 2 MPx | `AxisMap`, per run, by hand | **the texture unit**, `GL_LINEAR`, free |
//! | the gradient | computed per run from neighbouring cells | **`fwidth(v)`**, which is what the hardware derivative is for |
//! | `distance < threshold` → coverage | per pixel on the ARM cores | one fragment shader, 2 MPx in parallel |
//!
//! **`fwidth` is the whole trick.** The CPU version's longest comment argues
//! that dividing by the local gradient is what keeps every line the same width
//! regardless of the slope under it. `fwidth(v)` is exactly that quantity —
//! the change in `v` between adjacent fragments — so the shader gets for one
//! instruction what the run loop computes ninety-six times a row.
//!
//! # And then Life, which is the same shape of answer with one difference
//!
//! Conway's Game of Life measured `background 26.15/49.56 ms` on the CPU on
//! `rivet-01` — 79% of the 33.3 ms budget on its own, and **`DRAWING 35.07 ms`,
//! 105%, once the widgets' 8.91 ms are added**. It is in the same position
//! Contours was, it had simply never been measured, and it moves here for the
//! same reason. The port is Aurora's mechanism plus a modulo:
//!
//! | | CPU version | here |
//! | :-- | :-- | :-- |
//! | the automaton | 14,400 cells, 4 steps a second | **unchanged, still the CPU** — see [`Life`] |
//! | the 2 MPx clear | `draw::fill` over the whole canvas | the fragment shader's own output; there is no clear to do |
//! | a disc per live cell | `circle_alpha`, a bounding box each | **one fragment shader**, `floor(uv * grid)` to find the cell |
//! | the antialiased edge | `disc_coverage` per pixel | the same expression, one instruction |
//!
//! **`GL_NEAREST` and not `GL_LINEAR`, and this is the one place the pattern
//! differs.** Contours and Aurora are sampled fields and want the texture unit
//! interpolating between cells; Life's cells are *discrete* and a linear filter
//! would smear each one into its neighbours — a glider would arrive as a blur
//! rather than as five cells.
//!
//! **The board is `GL_LUMINANCE_ALPHA` and not `GL_LUMINANCE`**, which is the
//! one deviation from the two-background pattern that is worth stating up
//! front. `Pulse` needs *two* numbers per cell — the heat, which is the colour
//! and the trail, and the radius fraction, which is the cell's age — and they
//! are independent: a newborn and a settled cell both have heat `1.0` and must
//! be a pinprick and a full disc respectively. One 8-bit channel cannot carry
//! both, and carrying the second in a second texture would cost a second fetch
//! for no gain. Two channels, one fetch.
//!
//! # Starfield, which is the one that does NOT fit the pattern
//!
//! Contours, Aurora and Life are all the same sentence: *upload a small grid,
//! let one full-screen fragment shader read it*. That works because their state
//! **is** a grid — every pixel's answer is a function of the cell it lands in.
//!
//! **Starfield's state is 320 moving points, and a point is not a grid.** The
//! pattern above, applied here, would be a full-screen shader testing every
//! fragment against every star: 2 MPx x 320 = **640 million distance tests a
//! frame**, against the CPU version's 320 writes of one or two pixels each. It
//! would draw the right picture and be three orders of magnitude slower than the
//! thing it was replacing, which is the exact failure a port like this invites.
//!
//! So Starfield is the other primitive: **`GL_POINTS` with a vertex buffer**,
//! one vertex per star.
//!
//! | | CPU version | here |
//! | :-- | :-- | :-- |
//! | 320 rays advancing | `radius += speed * dt`, per star | **unchanged, still the CPU** — see [`Starfield`]; 320 stars is nothing |
//! | the 2 MPx clear | `draw::clear` | `glClear`, which on a tile-based renderer is a tile flag and not 2 MPx of writes |
//! | plotting a star | `draw::fill` of a 1x1 or 2x2 rect | one `GL_POINTS` vertex, `gl_PointSize` from the layer |
//! | the per-star brightness | `scale(colour, brightness)` on the CPU | a `varying float`, multiplied in the fragment shader |
//!
//! **The upload is 320 vertices of four floats — 5 KB a frame**, which is a
//! quarter of what Contours' field costs and an eighth of Life's board. The
//! interesting consequence is that Starfield's GPU cost is almost entirely the
//! **clear and the tile store**, because the ~400 fragments the points actually
//! cover are not a measurable quantity. Whatever number it reports is therefore
//! close to the *floor* for any GPU background on this surface, and that makes
//! it worth having even if it does not beat the CPU.
//!
//! # And Boxes, which fits the pattern so exactly it is a calibration
//!
//! Boxes is an 8x8 grid of flat rectangles that change colour once a second. Its
//! state **is** a grid — the smallest one in the pack, 64 cells — so it goes
//! back to the full-screen quad with a `GL_NEAREST` 8x8 `GL_RGB` texture, and
//! the shader is one fetch and a multiply.
//!
//! **There is nothing else in it.** No field, no derivative, no distance, no
//! second texture. That is the point of measuring it: Boxes' number is *"what
//! does it cost this V3D to write two megapixels at all"*, and every other
//! background's number minus Boxes' is what that background's own shader costs.
//! Contours at `8.19 ms` and Aurora at `6.76 ms` cannot be read without it.
//!
//! **It is also the one most likely to lose to the CPU**, and that is said here
//! rather than discovered later: 64 rectangles is 64 `draw::fill` calls that now
//! reach `memset`, and a two-megapixel `memset` on four ARM cores is not
//! obviously worse than two megapixels through a tile store. See
//! [`BOX_TICK_SECONDS`] for the other half of that argument.

use std::ffi::{c_char, c_int, c_uint, c_void};

// ---- the field, unchanged from `designed.rs` ------------------------------

const FIELD_W: usize = 96;
const FIELD_H: usize = 54;

const WAVE_X: f32 = 0.031;
const WAVE_Y: f32 = 0.043;
const WAVE_D: f32 = 0.024;
const DRIFT_X: f32 = 0.070;
const DRIFT_Y: f32 = 0.052;
const DRIFT_D: f32 = 0.031;

/// §6 asks Contours to drift slowly; this is how much slower than Aurora.
const CONTOUR_RATE: f32 = 0.38;
/// Aurora drifts at the rate the `DRIFT_*` family was tuned at.
const AURORA_RATE: f32 = 1.0;

/// **Aurora's Green/Violet stops**, low to high — `designed.rs`'s
/// `AuroraPalette::GreenViolet`, which is the first of its three and the one
/// the picker shows first.
///
/// Every ramp starts near black on purpose: Aurora is the brightest background
/// in the pack and the only one with no black ground at all, so the low end of
/// the ramp is the only darkness on the screen.
const AURORA_STOPS: [[u8; 3]; 5] = [
    [4, 6, 14],
    [10, 60, 50],
    [30, 150, 90],
    [90, 110, 190],
    [150, 70, 180],
];

/// The five stops expanded into the 256-entry ramp `designed.rs` builds once
/// and indexes per pixel.
///
/// **On the GPU this becomes a 256x1 texture and the indexing becomes a
/// second `texture2D`** — with `GL_LINEAR` the hardware interpolates between
/// entries, which is strictly smoother than the CPU version's integer index
/// and costs nothing.
fn aurora_ramp() -> [u8; 256 * 3] {
    let mut out = [0u8; 256 * 3];
    for (i, slot) in out.chunks_exact_mut(3).enumerate() {
        // Four segments between five stops.
        let position = i as f32 / 255.0 * 4.0;
        let low = (position as usize).min(3);
        let frac = position - low as f32;
        for channel in 0..3 {
            let a = f32::from(AURORA_STOPS[low][channel]);
            let b = f32::from(AURORA_STOPS[low + 1][channel]);
            slot[channel] = (a + (b - a) * frac + 0.5) as u8;
        }
    }
    out
}
/// Half a line's width in pixels. `designed.rs`'s value and its reasoning.
const CONTOUR_HALF_WIDTH: f32 = 1.3;

/// Ice Blue — the theme `rivet-01` is actually showing, so the comparison is
/// against the number this board reported rather than against a different one.
/// It is RGB `190, 215, 255`; the blue channel is written as the `1.0` that
/// `255 / 255` is.
const COLOUR: [f32; 3] = [190.0 / 255.0, 215.0 / 255.0, 1.0];
/// Ice Blue's level count: the densest of the three.
const LEVELS: f32 = 20.0;

/// `sin` in turns, as `designed.rs` computes it.
///
/// **`designed.rs` reads a 2048-entry table and interpolates; this calls libm.**
/// That deviation predates Starfield and is deliberate: the table exists there
/// to keep hundreds of thousands of sines a second off libm, and nothing in this
/// probe evaluates more than 300 of them a frame. The two agree to within the
/// table's own interpolation error, which for [`Field`] is a fraction of one
/// 8-bit texture level and for [`Starfield`] is a fraction of a degree of a
/// respawn's direction.
fn sin_turns(turns: f32) -> f32 {
    (turns * std::f32::consts::TAU).sin()
}

/// Cosine, for the one place that needs a direction vector rather than a wave —
/// sine a quarter turn ahead. `designed.rs`'s `cos_turns`, verbatim.
fn cos_turns(turns: f32) -> f32 {
    sin_turns(turns + 0.25)
}

/// The drifting height field. **Still on the CPU, and that is not a
/// compromise**: 5,184 cells of three multiplies each is under a tenth of a
/// millisecond, and moving it to the GPU would mean a second shader pass to
/// save nothing measurable.
pub struct Field {
    cells: [f32; FIELD_W * FIELD_H],
    time: f32,
    phase: [f32; 3],
    /// **Aurora and Contours are the same field at different speeds** —
    /// `aura-background-pack`'s `designed.rs` module doc: they *"share one
    /// 96x54 HeightField and differ only"* in how they read it.
    /// Carrying the rate here is what lets one struct serve both.
    rate: f32,
}

impl Field {
    #[must_use]
    pub fn new(rate: f32) -> Self {
        let mut field = Self {
            cells: [0.0; FIELD_W * FIELD_H],
            time: 0.0,
            // Fixed rather than seeded: a probe that drew a different picture
            // on every run would make two measurements incomparable.
            phase: [0.13, 0.57, 0.81],
            rate,
        };
        field.update(0.0);
        field
    }

    /// `designed.rs`'s `HeightField::update`, verbatim in shape — 300 sines,
    /// not 15,552, because each wave's argument depends on one axis only.
    pub fn update(&mut self, dt: f32) {
        self.time += dt * self.rate;
        let t = self.time;

        let mut wave_x = [0.0f32; FIELD_W];
        for (x, slot) in wave_x.iter_mut().enumerate() {
            let fx = x as f32;
            *slot = sin_turns(fx.mul_add(WAVE_X, t * DRIFT_X) + self.phase[0]);
        }
        let mut wave_y = [0.0f32; FIELD_H];
        for (y, slot) in wave_y.iter_mut().enumerate() {
            let fy = y as f32;
            *slot = sin_turns(fy.mul_add(WAVE_Y, -t * DRIFT_Y) + self.phase[1]);
        }
        let mut wave_d = [0.0f32; FIELD_W + FIELD_H];
        for (d, slot) in wave_d.iter_mut().enumerate() {
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

    /// The field as bytes, for the texture upload. `GL_LUMINANCE` is 8-bit,
    /// which is 1/256 of the field's range — far finer than
    /// `CONTOUR_HALF_WIDTH` can show at 20 levels, and a float texture would
    /// need an extension for no visible gain.
    fn bytes(&self) -> [u8; FIELD_W * FIELD_H] {
        let mut out = [0u8; FIELD_W * FIELD_H];
        for (slot, cell) in out.iter_mut().zip(self.cells.iter()) {
            *slot = (cell.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        out
    }
}

// ---- Conway's Game of Life, unchanged from `designed.rs` ------------------

/// `std::mt19937`, carried over from `aura-background-pack`'s `rng.rs` so that
/// the soup this probe draws is the soup the CPU version draws.
///
/// Only the four entry points Life uses are here — `new`, `next_u32`, `twist`,
/// `real` and `int` — and each is the pack's byte for byte. Nothing about the
/// measurement depends on the *sequence*, but the **distribution** decides how
/// many cells are alive, and a generator that clumped would hand the GPU a
/// different picture than the one the 26.15 ms was measured against.
struct Mt19937 {
    state: [u32; MT_N],
    index: usize,
}

const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_MATRIX_A: u32 = 0x9908_b0df;
const MT_UPPER: u32 = 0x8000_0000;
const MT_LOWER: u32 = 0x7fff_ffff;

impl Mt19937 {
    fn new(seed: u32) -> Self {
        let mut state = [0u32; MT_N];
        state[0] = seed;
        for i in 1..MT_N {
            state[i] = 1_812_433_253u32
                .wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        Self { state, index: MT_N }
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= MT_N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    fn twist(&mut self) {
        for i in 0..MT_N {
            let y = (self.state[i] & MT_UPPER) | (self.state[(i + 1) % MT_N] & MT_LOWER);
            let mut next = self.state[(i + MT_M) % MT_N] ^ (y >> 1);
            if y & 1 != 0 {
                next ^= MT_MATRIX_A;
            }
            self.state[i] = next;
        }
        self.index = 0;
    }

    #[allow(clippy::cast_precision_loss)]
    fn real(&mut self, low: f32, high: f32) -> f32 {
        let unit = (self.next_u32() >> 8) as f32 / 16_777_216.0;
        low + (high - low) * unit
    }

    fn int(&mut self, low: i32, high: i32) -> i32 {
        if low >= high {
            return low;
        }
        let span = (i64::from(high) - i64::from(low) + 1) as u64;
        let limit = (u64::from(u32::MAX) + 1) / span * span;
        loop {
            let draw = u64::from(self.next_u32());
            if draw < limit {
                return (i64::from(low) + (draw % span) as i64) as i32;
            }
        }
    }
}

/// **The seed is fixed**, for [`Field`]'s reason: a probe that drew a different
/// soup on every run would make two measurements incomparable, and the whole
/// point of this program is that the number it prints can be put beside another
/// number. `designed.rs` uses `Mt19937::from_the_clock` in production, where the
/// opposite is wanted.
const LIFE_SEED: u32 = 20_260_920;

/// Life's themes — **the cell's shape** — exactly as `designed.rs` lists them.
/// Only [`LifeStyle::Pulse`] is measured here; see [`LIFE_STYLE`].
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeStyle {
    /// Filled square cells. The radius carries nothing.
    Squares,
    /// Circles at a fixed radius. The radius carries nothing.
    Dots,
    /// Circles whose **radius carries the cell's age** — `designed.rs` calls
    /// this *"the one worth having"*, because a glider under it grows a head
    /// of pinpricks and a shrinking tail, so a still frame shows which way it
    /// is travelling.
    Pulse,
}

/// **`Pulse`, because it is the expensive one and the measured one.**
///
/// It draws a disc for every cell with any heat at all, at a radius that
/// changes per cell per frame; `Squares` is a fill and `Dots` is a disc at one
/// radius. The `26.15 ms` this probe exists to beat was `Pulse`, so `Pulse` is
/// what has to appear in the report for the two numbers to mean the same
/// thing. The other two are supported — see [`LIFE_FRAGMENT_SRC`] and
/// [`LIFE_SQUARES_FRAGMENT_SRC`] — and cost strictly less.
///
/// **Changing this means changing two strings with it**: `Which::id`'s
/// `"life/pulse"`, which is the id the CPU measurement was filed under, and
/// [`Scene::describe`]. They are written out rather than derived because a
/// report that names the wrong style is worse than one that names none.
const LIFE_STYLE: LifeStyle = LifeStyle::Pulse;

/// §6's *"roughly 160x90 cells"*. At 1080p a cell is exactly 12 pixels.
const LIFE_COLS: usize = 160;
/// See [`LIFE_COLS`].
const LIFE_ROWS: usize = 90;
const LIFE_CELLS: usize = LIFE_COLS * LIFE_ROWS;

/// §6: *"four steps a second is livelier to watch than sixty"*.
const LIFE_STEPS_PER_SECOND: f32 = 4.0;
/// §6's *"long fade on dying cells"* — three and a half steps of decay.
const LIFE_FADE_SECONDS: f32 = 0.9;

/// The family's `ice-blue` ink, as `designed.rs` has it, in the `[0, 1]` the
/// uniform wants rather than the `[0, 255]` a canvas wants.
const LIFE_COLOUR: [f32; 3] = [190.0 / 255.0, 205.0 / 255.0, 247.0 / 255.0];

/// How many steps a cell must survive to reach full radius under `Pulse`.
const LIFE_SETTLE_STEPS: f32 = 3.0;
/// The fraction of full radius a newborn cell is drawn at.
const LIFE_NEWBORN_FRACTION: f32 = 0.18;
/// A dying cell's radius shrinks to this fraction rather than to nothing.
const LIFE_DYING_FLOOR: f32 = 0.35;

/// How many recent grid states the stasis watchdog remembers. Sixteen covers
/// every oscillator a Conway soup actually settles into.
const LIFE_HISTORY: usize = 16;
/// A population at or below this fraction of the grid counts as attrition.
const LIFE_SPARSE_FRACTION: f32 = 0.005;
/// How many consecutive sparse steps are tolerated before an injection.
const LIFE_SPARSE_STEPS: u32 = 120;

/// The injected patch: roughly 20x20 at about 35%.
const LIFE_PATCH_W: usize = 20;
/// See [`LIFE_PATCH_W`].
const LIFE_PATCH_H: usize = 20;
/// See [`LIFE_PATCH_W`].
const LIFE_PATCH_DENSITY: f32 = 0.35;
/// The density of a full reseed.
const LIFE_SOUP_DENSITY: f32 = 0.30;

/// `Pulse`'s full radius, as a fraction of the shorter cell edge —
/// `self.cell_w.min(self.cell_h) * 0.46` in `Life::draw`.
const LIFE_FULL_RADIUS: f32 = 0.46;
/// `Dots`' fixed radius, from the same line: `* 0.40`.
///
/// **It is expressed as a fraction of [`LIFE_FULL_RADIUS`]** and written into
/// the same texture channel `Pulse` writes its age fraction into, so one
/// shader serves both styles and the difference between them is entirely on
/// the CPU side. `0.40 / 0.46` reproduces the CPU radius exactly.
const LIFE_DOT_FRACTION: f32 = 0.40 / LIFE_FULL_RADIUS;

/// **Conway's Game of Life, B3/S23, on a torus — the simulation only.**
///
/// # Why the automaton did not move to the GPU, and it is not a compromise
///
/// 14,400 cells stepping four times a second is **57,600 neighbour sums a
/// second**, which is under a tenth of a millisecond of ARM time. Moving it to
/// the GPU would mean a pair of ping-pong textures, a second program and a
/// render-to-texture pass, to save a quantity that does not appear in the
/// measurement. [`Field`] carries the same argument for the same reason.
///
/// **What was costing 26.15 ms was never the automaton.** It was a two-megapixel
/// clear plus an antialiased disc per live cell — **on every frame, not on
/// every step**, because [`LIFE_FADE_SECONDS`] keeps the picture changing
/// between steps and `Pulse`'s radius is interpolated across `progress`. Both
/// of those are one fragment shader here.
///
/// Everything below is `designed.rs`'s `Life` with the drawing removed: the
/// B3/S23 step, the toroidal wrap, the per-frame fade, the age that is **not**
/// reset on death, and all three arms of the watchdog (extinction → reseed,
/// stasis → injection, attrition → injection). The rule itself is untouched,
/// which is the point `designed.rs` makes at length.
pub struct Life {
    alive: Vec<bool>,
    /// Steps survived, saturating; **not reset on death**, so a corpse tapers
    /// from the size it reached rather than from full radius.
    age: Vec<u8>,
    /// `1.0` while alive, decaying to `0.0` over [`LIFE_FADE_SECONDS`]. The
    /// trail, and the thing the red channel of the texture carries.
    heat: Vec<f32>,
    next: Vec<bool>,
    accumulator: f32,
    /// How far through the current step, in `[0, 1)`. `Pulse` interpolates a
    /// growing cell's radius across it, so growth is smooth at 30 fps.
    progress: f32,
    history: [u64; LIFE_HISTORY],
    history_at: usize,
    sparse_steps: u32,
    /// Reported at the end of the run: how often the watchdog had to fire.
    interventions: u32,
    /// Reported at the end of the run, so that a suspiciously cheap number can
    /// be checked against "did the board actually do anything?".
    steps: u64,
    rng: Mt19937,
}

impl Life {
    #[must_use]
    pub fn new() -> Self {
        let mut life = Self {
            alive: vec![false; LIFE_CELLS],
            age: vec![0; LIFE_CELLS],
            heat: vec![0.0; LIFE_CELLS],
            next: vec![false; LIFE_CELLS],
            accumulator: 0.0,
            progress: 0.0,
            // Not `[0; N]`: an all-zero ring would match an empty grid's hash
            // on the first step and fire the stasis watchdog immediately.
            history: [u64::MAX; LIFE_HISTORY],
            history_at: 0,
            sparse_steps: 0,
            interventions: 0,
            steps: 0,
            rng: Mt19937::new(LIFE_SEED),
        };
        life.reseed();
        life
    }

    /// Fill the whole grid with a fresh soup. Only extinction reaches this
    /// after construction.
    fn reseed(&mut self) {
        for i in 0..LIFE_CELLS {
            let alive = self.rng.real(0.0, 1.0) < LIFE_SOUP_DENSITY;
            self.alive[i] = alive;
            self.age[i] = 0;
            self.heat[i] = if alive { 1.0 } else { 0.0 };
        }
    }

    /// Drop a patch of soup somewhere at random, leaving everything else —
    /// including whatever is still fading — exactly as it was.
    fn inject(&mut self) {
        let x0 = self.rng.int(0, LIFE_COLS as i32 - 1) as usize;
        let y0 = self.rng.int(0, LIFE_ROWS as i32 - 1) as usize;
        for dy in 0..LIFE_PATCH_H {
            for dx in 0..LIFE_PATCH_W {
                // Wrapped, so a patch near an edge is a patch and not a
                // clipped rectangle.
                let x = (x0 + dx) % LIFE_COLS;
                let y = (y0 + dy) % LIFE_ROWS;
                if self.rng.real(0.0, 1.0) < LIFE_PATCH_DENSITY {
                    let i = y * LIFE_COLS + x;
                    self.alive[i] = true;
                    self.age[i] = 0;
                    self.heat[i] = 1.0;
                }
            }
        }
    }

    /// FNV-1a over the live bits, eight cells to a byte.
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
    /// **Toroidal, and that is not an aesthetic choice**: on a bounded grid
    /// every glider dies at the wall, and a glider is the most interesting
    /// thing on this screen.
    fn step(&mut self) {
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
                // B3/S23, Conway's, unmodified.
                self.next[here + x] = if self.alive[here + x] {
                    neighbours == 2 || neighbours == 3
                } else {
                    neighbours == 3
                };
            }
        }

        let mut population = 0usize;
        for i in 0..LIFE_CELLS {
            let now = self.next[i];
            if now {
                population += 1;
                // Age advances only for a cell that was ALREADY alive. A birth
                // resets it, which is what makes `Pulse`'s pinprick a birth
                // rather than merely a small cell.
                self.age[i] = if self.alive[i] { self.age[i].saturating_add(1) } else { 0 };
                self.heat[i] = 1.0;
            }
            self.alive[i] = now;
        }

        self.steps += 1;
        self.watchdog(population);
    }

    /// The three interventions, in the order that matters: extinction first,
    /// because an empty grid is also a cyclic one.
    fn watchdog(&mut self, population: usize) {
        if population == 0 {
            self.reseed();
            self.interventions += 1;
            self.forget();
            return;
        }

        #[allow(clippy::cast_precision_loss)]
        let sparse = (population as f32) < LIFE_CELLS as f32 * LIFE_SPARSE_FRACTION;
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
            self.inject();
            self.interventions += 1;
            self.forget();
        }
    }

    /// Clear the cycle ring after an intervention, or a just-injected still
    /// life matches its own pre-injection hashes for sixteen more steps.
    fn forget(&mut self) {
        self.history = [u64::MAX; LIFE_HISTORY];
        self.history_at = 0;
        self.sparse_steps = 0;
    }

    /// The fade runs per FRAME and the automaton per STEP — §6's *"a step does
    /// not have to happen every frame"*, in both directions.
    pub fn update(&mut self, dt: f32) {
        let decay = dt / LIFE_FADE_SECONDS;
        for i in 0..LIFE_CELLS {
            if !self.alive[i] && self.heat[i] > 0.0 {
                self.heat[i] = (self.heat[i] - decay).max(0.0);
            }
        }

        let period = 1.0 / LIFE_STEPS_PER_SECOND;
        self.accumulator += dt;
        while self.accumulator >= period {
            self.accumulator -= period;
            self.step();
        }
        self.progress = (self.accumulator / period).clamp(0.0, 1.0);
    }

    /// The radius a cell is drawn at under `Pulse`, as a fraction of full.
    /// `designed.rs`'s `pulse_fraction`, unchanged.
    fn pulse_fraction(&self, i: usize) -> f32 {
        // A living cell's growth is interpolated across the current step; a
        // dead one's is frozen, because `progress` belongs to a step it is no
        // longer taking part in.
        let steps = if self.alive[i] {
            f32::from(self.age[i]) + self.progress
        } else {
            f32::from(self.age[i])
        };
        let settled = (steps / LIFE_SETTLE_STEPS).clamp(0.0, 1.0);
        let grown = (1.0 - LIFE_NEWBORN_FRACTION).mul_add(settled, LIFE_NEWBORN_FRACTION);
        // Shrinking with the fade, but only to a floor: a radius that reached
        // zero would make the tail vanish twice as fast as the fade intends.
        let shrink = (1.0 - LIFE_DYING_FLOOR).mul_add(self.heat[i], LIFE_DYING_FLOOR);
        grown * shrink
    }

    /// The board as `GL_LUMINANCE_ALPHA` texels: **luminance is the heat,
    /// alpha is the radius fraction.**
    ///
    /// Both quantise to 8 bits and neither cares. A full radius is 5.52 px at
    /// 1080p, so one step of the alpha channel is 0.02 px — two orders below
    /// what the antialiased edge can show. The heat fades over 0.9 s, which at
    /// 30 fps is nine levels of 255 a frame.
    ///
    /// An array and not a `Vec`, for [`Field::bytes`]'s reason: this runs
    /// inside the timed region and a 28,800-byte allocation a frame would be
    /// measured as part of the drawing it is not part of.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn texels(&self) -> [u8; LIFE_CELLS * 2] {
        let mut out = [0u8; LIFE_CELLS * 2];
        for (i, texel) in out.chunks_exact_mut(2).enumerate() {
            texel[0] = (self.heat[i].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            texel[1] = match LIFE_STYLE {
                // The shape shader for squares never reads this channel.
                LifeStyle::Squares => 0,
                LifeStyle::Dots => (LIFE_DOT_FRACTION * 255.0 + 0.5) as u8,
                LifeStyle::Pulse => (self.pulse_fraction(i).clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            };
        }
        out
    }

    /// What the report says about the automaton after a run — the two numbers
    /// that distinguish *"cheap because the GPU is fast"* from *"cheap because
    /// nothing was happening"*.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "the automaton took {} generation(s) on the CPU, {} watchdog intervention(s)",
            self.steps, self.interventions
        )
    }
}

// ---- Starfield, unchanged from `designed.rs` ------------------------------

/// §6's *"a few hundred points"*. `designed.rs`'s `STAR_COUNT`.
const STAR_COUNT: usize = 320;

/// §6's *"three parallax layers"*, as speed multipliers. `designed.rs`'s
/// `STAR_LAYER_SPEED`.
const STAR_LAYER_SPEED: [f32; 3] = [0.55, 0.80, 1.15];

/// And as brightness — `designed.rs`'s `STAR_LAYER_BRIGHTNESS`. **The near
/// layer is brighter**, which is the only depth cue that survives at a
/// one-pixel plot.
const STAR_LAYER_BRIGHTNESS: [f32; 3] = [0.42, 0.68, 1.0];

/// Acceleration with distance: a star's outward speed is
/// `radius * STAR_SPREAD + STAR_BASE_SPEED`. `designed.rs`'s value and its
/// reasoning — it is what makes the motion read as flying forward rather than
/// as an expanding ring.
const STAR_SPREAD: f32 = 0.55;

/// Pixels per second at the vanishing point itself, so a just-respawned star
/// still moves. `designed.rs`'s `STAR_BASE_SPEED`.
const STAR_BASE_SPEED: f32 = 25.0;

/// **Ice Blue**, for [`COLOUR`]'s and [`LIFE_COLOUR`]'s reason: it is the theme
/// `rivet-01` is showing, so the comparison is against the number this board
/// reported. `StarPalette::IceBlue::colour()` — `kCardIce`'s ink, the same
/// three bytes [`LIFE_COLOUR`] carries.
const STAR_COLOUR: [f32; 3] = [190.0 / 255.0, 205.0 / 255.0, 247.0 / 255.0];

/// `StarPalette::IceBlue::rate()` — *"the fastest of the three"*. It multiplies
/// every layer's speed, so it belongs beside [`STAR_LAYER_SPEED`] and not in a
/// theme enum this probe does not need.
const STAR_RATE: f32 = 1.35;

/// [`LIFE_SEED`]'s argument, for the same reason: a probe that drew a different
/// field on every run would make two measurements incomparable.
///
/// **`designed.rs` shares ONE `Mt19937` across the whole host**, so a star's
/// respawn there draws from a stream Digital Rain and Boxes are also pulling
/// on. A dedicated generator changes that *sequence* and not the
/// *distribution*, which is what decides how the field looks and how much of it
/// is on screen — the same argument [`Mt19937`]'s own note makes for Life.
const STAR_SEED: u32 = 20_260_920;

/// One point drifting outward, `designed.rs`'s `Star` verbatim.
///
/// Stored as a **direction and a radius** rather than a position and a
/// velocity: a star cannot drift off its own ray, so no accumulated
/// floating-point error can turn radial motion into a slow curve over the hours
/// this runs in production.
#[derive(Clone, Copy)]
struct Star {
    dx: f32,
    dy: f32,
    /// Distance from the vanishing point, in pixels.
    radius: f32,
    /// Index into [`STAR_LAYER_SPEED`]. Fixed for the star's whole life,
    /// including across respawns.
    layer: usize,
}

/// **Points drifting outward from a vanishing point — the simulation only.**
///
/// # Why the motion did not move to the GPU, and it is not a compromise
///
/// 320 stars is **one multiply-add and one bounds test each**, about 2,000
/// floating-point operations a frame. [`Field`] and [`Life`] carry the same
/// argument at 5,184 and 14,400 cells; at 320 it does not need making twice.
///
/// **What the GPU is being asked to do instead is the plotting**, and there is
/// less of that here than anywhere else in the pack: 320 points of one or two
/// pixels is about 400 fragments, against Contours' two million. Starfield's
/// cost on this side is the clear, the tile store and the 5 KB vertex upload —
/// see the module note.
///
/// Everything below is `designed.rs`'s `Starfield` with `draw` replaced by
/// [`Starfield::vertices`]: the same speed law, the same **rectangle** recycle
/// test (not a radius — a 16:9 screen is nearly twice as wide as it is tall),
/// the same distance fade, and the same two-pixel rule for a near star past
/// halfway.
pub struct Starfield {
    stars: [Star; STAR_COUNT],
    centre_x: f32,
    centre_y: f32,
    width: f32,
    height: f32,
    /// The radius at which a star reaches full brightness — half the shorter
    /// screen axis, as `designed.rs` has it.
    full_bright_radius: f32,
    rng: Mt19937,
    /// Reported at the end of the run, for [`Life::summary`]'s reason: a
    /// background can be cheap because the GPU is fast or cheap because nothing
    /// moved, and a respawn count near zero is what the second one looks like.
    respawns: u64,
}

impl Starfield {
    /// **The surface size is taken once, at construction**, exactly as
    /// `designed.rs` does. The compositor does not resize a full-screen surface
    /// under a running application, and a field that recomputed its vanishing
    /// point every frame would be answering a question nobody asked.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn new(width: usize, height: usize) -> Self {
        let (w, h) = (width as f32, height as f32);
        let mut field = Self {
            stars: [Star { dx: 0.0, dy: 0.0, radius: 0.0, layer: 0 }; STAR_COUNT],
            centre_x: w * 0.5,
            centre_y: h * 0.5,
            width: w,
            height: h,
            full_bright_radius: (w.min(h) * 0.5).max(1.0),
            rng: Mt19937::new(STAR_SEED),
            respawns: 0,
        };
        for i in 0..STAR_COUNT {
            let mut star = Star {
                dx: 0.0,
                dy: 0.0,
                radius: 0.0,
                layer: i % STAR_LAYER_SPEED.len(),
            };
            field.respawn(&mut star);
            // **The initial radius is spread over the whole screen**, unlike a
            // respawn's — `designed.rs`'s note about the opening bloom. It also
            // matters here for a reason it does not there: a field that began
            // with every star at the centre would spend the first second of a
            // twenty-second measurement drawing almost nothing.
            star.radius = field.rng.real(2.0, field.full_bright_radius * 1.9);
            field.stars[i] = star;
        }
        field
    }

    /// A fresh direction and a radius just off the vanishing point.
    /// `designed.rs`'s `respawn`, with the borrow rearranged because the rng
    /// lives in the struct here rather than in the host.
    fn respawn(&mut self, star: &mut Star) {
        let turns = self.rng.real(0.0, 1.0);
        star.dx = cos_turns(turns);
        star.dy = sin_turns(turns);
        // Not zero: the first frame of a respawned star's life would otherwise
        // put every one of them on the same pixel.
        star.radius = self.rng.real(2.0, 18.0);
    }

    /// `designed.rs`'s `Starfield::update`, unchanged.
    pub fn update(&mut self, dt: f32) {
        for i in 0..STAR_COUNT {
            let star = self.stars[i];
            let speed = star.radius.mul_add(STAR_SPREAD, STAR_BASE_SPEED)
                * STAR_LAYER_SPEED[star.layer]
                * STAR_RATE;
            let radius = star.radius + speed * dt;
            let x = star.dx.mul_add(radius, self.centre_x);
            let y = star.dy.mul_add(radius, self.centre_y);
            // **Recycled on leaving the RECTANGLE, not on passing a radius** —
            // `designed.rs`'s longest comment in this struct, and the reason a
            // third of the field is not invisible at any given moment.
            if x < 0.0 || y < 0.0 || x >= self.width || y >= self.height {
                let mut fresh = self.stars[i];
                self.respawn(&mut fresh);
                self.stars[i] = fresh;
                self.respawns += 1;
            } else {
                self.stars[i].radius = radius;
            }
        }
    }

    /// **The star list as `GL_POINTS` vertices** — `(x, y, point_size,
    /// brightness)`, four floats each, and the count actually on screen.
    ///
    /// This is `designed.rs`'s `Starfield::draw` with the `draw::fill` replaced
    /// by four numbers. Every line of arithmetic below is that function's:
    /// the same `approach`, the same `0.25 + 0.75 * approach` fade, the same
    /// `min(1.0)`, and the same `layer == 2 && approach > 0.5` rule for the
    /// second pixel.
    ///
    /// # The half-pixel, which is not a transcription
    ///
    /// `draw::fill(canvas, t, x as usize, y as usize, size, size, ..)` paints a
    /// square whose **top-left corner** is the truncated pixel. A `GL_POINTS`
    /// primitive of side `size` is a square **centred** on its vertex, covering
    /// the pixel centres inside it. `floor(x) + size * 0.5` is the centre that
    /// makes those two squares the same square — `+0.5` for a 1x1, `+1.0` for a
    /// 2x2 — and it is computed here rather than in the vertex shader so the
    /// reasoning sits beside the `draw::fill` it is reproducing.
    ///
    /// An array and not a `Vec`, for [`Life::texels`]'s reason: 5 KB is nothing,
    /// but an allocation a frame inside the timed region would be measured as
    /// part of the drawing it is not part of.
    #[allow(clippy::cast_precision_loss)]
    fn vertices(&self) -> ([f32; STAR_COUNT * 4], usize) {
        let mut out = [0.0f32; STAR_COUNT * 4];
        let mut count = 0usize;
        for star in &self.stars {
            let x = star.dx.mul_add(star.radius, self.centre_x);
            let y = star.dy.mul_add(star.radius, self.centre_y);
            // `designed.rs`'s `continue`. `update` has already recycled
            // everything that left, so this is defensive there and here alike —
            // but a star dropped from the buffer is a star not drawn, which is
            // exactly what the CPU version's skip does.
            if x < 0.0 || y < 0.0 || x >= self.width || y >= self.height {
                continue;
            }
            // Distance fades a star IN as it approaches, on top of its layer's
            // constant — the two together are what stops the field reading as a
            // flat sheet of dots at three brightnesses.
            let approach = (star.radius / self.full_bright_radius).clamp(0.0, 1.0);
            let brightness = (STAR_LAYER_BRIGHTNESS[star.layer]
                * 0.25f32.mul_add(1.0, 0.75 * approach))
            .min(1.0);
            // **Two pixels only for a near star past halfway.** §6 says one or
            // two; this is where the second one is spent.
            let size = if star.layer == 2 && approach > 0.5 { 2.0 } else { 1.0 };
            let slot = count * 4;
            out[slot] = x.floor() + size * 0.5;
            out[slot + 1] = y.floor() + size * 0.5;
            out[slot + 2] = size;
            out[slot + 3] = brightness;
            count += 1;
        }
        (out, count)
    }

    /// See [`Starfield::respawns`].
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{STAR_COUNT} stars on the CPU, {} respawn(s) — a field that stopped moving \
             would report none",
            self.respawns
        )
    }
}

// ---- Boxes, unchanged from `background.rs` --------------------------------

/// `BoxesPalette` — `ss_boxes.hpp`, under the ruled names. All four are carried
/// because [`BOX_PALETTE`] picks one and the other three cost four lines each;
/// [`LifeStyle`] is here for the same reason.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxPalette {
    /// The default in `main.cpp`'s `ss_boxes{BoxesPalette::IceBlue}`, and
    /// `CATALOGUE`'s first Boxes row.
    IceBlue,
    Chartreuse,
    Dark,
    BornThisWay,
}

/// **Ice Blue**, for [`COLOUR`]'s reason and because it is the C++ default.
const BOX_PALETTE: BoxPalette = BoxPalette::IceBlue;

/// `kBoxCols`.
const BOX_COLS: usize = 8;
/// `kBoxRows`.
const BOX_ROWS: usize = 8;
/// `kBoxTickSeconds`.
///
/// **One second, and it is why Boxes is the hardest of the five to beat.** The
/// picture is *identical* for thirty consecutive frames, so every one of those
/// frames is two megapixels of the same flat colours — on either processor. A
/// GPU port cannot win by computing less, because there is nothing to compute;
/// it can only win on how fast two megapixels can be written. That is a
/// bandwidth question, and bandwidth on a Pi 4 is shared.
const BOX_TICK_SECONDS: f32 = 1.0;

/// `kBoxIcePalette` — themes.csv row 3, with three entries duplicated to weight
/// the draw, exactly as the C++ has them.
const BOX_ICE: [[u8; 3]; 8] = [
    [255, 255, 255],
    [239, 243, 245],
    [239, 243, 245],
    [223, 231, 235],
    [223, 231, 235],
    [206, 219, 225],
    [206, 219, 225],
    [190, 205, 247],
];

/// `kBoxGreenPalette`.
const BOX_CHARTREUSE: [[u8; 3]; 8] = [
    [0, 0, 0],
    [2, 6, 0],
    [2, 6, 0],
    [10, 28, 0],
    [35, 80, 0],
    [80, 160, 5],
    [145, 230, 60],
    [220, 255, 170],
];

/// `kBoxDarkPalette`.
const BOX_DARK: [[u8; 3]; 8] = [
    [5, 10, 16],
    [5, 10, 16],
    [13, 18, 28],
    [18, 24, 34],
    [20, 27, 37],
    [22, 30, 39],
    [26, 34, 44],
    [29, 37, 48],
];

/// `kBoxPridePalette`, and the same six stops `ss_matrix.hpp` carries. **Six and
/// not eight**, which is why [`Boxes::palette_size`] exists.
const BOX_PRIDE: [[u8; 3]; 6] = [
    [228, 3, 3],
    [255, 140, 0],
    [255, 237, 0],
    [0, 128, 38],
    [0, 77, 255],
    [117, 7, 135],
];

/// [`LIFE_SEED`]'s and [`STAR_SEED`]'s argument.
const BOX_SEED: u32 = 20_260_920;

/// One cell's age and colour index. `background.rs`'s `Cell`, renamed only to
/// keep it distinct from anything else in this file.
#[derive(Clone, Copy, Default)]
struct BoxCell {
    age: i32,
    colour: usize,
}

/// **`BoxesScreensaver` — the simulation only.**
///
/// Every second each age is decremented, and a cell that was already at zero
/// snaps to a new colour and a fresh age: **no interpolation, an instant pop**,
/// which `background.rs` names as the whole character of the effect and the one
/// thing it would be tempting to smooth. Nothing here smooths it.
///
/// 64 cells once a second is not a workload on any processor; the entire cost of
/// this background, on either side, is painting two megapixels of flat colour.
pub struct Boxes {
    cells: [BoxCell; BOX_COLS * BOX_ROWS],
    tick_accumulator: f32,
    /// **Integer division, as the C++ has it.** At 1920x1080 both divide
    /// exactly; at a size that does not, the C++ leaves a strip uncovered down
    /// the right and along the bottom — and so does the shader, which is why it
    /// works in pixels rather than in `uv`. See [`BOXES_FRAGMENT_SRC`].
    cell_w: usize,
    /// See [`Boxes::cell_w`].
    cell_h: usize,
    rng: Mt19937,
    /// Reported at the end of the run, for [`Starfield::respawns`]'s reason: at
    /// [`BOX_TICK_SECONDS`] a twenty-second run should report about twenty.
    ticks: u64,
}

impl Boxes {
    /// [`Starfield::new`]'s note on taking the surface size once applies here
    /// too, and more strongly: `cell_w` is a truncating division and recomputing
    /// it per frame would be pretending it could change.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let mut boxes = Self {
            cells: [BoxCell::default(); BOX_COLS * BOX_ROWS],
            tick_accumulator: 0.0,
            cell_w: width / BOX_COLS,
            cell_h: height / BOX_ROWS,
            rng: Mt19937::new(BOX_SEED),
            ticks: 0,
        };
        boxes.reset();
        boxes
    }

    /// `BornThisWay` has six stops and the other three have eight. It reaches
    /// the rng as the upper bound of every colour draw, so getting it wrong
    /// would index past [`BOX_PRIDE`] rather than merely look different.
    const fn palette_size() -> i32 {
        match BOX_PALETTE {
            BoxPalette::BornThisWay => BOX_PRIDE.len() as i32,
            _ => BOX_ICE.len() as i32,
        }
    }

    fn colour(index: usize) -> [u8; 3] {
        match BOX_PALETTE {
            BoxPalette::IceBlue => BOX_ICE[index],
            BoxPalette::Chartreuse => BOX_CHARTREUSE[index],
            BoxPalette::Dark => BOX_DARK[index],
            BoxPalette::BornThisWay => BOX_PRIDE[index],
        }
    }

    /// `rng.int(1, 15)` and `rng.int(0, size - 1)`, `background.rs`'s values.
    fn reset(&mut self) {
        let size = Self::palette_size();
        for i in 0..BOX_COLS * BOX_ROWS {
            self.cells[i].age = self.rng.int(1, 15);
            self.cells[i].colour = self.rng.int(0, size - 1) as usize;
        }
    }

    /// `background.rs`'s `Boxes::update`, unchanged — a `while` and not an `if`,
    /// so a frame that took longer than a tick still ticks once per tick.
    pub fn update(&mut self, dt: f32) {
        self.tick_accumulator += dt;
        while self.tick_accumulator >= BOX_TICK_SECONDS {
            self.tick_accumulator -= BOX_TICK_SECONDS;
            let size = Self::palette_size();
            for i in 0..BOX_COLS * BOX_ROWS {
                if self.cells[i].age > 0 {
                    self.cells[i].age -= 1;
                } else {
                    self.cells[i].age = self.rng.int(1, 15);
                    self.cells[i].colour = self.rng.int(0, size - 1) as usize;
                }
            }
            self.ticks += 1;
        }
    }

    /// The grid as 8x8 `GL_RGB` texels — **192 bytes**, the smallest upload in
    /// this file by a factor of twenty-seven.
    ///
    /// There is no quantisation to argue about here, unlike [`Field::bytes`] and
    /// [`Life::texels`]: the palettes are already `[u8; 3]`, so this is a copy.
    fn texels(&self) -> [u8; BOX_COLS * BOX_ROWS * 3] {
        let mut out = [0u8; BOX_COLS * BOX_ROWS * 3];
        for (i, texel) in out.chunks_exact_mut(3).enumerate() {
            texel.copy_from_slice(&Self::colour(self.cells[i].colour));
        }
        out
    }

    /// See [`Boxes::ticks`].
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} tick(s) on the CPU — one a second, so a twenty-second run should say about \
             twenty",
            self.ticks
        )
    }
}

// ---- what a measurement is driving --------------------------------------

/// The CPU-side state one background needs.
///
/// **Four shapes and not one**, because the families genuinely differ:
/// Contours and Aurora are one drifting [`Field`] at two rates, Life is an
/// automaton with no rate at all, [`Starfield`] is a list of points with no grid
/// at all, and [`Boxes`] is a grid so coarse it is a palette index per eighth of
/// the screen. Making `Which::rate()` return a number three of them would
/// ignore, and `Field` carry cells three of them would not use, would be a
/// pretence that they are the same thing.
///
/// **The variants are lopsided and that is fine.** [`Field`] holds its 5,184
/// cells inline — 20 KB — [`Starfield`] holds 320 stars inline, and [`Life`]
/// holds four `Vec`s, so the enum is the size of the largest. `measure`
/// constructs exactly one and drops it, so the difference is 20 KB of stack once
/// per background; boxing it would buy a pointer chase in a program whose whole
/// output is a timing number.
#[allow(clippy::large_enum_variant)]
pub enum Scene {
    Field(Field),
    Life(Life),
    Stars(Starfield),
    Boxes(Boxes),
}

impl Scene {
    /// **`width` and `height` are the compositor's surface**, and two of the
    /// four need them at construction: [`Starfield`]'s vanishing point and
    /// [`Boxes`]' truncating cell division are both fixed for the life of the
    /// background in `designed.rs` and `background.rs`, and making them
    /// per-frame here would be a change to what is being measured.
    #[must_use]
    pub fn new(which: Which, width: usize, height: usize) -> Self {
        match which {
            Which::Contours | Which::Aurora => Self::Field(Field::new(which.rate())),
            Which::Life => Self::Life(Life::new()),
            Which::Starfield => Self::Stars(Starfield::new(width, height)),
            Which::Boxes => Self::Boxes(Boxes::new(width, height)),
        }
    }

    /// Advance by wall-clock `dt`, as the pack's host does.
    pub fn update(&mut self, dt: f32) {
        match self {
            Self::Field(field) => field.update(dt),
            Self::Life(life) => life.update(dt),
            Self::Stars(stars) => stars.update(dt),
            Self::Boxes(boxes) => boxes.update(dt),
        }
    }

    /// What the journal says this background is, before it starts.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Field(_) => "96x54 field as a texture",
            Self::Life(_) => {
                "160x90 board as a texture, Pulse; the automaton stays on the CPU"
            }
            // **The one line in this file that says Starfield is not a
            // texture**, because it is the thing a reader of the report will
            // want to know before reading the number.
            Self::Stars(_) => {
                "320 stars as GL_POINTS, not a texture; the motion stays on the CPU"
            }
            Self::Boxes(_) => "8x8 grid as a texture; two megapixels of flat colour",
        }
    }

    /// Anything the background wants said after the run — the *"cheap because
    /// the GPU is fast"* versus *"cheap because nothing happened"* check. Only
    /// the field pair has nothing to say, because a field that stopped drifting
    /// would cost exactly what a drifting one does.
    #[must_use]
    pub fn summary(&self) -> Option<String> {
        match self {
            Self::Field(_) => None,
            Self::Life(life) => Some(life.summary()),
            Self::Stars(stars) => Some(stars.summary()),
            Self::Boxes(boxes) => Some(boxes.summary()),
        }
    }
}

// ---- GLES 2.0 -------------------------------------------------------------

const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_STATIC_DRAW: u32 = 0x88E4;
/// **Starfield's buffer, and the hint is not decoration.** 320 vertices are
/// rewritten every frame; telling the driver so is what lets it orphan the old
/// storage instead of stalling until the previous frame has finished reading it.
const GL_DYNAMIC_DRAW: u32 = 0x88E8;
const GL_FLOAT: u32 = 0x1406;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
/// **Starfield's primitive.** See the module note for why it is not a
/// full-screen quad like the other four.
const GL_POINTS: u32 = 0x0000;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE0: u32 = 0x84C0;
const GL_TEXTURE1: u32 = 0x84C1;
const GL_RGB: u32 = 0x1907;
const GL_LUMINANCE: u32 = 0x1909;
/// Two bytes a texel, `(L, L, L, A)` out of `texture2D`. **Life's board needs
/// two independent numbers per cell** — see the module note — and this is the
/// only two-channel format ES 2.0 has without an extension.
const GL_LUMINANCE_ALPHA: u32 = 0x190A;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
/// **Life's filter, and the one place this file departs from Contours.** A
/// cell is either there or it is not; interpolating between two of them draws
/// something that is neither.
const GL_NEAREST: i32 = 0x2600;
const GL_LINEAR: i32 = 0x2601;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;

#[link(name = "GLESv2")]
unsafe extern "C" {
    fn glCreateShader(kind: u32) -> u32;
    fn glShaderSource(shader: u32, count: i32, string: *const *const c_char, length: *const i32);
    fn glCompileShader(shader: u32);
    fn glGetShaderiv(shader: u32, pname: u32, params: *mut i32);
    fn glGetShaderInfoLog(shader: u32, max: i32, length: *mut i32, log: *mut c_char);
    fn glCreateProgram() -> u32;
    fn glAttachShader(program: u32, shader: u32);
    fn glLinkProgram(program: u32);
    fn glGetProgramiv(program: u32, pname: u32, params: *mut i32);
    fn glGetProgramInfoLog(program: u32, max: i32, length: *mut i32, log: *mut c_char);
    fn glUseProgram(program: u32);
    fn glGetAttribLocation(program: u32, name: *const c_char) -> i32;
    fn glGetUniformLocation(program: u32, name: *const c_char) -> i32;
    fn glUniform1f(location: i32, v: f32);
    fn glUniform1i(location: i32, v: i32);
    fn glUniform2f(location: i32, a: f32, b: f32);
    fn glUniform3f(location: i32, a: f32, b: f32, c: f32);
    fn glGenBuffers(n: i32, buffers: *mut u32);
    fn glBindBuffer(target: u32, buffer: u32);
    fn glBufferData(target: u32, size: isize, data: *const c_void, usage: u32);
    fn glEnableVertexAttribArray(index: u32);
    fn glDisableVertexAttribArray(index: u32);
    fn glVertexAttribPointer(
        index: u32,
        size: i32,
        kind: u32,
        normalized: u8,
        stride: i32,
        pointer: *const c_void,
    );
    fn glGenTextures(n: i32, textures: *mut u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glActiveTexture(unit: u32);
    fn glTexImage2D(
        target: u32,
        level: i32,
        internal: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        kind: u32,
        pixels: *const c_void,
    );
    fn glTexParameteri(target: u32, pname: u32, param: i32);
    fn glPixelStorei(pname: u32, param: i32);
    fn glDrawArrays(mode: u32, first: i32, count: i32);
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
    fn glFinish();
}

/// A full-screen triangle strip in clip space, with the field's texture
/// coordinates alongside.
const QUAD: [f32; 16] = [
    -1.0, -1.0, 0.0, 1.0, //
    1.0, -1.0, 1.0, 1.0, //
    -1.0, 1.0, 0.0, 0.0, //
    1.0, 1.0, 1.0, 0.0,
];

const VERTEX_SRC: &str = "attribute vec2 position;\n\
                          attribute vec2 uv_in;\n\
                          varying vec2 uv;\n\
                          void main() {\n\
                          \x20 uv = uv_in;\n\
                          \x20 gl_Position = vec4(position, 0.0, 1.0);\n\
                          }\n";

/// **The whole of Contours, in eight lines.**
///
/// `texture2D` with `GL_LINEAR` is the bilinear interpolation the CPU version
/// spends `AxisMap` and a run loop on. `fwidth(v)` is the local gradient its
/// longest comment is about. The rest is the same compare and the same
/// coverage ramp.
///
/// `GL_OES_standard_derivatives` is what `fwidth` needs in ES 2.0. It is
/// present on the V3D; a machine without it is refused by name at compile
/// rather than drawing something subtly wrong.
const FRAGMENT_SRC: &str = "#extension GL_OES_standard_derivatives : require\n\
                            precision highp float;\n\
                            varying vec2 uv;\n\
                            uniform sampler2D field;\n\
                            uniform float levels;\n\
                            uniform vec3 line_colour;\n\
                            uniform float half_width;\n\
                            void main() {\n\
                            \x20 float v = texture2D(field, uv).r * levels;\n\
                            \x20 float distance = abs(v - floor(v + 0.5));\n\
                            \x20 float threshold = half_width * fwidth(v);\n\
                            \x20 float coverage = 1.0 - min(distance / max(threshold, 1e-6), 1.0);\n\
                            \x20 gl_FragColor = vec4(line_colour * coverage, 1.0);\n\
                            }\n";

/// **Aurora's shader, and it is shorter than Contours'.**
///
/// Sample the field, then use that as the coordinate into a 256x1 ramp. Two
/// texture fetches and nothing else — `designed.rs` spends `build_ramp`, a
/// `[[u8; 4]; 256]` table and a per-pixel index on exactly this.
const AURORA_FRAGMENT_SRC: &str = "precision highp float;\n\
                                   varying vec2 uv;\n\
                                   uniform sampler2D field;\n\
                                   uniform sampler2D ramp;\n\
                                   void main() {\n\
                                   \x20 float v = texture2D(field, uv).r;\n\
                                   \x20 gl_FragColor = vec4(texture2D(ramp, vec2(v, 0.5)).rgb, 1.0);\n\
                                   }\n";

/// **Life's cells, and it is Aurora's shader plus a modulo.**
///
/// `uv * grid` puts the fragment in cell coordinates. `floor` is which cell it
/// is in and the remainder is where it is *within* that cell, which is the
/// whole of the shape. One `texture2D` at the cell centre carries both of the
/// cell's numbers: `.r` is the heat and `.a` is the radius fraction.
///
/// **The centre sample is deliberate.** With `GL_NEAREST` a fetch at `uv`
/// would land on the same texel, but only because of how the driver rounds at
/// a cell boundary; `(cell + 0.5) / grid` is the cell centre by construction
/// and does not depend on that.
///
/// `clamp(radius - distance + 0.5, 0, 1)` is `rivet_sdk::draw::disc_coverage`
/// transcribed — `(0.5 - (distance - radius)).clamp(0, 1)` — so the edge is
/// the same exact-coverage edge the CPU version draws rather than a
/// `smoothstep` that would be close but not the same.
///
/// Both offsets are scaled into **pixels** by `cell_px` before the distance is
/// taken, because `designed.rs` takes its radius from
/// `cell_w.min(cell_h) * 0.46` — a circle in pixel space, which stays a circle
/// on a surface whose cells are not square.
///
/// **This one shader draws `Dots` as well as `Pulse`**: `Dots` is a constant
/// in the alpha channel and `Pulse` is `pulse_fraction`, so the difference
/// between the two styles never reaches the GPU.
const LIFE_FRAGMENT_SRC: &str = "precision highp float;\n\
                                 varying vec2 uv;\n\
                                 uniform sampler2D board;\n\
                                 uniform vec2 grid;\n\
                                 uniform vec2 cell_px;\n\
                                 uniform float full_radius;\n\
                                 uniform vec3 cell_colour;\n\
                                 void main() {\n\
                                 \x20 vec2 scaled = uv * grid;\n\
                                 \x20 vec2 cell = floor(scaled);\n\
                                 \x20 vec4 texel = texture2D(board, (cell + 0.5) / grid);\n\
                                 \x20 vec2 offset = (scaled - cell - 0.5) * cell_px;\n\
                                 \x20 float radius = texel.a * full_radius;\n\
                                 \x20 float coverage = clamp(radius - length(offset) + 0.5, 0.0, 1.0);\n\
                                 \x20 gl_FragColor = vec4(cell_colour * (texel.r * coverage), 1.0);\n\
                                 }\n";

/// **`Squares` has no shape at all**, which is the clearest statement of what
/// moving to the GPU did to this background: a nearest-filtered fetch *is* the
/// tiling that `designed.rs` spends four truncated bounds and a `draw::fill`
/// on, and the colour is the heat.
///
/// Kept for completeness and because it costs six lines; [`LIFE_STYLE`] is
/// what decides which of the two is compiled.
const LIFE_SQUARES_FRAGMENT_SRC: &str = "precision highp float;\n\
                                         varying vec2 uv;\n\
                                         uniform sampler2D board;\n\
                                         uniform vec3 cell_colour;\n\
                                         void main() {\n\
                                         \x20 gl_FragColor = vec4(cell_colour * texture2D(board, uv).r, 1.0);\n\
                                         }\n";

/// **Starfield's vertex shader — the only one in this file that does any work.**
///
/// Three things, none of which the fragment stage could do:
///
/// * **`gl_PointSize`**, which must be written from the vertex stage in ES 2.0
///   and is what makes `designed.rs`'s *"two pixels for a near star past
///   halfway"* a per-star quantity rather than a global one.
/// * **the pixel-to-clip transform.** The star list is in `designed.rs`'s own
///   pixel space, so the constants in [`Starfield`] stay the constants
///   `designed.rs` wrote. `viewport` is the surface, not a baked 1920x1080.
/// * **the Y flip**, `1.0 - y / height * 2.0`. Row 0 is the TOP on the canvas
///   side and clip `+1` is the top here, which is the same convention [`QUAD`]
///   encodes for the four textured backgrounds. Getting it wrong would draw a
///   field that is correct in every respect and upside down.
///
/// `brightness` rides through as a `varying`. A point primitive has one vertex,
/// so there is nothing to interpolate and every fragment of a star gets that
/// star's value exactly.
const STARFIELD_VERTEX_SRC: &str = "attribute vec2 star;\n\
                                    attribute float point_size;\n\
                                    attribute float brightness;\n\
                                    uniform vec2 viewport;\n\
                                    varying float lit;\n\
                                    void main() {\n\
                                    \x20 lit = brightness;\n\
                                    \x20 gl_PointSize = point_size;\n\
                                    \x20 gl_Position = vec4(star.x / viewport.x * 2.0 - 1.0,\n\
                                    \x20                    1.0 - star.y / viewport.y * 2.0,\n\
                                    \x20                    0.0, 1.0);\n\
                                    }\n";

/// **Starfield's fragment shader, and it is one multiply.**
///
/// `designed.rs` computes `scale(colour, brightness)` on the CPU — three
/// integer multiplies and three truncations to `u8` per star — and its own
/// comment says that is *"correct only over a black ground, so it is used only
/// over one"*. That is still true here, and the clear is still black, so
/// `colour * lit` is the same blend.
///
/// **It runs about four hundred times a frame**, against Contours' two million.
/// Nothing about this shader is on the critical path; it is here so that the
/// measurement has a shader at all.
const STARFIELD_FRAGMENT_SRC: &str = "precision highp float;\n\
                                      varying float lit;\n\
                                      uniform vec3 star_colour;\n\
                                      void main() {\n\
                                      \x20 gl_FragColor = vec4(star_colour * lit, 1.0);\n\
                                      }\n";

/// **Boxes, and it is the shortest shader here by design** — see the module
/// note on why its number is a calibration for the other four.
///
/// # Why it works in pixels and not in `uv`
///
/// `background.rs` derives `cell_w = width / BOX_COLS` by **integer division**,
/// and says so: *"at a size that does not divide, the C++ leaves a strip of
/// background uncovered down the right and along the bottom, and so does this"*.
/// `floor(uv * grid)` would spread eight cells across the full width instead,
/// covering that strip and making every cell a fraction of a pixel wider. So the
/// fragment recovers its pixel with `uv * viewport`, divides by the **truncated**
/// `cell_px` the CPU computed, and lands in the same cell the CPU version would
/// have filled.
///
/// `step(cell, grid - 1.0)` is that strip: a fragment past the last whole cell
/// gets column or row 8, `step` gives it `0.0`, and it stays the clear colour.
/// **Branchless on purpose** — at 1920x1080 the division is exact and the strip
/// is empty, so a branch would be uniformly not-taken and would still leave a
/// reader of the number wondering whether it was. `min(cell, grid - 1.0)` keeps
/// the fetch in range so the discarded fragments cannot sample off the end.
///
/// The one thing that is NOT faithful: the CPU version leaves that strip as
/// whatever was underneath, and here it is black. Nothing in this probe draws
/// underneath, so the two agree wherever the question is askable.
const BOXES_FRAGMENT_SRC: &str = "precision highp float;\n\
                                  varying vec2 uv;\n\
                                  uniform sampler2D cells;\n\
                                  uniform vec2 grid;\n\
                                  uniform vec2 cell_px;\n\
                                  uniform vec2 viewport;\n\
                                  void main() {\n\
                                  \x20 vec2 cell = floor(uv * viewport / cell_px);\n\
                                  \x20 vec2 inside = step(cell, grid - 1.0);\n\
                                  \x20 vec2 at = (min(cell, grid - 1.0) + 0.5) / grid;\n\
                                  \x20 vec3 rgb = texture2D(cells, at).rgb;\n\
                                  \x20 gl_FragColor = vec4(rgb * inside.x * inside.y, 1.0);\n\
                                  }\n";

/// Which of the five backgrounds a [`Gpu`] is drawing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Contours,
    Aurora,
    Life,
    Starfield,
    Boxes,
}

impl Which {
    /// The name the report prints, matching `CATALOGUE`'s `background/theme`
    /// ids so the two measurements can be put beside each other.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Contours => "contours/ice-blue",
            Self::Aurora => "aurora/green-violet",
            Self::Life => "life/pulse",
            Self::Starfield => "starfield/ice-blue",
            Self::Boxes => "boxes/ice-blue",
        }
    }

    /// How fast its field drifts. Only Contours and Aurora have a [`Field`]; it
    /// is [`Scene`] and not this that decides what a background carries, so this
    /// is only ever asked of the two that have an answer. The other three are
    /// listed rather than swept into a `_` arm, which would also swallow a sixth
    /// background added later.
    pub const fn rate(self) -> f32 {
        match self {
            Self::Contours => CONTOUR_RATE,
            Self::Aurora | Self::Life | Self::Starfield | Self::Boxes => AURORA_RATE,
        }
    }

    /// Whether this background draws points instead of a full-screen quad.
    /// One predicate, consulted in three places, so that the answer cannot
    /// drift between them.
    const fn is_points(self) -> bool {
        matches!(self, Self::Starfield)
    }
}

/// Everything the GPU holds for one field background.
pub struct Gpu {
    which: Which,
    program: u32,
    texture: u32,
    /// Aurora's 256x1 colour ramp. Unused by Contours, which has one colour.
    ramp: u32,
    u_levels: i32,
    u_colour: i32,
    u_half_width: i32,
    u_field: i32,
    u_ramp: i32,
    /// Life's four. A location of `-1` — which is what every one of these is
    /// in the Contours and Aurora programs — makes `glUniform*` a defined
    /// no-op, so nothing has to be conditional on the way in.
    u_board: i32,
    /// Shared between Life and Boxes: both are a cell count, and a uniform's
    /// location is per-program, so one field serves whichever program is
    /// linked.
    u_grid: i32,
    /// Shared between Life and Boxes — see [`Gpu::u_grid`]. Life's is the cell
    /// size in pixels so a disc stays a circle; Boxes' is the **truncated**
    /// cell size, which is a different quantity with the same units and the
    /// same name on both sides.
    u_cell_px: i32,
    u_full_radius: i32,
    u_cell_colour: i32,
    /// Boxes' 8x8 palette sampler.
    u_cells: i32,
    /// The surface, in pixels. Starfield's vertex shader needs it to reach clip
    /// space and Boxes' fragment shader needs it to reach a pixel; neither can
    /// bake 1920x1080 in, because the size is the compositor's to choose.
    u_viewport: i32,
    u_star_colour: i32,
    /// Starfield's vertex buffer, rewritten every frame. `0` for every other
    /// background, which never binds it.
    stars: u32,
}

impl Gpu {
    /// Compile, link and set up the one quad and the one texture.
    ///
    /// # Errors
    ///
    /// The compiler's or linker's own log, verbatim. A shader that will not
    /// build on this driver is the most likely failure here and the least
    /// guessable, so nothing is summarised.
    pub fn new(which: Which) -> Result<Self, String> {
        // SAFETY: a context is current; every call below is ordinary GLES.
        unsafe {
            let vertex = compile(
                GL_VERTEX_SHADER,
                if which.is_points() { STARFIELD_VERTEX_SRC } else { VERTEX_SRC },
            )?;
            let fragment = compile(
                GL_FRAGMENT_SHADER,
                match which {
                    Which::Contours => FRAGMENT_SRC,
                    Which::Aurora => AURORA_FRAGMENT_SRC,
                    Which::Life => match LIFE_STYLE {
                        LifeStyle::Squares => LIFE_SQUARES_FRAGMENT_SRC,
                        LifeStyle::Dots | LifeStyle::Pulse => LIFE_FRAGMENT_SRC,
                    },
                    Which::Starfield => STARFIELD_FRAGMENT_SRC,
                    Which::Boxes => BOXES_FRAGMENT_SRC,
                },
            )?;
            let program = glCreateProgram();
            glAttachShader(program, vertex);
            glAttachShader(program, fragment);
            glLinkProgram(program);
            let mut ok: i32 = 0;
            glGetProgramiv(program, GL_LINK_STATUS, &raw mut ok);
            if ok == 0 {
                return Err(format!("the program would not link: {}", program_log(program)));
            }
            glUseProgram(program);

            // **Every attribute array off before any is turned on.** Five
            // `Gpu`s are built in sequence in one context and nothing here is
            // ever deleted, so a slot the *previous* background enabled is
            // still enabled when this one links. Contours enables two and
            // Starfield three; if the linker did not hand out the same
            // locations, a stale array would be fed to a program that never
            // declared it. ES 2.0 guarantees at least eight slots.
            for slot in 0..8 {
                glDisableVertexAttribArray(slot);
            }

            let stride = 4 * std::mem::size_of::<f32>() as i32;
            let float = std::mem::size_of::<f32>();
            let mut buffer: u32 = 0;
            glGenBuffers(1, &raw mut buffer);
            glBindBuffer(GL_ARRAY_BUFFER, buffer);
            let mut stars: u32 = 0;
            if which.is_points() {
                // **Allocated once at full size and orphaned per frame.** The
                // contents are `glBufferData`'d again in `draw`; this call is
                // what fixes the storage the attribute pointers below refer to.
                stars = buffer;
                glBufferData(
                    GL_ARRAY_BUFFER,
                    (STAR_COUNT * 4 * float) as isize,
                    std::ptr::null(),
                    GL_DYNAMIC_DRAW,
                );
                // **Named, not `.max(0)`-ed.** A missing attribute here would
                // otherwise silently alias slot 0 with `star` and draw 320
                // points on top of each other, which looks like a fast frame.
                let star = glGetAttribLocation(program, c"star".as_ptr());
                let point_size = glGetAttribLocation(program, c"point_size".as_ptr());
                let brightness = glGetAttribLocation(program, c"brightness".as_ptr());
                if star < 0 || point_size < 0 || brightness < 0 {
                    return Err(format!(
                        "the starfield program dropped an attribute: star {star}, \
                         point_size {point_size}, brightness {brightness}"
                    ));
                }
                glEnableVertexAttribArray(star as u32);
                glVertexAttribPointer(star as u32, 2, GL_FLOAT, 0, stride, std::ptr::null());
                glEnableVertexAttribArray(point_size as u32);
                glVertexAttribPointer(
                    point_size as u32,
                    1,
                    GL_FLOAT,
                    0,
                    stride,
                    (2 * float) as *const c_void,
                );
                glEnableVertexAttribArray(brightness as u32);
                glVertexAttribPointer(
                    brightness as u32,
                    1,
                    GL_FLOAT,
                    0,
                    stride,
                    (3 * float) as *const c_void,
                );
            } else {
                glBufferData(
                    GL_ARRAY_BUFFER,
                    std::mem::size_of_val(&QUAD) as isize,
                    QUAD.as_ptr().cast(),
                    GL_STATIC_DRAW,
                );
                let position = glGetAttribLocation(program, c"position".as_ptr()).max(0) as u32;
                let uv_in = glGetAttribLocation(program, c"uv_in".as_ptr()).max(0) as u32;
                glEnableVertexAttribArray(position);
                glVertexAttribPointer(position, 2, GL_FLOAT, 0, stride, std::ptr::null());
                glEnableVertexAttribArray(uv_in);
                glVertexAttribPointer(
                    uv_in,
                    2,
                    GL_FLOAT,
                    0,
                    stride,
                    (2 * float) as *const c_void,
                );
            }

            // **Starfield is the one background with no texture at all**, which
            // is the clearest statement of how differently it is shaped: there
            // is no grid to upload, only 320 vertices. `texture` stays `0`, and
            // `draw`'s unconditional bind of it is a defined no-op.
            let mut texture: u32 = 0;
            if !which.is_points() {
                glGenTextures(1, &raw mut texture);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, texture);
                // **`GL_LINEAR` is the bilinear interpolation**, and it is the
                // reason this is cheap: the texture unit does in hardware what
                // the CPU version's `AxisMap` and run loop do by hand.
                //
                // **`GL_NEAREST` for Life, and it is not an oversight.** Its
                // cells are discrete: a filter that mixed a live cell with its
                // dead neighbour would draw a half-alive cell, which is not a
                // state Conway's rule has.
                //
                // **And `GL_NEAREST` for Boxes, for a stronger version of the
                // same reason.** Its cells are 240 pixels wide and its whole
                // character is `background.rs`'s *"no interpolation, an instant
                // pop"*. `GL_LINEAR` would put a 240-pixel gradient between
                // every pair of them and turn a grid of flat rectangles into
                // eight-by-eight bilinear mush — the one background where the
                // wrong filter is not a subtlety.
                let filter = if which == Which::Life || which == Which::Boxes {
                    GL_NEAREST
                } else {
                    GL_LINEAR
                };
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter);
                // Clamp, because the field is a window onto a wave and not a
                // tile; wrapping would put a seam down the right edge. Life's
                // board is a torus and would be entitled to `GL_REPEAT`, but it
                // is only ever sampled at a cell centre inside `[0, 1]`, so the
                // two are the same and this stays one line rather than two.
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                // 96 bytes a row is not a multiple of 4 in the general case; a
                // field whose width changes would tear diagonally without this.
                // Boxes needs it for the same reason at 24 bytes a row.
                glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            }

            // **Aurora's ramp, uploaded once.** It never changes, unlike the
            // field, so it is a `glTexImage2D` at start-up and a second
            // sampler per fragment thereafter. `GL_LINEAR` here interpolates
            // BETWEEN ramp entries, which is smoother than the CPU version's
            // integer index and costs nothing.
            let mut ramp: u32 = 0;
            if which == Which::Aurora {
                glGenTextures(1, &raw mut ramp);
                glActiveTexture(GL_TEXTURE1);
                glBindTexture(GL_TEXTURE_2D, ramp);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                let bytes = aurora_ramp();
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGB as i32,
                    256,
                    1,
                    0,
                    GL_RGB,
                    GL_UNSIGNED_BYTE,
                    bytes.as_ptr().cast(),
                );
            }

            Ok(Self {
                which,
                program,
                texture,
                ramp,
                u_levels: glGetUniformLocation(program, c"levels".as_ptr()),
                u_colour: glGetUniformLocation(program, c"line_colour".as_ptr()),
                u_half_width: glGetUniformLocation(program, c"half_width".as_ptr()),
                u_field: glGetUniformLocation(program, c"field".as_ptr()),
                u_ramp: glGetUniformLocation(program, c"ramp".as_ptr()),
                u_board: glGetUniformLocation(program, c"board".as_ptr()),
                u_grid: glGetUniformLocation(program, c"grid".as_ptr()),
                u_cell_px: glGetUniformLocation(program, c"cell_px".as_ptr()),
                u_full_radius: glGetUniformLocation(program, c"full_radius".as_ptr()),
                u_cell_colour: glGetUniformLocation(program, c"cell_colour".as_ptr()),
                u_cells: glGetUniformLocation(program, c"cells".as_ptr()),
                u_viewport: glGetUniformLocation(program, c"viewport".as_ptr()),
                u_star_colour: glGetUniformLocation(program, c"star_colour".as_ptr()),
                stars,
            })
        }
    }

    /// Upload this frame's texture and draw one frame.
    ///
    /// **The clear stays**, even for Life, whose shader writes every fragment
    /// and strictly does not need one. It is a tile-based renderer's cheapest
    /// hint that the previous frame need not be loaded back in, and removing
    /// it for one background would make that background's number the answer to
    /// a slightly different question than the other two's.
    #[allow(clippy::cast_precision_loss)]
    pub fn draw(&self, scene: &Scene, width: usize, height: usize) {
        // SAFETY: a context is current and every object below is this
        // struct's, created in `new`.
        unsafe {
            glViewport(0, 0, width as i32, height as i32);
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glUseProgram(self.program);
            glActiveTexture(GL_TEXTURE0);
            // `self.texture` is `0` for Starfield, which never made one; binding
            // the default texture object is a defined no-op and keeps this line
            // out of the match.
            glBindTexture(GL_TEXTURE_2D, self.texture);
            // **How many points to draw, decided inside the match**, because
            // Starfield is the only background whose primitive count varies
            // from frame to frame — a star that has left the rectangle is a
            // vertex that is not in the buffer. `-1` means *"not a point
            // background"*, and the four that are not fall through to the quad.
            let mut points: i32 = -1;
            match scene {
                Scene::Field(field) => {
                    let bytes = field.bytes();
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_LUMINANCE as i32,
                        FIELD_W as i32,
                        FIELD_H as i32,
                        0,
                        GL_LUMINANCE,
                        GL_UNSIGNED_BYTE,
                        bytes.as_ptr().cast(),
                    );
                    glUniform1i(self.u_field, 0);
                    match self.which {
                        Which::Contours => {
                            glUniform1f(self.u_levels, LEVELS);
                            glUniform1f(self.u_half_width, CONTOUR_HALF_WIDTH);
                            glUniform3f(self.u_colour, COLOUR[0], COLOUR[1], COLOUR[2]);
                        }
                        Which::Aurora => {
                            glActiveTexture(GL_TEXTURE1);
                            glBindTexture(GL_TEXTURE_2D, self.ramp);
                            glUniform1i(self.u_ramp, 1);
                            // Back to unit 0, so the next frame's field upload
                            // lands on the field and not on the ramp.
                            glActiveTexture(GL_TEXTURE0);
                        }
                        // Unreachable by construction: `Scene::new` pairs each
                        // of these with its own `Scene` variant. Said here
                        // rather than swallowed by a `_` arm, which would also
                        // swallow a sixth background added later.
                        Which::Life | Which::Starfield | Which::Boxes => {}
                    }
                }
                Scene::Life(life) => {
                    let texels = life.texels();
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_LUMINANCE_ALPHA as i32,
                        LIFE_COLS as i32,
                        LIFE_ROWS as i32,
                        0,
                        GL_LUMINANCE_ALPHA,
                        GL_UNSIGNED_BYTE,
                        texels.as_ptr().cast(),
                    );
                    glUniform1i(self.u_board, 0);
                    // **Per frame and not once**, because the surface size is
                    // the compositor's to choose and a probe that baked 1080p
                    // in would draw ellipses on anything else.
                    let cell_w = width as f32 / LIFE_COLS as f32;
                    let cell_h = height as f32 / LIFE_ROWS as f32;
                    glUniform2f(self.u_grid, LIFE_COLS as f32, LIFE_ROWS as f32);
                    glUniform2f(self.u_cell_px, cell_w, cell_h);
                    // `cell_w.min(cell_h) * 0.46`, from `Life::draw`. Computed
                    // here rather than in the shader so that the constant
                    // stays beside the citation that justifies it.
                    glUniform1f(self.u_full_radius, cell_w.min(cell_h) * LIFE_FULL_RADIUS);
                    glUniform3f(
                        self.u_cell_colour,
                        LIFE_COLOUR[0],
                        LIFE_COLOUR[1],
                        LIFE_COLOUR[2],
                    );
                }
                Scene::Stars(field) => {
                    // **5 KB a frame, and the whole of the CPU-to-GPU traffic
                    // for this background.** Contours uploads 5,184 bytes and
                    // Life 28,800; Starfield's is smaller than either and is
                    // 320 vertices rather than a grid.
                    let (vertices, count) = field.vertices();
                    glBindBuffer(GL_ARRAY_BUFFER, self.stars);
                    // `glBufferData` and not `glBufferSubData`: respecifying the
                    // whole store lets the driver orphan the copy the previous
                    // frame may still be reading instead of stalling on it.
                    // Always the full array, so the size never changes and the
                    // orphan is a straight swap; only `count` of it is drawn.
                    glBufferData(
                        GL_ARRAY_BUFFER,
                        std::mem::size_of_val(&vertices) as isize,
                        vertices.as_ptr().cast(),
                        GL_DYNAMIC_DRAW,
                    );
                    glUniform2f(self.u_viewport, width as f32, height as f32);
                    glUniform3f(
                        self.u_star_colour,
                        STAR_COLOUR[0],
                        STAR_COLOUR[1],
                        STAR_COLOUR[2],
                    );
                    points = count as i32;
                }
                Scene::Boxes(boxes) => {
                    let texels = boxes.texels();
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_RGB as i32,
                        BOX_COLS as i32,
                        BOX_ROWS as i32,
                        0,
                        GL_RGB,
                        GL_UNSIGNED_BYTE,
                        texels.as_ptr().cast(),
                    );
                    glUniform1i(self.u_cells, 0);
                    glUniform2f(self.u_grid, BOX_COLS as f32, BOX_ROWS as f32);
                    // **The TRUNCATED cell size**, computed once by `Boxes::new`
                    // and passed through unchanged — see `BOXES_FRAGMENT_SRC`
                    // for why the shader cannot just divide `uv` by eight.
                    glUniform2f(self.u_cell_px, boxes.cell_w as f32, boxes.cell_h as f32);
                    glUniform2f(self.u_viewport, width as f32, height as f32);
                }
            }
            if points >= 0 {
                glDrawArrays(GL_POINTS, 0, points);
            } else {
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            }
            // **Here on purpose**, as in `main.rs`: without it a driver that
            // defers everything reports a frame rate that is not a frame rate.
            glFinish();
        }
    }
}

/// Compile one shader, or return the driver's log.
unsafe fn compile(kind: u32, source: &str) -> Result<u32, String> {
    unsafe {
        let shader = glCreateShader(kind);
        let bytes = source.as_bytes();
        let ptr = bytes.as_ptr().cast::<c_char>();
        let len = bytes.len() as i32;
        glShaderSource(shader, 1, &raw const ptr, &raw const len);
        glCompileShader(shader);
        let mut ok: i32 = 0;
        glGetShaderiv(shader, GL_COMPILE_STATUS, &raw mut ok);
        if ok == 0 {
            let which = if kind == GL_VERTEX_SHADER { "vertex" } else { "fragment" };
            return Err(format!("the {which} shader would not compile: {}", shader_log(shader)));
        }
        Ok(shader)
    }
}

// **`u8` and not `i8`, because `c_char` is unsigned on `aarch64`.** A buffer
// typed as `i8` compiles on the development machine and not on the target,
// which is the sort of difference that only a cross build reports.
unsafe fn shader_log(shader: u32) -> String {
    unsafe {
        let mut buffer = [0u8; 1024];
        let mut length: i32 = 0;
        glGetShaderInfoLog(shader, 1024, &raw mut length, buffer.as_mut_ptr().cast::<c_char>());
        text(&buffer, length)
    }
}

unsafe fn program_log(program: u32) -> String {
    unsafe {
        let mut buffer = [0u8; 1024];
        let mut length: i32 = 0;
        glGetProgramInfoLog(program, 1024, &raw mut length, buffer.as_mut_ptr().cast::<c_char>());
        text(&buffer, length)
    }
}

fn text(buffer: &[u8], length: i32) -> String {
    let n = usize::try_from(length).unwrap_or(0).min(buffer.len());
    String::from_utf8_lossy(&buffer[..n]).trim().to_string()
}

/// Silence the unused-import warning for the C types the `extern` block needs.
const _: Option<c_int> = None;
const _: Option<c_uint> = None;
