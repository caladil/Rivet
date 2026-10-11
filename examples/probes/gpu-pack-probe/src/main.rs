//! **What does the SHIPPING pack cost on the GPU?**
//!
//! `examples/probes/gpu-surface` measured Contours at 8.19 ms and Life at 9.80
//! and `.rulings/` R-207 was ruled on those numbers. They were measured
//! against **that probe's own copy** of the backgrounds — 2,049 lines
//! transcribed out of `aura-background-pack` — and between the copy and the
//! product there is now an entire boundary that did not exist when the copy
//! was written:
//!
//! * a `dlopen` of a signed library out of `/state/lib`,
//! * `rivetos_addon::gl`'s descriptor, composed by the host and validated by
//!   the pack,
//! * a GLES table the pack resolved with `dlsym` rather than linked,
//! * and the pack's own state, packed into a texture by code that is not the
//!   probe's.
//!
//! **Not one metre of that was in the 8.19.** This probe drives the real
//! object across a real surface and reports what it actually costs.
//!
//! # What it proves and what it does not
//!
//! It proves the boundary carries a frame and what the frame costs. It does
//! **not** prove the picture is right — nothing here reads a pixel back, and
//! a shader that drew the wrong thing at the right speed would pass. That is
//! what the HDMI capture is for, and the two go together: this answers *"is it
//! under budget"*, the capture answers *"is it Contours"*.
//!
//! # Why it reports per background rather than in total
//!
//! Three backgrounds with different shaders and different uploads — 5,184
//! bytes a frame, 28,800 and 192 — and a mean across them would describe none
//! of them. `GPU-Backgrounds` §1.3 is a table for the same reason.

use std::time::{Duration, Instant};

use rivetos_addon::frame::Scene;
use rivetos_addon::gl::GlTarget;
use rivetos_addon::host::Loaded;
use rivetos_addon::protocol::{Request, Response};

/// The class this probe hosts. Named and not a path: R-199's arrangement is
/// that a host names a class and the operator decides which object provides
/// it.
const CLASS: &str = "aura-background";

/// How long to measure each background. Long enough that a slow first frame —
/// three shaders compiling, a texture created — is averaged away rather than
/// reported as the cost of drawing.
const RUN_FOR: Duration = Duration::from_secs(15);

/// `aura-renderer`'s budget, so a percentage here means what it means there.
const FRAME_BUDGET: Duration = Duration::from_micros(33_333);

/// **The frame this probe reports from.** The first is thrown away by
/// [`RUN_FOR`] being long, but the very first GPU frame also compiles three
/// programs, and including it would put tens of milliseconds into a mean
/// computed over a few hundred frames.
const WARMUP_FRAMES: u64 = 5;

