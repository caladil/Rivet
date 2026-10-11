//! **The three PORTED backgrounds — Digital Rain, Boxes and Solid.**
//!
//! The pack has seven. The other four are in `crate::designed`, and the
//! reason for the split has a section of its own below.
//!
//! **Moved here constant for constant from
//! `examples/aura/aura-renderer/src/background.rs`**,
//! `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T6 / `.rulings/` R-199 D1.
//! Every constant, every struct and every line of drawing logic below is
//! copied unchanged from the renderer's copy — the renderer is the authority
//! this move copies FROM, the same way the C++ was the authority the
//! renderer's own port copied from (see the next paragraph, also unchanged).
//! `evidence.rs` in this crate proves the copy is exact: it renders the same
//! ten `(background, theme)` pairs at the same four frame indices from the
//! same seed and checks every hash against
//! `examples/aura/aura-renderer/evidence/T6-background-baseline/manifest.json`,
//! which was captured from the renderer **before** this move.
//!
//! **What changed on the way across is the *caller*, not the *code*.**
//! `crate::render::render_frame` is new glue in this crate that hands
//! [`Screensaver::draw`] a canvas exactly as `aura-renderer`'s own `main.rs`
//! does, then copies the finished picture into a
//! `rivetos_addon::frame::Surface` — `rivet-addon/src/frame.rs`'s boundary,
//! which did not exist when this file was first written. Nothing about that
//! boundary is a reason to touch the numbers below.
//!
//! `cpp/src/main.cpp` line 413 is commented `// Background / screensaver` and
//! the switch under it is the first thing drawn on every frame; the cards are
//! composited over whatever it left. There is no separate screensaver mode and
//! no blanking: Aura is a wall panel and something is always on it.
//!
//! # This is a PORT and there is no design in it
//!
//! Ruled by the maintainer, 2026-09-01: the screensavers leave no creative room
//! at all, and the port is pixel-exact (`.rulings/` R-125). Every constant
//! below is lifted from `cpp/src/ss_matrix.hpp`, `cpp/src/ss_boxes.hpp` and
//! `cpp/src/ss_solid.hpp` verbatim — cell sizes, decay steps, speed ranges,
//! spawn probabilities, tick periods, palette channel values, the lot. Where
//! the C++ picks a number this file has that number, and where the C++ does
//! something arguably wrong this file does the same thing and says so beside
//! it.
//!
//! # What could not be reproduced exactly, stated rather than hidden
//!
//! 1. **Glyph antialiasing.** raylib rasterises through `stb_truetype` into a
//!    texture atlas; this draws through `fontdue`. The outlines and the sizes
//!    are identical (see `crate::typeface`), the edge coverage is not, and no
//!    amount of care makes two rasterisers agree at the boundary.
//! 2. **The random sequence.** Both C++ screensavers seed from
//!    `std::random_device`, so no two runs of Aura itself match either.
//!    `crate::rng` is the same generator and the same ranges; see its note on
//!    what "the same distribution" can and cannot mean across two standard
//!    libraries.
//!
//! Everything else in this file is the same arithmetic on the same numbers.
//!
//! # The four that are NOT a port live next door
//!
//! T8 added Starfield, Aurora, Contours and Conway's Game of Life, and they
//! are in `crate::designed` rather than here **precisely so that the previous
//! section keeps being true of this file**. Every number below is answerable
//! to a line of C++; every number in that file was chosen in this repository.
//! A reader who cannot tell the two apart will eventually defend one of the
//! new ones as though the port required it.
//!
//! What they take from this file is only the dispatch: four variants on
//! [`Background`], four arms on [`Background::name`], and one
//! `Option<crate::designed::Designed>` on [`Screensaver`]. The T6 baseline
//! comparison in `evidence.rs` is what proves that dispatch cost the existing
//! ten rows nothing — it still hashes the same forty frames to the same forty
//! values.

use rivet_sdk::draw;
use rivet_sdk::text::Target;

use crate::designed::{AuroraPalette, ContourPalette, Designed, LifeStyle, StarPalette};
use crate::rng::Mt19937;
use crate::typeface::Typeface;

