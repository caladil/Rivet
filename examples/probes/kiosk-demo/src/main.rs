//! Open the screen and every input device, and say exactly what happened.
//!
//! This is the artifact behind `ADR-015` §6.2. It proves the grant by doing the
//! two things an application on a Rivet cannot do without it — driving the
//! display it was handed and reading `/dev/input/event*` — and it is written so
//! that a failure is a **report** rather than a crash, because a program that
//! died on `EACCES` would tell you only that it died.
//!
//! ~~writing `/dev/fb0`~~ **Struck 2026-09-04.** It stopped writing the
//! framebuffer on 2026-09-01, when `crate::drm` replaced it with KMS, and the
//! sentence outlived the code by three days. It is now false in a second way as
//! well: a lessee is not granted `/dev/fb0` at all (`ADR-036` §3.1, amended
//! 2026-09-04), so writing it is not something this program *could* do with the
//! grant it has.
//!
//! It draws a heartbeat that changes every tick, so a person looking at the
//! screen can see it is alive without reading a journal. And it names each
//! input event by device and by action, so a person can press a key and watch
//! the screen agree with the journal.
//!
//! **It draws with the platform's own type and shapes**, since 2026-09-04 —
//! `rivet_sdk::text` and `rivet_sdk::draw`, against the faces the image
//! carries. ~~A five-by-seven bitmap font, built in, because the image has no
//! font stack.~~ The image has had one since 2026-09-01; this crate's copy of
//! `cmd/rivet-splash`'s bitmap outlived it by three days, and it had no capital
//! letters, so the screen said `long back` where the journal said `LONG BACK`.
//! An example whose job is to demonstrate the platform should not look worse
//! than the platform.

mod drm;

use std::io::Read;
use std::os::fd::AsRawFd;

/// **The card this program opens only when nothing else can be holding it.**
///
/// One screen, and `ADR-015` §6.2 makes it one owner. ~~`ADR-015` §6.2's
/// `display` class grants the DRM nodes and `/dev/fb0` together.~~ **Corrected
/// 2026-09-04** (`ADR-036` §3.1, `.rulings/` R-138): the class expands to
/// `/dev/dri/renderD*` for a LESSEE, which is what this program is under a
/// supervisor. ~~The card node stays `root:root`~~ — **corrected 2026-09-04 by
/// the board**, which reported it `mode 0600 uid 34030 gid 34030`, and 34030 is
/// `rivet-launcher`: the card node is granted to the ARBITER, which is what
/// `ADR-036` §3.1.1's table says and what `root:root` was a careless shorthand
/// for. Either way this application does not own it and the open is refused, so
/// this path is reachable only where it was always correct — run by hand on a
/// board with no launcher, as root. See [`may_take_the_card_by_path`].
const CARD: &str = "/dev/dri/card0";

/// Where evdev nodes live.
const INPUT_DIR: &str = "/dev/input";

