//! `rivet-launcher` — **the menu, and nothing else.**
//!
//! `ADR-058` §4 split this program in two on 2026-09-20 and this is the half
//! that is allowed to fail. `rivet-displayd` owns the GPU and the screen; this
//! draws a menu and asks the supervisor to start something. If it crashes it is
//! restarted, and the screen survives — which is the whole point of the split.
//!
//! # What it stopped being
//!
//! It used to hold **DRM master for the life of the machine**: enumerate
//! connectors, set a mode, allocate dumb buffers, page-flip, cut a lease for
//! the application it started, revoke it afterwards, and fight the framebuffer
//! console over who owns `/dev/fb0`. `drm.rs` was 26 000 bytes of that and is
//! deleted. So are `confirm.rs` and `farewell.rs`, and those two are worth a
//! sentence because they were not dead code — they were **the exit dialog**,
//! and the reason they could never answer *"no, don't exit"* was that a
//! revoked lease cannot be resumed (`§104.36.4`). Under a compositor there is
//! no lease to revoke: the launcher's surface never goes away, it is simply
//! behind whatever is running. The dialog those files existed for does not
//! have a problem to solve any more.
//!
//! # What it is now
//!
//! A Wayland client, about 300 lines, that:
//!
//! 1. connects to the socket `rivet_core::compositor` names,
//! 2. asks the supervisor what this machine offers (`ADR-042` §8 — it may
//!    not read the operator's document),
//! 3. draws `crate::menu`'s layout into a shared-memory buffer,
//! 4. reads the `ADR-015` gesture vocabulary off evdev and the control plane,
//! 5. asks the supervisor to select one, or to restart the machine.
//!
//! It is still an image service and still runs as uid 0 with no Landlock
//! ruleset — `crate::service::IMAGE_DEFINED` — which is what lets
//! `crate::address` read `/proc` to say where this machine answers.

mod address;
mod agent;

mod input;
mod kiosk;
mod menu;
mod offered;

mod surface;
mod text;
mod ui;

use std::os::unix::fs::OpenOptionsExt;

/// The name this program answers to in the journal.
const ME: &str = "rivet-launcher";

/// How often the loop turns.
///
/// 50 ms, carried over from the DRM launcher and for its reason: nothing here
/// animates, and the number that matters is [`input::HOLD`], which is two
/// seconds. A loop an order of magnitude faster than the shortest gesture
/// cannot miss one.
const TICK: std::time::Duration = std::time::Duration::from_millis(50);

/// How many times to ask the supervisor for the menu before drawing without it.
const MENU_TRIES: u32 = 8;

