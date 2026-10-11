//! **A screen handed over by the compositor** — `ADR-058` §5.
//!
//! This is [`crate::screen`]'s replacement, and it is the same five things: a
//! width, a height, a stride, a sentence about the display, and [`Screen::blit`].
//! An application that draws a frame into a `&[u8]` and hands it over does not
//! change at all — which is the whole reason the seam was put here.
//!
//! # Why the SDK and not the application
//!
//! Ruled 2026-09-20. Asked whether each kiosk application should speak Wayland
//! itself or be handed a surface by the SDK, the maintainer chose the SDK
//! (`.rulings/` R-318). `ADR-058` §5 records it and
//! `docs/design-candidates/Aura-as-a-Wayland-Client-2026-09-20.md` is the three
//! options.
//!
//! The reasoning is §3.1 read one layer up. The argument for a compositor was
//! that **an application should be handed a result rather than a mechanism**;
//! handing it a Wayland connection instead of a DRM lease would be a smaller
//! version of the same mistake.
//!
//! # What an application gives up, and it is not small
//!
//! | | |
//! | :-- | :-- |
//! | `devices = ["display"]` | **gone.** No DRM node, no render node, no framebuffer, and none of `ADR-058` §1's five confinement grants |
//! | `connects = ["rivet-displayd:wayland"]` | **added.** An endpoint nobody provides is a refusal the supervisor reports by name, where a missing socket would be a black screen and silence |
//! | `Handed::capture` | **dead.** `ADR-058` §5 moved capture to the compositor, which is the one program that can see the screen rather than one program's frame |
//!
//! # It is still a static binary
//!
//! `smithay-client-toolkit` with `default-features = false` is pure Rust — no
//! `xkbcommon`, no `calloop`. `cmd/rivet-launcher` proved it: `readelf -d`
//! reports no `NEEDED` entries at all. So this feature adds no loader and no C
//! library to an application, which is what made `ADR-003` §5 survive the move.

use std::os::unix::io::AsRawFd;

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::{registry_queue_init, GlobalList};
use smithay_client_toolkit::reexports::client::protocol::{wl_callback, wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, EventQueue, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::subcompositor::SubcompositorState;
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_subcompositor,
    delegate_xdg_shell,
    delegate_xdg_window, registry_handlers,
};

/// How long the compositor may be silent before it is asked whether it is
/// still there — at start-up, and after every frame.
///
/// **It is the one deadline this module has, and it bounds both waits.** At
/// start-up an answer resets it, because a compositor with no display is alive
/// and configures nothing until one arrives. After a frame there is nothing to
/// wait for but the answer itself, so a compositor that gives none within this
/// long has stopped, and the surface is closed so the application can exit and
/// be restarted ([`frame_round_trip`]). The same value serves both because the
/// question is the same: how long a live compositor can take to answer a sync.
///
/// **An application that got no configure has no size**, so it cannot draw a
/// first frame at all. It waits for one as long as the compositor answers —
/// `rivet-displayd` configures nothing until a display is plugged in
/// (`.rulings/` R-236) — and gives up only on a compositor that stops
/// answering, because a program blocked for ever on a hung socket is worse
/// than one that says why it stopped. Generous against a compositor that is
/// starting at the same moment, which is the ordinary case: the supervisor
/// starts `rivet-displayd` before anything that connects to it, but "before"
/// is an ordering and not a barrier.
const CONFIGURE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// A screen, handed over.
///
/// **Deliberately the same shape as [`crate::screen::Screen`]** — `width`,
/// `height`, `stride`, `what` and `blit` — so an application moves by changing
/// which one it asks for.
pub struct Screen {
    inner: Inner,
    queue: EventQueue<Inner>,
    handle: QueueHandle<Inner>,
    /// **Held so the round trip after each frame can be bounded** — it sends
    /// its own `wl_display.sync` rather than calling `EventQueue::roundtrip`,
    /// which waits for ever. See [`frame_round_trip`].
    conn: Connection,

    /// The surface's width in pixels, as the compositor configured it.
    pub width: usize,
    /// The surface's height in pixels.
    pub height: usize,
    /// Bytes per row. **`width * 4` here and not the kernel's choice**, which
    /// is the one number that differs in spirit from the DRM path: a
    /// shared-memory buffer's stride is the client's to pick, so this picks the
    /// tight one and an application's existing arithmetic is unchanged.
    pub stride: usize,
    /// Which compositor, how big, and how frames reach the screen — the record
    /// an application prints, exactly as `screen::Screen::what` is.
    pub what: String,
}