/// Which background is drawn, and in which palette.
///
/// **`allow(dead_code)` on the palettes is deliberate.** A pack describes
/// every background and theme it has (`Addon::describe`'s `backgrounds`) and
/// draws whichever one a `Frame` names — unlike the renderer this was ported
/// from, which only ever constructed the one its `BACKGROUND` constant named.
/// Every variant here is reachable through `crate::CATALOGUE` and
/// `evidence.rs`, even though no single test happens to construct all ten in
/// one place the way this lint would want to see.
///
/// The names are the maintainer's ruling of 2026-09-01, carried in
/// `crate::CATALOGUE`: **Digital Rain**, never `matrix` and never
/// `construct`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    /// `ss_solid.hpp`. Not animated; the screen goes static between changes.
    Solid(Solid),
    /// `ss_matrix.hpp`, renamed. Animated.
    DigitalRain(RainPalette),
    /// `ss_boxes.hpp`. Animated.
    Boxes(BoxPalette),
    /// T8, and **no C++ behind it** — `crate::designed`. Animated.
    Starfield(StarPalette),
    /// T8, drawn by `crate::designed`. Animated.
    Aurora(AuroraPalette),
    /// T8, drawn by `crate::designed`. Animated.
    Contours(ContourPalette),
    /// T8, drawn by `crate::designed`. Animated.
    Life(LifeStyle),
}

impl Background {
    /// `isAnimated()`. Kept from the renderer's port; this crate does not
    /// currently read it (a pack has no redraw-on-change optimisation of its
    /// own to gate — the host asks for a frame and this pack draws one), but
    /// it costs nothing to keep and a host built later may want it.
    #[allow(dead_code)]
    #[must_use]
    pub const fn is_animated(&self) -> bool {
        !matches!(self, Self::Solid(_))
    }

    /// What an operator would call it, for the journal. Kept from the
    /// renderer's port; this crate has no journal call site of its own yet
    /// (`.rulings/` `Journal::for_service` is a host's, and this pack is not
    /// one) — `T7`'s host wiring is the reader `crate::catalogue`'s ids
    /// already anticipate.
    #[allow(dead_code)]
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Solid(Solid::Black) => "Solid / Black",
            Self::Solid(Solid::White) => "Solid / White",
            Self::Solid(Solid::Navy) => "Solid / Navy",
            Self::DigitalRain(RainPalette::PhosphorGreen) => "Digital Rain / Phosphor Green",
            Self::DigitalRain(RainPalette::IceBlue) => "Digital Rain / Ice Blue",
            Self::DigitalRain(RainPalette::BornThisWay) => "Digital Rain / Born This Way",
            Self::Boxes(BoxPalette::IceBlue) => "Boxes / Ice Blue",
            Self::Boxes(BoxPalette::Chartreuse) => "Boxes / Chartreuse",
            Self::Boxes(BoxPalette::Dark) => "Boxes / Dark",
            Self::Boxes(BoxPalette::BornThisWay) => "Boxes / Born This Way",
            Self::Starfield(StarPalette::White) => "Starfield / White",
            Self::Starfield(StarPalette::Amber) => "Starfield / Amber",
            Self::Starfield(StarPalette::IceBlue) => "Starfield / Ice Blue",
            Self::Aurora(AuroraPalette::GreenViolet) => "Aurora / Green Violet",
            Self::Aurora(AuroraPalette::DeepBlue) => "Aurora / Deep Blue",
            Self::Aurora(AuroraPalette::Ember) => "Aurora / Ember",
            Self::Contours(ContourPalette::PhosphorGreen) => "Contours / Phosphor Green",
            Self::Contours(ContourPalette::IceBlue) => "Contours / Ice Blue",
            Self::Contours(ContourPalette::Ember) => "Contours / Ember",
            // **The long name here too.** `crate::catalogue` carries the
            // argument for it; this is the journal's copy of the same string
            // and the two disagreeing would be worse than either being wrong.
            Self::Life(LifeStyle::Squares) => "Conway's Game of Life / Squares",
            Self::Life(LifeStyle::Dots) => "Conway's Game of Life / Dots",
            Self::Life(LifeStyle::Pulse) => "Conway's Game of Life / Pulse",
        }
    }
}

// --- Solid -----------------------------------------------------------------

/// `SolidScreensaver`'s named presets — `ss_solid.hpp`.
///
/// **`Warm` is deliberately absent.** `ss_solid.hpp` defines it, and
/// `ParseScreensaver` in `cpp/src/main.cpp` has no string that reaches it: the
/// preset exists in the header and is unreachable from the product. It is
/// therefore not part of Aura's appearance and porting it would be inventing a
/// background rather than copying one. Reported to the maintainer rather than
/// decided here — carried forward unchanged from the renderer's own port.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solid {
    /// `SolidScreensaver{}` — the default `ss_mode` in `main.cpp`.
    Black,
    White,
    Navy,
}

impl Solid {
    /// The fill. `BLACK`, `WHITE`, and `{10, 18, 40}` from `ss_solid.hpp`.
    #[must_use]
    pub const fn colour(self) -> [u8; 3] {
        match self {
            Self::Black => [0, 0, 0],
            Self::White => [255, 255, 255],
            Self::Navy => [10, 18, 40],
        }
    }
}