fn main() {
    let journal = rivet_sdk::journal::Journal::for_service("gpu-pack-probe");
    let say = |line: &str| {
        journal.info(line);
        println!("gpu-pack-probe: {line}");
    };

    let installed = rivetos_addon::host::installed(CLASS);
    say(&format!(
        "enumerated {} librar{} of class {CLASS}",
        installed.len(),
        if installed.len() == 1 { "y" } else { "ies" }
    ));
    if installed.is_empty() {
        say("no pack of this class is installed, so there is nothing to measure");
        return;
    }

    // **Chosen by CAPABILITY and not by position**, which the first run of
    // this probe taught it. `rivet-01` had two libraries of class
    // `aura-background` installed — a board accumulates them, because there is
    // no uninstall verb (`docs/RELEASE-BLOCKERS.md` §5) — and the first one
    // enumerated was the older, whose `draws_gl` is `false`. The probe
    // reported *"this pack exports no GPU entry point"*, which was true of the
    // object it had opened and false of the machine.
    //
    // **The same trap a host falls into**, and worth stating in a probe rather
    // than only fixing here: a host that opens `installed(class).first()` and
    // asks it to draw gets whichever library the enumeration happened to put
    // first. Every one of them is opened below and what each answers is
    // printed, so the report says what is on the machine rather than what the
    // probe happened to pick.
    let mut chosen: Option<Loaded> = None;
    for entry in &installed {
        let pack = match Loaded::open_path(&entry.path) {
            Ok(pack) => pack,
            Err(why) => {
                say(&format!("  {}: {why}", entry.path.display()));
                continue;
            }
        };
        say(&format!(
            "  {} at {}: draws={} draws_gl={} gl descriptor version={}",
            entry.soname(),
            entry.path.display(),
            pack.draws(),
            pack.draws_gl(),
            pack.gl_abi()
                .map_or_else(|| "(none)".to_string(), |v| v.to_string())
        ));
        if pack.draws_gl() && chosen.is_none() {
            chosen = Some(pack);
        }
    }
    let Some(pack) = chosen else {
        say(&format!(
            "none of the {} installed librar{} exports a GPU entry point — nothing to measure, \
             and not a fault",
            installed.len(),
            if installed.len() == 1 { "y" } else { "ies" }
        ));
        return;
    };

    // The catalogue decides what is measured. A probe with a hard-coded list
    // would keep measuring a background the pack had stopped claiming.
    let described = match pack.call(&Request::Describe) {
        Ok(Response::Described(described)) => described,
        Ok(other) => {
            say(&format!("the pack answered Describe with {other:?}"));
            return;
        }
        Err(why) => {
            say(&format!("the pack would not describe itself: {why}"));
            return;
        }
    };
    say(&format!(
        "{} {} describes {} background(s)",
        described.name,
        described.version,
        described.backgrounds.len()
    ));

    // **One pair per background, not every pair.** A theme changes a colour
    // and a level count; it does not change how many fragments the shader
    // runs, so measuring twenty-two of them would take seven minutes to
    // produce three numbers.
    let mut wanted: Vec<(String, String)> = Vec::new();
    for background in &described.backgrounds {
        if !background.drawn.on_gpu() {
            say(&format!(
                "  {} says {:?} — skipped, and that is the catalogue doing its job",
                background.id, background.drawn
            ));
            continue;
        }
        match background.themes.first() {
            Some(theme) => wanted.push((background.id.clone(), theme.id.clone())),
            None => say(&format!("  {} claims a GPU path and lists no theme", background.id)),
        }
    }
    if wanted.is_empty() {
        say("the pack has a GPU entry point and no background that claims it");
        return;
    }

    let mut screen = match rivet_sdk::gpu::take() {
        Ok(screen) => screen,
        Err(why) => {
            // Every failure in `take` names its step: the render node, GBM,
            // EGL, the socket, or a compositor with no `zwp_linux_dmabuf`.
            say(&format!("no GPU surface: {why}"));
            return;
        }
    };
    // **`V3D 4.2` versus `llvmpipe` is the difference between measuring the
    // hardware and measuring a software rasteriser**, and the two are a factor
    // of thirty apart and indistinguishable from every other vantage point.
    say(&format!(
        "drawing on {} at {}x{}",
        screen.what, screen.width, screen.height
    ));

    let mut failed = 0u32;
    for (background, theme) in &wanted {
        if screen.closed() {
            say("the compositor closed the surface");
            break;
        }
        failed += measure(&say, &pack, &mut screen, background, theme);
    }

    let verdict = if failed == 0 {
        "PASSED — every background the catalogue claims for the GPU drew through the boundary"
    } else {
        "FAILED — see the refusals above"
    };
    say(verdict);
}