/// Ask the compositor for a surface.
///
/// # Errors
///
/// The socket, the connection, a missing global, a pool the compositor would
/// not create, or a configure that never arrived. Each names which, because the
/// journal is the only witness a headless board has.
pub fn take() -> Result<Screen, String> {
    let path = SOCKET;
    let stream = std::os::unix::net::UnixStream::connect(&path).map_err(|e| {
        format!(
            "{path}: {e} — this application may not have been granted the compositor. It is \
             granted by declaring connects = [\"rivet-displayd:wayland\"] in the manifest, and \
             rivet-displayd has to be enabled on this machine"
        )
    })?;

    // **The deadline starts here, before any Wayland traffic at all — B10.**
    // Registry discovery is a blocking round trip like any other, and a
    // compositor that accepts the connection and then falls silent must not
    // get an unbounded wait just because the silence started before the
    // configure loop below rather than inside it. `watchdog` is a second
    // handle on the same socket, kept so [`bounded_registry_init`] can force
    // that round trip to give up — see its own comment for why that is the
    // only lever this crate leaves for bounding it.
    let deadline = std::time::Instant::now() + CONFIGURE_PATIENCE;
    let watchdog = stream.try_clone().map_err(|e| {
        format!("{path}: the compositor socket could not be duplicated for the start-up deadline: {e}")
    })?;

    let conn = Connection::from_socket(stream)
        .map_err(|e| format!("{path}: the compositor would not talk Wayland: {e}"))?;
    let (globals, mut queue) = bounded_registry_init(&conn, watchdog, deadline)?;
    let handle = queue.handle();

    let compositor = CompositorState::bind(&globals, &handle)
        .map_err(|e| format!("no wl_compositor: {e} — this is not a compositor"))?;
    let shell = XdgShell::bind(&globals, &handle)
        .map_err(|e| format!("no xdg_shell: {e} — there is no way to ask for a window"))?;
    let shm = Shm::bind(&globals, &handle)
        .map_err(|e| format!("no wl_shm: {e} — there is no way to hand over pixels"))?;
    // **Bound here and used only if the application asks for an overlay.**
    // `CompositorState::bind` above does not give it — `wl_subcompositor` is a
    // separate global — and binding it at `take` rather than at
    // `take_overlay` means a compositor that lacks it is named once, here,
    // beside the other two, rather than at the first frame of an application
    // that had already started.
    let subcompositor = SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &handle)
        .map_err(|e| format!("no wl_subcompositor: {e} — there is no way to layer surfaces"))?;

    let surface = compositor.create_surface(&handle);
    let window = shell.create_window(surface, WindowDecorations::RequestServer, &handle);
    window.set_app_id("org.rivet.application");
    // **The first commit carries no buffer**, which is the protocol's rule: it
    // is what asks for the configure, and until that arrives this process does
    // not know how big it is.
    window.commit();

    // A pool sized for nothing yet; `blit` resizes it on the first frame.
    let pool = SlotPool::new(4, &shm)
        .map_err(|e| format!("the shared-memory pool could not be created: {e}"))?;

    let mut inner = Inner {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &handle),
        shm,
        pool,
        window,
        subcompositor,
        overlay: None,
        buffer: None,
        width: 0,
        height: 0,
        resized: true,
        closed: false,
    };

    // **Block until the compositor says how big.** Every other answer this
    // module gives is a lie until it has one. With no display plugged in that
    // is a wait for one, as long as it takes — [`wait_for_configure`] — and a
    // compositor that stops answering is still refused, as B10 requires.
    wait_for_configure(
        &conn,
        &mut queue,
        &mut inner,
        CONFIGURE_PATIENCE,
        |inner| inner.width != 0 && inner.height != 0,
        || {
            eprintln!(
                "rivet-sdk: the compositor has no display yet; this surface is sized and drawn \
                 when one is connected"
            );
        },
    )
    .map_err(|e| format!("{path}: {e}"))?;

    let (width, height) = (inner.width, inner.height);
    Ok(Screen {
        inner,
        queue,
        handle,
        conn,
        width,
        height,
        stride: width * 4,
        what: format!("{path}, {width}x{height}, wl_shm XRGB8888 (ADR-058)"),
    })
}

/// Bound `registry_queue_init` by `deadline` — B10.
///
/// The crate gives this call no timeout of its own. `registry_queue_init`
/// calls `Connection::roundtrip` internally, which polls the socket with
/// `None` (forever) and retries through `EINTR` in a loop, so not even a
/// signal breaks it out. The only lever left is the socket itself: a
/// watchdog thread shuts `watchdog` down — a second handle on the same
/// socket `conn` was built from — if `deadline` passes before
/// `registry_queue_init` returns on its own. A shutdown socket makes the
/// blocked poll return with an error instead of never returning, which is
/// what actually bounds it.
///
/// **The watchdog is disarmed before this function returns, either way.** A
/// thread left armed past that point would shut the connection down later,
/// during ordinary frame drawing, rather than during start-up — cancellation
/// is by dropping `cancel_tx`, which the standard library's
/// `mpsc::Receiver::recv_timeout` reports as `Disconnected` rather than
/// `Timeout`, and only `Timeout` arms the shutdown.
fn bounded_registry_init(
    conn: &Connection,
    watchdog: std::os::unix::net::UnixStream,
    deadline: std::time::Instant,
) -> Result<(GlobalList, EventQueue<Inner>), String> {
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let timer = std::thread::spawn(move || {
        if matches!(
            cancel_rx.recv_timeout(remaining),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            let _ = watchdog.shutdown(std::net::Shutdown::Both);
        }
    });

    let result = registry_queue_init::<Inner>(conn)
        .map_err(|e| format!("{SOCKET}: the compositor's globals could not be read: {e}"));

    drop(cancel_tx);
    let _ = timer.join();
    result
}