// --- Boxes -----------------------------------------------------------------

#[allow(dead_code)]
/// `BoxesPalette` — `ss_boxes.hpp`, under the ruled names.
///
/// `Chartreuse` is `BoxesPalette::Green`; `crate::CATALOGUE` carries the
/// argument for the name, which is that the ramp's red climbs while its blue
/// stays near zero and the C++ file's own comment already says *bright
/// chartreuse*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxPalette {
    /// The default in `main.cpp`'s `ss_boxes{BoxesPalette::IceBlue}`.
    IceBlue,
    Chartreuse,
    Dark,
    BornThisWay,
}

/// `kBoxCols`.
const BOX_COLS: usize = 8;
/// `kBoxRows`.
const BOX_ROWS: usize = 8;
/// `kBoxTickSeconds`.
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

/// `kBoxPridePalette`, and the same six stops `ss_matrix.hpp` and
/// `DrawRainbowCard` carry.
const PRIDE: [[u8; 3]; 6] = [
    [228, 3, 3],
    [255, 140, 0],
    [255, 237, 0],
    [0, 128, 38],
    [0, 77, 255],
    [117, 7, 135],
];

/// `BoxesScreensaver`.
///
/// One cell has an age and a colour. Every second each age is decremented, and
/// a cell that was already at zero snaps to a new colour and a fresh age —
/// **no interpolation, an instant pop**, which is the whole character of the
/// effect and the one thing that would be tempting to smooth.
struct Boxes {
    palette: BoxPalette,
    cells: [Cell; BOX_COLS * BOX_ROWS],
    tick_accumulator: f32,
    cell_w: usize,
    cell_h: usize,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    age: i32,
    colour: usize,
}

impl Boxes {
    fn new(palette: BoxPalette, width: usize, height: usize, rng: &mut Mt19937) -> Self {
        let mut boxes = Self {
            palette,
            cells: [Cell::default(); BOX_COLS * BOX_ROWS],
            tick_accumulator: 0.0,
            // Integer division, as the C++ has it. At 1920x1080 both divide
            // exactly; at a size that does not, the C++ leaves a strip of
            // background uncovered down the right and along the bottom, and so
            // does this.
            cell_w: width / BOX_COLS,
            cell_h: height / BOX_ROWS,
        };
        boxes.reset(rng);
        boxes
    }

    fn palette_size(&self) -> i32 {
        if self.palette == BoxPalette::BornThisWay {
            PRIDE.len() as i32
        } else {
            BOX_ICE.len() as i32
        }
    }

    /// **This frame's palette as an 8x8 `GL_RGB` texture**, for the GPU path.
    ///
    /// 192 bytes — the smallest upload of the three GPU backgrounds by two
    /// orders of magnitude, because Boxes' whole state *is* sixty-four colour
    /// indices. The shader turns it into two million pixels; the CPU path
    /// spends sixty-four `draw::fill`s of about 32,400 pixels each doing the
    /// same thing.
    ///
    /// `cell_w`/`cell_h` go across separately and **truncated**, because that
    /// is how [`Boxes::new`] computes them and the unpainted strip at the
    /// right and bottom of a surface that does not divide by eight is part of
    /// what this background looks like.
    fn uploaded<'a>(&self, scratch: &'a mut Vec<u8>) -> crate::gpu::Uploaded<'a> {
        let want = BOX_COLS * BOX_ROWS * 3;
        if scratch.len() != want {
            scratch.clear();
            scratch.resize(want, 0);
        }
        for (i, texel) in scratch.chunks_exact_mut(3).enumerate() {
            texel.copy_from_slice(&self.colour(self.cells[i].colour));
        }
        #[allow(clippy::cast_precision_loss)]
        crate::gpu::Uploaded::Boxes {
            cols: BOX_COLS as f32,
            rows: BOX_ROWS as f32,
            cell_px: (self.cell_w as f32, self.cell_h as f32),
            cells: scratch,
        }
    }

    fn colour(&self, index: usize) -> [u8; 3] {
        match self.palette {
            BoxPalette::IceBlue => BOX_ICE[index],
            BoxPalette::Chartreuse => BOX_CHARTREUSE[index],
            BoxPalette::Dark => BOX_DARK[index],
            BoxPalette::BornThisWay => PRIDE[index],
        }
    }

    fn reset(&mut self, rng: &mut Mt19937) {
        let size = self.palette_size();
        for cell in &mut self.cells {
            cell.age = rng.int(1, 15);
            cell.colour = rng.int(0, size - 1) as usize;
        }
    }

    fn update(&mut self, dt: f32, rng: &mut Mt19937) {
        self.tick_accumulator += dt;
        while self.tick_accumulator >= BOX_TICK_SECONDS {
            self.tick_accumulator -= BOX_TICK_SECONDS;
            let size = self.palette_size();
            for cell in &mut self.cells {
                if cell.age > 0 {
                    cell.age -= 1;
                } else {
                    cell.age = rng.int(1, 15);
                    cell.colour = rng.int(0, size - 1) as usize;
                }
            }
        }
    }

    fn draw(&self, canvas: &mut [u8], t: &Target) {
        for i in 0..BOX_COLS * BOX_ROWS {
            let col = i % BOX_COLS;
            let row = i / BOX_COLS;
            draw::fill(
                canvas,
                t,
                col * self.cell_w,
                row * self.cell_h,
                self.cell_w,
                self.cell_h,
                self.colour(self.cells[i].colour),
            );
        }
    }
}

