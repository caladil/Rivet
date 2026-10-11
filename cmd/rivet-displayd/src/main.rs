//! `rivet-displayd` — **the program that owns the GPU and the screen.**
//!
//! `ADR-058` §4 split it out of `rivet-launcher` for one reason, and the
//! reason is a category rather than a feature. The maintainer, on a draft that
//! listed the launcher's being unable to fail as a cost, ruled that it is not
//! one: system programs often cannot be allowed to fail (.rulings/ R-310).
//! `rivet-initd` is PID 1, `rivet-sysd` is the supervisor, and neither may
//! crash — nobody records that as a drawback of having them.
//!
//! So the part that holds the screen is separated from the part that draws a
//! menu, and the split is where the risk is:
//!
//! | | job | may it fail? |
//! | :-- | :-- | :-- |
//! | **`rivet-displayd`** | owns the GPU and KMS, hands out surfaces, composites, page-flips | **No.** |
//! | **`rivet-launcher`** | draws the menu and asks the supervisor to start an application | **Yes.** Restart it; the screen survives |
//!
//! # It did not write a compositor
//!
//! `ADR-058` §5, ruled 2026-09-20 after ~600 lines of the wrong answer had
//! already been written: `ADR-009`'s table makes **an existing Rust crate the
//! default** and writing one the exception, and that test was skipped. The
//! maintainer asked the question that undid it — whether a well-made, tested
//! Rust drop-in already existed — and the answer that decided it was his: if he
//! ever adopts a desktop it would almost certainly be COSMIC first, and COSMIC
//! is built on Smithay, so Smithay is the one choice that fits (.rulings/
//! R-313).
//!
//! `cosmic-comp` is built on Smithay. So adopting COSMIC later and adopting
//! Smithay now are **one decision taken at two times**, and a hand-rolled
//! compositor would have made the later one a rewrite instead of an addition.
//!
//! # How it is written
//!
//! To `ADR-017` §3's standard, which is PID 1's: **nothing unwinds out of
//! `main`.** Every fallible thing below answers with a `Result` carrying a
//! sentence for the journal, `Cargo.toml` denies `unwrap`, `expect`, `panic`
//! and `indexing_slicing`, and `main` returns an exit code rather than calling
//! `exit`, so every destructor runs and DRM master is dropped on the way out.

mod capture;
mod clients;
mod hotplug;
mod pacing;
mod screen;
mod qoi;
mod state;

use smithay::backend::renderer::ImportDma;
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};
use smithay::reexports::wayland_server::Display;

use state::Rivet;