/// One bounded step of dispatch, in place of `EventQueue::blocking_dispatch` — B10.
///
/// `blocking_dispatch` polls the socket with no timeout of its own, so if
/// the compositor accepts the connection and then goes silent mid-wait, the
/// call inside it never returns — and the deadline check that wraps every
/// call to this function in `take`'s loop never runs again either, because
/// it never gets the chance to. This is the same prepare-read/poll/read
/// protocol `blocking_dispatch` uses internally
/// (`EventQueue::prepare_read`), with the poll bounded by the time left
/// until `deadline` rather than blocked forever, and the guard dropped —
/// which `wayland-backend` 0.3 documents as cancelling the prepared read —
/// rather than read from, when the deadline wins the race.
///
/// Returns `Ok(0)` on a timed-out poll: the same "nothing dispatched yet"
/// contract `dispatch_pending` already has, so the caller's own loop
/// re-checks `deadline` exactly as it already does around every call.
///
/// **Generic in `State`, though `take` only ever calls it with `Inner`.**
/// `EventQueue::dispatch_pending`/`prepare_read`/`flush` need no `Dispatch`
/// bound themselves — dispatch looks callbacks up by object id, already
/// registered when each proxy was created — so this needs none either. The
/// tests use that: they exercise the real prepare-read/poll/read path
/// against a real socket with `State = ()`, which needs no fake compositor
/// that can actually bind globals and create a surface.
pub(crate) fn bounded_dispatch<State>(
    queue: &mut EventQueue<State>,
    data: &mut State,
    deadline: std::time::Instant,
) -> Result<usize, String> {
    let dispatched = queue
        .dispatch_pending(data)
        .map_err(|e| format!("the compositor stopped answering: {e}"))?;
    if dispatched > 0 {
        return Ok(dispatched);
    }

    queue
        .flush()
        .map_err(|e| format!("the compositor stopped answering: {e}"))?;

    let Some(guard) = queue.prepare_read() else {
        // Another reader already owns the socket; there is nothing of ours
        // to poll on. Dispatch whatever it leaves behind.
        return queue
            .dispatch_pending(data)
            .map_err(|e| format!("the compositor stopped answering: {e}"));
    };

    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    let timeout_ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
    let readable = poll_readable(guard.connection_fd().as_raw_fd(), timeout_ms)
        .map_err(|e| format!("the compositor stopped answering: {e}"))?;
    if !readable {
        // Timed out (or, for an already-passed deadline, polled once without
        // blocking and found nothing). Dropping the guard cancels the
        // prepared read rather than reading blind past the deadline.
        drop(guard);
        return Ok(0);
    }

    match guard.read() {
        Ok(_) => {}
        Err(smithay_client_toolkit::reexports::client::backend::WaylandError::Io(e))
            if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => return Err(format!("the compositor stopped answering: {e}")),
    }

    queue
        .dispatch_pending(data)
        .map_err(|e| format!("the compositor stopped answering: {e}"))
}

/// A client state that can record the compositor going away, so the
/// application's next look at `closed()` says so.
pub(crate) trait Closes {
    /// Mark the surface closed. Never undone: a Wayland connection that has
    /// failed does not recover, and the way back is a new process.
    fn close(&mut self);
}

/// **Flush what is queued, read what has arrived, dispatch it — without
/// waiting**, and close the surface if the connection has gone.
///
/// `dispatch_pending` drains events that have **already been read** off the
/// socket; it does not read. A loop of flush-and-dispatch therefore sends
/// every frame and receives nothing, so `prepare_read` + `read` is here, and
/// `WouldBlock` from either half is the ordinary case — a full socket buffer,
/// or nothing arrived yet — and is not a failure.
///
/// **Any other failure is the compositor having gone, and it closes the
/// surface.** A broken connection never comes back: an application that only
/// reported the error would keep drawing into a dead socket for ever, while
/// its restart policy, which is the actual way back, waits for an exit that
/// never comes. Closing here is what lets `closed()` tell the application to
/// stop.
///
/// # Errors
///
/// The connection has failed. The surface is closed by the time this returns.
pub(crate) fn pump<State: Closes>(
    queue: &mut EventQueue<State>,
    state: &mut State,
) -> Result<(), String> {
    let pumped = pump_once(queue, state);
    if pumped.is_err() {
        state.close();
    }
    pumped
}

fn pump_once<State>(queue: &mut EventQueue<State>, state: &mut State) -> Result<(), String> {
    use smithay_client_toolkit::reexports::client::backend::WaylandError;
    match queue.flush() {
        Ok(()) => {}
        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => return Err(format!("the compositor stopped listening: {e}")),
    }
    if let Some(guard) = queue.prepare_read() {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("the compositor stopped answering: {e}")),
        }
    }
    queue
        .dispatch_pending(state)
        .map(|_| ())
        .map_err(|e| format!("the compositor stopped answering: {e}"))
}

/// **Wait for the first configure for as long as the compositor is alive** —
/// `.rulings/` R-236, and B10's bound kept.
///
/// `rivet-displayd` does not configure a window until a display is plugged
/// in, and on a kiosk that may be never or hours away; an application started
/// without a screen is meant to wait for one, not exit. But a compositor that
/// accepted the connection and then hung looks the same from here — silence —
/// and B10 is that an application must not wait on a hung compositor for ever.
///
/// So silence is a question, not a verdict. Each time `patience` passes with no
/// configure, the compositor is asked a `wl_display.sync`: one that answers
/// within `patience` is alive and waiting for a screen, and the wait goes on;
/// one that does not is refused exactly as before. `waiting` is called once,
/// the first time the answer is *alive*, so the application can say why it has
/// not started drawing.
///
/// # Errors
///
/// The connection failed, or the compositor stopped answering.
pub(crate) fn wait_for_configure<State>(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    data: &mut State,
    patience: std::time::Duration,
    configured: impl Fn(&State) -> bool,
    mut waiting: impl FnMut(),
) -> Result<(), String>
where
    State: Dispatch<wl_callback::WlCallback, Answered> + 'static,
{
    let mut deadline = std::time::Instant::now() + patience;
    let mut said = false;
    while !configured(data) {
        if std::time::Instant::now() > deadline {
            if !answers(conn, queue, data, std::time::Instant::now() + patience)? {
                return Err(format!(
                    "the compositor accepted a surface, never configured it, and then stopped \
                     answering for {}s",
                    patience.as_secs()
                ));
            }
            if !said {
                waiting();
                said = true;
            }
            deadline = std::time::Instant::now() + patience;
        }
        bounded_dispatch(queue, data, deadline)?;
    }
    Ok(())
}

