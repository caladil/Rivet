//! **T6's proof: the pack renders the same bytes the renderer did.**
//!
//! `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T6's rule, repeated because it
//! is the whole reason this file exists: *"a background that renders one
//! pixel differently has not been ported."*
//! `examples/aura/aura-renderer/evidence/T6-background-baseline/` is that
//! rule made checkable — a BLAKE3 hash of every `(background, theme, frame
//! index)` `aura-renderer` drew, captured from `aura-renderer` 0.4.0 before
//! any of `crate::background` moved. This file renders the same forty
//! entries **through this pack** and compares them, entry by entry.
//!
//! # "Through this pack", precisely
//!
//! Every entry is drawn by [`crate::render::render_frame`] — the exact
//! function [`crate::BackgroundPack`]'s `Drawing::draw` calls in production,
//! operating on a real `rivetos_addon::frame::Surface` — not by calling
//! `Screensaver::draw` against a bare canvas and skipping the addon
//! boundary. What differs from production is only how the animation is
//! **driven**: `BackgroundPack::draw` derives `dt` from consecutive hosts'
//! `time_ms` (new glue this task added, unmeasured against real hardware
//! timing — see `lib.rs`), where this file drives `Screensaver::update` with
//! a fixed step directly, the same way
//! `examples/aura/aura-renderer/src/evidence.rs` already did before this
//! move. That is deliberate and not a shortcut: T6's proof is about the
//! PORTED DRAWING CODE being pixel-identical to what it was captured from,
//! and the two ways of driving `update` are `crate::background::Screensaver`
//! being reused twice, not two different Screensavers.
//!
//! `examples/aura/aura-renderer/evidence/T6-background-baseline/README.md`
//! already says this in almost these words, from before this pack existed:
//! *"the pack's own tests render the same ten backgrounds and themes at the
//! same four frame indices from the same seed... and compare their hashes
//! against `manifest.json` by the same three keys."* This is that.
//!
//! # Why this crate needed `Screensaver::new_with_rng` too
//!
//! Unchanged from `aura-renderer/src/evidence.rs`'s own account: `Screensaver
//! ::new`'s `Mt19937::from_the_clock()` makes two captures seconds apart
//! paint two different pictures, and a mismatched hash from that would prove
//! nothing about the port. `new_with_rng` is `crate::background`'s own seam
//! for exactly this, carried across unchanged.
//!
//! # What this does NOT prove
//!
//! The same two limits `aura-renderer`'s own baseline README states, and
//! they are unchanged by the move:
//!
//! * **Boxes and Solid have never been captured off a board** (`SPEC-04`
//!   §12). This proves the pack renders the same bytes the renderer did, not
//!   that either rendering matches the original C++ on a screen.
//! * **One random sequence per background, not the whole space `Mt19937` can
//!   produce.** A port that broke only for a rare seed is not caught here.

use rivetos_addon::frame::Surface;

use crate::background::Screensaver;
use crate::catalogue::ROWS;
use crate::render::render_frame;
use crate::rng::Mt19937;
use crate::typeface::Typeface;

/// The board's own resolution — `examples/aura/aura-renderer/src/
/// evidence.rs`'s `WIDTH`/`HEIGHT`, unchanged, because the baseline was
/// captured at this size and a `Surface` of a different size would not be
/// the same picture even with every other input held equal.
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// `main.rs`'s `FRAME_BUDGET` as seconds — `aura-renderer/src/evidence.rs`'s
/// `FRAME_STEP_SECONDS`, unchanged.
const FRAME_STEP_SECONDS: f32 = 1.0 / 30.0;

/// See `aura-renderer/src/evidence.rs`'s module doc comment for why these
/// four — unchanged.
const FRAME_INDICES: [u32; 4] = [1, 30, 150, 600];

/// The same fixed seed `aura-renderer/src/evidence.rs` captured the baseline
/// with. Changing this number would change every hash in this file without
/// changing a single line of the port, which is exactly the false failure
/// the baseline exists to avoid — so it is not a coincidence that it matches;
/// it is the whole point.
const SEED: u32 = 20_260_919;