fn main() -> std::process::ExitCode {
    // SAFETY: `note_stopping` only stores into an `AtomicBool`, which is one of
    // the few things a signal handler may do. Everything the shutdown does
    // happens on the main loop, which merely reads it.
    unsafe {
        signal(SIGTERM, note_stopping as *const () as usize);
    }

    match run() {
        Ok(()) => {
            println!("{ME}: the menu is done");
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            println!("{ME}: STOPPING — {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    println!("{ME}: starting, pid {}", std::process::id());

    let face = match text::Face::load(text::SYSTEM_FONT) {
        Ok(f) => {
            println!("{ME}: type from {}", text::SYSTEM_FONT);
            Some(f)
        }
        Err(why) => {
            // **Not fatal, and `ADR-018` §8 is why.** A machine missing a font
            // is still a machine somebody can look at and repair; a launcher
            // that refused to start over one would take the screen away
            // instead. `menu.rs` has a test for exactly this.
            println!("{ME}: NO SYSTEM FONT — {why}");
            None
        }
    };

    let (mut screen, mut queue, conn) = surface::open()?;
    let qh = queue.handle();

    let mut state = State {
        menu: ask_menu(),
        focus: menu::Focus::Card(0),
        clock: now_hhmm(),

        host: None,
        address: None,
        redraw: true,
    };
    state.refresh_identity();
    // **The kiosk's application is started before there is a screen to show
    // it on** — `.rulings/` R-236: applications start without one and are
    // handed a surface when one arrives. Waiting for the configure first would
    // make a kiosk booted with its screen off start nothing until somebody
    // plugged one in.
    state.auto_run();

    // **The first configure is what tells this process how big its surface
    // is.** Drawing before it arrives would put a 1280x720 picture on a
    // 1920x1080 screen. With no display plugged in it does not come until one
    // is, and the launcher waits for it as long as the compositor answers —
    // `surface::wait_for_configure`, which still refuses a compositor that
    // has gone silent (B10).
    surface::wait_for_configure(
        &conn,
        &mut queue,
        &mut screen,
        surface::CONFIGURE_PATIENCE,
        || println!("{ME}: no display is connected; the menu is drawn when one is"),
    )
    .map_err(|e| format!("the compositor did not answer: {e}"))?;

    let mut inputs = open_inputs();
    println!("{ME}: {} input device(s)", inputs.len());
    let presses = agent::Presses::offered();

    let mut held: Option<std::time::Instant> = None;
    let mut last_identity = std::time::Instant::now();

    while !stopping() {
        // **Bounded, so a hung compositor ends the launcher rather than
        // holding it for ever.** The error is the journal's record of what was
        // being waited for; `restart = "always"` brings the launcher back.
        surface::frame_round_trip(&conn, &mut queue, &mut screen, surface::CONFIGURE_PATIENCE)?;
        if screen.closed {
            return Ok(());
        }

        for action in gather(&mut inputs, presses.as_ref(), &mut held) {
            state.act(action);
        }

        let clock = now_hhmm();
        if clock != state.clock {
            state.clock = clock;
            state.redraw = true;
        }

        // **The address is re-read every half minute**, not every frame.
        // `crate::address` walks `/proc/<pid>/fd`, which is cheap and is not
        // free, and a control plane does not move between two ticks of a
        // clock. It is re-read at all because DHCP exists.
        if last_identity.elapsed() >= std::time::Duration::from_secs(30) {
            last_identity = std::time::Instant::now();
            state.refresh_identity();
        }

        if state.redraw || screen.dirty {
            state.redraw = false;
            let offered = state.menu.offered.clone();
            let frame = menu::Frame {
                hostname: state.host.as_deref(),
                address: state.address.as_deref(),
                offered: &offered,
                focus: state.focus,
                clock: &state.clock,

            };
            if let Err(why) = screen.paint(&qh, |canvas, target| {
                menu::draw(canvas, target, face.as_ref(), &frame);
            }) {
                println!("{ME}: the menu was not drawn — {why}");
            }
        }

        std::thread::sleep(TICK);
    }

    println!("{ME}: SIGTERM received; the surface goes with this process");
    Ok(())
}

/// What the launcher knows between two turns of the loop.
struct State {
    menu: offered::Menu,
    focus: menu::Focus,
    clock: String,

    host: Option<String>,
    address: Option<String>,
    redraw: bool,
}

impl State {
    /// Act on one gesture from `ADR-015`'s vocabulary.
    fn act(&mut self, action: input::Action) {
        // **Said out loud, because `PLAN.md` §104.18 makes two people agreeing
        // about what happened the point** — one reading a journal, one holding
        // a pad in another room. `Action::word` is that vocabulary and this is
        // its only caller.
        println!("{ME}: {}", action.word());
        let cards = self.menu.offered.len();
        match action {
            // **Left and right only, and up and down do the same thing.** The
            // layout is one row, so a person pressing down on a pad should not
            // find that nothing happens — `ADR-015`'s vocabulary is fixed and
            // a menu that ignores half of it reads as broken.
            input::Action::Right | input::Action::Down => {
                self.focus = self.focus.next(cards);
                self.redraw = true;
            }
            input::Action::Left | input::Action::Up => {
                self.focus = self.focus.previous(cards);
                self.redraw = true;
            }
            input::Action::Enter => self.choose(),
            input::Action::Back => {
                self.menu = ask_menu();
                self.redraw = true;
            }
            input::Action::BackHeld => self.restart(),
            input::Action::Capture => self.capture(),
        }
    }

    /// Start what the focus is on, or restart the machine.
    fn choose(&mut self) {
        match self.focus {
            menu::Focus::Power => self.restart(),
            menu::Focus::Card(index) => {
                let Some(offer) = self.menu.offered.get(index) else {
                    return;
                };
                println!("{ME}: asking the supervisor for {}", offer.name);
                let request = rivet_core::kiosk::Request::Select {
                    name: offer.name.clone(),
                };
                match kiosk::ask(&request) {
                    // **Nothing else happens here, and that is the change.**
                    // The DRM launcher had to drop master so the chosen
                    // application could take it. Under a compositor this
                    // surface simply stays where it is, behind whatever the
                    // supervisor starts, and comes back when that exits.
                    Ok(answer) => println!("{ME}: {answer:?}"),
                    Err(why) => {
                        println!("{ME}: {}: {why}", offer.name);

                        self.redraw = true;
                    }
                }
            }
        }
    }

    fn restart(&mut self) {
        println!("{ME}: asking the supervisor to restart this machine");
        match kiosk::ask(&rivet_core::kiosk::Request::Restart) {
            Ok(answer) => println!("{ME}: {answer:?}"),
            // The journal, and nowhere else. See `auto_run`.
            Err(why) => println!("{ME}: this machine could not be restarted: {why}"),
        }
    }

    /// **Ask the compositor for a picture of the screen** — `ADR-058` §5.
    ///
    /// `ADR-012` §9 makes `apply input capture` an action an agent may send,
    /// and it arrives here because this program provides the input endpoint.
    /// **It is forwarded rather than performed**: a client cannot see the
    /// screen, only its own surface, and a "screenshot" that photographed the
    /// menu while an application was on top of it would be worse than none —
    /// it would look right.
    ///
    /// The launcher is a courier here and nothing more. Nothing is read back
    /// over this connection but the compositor's own sentence, which goes
    /// straight to the journal so an operator who sent the press can see what
    /// happened without a second question.
    fn capture(&mut self) {
        use std::io::{Read, Write};

        let path = capture_socket();
        let mut stream = match std::os::unix::net::UnixStream::connect(&path) {
            Ok(stream) => stream,
            Err(why) => {
                println!("{ME}: {path}: the compositor would not take a capture request: {why}");
                return;
            }
        };
        if let Err(why) = stream.write_all(rivet_core::compositor::CAPTURE_REQUEST.as_bytes()) {
            println!("{ME}: the capture request could not be sent: {why}");
            return;
        }
        // **A deadline, because this is the draw loop.** The compositor renders
        // a whole extra frame and writes a file; a blocking read with no bound
        // would stop the menu for as long as the filesystem took.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut answer = String::new();
        match stream.read_to_string(&mut answer) {
            Ok(_) => println!("{ME}: capture — {}", answer.trim_end()),
            Err(why) => println!("{ME}: the capture was asked for and not answered: {why}"),
        }
    }

    /// **`[kiosk] auto-run`: start the named application without being asked.**
    ///
    /// Run once, after the menu is read and before the loop turns. A machine
    /// in a lobby comes up showing the thing it is for, and nobody has to press
    /// anything — which is what a kiosk IS, and the reason `[kiosk]` is a mode
    /// and not a preference.
    ///
    /// # It was missing from the first Wayland launcher, for one day
    ///
    /// The DRM launcher did this and the rewrite dropped it. Nothing failed:
    /// the machine came up on a menu that a person could drive, which looks
    /// like working — which is exactly why the maintainer asking whether the new
    /// launcher actually autostarted applications found it and no test did
    /// (.rulings/ R-335).
    ///
    /// # Why an unknown name is drawn rather than only logged
    ///
    /// A document naming an application that is not offered is an operator's
    /// typo or an application that failed to install, and it goes to the
    /// JOURNAL. `ADR-017` §7 captures this program's output, so `rivet get log`
    /// and `cargo rivet run -- get log` both have it.
    ///
    /// **An earlier version drew it on the screen** and argued that the person
    /// who can fix it is standing in front of one. The maintainer, on seeing
    /// it, had the error removed and ruled that errors belong in journals and
    /// not on his screen (.rulings/ R-205) — `crate::menu` carries why that
    /// argument was wrong on its own terms.
    fn auto_run(&mut self) {
        let Some(name) = self.menu.auto_run.clone() else {
            return;
        };
        if !self.menu.offered.iter().any(|o| o.name == name) {
            println!("{ME}: [kiosk] auto-run names {name}, which is not offered");
            return;
        }
        println!("{ME}: [kiosk] auto-run names {name}, starting it");
        match kiosk::ask(&rivet_core::kiosk::Request::Select { name: name.clone() }) {
            Ok(answer) => println!("{ME}: {answer:?}"),
            Err(why) => {
                println!("{ME}: auto-run did not start: {why}");
            }
        }
    }

    /// Re-read what this machine is called and where it answers.
    fn refresh_identity(&mut self) {
        let facts = rivet_sdk::facts::ask().ok();
        let host = facts.as_ref().and_then(|f| f.hostname.clone());
        let mcpd = facts
            .as_ref()
            .and_then(|f| f.service(rivet_core::service::MCPD_SERVICE_NAME))
            .and_then(|s| s.pid);
        let address = address::control_plane(mcpd);

        if host != self.host || address != self.address {
            self.host = host;
            self.address = address;
            self.redraw = true;
        }
    }
}

/// Ask the supervisor what this machine offers, a few times.
///
/// **An unanswered ask is not fatal** — `ADR-018` §8. A launcher that refused
/// to start because one socket request failed would take the screen away over
/// a fault that may have nothing to do with the screen, and there is no longer
/// a document to fall back on reading.
fn ask_menu() -> offered::Menu {
    let mut last = String::new();
    for attempt in 0..MENU_TRIES {
        match kiosk::ask(&rivet_core::kiosk::Request::Offer) {
            Ok(answer) => match offered::from_answer(&answer) {
                Ok(menu) => {
                    println!("{ME}: {} application(s) offered", menu.offered.len());
                    return menu;
                }
                Err(why) => last = why,
            },
            Err(why) => last = why,
        }
        if attempt + 1 < MENU_TRIES {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    println!("{ME}: the menu could not be read — {last}");
    offered::Menu::unanswered(last)
}

/// Everything pressed since the last pass, from every source.
fn gather(
    inputs: &mut [(String, std::fs::File)],
    presses: Option<&agent::Presses>,
    held: &mut Option<std::time::Instant>,
) -> Vec<input::Action> {
    use std::io::Read;

    let mut actions = Vec::new();
    let mut buffer = [0u8; input::EVENT_SIZE * 64];

    for (path, file) in inputs.iter_mut() {
        let read = match file.read(&mut buffer) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => {
                println!("{ME}: {path} stopped answering: {e}");
                continue;
            }
        };
        let Some(chunk) = buffer.get(..read) else {
            continue;
        };
        for (kind, code, value) in input::events_in(chunk) {
            // The back control is decided on RELEASE, because a hold is a
            // different event rather than a longer one — the maintainer named
            // that hazard before it could be got wrong.
            if let Some(down) = input::back_edge(kind, code, value) {
                if down {
                    *held = Some(std::time::Instant::now());
                } else if let Some(since) = held.take() {
                    actions.push(if since.elapsed() >= input::HOLD {
                        input::Action::BackHeld
                    } else {
                        input::Action::Back
                    });
                }
                continue;
            }
            if let Some(action) = input::action_of(kind, code, value) {
                actions.push(action);
            }
        }
    }

    // A hold that has passed the threshold fires without waiting for a
    // release, so the machine responds while the button is still down.
    if let Some(since) = *held {
        if since.elapsed() >= input::HOLD {
            *held = None;
            actions.push(input::Action::BackHeld);
        }
    }

    if let Some(presses) = presses {
        let remote = presses.drain();
        // **Attributed, and `ADR-012` §9 is the whole reason.** A journal
        // that cannot tell a person at the panel from an agent on the control
        // plane is a journal nobody can use to work out what happened.
        for action in &remote {
            println!("{ME}: {} from {}", action.word(), agent::ACTOR);
        }
        actions.extend(remote);
    }
    actions
}

/// Where the compositor accepts capture requests.
///
/// The supervisor's own layout for a provided endpoint, composed rather than
/// spelled — `rivet_core::compositor::socket_path` does the same for the
/// Wayland one and for the same reason.
fn capture_socket() -> String {
    use rivet_core::service::{provided_socket_path, ServiceName};

    match ServiceName::new(rivet_core::compositor::PROVIDER) {
        Some(name) => provided_socket_path(&name, rivet_core::compositor::CAPTURE_ENDPOINT),
        None => format!(
            "{}/{}/{}",
            rivet_core::service::LISTEN_DIR,
            rivet_core::compositor::PROVIDER,
            rivet_core::compositor::CAPTURE_ENDPOINT
        ),
    }
}

/// Every `/dev/input/event*`, opened non-blocking.
fn open_inputs() -> Vec<(String, std::fs::File)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        println!("{ME}: no /dev/input, so nothing can be chosen from the panel");
        return out;
    };
    let mut paths: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("event"))
        .collect();
    paths.sort();
    for name in paths {
        let path = format!("/dev/input/{name}");
        // `O_NONBLOCK`, because one quiet device must not stall the loop every
        // other device and the long-press clock share.
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => out.push((path, file)),
            Err(e) => println!("{ME}: {path} could not be opened: {e}"),
        }
    }
    out
}

/// `HH:MM` in local time.
///
/// **Local and not UTC**, because a clock on a wall is read by a person in a
/// room. `musl` owns the conversion and reads the tzfile `ADR-021` §9
/// projects, so this is the same answer `rivet get time` gives.
fn now_hhmm() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let clock = i64::try_from(now).unwrap_or(0);

    // SAFETY: `localtime_r` writes into a `tm` this call owns, and the pointer
    // is valid for the whole call. The reentrant form is used precisely so
    // nothing is shared with any other caller.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let filled = unsafe { libc::localtime_r(&raw const clock, &raw mut tm) };
    if filled.is_null() {
        return "--:--".to_string();
    }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// **A controlled shutdown has begun** — set from a signal handler.
static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const SIGTERM: core::ffi::c_int = 15;

unsafe extern "C" {
    fn signal(sig: core::ffi::c_int, handler: usize) -> usize;
}

extern "C" fn note_stopping(_sig: core::ffi::c_int) {
    STOPPING.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn stopping() -> bool {
    STOPPING.load(std::sync::atomic::Ordering::SeqCst)
}