/// **The round trip after a frame, bounded by `patience`.**
///
/// One `wl_display.sync` per frame, dispatching whatever arrives with its
/// answer — the configure, the buffer release, the frame callback — exactly as
/// `EventQueue::roundtrip` does, except that a compositor which does not answer
/// within `patience` ends the wait instead of holding the client for ever.
///
/// **A display that is unplugged never trips it.** `rivet-displayd` withholds
/// frame callbacks while it has no display (`ADR-058` §5) and answers a sync
/// regardless, so this wait, which is for the sync and never for a frame
/// callback, ends in an answer whether a display is connected or not. Only a
/// compositor that has stopped answering altogether runs it out.
///
/// # Errors
///
/// The connection failed, or the compositor did not answer within `patience`.
/// The text says what was being waited for, because it is the journal record a
/// stuck kiosk leaves behind.
pub(crate) fn frame_round_trip<State>(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    data: &mut State,
    patience: std::time::Duration,
) -> Result<(), String>
where
    State: Dispatch<wl_callback::WlCallback, Answered> + 'static,
{
    if answers(conn, queue, data, std::time::Instant::now() + patience)? {
        Ok(())
    } else {
        Err(format!(
            "the compositor did not answer the round trip after a frame within {}s: this \
             surface was waiting for {SOCKET} to acknowledge a committed frame, which it does \
             with or without a display connected",
            patience.as_secs()
        ))
    }
}

/// The user data of a `wl_display.sync` sent by [`answers`]: set when the
/// compositor answers it.
pub(crate) struct Answered(pub std::sync::Arc<std::sync::atomic::AtomicBool>);

/// Whether the compositor answers a `wl_display.sync` before `deadline`.
///
/// # Errors
///
/// The connection failed while waiting.
pub(crate) fn answers<State>(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    data: &mut State,
    deadline: std::time::Instant,
) -> Result<bool, String>
where
    State: Dispatch<wl_callback::WlCallback, Answered> + 'static,
{
    use std::sync::atomic::Ordering;

    let answered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _sync = conn
        .display()
        .sync(&queue.handle(), Answered(std::sync::Arc::clone(&answered)));
    while !answered.load(Ordering::SeqCst) {
        if std::time::Instant::now() > deadline {
            return Ok(false);
        }
        bounded_dispatch(queue, data, deadline)?;
    }
    Ok(true)
}