/// Where the committed baseline lives — captured by `aura-renderer`, read
/// from here rather than duplicated, so there is one manifest and not two
/// that could silently drift apart.
const BASELINE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../aura-renderer/evidence/T6-background-baseline/manifest.json"
);

/// One rendered-and-hashed cell of the manifest — the same shape
/// `aura-renderer/src/evidence.rs`'s `Entry` uses, so this crate can
/// deserialise the baseline it wrote.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
struct Entry {
    background_id: String,
    background_label: String,
    theme_id: String,
    theme_label: String,
    frame_index: u32,
    frame_time_ms: u32,
    width: usize,
    height: usize,
    stride: usize,
    bytes_per_pixel: usize,
    seed: u32,
    hash_algorithm: String,
    hash: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Manifest {
    schema: u32,
    description: String,
    seed: u32,
    frame_step_seconds: f32,
    width: usize,
    height: usize,
    entries: Vec<Entry>,
}

/// Render every `(background, theme, frame index)` [`ROWS`] describes,
/// **through [`render_frame`] and a real [`Surface`]**, and hash each frame.
fn build_manifest() -> Manifest {
    let rain_face = Typeface::load_embedded(18.0).expect("assets/AuraRain.ttf ships with this crate");
    let stride = WIDTH * 4;

    let mut entries = Vec::with_capacity(ROWS.len() * FRAME_INDICES.len());
    for row in &ROWS {
        let (background_id, background_label, theme_id, theme_label, background) = (
            row.background_id,
            row.background_label,
            row.theme_id,
            row.theme_label,
            row.background,
        );
        for frame_index in FRAME_INDICES {
            // A fresh `Screensaver` per capture, seeded identically every
            // time — `update` is stateful, so reaching frame 600 means
            // stepping from frame 0, exactly as
            // `aura-renderer/src/evidence.rs` does.
            let mut saver = Screensaver::new_with_rng(
                background,
                WIDTH as usize,
                HEIGHT as usize,
                Mt19937::new(SEED),
            );
            for _ in 0..frame_index {
                saver.update(FRAME_STEP_SECONDS);
            }

            let mut bytes = vec![0u8; (stride * HEIGHT) as usize];
            {
                let mut surface =
                    Surface::over(&mut bytes, WIDTH, HEIGHT, stride).expect("1920x1080 is a surface");
                // The evidence surface is tightly packed, so this canvas stays
                // empty and the drawing goes straight into `bytes` — which is
                // the path a real host takes too, and the reason to keep the
                // baseline going through this function rather than around it.
                render_frame(&saver, &mut surface, &rain_face, &mut Vec::new())
                    .expect("every ROWS entry is a background and theme this pack has");
            }
            // `bytes` is tightly packed (`stride == WIDTH * 4`, no padding of
            // its own), so hashing the whole buffer hashes exactly the
            // visible canvas — the same bytes `aura-renderer/src/
            // evidence.rs`'s `canvas` held when IT was hashed.
            let hash = blake3::hash(&bytes).to_hex().to_string();

            entries.push(Entry {
                background_id: background_id.to_string(),
                background_label: background_label.to_string(),
                theme_id: theme_id.to_string(),
                theme_label: theme_label.to_string(),
                frame_index,
                frame_time_ms: (f64::from(frame_index) * f64::from(FRAME_STEP_SECONDS) * 1000.0)
                    .round() as u32,
                width: WIDTH as usize,
                height: HEIGHT as usize,
                stride: stride as usize,
                bytes_per_pixel: 4,
                seed: SEED,
                hash_algorithm: "blake3".to_string(),
                hash,
            });
        }
    }

    Manifest {
        schema: 1,
        description: "T6 comparison — the same forty entries aura-renderer's baseline recorded, \
                       rendered through aura-background-pack instead."
            .to_string(),
        seed: SEED,
        frame_step_seconds: FRAME_STEP_SECONDS,
        width: WIDTH as usize,
        height: HEIGHT as usize,
        entries,
    }
}

/// How many frames each row is timed over in
/// `tests::what_each_background_costs_relative_to_digital_rain`. Three
/// seconds at thirty frames a second, which is twelve of Life's steps — long
/// enough that its 4 Hz automaton is amortised honestly rather than landing
/// entirely inside or entirely outside the window.
#[cfg(test)]
const TIMED_FRAMES: u32 = 90;

/// Frames rendered and discarded before the clock starts.
///
/// **Six hundred, and the number matters more than it looks.** A handful of
/// warm-up frames is enough to pay for a first allocation and a cold cache,
/// and it is nowhere near enough for Digital Rain — which is the row every
/// other row is measured against. A rain column starts with a delay of up to
/// six seconds and then falls from up to half a screen above the top at four
/// to fourteen rows a second, so three seconds in, most of the screen is
/// still empty and the rain costs almost exactly what a plain fill costs.
/// Measured at ten frames of warm-up it came out at 0.762 ms against Solid
/// Black's 0.755 — a reference that was really measuring `draw::fill` and
/// that inflated every ratio in the table by whatever the glyphs were going
/// to cost once they arrived.
///
/// Six hundred frames is twenty seconds, and it is [`FRAME_INDICES`]'s own
/// last entry for the same reason: it is the point the baseline treats as
/// settled.
#[cfg(test)]
const TIMING_WARMUP: u32 = 600;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The in-process half of "run it twice, get the same hashes" — the same
    /// determinism check `aura-renderer/src/evidence.rs` makes of itself,
    /// made here of the pack.
    #[test]
    fn the_same_seed_and_frame_index_render_the_same_bytes_twice() {
        let first = build_manifest();
        let second = build_manifest();
        assert_eq!(first.entries.len(), second.entries.len());
        for (a, b) in first.entries.iter().zip(second.entries.iter()) {
            assert_eq!(
                a, b,
                "{}/{} at frame {} hashed differently across two renders in this crate",
                a.background_id, a.theme_id, a.frame_index
            );
        }
    }

