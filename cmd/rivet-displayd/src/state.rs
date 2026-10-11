//! **What the compositor knows**, and the handlers Wayland calls into.
//!
//! `ADR-058` §4 puts mechanism here and policy in `rivet-launcher`, so this
//! module deliberately decides almost nothing. Every window is fullscreen,
//! every window is stacked in arrival order, and the topmost gets the keyboard
//! — with one exception, which is mechanism rather than policy: **the
//! launcher is always beneath every other window** (`Rivet::bury`). It is
//! the shell the applications are shown over, and a stack that ordered it by
//! arrival alone let a restarted launcher, or one that started beside an
//! application the supervisor resumed at boot, cover what holds the screen.
//! Wayland gives a client no way to lower itself, so this is the one place
//! the rule can live.

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::desktop::space::SpaceElement;
use smithay::desktop::{Space, Window};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::{Client, DisplayHandle};
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    get_parent, is_sync_subsurface, with_states, CompositorClientState, CompositorHandler,
    CompositorState,
};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_output, delegate_seat,
    delegate_shm, delegate_xdg_shell,
};

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::Format;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::utils::{Logical, Size};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};

/// Everything the compositor holds that is not the screen itself.
///
/// The screen — DRM, GBM, EGL and the renderer — lives in [`crate::screen`] and
/// is deliberately NOT a field here: the Wayland handlers below are called from
/// inside a dispatch and must not be able to reach the page-flip loop by
/// accident. What they can do is set [`Self::redraw`], and the frame loop reads
/// it.
pub struct Rivet {
    pub handle: DisplayHandle,
    pub compositor: CompositorState,
    pub xdg: XdgShellState,
    pub shm: ShmState,
    pub output_manager: OutputManagerState,
    pub seat_state: SeatState<Self>,
    pub data_device: DataDeviceState,

    /// **How an application draws on the GPU** — `.rulings/` R-206, `ADR-058`
    /// §5's *"whether a 3D surface is an EGL handle, a dma-buf the application
    /// renders into, or a GL framebuffer object"*, answered: a dma-buf.
    ///
    /// `shm` above is the CPU path: a client writes pixels into shared memory
    /// and this process uploads them as a texture every frame. For a
    /// background that is already a texture in GPU memory that upload is a
    /// round trip through main memory for nothing, and
    /// `GPU-Backgrounds-2026-09-20` §3 is the measurement — *"a readback of
    /// 8.29 MB at 1080p costs more than the 32 ms being saved."*
    ///
    /// A dmabuf is the same frame with no copy at all: the client renders into
    /// a `gbm_bo` on its own render node, exports the descriptor, and this
    /// process imports it as a texture. **The pixels never leave the GPU.**
    pub dmabuf: DmabufState,
    /// The advertised global, held because dropping it withdraws the protocol.
    pub dmabuf_global: DmabufGlobal,
    /// **Imports waiting for the frame loop**, because the renderer is not
    /// reachable from here — see this struct's own note.
    ///
    /// Smithay asks a compositor to import a dmabuf into its renderer *inside*
    /// the handler, and the renderer lives in [`crate::screen::Kms`],
    /// deliberately out of reach of anything called from a dispatch. So the
    /// handler parks the pair and the frame loop drains it, which is what
    /// [`ImportNotifier`] is shaped for: it may be answered later, and a client
    /// that dies before then is handled by Smithay rather than by us.
    ///
    /// **An unanswered notifier is a protocol error**, so the loop must drain
    /// this every pass and never conditionally.
    pub pending_imports: Vec<(Dmabuf, ImportNotifier)>,

    /// The screen, **when one is plugged in** — `.rulings/` R-236: on a kiosk a
    /// screen may come and go at any time, and `None` is a state to wait in,
    /// not a fault. [`Rivet::screen_changed`] is the one place it changes.
    ///
    /// `ADR-058` does not rule out a second screen and nothing here assumes
    /// there is only one *forever* — but a Pi 4 kiosk has one, and a `Vec`
    /// with one element in it for years is a worse lie than a field.
    pub output: Option<Output>,