/// The name this program answers to in the journal, once, so every line agrees.
const ME: &str = "rivet-displayd";

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => {
            println!("{ME}: the screen was released and this process is done");
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            // **The journal is the only witness a headless board has.** A
            // failure that prints nothing here is a black screen and no
            // explanation, which this project has paid for more than once.
            println!("{ME}: STOPPING — {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    println!("{ME}: starting, pid {}", std::process::id());

    // **The GPU first; the display whenever it comes.** `.rulings/` R-236: on
    // a kiosk this program waits indefinitely for a screen, and a client may
    // connect before one exists. So what is opened here is the device and the
    // renderer, which every client needs whether or not anything is plugged
    // in — and a machine with no device that can drive a display at all is
    // still a refusal, because no amount of waiting fixes that.
    let (kms, drm_events) = screen::Kms::open()?;
    println!("{ME}: driving displays from {}", kms.path());

    let mut display: Display<Rivet> = Display::new()
        .map_err(|e| format!("the Wayland display could not be created: {e}"))?;
    let handle = display.handle();

    // **The formats this renderer will actually accept**, asked of the
    // renderer rather than declared — `.rulings/` R-206. `Kms` is built
    // before this point precisely so the list can come off the GLES renderer
    // that will be asked to honour it.
    let dmabuf_formats: Vec<_> = kms.renderer.dmabuf_formats().into_iter().collect();
    println!(
        "{ME}: dma-buf import on, {} format/modifier pair(s)",
        dmabuf_formats.len()
    );
    let state = Rivet::new(&handle, dmabuf_formats);

    let mut event_loop: EventLoop<Loop> = EventLoop::try_new()
        .map_err(|e| format!("the event loop could not be created: {e}"))?;
    let signal = event_loop.get_signal();
    let loop_handle = event_loop.handle();

    clients::wire(&loop_handle, &mut display)?;

    // **The kernel says when a display arrives or leaves** — `.rulings/`
    // R-212. Joined BEFORE the first look at the connectors, so a display
    // plugged in between the two is announced rather than missed.
    //
    // Without it this compositor still drives whatever is plugged in now, and
    // says loudly that it will not notice a change: a screen that appears
    // later stays dark until a restart, which is the behaviour hot-plug
    // exists to remove, and not a reason to refuse to draw on the one there.
    match hotplug::Uevents::open() {
        Ok(uevents) => {
            loop_handle
                .insert_source(
                    Generic::new(uevents, Interest::READ, Mode::Level),
                    |_readiness, uevents, data| {
                        if uevents.drain() {
                            data.rescan = true;
                        }
                        Ok(PostAction::Continue)
                    },
                )
                .map_err(|e| format!("the uevent socket could not be watched: {e}"))?;
            // Said when it works as well as when it does not, so a journal
            // answers whether hot-plug is on rather than leaving it to the
            // absence of a line.
            println!("{ME}: hot-plug is on — listening for the kernel's display events");
        }
        Err(why) => println!(
            "{ME}: HOT-PLUG IS OFF — {why}. A display connected or disconnected from now on \
             will not be noticed until this program restarts"
        ),
    }

    // **Vblank drives the frame loop.** A timer would be a second opinion
    // about when the screen is ready, and the two would disagree under load in
    // exactly the conditions that make a display look broken.
    loop_handle
        .insert_source(drm_events, |event, _meta, data| {
            match event {
                smithay::backend::drm::DrmEvent::VBlank(crtc) => {
                    if let Err(why) = data.kms.submitted(crtc) {
                        println!("{ME}: the flip was not acknowledged — {why}");
                    }
                }
                smithay::backend::drm::DrmEvent::Error(why) => {
                    println!("{ME}: the device reported {why}");
                }
            }
            // A vblank is the only moment a new frame may be queued, so ask
            // for one unconditionally and let `Kms::frame` decide that nothing
            // changed.
            data.state.redraw = true;
        })
        .map_err(|e| format!("the DRM event source could not be inserted: {e}"))?;

    let mut data = Loop {
        state,
        kms,
        capture: None,
        capture_busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        // The first pass looks at the connectors: whatever is plugged in at
        // start is found the same way as whatever is plugged in later.
        rescan: true,
    };
    let mut stalls = pacing::StallLog::default();

    println!("{ME}: composing on {}", rivet_core::compositor::socket_path());

    while data.state.running {
        let pass = std::time::Instant::now();

        // **A hot-plug reads the EDID on this thread**, which is the kernel's
        // I2C and not ours to hurry; it happens once per plug, and its share of
        // a held pass is named in the stall line below.
        if std::mem::take(&mut data.rescan) {
            follow_the_screen(&mut data);
        }
        let probed = pass.elapsed();

        // **Answer every parked dmabuf before anything else touches the
        // renderer** — `Rivet::pending_imports` says why they are parked.
        //
        // Unconditional and first: a notifier that is never answered leaves the
        // client blocked in `zwp_linux_buffer_params_v1::create`, which looks
        // like an application that started and drew nothing.
        for (dmabuf, notifier) in std::mem::take(&mut data.state.pending_imports) {
            match data.kms.renderer.import_dmabuf(&dmabuf, None) {
                Ok(_) => {
                    if notifier.successful::<Rivet>().is_err() {
                        // The client died between offering the buffer and
                        // this moment. Not a fault of anybody's.
                    }
                }
                Err(why) => {
                    println!("{ME}: a client's dma-buf could not be imported — {why}");
                    notifier.failed();
                }
            }
        }
        let imported = pass.elapsed();

        // **A capture's copy, read back once it has had time to finish** —
        // `capture`'s module note says why it is not read at once. What is left
        // after the read goes to a thread of its own.
        if data.capture.as_ref().is_some_and(|(pending, _)| pending.ready())
            && let Some((pending, stream)) = data.capture.take()
        {
            match capture::collect(&mut data.kms.renderer, pending) {
                Ok(frame) => clients::complete_capture(frame, stream),
                Err(why) => {
                    println!("{ME}: the capture failed — {why}");
                    clients::answer_off_thread(stream, format!("failed {why}\n"));
                }
            }
        }
        let collected = pass.elapsed();

        if data.state.redraw {
            data.state.redraw = false;
            match data.kms.frame(&mut data.state) {
                Ok(true) => data.state.send_frame_callbacks(),
                Ok(false) => {}
                // **A failed frame is not a failed program.** A device that is
                // momentarily busy, or an element whose buffer went away under
                // us, must not take the screen down with it; the next vblank
                // asks again.
                Err(why) => println!("{ME}: frame not shown — {why}"),
            }
        }
        let framed = pass.elapsed();

        display
            .flush_clients()
            .map_err(|e| format!("clients could not be flushed: {e}"))?;

        if let Err(why) = event_loop.dispatch(
            std::time::Duration::from_millis(16),
            &mut data,
        ) {
            println!("{ME}: the event loop refused a dispatch — {why}");
            signal.stop();
            data.state.running = false;
        }
        let dispatched = pass.elapsed();

        display.dispatch_clients(&mut data.state).map_err(|e| {
            format!("a client could not be dispatched and the display is unusable: {e}")
        })?;

        // **A pass that held the thread is a screen that stood still** —
        // `pacing`'s module note. The four parts say what held it; the event
        // dispatch includes the loop's own wait of up to 16 ms, and a capture
        // being started, which runs inside it.
        let took = pass.elapsed();
        if took >= pacing::STALL
            && let Some(unlogged) = stalls.note(std::time::Instant::now())
        {
            println!(
                "{ME}: the compositor's thread was held {} ms — display probe {} ms, dma-buf \
                 imports {} ms, capture read-back {} ms, frame {} ms, event dispatch {} ms, \
                 client dispatch {} ms{}",
                took.as_millis(),
                probed.as_millis(),
                imported.saturating_sub(probed).as_millis(),
                collected.saturating_sub(imported).as_millis(),
                framed.saturating_sub(collected).as_millis(),
                dispatched.saturating_sub(framed).as_millis(),
                took.saturating_sub(dispatched).as_millis(),
                if unlogged == 0 {
                    String::new()
                } else {
                    format!("; {unlogged} more since the last line, not logged")
                }
            );
        }
    }

    Ok(())
}

/// What every event source is handed. Two halves, kept apart on purpose:
/// `state` is what Wayland's handlers may touch, `kms` is the screen and is
/// reachable only from the frame loop.
pub struct Loop {
    pub state: Rivet,
    pub kms: screen::Kms,
    /// A capture on the readback fallback between its two pieces on this
    /// thread, with the connection that asked for it — `capture`'s module note.
    pub capture: Option<(capture::Pending, std::os::unix::net::UnixStream)>,
    /// Set while a capture drawn into a linear buffer is being finished on a
    /// thread of its own, so a second one is refused rather than racing it.
    pub capture_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set when the kernel announced a display hot-plug, and at start; the
    /// frame loop asks the connectors again and clears it.
    pub rescan: bool,
}

/// **Ask the connectors what is plugged in, and make the Wayland side agree**
/// — `.rulings/` R-212 and R-236.
///
/// Every outcome is a journal line, because a screen that appeared or vanished
/// is the first thing anybody reading a kiosk's journal will be looking for.
fn follow_the_screen(data: &mut Loop) {
    match data.kms.rescan() {
        Ok(screen::Changed::Nothing) => {
            if data.kms.head.is_none() && data.state.output.is_none() {
                println!("{ME}: no display is connected; waiting for one");
                capture::withdraw();
            }
        }
        Ok(screen::Changed::Now(output)) => {
            match output.current_mode() {
                Some(mode) => println!(
                    "{ME}: {} at {}x{} @ {:.2} Hz",
                    output.name(),
                    mode.size.w,
                    mode.size.h,
                    f64::from(mode.refresh) / 1000.0
                ),
                None => println!("{ME}: {} is being driven", output.name()),
            }
            data.state.screen_changed(Some(output));
        }
        Ok(screen::Changed::Gone) => {
            let name = data.state.output.as_ref().map(Output::name).unwrap_or_default();
            println!(
                "{ME}: the display {name} was disconnected; every client keeps running, and \
                 frames pause until a display returns"
            );
            data.state.screen_changed(None);
            capture::withdraw();
        }
        Err(why) => println!("{ME}: the display could not be brought up — {why}"),
    }
}
