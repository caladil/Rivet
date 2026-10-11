//! **The launcher's whole relationship with the screen** — `ADR-058` §4.
//!
//! It used to be `drm.rs`: 26 000 bytes of mode enumeration, connector
//! selection, dumb buffers, `drmModeAddFB`, page flips and a fight with the
//! framebuffer console over who was DRM master. All of it is gone. What
//! replaced it is *ask for a surface, draw into it, commit* — which is
//! `SPEC-03` §2's *"an application declares what it needs and is handed it"*
//! for the one device where Rivet used to hand over the hardware instead of
//! the result.
//!
//! # The launcher is `rivet-displayd`'s first client, on purpose
//!
//! `ADR-058` §4: *"if drawing a menu through a handed surface is awkward, it
//! is better to find out there than in Aura."* This module is the answer to
//! that, and the answer is that it is about 200 lines against 900.
//!
//! # Nothing here reads `WAYLAND_DISPLAY`
//!
//! Wayland's own discovery has a client search `XDG_RUNTIME_DIR` for a
//! compositor. Rivet does not work that way: `rivet_core::compositor` names
//! one path, the supervisor binds it and chowns it to whoever declared
//! `connects = ["rivet-displayd:wayland"]`, and a program that did not declare
//! it cannot open the path at all. The refusal is the file mode, not a check
//! written here.
//!
//! # Why `smithay-client-toolkit` and not the protocol by hand
//!
//! `ADR-009`'s table makes an existing crate the default, and `ADR-058` §5
//! records what skipping that test cost last time. So it was evaluated before
//! anything was written: with `default-features = false` the toolkit is pure
//! Rust — no `xkbcommon`, no `calloop` — and **this binary is still static**,
//! `readelf -d` reporting no `NEEDED` entries at all. `ADR-003` §5 is intact
//! for the launcher; the compositor pays that exception and the menu does not.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::xdg::window::{
    Window, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window, registry_handlers,
};
use smithay_client_toolkit::reexports::client::globals::{registry_queue_init, GlobalList};
use smithay_client_toolkit::reexports::client::protocol::{wl_callback, wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, EventQueue, QueueHandle};

/// The size a surface is assumed to be until the compositor says otherwise.
///
/// **It is never drawn at this size.** `rivet-displayd` configures every
/// toplevel fullscreen *before* it draws once, so the first configure replaces
/// these. They exist so the pool has something to be created with.
const ASSUMED: (u32, u32) = (1280, 720);

/// How long the compositor may be silent before it is asked whether it is
/// still there — B10, and [`wait_for_configure`]. The same value `rivet_sdk::composited`/`rivet_sdk::gpu` use,
/// duplicated rather than shared for the reason `gpu`'s own copy states:
/// this program does not depend on the SDK's `composited` feature, and the
/// three are alternatives against the same protocol rather than one thing.
pub(crate) const CONFIGURE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Everything the Wayland side holds.
pub struct Screen {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    buffer: Option<Buffer>,

    /// What the compositor last said the surface is. Read by the drawing code
    /// as the canvas size, so a mode change is followed rather than assumed.
    pub width: u32,
    pub height: u32,

    /// Set by a configure, cleared once a frame has been drawn at the new size.
    pub dirty: bool,
    /// The compositor asked this window to close, or the connection went away.
    pub closed: bool,

    /// Whether a real configure has ever arrived — B10.
    ///
    /// **Not `dirty`.** `dirty` starts `true` only so the very first frame
    /// draws at all, and toggles on every draw/configure after that — it
    /// cannot tell a caller whether the NEXT draw would still be at
    /// `ASSUMED` or at a real size, because a true `dirty` means both
    /// "never configured" and "configured, redraw". This starts `false` and
    /// is set once true, in `WindowHandler::configure` below, so `main`'s
    /// start-up can wait specifically for the first configure rather than
    /// for any round trip's completion.
    pub configured: bool,
}