fn main() {
    println!(
        "kiosk-demo: uid {}, pid {}",
        own_uid().unwrap_or_else(|| "unknown".to_string()),
        std::process::id()
    );

    // **Handed a display, or take one** — `PLAN.md` §106.2, `ADR-036` §3.1.
    //
    // A Rivet kiosk application is a LESSEE: the launcher holds DRM master for
    // the life of the machine and sends a lease on `RIVET_DISPLAY`. Opening the
    // card by path answers `EACCES` while anything else is master, so the
    // handed descriptor is tried first and always.
    //
    // The fallback is not politeness. This example is also run BY HAND on a
    // board with no launcher, which is how it was written and how it is still
    // debugged — and on that machine there is no lease and opening the node is
    // correct. **What was wrong is that it did not check which of those it was
    // in** — see `may_take_the_card_by_path`.
    let handed = match rivet_sdk::display::Channel::from_env() {
        Ok(channel) => {
            println!("kiosk-demo: waiting for a display on {}", rivet_sdk::display::DISPLAY_ENV);
            match channel.next() {
                Ok(fd) => {
                    // Leaked deliberately: `drm::Screen` owns the descriptor
                    // for the life of the program and closes it in `release`.
                    // Dropping the `OwnedFd` here would close the display the
                    // line below is about to drive.
                    let raw = std::os::fd::IntoRawFd::into_raw_fd(fd);
                    println!("kiosk-demo: leased display on descriptor {raw}");
                    Some(raw)
                }
                Err(why) => {
                    println!("kiosk-demo: {why}");
                    None
                }
            }
        }
        Err(why) => {
            println!("kiosk-demo: {why}");
            None
        }
    };

    // **Which of the two it was, in one word, for the screen.** A person at the
    // panel should be able to tell a leased display from one this program
    // opened by path without reading a journal — they are different proofs and
    // only the first is what a supervised application does.
    let how = if handed.is_some() { "leased" } else { "direct" };

    let taken = match handed {
        Some(raw) => drm::take_leased(raw),
        None => match may_take_the_card_by_path() {
            Ok(()) => drm::take(CARD),
            Err(why) => Err(why),
        },
    };
    let mut screen = match taken {
        Ok(screen) => {
            println!(
                "kiosk-demo: TOOK THE DISPLAY - {} - {}x{} stride {}",
                screen.what, screen.width, screen.height, screen.stride
            );
            println!(
                "kiosk-demo: DRM master is held, so the framebuffer console is suspended and \
                 the journal can no longer draw over this program (PLAN.md §104.33.3)"
            );
            Some(screen)
        }
        Err(why) => {
            println!("kiosk-demo: NO DISPLAY - {why}");
            println!(
                "kiosk-demo: the program continues and reports input, because a screen that \
                 could not be taken is a thing to report rather than to die of"
            );
            None
        }
    };

    report_dri_nodes(screen.is_some());

    // Loaded once, before the loop, and only if there is something to draw on.
    let faces = if screen.is_some() { Faces::load() } else { Faces { sans: None, mono: None } };

    let mut inputs = open_inputs();
    println!("kiosk-demo: {} input device(s) opened", inputs.len());

    // The screen's shape. `/dev/fb0` has no ioctl-free way to ask, so this
    // writes a bounded number of bytes rather than assuming a resolution —
    // enough to be visible, never enough to run off the end of a mapping this
    // program does not know the size of.
    let mut tick: u64 = 0;
    let mut events: u64 = 0;
    // The last few actions, newest first, as (device, action).
    let mut recent: Vec<(String, Action)> = Vec::new();
    // When BACK went down and has not come up. An `Instant` rather than a tick
    // count: the loop's period is a detail and two seconds is the ruling.
    let mut back_since: Option<std::time::Instant> = None;
    let mut held_announced = false;
    loop {
        if let Some(surface) = screen.as_mut() {
            // **A screen a person can read, since 2026-08-31.**
            //
            // This used to write a band of one colour whose shade changed every
            // tick: a liveness signal, and the right thing while the question
            // was *"does the display grant work"*. The maintainer asked for
            // something else before a controller test — an on-screen indication
            // of which key event was captured from which device, so that each
            // input can be tested and both parties agree on what happened
            // (`.rulings/` R-353) — and a colour cannot say which device or
            // which button. `PLAN.md` §104.18.
            //
            // **Two people have to agree on what happened**, one of them
            // reading a journal over a control plane and one of them holding a
            // pad in another room. A number changing on a screen would only
            // tell the second of them that SOMETHING arrived.
            // **The gesture is PROVEN here, not acted on.** This program does
            // not exit: stopping a kiosk application is the supervisor's, and
            // an example that did it would be claiming a power `ADR-015` gives
            // it no way to have. What it does is show the hold was seen, so the
            // gesture is known to be detectable before `rivet-sysd` is taught
            // to act on it.
            if let Some(since) = back_since
                && since.elapsed() >= HOLD
                && !held_announced
            {
                held_announced = true;
                recent.insert(0, ("held".to_string(), Action::BackHeld));
                recent.truncate(RECENT);
                println!(
                    "kiosk-demo: LONG BACK - held past two seconds. The supervisor would stop \
                     this application here; this example only reports it"
                );
            }
            draw_screen(surface, &faces, &recent, tick, events, inputs.len(), how);
        }

        // **Look for devices that appeared since the last tick.** A Rivet has
        // no udev and this program has no netlink socket, so `/dev/input` is
        // re-listed instead — eight entries once a second, which is nothing.
        //
        // It is here because without it this example answers the wrong
        // question. It enumerated once at startup, so a controller plugged in
        // afterwards was never opened: the tester presses A, nothing happens,
        // and the conclusion is *input is broken* when the truth is *nobody
        // was listening*. `PLAN.md` §104.16.4.
        for (path, name, file) in appeared(&inputs) {
            println!("kiosk-demo: PLUGGED IN {path} — {name}");
            inputs.push((path, file));
        }

        // Drain whatever input arrived. Non-blocking, so a tick never waits on
        // a keyboard nobody is touching. Devices that went away are dropped
        // after the loop — removing inside it would shift the indices under it.
        let mut gone: Vec<String> = Vec::new();
        for (path, file) in inputs.iter_mut() {
            let mut scratch = [0u8; 1024];
            match file.read(&mut scratch) {
                Ok(0) => {}
                Ok(n) => {
                    // An evdev event is 24 bytes on 64-bit.
                    //
                    // **Decoded, since 2026-08-31.** It used to count bytes and
                    // say so, on the reasoning that what was being proved was
                    // that the descriptor delivers. That was right until the
                    // maintainer named a launcher driven by up/down/left/right/
                    // enter/back from a keyboard OR a controller — at which
                    // point `48 bytes of input` stops answering the question
                    // and `D-pad UP` starts. `PLAN.md` §104.16.
                    events += (n / 24) as u64;
                    let decoded: Vec<&str> = events_in(&scratch[..n])
                        .filter_map(|(kind, code, value)| action_of(kind, code, value))
                        .map(Action::word)
                        .collect();
                    for (k, c, v) in events_in(&scratch[..n]) {
                        match back_edge(k, c, v) {
                            Some(true) if back_since.is_none() => {
                                back_since = Some(std::time::Instant::now());
                            }
                            Some(false) => {
                                // **The short press is decided here.** If the
                                // long one already fired, this release is the
                                // end of that gesture and not a second event.
                                if let Some(since) = back_since
                                    && !held_announced
                                    && since.elapsed() < HOLD
                                {
                                    recent.insert(0, (name_of(path), Action::Back));
                                    recent.truncate(RECENT);
                                    events += 1;
                                    println!("kiosk-demo: {path} - BACK");
                                }
                                back_since = None;
                                held_announced = false;
                            }
                            _ => {}
                        }
                    }
                    for action in events_in(&scratch[..n])
                        .filter_map(|(k, c, v)| action_of(k, c, v))
                    {
                        // Newest first, and a short window: this is a thing
                        // somebody reads off a monitor from across a room.
                        recent.insert(0, (name_of(path), action));
                        recent.truncate(RECENT);
                    }
                    if decoded.is_empty() {
                        // Still reported. An event this build does not map is
                        // not nothing — it is a device saying something in a
                        // vocabulary the launcher would ignore, and knowing
                        // that is the difference between a dead pad and an
                        // unmapped one.
                        println!(
                            "kiosk-demo: {n} bytes from {path} — no launcher action in them"
                        );
                    } else {
                        println!("kiosk-demo: {path} — {}", decoded.join(" "));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    // **Unplugged, most likely.** A removed evdev node answers
                    // ENODEV forever, so keeping it would print this once a
                    // second until the machine is restarted.
                    println!("kiosk-demo: UNPLUGGED {path}: {e}");
                    gone.push(path.clone());
                }
            }
        }
        inputs.retain(|(path, _)| !gone.contains(path));

        // **Once every ten seconds, though the loop runs every second.** The
        // journal is on an SD card (`ADR-022` §8) and a heartbeat that wrote
        // ten times as often would spend the retention this example is not
        // important enough to spend. Input is reported as it arrives, above.
        if tick % 10 == 0 {
            println!(
                "kiosk-demo: tick {tick} — screen {}, {} input device(s), {events} event(s) seen",
                if screen.is_some() { "held" } else { "DENIED" },
                inputs.len()
            );
        }
        tick += 1;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// **May this program open the card by path when it was handed no lease?**
///
/// `ADR-036` §3.1, register `R-33`. The fallback below `take_leased` used to be
/// unconditional, and the reasoning for it was sound in the case it was written
/// for and wrong in the other one:
///
/// * **Run by hand on a board with no launcher** — which is how this example
///   was written and how it is still debugged — there is no lease, nothing else
///   is DRM master, and opening `/dev/dri/card0` is exactly right.
/// * **Started by a supervisor with no display granted** — the launcher is
///   holding the screen and drawing the menu. ~~`ADR-015` §6.2's grant chowned
///   `card*` to this application's uid, so the open **succeeds**, and this
///   program becomes DRM master the moment the launcher's own master drops for
///   an instant.~~ **That was true until 2026-09-04**, and it was `ADR-036`
///   §3.1's *"no compatibility path"* being untrue, demonstrated by the example
///   that is supposed to teach it. The grant is narrowed now (`.rulings/`
///   R-138): a lessee is given `/dev/dri/renderD*` and the card node is the
///   ARBITER's — `0600` and the launcher's own uid, read off the board on
///   2026-09-04 — so the open is refused by the **kernel**, with `EACCES`.
///
/// # So why keep asking, when the kernel now answers
///
/// Because the two gates say different things, and only one of them is legible.
/// `AGENTS.md` §3 ranks the dependency graph above review above convention; the
/// same ranking applies here, and **the kernel's refusal is the strong gate**.
/// What it cannot do is explain itself: an operator reading `EACCES` on
/// `/dev/dri/card0` learns that a permission is missing, not that this machine
/// hands displays out on a socket and that the way to get one is to select this
/// application from the menu. This check exists to say that sentence, and it is
/// deliberately not the thing standing between the program and the node.
///
/// `RIVET_APP_DATA` is set for every application a supervisor starts and for
/// nothing else, so its presence is the whole of the distinction. It is asked
/// through the SDK rather than read as a literal, because that is the point
/// `ADR-036` §3 is making.
///
/// # Errors
///
/// A sentence naming which case this is, for the caller to report.
fn may_take_the_card_by_path() -> Result<(), String> {
    match rivet_sdk::app::data() {
        // Not handed a data directory, so not started by a supervisor: there is
        // no launcher on this machine and nothing else can hold the card.
        Err(_) => Ok(()),
        Ok(_) => Err(format!(
            "a supervisor started this and {} is not set, so the display was NOT granted to \
             it. {CARD} is refused rather than opened, and the kernel would refuse it too — \
             measured on a board 2026-09-04, EACCES, errno 13: a lessee's display grant covers \
             /dev/dri/renderD* only, and the card node belongs to the ARBITER, which is the \
             launcher (ADR-036 §3.1). This says so in a sentence instead of as an EACCES \
             nobody can act on. Select this application from the launcher's menu and it will \
             be handed a lease.",
            rivet_sdk::display::DISPLAY_ENV,
        )),
    }
}

/// Where the DRM nodes live.
const DRI_DIR: &str = "/dev/dri";

/// **Open every DRM node by path and report what the KERNEL said** —
/// `.rulings/` R-138, register `R-32`, `ADR-036` §3.1.
///
/// # Why a chown is not a refusal
///
/// R-138 narrowed the `display` grant: an application handed a LEASE is
/// chowned `/dev/dri/renderD*`, and the card node goes to the ARBITER instead
/// (`ADR-036` §3.1.1's table). The supervisor's tests prove which node is
/// chowned to which uid, and that is a test of the supervisor's intention.
/// **It is not a reading of the outcome.** A `chown` says what the mode bits
/// are; whether the kernel then refuses an `open` is a different sentence, and
/// until 2026-09-04 nobody had watched an application be told no. `PLAN.md`
/// §129.6 listed it as still unproven in exactly those words.
///
/// **What it answered, on the Pi 4, 2026-09-04**, with `kiosk-demo` selected
/// from the launcher's menu and holding a lease as uid 35692:
///
/// ```text
/// GRANT CHECK /dev/dri/card0      mode 0600 uid 34030 gid 34030 — REFUSED, errno 13 (Permission denied (os error 13))
/// GRANT CHECK /dev/dri/card1      mode 0600 uid 34030 gid 34030 — REFUSED, errno 13 (Permission denied (os error 13))
/// GRANT CHECK /dev/dri/renderD128 mode 0600 uid 35692 gid 35692 — OPENED read-write
/// ```
///
/// 34030 is `rivet-launcher`. Keeping the check in the program rather than
/// writing the answer down and deleting it is the point: the grant is a thing
/// the supervisor computes at every start, and a transcript in a document
/// cannot notice when it changes.
///
/// This function is that reading. It runs after the display has been resolved,
/// so the line it prints is stamped with which case the program is in, and it
/// reports **every** node rather than only the card — a refusal on `card0` is
/// only half the finding, and the other half is that `renderD128` opens, which
/// is what says the grant arrived at all rather than nothing being granted.
///
/// # It reports and never acts
///
/// Nothing here is used. The descriptor is dropped at the end of each arm and
/// the program draws on the leased `Screen` it already has. Opening a primary
/// node with no other master would make this process master implicitly, which
/// is precisely the thing R-138 exists to prevent — so this runs AFTER
/// `drm::take*`, where either the launcher is master (and the open is refused,
/// which is the finding) or this program already is (and a second descriptor
/// is not a second master).
fn report_dri_nodes(holding: bool) {
    let uid = own_uid().unwrap_or_else(|| "unknown".to_string());
    let state = if holding { "HOLDING a display" } else { "holding NO display" };

    let Ok(entries) = std::fs::read_dir(DRI_DIR) else {
        println!(
            "kiosk-demo: GRANT CHECK {DRI_DIR} cannot be listed, so this machine has no DRM \
             nodes to report. On a card built without the overlay there are none — that is a \
             build gate now (PLAN.md §123, Wave 4.3) and not something to work around here"
        );
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    println!(
        "kiosk-demo: GRANT CHECK — {} node(s) under {DRI_DIR}, opened by path as uid {uid}, \
         {state}",
        names.len()
    );
    for name in names {
        let path = format!("{DRI_DIR}/{name}");
        let owner = ownership_of(&path);
        let outcome = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map(drop);
        println!("kiosk-demo: GRANT CHECK {}", open_report(&path, &owner, outcome));
    }
}

/// `mode uid gid`, or why they could not be read.
///
/// `MetadataExt` rather than a `stat` binding: the numbers are what make the
/// refusal legible — `crw------- root:root` beside an `EACCES` is a complete
/// sentence, and `EACCES` on its own is not.
fn ownership_of(path: &str) -> String {
    use std::os::unix::fs::MetadataExt as _;
    match std::fs::metadata(path) {
        Ok(m) => format!("mode {:04o} uid {} gid {}", m.mode() & 0o7777, m.uid(), m.gid()),
        Err(e) => format!("cannot be stat'd ({e})"),
    }
}

/// One node's line.
///
/// Split out from [`report_dri_nodes`] so the two directions can be tested
/// without a `/dev/dri` — a host container has none, and a report whose
/// refusal arm has never been exercised is the report that will be wrong on
/// the day it matters.
///
/// **The errno is printed as a NUMBER as well as a sentence.** `AGENTS.md` §5:
/// what goes in the record is what the system said. `Permission denied` is the
/// C library's rendering and `13` is the kernel's answer, and only the second
/// survives being read by somebody whose locale is not this one.
fn open_report(path: &str, owner: &str, outcome: std::io::Result<()>) -> String {
    match outcome {
        Ok(()) => format!("{path} {owner} — OPENED read-write"),
        Err(e) => {
            let code = e
                .raw_os_error()
                .map_or_else(|| "none".to_string(), |n| n.to_string());
            format!("{path} {owner} — REFUSED, errno {code} ({e})")
        }
    }
}

/// Evdev nodes that exist and are not already open.
///
/// **The whole of hotplug on a machine with no udev.** `ADR-015` §6.2 grants
/// the `input` class by chowning the nodes that exist when a service starts —
/// so a node created later is `0600 root:root` and this open FAILS, which is
/// the honest thing for it to do and is reported rather than retried into
/// silence. That is a real limit of the grant and `PLAN.md` §104.16.4 records
/// it; a controller plugged in before the service starts works today.
fn appeared(open: &[(String, std::fs::File)]) -> Vec<(String, String, std::fs::File)> {
    let Ok(entries) = std::fs::read_dir(INPUT_DIR) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let node = entry.file_name().to_string_lossy().into_owned();
        if !node.starts_with("event") {
            continue;
        }
        let path = format!("{INPUT_DIR}/{node}");
        if open.iter().any(|(p, _)| *p == path) {
            continue;
        }
        match open_nonblocking(&path) {
            Ok(file) => found.push((path, device_name(&node), file)),
            Err(e) => println!(
                "kiosk-demo: {path} appeared and could NOT be opened: {e} — a node created \
                 after this service started is not covered by the grant it was given"
            ),
        }
    }
    found
}

/// Open every evdev node, non-blocking, and say which ones refused.
fn open_inputs() -> Vec<(String, std::fs::File)> {
    let mut open = Vec::new();
    let Ok(entries) = std::fs::read_dir(INPUT_DIR) else {
        println!("kiosk-demo: {INPUT_DIR} cannot be listed, so there is no input to open");
        return open;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("event"))
        .collect();
    names.sort();

    for name in names {
        let path = format!("{INPUT_DIR}/{name}");
        match open_nonblocking(&path) {
            Ok(file) => {
                println!("kiosk-demo: OPENED {path} — {}", device_name(&name));
                open.push((path, file));
            }
            Err(e) => println!("kiosk-demo: could NOT open {path}: {e}"),
        }
    }
    open
}

/// Open one evdev node so that a read never blocks.
///
/// **`O_NONBLOCK` after the fact**, because `OpenOptions` has no portable way
/// to ask for it. One function rather than two copies: the startup scan and
/// the hotplug scan must open a device the same way, and the day they differ
/// is the day a plugged-in pad parks the loop forever.
fn open_nonblocking(path: &str) -> std::io::Result<std::fs::File> {
    let file = std::fs::File::open(path)?;
    // SAFETY: `file` owns the descriptor for the whole call and `fcntl` with
    // `F_SETFL` neither allocates nor closes it.
    unsafe {
        let fd = file.as_raw_fd();
        let flags = libc_fcntl(fd, 3 /* F_GETFL */, 0);
        if flags >= 0 {
            libc_fcntl(fd, 4 /* F_SETFL */, flags | 0o4000 /* O_NONBLOCK */);
        }
    }
    Ok(file)
}

// The one libc call this needs, declared rather than depended on: a whole crate
// for `fcntl` would be a dependency in an example whose subject is a
// permission, and `ADR-009` §5 asks that every one be justified.
unsafe extern "C" {
    #[link_name = "fcntl"]
    fn libc_fcntl(fd: i32, cmd: i32, arg: i32) -> i32;
}

/// This process's real uid, from `/proc` — so this example needs no crate at
/// all beyond the one extern above.
fn own_uid() -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    Some(line.split_whitespace().nth(1)?.to_string())
}

/// **The six things a launcher needs, from a keyboard OR a gamepad.**
///
/// `PLAN.md` §104.16. This is deliberately not a general input library: it is
/// the exact vocabulary the maintainer named for a kiosk launcher — up, down,
/// left, right, enter, back — and decoding precisely that turns a controller
/// test from *"the descriptor delivered 48 bytes"* into *"the D-pad works and
/// so does A"*.
///
/// **Both device kinds, because a launcher cannot care which is plugged in.**
/// A keyboard sends `EV_KEY` with `KEY_UP`; a pad sends `EV_ABS` with
/// `ABS_HAT0Y = -1`, or `EV_KEY` with `BTN_DPAD_UP` if it has real buttons.
/// All three arrive here as `Action::Up`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// A short press of the back control, decided **on release**.
    Back,
    /// The back control held past [`HOLD`] — a DIFFERENT event, not a longer
    /// one, and the whole reason `Back` waits for the release.
    BackHeld,
}

impl Action {
    const fn word(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Enter => "ENTER",
            Self::Back => "BACK",
            Self::BackHeld => "LONG BACK",
        }
    }
}