// --- Digital Rain ----------------------------------------------------------

#[allow(dead_code)]
/// `ConstructPalette` — `ss_matrix.hpp`, under the ruled names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RainPalette {
    /// `ConstructPalette::Green`, and the default in `theme::BACKGROUND_PALETTES`.
    PhosphorGreen,
    IceBlue,
    BornThisWay,
}

/// `kCellW`.
const CELL_W: usize = 18;
/// `kCellH`. Also the font size every glyph is drawn at, which is why the atlas
/// and the cell cannot drift apart.
const CELL_H: usize = 18;
/// `kDecayPerFrame`.
const DECAY_PER_FRAME: u8 = 6;

/// The character pool — `ss_matrix.hpp`'s constructor.
///
/// **The comment in the C++ says "digits 0–9 + 43 katakana" and the code says
/// more than that**: it pushes the digits, then `'A'..='Z'`, then `'a'..='z'`,
/// then the katakana. 105 codepoints. The code is what runs, so the code is what
/// is ported, and this is exactly the kind of place where following the prose
/// would have quietly changed the picture.
///
/// All 105 are drawn from **IPAGothic**, not from Source Code Pro. The C++
/// builds one atlas out of `IPAGothic.ttf` for the whole pool, so the digits and
/// Latin letters in the rain are IPAGothic's shapes.
const KATAKANA: [char; 43] = [
    '\u{30A1}', '\u{30A2}', '\u{30A3}', '\u{30A4}', '\u{30A5}', '\u{30A6}', '\u{30A7}', '\u{30A8}',
    '\u{30A9}', '\u{30AA}', '\u{30AB}', '\u{30AC}', '\u{30AD}', '\u{30AE}', '\u{30AF}', '\u{30B1}',
    '\u{30B3}', '\u{30B5}', '\u{30B7}', '\u{30B9}', '\u{30BB}', '\u{30BD}', '\u{30BF}', '\u{30C1}',
    '\u{30C4}', '\u{30C6}', '\u{30C8}', '\u{30CA}', '\u{30CB}', '\u{30CC}', '\u{30CF}', '\u{30D2}',
    '\u{30D5}', '\u{30D8}', '\u{30DB}', '\u{30DE}', '\u{30DF}', '\u{30E0}', '\u{30E4}', '\u{30E6}',
    '\u{30E8}', '\u{30EF}', '\u{30F3}',
];

/// The pool, built in the C++'s order so an index means the same thing.
fn character_pool() -> Vec<char> {
    let mut pool: Vec<char> = "0123456789".chars().collect();
    pool.extend('A'..='Z');
    pool.extend('a'..='z');
    pool.extend(KATAKANA);
    pool
}

/// One falling column's head — `ConstructScreensaver::Column`.
#[derive(Clone, Copy, Default)]
struct Column {
    row: i32,
    frac: f32,
    speed: f32,
    delay: f32,
}

struct DigitalRain {
    palette: RainPalette,
    cols: usize,
    rows: usize,
    brightness: Vec<u8>,
    chars: Vec<char>,
    columns: Vec<Column>,
    pool: Vec<char>,
    head: [u8; 3],
    trail: [u8; 3],
    dim: [u8; 3],
}

impl DigitalRain {
    fn new(palette: RainPalette, width: usize, height: usize, rng: &mut Mt19937) -> Self {
        let cols = width / CELL_W;
        let rows = height / CELL_H;
        // `set_palette`.
        let (head, trail, dim) = match palette {
            RainPalette::PhosphorGreen => ([180, 255, 180], [0, 200, 50], [0, 100, 20]),
            RainPalette::IceBlue => ([239, 243, 245], [190, 205, 247], [40, 55, 110]),
            RainPalette::BornThisWay => ([255, 255, 255], [255, 255, 255], [0, 0, 0]),
        };
        let mut rain = Self {
            palette,
            cols,
            rows,
            brightness: vec![0; rows * cols],
            // `chars_.assign(rows*cols, (int)'0')`.
            chars: vec!['0'; rows * cols],
            columns: vec![Column::default(); cols],
            pool: character_pool(),
            head,
            trail,
            dim,
        };
        for c in 0..cols {
            rain.reset_column(c, true, rng);
        }
        rain
    }

