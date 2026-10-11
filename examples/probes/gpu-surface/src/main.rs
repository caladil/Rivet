//! **Does a GPU frame cross to the compositor and reach the glass?**
//!
//! `examples/probes/gpu-flip` asked this for a DRM lease and answered yes:
//! a `gbm_bo` can become a framebuffer a leased CRTC will flip to. `ADR-058`
//! then replaced leases with surfaces, so that proof stopped covering the path
//! an application takes. This is the same question asked of the architecture
//! that exists now, and `.rulings/` R-206 rests on the answer:
//!
//! > A buffer allocated on `/dev/dri/renderD*`, rendered into with GLES,
//! > exported as a dma-buf and handed to `rivet-displayd` — does it composite?
//!
//! If the answer is no, `Contours` on the GPU is moot and everything built on
//! top of R-206 has to be rethought. That is worth one probe and a few seconds
//! of runtime before any of it is written.
//!
//! # What it draws, and why it is deliberately almost nothing
//!
//! A full-screen clear whose colour walks through a hue cycle, once a frame.
//! There is no geometry, no shader of its own and no texture — because none of
//! those are in question. **What is in question is the seam**, and a probe that
//! also drew a spinning cube would answer two things at once and be unable to
//! say which one failed.
//!
//! The colour moving is the part that matters: a static frame proves a buffer
//! crossed once, and a moving one proves buffers keep crossing and keep being
//! released. `gpu-flip`'s note on `gbm_surface_lock_front_buffer` returning
//! NULL within a few frames is exactly the failure a single frame would hide.
//!
//! # What it is NOT
//!
//! Not a benchmark. It reports a frame rate because the number is free and
//! tells an operator the path is not silently falling back to something slow,
//! but `GPU-Backgrounds-2026-09-20` §6 item 5 — *Aurora and Contours
//! re-measured against the 33.3 ms budget* — is a different exercise against
//! real drawing.

mod contours;

use std::ffi::c_void;
use std::time::{Duration, Instant};

/// How long to run before stopping. `restart = "never"`: this is a probe, it
/// answers a question and exits, and a kiosk that came up showing a colour
/// cycle for ever would be worse than one that came up showing nothing.
const RUN_FOR: Duration = Duration::from_secs(20);

/// How often to report. Long enough that the journal is not flooded — which
/// this project has paid for — and short enough that a stall is visible.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// `aura-renderer`'s own budget, so the percentage means the same thing in
/// both reports. 30 fps.
const FRAME_BUDGET: Duration = Duration::from_micros(33_333);

/// Unused, and kept as the reason this file has no `libdrm` in it: everything
/// that touches KMS is the compositor's now.
#[allow(dead_code)]
const NOTHING_HERE_TOUCHES_DRM: *const c_void = std::ptr::null();

fn main() {
    let journal = rivet_sdk::journal::Journal::for_service("gpu-surface");

    let mut screen = match rivet_sdk::gpu::take() {
        Ok(screen) => screen,
        Err(why) => {
            // **A named refusal and not a black screen.** Every failure in
            // `take` says which step: the render node, GBM, EGL, the socket,
            // or a compositor with no `zwp_linux_dmabuf`. That last one means
            // `rivet-displayd` predates R-206.
            journal.error(&format!("no GPU surface: {why}"));
            println!("gpu-surface: no GPU surface: {why}");
            return;
        }
    };

    // **This line is the probe's first real answer.** `what` names
    // `GL_RENDERER`, and *"V3D 4.2"* versus *"llvmpipe"* is the difference
    // between using the hardware and emulating it in software — indistinguishable
    // from every other vantage point and a factor of thirty apart.
    journal.info(&format!("drawing on {}", screen.what));
    println!("gpu-surface: drawing on {}", screen.what);

    // **All five, in one run** — `GPU-Backgrounds-2026-09-20` §6 item 5 asks
    // for *"Aurora and Contours re-measured against the 33.3 ms budget, on the
    // board"*, Life joined them when the CPU version measured `26.15 ms`, and
    // Starfield and Boxes complete the comparison table.
    //
    // **The last two are here for a different reason than the first three.**
    // Contours, Aurora and Life were measured because they missed the budget.
    // Starfield at `4.11 ms` on the CPU is nowhere near missing it, and Boxes
    // does almost nothing at all — they are measured so that the table has a
    // row where the CPU is expected to WIN. A comparison in which every row
    // favours the GPU is a comparison that has not been tested against a case
    // it could fail.
    //
    // Measured in sequence rather than in five deployments, because the
    // numbers are only comparable if the machine, the driver and the
    // compositor are identical — which is easy to assert and hard to be sure
    // of across a reboot. The run is [`RUN_FOR`] per background, so five of
    // them is a hundred seconds.
    for which in [
        contours::Which::Contours,
        contours::Which::Aurora,
        contours::Which::Life,
        contours::Which::Starfield,
        contours::Which::Boxes,
    ] {
        if screen.closed() {
            break;
        }
        if let Err(why) = measure(&journal, &mut screen, which) {
            journal.error(&format!("{}: {why}", which.id()));
            println!("gpu-surface: {}: {why}", which.id());
            return;
        }
    }
}