    /// The `wl_output` global advertising [`Self::output`].
    pub output_global: Option<GlobalId>,

    /// **The global of a screen that has left: disabled, awaiting removal.**
    ///
    /// Removing a global outright kills any client whose bind was already on
    /// the wire when it went — `wayland-server`'s own documentation says so —
    /// so a departed screen's global is disabled first, which withdraws it
    /// from every registry, and removed at the next change, by which time
    /// nothing can still be binding it. At most one is ever held.
    pub retired_global: Option<GlobalId>,

    /// Where windows are. Every one is mapped at the origin, fullscreen.
    pub space: Space<Window>,

    pub seat: Seat<Self>,

    /// Set by any commit that changed what is on screen. The frame loop clears
    /// it. **This is the whole of the damage tracking at this layer** — Smithay
    /// does the real thing per-element inside `render_frame`, and a second,
    /// coarser copy here would only be able to be wrong.
    pub redraw: bool,

    /// Cleared by a signal or an unrecoverable screen error. `main` returns
    /// rather than exits, so destructors run and DRM master is dropped.
    pub running: bool,
}

impl Rivet {
    /// Everything the compositor holds, with no screen yet.
    ///
    /// `dmabuf_formats` are the renderer's own — `.rulings/` R-206: a
    /// compositor that advertises a format it cannot import turns a client's
    /// successful `eglCreateImage` into a failed commit one frame later.
    pub fn new(handle: &DisplayHandle, dmabuf_formats: Vec<Format>) -> Self {
        let mut seat_state = SeatState::new();
        // **A seat with nothing on it yet, and that is a decision rather than a
        // gap.** `add_keyboard` compiles an XKB keymap from layout names, which
        // reads the `xkeyboard-config` dataset — about 5 MB of text, twice over
        // on an A/B card (`ADR-018` §4). Nothing on this machine has asked for
        // `wl_keyboard`: the `ADR-015` gesture vocabulary travels on
        // `rivet_core::input`, which the launcher already reads.
        let seat = seat_state.new_wl_seat(handle, "rivet");

        let mut dmabuf = DmabufState::new();
        let dmabuf_global = dmabuf.create_global::<Self>(handle, dmabuf_formats);

        Self {
            compositor: CompositorState::new::<Self>(handle),
            xdg: XdgShellState::new::<Self>(handle),
            dmabuf,
            dmabuf_global,
            pending_imports: Vec::new(),
            // **`vec![]`, not a format list.** `ShmState::new` always
            // advertises ARGB8888 and XRGB8888, which is every format a Rivet
            // client draws in; anything more would be a claim the renderer has
            // to honour.
            shm: ShmState::new::<Self>(handle, vec![]),
            output_manager: OutputManagerState::new_with_xdg_output::<Self>(handle),
            data_device: DataDeviceState::new::<Self>(handle),
            seat_state,
            seat,
            output: None,
            output_global: None,
            retired_global: None,
            space: Space::default(),
            handle: handle.clone(),
            redraw: true,
            running: true,
        }
    }

    /// **The screen is now `now`** — a display arrived, left, or was replaced
    /// by one at another mode. `.rulings/` R-212 and R-236.
    ///
    /// The old output leaves the space and its global is retired; the new one
    /// is advertised, mapped, and **every window is configured to its size** —
    /// including a window that connected while there was no screen and has
    /// been waiting for its first configure ever since. A window is never
    /// unmapped or closed here: its client keeps running with its state
    /// intact, and simply receives no frame callbacks until a screen returns,
    /// because [`Self::send_frame_callbacks`] is only reached when a frame was
    /// shown.
    pub fn screen_changed(&mut self, now: Option<Output>) {
        if let Some(old) = self.output.take() {
            self.space.unmap_output(&old);
        }
        if let Some(global) = self.output_global.take() {
            self.handle.disable_global::<Self>(global.clone());
            if let Some(older) = self.retired_global.replace(global) {
                self.handle.remove_global::<Self>(older);
            }
        }

        if let Some(output) = now {
            self.output_global = Some(output.create_global::<Self>(&self.handle));
            self.space.map_output(&output, (0, 0));
            self.output = Some(output);
            if let Some(size) = self.fullscreen_size() {
                let windows: Vec<Window> = self.space.elements().cloned().collect();
                for window in windows {
                    if let Some(toplevel) = window.toplevel() {
                        configure_fullscreen(toplevel, size);
                    }
                }
            }
        }
        self.redraw = true;
    }