    /// **The proof T6's brief asked for.** Every entry the pack renders
    /// through [`render_frame`] is compared against the baseline
    /// `aura-renderer` captured before this pack existed, entry by entry,
    /// naming every difference rather than stopping at the first.
    #[test]
    fn the_pack_renders_what_the_t6_baseline_recorded() {
        let current = build_manifest();

        let text = std::fs::read_to_string(BASELINE_PATH).unwrap_or_else(|why| {
            panic!(
                "the T6 baseline manifest should be checked in at {BASELINE_PATH}: {why}. It is \
                 captured by aura-renderer's own `cargo test -p aura-renderer --lib \
                 evidence::tests::regenerate_the_t6_background_baseline_manifest -- --ignored`, \
                 not by anything in this crate."
            )
        });
        let baseline: Manifest = serde_json::from_str(&text)
            .unwrap_or_else(|why| panic!("{BASELINE_PATH} did not parse as this file's Manifest schema: {why}"));
        assert_eq!(
            baseline.schema, 1,
            "the baseline manifest's schema version has moved on without this file"
        );
        assert_eq!(baseline.entries.len(), 40, "the T6 baseline is forty entries");

        let key = |e: &Entry| (e.background_id.clone(), e.theme_id.clone(), e.frame_index);

        let mut differences = Vec::new();
        for want in &baseline.entries {
            match current.entries.iter().find(|got| key(got) == key(want)) {
                None => differences.push(format!(
                    "{}/{} @ frame {} ({} ms): in the baseline, but this pack renders no such \
                     entry",
                    want.background_id, want.theme_id, want.frame_index, want.frame_time_ms
                )),
                Some(got) if got.hash != want.hash => differences.push(format!(
                    "{}/{} @ frame {} ({} ms): HASH DIFFERS — baseline {} now {}",
                    want.background_id,
                    want.theme_id,
                    want.frame_index,
                    want.frame_time_ms,
                    want.hash,
                    got.hash
                )),
                Some(got) if got.width != want.width || got.height != want.height => {
                    differences.push(format!(
                        "{}/{} @ frame {}: dimensions differ — baseline {}x{} now {}x{}",
                        want.background_id,
                        want.theme_id,
                        want.frame_index,
                        want.width,
                        want.height,
                        got.width,
                        got.height
                    ));
                }
                Some(_) => {}
            }
        }
        // **The reverse direction, scoped to the combinations the baseline
        // knows about.**
        //
        // `build_manifest` walks `ROWS`, and `ROWS` grew from ten rows to
        // twenty-two when T8 added four backgrounds. The baseline did not and
        // must not: it is a recording of what `aura-renderer` 0.4.0 drew
        // before this pack existed, and regenerating it to accommodate new
        // work would destroy the only evidence that the port is faithful. So
        // an entry for a `(background, theme)` pair the baseline has never
        // heard of is not a difference — it is a background that did not
        // exist when the recording was made.
        //
        // **The check is kept for pairs the baseline DOES have**, which is
        // the half that can still catch a real regression: a frame index
        // dropped from or added to one of the original ten rows, or a row
        // renamed so that it no longer lines up with its recording. Deleting
        // this loop outright would have been the easy fix and would have
        // silently allowed all three.
        let baseline_knows = |entry: &Entry| {
            baseline
                .entries
                .iter()
                .any(|want| want.background_id == entry.background_id && want.theme_id == entry.theme_id)
        };
        for got in &current.entries {
            if !baseline_knows(got) {
                continue;
            }
            if !baseline.entries.iter().any(|want| key(want) == key(got)) {
                differences.push(format!(
                    "{}/{} @ frame {} ({} ms): this pack renders it, but it is not in the \
                     baseline",
                    got.background_id, got.theme_id, got.frame_index, got.frame_time_ms
                ));
            }
        }

        assert!(
            differences.is_empty(),
            "the T6 comparison found {} difference(s):\n{}",
            differences.len(),
            differences.join("\n")
        );
    }