/// Connect, bind the four globals a menu needs, and map a surface.
///
/// **Does not wait for the configure.** [`wait_for_configure`] does, called
/// from `main::run`.
///
/// # Errors
///
/// The socket, the connection, a missing global, or a shared-memory pool the
/// compositor would not create. Each names which.
pub fn open() -> Result<(Screen, EventQueue<Screen>, Connection), String> {
    let path = rivet_core::compositor::socket_path();
    let stream = std::os::unix::net::UnixStream::connect(&path).map_err(|e| {
        format!(
            "{path}: {e} — this program may not have been granted the compositor. It is \
             granted by declaring connects = [\"{}:{}\"], and rivet-displayd has to be \
             enabled and started before anything can connect to it",
            rivet_core::compositor::PROVIDER,
            rivet_core::compositor::ENDPOINT
        )
    })?;

    // **The deadline starts here, before any Wayland traffic at all — B10.**
    // See `bounded_registry_init`'s own comment for why registry discovery
    // needs a watchdog thread rather than just a bound on the configure wait
    // that follows it.
    let deadline = std::time::Instant::now() + CONFIGURE_PATIENCE;
    let watchdog = stream.try_clone().map_err(|e| {
        format!("{path}: the compositor socket could not be duplicated for the start-up deadline: {e}")
    })?;

    let conn = Connection::from_socket(stream)
        .map_err(|e| format!("{path}: the compositor would not talk Wayland: {e}"))?;
    let (globals, queue) = bounded_registry_init(&conn, watchdog, deadline, &path)?;
    let qh = queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| format!("no wl_compositor: {e} — this is not a compositor"))?;
    let shell = XdgShell::bind(&globals, &qh)
        .map_err(|e| format!("no xdg_shell: {e} — there is no way to ask for a window"))?;
    let shm = Shm::bind(&globals, &qh)
        .map_err(|e| format!("no wl_shm: {e} — there is no way to hand over pixels"))?;

    let surface = compositor.create_surface(&qh);
    // **`RequestServer`, and it does not matter today.** `rivet-displayd`
    // draws no decorations at all and configures everything fullscreen, so
    // nothing is negotiated. It is the honest request anyway: a kiosk client
    // asking to draw its own title bar would be claiming an intention it has
    // no code for.
    let window = shell.create_window(surface, WindowDecorations::RequestServer, &qh);
    window.set_title("Rivet");
    // The name the compositor keeps beneath every application's window.
    window.set_app_id(rivet_core::compositor::LAUNCHER_APP_ID);
    // **The initial commit carries no buffer**, which is the protocol's rule
    // and not a shortcut: it is what asks for the first configure, and until
    // that configure arrives a client does not know how big it is.
    window.commit();

    let (w, h) = ASSUMED;
    let pool = SlotPool::new(w as usize * h as usize * 4, &shm)
        .map_err(|e| format!("the shared-memory pool could not be created: {e}"))?;

    Ok((
        Screen {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            shm,
            pool,
            window,
            buffer: None,
            width: w,
            height: h,
            dirty: true,
            closed: false,
            configured: false,
        },
        queue,
        conn,
    ))
}

impl Screen {
    /// Hand the drawing code a canvas, then put it on the screen.
    ///
    /// `paint` is given the pixel bytes and the layout they are in. The format
    /// is **ARGB8888, little-endian**, so a pixel is `[b, g, r, a]` in memory —
    /// which is what `crate::ui` and `crate::text` already write, because it is
    /// what the DRM path wanted too. That agreement is why the drawing code
    /// survived `ADR-058` untouched.
    ///
    /// # Errors
    ///
    /// A buffer the compositor would not give back.
    pub fn paint<F>(&mut self, qh: &QueueHandle<Self>, paint: F) -> Result<(), String>
    where
        F: FnOnce(&mut [u8], &crate::text::Target),
    {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let stride = w as i32 * 4;

        // **Re-created whenever the size changed**, because a slot is sized at
        // creation and a resized window drawn into the old one would be torn
        // diagonally — the classic wrong-stride picture.
        let needs_new = match &self.buffer {
            Some(_) => self.dirty,
            None => true,
        };
        if needs_new {
            let (buffer, _) = self
                .pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Xrgb8888)
                .map_err(|e| format!("a {w}x{h} buffer could not be created: {e}"))?;
            self.buffer = Some(buffer);
        }

        let Some(buffer) = self.buffer.as_mut() else {
            return Err("there is no buffer to draw into".to_string());
        };