    /// The size every window is configured to: the screen's mode, or `None`
    /// when there is no screen to be the size of.
    #[must_use]
    pub fn fullscreen_size(&self) -> Option<Size<i32, Logical>> {
        fullscreen_size(self.output.as_ref())
    }

    /// Tell every window that drew that its frame is on the glass, so it may
    /// draw the next one.
    ///
    /// **Not sending these stops a well-behaved client dead**, and it stops it
    /// silently — it is waiting, not failing. That is exactly what an unplug
    /// is meant to do to a client and nothing else is: with no screen there is
    /// no frame on the glass, so nothing is sent and a client that paces
    /// itself on callbacks idles until a screen returns.
    pub fn send_frame_callbacks(&self) {
        let Some(output) = self.output.clone() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        for window in self.space.elements() {
            window.send_frame(&output, now, None, |_, _| Some(output.clone()));
        }
    }

    /// The window the keyboard is pointed at — the most recently mapped one.
    ///
    /// Policy, and the smallest possible amount of it: `ADR-058` §4 leaves
    /// focus to the compositor because nothing else can see the stack, and a
    /// kiosk's answer is *the thing that just appeared*.
    pub fn focus(&self) -> Option<WlSurface> {
        self.space
            .elements()
            .next_back()
            .and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()))
    }

    /// Point the keyboard at [`Self::focus`], if there is one.
    pub fn refocus(&mut self, serial: Serial) {
        let target = self.focus();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, target, serial);
        }
    }

    /// **Keep the launcher beneath every other window** — [`sink`], and the
    /// keyboard follows whatever is then on top.
    ///
    /// Called on every commit of the launcher's window, because its `app_id`
    /// is not known when the window is created, only once the client has set
    /// it.
    fn bury(&mut self, launcher: &Window) {
        if sink(&mut self.space, launcher) {
            self.refocus(Serial::from(0));
        }
    }
}

/// **Put `bottom` beneath every other element, leaving their own order
/// alone**, and say whether anything moved.
///
/// Smithay's space has no *lower*; it stacks by raising. So everything else is
/// raised, bottom first, which keeps the others in the order they were already
/// in. Nothing moves when `bottom` is already there, which is every commit
/// after the first that needed it, or when it is not in the space at all.
fn sink<E: SpaceElement + PartialEq + Clone>(space: &mut Space<E>, bottom: &E) -> bool {
    if space.elements().next() == Some(bottom) || !space.elements().any(|e| e == bottom) {
        return false;
    }
    let others: Vec<E> = space.elements().filter(|e| *e != bottom).cloned().collect();
    for element in others {
        space.raise_element(&element, false);
    }
    true
}

/// The size a window is configured to on `output`: its mode, in logical
/// pixels at scale 1. `None` when there is no output, or it has no mode.
fn fullscreen_size(output: Option<&Output>) -> Option<Size<i32, Logical>> {
    output
        .and_then(Output::current_mode)
        .map(|mode| mode.size.to_logical(1))
}

/// Configure `surface` fullscreen and activated at `size`, and send it.
///
/// A toplevel awaiting its first configure is always sent one; after that,
/// only a change is sent, so a second screen at the same size costs a client
/// nothing.
fn configure_fullscreen(surface: &ToplevelSurface, size: Size<i32, Logical>) {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;

    surface.with_pending_state(|state| {
        state.size = Some(size);
        state.states.set(State::Activated);
        state.states.set(State::Fullscreen);
    });
    if surface.is_initial_configure_sent() {
        let _ = surface.send_pending_configure();
    } else {
        surface.send_configure();
    }
}