    /// `reset_column`.
    ///
    /// The two starts differ and the difference is the effect's first second:
    /// at `init` a column starts somewhere in the top half of the screen already
    /// falling (`row_dist(-(rows/2), -1)`), and on every wrap it starts a whole
    /// screen height above the top (`-(uniform_int(1, rows))`), so the rain does
    /// not restart in lockstep.
    fn reset_column(&mut self, c: usize, randomise_start: bool, rng: &mut Mt19937) {
        let speed = rng.real(4.0, 14.0);
        let delay = rng.real(0.0, 6.0);
        let row = if randomise_start {
            rng.int(-(self.rows as i32 / 2), -1)
        } else {
            -rng.int(1, self.rows as i32)
        };
        // The draws happen in the C++'s order — speed, delay, then the row —
        // because `columns_[c] = { row, 0, speed, delay }` evaluates its
        // initialiser list left to right but the two distributions above it are
        // constructed first. Order is what keeps the *rate* of consumption the
        // same even though the sequence cannot be.
        self.columns[c] = Column {
            row,
            frac: 0.0,
            speed,
            delay,
        };
    }

    fn random_char(&self, rng: &mut Mt19937) -> char {
        self.pool[rng.int(0, self.pool.len() as i32 - 1) as usize]
    }

    fn update(&mut self, dt: f32, rng: &mut Mt19937) {
        // `b -= kDecayPerFrame`, floored at zero. Per FRAME and not per second:
        // the C++ decays by a fixed step regardless of `dt`, so the trail length
        // is tied to the frame rate rather than to time. Ported as it is —
        // making it time-based would change how long a trail looks at 30 fps,
        // which is the rate this has run at on a wall for months.
        for b in &mut self.brightness {
            if *b > 0 {
                *b = b.saturating_sub(DECAY_PER_FRAME);
            }
        }

        // A lit cell has a 4% chance per frame of becoming a different glyph,
        // which is the flicker down the length of a trail.
        for i in 0..self.chars.len() {
            if self.brightness[i] > 80 && rng.real(0.0, 1.0) < 0.04 {
                self.chars[i] = self.random_char(rng);
            }
        }

        for c in 0..self.cols {
            if self.columns[c].delay > 0.0 {
                self.columns[c].delay -= dt;
                continue;
            }
            self.columns[c].frac += self.columns[c].speed * dt;
            while self.columns[c].frac >= 1.0 {
                self.columns[c].frac -= 1.0;
                self.columns[c].row += 1;

                let row = self.columns[c].row;
                if row >= 0 && row < self.rows as i32 {
                    let index = row as usize * self.cols + c;
                    self.brightness[index] = 255;
                    self.chars[index] = self.random_char(rng);
                    // **`row >= rows` is compared as a SIGNED integer**, and
                    // that is not pedantry. The C++'s `col.row` is an `int` and
                    // spends most of its life negative — a column that has
                    // wrapped starts a whole screen height above the top. This
                    // was first written as `row as usize >= self.rows`, which
                    // makes every negative row a number near `usize::MAX` and
                    // therefore larger than `rows`: **every column above the
                    // screen reset itself on every step and almost none ever
                    // fell.** The rain drew four columns out of a hundred and
                    // six, and it looked deliberate.
                } else if row >= self.rows as i32 {
                    // **The reset happens inside the `while`**, so the fresh
                    // negative row is then stepped by whatever `frac` is left —
                    // and the fresh `delay` is NOT consulted until the next
                    // frame, because the check is above the loop. Both are
                    // faithful to the C++ and both are visible: a fast column
                    // that wraps mid-frame loses a row off its new start.
                    self.reset_column(c, false, rng);
                }
            }
        }
    }