/// `linux/input-event-codes.h`, the handful this needs.
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

/// Decode one evdev event, or `None` if it is not one of the six.
///
/// **A press and not a release.** `value` is 1 for a press and 2 for autorepeat
/// on a key, 0 for release; a menu that moved on both would move twice per
/// button. The hat axes are the exception and are handled on their own terms:
/// they report a DIRECTION rather than a state, and returning to centre is
/// `0`, which is the release.
fn action_of(kind: u16, code: u16, value: i32) -> Option<Action> {
    match kind {
        EV_KEY if value == 1 || value == 2 => match code {
            // Keyboard.
            103 => Some(Action::Up),      // KEY_UP
            108 => Some(Action::Down),    // KEY_DOWN
            105 => Some(Action::Left),    // KEY_LEFT
            106 => Some(Action::Right),   // KEY_RIGHT
            28 | 96 => Some(Action::Enter), // KEY_ENTER, KEY_KPENTER
            // **BACK is deliberately absent**, and it is the only one that is.
            // A short press and a long one have to be DIFFERENT events, so
            // neither can be decided when the control goes down — see
            // `back_edge` and `HOLD`.
            // Gamepad buttons. BTN_SOUTH is `A` on an Xbox pad and the
            // bottom face button everywhere else, which is the one every
            // console UI uses to confirm.
            0x130 => Some(Action::Enter), // BTN_SOUTH / BTN_A
            // BTN_EAST / BTN_B is BACK and is handled on its edges, above.
            // Pads that expose the D-pad as buttons rather than as a hat.
            0x220 => Some(Action::Up),    // BTN_DPAD_UP
            0x221 => Some(Action::Down),  // BTN_DPAD_DOWN
            0x222 => Some(Action::Left),  // BTN_DPAD_LEFT
            0x223 => Some(Action::Right), // BTN_DPAD_RIGHT
            _ => None,
        },
        // The D-pad as a hat, which is what an Xbox pad sends through `xpad`.
        EV_ABS => match (code, value) {
            (0x11, v) if v < 0 => Some(Action::Up),    // ABS_HAT0Y
            (0x11, v) if v > 0 => Some(Action::Down),
            (0x10, v) if v < 0 => Some(Action::Left),  // ABS_HAT0X
            (0x10, v) if v > 0 => Some(Action::Right),
            _ => None,
        },
        _ => None,
    }
}