/// The `app_id` a window's client set, if it set one yet.
fn app_id(window: &Window) -> Option<String> {
    let toplevel = window.toplevel()?;
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok().and_then(|attributes| attributes.app_id.clone()))
    })
}

/// Per-client state. Wayland requires the compositor to carry one of these per
/// connection. It holds what Smithay needs (`CompositorClientState`) and
/// nothing of Rivet's, because Rivet asks a client nothing about itself.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}

impl CompositorHandler for Rivet {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // **The fallback is not laziness.** `get_data` returns `None` only for
        // a client this compositor did not insert, which cannot happen —
        // `crate::clients` is the one place a connection is accepted. A
        // `panic!` here would be a crash in the program that may not fail, in
        // order to report an impossibility, so an empty state is handed back
        // and the client simply gets nothing. `ADR-017` §3.
        static ORPHAN: std::sync::OnceLock<CompositorClientState> = std::sync::OnceLock::new();
        client
            .get_data::<ClientState>()
            .map(|s| &s.compositor_state)
            .unwrap_or_else(|| ORPHAN.get_or_init(CompositorClientState::default))
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            // Bound before it is used: `elements()` borrows the space for as
            // long as its iterator lives, and `bury` needs it mutably.
            let committed = self
                .space
                .elements()
                .find(|w| {
                    w.toplevel()
                        .is_some_and(|t| t.wl_surface() == &root)
                })
                .cloned();
            if let Some(window) = committed {
                window.on_commit();
                if app_id(&window).as_deref() == Some(rivet_core::compositor::LAUNCHER_APP_ID) {
                    self.bury(&window);
                }
            }
        }

        self.redraw = true;
    }
}

impl BufferHandler for Rivet {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Rivet {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

impl XdgShellHandler for Rivet {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg
    }

    /// **Every window is fullscreen, and it is told so before it draws once.**
    ///
    /// A client that is configured with a size never has to guess, never
    /// allocates a buffer the compositor will then letterbox, and never shows a
    /// first frame at the wrong size. That is the behaviour `SPEC-03` §2 wants
    /// from *handed what it needs*, expressed in the protocol's own terms.
    ///
    /// **With no screen, it is not configured at all yet** — `.rulings/`
    /// R-236, `ADR-058` §5. The window is mapped and waits; a client may not
    /// attach a buffer before its first configure, so it waits too, and
    /// [`Rivet::screen_changed`] configures it to the screen's size when one
    /// arrives. Configuring it to an invented size instead would have a client
    /// allocate, draw and size its whole state for a screen that does not
    /// exist, and every client this machine runs sizes itself once.
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        match self.fullscreen_size() {
            Some(size) => configure_fullscreen(&surface, size),
            None => println!(
                "rivet-displayd: a window is waiting for a display before it is configured"
            ),
        }

        let window = Window::new_wayland_window(surface);
        self.space.map_element(window, (0, 0), true);
        self.refocus(Serial::from(0));
        self.redraw = true;
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // The `let` binding is not a style choice — `elements()` borrows the
        // space for as long as its iterator is alive, and `unmap_elem` needs it
        // mutably. Collapsing these two statements does not compile.
        let gone = self
            .space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t == &surface))
            .cloned();
        if let Some(window) = gone {
            self.space.unmap_elem(&window);
        }
        self.refocus(Serial::from(0));
        self.redraw = true;
    }

    /// **Popups are accepted and not positioned**, which is honest rather than
    /// complete: nothing on this machine uses one yet. A menu that needs one
    /// will need a grab too, and both belong in the same change.
    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }
}

impl SeatHandler for Rivet {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: smithay::input::pointer::CursorImageStatus) {
    }
}

impl smithay::wayland::output::OutputHandler for Rivet {}

impl SelectionHandler for Rivet {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Rivet {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device
    }
}

impl ClientDndGrabHandler for Rivet {}
impl ServerDndGrabHandler for Rivet {}