    fn draw(&self, canvas: &mut [u8], t: &Target, face: &Typeface) {
        // `DrawRectangle(0, 0, screen_w_, screen_h_, BLACK)` — the rain paints
        // its own ground, every frame, whatever the theme's background is.
        draw::clear(canvas, t, [0, 0, 0]);

        let pride = self.palette == RainPalette::BornThisWay;
        for r in 0..self.rows {
            for c in 0..self.cols {
                let b = self.brightness[r * self.cols + c];
                if b == 0 {
                    continue;
                }
                // In Born This Way every column is one of the six stops, head
                // and trail alike, fading to black.
                let (head, trail, dim) = if pride {
                    (PRIDE[c % 6], PRIDE[c % 6], [0, 0, 0])
                } else {
                    (self.head, self.trail, self.dim)
                };
                let colour = if b > 200 {
                    head
                } else if b > 80 {
                    lerp(dim, trail, f32::from(b - 80) / 120.0)
                } else {
                    lerp([0, 0, 0], dim, f32::from(b) / 80.0)
                };
                // `col.a = b` — the brightness IS the opacity, on top of the
                // colour ramp. Both, not either: the ramp takes the glyph from
                // black through the dim colour to the trail colour, and the
                // alpha then fades the whole thing out against the ground.
                let mut buffer = [0u8; 4];
                face.draw(
                    canvas,
                    t,
                    (c * CELL_W) as f32,
                    (r * CELL_H) as f32,
                    self.chars[r * self.cols + c].encode_utf8(&mut buffer),
                    CELL_H as f32,
                    colour,
                    b,
                );
            }
        }
    }
}

