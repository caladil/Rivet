//! **Does a GBM buffer object Mesa destroys take its `wl_buffer` with it?**
//!
//! Mesa keeps up to four buffer objects for a surface. It allocates another
//! when the compositor holds every one it has, and destroys the oldest spare
//! after a thousand frames in which the surface had more free than it needed.
//! `rivet_sdk::gpu` gives each object one `wl_buffer`, and the compositor holds
//! a `wl_buffer`'s dma-buf until the client destroys it. So if the SDK keeps a
//! `wl_buffer` past its object, every destroy-and-reallocate leaves the
//! compositor holding one more frame of GPU memory for as long as this client
//! runs.
//!
//! # What it does
//!
//! [`CYCLES`] times over:
//!
//! 1. **A burst** — frames back to back with nothing between them, for
//!    [`BURST`]. The compositor cannot release a buffer as fast as one is
//!    committed, so Mesa allocates up to its fourth object.
//! 2. **A calm stretch** at 30 fps, `aura-renderer`'s rate, for [`CALM`] — more
//!    than a thousand frames, so Mesa finds a spare and destroys it.
//!
//! After each phase it reads how much GPU memory its own render node holds
//! (`/proc/self/fdinfo`, `drm-total-memory`) and journals it in frames, which
//! is how many buffer objects Mesa has at that moment. That number rising in
//! a burst and falling in the calm is the cycle happening; how many dma-bufs
//! the compositor holds for this client afterwards is the answer, and is read
//! from the compositor's side — which is why it ends by [`HOLD`]ing its
//! surface at two frames a second rather than exiting: a client that exits
//! takes every one of its buffers out of the compositor with it.
//!
//! What it draws is a clear of the whole surface in a colour that changes
//! per phase: nothing about the picture is in question, only the buffers.

use std::time::{Duration, Instant};

/// How many burst-and-calm cycles to run.
const CYCLES: u32 = 6;

/// How long a burst lasts. A second is hundreds of frames when each is a clear.
const BURST: Duration = Duration::from_secs(2);

/// How long the calm lasts: comfortably more than Mesa's thousand frames at
/// [`FRAME`], which is 33.3 s.
const CALM: Duration = Duration::from_secs(45);

/// `aura-renderer`'s frame budget, 30 fps.
const FRAME: Duration = Duration::from_micros(33_333);

/// How long the surface is held after the last cycle, so the compositor's
/// side can be counted while this client is still connected.
const HOLD: Duration = Duration::from_secs(90);

/// The pace of the hold: slow enough that a hundred and eighty frames cannot
/// reach Mesa's thousand, so nothing is trimmed while it is being counted.
const HOLD_FRAME: Duration = Duration::from_millis(500);

/// `GL_COLOR_BUFFER_BIT`.
const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;

#[link(name = "GLESv2")]
unsafe extern "C" {
    fn glClearColor(red: f32, green: f32, blue: f32, alpha: f32);
    fn glClear(mask: u32);
}

fn main() {
    let journal = rivet_sdk::journal::Journal::for_service("gpu-buffers");
    let mut screen = match rivet_sdk::gpu::take() {
        Ok(screen) => screen,
        Err(why) => {
            journal.error(&format!("no GPU surface: {why}"));
            println!("gpu-buffers: no GPU surface: {why}");
            return;
        }
    };
    let frame_kib = screen.width * screen.height * 4 / 1024;
    say(&journal, &format!("drawing on {}; one frame is {frame_kib} KiB", screen.what));
    say(&journal, &format!("start: {}", held(frame_kib)));

    for cycle in 1..=CYCLES {
        let burst = run(&mut screen, BURST, None, [0.6, 0.1, 0.1]);
        say(&journal, &format!("cycle {cycle} burst: {burst}; {}", held(frame_kib)));
        let calm = run(&mut screen, CALM, Some(FRAME), [0.1, 0.1, 0.6]);
        say(&journal, &format!("cycle {cycle} calm: {calm}; {}", held(frame_kib)));
        if screen.closed() {
            say(&journal, "the compositor closed the surface");
            return;
        }
    }
    say(&journal, &format!("holding the surface for {}s to be counted", HOLD.as_secs()));
    let hold = run(&mut screen, HOLD, Some(HOLD_FRAME), [0.1, 0.5, 0.1]);
    say(&journal, &format!("finished: hold {hold}; {}", held(frame_kib)));
}

/// Draw and present for `length`, every `pace` or as fast as possible.
fn run(
    screen: &mut rivet_sdk::gpu::Screen,
    length: Duration,
    pace: Option<Duration>,
    colour: [f32; 3],
) -> String {
    let began = Instant::now();
    let (mut frames, mut refused) = (0u64, 0u64);
    let mut first_refusal = None;
    while began.elapsed() < length && !screen.closed() {
        let frame_began = Instant::now();
        if let Err(why) = screen.make_current() {
            return format!("the context would not come current: {why}");
        }
        // SAFETY: the surface's context was made current on this thread just
        // above, which is all a clear needs.
        unsafe {
            glClearColor(colour[0], colour[1], colour[2], 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        match screen.present() {
            Ok(()) => frames += 1,
            Err(why) => {
                refused += 1;
                first_refusal.get_or_insert(why);
            }
        }
        if let Some(pace) = pace {
            if let Some(left) = pace.checked_sub(frame_began.elapsed()) {
                std::thread::sleep(left);
            }
        }
    }
    let mut line = format!("{frames} frames, {refused} refused");
    if let Some(why) = first_refusal {
        line.push_str(&format!(" (first: {why})"));
    }
    line
}

/// What this process's render node holds, in frames — how many buffer
/// objects Mesa has, give or take its small allocations.
fn held(frame_kib: usize) -> String {
    match render_node_kib() {
        Some(kib) => {
            #[allow(clippy::cast_precision_loss)]
            let frames = kib as f64 / frame_kib.max(1) as f64;
            format!("render node holds {kib} KiB, {frames:.2} frames")
        }
        None => "render node memory unreadable".to_string(),
    }
}

/// `drm-total-memory` for the first descriptor of this process that is a
/// render node.
fn render_node_kib() -> Option<u64> {
    for entry in std::fs::read_dir("/proc/self/fd").ok()?.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if !target.to_string_lossy().starts_with("/dev/dri/renderD") {
            continue;
        }
        let info = std::fs::read_to_string(format!(
            "/proc/self/fdinfo/{}",
            entry.file_name().to_string_lossy()
        ))
        .ok()?;
        return info.lines().find_map(|line| {
            let value = line.strip_prefix("drm-total-memory:")?.trim();
            value.strip_suffix("KiB")?.trim().parse().ok()
        });
    }
    None
}

/// One line to the journal and the same line to stdout.
fn say(journal: &rivet_sdk::journal::Journal, line: &str) {
    journal.info(line);
    println!("gpu-buffers: {line}");
}