/// Mark an [`Answered`] when its sync is done. Written once as a macro because
/// every client state in this crate needs the same three lines, and a trait
/// impl cannot be made generic over the foreign `Dispatch`.
macro_rules! answers_sync {
    ($state:ty) => {
        impl smithay_client_toolkit::reexports::client::Dispatch<
                smithay_client_toolkit::reexports::client::protocol::wl_callback::WlCallback,
                $crate::composited::Answered,
            > for $state
        {
            fn event(
                _state: &mut Self,
                _proxy: &smithay_client_toolkit::reexports::client::protocol::wl_callback::WlCallback,
                event: smithay_client_toolkit::reexports::client::protocol::wl_callback::Event,
                data: &$crate::composited::Answered,
                _conn: &smithay_client_toolkit::reexports::client::Connection,
                _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
            ) {
                if let smithay_client_toolkit::reexports::client::protocol::wl_callback::Event::Done {
                    ..
                } = event
                {
                    data.0.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    };
}
// Re-exported by path for `crate::gpu`; this module reaches it by textual scope.
#[cfg(feature = "composited-gpu")]
pub(crate) use answers_sync;

/// Poll one fd for readability, bounded by `timeout_ms` — `libc::poll`'s own
/// contract: `-1` blocks forever, `0` returns immediately, and every other
/// value bounds the wait to that many milliseconds.
///
/// **The one raw FFI call this module makes**, and it exists because
/// `wayland-client` 0.31 gives no bounded alternative to reading an
/// `EventQueue::prepare_read` guard — see `bounded_dispatch`.
#[allow(unsafe_code)]
fn poll_readable(fd: std::os::unix::io::RawFd, timeout_ms: i32) -> std::io::Result<bool> {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: `pfd` is one well-formed `pollfd` living on this stack frame
    // for the duration of the call, and `nfds` (1) matches the single entry
    // handed over.
    let ready = unsafe { libc::poll(&raw mut pfd, 1, timeout_ms) };
    if ready < 0 {
        let err = std::io::Error::last_os_error();
        return if err.kind() == std::io::ErrorKind::Interrupted { Ok(false) } else { Err(err) };
    }
    Ok(ready > 0)
}

impl Screen {
    /// Put a finished frame on the screen.
    ///
    /// `canvas` is `width * height * 4` bytes of **ARGB8888 little-endian** —
    /// `[b, g, r, x]` in memory, which is what `crate::draw` already writes and
    /// what the DRM path already wanted. That agreement is why an application's
    /// drawing code does not change.
    ///
    /// **Short input is drawn rather than refused.** A caller that hands over
    /// fewer bytes than the surface holds gets what it gave, and the rest of
    /// the frame is whatever was there. `screen::Screen::blit` takes the same
    /// position, and the reason is the same: a kiosk that went black because
    /// one frame was miscounted is a worse answer than one that looks wrong.
    pub fn blit(&mut self, canvas: &[u8]) {
        if let Err(why) = self.paint(canvas) {
            // **A note and not a failure.** `blit` returns nothing because the
            // DRM one returns nothing, and an application that cannot draw one
            // frame should go on running — the next one usually works.
            eprintln!("rivet-sdk: the frame was not handed over: {why}");
            return;
        }
        // One round trip per frame, which is what delivers the configure, the
        // buffer release and the frame callback. A client that never dispatches
        // is a client the compositor eventually stops hearing from.
        //
        // **Bounded by the start-up patience**, so a compositor that has hung
        // closes this surface rather than holding the application inside
        // `blit` for ever. The line printed here is the record of what was
        // being waited for; the application sees `closed()`, exits, and its
        // restart policy brings it back.
        if let Err(why) =
            frame_round_trip(&self.conn, &mut self.queue, &mut self.inner, CONFIGURE_PATIENCE)
        {
            eprintln!(
                "rivet-sdk: {why}. The surface is closed so the application can exit and be \
                 restarted"
            );
            self.inner.closed = true;
        }
        self.follow_resize();
    }

    /// Whether the compositor has gone away or asked this surface to close.
    ///
    /// An application drawing in a loop should stop when this is true; the
    /// alternative is a process spinning against a socket nobody is on.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.inner.closed
    }

    /// **Ask for a widget layer over this surface** — see [`crate::overlay`],
    /// which is where the argument for having one lives.
    ///
    /// Call once, after [`take`]. An application that draws one picture never
    /// calls it and pays nothing for it.
    ///
    /// **It is the same layer `gpu::Screen` offers**, deliberately: an
    /// application moving its background to the GPU should not have to change
    /// how its widgets are drawn, and `.rulings/` R-206 D3 means some machines
    /// will run the CPU path for ever.
    ///
    /// # Errors
    /// A pool the compositor would not create.
    pub fn take_overlay(&mut self) -> Result<(), String> {
        if self.inner.overlay.is_some() {
            return Ok(());
        }
        let overlay = crate::overlay::Overlay::create(
            &self.inner.subcompositor,
            self.inner.window.wl_surface(),
            &self.inner.shm,
            &self.handle,
        )?;
        self.inner.overlay = Some(overlay);
        Ok(())
    }

    /// The widget layer's pixels — **`ARGB8888`, premultiplied**, so a
    /// `Target` for it is built with [`crate::text::Target::argb`].
    ///
    /// The slice is whatever was drawn last time. An application that wants a
    /// clean layer calls [`crate::draw::erase`] first; one that redraws every
    /// pixel need not.
    ///
    /// # Errors
    /// If [`Screen::take_overlay`] has not been called, or the pool would not
    /// give a buffer.
    pub fn overlay(&mut self) -> Result<&mut [u8], String> {
        let (w, h) = (self.width, self.height);
        self.inner
            .overlay
            .as_mut()
            .ok_or_else(|| "take_overlay has not been called".to_string())?
            .canvas(w, h)
    }

    /// Put the widget layer on the screen.
    ///
    /// **Separate from [`Screen::blit`] on purpose, and that separation is
    /// the whole point.** The background and the widgets change at very
    /// different rates — an animated background every frame, a clock once a
    /// minute — and committing this only when the widgets CHANGE is what
    /// stops Aura spending 11.2 ms of a 19.5 ms budget redrawing a picture
    /// identical to the last one (`PLAN.md` §181).
    ///
    /// # Errors
    /// If [`Screen::take_overlay`] has not been called, or nothing has been
    /// drawn into it yet, or the compositor has gone — and then
    /// [`Screen::closed`] is true.
    pub fn present_overlay(&mut self) -> Result<(), String> {
        let (w, h) = (self.width, self.height);
        self.inner
            .overlay
            .as_mut()
            .ok_or_else(|| "take_overlay has not been called".to_string())?
            .present(w, h)?;
        pump(&mut self.queue, &mut self.inner)
    }

    fn paint(&mut self, canvas: &[u8]) -> Result<(), String> {
        let (w, h) = (self.inner.width.max(1), self.inner.height.max(1));
        let stride = (w * 4) as i32;

        let needs_new = self.inner.buffer.is_none() || self.inner.resized;
        if needs_new {
            let (buffer, _) = self
                .inner
                .pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Xrgb8888)
                .map_err(|e| format!("a {w}x{h} buffer could not be created: {e}"))?;
            self.inner.buffer = Some(buffer);
            self.inner.resized = false;
        }

        let Some(buffer) = self.inner.buffer.as_mut() else {
            return Err("there is no buffer to draw into".to_string());
        };

        // The compositor may still hold the last one; a second is made rather
        // than waiting, which is what every Wayland client does.
        let target = match self.inner.pool.canvas(buffer) {
            Some(target) => target,
            None => {
                let (second, target) = self
                    .inner
                    .pool
                    .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Xrgb8888)
                    .map_err(|e| format!("a second {w}x{h} buffer could not be created: {e}"))?;
                *buffer = second;
                target
            }
        };

        let n = target.len().min(canvas.len());
        if let (Some(into), Some(from)) = (target.get_mut(..n), canvas.get(..n)) {
            into.copy_from_slice(from);
        }

        let surface = self.inner.window.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        surface.frame(&self.handle, surface.clone());
        buffer
            .attach_to(surface)
            .map_err(|e| format!("the buffer could not be attached: {e}"))?;
        self.inner.window.commit();
        Ok(())
    }

    /// Republish the size after a configure, so an application reading
    /// `self.width` between frames sees what the compositor last said.
    fn follow_resize(&mut self) {
        if self.width == self.inner.width && self.height == self.inner.height {
            return;
        }
        self.width = self.inner.width;
        self.height = self.inner.height;
        self.stride = self.width * 4;
        self.what = format!(
            "{}, {}x{}, wl_shm XRGB8888 (ADR-058)",
            SOCKET,
            self.width,
            self.height
        );
    }
}