/// `lerp_color` from `ss_matrix.hpp`, including its truncation.
///
/// The C++ computes `(unsigned char)(a + (b - a) * t)` with the subtraction in
/// `int`, so a downward ramp works and the result is truncated rather than
/// rounded. Rounding here would lift every trail pixel by up to one level,
/// which is invisible alone and is exactly the sort of drift that makes two
/// photographs of the same screen not match.
fn lerp(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| -> u8 { (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8 };
    [channel(a[0], b[0]), channel(a[1], b[1]), channel(a[2], b[2])]
}

// --- the one a caller holds ------------------------------------------------

/// Whichever background is running, updated and drawn through one door.
pub struct Screensaver {
    which: Background,
    rng: Mt19937,
    boxes: Option<Boxes>,
    rain: Option<DigitalRain>,
    /// **The four T8 added, behind one field.** The two above each have their
    /// own `Option` because that is the shape the renderer had and this file
    /// does not get rearranged on the way across; the four that arrived
    /// together are an enum in `crate::designed`, which is what they are.
    designed: Option<Designed>,
}

impl Screensaver {
    /// Build the one background this application is configured for.
    ///
    /// **Clock-seeded**, for production draws where there is no "again" to
    /// compare against — see [`Self::new_with_rng`] for the seam a test uses
    /// instead.
    #[must_use]
    pub fn new(which: Background, width: usize, height: usize) -> Self {
        Self::new_with_rng(which, width, height, Mt19937::from_the_clock())
    }

    /// The same construction as [`Self::new`], with the generator taken as an
    /// **argument** instead of read from the clock.
    ///
    /// **Why this pack still needs it, unchanged from the renderer's own
    /// reason.** `evidence.rs` has to capture a frame of every background and
    /// theme from the pack and compare it, byte for byte, against
    /// `examples/aura/aura-renderer/evidence/T6-background-baseline/`, which
    /// was captured the same way. `new`'s `Mt19937::from_the_clock()` makes
    /// that comparison meaningless on its own — Boxes' starting ages and
    /// colours and Digital Rain's column speeds and delays are drawn from it,
    /// so two calls to `new` seconds apart paint two different pictures and a
    /// mismatched hash would prove nothing about the port.
    #[must_use]
    pub fn new_with_rng(which: Background, width: usize, height: usize, mut rng: Mt19937) -> Self {
        let (boxes, rain) = match which {
            Background::Boxes(palette) => (Some(Boxes::new(palette, width, height, &mut rng)), None),
            Background::DigitalRain(palette) => {
                (None, Some(DigitalRain::new(palette, width, height, &mut rng)))
            }
            // Solid draws from a constant, and the four T8 added construct
            // through `Designed::for_background` below — which answers `None`
            // for exactly the three arms this `match` handles.
            Background::Solid(_)
            | Background::Starfield(_)
            | Background::Aurora(_)
            | Background::Contours(_)
            | Background::Life(_) => (None, None),
        };
        let designed = Designed::for_background(which, width, height, &mut rng);
        Self {
            which,
            rng,
            boxes,
            rain,
            designed,
        }
    }

    /// Kept from the renderer's port. This crate's own `Drawing::draw` keys
    /// its cache on the id strings it was asked for rather than reading this
    /// back off the `Screensaver` it built, so nothing here calls it yet.
    #[allow(dead_code)]
    #[must_use]
    pub const fn which(&self) -> Background {
        self.which
    }

    /// Advance by `dt` seconds.
    ///
    /// The caller clamps `dt` to `[0.001, 0.1]` before this is reached — a
    /// long stall must not teleport every column down the screen. In this
    /// crate that clamp lives in `crate::render`, which derives `dt` from two
    /// consecutive frames' `time_ms`; the renderer this was ported from did
    /// the same clamp against `Instant::now()` deltas in its own frame loop.
    pub fn update(&mut self, dt: f32) {
        if let Some(boxes) = &mut self.boxes {
            boxes.update(dt, &mut self.rng);
        }
        if let Some(rain) = &mut self.rain {
            rain.update(dt, &mut self.rng);
        }
        if let Some(designed) = &mut self.designed {
            designed.update(dt, &mut self.rng);
        }
    }

    /// Paint the whole screen.
    pub fn draw(&self, canvas: &mut [u8], t: &Target, rain_face: &Typeface) {
        match self.which {
            Background::Solid(solid) => {
                draw::clear(canvas, t, solid.colour());
            }
            Background::Boxes(_) => {
                if let Some(boxes) = &self.boxes {
                    boxes.draw(canvas, t);
                }
            }
            Background::DigitalRain(_) => {
                if let Some(rain) = &self.rain {
                    rain.draw(canvas, t, rain_face);
                }
            }
            // None of the four takes a `rain_face`: Digital Rain is the only
            // background in this pack that draws a glyph.
            Background::Starfield(_)
            | Background::Aurora(_)
            | Background::Contours(_)
            | Background::Life(_) => {
                if let Some(designed) = &self.designed {
                    designed.draw(canvas, t);
                }
            }
        }
    }

    /// **This frame's state packed for the GPU**, or [`None`] for a background
    /// with no GPU path.
    ///
    /// R-207's three are Contours, Life and Boxes. Solid, Digital Rain,
    /// Starfield and Aurora answer `None` — and each for its own reason, which
    /// [`crate::designed::Designed::uploaded`] and the catalogue's `drawn`
    /// field both record. Solid's is the plainest: a `glClear` would do it,
    /// and a background that is one colour has nothing to move off the CPU.
    ///
    /// The caller owns `scratch` so that the texture bytes are built once into
    /// a buffer that survives between frames.
    pub fn uploaded<'a>(&self, scratch: &'a mut Vec<u8>) -> Option<crate::gpu::Uploaded<'a>> {
        match self.which {
            Background::Boxes(_) => self.boxes.as_ref().map(|boxes| boxes.uploaded(scratch)),
            Background::Starfield(_)
            | Background::Aurora(_)
            | Background::Contours(_)
            | Background::Life(_) => {
                self.designed.as_ref().and_then(|d| d.uploaded(scratch))
            }
            Background::Solid(_) | Background::DigitalRain(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The pool is 105 codepoints in the C++'s order, and the comment in
    /// `ss_matrix.hpp` that says 53 of them are not there is wrong.
    #[test]
    fn the_character_pool_is_the_one_the_code_builds_not_the_one_the_comment_describes() {
        let pool = character_pool();
        assert_eq!(pool.len(), 10 + 26 + 26 + 43);
        assert_eq!(pool[0], '0');
        assert_eq!(pool[9], '9');
        assert_eq!(pool[10], 'A');
        assert_eq!(pool[35], 'Z');
        assert_eq!(pool[36], 'a');
        assert_eq!(pool[61], 'z');
        assert_eq!(pool[62], '\u{30A1}');
        assert_eq!(pool[104], '\u{30F3}');
    }

    /// The three brightness bands and their exact boundaries. 201 is head, 200
    /// is the top of the trail ramp, 81 is its bottom, 80 is the top of the
    /// fade-in. An off-by-one here changes the look of every trail on screen.
    #[test]
    fn the_brightness_ramp_has_the_bands_the_cpp_has() {
        let head = [180u8, 255, 180];
        let trail = [0u8, 200, 50];
        let dim = [0u8, 100, 20];
        assert_eq!(lerp(dim, trail, f32::from(200u8 - 80) / 120.0), trail);
        assert_eq!(lerp(dim, trail, f32::from(81u8 - 80) / 120.0), [0, 100, 20]);
        assert_eq!(lerp([0, 0, 0], dim, f32::from(80u8) / 80.0), dim);
        assert_eq!(lerp([0, 0, 0], dim, 0.0), [0, 0, 0]);
        // The head band is a flat colour, not a ramp.
        assert_eq!(head, [180, 255, 180]);
    }

    /// Boxes' geometry is integer division of the screen by eight, and at
    /// 1920x1080 it is exact. The check is worth having because a cell size
    /// that rounded would leave a seam down the middle of the screen.
    #[test]
    fn boxes_tile_a_1080p_screen_exactly() {
        let mut rng = Mt19937::new(1);
        let boxes = Boxes::new(BoxPalette::IceBlue, 1920, 1080, &mut rng);
        assert_eq!(boxes.cell_w * BOX_COLS, 1920);
        assert_eq!(boxes.cell_h * BOX_ROWS, 1080);
    }

    /// Every cell starts with an age in `[1, 15]` and a colour that indexes the
    /// palette it was given. Born This Way has six entries and the others eight;
    /// an index off the end would panic on the first frame.
    #[test]
    fn every_box_starts_inside_its_palette() {
        for palette in [
            BoxPalette::IceBlue,
            BoxPalette::Chartreuse,
            BoxPalette::Dark,
            BoxPalette::BornThisWay,
        ] {
            let mut rng = Mt19937::new(99);
            let mut boxes = Boxes::new(palette, 1920, 1080, &mut rng);
            for cell in boxes.cells {
                assert!((1..=15).contains(&cell.age));
                assert!((cell.colour as i32) < boxes.palette_size());
            }
            // And after enough ticks that every cell has certainly rolled over.
            boxes.update(60.0, &mut rng);
            for cell in boxes.cells {
                assert!((0..=15).contains(&cell.age));
                assert!((cell.colour as i32) < boxes.palette_size());
            }
        }
    }

    /// The rain's grid comes out of the screen size, and a column never lights a
    /// cell outside it. The failure this catches is an index computed from a
    /// negative row, which is where the C++'s `col.row` spends most of its life.
    #[test]
    fn the_rain_never_lights_a_cell_outside_its_grid() {
        let mut rng = Mt19937::new(4);
        let mut rain = DigitalRain::new(RainPalette::PhosphorGreen, 1920, 1080, &mut rng);
        assert_eq!(rain.cols, 1920 / 18);
        assert_eq!(rain.rows, 1080 / 18);
        // Thirty seconds at thirty frames a second — long enough that every
        // column has started, fallen the height of the screen and wrapped.
        for _ in 0..900 {
            rain.update(1.0 / 30.0, &mut rng);
        }
        assert_eq!(rain.brightness.len(), rain.rows * rain.cols);
        assert!(
            rain.brightness.contains(&255),
            "after thirty seconds nothing is lit, so no column ever fell"
        );
        for column in &rain.columns {
            assert!(
                column.row < rain.rows as i32,
                "a column ran past the bottom without resetting"
            );
        }
    }

    /// **It rains across the whole screen, not down four columns of a hundred.**
    ///
    /// The defect this exists for: `row >= rows` written as `row as usize >=
    /// rows` makes every NEGATIVE row — which is where a wrapped column spends
    /// a third of its life — compare as a number near `usize::MAX`, so the
    /// column resets itself on every step and only ever falls when a reset
    /// happens to draw exactly `-1`. It shipped, it was captured off the board,
    /// and it looked like a design decision rather than a bug.
    ///
    /// A column's cycle is a delay of up to six seconds, a run of one to sixty
    /// rows above the top, then sixty rows down the screen, all at four to
    /// fourteen rows a second — so roughly half of them should have ink at any
    /// moment. A third is a conservative floor that still fails hard on the
    /// defect, which produced about one in twenty-five.
    #[test]
    fn most_columns_are_raining_at_any_given_moment() {
        let mut rng = Mt19937::new(20_260_902);
        let mut rain = DigitalRain::new(RainPalette::PhosphorGreen, 1920, 1080, &mut rng);
        // Twenty seconds in, which is past every start delay and past the
        // first wrap for the faster columns.
        for _ in 0..600 {
            rain.update(1.0 / 30.0, &mut rng);
        }
        let raining = (0..rain.cols)
            .filter(|c| (0..rain.rows).any(|r| rain.brightness[r * rain.cols + c] > 0))
            .count();
        assert!(
            raining * 3 >= rain.cols,
            "only {raining} of {} columns have any ink — the rain is not falling",
            rain.cols
        );
        // And a column above the screen must be ON ITS WAY DOWN rather than
        // being reset in place: none may sit below the bottom edge either.
        assert!(
            rain.columns.iter().all(|c| c.row < rain.rows as i32),
            "a column is past the bottom and did not wrap"
        );
    }

    /// A solid background is not animated and the other two are — which is what
    /// decides whether the loop redraws every frame.
    #[test]
    fn only_the_solid_backgrounds_let_the_screen_go_still() {
        assert!(!Background::Solid(Solid::Black).is_animated());
        assert!(!Background::Solid(Solid::Navy).is_animated());
        assert!(Background::DigitalRain(RainPalette::PhosphorGreen).is_animated());
        assert!(Background::Boxes(BoxPalette::IceBlue).is_animated());
    }
}