/// **Take the dmabuf and answer for it later** — see [`Rivet::pending_imports`].
///
/// This handler deliberately does no work. Smithay's own example imports into
/// the renderer here and answers the notifier immediately; this process cannot,
/// because the renderer is behind the frame loop on purpose, and reaching it
/// from inside a Wayland dispatch is the coupling [`Rivet`]'s own comment
/// exists to prevent.
impl DmabufHandler for Rivet {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        self.pending_imports.push((dmabuf, notifier));
        // The client is waiting on this before it can attach; a frame that
        // does not come is a client that never draws.
        self.redraw = true;
    }
}

delegate_dmabuf!(Rivet);
delegate_compositor!(Rivet);
delegate_shm!(Rivet);
delegate_xdg_shell!(Rivet);
delegate_seat!(Rivet);
delegate_data_device!(Rivet);
delegate_output!(Rivet);

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use smithay::utils::{IsAlive, Logical, Point, Rectangle};

    /// A stand-in for a window. [`sink`] reads nothing from an element but
    /// its identity, so a name is enough, and Smithay's own `Space` does the
    /// stacking — the thing these tests are about.
    #[derive(Clone, Debug, PartialEq)]
    struct Named(&'static str);

    impl IsAlive for Named {
        fn alive(&self) -> bool {
            true
        }
    }

    impl SpaceElement for Named {
        fn bbox(&self) -> Rectangle<i32, Logical> {
            Rectangle::default()
        }
        fn is_in_input_region(&self, _point: &Point<f64, Logical>) -> bool {
            false
        }
        fn set_activate(&self, _activated: bool) {}
        fn output_enter(&self, _output: &Output, _overlap: Rectangle<i32, Logical>) {}
        fn output_leave(&self, _output: &Output) {}
    }

    /// Mapped in this order, as `new_toplevel` maps a window when it arrives.
    fn arrived(names: &[&'static str]) -> Space<Named> {
        let mut space = Space::default();
        for &name in names {
            space.map_element(Named(name), (0, 0), true);
        }
        space
    }

    /// Bottom first, as [`Space::elements`] iterates.
    fn stack(space: &Space<Named>) -> Vec<&'static str> {
        space.elements().map(|e| e.0).collect()
    }

    /// **The boot this was written for.** The supervisor resumed the
    /// application the kiosk was showing, the launcher's window arrived after
    /// it, and arrival order put the menu over the application.
    #[test]
    fn a_launcher_that_arrived_last_goes_beneath_the_application() {
        let mut space = arrived(&["aura-renderer", "launcher"]);
        assert_eq!(stack(&space), ["aura-renderer", "launcher"], "the stack as it arrived");

        assert!(sink(&mut space, &Named("launcher")));
        assert_eq!(stack(&space), ["launcher", "aura-renderer"]);
    }

    /// Only the launcher moves: two windows above it keep the order they
    /// arrived in, so the one that holds the screen still holds it.
    #[test]
    fn the_other_windows_keep_their_own_order() {
        let mut space = arrived(&["first", "launcher", "second"]);

        assert!(sink(&mut space, &Named("launcher")));
        assert_eq!(stack(&space), ["launcher", "first", "second"]);
    }

    /// Every commit of the launcher's window asks again, so the answer after
    /// the first must be *nothing moved* — which is also the launcher alone,
    /// showing the menu.
    #[test]
    fn a_launcher_already_beneath_moves_nothing() {
        let mut space = arrived(&["launcher", "aura-renderer"]);
        assert!(!sink(&mut space, &Named("launcher")));
        assert_eq!(stack(&space), ["launcher", "aura-renderer"]);

        let mut alone = arrived(&["launcher"]);
        assert!(!sink(&mut alone, &Named("launcher")));
        assert_eq!(stack(&alone), ["launcher"]);
    }

    /// An element the space does not hold is not a reason to reshuffle the
    /// ones it does, nor to report that anything moved.
    #[test]
    fn an_element_that_is_not_mapped_moves_nothing() {
        let mut space = arrived(&["aura-renderer"]);
        assert!(!sink(&mut space, &Named("launcher")));
        assert_eq!(stack(&space), ["aura-renderer"]);
    }
}