/// Where the compositor accepts connections.
///
/// **Re-declared and not imported**, which is this crate's whole arrangement:
/// `rivet-sdk` links no Rivet crate, because an application is a foreign
/// program and must not link the operating system (`Cargo.toml`'s
/// `[dependencies]` table is empty by rule). So the SDK restates the constants
/// it needs and `tests/agreement.rs` proves the two still match —
/// `crate::listen::LISTEN_FDS_START` and `crate::health`'s channel path are
/// the same bargain.
///
/// **`pub` so that test can see it.** A private copy would be re-declared with
/// nothing proving it, which is the drift the whole arrangement is paying to
/// avoid.
pub const SOCKET: &str = "/run/rivet/listen/rivet-displayd/wayland";

/// Everything the Wayland side holds. Separate from [`Screen`] because the
/// toolkit's handlers take `&mut Self`, and an application must not be able to
/// reach them.
struct Inner {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    /// Bound at [`take`], used only by [`Screen::take_overlay`].
    subcompositor: SubcompositorState,
    /// **`None` until an application asks for a widget layer**, and most do
    /// not — a kiosk that draws one picture has nothing to put on a second
    /// surface. See [`crate::overlay`].
    overlay: Option<crate::overlay::Overlay>,
    buffer: Option<Buffer>,
    width: usize,
    height: usize,
    resized: bool,
    closed: bool,
}

impl CompositorHandler for Inner {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl Closes for Inner {
    fn close(&mut self) {
        self.closed = true;
    }
}

impl WindowHandler for Inner {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        // A compositor may configure `None`, meaning *choose for yourself*.
        // `rivet-displayd` never does; a client that assumed so would draw
        // nothing against one that did.
        let w = w.map_or(self.width, |w| w.get() as usize);
        let h = h.map_or(self.height, |h| h.get() as usize);
        if (w, h) != (self.width, self.height) {
            self.width = w;
            self.height = h;
            self.resized = true;
        }
    }
}

impl OutputHandler for Inner {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl ShmHandler for Inner {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Inner {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_subcompositor!(Inner);
delegate_compositor!(Inner);
delegate_output!(Inner);
delegate_shm!(Inner);
delegate_xdg_shell!(Inner);
delegate_xdg_window!(Inner);
delegate_registry!(Inner);
answers_sync!(Inner);

#[cfg(test)]
mod tests {
    //! B10: the compositor's first configure has a ten-second deadline; these
    //! prove it is actually enforced rather than merely checked around a
    //! call that can itself block forever. Both peers here are ends of a
    //! `UnixStream::pair()` the test holds open and never writes to — a
    //! real "accepts the connection and then says nothing" peer, not a
    //! model of one.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Every silent-peer test races the call under test against a bound of
    /// its own, well clear of the short `deadline` actually being tested —
    /// so that on the pre-B10 code, which can block forever, the test FAILS
    /// by timing out this harness rather than hanging the whole suite.
    const HARNESS_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

    /// B10, registry phase: a peer that accepts the connection and never
    /// sends a byte must not hang `bounded_registry_init` past `deadline`.
    ///
    /// Before B10, `take` called `registry_queue_init` directly here — no
    /// watchdog, no bound — and a silent peer left it blocked forever inside
    /// `Connection::roundtrip`'s own `poll(None)`.
    #[test]
    fn registry_discovery_silence_is_bounded() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let watchdog = client.try_clone().expect("clone the client socket");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);

        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _peer = peer; // held open and silent for the life of the call
            let result = bounded_registry_init(&conn, watchdog, deadline);
            let _ = tx.send(result);
        });