/// **Is this the BACK control, and is it going down or coming up?**
///
/// `action_of` deliberately ignores releases, because a menu that moved on both
/// edges would move twice per button. The hold-to-exit gesture needs both edges
/// and nothing else does, so it asks separately rather than widening that.
///
/// `PLAN.md` §104.18: the maintainer ruled a two-second hold of BACK as the way
/// out of a kiosk application, in place of a confirmation prompt Rivet cannot
/// draw over a running application without becoming a compositor.
///
/// **And a short press must not be a long one that has not finished yet.** The
/// maintainer named that before it could be got wrong: the test bed has to show
/// a press of the red button and a long press of it as two distinct events
/// (`.rulings/` R-334). So `Back` is decided on the RELEASE and only when the
/// hold was short; a long press emits [`Action::BackHeld`] at the two-second
/// mark and the release that follows emits nothing. One press, one event,
/// always.
/// How long the back control must be held to be a different event.
const HOLD: std::time::Duration = std::time::Duration::from_secs(2);

fn back_edge(kind: u16, code: u16, value: i32) -> Option<bool> {
    if kind != EV_KEY || !(code == 1 || code == 14 || code == 0x131) {
        return None;
    }
    match value {
        1 => Some(true),  // pressed
        0 => Some(false), // released
        _ => None,        // autorepeat says nothing new about the edge
    }
}

