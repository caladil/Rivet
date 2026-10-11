//! **Hand a pack a buffer, read the bytes back, and say what one frame cost**
//! — in the journal, because that is where a proof on a headless board has to
//! end up.
//!
//! Every line begins `frame-probe-host:` so that one `get log` reads the whole
//! article.
//!
//! # What it does, and why in this order
//!
//! 1. **Reports `LD_LIBRARY_PATH` and `RIVET_ADDONS`.** If the supervisor's
//!    class match did not happen, everything after this fails for a reason
//!    that has nothing to do with the boundary, and these two lines are what
//!    tells them apart.
//! 2. **Checks the trial note.** If a previous start died while loading this
//!    class it is skipped and named — `.rulings/` R-198 D3, and a pack that
//!    corrupts its host is exactly the failure that mechanism exists for.
//! 3. **Enumerates every library of its class** and runs the article against
//!    each, because `.rulings/` R-199 D1 makes a second pack the case to
//!    support and a run over one proves only half of it.
//! 4. **The article**: describe, configure, then frames whose bytes are
//!    checked rather than looked at — a flat fill, a per-row pattern, the
//!    host's own clock carried in a `u64`, a version refusal, two id
//!    refusals, and a background that panics while drawing.
//! 5. **The measurement.** `SPEC-04` §10 records what this board's renderer
//!    spends per phase; this reports what the crossing adds to it, measured
//!    here rather than scaled from a workstation.
//! 6. **Idles**, so the report stands still and `get process` has something to
//!    show.
//!
//! # The surface is the host's, and it is deliberately not a tidy one
//!
//! 1920x1080 at a stride of [`STRIDE`] rather than `width * 4`. A scanout
//! buffer is padded to the controller's pitch, and a pack that computes a row
//! address from the width would draw a sheared picture that still passes any
//! check made only on row zero. The padding is filled with [`SENTINEL`] before
//! every frame, so **writing outside the lines is a failure this program
//! reports rather than a defect somebody notices on a television.**

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use rivetos_addon::frame::{self, Scene};
use rivetos_addon::host::{DrawFailure, Installed, Loaded, Target};
use rivetos_addon::{Request, Response};

/// The class this probe accepts, the same string its manifest declares.
const CLASS: &str = "probe-background";

/// The surface, as the board's panel has it.
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// **Padded past `WIDTH * 4`, on purpose.** See this module's header: a stride
/// that happens to equal a row makes a whole class of addressing defect
/// invisible.
const STRIDE: u32 = 1920 * 4 + 128;

/// What the host writes over the whole buffer before a frame. A refusal that
/// claims nothing was written is checked against this byte and not against
/// zero, because zero is also a colour a pack might legitimately have drawn.
const SENTINEL: u8 = 0xAB;

/// A time the host makes up, so that the value carried in `Frame::time_ms` is
/// one this program can recognise coming back.
const STAMP_MS: u64 = 0x0123_4567_89AB;

/// Frames per timing round for the crossing, which writes nothing.
const CROSSING_FRAMES: u32 = 2_000;
/// Frames per timing round for a full-surface fill, which writes 8.29 MB each
/// time. Fewer, because the point of it is the per-frame number and not the
/// total.
const FILL_FRAMES: u32 = 200;
/// **Three rounds, reported separately.** A single round measures whatever the
/// governor and the scheduler were doing at the time; three that agree are a
/// measurement, and three that do not are a finding.
const ROUNDS: u32 = 3;

