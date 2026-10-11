//! **What a CPU rasteriser costs on this machine** — `SPEC-04` §10.
//!
//! Built to answer one question and to answer it twice: **does a frame fit in
//! the budget, with headroom, before and after the SDK's hot paths are
//! optimised?** It is deliberately an instrument and not a verdict — it reports
//! milliseconds against three budgets and leaves the conclusion to whoever
//! reads it.
//!
//! # Read the phases in this order
//!
//! **The floor comes first, and it is what makes every other number mean
//! something.** `canvas.fill(0)` is the memory-bandwidth cost of touching every
//! pixel once, and a bare `blit` is the cost of getting eight megabytes to the
//! display and doing nothing else. Without those two, *"`fill` improved from 40
//! ms to 6 ms"* is unreadable — nobody can tell whether 6 ms is near-optimal or
//! still five times off. The floor is what says when to stop optimising.
//!
//! Then the primitives, then two whole frames as they will actually occur.
//!
//! # Budgets, not frame rates
//!
//! Every phase is reported against 33.3 ms (30 fps), 25 ms (40 fps) and 16.7 ms
//! (60 fps). The decision this informs is *does this fit*, and a frame that
//! renders in 20 ms against a 33.3 ms budget is finished — which is more useful
//! than being told it reaches fifty frames a second and having to do the
//! arithmetic.
//!
//! # It is uncapped, and the renderer is not
//!
//! `aura-renderer` sleeps whatever is left of its frame budget. This does not,
//! and must not: the raw per-phase cost is the whole measurement. The two
//! programs are separate partly so that neither can accidentally become the
//! other.
//!
//! # What it cannot tell you, stated up front
//!
//! **Tearing.** `rivet_sdk::screen` is one dumb buffer with no page flip and no
//! vblank, so this program writes into the buffer that is being scanned out. A
//! dashboard that repaints once a second will not show it; a full-screen
//! animated background will, and no amount of CPU speed fixes it. That is a
//! platform question (double buffering in the SDK) rather than an Aura one, and
//! it is not something a stopwatch can measure.

use std::time::{Duration, Instant};

use rivet_sdk::draw;
use rivet_sdk::screen::Screen;
use rivet_sdk::text::{Face, Target};

/// How long each phase runs before the harness moves to the next.
///
/// Five seconds is long enough for a phase to be more than a cache artefact and
/// short enough that a full cycle of eleven phases takes about a minute, so a
/// person watching gets a table a minute and throttling is visible within a few
/// of them. **Change it here and rebuild**: an application on a Rivet has no
/// configuration surface at all — no `env` key in a manifest, no argv, no
/// control-plane verb that writes a file it could read. Reported in `SPEC-04`
/// §11 rather than worked around.
const PHASE_SECONDS: u64 = 5;

/// The three budgets every phase is reported against.
const BUDGETS: [(&str, f64); 3] = [("30fps", 33.333), ("40fps", 25.0), ("60fps", 16.667)];

/// What one phase measured.
struct Measurement {
    name: &'static str,
    /// The mean over the first second — the boost-clock figure.
    first_second_ms: f64,
    /// The mean over the whole window, first second excluded when there was
    /// more than one. This is the number that decides anything.
    sustained_ms: f64,
    iterations: u64,
}