/// One evdev record is 24 bytes on a 64-bit kernel: a 16-byte `timeval`, then
/// `type`, `code` and `value`.
///
/// **Read from the buffer rather than transmuted.** A `#[repr(C)]` struct and a
/// pointer cast would be the same thing with an alignment assumption attached,
/// and this program's whole point is that it does the simple thing visibly.
fn events_in(buffer: &[u8]) -> impl Iterator<Item = (u16, u16, i32)> + '_ {
    buffer.as_chunks::<24>().0.iter().map(|e| {
        (
            u16::from_ne_bytes([e[16], e[17]]),
            u16::from_ne_bytes([e[18], e[19]]),
            i32::from_ne_bytes([e[20], e[21], e[22], e[23]]),
        )
    })
}

/// What the kernel calls this device — `/sys/class/input/<node>/device/name`.
///
/// **Named rather than numbered**, because `event3` says nothing and
/// `Microsoft X-Box 360 pad` says everything, and the first question of any
/// controller test is *which node is the controller*.
fn device_name(node: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/input/{node}/device/name"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unnamed".to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The launcher's whole vocabulary, from a keyboard.** These are the six
    /// the maintainer named, and the test exists so that a controller session
    /// on a board is a CONFIRMATION rather than the first time anybody found
    /// out what the mapping was.
    #[test]
    fn a_keyboard_produces_every_launcher_action() {
        let press = |code| action_of(EV_KEY, code, 1);
        assert_eq!(press(103), Some(Action::Up));
        assert_eq!(press(108), Some(Action::Down));
        assert_eq!(press(105), Some(Action::Left));
        assert_eq!(press(106), Some(Action::Right));
        assert_eq!(press(28), Some(Action::Enter));
    }

    /// **BACK is not in `action_of`, and that is the design.** A short press and
    /// a long one must be different events, so neither can be decided when the
    /// control goes down. `back_edge` sees both edges; the loop decides.
    #[test]
    fn the_back_control_produces_no_action_when_it_goes_down() {
        for code in [1u16, 14, 0x131] {
            assert_eq!(
                action_of(EV_KEY, code, 1),
                None,
                "{code:#x} decided BACK on the press"
            );
        }
        assert_eq!(back_edge(EV_KEY, 1, 1), Some(true), "keyboard ESC down");
        assert_eq!(back_edge(EV_KEY, 0x131, 1), Some(true), "pad B down");
        assert_eq!(back_edge(EV_KEY, 0x131, 0), Some(false), "pad B up");
        assert_eq!(back_edge(EV_KEY, 0x131, 2), None, "autorepeat is not an edge");
        assert_eq!(back_edge(EV_KEY, 103, 1), None, "UP is not the back control");
    }

    /// The two are named differently, because a person reading a screen across a
    /// room is the whole point of the distinction.
    #[test]
    fn a_short_and_a_long_back_are_different_words() {
        assert_ne!(Action::Back.word(), Action::BackHeld.word());
        assert_eq!(Action::BackHeld.word(), "LONG BACK");
    }

    /// **And from a gamepad, through both shapes a D-pad takes.** An Xbox pad
    /// on `xpad` sends the D-pad as a hat AXIS; other pads send real buttons.
    /// A launcher cannot care which is plugged in, so both arrive as the same
    /// `Action`.
    #[test]
    fn a_gamepad_produces_every_launcher_action_through_either_dpad_shape() {
        // The hat, which is what xpad sends.
        assert_eq!(action_of(EV_ABS, 0x11, -1), Some(Action::Up));
        assert_eq!(action_of(EV_ABS, 0x11, 1), Some(Action::Down));
        assert_eq!(action_of(EV_ABS, 0x10, -1), Some(Action::Left));
        assert_eq!(action_of(EV_ABS, 0x10, 1), Some(Action::Right));
        // Buttons, which is what pads with a real D-pad send.
        assert_eq!(action_of(EV_KEY, 0x220, 1), Some(Action::Up));
        assert_eq!(action_of(EV_KEY, 0x223, 1), Some(Action::Right));
        // The two face buttons every console UI uses.
        assert_eq!(action_of(EV_KEY, 0x130, 1), Some(Action::Enter)); // BTN_SOUTH / A
    }

    /// **A release is not a press**, or a menu moves twice for every button —
    /// once going down and once coming up. Autorepeat (`2`) IS a press, so a
    /// held direction keeps scrolling.
    #[test]
    fn a_key_release_produces_nothing_and_autorepeat_produces_a_press() {
        assert_eq!(action_of(EV_KEY, 103, 0), None, "release");
        assert_eq!(action_of(EV_KEY, 103, 2), Some(Action::Up), "autorepeat");
    }

    /// **A hat returning to centre is nothing**, which is the same rule stated
    /// on an axis: `0` is the release.
    #[test]
    fn a_hat_returning_to_centre_produces_nothing() {
        assert_eq!(action_of(EV_ABS, 0x10, 0), None);
        assert_eq!(action_of(EV_ABS, 0x11, 0), None);
    }

    /// The analogue sticks are deliberately NOT mapped. A launcher driven by a
    /// stick moves on noise, because a resting stick reports small non-zero
    /// values forever and this build has no deadzone. `ABS_X` is the stick.
    #[test]
    fn the_analogue_sticks_are_not_a_dpad() {
        assert_eq!(action_of(EV_ABS, 0x00, -32000), None, "ABS_X");
        assert_eq!(action_of(EV_ABS, 0x01, 32000), None, "ABS_Y");
    }

    /// One 24-byte record, read out of a buffer exactly as the kernel wrote it.
    #[test]
    fn an_evdev_record_is_read_out_of_its_twenty_four_bytes() {
        let mut buf = [0u8; 48];
        buf[16..18].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[18..20].copy_from_slice(&103u16.to_ne_bytes());
        buf[20..24].copy_from_slice(&1i32.to_ne_bytes());
        buf[40..42].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[42..44].copy_from_slice(&28u16.to_ne_bytes());
        buf[44..48].copy_from_slice(&1i32.to_ne_bytes());
        let got: Vec<Action> = events_in(&buf)
            .filter_map(|(k, c, v)| action_of(k, c, v))
            .collect();
        assert_eq!(got, vec![Action::Up, Action::Enter]);
    }

    /// A partial record at the end of a read is dropped rather than misread —
    /// `chunks_exact` is what guarantees it, and this says so.
    #[test]
    fn a_partial_trailing_record_is_not_decoded() {
        let buf = [0u8; 30];
        assert_eq!(events_in(&buf).count(), 1, "24 whole bytes, 6 left over");
    }

    /// **The card is opened by path only when nothing else can be holding it** —
    /// `ADR-036` §3.1, register `R-33`.
    ///
    /// The fallback used to be unconditional. Under a supervisor that is not a
    /// fallback at all: `ADR-015` §6.2's grant chowned `card*` to this
    /// application, so the open succeeded and this program took DRM master from
    /// the launcher — the example that exists to teach the lease, teaching the
    /// way around it instead.
    ///
    /// **The refusal must say what the grant actually covers** — asserted since
    /// 2026-09-04, because the sentence outlived the behaviour once already. It
    /// claimed the open *would succeed*, which stopped being true when the grant
    /// was narrowed to `renderD*` (`.rulings/` R-138), and a refusal that
    /// explains itself with a fact that is no longer true sends an operator to
    /// the wrong place.
    ///
    /// Both cases are in one test because the environment is process-wide and
    /// the harness runs tests in parallel; two of these would see each other's
    /// variable and fail intermittently.
    #[test]
    fn the_card_is_opened_by_path_only_when_no_supervisor_started_this() {
        // SAFETY: `set_var`/`remove_var` race with any other thread reading the
        // environment, and this is the only test in this binary that touches
        // it — which is why all of it is here rather than split in two.
        unsafe { std::env::remove_var(rivet_sdk::app::APP_DATA_ENV) };
        assert!(
            may_take_the_card_by_path().is_ok(),
            "run by hand on a board with no launcher, opening the card is correct"
        );

        unsafe { std::env::set_var(rivet_sdk::app::APP_DATA_ENV, "/state/app/kiosk-demo/data") };
        let why = may_take_the_card_by_path()
            .expect_err("a supervisor started this and granted no display");
        assert!(why.contains(CARD), "it names what it refused: {why}");
        assert!(
            why.contains(rivet_sdk::display::DISPLAY_ENV),
            "it names the variable that would have changed the answer: {why}"
        );
        assert!(why.contains("ADR-036"), "it cites the rule: {why}");
        assert!(
            why.contains("/dev/dri/renderD*"),
            "it says what a lessee's display grant actually covers, so the sentence cannot rot \
             back into claiming the card node is this application's: {why}"
        );

        unsafe { std::env::remove_var(rivet_sdk::app::APP_DATA_ENV) };
    }

    /// **A refusal is reported as a refusal, with the kernel's own number** —
    /// `.rulings/` R-138, register `R-32`.
    ///
    /// This is the arm that matters and the arm a host cannot exercise: the
    /// container this test runs in has no `/dev/dri`, so without a seam here
    /// the refusal path would first execute on the board, at the one moment
    /// its output is the whole finding.
    ///
    /// Break it by having the `Err` arm say `OPENED` and this fails on the
    /// first assertion — the two directions must not be able to read alike.
    #[test]
    fn a_refused_node_reports_the_errno_and_an_opened_one_does_not() {
        let refused = open_report(
            "/dev/dri/card0",
            "mode 0660 uid 0 gid 0",
            Err(std::io::Error::from_raw_os_error(13)),
        );
        assert!(refused.contains("REFUSED"), "{refused}");
        assert!(refused.contains("errno 13"), "the number, not only the sentence: {refused}");
        assert!(refused.contains("/dev/dri/card0"), "it names the node: {refused}");
        assert!(refused.contains("uid 0"), "it carries the ownership beside the refusal: {refused}");
        assert!(!refused.contains("OPENED"), "{refused}");

        let opened = open_report("/dev/dri/renderD128", "mode 0660 uid 65534 gid 65534", Ok(()));
        assert!(opened.contains("OPENED"), "{opened}");
        assert!(!opened.contains("REFUSED"), "{opened}");
        assert!(!opened.contains("errno"), "nothing failed, so there is no errno: {opened}");
    }

    /// An error with no OS number — which is what a Rust-level failure looks
    /// like — says so rather than printing a plausible zero.
    #[test]
    fn an_error_with_no_errno_says_none_rather_than_inventing_one() {
        let line = open_report(
            "/dev/dri/card0",
            "mode 0660 uid 0 gid 0",
            Err(std::io::Error::other("something that never touched the kernel")),
        );
        assert!(line.contains("errno none"), "{line}");
    }

    /// The ownership half degrades to a named absence rather than to a number
    /// that would read as a reading.
    #[test]
    fn ownership_of_a_path_that_is_not_there_is_reported_as_such() {
        let said = ownership_of("/dev/dri/there-is-no-such-node");
        assert!(said.contains("cannot be stat'd"), "{said}");
        assert!(!said.contains("uid"), "it must not look like a reading: {said}");
    }
}


// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

/// How many actions the screen shows at once.
const RECENT: usize = 8;

/// `/dev/input/event4` -> `event4`, which is what fits and what a person reads.
///
/// **Lower case, and that is not a style choice.** `crate::font` carries the
/// whole lower-case alphabet and exactly five capitals — `B`, `O`, `R`, `S`,
/// `V` — because those are what `Rivet OS` and `Version` need and it was
/// written for the splash. The first screen this program drew said
/// `OS -  O  8    V  S  R SS` where it meant `KIOSK-DEMO * 8 INPUT DEVICES`,
/// which is a font doing exactly what it has and nothing more.
///
/// Lower-casing costs nothing and adding twenty-one glyphs by hand to spell one
/// heading would be the wrong repair. `PLAN.md` §104.18.5.
fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}
// **The framebuffer geometry probe is gone**, and its absence is the point.
//
// It read `/sys/class/graphics/fb0/{virtual_size,bits_per_pixel,stride}` and
// reconstructed a pixel format from the driver's bitfields, because `/dev/fb0`
// has no ioctl-free way to ask. KMS answers every one of those questions
// directly and authoritatively: the connector's preferred mode is the size, the
// kernel returns the stride when it allocates the buffer, and the format is
// whatever was asked for in `drmModeAddFB`. See `crate::drm`.
//
// It is deleted rather than kept behind a fallback. A program that could draw
// two ways would keep the failure mode this rewrite exists to remove -- writing
// to a framebuffer the console also owns -- available by accident.