    /// **What each background costs, as a ratio against Digital Rain.**
    ///
    /// `cargo test --release -p aura-background-pack --lib \
    ///  evidence::tests::what_each_background_costs_relative_to_digital_rain \
    ///  -- --ignored --nocapture`
    ///
    /// # This does not measure the board, and it cannot
    ///
    /// `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T8 says *"a background
    /// that misses the budget is a defect, not a preference"*, and the budget
    /// is 33.3 ms on a Raspberry Pi. **Nothing in this repository can measure
    /// a Raspberry Pi**, and a number printed by this test on a developer's
    /// machine is not that number and must never be reported as though it
    /// were. An absolute figure from here says only how fast this machine is.
    ///
    /// What it *can* do is establish a **ratio**, and a ratio is the part
    /// that transfers. `SPEC-04` §10 records Digital Rain's drawing at 12.4
    /// ms of the 33.3 ms budget on the target hardware — a measurement taken
    /// on the board, by someone holding it. A background measured here at a
    /// third of Digital Rain's cost is doing a third of the work, and the
    /// work is the same work on either machine: the same arithmetic over the
    /// same two million pixels. The absolute scale changes between machines;
    /// what the ratio is comparing does not.
    ///
    /// The ratio is not a proof either. It assumes the two backgrounds scale
    /// the same way from this machine to that one, and they do not exactly —
    /// Digital Rain is dominated by glyph rasterisation and cache-missing
    /// scattered writes, Aurora by a linear sweep a small in-order core will
    /// handle relatively better than a desktop does. **The absolute figure
    /// still has to come from the board, and as of this writing it has not
    /// been taken for any of the four.**
    ///
    /// `update` and `draw` are timed separately because they answer different
    /// questions. Life's `update` is where its automaton lives and it runs
    /// four times a second rather than thirty, so a combined number would
    /// hide the thing most worth knowing about it.
    #[test]
    #[ignore = "a timing measurement, not a correctness check — see the doc comment"]
    fn what_each_background_costs_relative_to_digital_rain() {
        use std::time::Instant;

        let rain_face =
            Typeface::load_embedded(18.0).expect("assets/AuraRain.ttf ships with this crate");
        let stride = WIDTH * 4;
        let mut bytes = vec![0u8; (stride * HEIGHT) as usize];

        let mut measured: Vec<(String, f64, f64, f64)> = Vec::new();
        for row in &ROWS {
            let mut saver = Screensaver::new_with_rng(
                row.background,
                WIDTH as usize,
                HEIGHT as usize,
                Mt19937::new(SEED),
            );
            {
                let mut surface =
                    Surface::over(&mut bytes, WIDTH, HEIGHT, stride).expect("1920x1080");
                for _ in 0..TIMING_WARMUP {
                    saver.update(FRAME_STEP_SECONDS);
                    render_frame(&saver, &mut surface, &rain_face, &mut Vec::new())
                        .expect("a catalogue row draws");
                }
            }

            let mut update_nanos = 0u128;
            let mut draw_nanos = 0u128;
            {
                let mut surface =
                    Surface::over(&mut bytes, WIDTH, HEIGHT, stride).expect("1920x1080");
                for _ in 0..TIMED_FRAMES {
                    let at = Instant::now();
                    saver.update(FRAME_STEP_SECONDS);
                    update_nanos += at.elapsed().as_nanos();

                    let at = Instant::now();
                    render_frame(&saver, &mut surface, &rain_face, &mut Vec::new())
                        .expect("a catalogue row draws");
                    draw_nanos += at.elapsed().as_nanos();
                }
            }

            // **How much of the screen the last frame actually lit.** Not a
            // performance number — a diagnostic for reading the ones beside
            // it. A background that costs what a plain fill costs while
            // inking 0.1% of the screen is a background whose drawing never
            // happened, which is precisely the mistake `TIMING_WARMUP`
            // documents; this column is what makes that visible in the table
            // rather than inferrable from it.
            let inked = bytes
                .chunks_exact(4)
                .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
                .count();
            #[allow(clippy::cast_precision_loss)]
            let ink = inked as f64 / f64::from(WIDTH * HEIGHT) * 100.0;

            let per_frame = |nanos: u128| -> f64 {
                #[allow(clippy::cast_precision_loss)]
                let total = nanos as f64;
                total / f64::from(TIMED_FRAMES) / 1_000_000.0
            };
            measured.push((
                format!("{}/{}", row.background_id, row.theme_id),
                per_frame(update_nanos),
                per_frame(draw_nanos),
                ink,
            ));
        }

        // The reference: the row `SPEC-04` §10's 12.4 ms belongs to.
        let total_of = |wanted: &str| -> f64 {
            measured
                .iter()
                .find(|(name, _, _, _)| name == wanted)
                .map(|(_, update, draw, _)| update + draw)
                .unwrap_or_else(|| panic!("{wanted} is a catalogue row"))
        };
        let reference = total_of("digital-rain/phosphor-green");
        // **The second reference, and for the full-screen backgrounds it is
        // the more honest one.** Solid Black is a single `draw::fill` over
        // the whole surface and nothing else, so it measures this machine's
        // memory write bandwidth and no arithmetic at all. A background's
        // multiple of *that* is how much computing it does per pixel, which
        // is the part that scales with the CPU rather than with the memory
        // bus — and the two scale very differently between a desktop and a
        // Raspberry Pi.
        let fill = total_of("solid/black");

        eprintln!();
        eprintln!(
            "1920x1080, {TIMED_FRAMES} frames after {TIMING_WARMUP} warm-up, THIS MACHINE — \
             not the board. The ratios are the transferable numbers; the milliseconds are not."
        );
        eprintln!(
            "{:<34} {:>10} {:>10} {:>10} {:>8} {:>8} {:>7}",
            "background/theme", "update ms", "draw ms", "total ms", "x rain", "x fill", "ink %"
        );
        for (name, update, draw, ink) in &measured {
            eprintln!(
                "{:<34} {update:>10.3} {draw:>10.3} {:>10.3} {:>8.2} {:>8.2} {ink:>7.2}",
                name,
                update + draw,
                (update + draw) / reference,
                (update + draw) / fill
            );
        }
        eprintln!();
        eprintln!(
            "Digital Rain is 12.4 ms of a 33.3 ms budget ON THE BOARD (SPEC-04 §10). \
             Scaling by the ratios above is an ESTIMATE; no board measurement has been taken \
             for any of the four backgrounds T8 added."
        );

        // **The rows that cost more than the reference, named rather than
        // left in the table to be scrolled past.** T8 calls a background that
        // misses the budget a defect, and a background that costs several
        // times the one known measurement is the thing a reader of this
        // output most needs to see.
        let mut dearer: Vec<String> = Vec::new();
        for (name, update, draw, _) in &measured {
            let ratio = (update + draw) / reference;
            if ratio > 1.1 {
                dearer.push(format!(
                    "  {name}: {ratio:.2}x rain ({:.1}x a bare full-screen fill)",
                    (update + draw) / fill
                ));
            }
        }
        if dearer.is_empty() {
            eprintln!("Every background is at or below Digital Rain on this machine.");
        } else {
            eprintln!();
            eprintln!("DEARER THAN DIGITAL RAIN — these need a measurement on the board:");
            for line in &dearer {
                eprintln!("{line}");
            }
            eprintln!();
            // **Why the obvious arithmetic is not printed here.** Multiplying
            // a ratio by 12.4 ms looks like an estimate of the board and is
            // worse than no number, because the two ends of the ratio do not
            // scale together. Digital Rain's 12.4 ms on the board is mostly
            // glyph rasterisation and scattered blended writes; a full-screen
            // fill of 8.29 MB is memory bandwidth, and a Raspberry Pi has
            // proportionally far less of that than this machine does, so the
            // fill is a much larger share of every number on the board than
            // it is here. Scaling naively therefore overstates Aurora and
            // Contours — by how much, nobody in this repository can say.
            eprintln!(
                "Do NOT multiply these ratios by 12.4 ms and call it a board measurement. The \
                 board's memory bandwidth is a far larger share of every figure there than it \
                 is here, and Digital Rain's cost there is mostly glyph work that does not \
                 scale like a linear sweep. The 'x fill' column is the compute-bound part and \
                 is the better guide, and neither is a measurement. Put it on a board."
            );
        }

        // **Not a budget gate, and deliberately not dressed up as one.**
        //
        // The honest thing this test can assert is a *regression tripwire*:
        // an edit that makes one of these several times more expensive than
        // it is today should fail rather than be discovered on a wall. It
        // cannot assert the 33.3 ms budget, because the budget belongs to a
        // Raspberry Pi and this is not one — see the doc comment. An earlier
        // draft asserted `ratio < 2.0` and it failed on Aurora at 3.5x, which
        // is a real finding about Aurora and was not a reason to move the
        // threshold until it passed.
        //
        // Ten is chosen as comfortably above every measurement taken
        // (the dearest is Contours at about 4.7x) and comfortably below
        // anything that would indicate the drawing had stopped being linear
        // in the pixel count.
        const REGRESSION_TRIPWIRE: f64 = 10.0;
        for (name, update, draw, _) in &measured {
            let ratio = (update + draw) / reference;
            assert!(
                ratio < REGRESSION_TRIPWIRE,
                "{name} costs {ratio:.2}x Digital Rain on this machine, past the \
                 {REGRESSION_TRIPWIRE:.0}x tripwire. This is not a budget failure — only a \
                 board can say that — but it is far outside what this background measured at \
                 when it was written, so something has changed shape."
            );
        }
    }
}