/// Draw one background for [`RUN_FOR`] and report what it cost.
fn measure(
    journal: &rivet_sdk::journal::Journal,
    screen: &mut rivet_sdk::gpu::Screen,
    which: contours::Which,
) -> Result<(), String> {
    let gpu = contours::Gpu::new(which)?;
    // **The surface size, not 1920x1080.** Starfield's vanishing point and
    // Boxes' truncating cell division are both fixed at construction in the CPU
    // versions, so they are fixed here too — and they need the size to do it.
    let mut scene = contours::Scene::new(which, screen.width, screen.height);
    journal.info(&format!(
        "{} is on the GPU: {}",
        which.id(),
        scene.describe()
    ));

    let started = Instant::now();
    let mut last_report = started;
    let mut last_frame = started;
    let mut frames: u64 = 0;
    let mut reported_frames: u64 = 0;
    let mut failures: u64 = 0;
    // **The two phases Aura's own report separates**, named the same way so
    // the numbers can be put beside each other without translation:
    // `background` is the drawing, `present` is what `blit-and-flip-wait` is.
    let mut background_total = Duration::ZERO;
    let mut background_max = Duration::ZERO;
    let mut present_total = Duration::ZERO;
    let mut present_max = Duration::ZERO;

    while started.elapsed() < RUN_FOR {
        if screen.closed() {
            journal.info("the compositor closed the surface");
            break;
        }

        screen.make_current()?;

        // **`dt` from the wall clock, as the pack's host does.** The field
        // advances by elapsed time and not per frame, so a slower frame rate
        // makes the picture choppy rather than slow — which is what
        // `aura-renderer` already relies on. Life's automaton is the same
        // bargain in the other direction: its step count comes out of `dt`, so
        // it takes four steps a second whatever the frame rate is.
        //
        // **Outside the timed region on purpose**, for all three. What this
        // probe is measuring is the drawing, and the CPU number it is being
        // compared against — 34.29 ms, 26.15 ms — is `background`, which on
        // that side is also the drawing and not the simulation.
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32();
        last_frame = now;
        scene.update(dt);

        let drawing_began = Instant::now();
        gpu.draw(&scene, screen.width, screen.height);
        let drawn = Instant::now();

        let background = drawn - drawing_began;
        background_total += background;
        background_max = background_max.max(background);

        match screen.present() {
            Ok(()) => {
                frames += 1;
                let presented = Instant::now() - drawn;
                present_total += presented;
                present_max = present_max.max(presented);
            }
            Err(why) => {
                failures += 1;
                // **Said once per report, never once per frame.** Thirty a
                // second buries everything else in a rolling ~1 MB journal.
                if failures == 1 {
                    journal.error(&format!("a frame was not handed over: {why}"));
                }
            }
        }

        if last_report.elapsed() >= REPORT_EVERY {
            let window = last_report.elapsed().as_secs_f64();
            let in_window = frames - reported_frames;
            #[allow(clippy::cast_precision_loss)]
            let fps = in_window as f64 / window;
            let line = phase_line(
                which,
                frames,
                background_total,
                background_max,
                present_total,
                present_max,
                fps,
                failures,
            );
            journal.info(&line);
            println!("gpu-surface: {line}");
            last_report = Instant::now();
            reported_frames = frames;
        }
    }

    // **The verdict, in one line, because this is what the probe is for.**
    let verdict = if frames == 0 {
        "NO FRAME CROSSED — a dma-buf did not reach the compositor"
    } else if failures > 0 {
        "frames crossed, but some were refused — see the error above"
    } else {
        "PASSED — a GPU buffer crossed to the compositor as a dma-buf and was composited"
    };
    journal.info(&format!(
        "{verdict}. {frames} frame(s), {failures} refusal(s), over {:.1}s",
        started.elapsed().as_secs_f64()
    ));
    println!("gpu-surface: {verdict} — {frames} frame(s), {failures} refusal(s)");

    // **Life's own two numbers**, because a background can be cheap for the
    // wrong reason. A generation count near zero, or an intervention count in
    // the dozens, would mean the board went static or kept being reseeded —
    // and a static board is cheap on the CPU too.
    if let Some(note) = scene.summary() {
        journal.info(&note);
        println!("gpu-surface: {note}");
    }

    // **The answer, in the same words Aura's report uses**, so the two numbers
    // can be read side by side without anybody converting units.
    #[allow(clippy::cast_precision_loss)]
    let fps = frames as f64 / started.elapsed().as_secs_f64().max(0.001);
    let line = phase_line(
        which,
        frames,
        background_total,
        background_max,
        present_total,
        present_max,
        fps,
        failures,
    );
    journal.info(&format!("FINAL — {line}"));
    println!("gpu-surface: FINAL — {line}");
    Ok(())
}

/// The frame cost in the shape `aura-renderer`'s own report uses.
///
/// **`background` against 33.3 ms is the question this probe exists for.** The
/// CPU versions' numbers on `rivet-01` are `34.29 ms` for Contours — 103% of
/// the whole budget spent on the background alone, before a single widget is
/// drawn — and `26.15/49.56 ms` for Life, which is 79% of the budget on its
/// own and **105% once the widgets' 8.91 ms are added**. Life is the one where
/// the mean is not the whole story: its maximum is half again the budget.
#[allow(clippy::cast_precision_loss)]
fn phase_line(
    which: contours::Which,
    frames: u64,
    background_total: Duration,
    background_max: Duration,
    present_total: Duration,
    present_max: Duration,
    fps: f64,
    failures: u64,
) -> String {
    let n = frames.max(1) as f64;
    let mean = |total: Duration| total.as_secs_f64() * 1000.0 / n;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let background = mean(background_total);
    let budget_ms = FRAME_BUDGET.as_secs_f64() * 1000.0;
    format!(
        "cost over {frames} frames drawing {} ON THE GPU: background \
         {background:.2}/{:.2}ms ({:.0}% of the {budget_ms:.1}ms budget), present \
         {:.2}/{:.2}ms, {fps:.1} fps, {failures} refusal(s)",
        which.id(),
        ms(background_max),
        background / budget_ms * 100.0,
        mean(present_total),
        ms(present_max),
    )
}