/// The two faces this screen draws with, loaded once.
///
/// **Once and not per frame.** `Face` keeps its rasterised glyphs (`SPEC-04`
/// §10.3), and a face rebuilt every tick would throw that cache away every tick
/// — which is the exact cost the cache was added to remove.
///
/// **Either may be absent and the program still runs.** `Face::load` fails with
/// a sentence rather than a fallback, and this example's whole posture is that
/// a failure is a report: an image built without a typeface should show a
/// person a live screen and tell the journal why it has no words on it, not
/// die at the panel with nothing on the glass.
struct Faces {
    sans: Option<rivet_sdk::text::Face>,
    mono: Option<rivet_sdk::text::Face>,
}

impl Faces {
    /// Load both, saying on stdout what happened to each.
    fn load() -> Self {
        let one = |path: &str, what: &str| match rivet_sdk::text::Face::load(path) {
            Ok(face) => {
                println!("kiosk-demo: {what} from {path}");
                Some(face)
            }
            Err(why) => {
                println!("kiosk-demo: NO {what} — {why}");
                None
            }
        };
        Self {
            sans: one(rivet_sdk::text::SYSTEM_FONT, "type"),
            mono: one(rivet_sdk::text::SYSTEM_MONO, "monospace"),
        }
    }
}

/// Rivet's own colour, `docs/brand/brand.json` — *"weathered oxide"*.
///
/// On the screen rather than in a stylesheet because this is the first thing a
/// person sees on a board that has taken the display, and a test card that
/// looks like the platform is worth the three bytes.
const ACCENT: [u8; 3] = [0xb0, 0x51, 0x2f];
const INK: [u8; 3] = [0xe6, 0xed, 0xf3];
const MUTED: [u8; 3] = [0x8b, 0x94, 0x9e];
const FAINT: [u8; 3] = [0x56, 0x5f, 0x6a];
const GROUND: [u8; 3] = [0x0d, 0x11, 0x17];
const PANEL: [u8; 3] = [0x16, 0x1b, 0x22];