        let result = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!(
                "bounded_registry_init did not return within the harness's own {HARNESS_BOUND:?} \
                 bound against a silent peer — this is the B10 regression: an unbounded wait"
            )
        });
        let elapsed = started.elapsed();

        let err = result.expect_err("a silent peer must not be reported as successful");
        assert!(
            err.contains("globals could not be read"),
            "expected the registry-discovery error message, got: {err}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "took {elapsed:?} against a 200ms deadline — bounded, but not by the deadline"
        );
    }

    /// B10, configure phase: a peer that answers nothing at all must not
    /// hang `bounded_dispatch` past `deadline` either — the same call
    /// `take`'s configure-wait loop makes every iteration, tested here with
    /// `State = ()` rather than a real `Inner` so the peer needs no
    /// protocol knowledge to be a faithful stand-in for "the registry
    /// answered, and then the compositor went quiet before the configure."
    ///
    /// Before B10 the loop called `EventQueue::blocking_dispatch` directly —
    /// no bound on the read inside it — so a silent peer left it blocked
    /// forever, between deadline checks that never got the chance to run
    /// again.
    #[test]
    fn configure_wait_silence_is_bounded() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);

        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _peer = peer;
            let mut queue = conn.new_event_queue::<()>();
            let mut data = ();
            // The exact shape of `take`'s own loop: check the deadline, then
            // make one bounded dispatch call.
            loop {
                if std::time::Instant::now() > deadline {
                    let _ = tx.send(Err("timed out, as expected".to_string()));
                    return;
                }
                if let Err(e) = bounded_dispatch(&mut queue, &mut data, deadline) {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        });

        let result = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!(
                "the configure-wait loop did not return within the harness's own \
                 {HARNESS_BOUND:?} bound against a silent peer — this is the B10 regression"
            )
        });
        let elapsed = started.elapsed();

        assert_eq!(result, Err("timed out, as expected".to_string()));
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "took {elapsed:?} against a 200ms deadline — bounded, but not by the deadline"
        );
    }

    /// A client state that answers syncs and nothing else — all
    /// [`wait_for_configure`] needs of one.
    struct Waiting;
    answers_sync!(Waiting);

    /// A compositor that answers every `wl_display.sync` and configures
    /// nothing: `rivet-displayd` with no display plugged in. Hand-composed
    /// from the protocol: `wl_callback.done` then `wl_display.delete_id`, as a
    /// server answers a sync. Returns how many syncs it answered.
    fn answering_peer(mut peer: std::os::unix::net::UnixStream) -> std::thread::JoinHandle<u32> {
        use std::io::{Read, Write};
        std::thread::spawn(move || {
            let mut answered = 0;
            let mut message = [0u8; 12];
            // wl_display.sync is object 1, opcode 0, one new_id: 12 bytes.
            while peer.read_exact(&mut message).is_ok() {
                let id = u32::from_ne_bytes([message[8], message[9], message[10], message[11]]);
                let mut reply = Vec::new();
                for (object, opcode, arg) in [(id, 0u32, 0u32), (1, 1, id)] {
                    reply.extend_from_slice(&object.to_ne_bytes());
                    reply.extend_from_slice(&((12 << 16) | opcode).to_ne_bytes());
                    reply.extend_from_slice(&arg.to_ne_bytes());
                }
                if peer.write_all(&reply).is_err() {
                    break;
                }
                answered += 1;
            }
            answered
        })
    }

    /// **`.rulings/` R-236: an application started with no display waits for
    /// one.** The compositor is alive and configures nothing for several
    /// times the patience, and the wait goes on — asking the compositor each
    /// time — until the configure comes. Before R-236 this was refused at the
    /// first patience, and the application exited on a kiosk whose screen was
    /// simply not plugged in yet.
    #[test]
    fn a_compositor_waiting_for_a_display_is_waited_for() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let peer = answering_peer(peer);
        let patience = std::time::Duration::from_millis(100);
        let configured_after = std::time::Duration::from_millis(450);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut queue = conn.new_event_queue::<Waiting>();
            let started = std::time::Instant::now();
            let mut said = 0;
            let result = wait_for_configure(
                &conn,
                &mut queue,
                &mut Waiting,
                patience,
                |_| started.elapsed() >= configured_after,
                || said += 1,
            );
            let _ = tx.send((result, said, started.elapsed()));
        });

        let (result, said, elapsed) = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!("the wait for a display did not end within {HARNESS_BOUND:?}")
        });
        assert_eq!(result, Ok(()), "a live compositor with no display was refused");
        assert_eq!(said, 1, "the application is told it is waiting once, not every time");
        assert!(elapsed >= configured_after, "returned before the configure came: {elapsed:?}");
        let answered = peer.join().expect("the peer thread");
        assert!(answered >= 2, "the compositor was asked only {answered} time(s)");
    }

    /// **A frame wait on a compositor that has stopped answering ends**, with
    /// a record that says what it was waiting for. Before this the round trip
    /// after every frame was unbounded, and a hung compositor left the client
    /// blocked inside it for ever with nothing in the journal and no restart.
    #[test]
    fn a_frame_wait_on_a_silent_compositor_is_bounded() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let patience = std::time::Duration::from_millis(100);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _peer = peer; // held open and silent
            let mut queue = conn.new_event_queue::<Waiting>();
            let started = std::time::Instant::now();
            let result = frame_round_trip(&conn, &mut queue, &mut Waiting, patience);
            let _ = tx.send((result, started.elapsed()));
        });

        let (result, elapsed) = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!("a frame wait on a silent compositor held past {HARNESS_BOUND:?}")
        });
        let err = result.expect_err("a silent compositor must not be waited for for ever");
        assert!(
            err.contains("frame") && err.contains("did not answer"),
            "the record must say what it was waiting for: {err}"
        );
        assert!(elapsed < std::time::Duration::from_secs(2), "took {elapsed:?}");
    }

    /// **An unplugged display never counts as a stuck frame wait.** With no
    /// display `rivet-displayd` withholds frame callbacks (`ADR-058` §5) and
    /// goes on answering everything else, which is exactly this peer: it
    /// answers every sync and sends nothing more. Frame after frame, each wait
    /// ends in an answer and none is refused.
    #[test]
    fn a_compositor_with_no_display_never_trips_the_frame_wait() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let peer = answering_peer(peer);
        let patience = std::time::Duration::from_millis(100);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut queue = conn.new_event_queue::<Waiting>();
            let results: Vec<Result<(), String>> = (0..8)
                .map(|_| frame_round_trip(&conn, &mut queue, &mut Waiting, patience))
                .collect();
            let _ = tx.send(results);
        });

        let results = rx
            .recv_timeout(HARNESS_BOUND)
            .unwrap_or_else(|_| panic!("eight frames did not finish within {HARNESS_BOUND:?}"));
        assert!(
            results.iter().all(Result::is_ok),
            "a compositor that answers was treated as stuck: {results:?}"
        );
        let answered = peer.join().expect("the peer thread");
        assert!(answered >= 8, "the compositor was asked only {answered} time(s)");
    }

    /// **B10 still holds**: a compositor that stops answering while the
    /// application waits for its first configure is refused, within one
    /// patience of silence and one of an unanswered question.
    #[test]
    fn a_compositor_that_stops_answering_is_still_refused() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let patience = std::time::Duration::from_millis(100);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _peer = peer; // held open and silent
            let mut queue = conn.new_event_queue::<Waiting>();
            let started = std::time::Instant::now();
            let result =
                wait_for_configure(&conn, &mut queue, &mut Waiting, patience, |_| false, || {});
            let _ = tx.send((result, started.elapsed()));
        });

        let (result, elapsed) = rx.recv_timeout(HARNESS_BOUND).unwrap_or_else(|_| {
            panic!("a silent compositor held the wait past {HARNESS_BOUND:?} — the B10 regression")
        });
        let err = result.expect_err("a silent compositor must not be waited for for ever");
        assert!(err.contains("stopped answering"), "{err}");
        assert!(elapsed < std::time::Duration::from_secs(2), "took {elapsed:?}");
    }

    /// A client state that records being closed and nothing else.
    #[derive(Default)]
    struct Surface {
        closed: bool,
    }
    impl Closes for Surface {
        fn close(&mut self) {
            self.closed = true;
        }
    }
    answers_sync!(Surface);

    /// **A compositor that restarts closes the surface, so the application
    /// exits and its restart policy brings it back.** The peer goes away with
    /// a request queued, which is a restarted `rivet-displayd` seen from a
    /// client mid-frame: the flush meets a broken pipe. Before the surface was
    /// closed here, the error was returned and nothing else, and a GPU client
    /// drew into the dead socket for ever while `restart = "always"` waited
    /// for an exit that never came.
    #[test]
    fn a_compositor_that_goes_away_mid_frame_closes_the_surface() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        drop(peer);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut queue = conn.new_event_queue::<Surface>();
            let mut surface = Surface::default();
            let _sync = conn.display().sync(&queue.handle(), Answered(std::sync::Arc::default()));
            let result = pump(&mut queue, &mut surface);
            let _ = tx.send((result, surface.closed));
        });

        let (result, closed) = rx
            .recv_timeout(HARNESS_BOUND)
            .unwrap_or_else(|_| panic!("a pump against a dead socket held past {HARNESS_BOUND:?}"));
        let err = result.expect_err("a dead socket must not be reported as a frame handed over");
        assert!(err.contains("compositor stopped"), "{err}");
        assert!(closed, "the connection failed and the surface was left open: {err}");
    }

    /// The same with nothing queued: the read finds the far end closed.
    #[test]
    fn a_compositor_that_went_away_between_frames_closes_the_surface() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        drop(peer);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut queue = conn.new_event_queue::<Surface>();
            let mut surface = Surface::default();
            let result = pump(&mut queue, &mut surface);
            let _ = tx.send((result, surface.closed));
        });

        let (result, closed) = rx
            .recv_timeout(HARNESS_BOUND)
            .unwrap_or_else(|_| panic!("a pump against a dead socket held past {HARNESS_BOUND:?}"));
        assert!(result.is_err(), "a closed far end was reported as a frame handed over");
        assert!(closed, "the connection failed and the surface was left open");
    }

    /// **A live compositor that has nothing to say is not a closed one.**
    /// The pump does not wait, so silence is `WouldBlock`, which is the
    /// ordinary case on every frame and must leave the surface open.
    #[test]
    fn a_silent_live_compositor_leaves_the_surface_open() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _peer = peer; // held open and silent
            let mut queue = conn.new_event_queue::<Surface>();
            let mut surface = Surface::default();
            let _sync = conn.display().sync(&queue.handle(), Answered(std::sync::Arc::default()));
            let result = pump(&mut queue, &mut surface);
            let _ = tx.send((result, surface.closed));
        });

        let (result, closed) = rx
            .recv_timeout(HARNESS_BOUND)
            .unwrap_or_else(|_| panic!("a pump waited on a silent compositor past {HARNESS_BOUND:?}"));
        assert_eq!(result, Ok(()));
        assert!(!closed, "a live compositor's silence closed the surface");
    }
}