/// Drive one background for [`RUN_FOR`] and report what it cost.
///
/// Returns `1` if it never drew, so the last line of the report is a verdict
/// rather than something to be read for.
fn measure(
    say: &impl Fn(&str),
    pack: &Loaded,
    screen: &mut rivet_sdk::gpu::Screen,
    background: &str,
    theme: &str,
) -> u32 {
    let started = Instant::now();
    let mut frames: u64 = 0;
    let mut timed: u64 = 0;
    let mut refusals: u64 = 0;
    let mut first_refusal: Option<String> = None;
    // **The two phases Aura's own report separates**, named the same way so
    // the numbers go beside each other without translation. `background` is
    // the pack's drawing — which on this path includes the upload, the shader
    // and the `dlopen`ed call itself; `present` is what `blit-and-flip-wait`
    // is on the CPU side.
    let mut background_total = Duration::ZERO;
    let mut background_max = Duration::ZERO;
    let mut present_total = Duration::ZERO;
    let mut present_max = Duration::ZERO;

    while started.elapsed() < RUN_FOR {
        if screen.closed() {
            break;
        }
        if let Err(why) = screen.make_current() {
            say(&format!("{background}/{theme}: the context would not come current: {why}"));
            return 1;
        }

        // **The frame's time is the wall clock**, which is what
        // `aura-background-pack`'s `advance` expects: it diffs consecutive
        // values to get `dt`, so the simulation advances by elapsed time and a
        // slower frame rate makes the picture choppy rather than slow.
        #[allow(clippy::cast_possible_truncation)]
        let time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);

        // **Framebuffer zero, and that is the whole point of allowing it.**
        // This host's target IS the surface it will present, so there is
        // nothing to create — `rivetos_addon::gl::GlFrame::framebuffer` argues
        // why forcing a render-to-texture here would spend a full-screen V3D
        // pass to satisfy a word rather than a reason.
        let Some(target) = GlTarget::over(screen.width as u32, screen.height as u32, 0) else {
            say(&format!("{background}/{theme}: the surface has no area"));
            return 1;
        };
        let scene = Scene {
            background,
            theme,
            time_ms,
        };

        let drawing_began = Instant::now();
        let drew = pack.draw_gl(&target, &scene);
        let drawn = Instant::now();

        match drew {
            Ok(()) => {
                frames += 1;
                if frames > WARMUP_FRAMES {
                    let cost = drawn - drawing_began;
                    background_total += cost;
                    background_max = background_max.max(cost);
                    timed += 1;
                }
            }
            Err(why) => {
                refusals += 1;
                if first_refusal.is_none() {
                    first_refusal = Some(why.to_string());
                }
            }
        }

        if let Err(why) = screen.present() {
            // Said once, never per frame: thirty a second buries everything
            // else in a rolling journal.
            if refusals == 0 && first_refusal.is_none() {
                first_refusal = Some(format!("a frame was not handed over: {why}"));
            }
        } else if frames > WARMUP_FRAMES {
            let presented = Instant::now() - drawn;
            present_total += presented;
            present_max = present_max.max(presented);
        }
    }

    if let Some(why) = &first_refusal {
        say(&format!("{background}/{theme}: {why}"));
    }
    if timed == 0 {
        say(&format!(
            "{background}/{theme}: NOTHING WAS DRAWN — {refusals} refusal(s) in {:.1}s",
            started.elapsed().as_secs_f64()
        ));
        return 1;
    }

    #[allow(clippy::cast_precision_loss)]
    let n = timed as f64;
    let mean = |total: Duration| total.as_secs_f64() * 1000.0 / n;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let drawing = mean(background_total);
    let budget_ms = FRAME_BUDGET.as_secs_f64() * 1000.0;
    #[allow(clippy::cast_precision_loss)]
    let fps = frames as f64 / started.elapsed().as_secs_f64();
    say(&format!(
        "cost over {timed} timed frames drawing {background}/{theme} THROUGH THE PACK ON THE \
         GPU: background {drawing:.2}/{:.2}ms ({:.0}% of the {budget_ms:.1}ms budget), present \
         {:.2}/{:.2}ms, {fps:.1} fps, {refusals} refusal(s)",
        ms(background_max),
        drawing / budget_ms * 100.0,
        mean(present_total),
        ms(present_max),
    ));
    0
}