/// Paint the whole screen: a heading, a status block, and the recent actions.
///
/// **The whole screen every tick, from one buffer.** A partial update would
/// leave the previous frame's longer line showing under a shorter one, which on
/// a test screen reads as an event that did not happen.
///
/// # Everything is scaled from the panel's own height
///
/// A kiosk application is handed whatever mode the connector prefers, and this
/// one has been run on more than one. Sizes are a fraction of 1080 rather than
/// pixel constants, so the same code fills a 720p panel and a 4K one instead of
/// drawing a postage stamp in the corner of the second.
fn draw_screen(
    screen: &mut drm::Screen,
    faces: &Faces,
    recent: &[(String, Action)],
    tick: u64,
    events: u64,
    devices: usize,
    display: &str,
) {
    use rivet_sdk::draw;
    use rivet_sdk::text::Target;

    // **A dumb buffer is XRGB8888**, which is what `drmModeAddFB` was told
    // (depth 24 in 32 bits) and what `drm::Screen::put` writes. That is also
    // the one layout `rivet_sdk::draw` and `rivet_sdk::text` write — they say
    // so and they carry no channel-offset indirection — so the two agree by
    // construction rather than by a constant this file has to keep in step —
    // and since 2026-09-20 that is enforced rather than merely asserted:
    // `Target::xrgb` names the format and derives the stride arithmetic from
    // it, so there is no longer a `BPP` here that could drift.

    let (width, height, stride) = (screen.width, screen.height, screen.stride);
    let mut canvas = vec![0u8; stride * height];
    let t = Target::xrgb(width, height, stride);

    // One number, and every size below is a multiple of it.
    let s = |v: f32| (v * height as f32 / 1080.0).round();
    let px = |v: f32| s(v) as usize;

    draw::fill(&mut canvas, &t, 0, 0, width, height, GROUND);
    draw::fill(&mut canvas, &t, 0, 0, width, px(8.0), ACCENT);

    // **The heartbeat is a dot and not a character**, so it reads at across-the-
    // room distance and does not depend on a face having loaded. A frozen
    // screen and a quiet one look different from the doorway.
    let beat = if tick % 2 == 0 { ACCENT } else { PANEL };
    draw::circle(&mut canvas, &t, s(76.0), s(120.0), s(13.0), beat);

    if let Some(sans) = faces.sans.as_ref() {
        sans.draw(&mut canvas, &t, px(112.0), px(136.0), "kiosk-demo", s(60.0), INK);
        sans.draw(
            &mut canvas,
            &t,
            px(112.0),
            px(186.0),
            "the display grant, proven by drawing this",
            s(26.0),
            MUTED,
        );
    }

    // The status block, right-aligned, in the monospace — every one of these
    // three is a number that changes in place, which is the whole reason the
    // image carries a second face (`rivet_core::etc::SYSTEM_MONO`).
    if let Some(mono) = faces.mono.as_ref() {
        let size = s(26.0);
        let lines = [
            format!("tick     {tick:>6}"),
            format!("events   {events:>6}"),
            format!("devices  {devices:>6}"),
            format!("display  {display:>6}"),
        ];
        let widest = lines.iter().map(|l| mono.width_of(l, size)).max().unwrap_or(0);
        let right = width.saturating_sub(px(64.0));
        let left = right.saturating_sub(widest);
        for (i, line) in lines.iter().enumerate() {
            mono.draw(&mut canvas, &t, left, px(120.0) + i * px(38.0), line, size, MUTED);
        }
    }

    // A hairline, then the list. The rule is what separates the heading from
    // the thing the tester is actually watching.
    let top = px(268.0);
    draw::fill(&mut canvas, &t, px(112.0), top, width.saturating_sub(px(224.0)), px(2.0), PANEL);

    if recent.is_empty() {
        if let Some(sans) = faces.sans.as_ref() {
            sans.draw(
                &mut canvas,
                &t,
                px(112.0),
                top + px(80.0),
                "no input seen yet — press anything",
                s(32.0),
                FAINT,
            );
        }
        screen.blit(&canvas);
        return;
    }

    // Newest at the top and brightest, so the eye lands on what just happened.
    // The dimming is applied as ALPHA and not by scaling the colour: over a
    // panel that is not black, a scaled colour fades toward black instead of
    // toward what is behind it, which `Face::draw_alpha` says at length.
    let row_h = px(76.0);
    let gap = px(12.0);
    for (i, (device, action)) in recent.iter().enumerate() {
        let y = top + px(36.0) + i * (row_h + gap);
        if y + row_h >= height {
            break;
        }
        let alpha = 0xff_u32.saturating_sub(i as u32 * 34) as u8;
        draw::fill_rounded(
            &mut canvas,
            &t,
            px(112.0),
            y,
            width.saturating_sub(px(224.0)),
            row_h,
            px(10.0),
            PANEL,
        );
        // The device on the left in the monospace, so a column of names lines
        // up and a changed one is visible without reading it.
        if let Some(mono) = faces.mono.as_ref() {
            mono.draw_alpha(
                &mut canvas,
                &t,
                px(144.0),
                y + px(50.0),
                device,
                s(26.0),
                MUTED,
                alpha,
            );
        }
        // **The capitals are the journal's own**, since 2026-09-04.
        //
        // ~~The journal keeps the capitals; the screen cannot render them.~~
        // That was true of the five-by-seven bitmap this crate used to carry,
        // which had no upper case at all — so the screen said `long back` while
        // the journal said `LONG BACK`, and two people comparing them were
        // comparing two spellings of one event. `Action::word` is now drawn
        // verbatim.
        if let Some(sans) = faces.sans.as_ref() {
            sans.draw_alpha(
                &mut canvas,
                &t,
                px(560.0),
                y + px(52.0),
                action.word(),
                s(34.0),
                INK,
                alpha,
            );
        }
    }

    // One copy into the scanout buffer. No page flip: `crate::drm` carries why
    // a demo does not need one, and tearing on a static screen is invisible.
    screen.blit(&canvas);
}