fn main() {
    let data_dir = std::env::var("RIVET_APP_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("frame-probe-host"));

    say(&format!(
        "starting. LD_LIBRARY_PATH={}",
        std::env::var("LD_LIBRARY_PATH").unwrap_or_else(|_| "(unset)".to_string())
    ));
    say(&format!(
        "RIVET_ADDONS={}",
        std::env::var(rivetos_addon::host::ADDONS_ENV)
            .unwrap_or_else(|_| "(unset — this machine names no addons)".to_string())
    ));
    say(&format!(
        "surface {WIDTH}x{HEIGHT}, stride {STRIDE} ({} bytes of padding per row), \
         frame descriptor version {}",
        STRIDE - WIDTH * 4,
        frame::FRAME_VERSION
    ));

    // Enumerated BEFORE the trial note is written, because it is what the
    // note's identity is compared against.
    let joined = rivetos_addon::host::installed(CLASS);
    say(&format!(
        "enumerated {} librar{} of class {CLASS}",
        joined.len(),
        if joined.len() == 1 { "y" } else { "ies" }
    ));
    for (n, entry) in joined.iter().enumerate() {
        say(&format!(
            "  [{n}] {} at {}",
            entry.soname(),
            entry.path.display()
        ));
    }
    let identity = match joined.as_slice() {
        [only] => only.identity(),
        _ => rivetos_addon::abi::soname_for_class(CLASS),
    };

    match rivetos_addon::trial::left_behind(&data_dir, CLASS, &identity) {
        rivetos_addon::trial::LeftBehind::Clear => {}
        rivetos_addon::trial::LeftBehind::Same => {
            say(&format!(
                "SKIPPED a pack of class {CLASS} was being loaded when this program last \
                 stopped, so it is not loaded now. Reinstalling it clears that."
            ));
            idle();
        }
        rivetos_addon::trial::LeftBehind::Unidentified => {
            say(&format!(
                "SKIPPED a pack of class {CLASS} was being loaded when this program last \
                 stopped, so it is not loaded now. This note predates identity tracking, so a \
                 reinstall cannot be told apart from the crash that wrote it; delete {} to \
                 clear it.",
                rivetos_addon::trial::note_path(&data_dir, CLASS).display()
            ));
            idle();
        }
        rivetos_addon::trial::LeftBehind::Different => {
            if let Err(e) = rivetos_addon::trial::forgive(&data_dir, CLASS) {
                say(&format!(
                    "REFUSED a stale trial note for {CLASS} names an object other than what is \
                     installed now, and could not be cleared: {e}"
                ));
                idle();
            }
        }
    }
    if let Err(e) = rivetos_addon::trial::begin(&data_dir, CLASS, &identity) {
        say(&format!(
            "REFUSED to load an addon without recording it first: {e}. An unrecorded load \
             cannot be rolled back."
        ));
        idle();
    }

    if joined.is_empty() {
        match Loaded::open(CLASS) {
            Ok(a) => say(&format!("open({CLASS}) gave {}", a.soname())),
            Err(e) => say(&format!("open({CLASS}) said: {e}")),
        }
        say("no pack of this class is named, so there is nothing to draw through");
        let _ = rivetos_addon::trial::settled(&data_dir, CLASS);
        idle();
    }

    // **The surface is allocated once, outside every loop.** The host owns it
    // for the life of this program, which is the arrangement the boundary's
    // rule describes: the addon never allocates it and never frees it.
    let mut pixels = vec![SENTINEL; (STRIDE as usize) * (HEIGHT as usize)];

    let mut settled = false;
    let mut failures = 0u32;
    for entry in &joined {
        failures += article(entry, &mut pixels, &data_dir, &mut settled);
    }

    if failures == 0 {
        say("ARTICLE COMPLETE — every check passed");
    } else {
        say(&format!("ARTICLE COMPLETE — {failures} CHECK(S) FAILED"));
    }
    idle();
}

/// **The whole conversation with one object**, returning how many checks it
/// failed so the last line of the report is a verdict rather than something to
/// be read for.
fn article(
    entry: &Installed,
    pixels: &mut [u8],
    data_dir: &std::path::Path,
    settled: &mut bool,
) -> u32 {
    let name = entry.soname().to_string();
    say(&format!("---- {name} — {}", entry.path.display()));

    let pack = match Loaded::open_path(&entry.path) {
        Ok(a) => a,
        Err(e) => {
            say(&format!("{name}: {e}"));
            return 1;
        }
    };
    say(&format!(
        "{name}: draws={} frame descriptor version={}",
        pack.draws(),
        pack.frame_abi().map_or_else(
            || "(none — it does not draw)".to_string(),
            |v| v.to_string()
        )
    ));

    // The JSON half first, because the catalogue it returns is what says which
    // ids the frames below may legally name.
    match pack.call(&Request::Describe) {
        Ok(Response::Described(d)) => {
            say(&format!(
                "{name}: described: {} {} class={} backgrounds={}",
                d.name,
                d.version,
                d.class,
                d.backgrounds.len()
            ));
            for background in &d.backgrounds {
                say(&format!(
                    "    {} ({}) themes: {}",
                    background.id,
                    background.name,
                    background
                        .themes
                        .iter()
                        .map(|t| format!("{} ({})", t.id, t.label))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        Ok(other) => say(&format!("{name}: describe answered {other:?}")),
        Err(e) => say(&format!("{name}: describe failed: {e}")),
    }

    if !*settled {
        *settled = true;
        match rivetos_addon::trial::settled(data_dir, CLASS) {
            Ok(()) => say("the load settled; the trial note is cleared"),
            Err(e) => say(&format!("the trial note could not be cleared: {e}")),
        }
    }

    configure(&pack, &name, true);

    let mut failures = 0u32;
    failures += checks(&pack, &name, pixels);

    // Every check the pack can skip, skipped — so the measurement is of the
    // boundary and not of a pack being careful.
    configure(&pack, &name, false);
    measure(&pack, &name, pixels);

    // **The two halves have to reach ONE object**, and this is where that is
    // checked: the pack counts the frames it drew and reports the count
    // through the JSON half. It read zero the first time this article ran,
    // because the generated glue held two instances — a defect nothing but a
    // run across a real boundary would have shown, since each half passed its
    // own tests perfectly.
    let mut verdict = Verdict {
        name: &name,
        failures: 0,
    };
    match pack.call(&Request::Poll { now_ms: 0 }) {
        Ok(Response::Polled(p)) => {
            let drawn = p.reading.map_or(0.0, |r| r.value);
            say(&format!(
                "{name}: poll: status={:?} frames drawn={drawn} — {}",
                p.status, p.complaint
            ));
            if drawn > 0.0 {
                verdict.passed(&format!(
                    "one instance — the object that described itself is the object that drew \
                     {drawn} frames"
                ));
            } else {
                verdict.failed(
                    "one instance",
                    "the pack reports having drawn no frame, so the drawing entry and the call \
                     entry are reaching two different objects",
                );
            }
        }
        Ok(other) => verdict.failed("one instance", &format!("poll answered {other:?}")),
        Err(e) => verdict.failed("one instance", &e.to_string()),
    }

    // LAST, because it poisons the pack — see `the_panic`'s own account.
    failures + verdict.failures + the_panic(&pack, &name, pixels)
}

fn configure(pack: &Loaded, name: &str, strict: bool) {
    let mut settings = serde_json::Map::new();
    settings.insert("strict".to_string(), serde_json::json!(strict));
    match pack.call(&Request::Configure {
        settings,
        material: BTreeMap::new(),
    }) {
        Ok(Response::Configured { usable, complaint }) => say(&format!(
            "{name}: configured strict={strict}: usable={usable}{}",
            if complaint.is_empty() {
                String::new()
            } else {
                format!(" — {complaint}")
            }
        )),
        Ok(other) => say(&format!("{name}: configure answered {other:?}")),
        Err(e) => say(&format!("{name}: configure failed: {e}")),
    }
}

/// **A running verdict.** Every check says PASSED or FAILED on its own line
/// and the count is what makes the report's last line a verdict rather than
/// something a reader has to scan for.
struct Verdict<'a> {
    name: &'a str,
    failures: u32,
}

impl Verdict<'_> {
    fn passed(&self, line: &str) {
        say(&format!("{}: PASSED {line}", self.name));
    }

    fn failed(&mut self, what: &str, why: &str) {
        say(&format!("{}: FAILED {what}: {why}", self.name));
        self.failures += 1;
    }
}

/// **Six frames, every one of them checked by reading the surface back.**
/// The panic is not among them — see [`the_panic`] for why it runs last.
fn checks(pack: &Loaded, name: &str, pixels: &mut [u8]) -> u32 {
    let mut verdict = Verdict { name, failures: 0 };

    // ---- 1. a flat fill, and the padding it must not have touched ---------
    {
        let mut target = prepare(pixels);
        let scene = Scene {
            background: "flat",
            theme: "red",
            time_ms: 0,
        };
        match pack.draw(&mut target, &scene) {
            Ok(()) => {
                let want = 0x00FF_0000;
                match first_visible_mismatch(&target, |_, _| want) {
                    Some((x, y, got)) => verdict.failed(
                        "flat fill",
                        &format!("pixel ({x},{y}) is {got:#08x} and should be {want:#08x}"),
                    ),
                    None => verdict.passed(&format!(
                        "flat fill — every one of {} visible pixels is {want:#08x}",
                        WIDTH * HEIGHT
                    )),
                }
                match first_touched_padding(&target) {
                    Some((y, at)) => verdict.failed(
                        "flat fill",
                        &format!("the padding of row {y} was written at byte {at}"),
                    ),
                    None => verdict.passed("the padding between rows is untouched"),
                }
            }
            Err(e) => verdict.failed("flat fill", &e.to_string()),
        }
    }

    // ---- 2. one colour per row: the stride, read correctly ----------------
    {
        let mut target = prepare(pixels);
        let scene = Scene {
            background: "bands",
            theme: "green",
            time_ms: 0,
        };
        match pack.draw(&mut target, &scene) {
            Ok(()) => match first_visible_mismatch(&target, |_, y| 0x0000_FF00 | (y & 0xFF)) {
                Some((x, y, got)) => verdict.failed(
                    "row bands",
                    &format!("pixel ({x},{y}) is {got:#08x} and the row's band is not that"),
                ),
                None => verdict.passed(&format!(
                    "row bands — all {HEIGHT} rows carry their own index, so the stride was read \
                     as a stride"
                )),
            },
            Err(e) => verdict.failed("row bands", &e.to_string()),
        }
    }

    // ---- 3. the host's own clock, carried as a u64 ------------------------
    {
        let mut target = prepare(pixels);
        let scene = Scene {
            background: "stamp",
            theme: "blue",
            time_ms: STAMP_MS,
        };
        match pack.draw(&mut target, &scene) {
            Ok(()) => {
                let want = u32::try_from(STAMP_MS & 0x00FF_FFFF).unwrap_or(0);
                let got = target.pixel(0, 0).unwrap_or(0);
                if got == want {
                    verdict.passed(&format!(
                        "the clock crossed — {STAMP_MS:#x} came back as {got:#08x} in pixel one"
                    ));
                } else {
                    verdict.failed(
                        "the clock",
                        &format!("pixel one is {got:#08x} and the stamp is {want:#08x}"),
                    );
                }
            }
            Err(e) => verdict.failed("the clock", &e.to_string()),
        }
    }

    // ---- 4. A VERSION THE PACK DOES NOT SPEAK, REFUSED BEFORE A PIXEL -----
    //
    // The boundary's first rule, checked the only way it can be checked from
    // outside: hand over a descriptor stamped with a version nothing speaks,
    // then prove the surface is byte for byte what this program wrote into it.
    {
        let mut target = prepare(pixels);
        let stranger = frame::FRAME_VERSION + 1;
        let scene = Scene {
            background: "flat",
            theme: "red",
            time_ms: 0,
        };
        match pack.draw_claiming_abi(stranger, &mut target, &scene) {
            Err(DrawFailure::Refused { code, .. }) if code == frame::REFUSED_VERSION => {
                match first_written_byte(&target) {
                    Some(at) => verdict.failed(
                        "the version refusal",
                        &format!("the pack refused version {stranger} and still wrote byte {at}"),
                    ),
                    None => verdict.passed(&format!(
                        "a descriptor claiming version {stranger} was refused and all {} bytes of \
                         the surface are still the host's",
                        target.pixels().len()
                    )),
                }
            }
            Err(other) => verdict.failed(
                "the version refusal",
                &format!("expected the pack's own refusal and read: {other}"),
            ),
            Ok(()) => verdict.failed(
                "the version refusal",
                &format!("the pack drew a frame it was told was version {stranger}"),
            ),
        }
    }

    // ---- 5. two ids the pack does not have --------------------------------
    for (background, theme, want, what) in [
        (
            "starfield",
            "red",
            frame::REFUSED_BACKGROUND,
            "an unknown background",
        ),
        ("flat", "phosphor", frame::REFUSED_THEME, "an unknown theme"),
    ] {
        let mut target = prepare(pixels);
        let scene = Scene {
            background,
            theme,
            time_ms: 0,
        };
        match pack.draw(&mut target, &scene) {
            Err(DrawFailure::Refused { code, .. }) if code == want => {
                match first_written_byte(&target) {
                    Some(at) => {
                        verdict.failed(what, &format!("refused, and still wrote byte {at}"));
                    }
                    None => verdict.passed(&format!("{what} was refused and nothing moved")),
                }
            }
            Err(other) => {
                verdict.failed(what, &format!("expected code {want} and read: {other}"));
            }
            Ok(()) => verdict.failed(what, "the pack drew it"),
        }
    }

    verdict.failures
}

/// **A background that panics, and a host that does not** — and it is the
/// LAST thing this program does to a pack, deliberately.
///
/// Unwinding out of an `extern "C"` function is undefined behaviour, so the
/// generated drawing entry catches it and answers with a code. That
/// `catch_unwind` only runs when a host has really `dlopen`ed the object,
/// which is why no test inside either crate can reach it and why this program
/// is where the rule is exercised at all.
///
/// **It runs last because a panic poisons the pack**, which is correct and is
/// the whole design: the glue holds one instance behind a `Mutex`, and a panic
/// taken while that lock was held leaves state nobody can vouch for. Every
/// call after it — drawing or otherwise — is answered with a refusal rather
/// than served from it. Running this before the measurement would therefore
/// measure a poisoned pack refusing, which is a real behaviour and not the one
/// being measured.
fn the_panic(pack: &Loaded, name: &str, pixels: &mut [u8]) -> u32 {
    let mut verdict = Verdict { name, failures: 0 };
    let mut target = prepare(pixels);

    let scene = Scene {
        background: "breaks",
        theme: "still",
        time_ms: 0,
    };
    match pack.draw(&mut target, &scene) {
        Err(DrawFailure::Refused { code, .. }) if code == frame::PANICKED => {
            verdict.passed("the pack panicked while drawing and this program did not");
        }
        Err(other) => verdict.failed(
            "a panicking background",
            &format!("expected code {} and read: {other}", frame::PANICKED),
        ),
        Ok(()) => verdict.failed("a panicking background", "the pack reported drawing it"),
    }

    // A frame after the panic: the pack is poisoned and says so, and — the
    // part that matters for a host holding a display — says so *instead of*
    // writing anything.
    let after = Scene {
        background: "flat",
        theme: "red",
        time_ms: 0,
    };
    match pack.draw(&mut target, &after) {
        Err(DrawFailure::Refused { code, .. }) if code == frame::PANICKED => {
            match first_written_byte(&target) {
                Some(at) => verdict.failed(
                    "a poisoned pack",
                    &format!("it answered PANICKED and still wrote byte {at}"),
                ),
                None => verdict.passed(
                    "a pack that panicked once keeps saying so rather than drawing from state \
                     nobody can vouch for",
                ),
            }
        }
        Err(other) => verdict.failed("a poisoned pack", &format!("read: {other}")),
        Ok(()) => verdict.failed(
            "a poisoned pack",
            "it drew a frame out of state an earlier panic left behind",
        ),
    }

    // **And the JSON half is poisoned too, which is one instance proven from
    // the other direction.** The frame entry and the call entry share a
    // `Mutex`; a panic taken inside one of them is therefore visible from the
    // other, and it could not be if the generated glue held two.
    match pack.call(&Request::Poll { now_ms: 0 }) {
        Ok(Response::Fault(f)) if f.kind == "poisoned" => verdict.passed(
            "the JSON half is poisoned by the DRAWING half's panic — the two entries share one \
             object, shown from the opposite side",
        ),
        Ok(other) => verdict.failed(
            "one instance, after a panic",
            &format!("the call entry answered {other:?} as though nothing had happened"),
        ),
        Err(e) => verdict.failed("one instance, after a panic", &e.to_string()),
    }

    verdict.failures
}

/// **What one frame across this boundary costs.**
///
/// Three phases, and the subtraction between the first two is the answer:
///
/// * **the timing loop alone** — this program's own `Instant` and loop, with
///   nothing in it. Without this, the crossing's number is the crossing plus
///   however long it takes to read a clock, and on a board that is not
///   nothing;
/// * **a frame that draws nothing** — the pack's `nothing` background returns
///   without writing a byte, so what is left is the whole per-frame cost of
///   the boundary: composing a descriptor, validating two ids, the indirect
///   call, the pack's own version and geometry checks, and the return;
/// * **a frame that fills the whole surface**, beside the identical fill done
///   locally in this binary. That pair is the honest way to say what the
///   boundary adds to a REAL frame, and it is reported as a pair rather than
///   as a single difference because the two fills are two compilations of the
///   same idea and the difference carries that as well.
fn measure(pack: &Loaded, name: &str, pixels: &mut [u8]) {
    say(&format!(
        "{name}: measuring — {CROSSING_FRAMES} crossings and {FILL_FRAMES} full fills per round, \
         {ROUNDS} rounds. SPEC-04 §10 has this board's renderer at 12.4 ms of a 33.3 ms budget \
         for the background phase."
    ));

    for round in 1..=ROUNDS {
        let mut target = prepare(pixels);

        // Phase A — the loop and the clock, with nothing in them.
        let started = Instant::now();
        let mut counter = 0u64;
        for _ in 0..CROSSING_FRAMES {
            counter = std::hint::black_box(counter + 1);
        }
        let bare = started.elapsed().as_nanos() / u128::from(CROSSING_FRAMES);
        std::hint::black_box(counter);

        // Phase B — the crossing, with no drawing inside it.
        let nothing = Scene {
            background: "nothing",
            theme: "still",
            time_ms: STAMP_MS,
        };
        let started = Instant::now();
        let mut refused = 0u32;
        for _ in 0..CROSSING_FRAMES {
            if pack.draw(&mut target, &nothing).is_err() {
                refused += 1;
            }
        }
        let crossing = started.elapsed().as_nanos() / u128::from(CROSSING_FRAMES);

        // Phase C — a whole 1080p frame, across the boundary.
        let flat = Scene {
            background: "flat",
            theme: "blue",
            time_ms: 0,
        };
        let started = Instant::now();
        for _ in 0..FILL_FRAMES {
            if pack.draw(&mut target, &flat).is_err() {
                refused += 1;
            }
        }
        let crossed_fill = started.elapsed().as_nanos() / u128::from(FILL_FRAMES);

        // Phase D — the same fill, in this binary, with nothing crossing.
        let started = Instant::now();
        for _ in 0..FILL_FRAMES {
            fill_locally(&mut target, 0x0000_00FF);
        }
        let local_fill = started.elapsed().as_nanos() / u128::from(FILL_FRAMES);

        say(&format!(
            "{name}: round {round}: loop alone {bare} ns; crossing {crossing} ns; \
             fill across {} us; fill locally {} us; refusals {refused}",
            crossed_fill / 1_000,
            local_fill / 1_000
        ));
        say(&format!(
            "{name}: round {round}: ONE CROSSING COSTS {} ns — {} of a 33.3 ms frame budget, \
             and {} of SPEC-04's 12.4 ms background phase",
            crossing.saturating_sub(bare),
            share(crossing.saturating_sub(bare), 33_300_000),
            share(crossing.saturating_sub(bare), 12_400_000)
        ));
    }
}

/// A share, in parts per million, printed as a percentage with enough places
/// to be read. A number this small rounds to zero at two decimal places, and a
/// zero in a measurement is the one thing nobody can act on.
fn share(part: u128, whole: u128) -> String {
    let ppm = part.saturating_mul(1_000_000) / whole;
    format!("{}.{:04}%", ppm / 10_000, ppm % 10_000)
}

/// The same fill the pack does, in this binary, so the two can be compared.
/// Written the way a background would write it — row by row through the
/// stride, four bytes at a time.
fn fill_locally(target: &mut Target<'_>, pixel: u32) {
    let word = pixel.to_le_bytes();
    let row_bytes = (WIDTH as usize) * 4;
    let stride = STRIDE as usize;
    let bytes = target.pixels_mut();
    for y in 0..HEIGHT as usize {
        let start = y * stride;
        for cell in bytes[start..start + row_bytes].as_chunks_mut::<4>().0 {
            cell.copy_from_slice(&word);
        }
    }
}

/// Write the sentinel over every byte and describe the buffer as a surface.
/// Called before each frame, so that what a check reads back is this
/// program's own bytes and never the previous frame's.
fn prepare(pixels: &mut [u8]) -> Target<'_> {
    pixels.fill(SENTINEL);
    Target::new(pixels, WIDTH, HEIGHT, STRIDE, frame::FORMAT_XRGB8888)
        .expect("a surface this program allocated and described itself")
}

/// The first visible pixel that is not what `want` says it should be.
fn first_visible_mismatch(
    target: &Target<'_>,
    want: impl Fn(u32, u32) -> u32,
) -> Option<(u32, u32, u32)> {
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            // `pixel` answers `None` only outside the visible area, and this
            // walk is the visible area — the geometry is this program's own.
            // A `None` here would mean the surface is not the shape this
            // program described, which is a failure rather than a pass.
            let Some(got) = target.pixel(x, y) else {
                return Some((x, y, 0));
            };
            if got != want(x, y) {
                return Some((x, y, got));
            }
        }
    }
    None
}

/// The first row whose padding is no longer the sentinel, with the byte.
fn first_touched_padding(target: &Target<'_>) -> Option<(u32, usize)> {
    let bytes = target.pixels();
    let row_bytes = (WIDTH as usize) * 4;
    let stride = STRIDE as usize;
    for y in 0..HEIGHT as usize {
        let start = y * stride + row_bytes;
        let end = (y + 1) * stride;
        for (offset, byte) in bytes[start..end].iter().enumerate() {
            if *byte != SENTINEL {
                return Some((u32::try_from(y).unwrap_or(u32::MAX), start + offset));
            }
        }
    }
    None
}

/// The first byte of the whole surface — padding included — that is not the
/// sentinel. What a refusal has to leave behind.
fn first_written_byte(target: &Target<'_>) -> Option<usize> {
    target.pixels().iter().position(|b| *b != SENTINEL)
}

/// One line, prefixed, flushed. The supervisor reads this program's stdout
/// into the journal (`ADR-022`), so a `println!` is the journal.
fn say(line: &str) {
    println!("frame-probe-host: {line}");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
}

/// Stay alive so the report stands still and `get process` has something to
/// show. `restart = "never"` means exiting would be final anyway; idling is
/// the more readable of the two.
fn idle() -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