impl Measurement {
    /// `yes` when the sustained figure fits a budget, with the margin.
    fn verdict(&self) -> String {
        BUDGETS
            .iter()
            .map(|(name, budget)| {
                if self.sustained_ms <= *budget {
                    format!("{name}:fits({:.0}%)", self.sustained_ms / budget * 100.0)
                } else {
                    format!("{name}:NO({:.1}x)", self.sustained_ms / budget)
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// Run one closure for [`PHASE_SECONDS`], reporting the first second and the
/// rest separately.
///
/// **Both halves are reported rather than averaged**, because a Raspberry Pi 4
/// throttles under sustained drawing and an average hides exactly the thing the
/// screensaver decision depends on: what a frame costs ten minutes in.
fn phase(name: &'static str, mut work: impl FnMut()) -> Measurement {
    // One warm run, so first-touch page faults on an eight-megabyte canvas are
    // not charged to the measurement.
    work();

    let window = Duration::from_secs(PHASE_SECONDS);
    let began = Instant::now();
    let mut first_second_iterations = 0u64;
    let mut first_second_elapsed = Duration::ZERO;
    let mut iterations = 0u64;

    loop {
        let at = Instant::now();
        work();
        let took = at.elapsed();
        iterations += 1;
        let since_start = began.elapsed();
        if since_start <= Duration::from_secs(1) {
            first_second_iterations += 1;
            first_second_elapsed += took;
        }
        if since_start >= window {
            break;
        }
    }

    let total = began.elapsed();
    let first_second_ms = if first_second_iterations == 0 {
        0.0
    } else {
        first_second_elapsed.as_secs_f64() * 1000.0 / first_second_iterations as f64
    };
    let later = iterations.saturating_sub(first_second_iterations);
    let sustained_ms = if later == 0 {
        first_second_ms
    } else {
        (total.as_secs_f64() - first_second_elapsed.as_secs_f64()) * 1000.0 / later as f64
    };
    Measurement {
        name,
        first_second_ms,
        sustained_ms,
        iterations,
    }
}

/// Aura's colours, so the measurement draws what the renderer draws.
const BACKGROUND: [u8; 3] = [0x05, 0x0a, 0x10];
const CARD: [u8; 3] = [0x03, 0x05, 0x0e];
const INK: [u8; 3] = [0xbe, 0xcd, 0xf7];
const MUTED: [u8; 3] = [0x5a, 0x66, 0x8c];

fn main() {
    // **The channel is dropped here, and for this program that is correct.** A
    // probe measures a fixed set of phases and exits; it never draws a second
    // frame, so it has nothing to do with a re-issued display. Every other
    // consumer of `handed()` must keep the channel — see its documentation, and
    // `template-kiosk` for the loop.
    let mut screen = match rivet_sdk::screen::handed() {
        Ok(handed) => Some(handed.screen),
        Err(why) => {
            // The drawing phases still mean something without a display; only
            // the blit does not. Say which is missing and carry on, rather than
            // exiting and leaving nothing measured.
            println!("aura-probe: no display was handed over: {why}");
            println!("aura-probe: measuring the drawing phases only; the blit needs a screen");
            None
        }
    };
    // 1080p when there is no screen to ask, because that is the panel every
    // number in `SPEC-04` §10 is about.
    let (width, height, stride) = screen
        .as_ref()
        .map_or((1920, 1080, 1920 * 4), |s| (s.width, s.height, s.stride));

    let target = Target::xrgb(width, height, stride);
    let face = match Face::load(rivet_sdk::text::SYSTEM_FONT) {
        Ok(face) => face,
        Err(why) => {
            println!("aura-probe: no system font, so the type phases cannot run: {why}");
            return;
        }
    };

    println!(
        "aura-probe: {width}x{height}, stride {stride}, {} bytes a frame, {PHASE_SECONDS}s a phase",
        stride * height
    );
    println!("aura-probe: budgets 30fps=33.3ms 40fps=25.0ms 60fps=16.7ms");

    let mut canvas = vec![0u8; stride * height];
    let started = Instant::now();

    for cycle in 1u64..=u64::MAX {
        let measurements = one_cycle(&mut canvas, &target, &face, screen.as_mut(), width, height);
        report(cycle, started.elapsed(), &measurements);
        if let Some(screen) = screen.as_mut() {
            draw_report(&mut canvas, &target, &face, &measurements, cycle);
            screen.blit(&canvas);
            // A picture of the table, so an agent that cannot stand in front of
            // the machine can still read it (`PLAN.md` §108.5).
            match screen.capture(&canvas) {
                Ok(where_it_went) => println!("aura-probe: captured {where_it_went}"),
                Err(why) => println!("aura-probe: capture failed: {why}"),
            }
        }
    }
}

/// Every phase, once.
fn one_cycle(
    canvas: &mut [u8],
    t: &Target,
    face: &Face,
    mut screen: Option<&mut Screen>,
    width: usize,
    height: usize,
) -> Vec<Measurement> {
    let mut out = Vec::new();

    // ---- The floor ---------------------------------------------------------
    // Touching every pixel once, with no arithmetic. This is the memory
    // bandwidth limit and nothing drawn can beat it.
    out.push(phase("FLOOR memset the canvas", || {
        canvas.fill(0);
        std::hint::black_box(&canvas);
    }));

    // Getting the frame to the display and doing nothing else. On a Pi this is
    // a sequential write to write-combined memory, which is the case WC exists
    // for — and is still eight megabytes.
    if let Some(screen) = screen.as_deref_mut() {
        out.push(phase("FLOOR bare blit, nothing drawn", || {
            screen.blit(canvas);
        }));
    }

    // ---- The primitives ----------------------------------------------------
    out.push(phase("draw::fill full screen", || {
        draw::fill(canvas, t, 0, 0, width, height, BACKGROUND);
    }));
    out.push(phase("draw::dim full screen", || {
        draw::dim(canvas, t, 128);
    }));

    let card = card_rect(width, height);
    out.push(phase("draw::fill_rounded one card", || {
        draw::fill_rounded(canvas, t, card.0, card.1, card.2, card.3, 28, CARD);
    }));
    out.push(phase("draw::stroke_rounded one card", || {
        draw::stroke_rounded(canvas, t, card.0, card.1, card.2, card.3, 28, 3, INK);
    }));
    out.push(phase("text: one card's worth", || {
        card_text(canvas, t, face, card);
    }));
    out.push(phase("text: one glyph at 292px", || {
        face.draw(canvas, t, 100, 400, "8", 292.0, INK);
    }));

    // ---- The two backgrounds, as they actually are -------------------------
    // Neither uses a shader and neither ever did: `cpp/assets/shaders/` is
    // empty and nothing in `cpp/src` names one. Both are 2D primitives and
    // text, which is why they map onto the phases above.
    out.push(phase("background: Boxes, one frame", || {
        boxes_frame(canvas, t, width, height);
    }));
    out.push(phase("background: Digital Rain, one frame", || {
        digital_rain_frame(canvas, t, face, width, height);
    }));

    // ---- Whole frames ------------------------------------------------------
    // **The number that decides anything.** The screensaver is the BACKGROUND,
    // not an alternative to the dashboard — `cpp/src/main.cpp` draws it and
    // then composites the cards on top every frame. So a realistic frame is a
    // full-screen animated background plus four cards, and "the cards are
    // static and therefore cheap" is false.
    out.push(phase("FRAME solid background + 4 cards", || {
        draw::fill(canvas, t, 0, 0, width, height, BACKGROUND);
        four_cards(canvas, t, face, width, height);
    }));
    out.push(phase("FRAME Boxes background + 4 cards", || {
        boxes_frame(canvas, t, width, height);
        four_cards(canvas, t, face, width, height);
    }));
    out.push(phase("FRAME Digital Rain + 4 cards", || {
        digital_rain_frame(canvas, t, face, width, height);
        four_cards(canvas, t, face, width, height);
    }));

    if let Some(screen) = screen.as_mut() {
        out.push(phase("FRAME Digital Rain + 4 cards + blit", || {
            digital_rain_frame(canvas, t, face, width, height);
            four_cards(canvas, t, face, width, height);
            screen.blit(canvas);
        }));
    }

    // ---- The layered alternative -------------------------------------------
    // If the cards were composited from a cached layer instead of redrawn, a
    // frame would be the background plus one full-screen composite. That
    // composite is not free, so it is measured rather than assumed better; the
    // comparison is this against the two FRAME phases above.
    let layer = vec![0u8; canvas.len()];
    out.push(phase("layered: full-screen composite pass", || {
        composite(canvas, &layer);
    }));

    out
}

/// One quadrant, the size `aura-renderer` draws.
const fn card_rect(width: usize, height: usize) -> (usize, usize, usize, usize) {
    const GUTTER: usize = 64;
    (
        GUTTER,
        GUTTER,
        (width - GUTTER * 3) / 2,
        (height - GUTTER * 3) / 2,
    )
}

/// A card's type: a big value, a unit, a second line, a caption.
fn card_text(canvas: &mut [u8], t: &Target, face: &Face, card: (usize, usize, usize, usize)) {
    let (x, y, w, h) = card;
    let value_px = (h as f32 * 0.34).min(w as f32 * 0.30);
    face.draw(canvas, t, x + 40, y + 200, "22:15", value_px, INK);
    face.draw(canvas, t, x + 40, y + 300, "°F", value_px * 0.28, MUTED);
    face.draw(canvas, t, x + 40, y + 380, "62 °F", value_px * 0.42, MUTED);
    face.draw(
        canvas,
        t,
        x + 40,
        y + 440,
        "Monday, September 1",
        value_px * 0.22,
        MUTED,
    );
}

/// Four cards, as the renderer draws them.
fn four_cards(canvas: &mut [u8], t: &Target, face: &Face, width: usize, height: usize) {
    const GUTTER: usize = 64;
    let w = (width - GUTTER * 3) / 2;
    let h = (height - GUTTER * 3) / 2;
    for (x, y) in [
        (GUTTER, GUTTER),
        (GUTTER * 2 + w, GUTTER),
        (GUTTER, GUTTER * 2 + h),
        (GUTTER * 2 + w, GUTTER * 2 + h),
    ] {
        draw::fill_rounded(canvas, t, x, y, w, h, 28, CARD);
        card_text(canvas, t, face, (x, y, w, h));
    }
}

/// The Boxes background: an 8x8 grid of flat rectangles, repainted every frame.
///
/// `cpp/src/ss_boxes.hpp`. The cells only change colour once a second, but the
/// whole grid is redrawn on every frame that anything else forces, which is
/// every frame while it is animated.
fn boxes_frame(canvas: &mut [u8], t: &Target, width: usize, height: usize) {
    const COLS: usize = 8;
    const ROWS: usize = 8;
    let (cw, ch) = (width / COLS, height / ROWS);
    for row in 0..ROWS {
        for col in 0..COLS {
            // Eight palette entries, walked deterministically. The real one
            // picks at random; the cost is the same and a benchmark that varies
            // its own workload cannot be compared with itself.
            let shade = u8::try_from((row * COLS + col) % 8 * 4).unwrap_or(0);
            draw::fill(
                canvas,
                t,
                col * cw,
                row * ch,
                cw,
                ch,
                [shade / 2, shade, shade + 8],
            );
        }
    }
}

/// The Digital Rain background: a grid of glyphs, one per lit cell.
///
/// `cpp/src/ss_matrix.hpp` — the file name is the only place the old word
/// survives; every user-facing string in Aura's portal already says *Digital
/// Rain*. An 18px cell over 1080p is 106 x 60 cells; in steady state roughly a
/// quarter of them carry a non-zero brightness, and the C++ renderer skips the
/// rest (`if (b == 0) continue`). A quarter is what is drawn here.
fn digital_rain_frame(canvas: &mut [u8], t: &Target, face: &Face, width: usize, height: usize) {
    const CELL: usize = 18;
    draw::fill(canvas, t, 0, 0, width, height, [0, 0, 0]);
    let cols = width / CELL;
    let rows = height / CELL;
    for row in 0..rows {
        for col in (row % 4..cols).step_by(4) {
            face.draw(
                canvas,
                t,
                col * CELL,
                row * CELL + 16,
                "A",
                CELL as f32,
                [0, 200, 50],
            );
        }
    }
}

/// A full-screen alpha composite of a cached layer over the frame.
///
/// The layered design's per-frame cost: instead of redrawing every rounded
/// rectangle and every glyph, blend a prepared card layer in. Measured because
/// a full-screen composite is not free either, and the comparison is the whole
/// question.
fn composite(canvas: &mut [u8], layer: &[u8]) {
    for (dst, src) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(layer.as_chunks::<4>().0) {
        // The fourth byte is the alpha the layer was drawn with. XRGB8888 has
        // no alpha channel on screen, but a cached layer in ordinary memory is
        // free to use that byte for one.
        let a = u32::from(src[3]);
        if a == 0 {
            continue;
        }
        for c in 0..3 {
            dst[c] = u8::try_from(
                (u32::from(src[c]) * a + u32::from(dst[c]) * (255 - a)) / 255,
            )
            .unwrap_or(dst[c]);
        }
    }
    std::hint::black_box(canvas);
}

/// Print the cycle's table where the journal will catch it.
fn report(cycle: u64, elapsed: Duration, measurements: &[Measurement]) {
    println!(
        "\naura-probe: cycle {cycle}, {:.0}s in --------------------------------",
        elapsed.as_secs_f64()
    );
    println!(
        "{:<40} {:>10} {:>10}  budget",
        "phase", "first-1s", "sustained"
    );
    for m in measurements {
        println!(
            "{:<40} {:>8.3}ms {:>8.3}ms  {}   (n={})",
            m.name,
            m.first_second_ms,
            m.sustained_ms,
            m.verdict(),
            m.iterations
        );
    }
}

/// The same table, on the screen, so it can be photographed or captured.
fn draw_report(
    canvas: &mut [u8],
    t: &Target,
    face: &Face,
    measurements: &[Measurement],
    cycle: u64,
) {
    draw::fill(canvas, t, 0, 0, t.width, t.height, [0, 0, 0]);
    let px = (t.height as f32 / 34.0).min(28.0);
    let line = face.line_height(px);
    let mut y = line * 2;
    face.draw(
        canvas,
        t,
        40,
        y,
        &format!("aura-probe  cycle {cycle}   {}x{}", t.width, t.height),
        px,
        INK,
    );
    y += line * 2;
    for m in measurements {
        let text = format!(
            "{:<38} {:>8.2}ms  {:>8.2}ms  {}",
            m.name, m.first_second_ms, m.sustained_ms, m.verdict()
        );
        face.draw(canvas, t, 40, y, &text, px, MUTED);
        y += line;
        if y > t.height {
            break;
        }
    }
}