        // **The double-buffer case is not an error.** `canvas` answers `None`
        // while the compositor still holds the last buffer, which happens
        // whenever this draws faster than the screen refreshes. A second
        // buffer is made and the old one dropped, which is what every Wayland
        // client does and what `SlotPool` exists to make cheap.
        let canvas = match self.pool.canvas(buffer) {
            Some(canvas) => canvas,
            None => {
                let (second, canvas) = self
                    .pool
                    .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Xrgb8888)
                    .map_err(|e| format!("a second {w}x{h} buffer could not be created: {e}"))?;
                *buffer = second;
                canvas
            }
        };

        let target = crate::text::Target {
            width: w as usize,
            height: h as usize,
            stride: stride as usize,
            bytes_per_pixel: 4,
        };
        paint(canvas, &target);

        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        // **Ask for the next frame before committing this one.** A client that
        // asks afterwards can miss the callback for the frame it just posted,
        // and the symptom is a menu that updates once and then stops.
        surface.frame(qh, surface.clone());
        buffer
            .attach_to(surface)
            .map_err(|e| format!("the buffer could not be attached: {e}"))?;
        self.window.commit();
        self.dirty = false;
        Ok(())
    }
}

impl CompositorHandler for Screen {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // **Ignored, and it is a decision.** A kiosk panel is one display at
        // one scale; honouring a fractional scale here would mean resampling
        // every glyph for a case this machine does not have. The day Rivet has
        // a HiDPI panel, the fix is to ask the compositor for a buffer scale
        // rather than to interpolate.
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
        // The compositor is ready for another frame. `main`'s loop decides
        // whether there is anything new to put in it — a clock that has not
        // changed minute is not a reason to redraw 2 MB.
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

impl WindowHandler for Screen {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {
        // **The launcher does not close itself.** It is `restart = "always"`,
        // and a machine whose menu exited on request would be a machine with a
        // black screen and a healthy supervisor. Recorded rather than honoured.
        println!("rivet-launcher: the compositor asked this window to close, and it will not");
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
        // **A compositor may configure a size of `None`**, meaning *choose for
        // yourself*. `rivet-displayd` never does — it configures fullscreen
        // with the mode's size — but a client that assumed so would draw
        // nothing at all against one that did, and be very hard to read.
        let w = w.map_or(self.width, std::num::NonZeroU32::get);
        let h = h.map_or(self.height, std::num::NonZeroU32::get);
        if (w, h) != (self.width, self.height) {
            println!("rivet-launcher: the surface is {w}x{h}");
            self.width = w;
            self.height = h;
        }
        self.dirty = true;
        self.configured = true;
    }
}

impl OutputHandler for Screen {
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

impl ShmHandler for Screen {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Screen {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Screen);
delegate_output!(Screen);
delegate_shm!(Screen);
delegate_xdg_shell!(Screen);
delegate_xdg_window!(Screen);
delegate_registry!(Screen);

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
/// during ordinary menu drawing, rather than during start-up — cancellation
/// is by dropping `cancel_tx`, which the standard library's
/// `mpsc::Receiver::recv_timeout` reports as `Disconnected` rather than
/// `Timeout`, and only `Timeout` arms the shutdown.
///
/// Identical in shape to `rivet_sdk::composited`'s own private copy of this
/// function — this program does not depend on that feature, so it cannot
/// share the code, only the technique.
fn bounded_registry_init(
    conn: &Connection,
    watchdog: std::os::unix::net::UnixStream,
    deadline: std::time::Instant,
    path: &str,
) -> Result<(GlobalList, EventQueue<Screen>), String> {
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

    let result = registry_queue_init::<Screen>(conn)
        .map_err(|e| format!("{path}: the compositor's globals could not be read: {e}"));

    drop(cancel_tx);
    let _ = timer.join();
    result
}

/// One bounded step of dispatch, in place of `EventQueue::blocking_dispatch` — B10.
///
/// `blocking_dispatch` polls the socket with no timeout of its own, so a
/// compositor that accepts the connection and then goes silent mid-wait
/// leaves it blocked forever. This is the same prepare-read/poll/read
/// protocol `blocking_dispatch` uses internally (`EventQueue::prepare_read`),
/// with the poll bounded by the time left until `deadline` rather than
/// blocked forever, and the guard dropped — which `wayland-backend` 0.3
/// documents as cancelling the prepared read — rather than read from, when
/// the deadline wins the race.
///
/// Returns `Ok(0)` on a timed-out poll: the same "nothing dispatched yet"
/// contract `dispatch_pending` already has, so a caller looping this can
/// re-check its own deadline exactly as it would around `blocking_dispatch`.
///
/// **Generic in `State`, though this program only ever calls it with
/// `Screen`.** `EventQueue::dispatch_pending`/`prepare_read`/`flush` need no
/// `Dispatch` bound themselves — dispatch looks callbacks up by object id,
/// already registered when each proxy was created — so this needs none
/// either. The tests use that: they exercise the real prepare-read/poll/read
/// path against a real socket with `State = ()`, which needs no fake
/// compositor that can actually bind globals and create a surface.
pub(crate) fn bounded_dispatch<State>(
    queue: &mut EventQueue<State>,
    data: &mut State,
    deadline: std::time::Instant,
) -> Result<usize, String> {
    use std::os::unix::io::AsRawFd;

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

/// **Wait for the first configure for as long as the compositor is alive** —
/// `.rulings/` R-236, and B10's bound kept.
///
/// `rivet-displayd` configures no window until a display is plugged in, and a
/// kiosk booted without one may wait hours. The launcher is meant to wait with
/// it rather than exit and be given up on by the supervisor. A compositor that
/// hung looks the same from here — silence — so each time `patience` passes
/// with no configure the compositor is asked a `wl_display.sync`: one that
/// answers within `patience` is alive and the wait goes on; one that does not
/// is refused. `waiting` runs once, the first time the answer is *alive*.
///
/// `rivet_sdk::composited` carries the same function for applications; this
/// copy exists for the reason [`CONFIGURE_PATIENCE`] states.
///
/// # Errors
///
/// The connection failed, or the compositor stopped answering.
pub(crate) fn wait_for_configure(
    conn: &Connection,
    queue: &mut EventQueue<Screen>,
    screen: &mut Screen,
    patience: std::time::Duration,
    mut waiting: impl FnMut(),
) -> Result<(), String> {
    let mut deadline = std::time::Instant::now() + patience;
    let mut said = false;
    while !screen.configured {
        if std::time::Instant::now() > deadline {
            if !answers(conn, queue, screen, std::time::Instant::now() + patience)? {
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
        bounded_dispatch(queue, screen, deadline)?;
    }
    Ok(())
}

/// **The round trip after each turn of the menu's loop, bounded by `patience`.**
///
/// One `wl_display.sync`, dispatching whatever arrives with its answer — the
/// configure, the buffer release, the frame callback — as
/// `EventQueue::roundtrip` does, except that a compositor which does not
/// answer within `patience` ends the wait instead of holding the launcher for
/// ever with nothing in the journal.
///
/// **An unplugged display never trips it.** `rivet-displayd` withholds frame
/// callbacks while it has no display (`ADR-058` §5) and answers a sync
/// regardless, so this wait, which is for the sync and never for a frame
/// callback, ends in an answer whether a display is connected or not. Only a
/// compositor that has stopped answering altogether runs it out.
///
/// `rivet_sdk::composited` carries the same function for applications; this
/// copy exists for the reason [`CONFIGURE_PATIENCE`] states.
///
/// # Errors
///
/// The connection failed, or the compositor did not answer within `patience`.
/// The text says what was being waited for, because it is the record a stuck
/// launcher leaves in the journal before it exits and is restarted.
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
            "the compositor did not answer the round trip after a frame within {}s: the menu \
             was waiting for {} to acknowledge a committed frame, which it does with or \
             without a display connected",
            patience.as_secs(),
            rivet_core::compositor::socket_path()
        ))
    }
}

/// The user data of a `wl_display.sync` sent by [`answers`]: set when the
/// compositor answers it.
pub(crate) struct Answered(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Dispatch<wl_callback::WlCallback, Answered> for Screen {
    fn event(
        _state: &mut Self,
        _proxy: &wl_callback::WlCallback,
        event: wl_callback::Event,
        data: &Answered,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            data.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Whether the compositor answers a `wl_display.sync` before `deadline`.
///
/// # Errors
///
/// The connection failed while waiting.
fn answers<State>(
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

/// Poll one fd for readability, bounded by `timeout_ms` — `libc::poll`'s own
/// contract: `-1` blocks forever, `0` returns immediately, and every other
/// value bounds the wait to that many milliseconds.
///
/// **The one raw FFI call this module makes.** It exists because
/// `wayland-client` 0.31 gives no bounded alternative to reading an
/// `EventQueue::prepare_read` guard, and this crate already carries
/// `unsafe_code = "allow"` for exactly this kind of small, locally
/// documented FFI — the same latitude `crate::kiosk`'s `sendmsg`/`CMSG_*`
/// use already takes.
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
    /// Before B10, `open` called `registry_queue_init` directly here — no
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
            let result = bounded_registry_init(&conn, watchdog, deadline, "test-socket");
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
    /// `main::run`'s configure-wait loop makes every iteration, tested here
    /// with `State = ()` rather than a real `Screen` so the peer needs no
    /// protocol knowledge to be a faithful stand-in for "the registry
    /// answered, and then the compositor went quiet before the configure."
    ///
    /// Before B10, `main::run` called `EventQueue::roundtrip` once, unbounded
    /// — itself a loop of `blocking_dispatch` with no bound on the read
    /// inside it — so a silent peer left it blocked forever.
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
            // The exact shape of `main::run`'s own loop: check the deadline,
            // then make one bounded dispatch call.
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

    /// A state that answers syncs and nothing else — all [`answers`] needs.
    struct Asking;

    impl Dispatch<wl_callback::WlCallback, Answered> for Asking {
        fn event(
            _state: &mut Self,
            _proxy: &wl_callback::WlCallback,
            event: wl_callback::Event,
            data: &Answered,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
            if let wl_callback::Event::Done { .. } = event {
                data.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    /// **The question [`wait_for_configure`] asks of a silent compositor**,
    /// against both answers. A compositor that answers a `wl_display.sync` is
    /// alive and waiting for a display (`.rulings/` R-236); one that does not
    /// is refused (B10). The answering peer is hand-composed from the
    /// protocol: `wl_callback.done`, then `wl_display.delete_id`.
    #[test]
    fn a_live_compositor_answers_and_a_silent_one_does_not() {
        use std::io::{Read, Write};

        let (client, mut peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let replier = std::thread::spawn(move || {
            let mut message = [0u8; 12];
            if peer.read_exact(&mut message).is_ok() {
                let id = u32::from_ne_bytes([message[8], message[9], message[10], message[11]]);
                let mut reply = Vec::new();
                for (object, opcode, arg) in [(id, 0u32, 0u32), (1, 1, id)] {
                    reply.extend_from_slice(&object.to_ne_bytes());
                    reply.extend_from_slice(&((12 << 16) | opcode).to_ne_bytes());
                    reply.extend_from_slice(&arg.to_ne_bytes());
                }
                peer.write_all(&reply).expect("the answer is written");
            }
            peer
        });
        let mut queue = conn.new_event_queue::<Asking>();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        assert_eq!(answers(&conn, &mut queue, &mut Asking, deadline), Ok(true));
        let _held_open = replier.join().expect("the peer thread");

        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let _silent = peer;
        let mut queue = conn.new_event_queue::<Asking>();
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_millis(200);
        assert_eq!(answers(&conn, &mut queue, &mut Asking, deadline), Ok(false));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    /// A peer that answers every `wl_display.sync` and sends nothing else —
    /// what `rivet-displayd` is to a client while no display is connected.
    /// Hand-composed from the protocol: `wl_callback.done`, then
    /// `wl_display.delete_id`. Returns how many syncs it answered.
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

    /// **The menu loop's wait on a compositor that has stopped answering
    /// ends**, with a record that says what it was waiting for, so the
    /// launcher exits and its restart policy brings it back.
    #[test]
    fn a_frame_wait_on_a_silent_compositor_is_bounded() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let patience = std::time::Duration::from_millis(100);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _peer = peer; // held open and silent
            let mut queue = conn.new_event_queue::<Asking>();
            let started = std::time::Instant::now();
            let result = frame_round_trip(&conn, &mut queue, &mut Asking, patience);
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
    /// goes on answering everything else, which is exactly this peer. Turn
    /// after turn, each wait ends in an answer and none is refused.
    #[test]
    fn a_compositor_with_no_display_never_trips_the_frame_wait() {
        let (client, peer) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        let conn = Connection::from_socket(client).expect("wrap the client socket");
        let peer = answering_peer(peer);
        let patience = std::time::Duration::from_millis(100);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut queue = conn.new_event_queue::<Asking>();
            let results: Vec<Result<(), String>> = (0..8)
                .map(|_| frame_round_trip(&conn, &mut queue, &mut Asking, patience))
                .collect();
            let _ = tx.send(results);
        });

        let results = rx
            .recv_timeout(HARNESS_BOUND)
            .unwrap_or_else(|_| panic!("eight turns did not finish within {HARNESS_BOUND:?}"));
        assert!(
            results.iter().all(Result::is_ok),
            "a compositor that answers was treated as stuck: {results:?}"
        );
        let answered = peer.join().expect("the peer thread");
        assert!(answered >= 8, "the compositor was asked only {answered} time(s)");
    }
}