/// **A screen arriving, leaving and changing, against a real client
/// connection** — `.rulings/` R-212 and R-236, `ADR-058` §5.
///
/// The compositor here is the real one — Smithay's handlers and this crate's
/// [`Rivet`] — behind a real `wl_display`. Only the screen is missing, and
/// that is the point: [`Rivet::screen_changed`] is what the frame loop calls
/// when the DRM device reports a connector change, so calling it directly is
/// the hot-plug with the kernel taken out.
///
/// The client is a socket this test writes the Wayland wire format to by hand.
/// No client library is in this crate's lockfile, and adding one to test the
/// server would be a dependency for the test's convenience. Every message
/// below is hand-composed from the protocol XML, not captured from a client.
#[cfg(test)]
mod screen_tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use smithay::output::{Mode, PhysicalProperties, Subpixel};
    use smithay::reexports::wayland_server::Display;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    /// One event as it came off the wire.
    #[derive(Debug)]
    struct Event {
        object: u32,
        opcode: u16,
        args: Vec<u8>,
    }

    impl Event {
        fn word(&self, index: usize) -> u32 {
            u32::from_ne_bytes(self.args[index * 4..index * 4 + 4].try_into().unwrap())
        }
    }

    struct Harness {
        display: Display<Rivet>,
        state: Rivet,
        client: UnixStream,
        unread: Vec<u8>,
    }

    // Object ids this client allocates, in the order it creates them.
    const DISPLAY: u32 = 1;
    const REGISTRY: u32 = 2;
    const SYNC: u32 = 3;
    const COMPOSITOR: u32 = 4;
    const WM_BASE: u32 = 5;
    const SURFACE: u32 = 6;
    const XDG_SURFACE: u32 = 7;
    const TOPLEVEL: u32 = 8;

    impl Harness {
        fn new() -> Self {
            let display: Display<Rivet> = Display::new().expect("a wl_display");
            let mut handle = display.handle();
            let state = Rivet::new(&handle, Vec::new());
            let (server, client) = UnixStream::pair().expect("a socket pair");
            handle
                .insert_client(server, std::sync::Arc::new(ClientState::default()))
                .expect("the client is accepted");
            client.set_nonblocking(true).expect("non-blocking");
            Self { display, state, client, unread: Vec::new() }
        }

        /// `object.opcode(args)`, framed as the wire format frames it.
        fn send(&mut self, object: u32, opcode: u16, args: &[u8]) {
            let size = u32::try_from(8 + args.len()).unwrap();
            let mut message = Vec::new();
            message.extend_from_slice(&object.to_ne_bytes());
            message.extend_from_slice(&((size << 16) | u32::from(opcode)).to_ne_bytes());
            message.extend_from_slice(args);
            self.client.write_all(&message).expect("the request is written");
        }

        /// Let the compositor read what was sent and write what it answered.
        fn pump(&mut self) {
            self.display.dispatch_clients(&mut self.state).expect("dispatch");
            self.display.flush_clients().expect("flush");
        }

        /// Every event the compositor has written since the last call.
        fn events(&mut self) -> Vec<Event> {
            let mut chunk = [0u8; 4096];
            loop {
                match self.client.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => self.unread.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("the client socket failed: {e}"),
                }
            }
            let mut events = Vec::new();
            while self.unread.len() >= 8 {
                let object = u32::from_ne_bytes(self.unread[0..4].try_into().unwrap());
                let word = u32::from_ne_bytes(self.unread[4..8].try_into().unwrap());
                let size = (word >> 16) as usize;
                if self.unread.len() < size {
                    break;
                }
                let args = self.unread[8..size].to_vec();
                events.push(Event { object, opcode: (word & 0xffff) as u16, args });
                self.unread.drain(..size);
            }
            for event in &events {
                assert!(
                    !(event.object == DISPLAY && event.opcode == 0),
                    "the compositor posted a protocol error and the client is dead: {event:?}"
                );
            }
            events
        }

        /// Bind `interface` at `version` as `id`, from the registry's globals.
        fn bind(&mut self, globals: &[(u32, String)], interface: &str, version: u32, id: u32) {
            let name = globals
                .iter()
                .find(|(_, i)| i == interface)
                .unwrap_or_else(|| panic!("no {interface} global in {globals:?}"))
                .0;
            let mut args = Vec::new();
            args.extend_from_slice(&name.to_ne_bytes());
            args.extend_from_slice(&string(interface));
            args.extend_from_slice(&version.to_ne_bytes());
            args.extend_from_slice(&id.to_ne_bytes());
            self.send(REGISTRY, 0, &args);
        }

        /// A client that has asked for a window and committed, as every Rivet
        /// client does before it waits for its size.
        fn with_a_window() -> Self {
            let mut h = Self::new();
            h.send(DISPLAY, 1, &REGISTRY.to_ne_bytes()); // wl_display.get_registry
            h.send(DISPLAY, 0, &SYNC.to_ne_bytes()); // wl_display.sync
            h.pump();
            let globals = globals(&h.events());

            h.bind(&globals, "wl_compositor", 4, COMPOSITOR);
            h.bind(&globals, "xdg_wm_base", 1, WM_BASE);
            h.send(COMPOSITOR, 0, &SURFACE.to_ne_bytes()); // create_surface
            h.send(WM_BASE, 2, &words(&[XDG_SURFACE, SURFACE])); // get_xdg_surface
            h.send(XDG_SURFACE, 1, &TOPLEVEL.to_ne_bytes()); // get_toplevel
            h.send(SURFACE, 6, &[]); // commit
            h.pump();
            h
        }

        /// `wl_surface.frame` then `wl_surface.commit`: ask to be told when
        /// the next frame is on the glass.
        fn ask_for_a_frame(&mut self, callback: u32) {
            self.send(SURFACE, 3, &callback.to_ne_bytes());
            self.send(SURFACE, 6, &[]);
            self.pump();
        }
    }

    /// The wire's string: length including the NUL, the bytes, the NUL, and
    /// padding to four.
    fn string(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&u32::try_from(s.len() + 1).unwrap().to_ne_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_ne_bytes()).collect()
    }

    /// Every `wl_registry.global` among `events`, as (name, interface).
    fn globals(events: &[Event]) -> Vec<(u32, String)> {
        events
            .iter()
            .filter(|e| e.object == REGISTRY && e.opcode == 0)
            .map(|e| {
                let name = e.word(0);
                let length = e.word(1) as usize;
                let interface = String::from_utf8(e.args[8..8 + length - 1].to_vec()).unwrap();
                (name, interface)
            })
            .collect()
    }

    /// The sizes every `xdg_toplevel.configure` among `events` carried.
    fn configured(events: &[Event]) -> Vec<(i32, i32)> {
        events
            .iter()
            .filter(|e| e.object == TOPLEVEL && e.opcode == 0)
            .map(|e| (e.word(0) as i32, e.word(1) as i32))
            .collect()
    }

    /// Whether `wl_callback.done` arrived for `callback`.
    fn done(events: &[Event], callback: u32) -> bool {
        events.iter().any(|e| e.object == callback && e.opcode == 0)
    }

    /// A screen as `Kms::rescan` hands one over: named, at one mode.
    fn screen(width: i32, height: i32) -> Output {
        let output = Output::new(
            "HDMIA-1".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Rivet".into(),
                model: "test".into(),
            },
        );
        let mode = Mode { size: (width, height).into(), refresh: 60_000 };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        output
    }

    /// **The sequence R-236 asks for**: no screen at start, a window that
    /// waits for one, a screen arriving, leaving, and a different one
    /// arriving — and through all of it the client is never cut off.
    #[test]
    fn a_window_waits_for_a_screen_and_follows_it_as_it_comes_and_goes() {
        let mut h = Harness::with_a_window();
        let before = h.events();
        assert!(
            configured(&before).is_empty(),
            "a window was configured with no screen to be the size of: {before:?}"
        );
        assert!(h.state.output.is_none() && h.state.output_global.is_none());

        // A display arrives.
        h.state.screen_changed(Some(screen(1920, 1080)));
        h.pump();
        let arrived = h.events();
        assert_eq!(configured(&arrived), [(1920, 1080)], "{arrived:?}");
        assert!(
            globals(&arrived).iter().any(|(_, i)| i == "wl_output"),
            "the screen was not advertised: {arrived:?}"
        );
        assert_eq!(h.state.space.outputs().count(), 1);

        // It leaves: the global is withdrawn, the window is left alone.
        h.state.screen_changed(None);
        h.pump();
        let left = h.events();
        assert!(
            left.iter().any(|e| e.object == REGISTRY && e.opcode == 1),
            "the departed screen's global was not withdrawn: {left:?}"
        );
        assert!(configured(&left).is_empty(), "an unplug reconfigured the window: {left:?}");
        assert!(h.state.output.is_none());
        assert_eq!(h.state.space.outputs().count(), 0);
        assert_eq!(h.state.space.elements().count(), 1, "the window must outlive the screen");

        // A different display arrives, smaller: the window is told its new size.
        h.state.screen_changed(Some(screen(1280, 720)));
        h.pump();
        let other = h.events();
        assert_eq!(configured(&other), [(1280, 720)], "{other:?}");
        assert_eq!(h.state.space.outputs().count(), 1);
    }

    /// **An unplug pauses frame callbacks and nothing else** — `.rulings/`
    /// R-236. A callback asked for while there is no screen is held, not
    /// dropped, and is answered by the first frame after a screen returns.
    #[test]
    fn frame_callbacks_pause_while_there_is_no_screen() {
        let mut h = Harness::with_a_window();
        h.state.screen_changed(Some(screen(1920, 1080)));
        h.pump();
        h.events();

        // A client allocates ids in order, and the next free one is 9.
        h.ask_for_a_frame(9);
        h.state.send_frame_callbacks();
        h.pump();
        assert!(done(&h.events(), 9), "a frame with a screen was not answered");

        h.state.screen_changed(None);
        h.pump();
        h.events();
        h.ask_for_a_frame(10);
        h.state.send_frame_callbacks();
        h.pump();
        assert!(!done(&h.events(), 10), "a frame callback was answered with no screen");

        h.state.screen_changed(Some(screen(1920, 1080)));
        h.state.send_frame_callbacks();
        h.pump();
        assert!(done(&h.events(), 10), "the held callback was not answered when the screen came back");
    }

    /// The same display unplugged and plugged back in at the same size: the
    /// window already has that size, so it is sent nothing to redo.
    #[test]
    fn the_same_size_again_sends_no_configure() {
        let mut h = Harness::with_a_window();
        h.state.screen_changed(Some(screen(1920, 1080)));
        h.pump();
        assert_eq!(configured(&h.events()), [(1920, 1080)]);
        h.state.screen_changed(None);
        h.state.screen_changed(Some(screen(1920, 1080)));
        h.pump();
        assert!(configured(&h.events()).is_empty());
    }

    /// A window arriving while a screen is up is configured at once, as it
    /// always was.
    #[test]
    fn a_window_arriving_with_a_screen_is_configured_at_once() {
        let mut h = Harness::new();
        h.state.screen_changed(Some(screen(1920, 1080)));
        h.send(DISPLAY, 1, &REGISTRY.to_ne_bytes());
        h.send(DISPLAY, 0, &SYNC.to_ne_bytes());
        h.pump();
        let globals = globals(&h.events());
        h.bind(&globals, "wl_compositor", 4, COMPOSITOR);
        h.bind(&globals, "xdg_wm_base", 1, WM_BASE);
        h.send(COMPOSITOR, 0, &SURFACE.to_ne_bytes());
        h.send(WM_BASE, 2, &words(&[XDG_SURFACE, SURFACE]));
        h.send(XDG_SURFACE, 1, &TOPLEVEL.to_ne_bytes());
        h.send(SURFACE, 6, &[]);
        h.pump();
        assert_eq!(configured(&h.events()), [(1920, 1080)]);
    }
}
