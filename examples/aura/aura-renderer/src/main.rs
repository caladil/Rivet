//! **Aura's renderer** — a Rivet kiosk application. `SPEC-04`.
//!
//! It is handed a display lease, is handed four listening sockets, and turns
//! whatever arrives on them into a picture. It opens no device node, binds no
//! port, fetches nothing and knows nothing about weather, river gauges or buses.
//!
//! # The shape, in five sentences
//!
//! 1. A producer connects to the socket for the slot it was granted and writes
//!    [`aura_card::Message`]s. `crate::ingest` is the only place cards arrive.
//! 2. `crate::store` holds at most one card per slot and drops each when its
//!    own stated `ttl` runs out.
//! 3. The renderer synthesises the clock card itself (`crate::clock`) and hands
//!    it to the same store.
//! 4. `crate::background` paints the whole screen first — **the screensaver IS
//!    the background** — and `crate::widgets` composites the cards over it.
//! 5. `crate::typeface` is what makes any of that the same size as Aura.
//!
//! # The frame budget, and why there is one
//!
//! The loop sleeps whatever is left of [`FRAME_BUDGET`] after the work, rather
//! than sleeping a fixed amount **after** it. The difference matters:
//! `examples/templates/template-kiosk` and `cmd/rivet-launcher` both sleep a
//! constant, so their real interval is `work + constant` and silently stretches
//! as the drawing gets heavier. A budget holds the rate steady and makes an
//! overrun visible as an overrun.
//!
//! `rivet_sdk::screen::Screen::blit` waits for the page flip it asked for, so on
//! a double-buffered display the loop is paced by the panel and the sleep is
//! usually zero. **Whether it is double-buffered is now printed**, because
//! until 2026-09-02 nothing said, and a screen that had silently degraded to one
//! page — torn, and torn permanently — looked exactly like one that had not.
//!
//! # An application on a Rivet has no configuration surface
//!
//! There is no `env` key in a manifest, no argv, and nothing an operator can
//! write into `RIVET_APP_DATA` over the control plane — `rivet` has `get file`
//! and no counterpart (`SPEC-03` §3, `ADR-006` §8.10.5). So *configurable*
//! here means **one named constant in one file**, and every setting Aura's Go
//! controller exposes through an HTTP portal is in the same position — the
//! background, the brightness, the twelve-hour clock, the temperature unit. That
//! is a real gap and it is reported rather than worked around; `SPEC-04` §11
//! carries it as an open question for the maintainer.

mod clock;
mod display;
mod icons;
mod controller;
mod ingest;
mod layout;
// **The background pack, hosted** — what `mod background` used to be.
//
// This program carried its own copy of every background until 2026-09-20.
// `examples/aura/aura-background-pack` is the authority now and this module is
// the host side of that; `mod evidence` went with the drawing, because the
// crate that draws is the crate whose frames are worth hashing.
mod pack;
mod rng;
mod store;
mod theme;
mod typeface;
mod unmarked;
mod widgets;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aura_card::{Block, Message, Slot};

use widgets::Corner;

/// **How long one frame may take, and how often one is drawn.**
///
/// 33.3 ms — thirty frames a second, which is what Aura's C++ renderer asks
/// raylib for (`SetTargetFPS(30)`) and has run at on a wall for months. It is
/// the right default for three reasons and none of them is graphics: a clock
/// changes once a minute, a forecast every twenty, and the animated backgrounds
/// tick at one to fifteen cells a second. Nothing here is helped by sixty and
/// nothing is helped by a hundred and eighty.
const FRAME_BUDGET: Duration = Duration::from_micros(33_333);

/// **How often a refused frame may reach the journal**, in frames.
///
/// A pack that refuses every frame refuses thirty a second. The journal is a
/// rolling ~1 MB (`ADR-022` §8), so logging each one would bury the record
/// that says *why* under the record that says *again* — the trap
/// `rivet-your-own-queries-flood-the-journal` names one level up, arriving
/// here by a different road. 900 frames is thirty seconds.
const REFUSAL_QUIET_FRAMES: u32 = 900;

/// **How often the renderer reports what a frame costs it** — `SPEC-04` §10.
///
/// The only measurement that decides anything is one taken from the program
/// that is actually drawing, at the sizes it actually draws, with the typefaces
/// it actually ships. `examples/aura/aura-probe` measures the primitives and a
/// synthetic approximation of the two animated backgrounds; this measures the
/// port.
///
/// **Nine thousand frames is five minutes at thirty a second**, stepped up from
/// six hundred (twenty seconds) on 2026-09-03 at the maintainer's ruling: the
/// journal is a rolling ~1 MB (`ADR-022` §8), and at twenty seconds this one
/// line filled 840 KB of it in under nine hours. Five minutes keeps the journal
/// manageable during development and buys about **five and a half days** of
/// history in the same space.
///
/// **What the coarser window costs, stated rather than assumed.** The 8h45m
/// soak of 2026-09-02/03 is the evidence it is affordable: 1558 samples held
/// DRAWING inside 19.16-22.28 ms with a mean of 21.56, so the signal is a flat
/// line with about 1 ms of jitter on it. Averaging fifteen of those windows
/// together loses nothing that was being read — the thing being watched is
/// drift over hours, and that survives.
///
/// **What it does lose is a step between two consecutive lines.** A Pi 4
/// dropping to its throttled clock used to show up inside twenty seconds; now
/// a five-minute window can average a short excursion away. That is an
/// acceptable trade because the throttle is not measured here: `get thermal`'s
/// `throttle.*_since_boot` flags answer it for the whole run and cannot be
/// averaged away at all (`§119.5`), which is the field that should be read for
/// that question anyway.
const REPORT_EVERY_FRAMES: u32 = 9000;

/// How often the application takes its own picture.
///
/// **It has to be the application**, because while it holds the display lease
/// the launcher cannot see these pixels and correctly refuses to capture
/// (`Screen::capture`'s own note). Sixty seconds is a compromise: a QOI encode
/// of eight megabytes is not free and it is paid inside one frame, so a shorter
/// interval would be visible as a periodic stutter on a wall panel.
///
/// **A capture proves what was drawn. It does not prove the absence of
/// tearing**, which is a property of when the buffer reaches the glass and only
/// a person in front of the screen can judge. The journal line about page flips
/// is the evidence for that, and it is a different piece of evidence.
const CAPTURE_EVERY: Duration = Duration::from_secs(60);

/// How bright, out of a hundred. Aura's `brightness` command, which reaches
/// every colour on a card and not only the ink.
///
/// **60, which is Aura's own default** (`internal/config/config.go`), not 100.
/// It is a real part of the appearance: the grey the widgets draw with is
/// `(unsigned char)(0.60 * 255)` = 153, and every themed colour is multiplied
/// by `153/255` on its way to the screen.
const BRIGHTNESS_PERCENT: u8 = 60;

/// The typefaces, shipped as package assets — `ADR-015` §3's *"optional
/// assets"*, unpacked flat beside the program and found through
/// `RIVET_APP_DIR`.
///
/// **Not the system font.** `rivet_sdk::text::SYSTEM_FONT` is Geist, which is
/// the right answer for a Rivet screen and the wrong answer for Aura: these are
/// the faces Aura's numbers were set in and the sizes in `widgets.hpp` were
/// chosen against. Shipping them in the `.rvt` rather than depending on the
/// image is what `ADR-015` §3 is for.
const BOLD_FONT: &str = "SourceCodePro-Bold.ttf";
const REGULAR_FONT: &str = "SourceCodePro-Regular.ttf";
// **`AuraRain.ttf` is not here any more**, and neither is the IPA licence that
// has to travel with it. Digital Rain's face went to
// `examples/aura/aura-background-pack/assets/` with Digital Rain on
// 2026-09-20, and the pack embeds it in its own object rather than reading it
// from a path — a shared library cannot assume a `RIVET_APP_DIR`, because the
// directory belongs to whichever host loaded it.
//
// Carrying a copy here would be 19 KB of font this program never rasterises,
// and — more to the point — an IPA-licensed file in an artifact that no longer
// uses it, which is the kind of thing that is a puzzle to somebody in a year.
// `assets/README.md` records where it went.

// **The icon face is named by `crate::icons::FONT`** and not by a constant
// here, unlike the three faces above.
//
// Those three are this file's business: it decides which typeface the clock is
// set in. The marks are `crate::icons`' business entirely — that module owns
// the codepoints, the atlas size and the missing-asset rule — so the filename
// lives with them. A second spelling here is a second thing to keep in step
// (`ADR-011`), and a renderer that loaded the wrong file would draw the wrong
// glyphs, silently.

fn main() {
    let journal = rivet_sdk::journal::Journal::for_service("aura-renderer");

    // **A SURFACE, not a lease** — `ADR-058`, and this is the whole of what
    // changed in this program.
    //
    // It used to be `screen::handed()`: a channel on `RIVET_DISPLAY` carrying
    // DRM leases, a `Screen` built by modesetting the card it was handed, and a
    // re-acquire in the frame loop because a revoked lease cannot be resumed.
    // All of it is gone. The compositor owns the device, this asks for a
    // surface, and the surface does not go away — so there is nothing to
    // re-acquire and no revocation to survive.
    //
    // `Screen::what`'s line is kept, and `is_double_buffered`'s is not: the
    // second asked whether a leased page flip had been accepted or whether this
    // program had quietly fallen back to tearing. A Wayland client cannot tear
    // — the compositor presents whole frames — so a warning about it would be
    // a claim about a failure that no longer has a mechanism.
    // **A GPU surface where there is one** — `.rulings/` R-206, and
    // `src/display.rs` argues why the choice is made once rather than per
    // background. A machine with no render node falls back to `wl_shm` and
    // draws every background on the CPU, which is what this program did
    // before 2026-09-21 and what R-206 D3 exists to preserve.
    let (mut display, fell_back) = match display::Display::take() {
        Ok(taken) => taken,
        Err(why) => {
            // An ordinary state, not a crash: on a machine whose compositor is
            // not running there is no surface, and saying so beats exiting
            // silently. `restart = "always"` brings this back.
            journal.error(&format!("no surface was handed over: {why}"));
            println!("aura-renderer: no surface was handed over: {why}");
            return;
        }
    };
    journal.info(&format!(
        "drawing at {}x{}",
        display.width(),
        display.height()
    ));
    journal.info(&format!("the screen says: {}", display.what()));
    // **Why the GPU was not used, said once, at the top.** A board that draws
    // every background on the CPU because its render node was unreachable
    // looks exactly like one that has no GPU backgrounds selected, and the
    // difference is a line in the journal or an afternoon.
    if let Some(why) = fell_back {
        journal.warn(&format!(
            "no GPU surface, so every background draws on the CPU: {why}"
        ));
    }

    let Some(assets) = rivet_sdk::app::dir().ok() else {
        journal.error(
            "RIVET_APP_DIR is not set, so this process was not started by a supervisor and \
             cannot find the typefaces it ships",
        );
        return;
    };
    let face = |name: &str, base: f32| {
        typeface::Typeface::load(&assets.join(name).to_string_lossy(), base)
    };
    // **No fallback.** A renderer whose whole job is legible type has nothing to
    // say without its typefaces, and a substitute face would look like a
    // rendering bug on a machine whose actual problem is a missing file — which
    // is precisely the defect this replaces.
    // **`AuraRain.ttf` is no longer loaded here.** Digital Rain's glyphs went
    // to the pack with Digital Rain, and the pack rasterises them from bytes
    // embedded in its own object rather than from a path — `aura-background-
    // pack/src/embedded_face.rs` says why a shared library must. A face loaded
    // here would be a face nothing draws with.
    let (bold, regular) = match (
        face(BOLD_FONT, widgets::CLOCK_FONT_SIZE),
        face(REGULAR_FONT, widgets::REGULAR_FONT_SIZE),
    ) {
        (Ok(bold), Ok(regular)) => (bold, regular),
        (bold, regular) => {
            for why in [bold.err(), regular.err()].into_iter().flatten() {
                journal.error(&format!("a typeface this package ships is unusable: {why}"));
            }
            return;
        }
    };
    journal.info(&format!(
        "typefaces loaded from {}: {BOLD_FONT} at {}, {REGULAR_FONT} at {}",
        assets.display(),
        widgets::CLOCK_FONT_SIZE,
        widgets::REGULAR_FONT_SIZE
    ));

    // **The icon face, and its absence is survivable** — see `ICON_FONT`. Every
    // reading still draws; only its mark is missing, which `SPEC-04` §7 rule 5
    // settles as the lesser of the two failures.
    let icon_face = face(icons::FONT, icons::BASE_SIZE).ok();
    match &icon_face {
        Some(_) => journal.info(&format!(
            "{} loaded at {}: trend arrows and connection marks will draw",
            icons::FONT,
            icons::BASE_SIZE
        )),
        None => journal.warn(&format!(
            "{} did not load, so trend arrows and connection marks are OMITTED and \
             not substituted. Readings still draw. SPEC-04 §7 rule 5: a mark that is absent is \
             diagnosable, a mark that is wrong is not",
            icons::FONT
        )),
    }

    let mut ingest = match ingest::Ingest::take() {
        Ok(ingest) => ingest,
        Err(why) => {
            journal.error(&format!("the handed sockets are unusable: {why}"));
            return;
        }
    };
    let granted: Vec<&str> = ingest.slots().map(Slot::as_str).collect();
    journal.info(&format!("slot sockets handed over: {}", granted.join(", ")));

    // **There is no `Target` for the background any more, and its absence is
    // the finding.** This program built one over `display.stride()` and used it
    // for nothing but the widgets — the background is drawn by the pack
    // through `rivetos_addon::frame`, which takes its own descriptor. Moving
    // the widgets to their own surface left the old one with no callers at
    // all, which is the compiler stating what the separation actually was.
    //
    // **The widget layer's own target, and it is `ARGB` where the background
    // is `XRGB`.** The background is opaque and its fourth byte is padding;
    // this one is drawn over it, so its fourth byte is alpha and every
    // primitive has to maintain it. `rivet_sdk::text::Format` is where that
    // distinction is argued, and building this with `::xrgb` would produce a
    // layer that is transparent everywhere — indistinguishable on a screen
    // from widgets that had stopped being drawn.
    //
    // **Its stride is `width * 4` and not `display.stride()`**: the overlay
    // buffer is this program's to shape and the SDK allocates it tight, where
    // the background's stride is the compositor's.
    let overlay_target =
        rivet_sdk::text::Target::argb(display.width(), display.height(), display.width() * 4);
    if let Err(why) = display.take_overlay() {
        // **Not fatal.** A compositor with no `wl_subcompositor` still shows
        // the background, and a kiosk with a background and no clock is worth
        // more than one that exited. Every frame's widget draw then reports
        // the same refusal at `REFUSAL_QUIET_FRAMES`.
        journal.error(&format!(
            "no widget layer: {why} — the background will draw and the cards will not"
        ));
    }
    let mut canvas = vec![0u8; display.stride() * display.height()];
    let mut store = store::Store::default();

    // **These were `const` and are now state**, because the controller sets
    // them: `brightness` and `set_screensaver` are directives it sends on
    // connect and on every change the portal saves. The constants remain as the
    // values a renderer starts with when no controller ever speaks to it.
    let mut brightness = BRIGHTNESS_PERCENT;
    let mut show_weather = true;
    // `display_sleep` blanks the overlay widgets and leaves the background
    // drawing. Distinct from the machine switching the SCREEN off, which is
    // `ADR-039`'s display hours and not this program's to do.
    let mut show_widgets = true;

    // **The pack, opened once.** A machine with none installed draws black and
    // says so — `ADR-053` joins by class and is silent when nothing matches,
    // and black is Aura's own default picture, so the failure looks like the
    // default rather than like a fault.
    let pack = match pack::Pack::open() {
        Ok(pack) => {
            let catalogue = pack.described();
            journal.info(&format!(
                "backgrounds from {} v{}: {}",
                pack.soname(),
                catalogue.version,
                catalogue
                    .backgrounds
                    .iter()
                    .map(|b| format!("{} ({})", b.name, b.themes.len()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            Some(pack)
        }
        Err(e @ pack::NotOpened::Absent(_)) => {
            journal.info(&format!(
                "{e} — no background pack is installed, so the screen stays black behind the \
                 cards. That is an ordinary machine, not a fault"
            ));
            None
        }
        Err(e) => {
            journal.error(&format!("{e}"));
            None
        }
    };

    // **What is selected**, as the two ids the pack speaks. Owned, because a
    // controller directive names a background this binary has never heard of
    // and there is nothing to intern it against.
    let mut background_id = pack::DEFAULT.background.to_string();
    let mut theme_id = pack::DEFAULT.theme.to_string();
    // **If the pack does not have the default, take its first pair.** A second
    // pack with an entirely different catalogue is the case `ADR-053` was
    // written for, and a renderer that insisted on `digital-rain` would draw
    // nothing on one.
    if let Some(p) = &pack
        && !p.has(&background_id, &theme_id)
        && let Some(first) = p.described().backgrounds.first()
        && let Some(theme) = first.themes.first()
    {
        journal.info(&format!(
            "this pack has no `{background_id}/{theme_id}`, so `{}/{}` is what starts",
            first.id, theme.id
        ));
        background_id = first.id.clone();
        theme_id = theme.id.clone();
    }

    let mut appearance = pack
        .as_ref()
        .and_then(|p| p.appearance(&background_id, &theme_id))
        .unwrap_or(pack::Appearance {
            card: None,
            animated: false,
            // No pack, or a pair it does not have: the CPU path, which is
            // the one that cannot fail for want of a driver.
            on_gpu: false,
        });
    let mut card_theme = appearance.card;
    // How many consecutive frames the pack has refused — see the draw call.
    let mut refusals: u32 = 0;
    // What has been said about `secondary`'s unmarked reading — see the call.
    let mut unmarked_said = unmarked::Watch::default();
    let mut pad = widgets::widget_pad(card_theme.as_ref());
    let mut grey = widgets::widget_grey(BRIGHTNESS_PERCENT);
    journal.info(&format!(
        "background `{}`, {} at {BRIGHTNESS_PERCENT}% with {pad}px padding",
        pack.as_ref()
            .and_then(|p| p.label(&background_id, &theme_id))
            .unwrap_or_else(|| format!("{background_id}/{theme_id}")),
        if card_theme.is_some() { "on a card" } else { "with no card" },
    ));

    // The controller, read directly — `cpp/src/main.cpp`'s arrangement.
    let mut controller = controller::Controller::take(std::time::Instant::now());
    match controller.as_ref() {
        Some(c) => journal.info(&format!(
            "reading the controller at {} for state and directives",
            c.path().display()
        )),
        None => journal.info(
            "aura-controller:kiosk.sock was not granted, so this renderer draws its own clock and \
             whatever addons send it. That is an ordinary machine, not a fault",
        ),
    }
    if card_theme.as_ref().is_some_and(|theme| theme.rainbow) {
        journal.info(
            "this background's card theme is the six-stop gradient `DrawRainbowCard`, and it \
             DRAWS as of 2026-09-02 (SPEC-04 §11 (14)). It was skipped entirely while \
             rivet_sdk::draw had no gradient fill, and skipping was the right refusal: a flat \
             fill in its place would have been a different picture presented as this one",
        );
    }

    // **The machine's display hours** - `ADR-039` §8.3, read once.
    //
    // Read here rather than per frame because `rivet_sdk::schedule::hours`
    // opens a file, and thirty of those a second on a Pi that is also
    // compositing is a cost for an answer that changes twice a day. The window
    // is the DOCUMENT's, published to `/run/rivet/display-hours`, so a change
    // arrives with a restart - which is what the schema publishes
    // (`takes_effect: on-next-restart`) rather than a shortcut taken here.
    let display_hours = rivet_sdk::schedule::hours();
    match display_hours {
        Some(hours) => journal.info(&format!(
            "the machine says the screen is dark from {} to {} local; outside those hours \
             this renderer blanks and stops drawing (ADR-039)",
            hours.off(),
            hours.on()
        )),
        None => journal.info(
            "the machine sets no display hours, so this screen is never switched off - the \
             `[display]` table of the configuration document is what would change that",
        ),
    }
    // **Blanking is all this does.** `ADR-039` §8.2: a dark panel commonly
    // keeps its backlight lit, and switching the TELEVISION off is HDMI-CEC,
    // which is the machine's - `rivet-adapter-cec`, built 2026-09-02, and
    // reaching a board only with a system image. Said once, at start, so a
    // building that is still drawing power at midnight has an explanation in
    // the journal rather than a mystery.
    if display_hours.is_some() {
        journal.warn(
            "these hours BLANK THE PICTURE only, and blanking is this process's whole part in them. \
             It stops drawing and paints black; it does not put the panel into DPMS standby. \
             Switching a TELEVISION off is HDMI-CEC, which is the MACHINE's to send and not an \
             application's (ADR-039 §8.2) - so whether a screen with its own backlight stops \
             drawing power depends on this machine's image and its [display] table, not on \
             anything here",
        );
    }

    let mut rng = rng::Mt19937::from_the_clock();
    let mut clock_corner = Corner::random(&mut rng);
    let mut next_corner_change = Instant::now() + Corner::interval(&mut rng);

    // Whether the screen is inside its lit hours. `true` until proven
    // otherwise, which is `rivet_sdk::schedule`'s own direction: a screen lit
    // when it should not be is visible, and a building dark at nine in the
    // morning is a phone call.
    let mut screen_lit = true;
    let mut last_minute = u64::MAX;
    // The C++'s `bool needs_redraw = true; // force first frame`.
    let mut needs_redraw = true;
    let mut cost = FrameCost::default();
    let started = Instant::now();
    let mut next_capture = started + CAPTURE_EVERY;

    loop {
        let frame_began = Instant::now();

        // **The re-acquire is gone, and so is the question it answered** —
        // `ADR-058`.
        //
        // This block used to ask the display channel once a frame whether the
        // launcher had handed the screen back, because a revoked lease cannot
        // be resumed: the descriptor died with the revocation and the way back
        // was a NEW one. A surface is not revoked. It stays mapped while this
        // process lives, goes behind whatever the launcher starts next, and
        // comes back when that exits — so there is nothing to ask and nothing
        // to rebuild.
        //
        // **What replaced it is one check.** The compositor going away is the
        // only remaining reason this program cannot draw, and exiting is
        // correct: `restart = "always"` brings it back when there is a screen
        // for it again.
        // That includes a compositor that restarted: its socket is gone, the
        // SDK closes the surface on the first failed hand-over, and this exit
        // is what lets the supervisor start a renderer on the new one.
        if display.closed() {
            journal.warn(
                "the compositor closed this surface or went away; exiting so the supervisor \
                 starts this renderer again on a new connection",
            );
            println!("aura-renderer: the compositor closed this surface; exiting");
            return;
        }

        // **`dt` is no longer computed here**, and the clamp that went with it
        // — `clamp(now - last_frame, 0.001, 0.1)`, main.cpp's, which stops a
        // stall from teleporting every column down the screen — moved into the
        // pack with the drawing it protects. `aura-background-pack/src/lib.rs`
        // applies the identical clamp to the difference between consecutive
        // `time_ms` values, so the behaviour is unchanged and it is now stated
        // once, in the crate that animates.

        if frame_began >= next_corner_change {
            clock_corner = clock_corner.moved(&mut rng);
            next_corner_change = frame_began + Corner::interval(&mut rng);
            needs_redraw = true;
            journal.debug("the clock moved to another corner");
        }

        for note in ingest.poll(&mut store, frame_began) {
            journal.info(&note);
        }

        // **The controller, polled the same way and on the same frame.** Its
        // state becomes cards in the same store an addon writes to, so nothing
        // downstream can tell where a card came from — which is `SPEC-04` §4.2's
        // rule and the reason the weather panel needs no special case.
        if let Some(c) = controller.as_mut() {
            let (notes, commands) = c.poll(&mut store, frame_began);
            for note in notes {
                journal.info(&note);
            }
            for command in commands {
                match command {
                    controller::Command::Brightness(percent) => {
                        if percent != brightness {
                            brightness = percent;
                            grey = widgets::widget_grey(brightness);
                            needs_redraw = true;
                            journal.info(&format!("the controller set brightness to {percent}%"));
                        }
                    }
                    controller::Command::Screensaver { background, theme } => {
                        if background != background_id || theme != theme_id {
                            // **Refused before it is adopted, not after.** A
                            // selection the pack does not have would otherwise
                            // become the current one and refuse every frame
                            // from here on — a black screen, thirty times a
                            // second, from one bad directive. Keeping the old
                            // pair leaves a picture on the wall and one
                            // sentence in the journal.
                            match pack.as_ref() {
                                Some(p) if !p.has(&background, &theme) => {
                                    journal.warn(&format!(
                                        "the controller asked for `{background}/{theme}` and \
                                         {} does not have it, so the screen keeps \
                                         `{background_id}/{theme_id}`",
                                        p.soname()
                                    ));
                                }
                                Some(p) => {
                                    background_id = background;
                                    theme_id = theme;
                                    appearance = p
                                        .appearance(&background_id, &theme_id)
                                        .unwrap_or(pack::Appearance {
                                            card: None,
                                            animated: false,
                                            // No pack, or a pair it does not have: the CPU path, which is
                                            // the one that cannot fail for want of a driver.
                                            on_gpu: false,
                                        });
                                    card_theme = appearance.card;
                                    pad = widgets::widget_pad(card_theme.as_ref());
                                    needs_redraw = true;
                                    journal.info(&format!(
                                        "the controller set the background to `{}`",
                                        p.label(&background_id, &theme_id).unwrap_or_else(
                                            || format!("{background_id}/{theme_id}")
                                        )
                                    ));
                                }
                                None => {
                                    journal.warn(&format!(
                                        "the controller asked for `{background}/{theme}` and no \
                                         background pack is loaded, so nothing changes"
                                    ));
                                }
                            }
                        }
                    }
                    controller::Command::ShowWeather(on) => {
                        if on != show_weather {
                            show_weather = on;
                            needs_redraw = true;
                            journal.info(if on {
                                "the controller turned the weather panel on"
                            } else {
                                "the controller turned the weather panel off"
                            });
                        }
                    }
                    controller::Command::RotateCorner(named) => {
                        // The schema: *"omitted picks a random different
                        // corner"* — which is exactly the rotation already
                        // running, so an omitted corner takes the same path the
                        // burn-in timer takes.
                        clock_corner = named.unwrap_or_else(|| clock_corner.moved(&mut rng));
                        next_corner_change = frame_began + Corner::interval(&mut rng);
                        needs_redraw = true;
                        journal.info("the controller moved the clock to another corner");
                    }
                    controller::Command::ShowWidgets(on) => {
                        if on != show_widgets {
                            show_widgets = on;
                            needs_redraw = true;
                            journal.info(if on {
                                "the controller restored the overlay widgets"
                            } else {
                                "the controller blanked the overlay widgets; the background keeps \
                                 drawing and the screen stays on"
                            });
                        }
                    }
                    controller::Command::Unhandled(name) => {
                        journal.warn(&format!(
                            "`{name}` is a directive Aura's schema names and this port has not \
                             built yet, so it was read and not acted on"
                        ));
                    }
                }
            }
        }
        for slot in store.expire(frame_began) {
            journal.warn(&format!(
                "{}: the card expired and the slot is now empty; its producer has gone quiet",
                slot.as_str()
            ));
        }

        // **The clock, produced here and pushed through the same door.**
        // `Store::apply` is the only way a card reaches the screen, so this
        // takes exactly the path a socket's card takes — including the check
        // that the card names the slot it was applied to.
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let minute = epoch / 60;
        if minute != last_minute {
            last_minute = minute;
            // **Only when the controller is not supplying one.** `wire.rs` puts
            // the format in the controller's hands — 12- or 24-hour, date shown
            // or not — and two producers writing `Primary` on their own
            // cadences would fight, which is what made the portal's toggle look
            // inert. This is the clock for a renderer with no controller, which
            // is the case `crate::clock` was written for.
            let controller_has_the_clock = controller
                .as_ref()
                .is_some_and(|c| c.clock_is_live(frame_began));
            if !controller_has_the_clock {
                let card = Message::Card(clock::card(i64::try_from(epoch).unwrap_or(i64::MAX)));
                if let Err(why) = store.apply(Slot::Primary, &card, frame_began) {
                    journal.error(&format!("the synthesised clock was refused: {why}"));
                }
            }

            // **The display window, re-evaluated once a minute** - `ADR-039`
            // §2's predicate, and a minute is the resolution the window is
            // written at so anything finer would ask the same question twice.
            //
            // Re-evaluated rather than scheduled: nothing here stores the
            // instant of the next transition, so a wall clock that steps -
            // which every Rivet does at boot, `ADR-021` §4.2 - cannot leave
            // this waiting for a moment that has passed.
            let lit = display_hours.is_none_or(|hours| {
                rivet_sdk::schedule::local_now().is_none_or(|now| hours.is_on(now))
            });
            if lit != screen_lit {
                screen_lit = lit;
                needs_redraw = true;
                journal.info(if lit {
                    "inside the machine's display hours: drawing again"
                } else {
                    "outside the machine's display hours: blanking, and drawing nothing until \
                     the screen is due back on"
                });
            }
        }

        // **The mark this build cannot draw, said once per change of state.**
        //
        // `SPEC-04` §7 rule 4 requires a mark beside a `stale` reading and
        // instead of a value for `connecting` and `unavailable`.
        //
        // The icon face ships, and the generic composition draws both marks
        // and trend arrows. What remains is narrower and is a RULING rather
        // than a gap: `DrawWeatherPanel` has no mark in it, and R-125 fixes the
        // panel's appearance, so putting one there would be inventing a
        // picture for Aura's own card. Rule 5 still settles the outcome —
        // omitting is diagnosable, drawing the wrong thing is not.
        //
        // **Said when the slot's state changes, not per card.** The store is
        // dirtied by every card in every slot, and a producer re-sends the same
        // `unavailable` reading on its own cadence, so keying this on
        // `is_dirty` alone wrote the same sentence many times a minute into a
        // journal that is a rolling buffer. `unmarked::Watch` says when it
        // starts, when the state differs, and when it clears.
        if store.is_dirty() {
            let now = store.get(Slot::Secondary).and_then(unmarked_state);
            match unmarked_said.observe(now) {
                Some(unmarked::Change::Arrived(state)) => journal.warn(&format!(
                    "secondary: a reading arrived with state `{state}` and SPEC-04 §7 rule 4 \
                     asks for a mark beside it. The WEATHER PANEL does not draw one — \
                     `DrawWeatherPanel` has no mark in it and its appearance is ruled fixed \
                     (R-125), so adding one would be inventing a picture. `tertiary` and \
                     `quaternary` draw the mark through the generic composition (SPEC-04 §5.9). \
                     Said once until the state changes"
                )),
                Some(unmarked::Change::Cleared(was)) => journal.info(&format!(
                    "secondary: no longer holds a reading in state `{was}`; the panel needs no \
                     mark it cannot draw"
                )),
                None => {}
            }
        }

        // **Nothing is updated or drawn while the screen is dark**, which is
        // the point of the exercise: the animated background is the whole
        // screen changing thirty times a second, and a building's twenty
        // screens doing that all night is the power the schedule exists to
        // save. Cards still arrive and still expire - a producer is not told to
        // go quiet, and the store is correct the moment the screen comes back.
        // **Captured before animation is folded in** — see the note below on
        // why this one read is the whole of the split.
        let widgets_forced = needs_redraw;

        if screen_lit {
            // **The pack advances itself.** `update(dt)` was this program's
            // call while it owned the drawing; now `time_ms` crosses in the
            // frame descriptor and the pack derives its own `dt` from
            // consecutive values. That is what lets a background step a
            // simulation at four steps a second without this loop knowing it
            // does — which is a thing this loop has no business knowing.
            //
            // `if (ss_animated) needs_redraw = true;` — an animated background
            // is the whole screen changing, which the dirty flag cannot see.
            // The pack says which of its themes animate; `false` is the safe
            // default and the reason is on `rivetos_addon::Theme::animated`.
            if appearance.animated || store.is_dirty() {
                needs_redraw = true;
            }
        }

        // **What the WIDGETS need is not what the background needs**, and
        // until 2026-09-21 this loop could not tell the two apart — there was
        // one buffer, so anything that redrew the background redrew the clock
        // whether or not it had changed. `PLAN.md` §181's soak measures that
        // at 11.2 ms of a 19.5 ms drawing budget, spent on a picture identical
        // to the last one for every frame but one in eighteen hundred.
        //
        // **Read BEFORE the clause above, which is the whole trick.** Every
        // other thing that sets `needs_redraw` — a card arriving, the minute
        // turning, brightness, the theme, the corner moving, the screen
        // lighting — is a reason the WIDGETS changed. Only `appearance.
        // animated` is a reason only the background did. So the flag's value
        // at this point, before animation is folded in, already IS "a widget
        // reason happened", and one line separates them rather than eight
        // scattered assignments that a ninth reason could quietly miss.
        let widgets_dirty = widgets_forced || store.is_dirty();

        if needs_redraw {
            needs_redraw = false;
            let drawing_began = Instant::now();
            // **The context becomes current before anything draws.** On a
            // `wl_shm` surface this is a no-op; on a GPU one it is the host's
            // half of `rivetos_addon::gl`'s contract, and it has to happen
            // before the pack is called AND before `show_cpu` uploads, which
            // is why it is here rather than in either branch.
            if let Err(why) = display.begin() {
                if refusals.is_multiple_of(REFUSAL_QUIET_FRAMES) {
                    journal.error(&format!("the context would not come current: {why}"));
                }
                refusals += 1;
                continue;
            }
            if !screen_lit {
                // **Black, and then nothing.** One blit puts it on the glass
                // and the loop draws no more frames until the window opens,
                // because `needs_redraw` is only set again by the minute tick
                // above. See the start-up warning for what this does NOT do.
                canvas.fill(0);
                // **The widget layer has to go dark too, and it is a separate
                // surface now.** Blanking the background alone would leave the
                // clock and the cards lit on top of it — a screen that the
                // schedule turned off and that still glows with everything a
                // passer-by can read. The background's `fill(0)` used to cover
                // them because they were in the same buffer.
                if let Ok(widget_canvas) = display.overlay() {
                    rivet_sdk::draw::erase(widget_canvas, &overlay_target);
                    let _ = display.present_overlay();
                }
                let _ = display.show_cpu(&canvas);
                let _ = display.finish();
                store.clean();
                let spent = frame_began.elapsed();
                if let Some(left) = FRAME_BUDGET.checked_sub(spent) {
                    std::thread::sleep(left);
                }
                continue;
            }
            // **The background, drawn by somebody else's code in this address
            // space.** A refusal leaves whatever was in the canvas, which is
            // the previous frame — better on a wall than a flash of black, and
            // the journal carries the reason at most once every
            // `REFUSAL_QUIET_FRAMES`.
            //
            // **Which BOUNDARY it crosses is decided here, per frame** —
            // `.rulings/` R-206 and R-207. Three conditions, all of which
            // have to hold before a pixel is drawn on the GPU:
            //
            // 1. this machine gave a GPU surface (`display.is_gpu()`),
            // 2. the object exports a GPU entry point (`p.draws_gl()`),
            // 3. and **this background** declares one (`appearance.on_gpu`),
            //    which is per background and not per pack: Contours, Life and
            //    Boxes say yes, Digital Rain and Starfield say no, in the one
            //    pack, switchable from the portal while this loop runs.
            //
            // Anything else is the CPU path, drawn into `canvas` and handed
            // over by `show_cpu` — which on a GPU surface is an upload and on
            // a `wl_shm` one is the blit this program has always done.
            let on_gpu = display.is_gpu()
                && appearance.on_gpu
                && pack.as_ref().is_some_and(pack::Pack::draws_gl);
            match pack.as_ref() {
                Some(p) => {
                    let time_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as u64);
                    let drew = if on_gpu {
                        // The host's half of `rivetos_addon::gl`'s contract:
                        // the context is current (`begin`, above) and the
                        // target is the surface itself.
                        match display.gl_target() {
                            Some(target) => {
                                p.draw_gl(&target, (&background_id, &theme_id), time_ms)
                            }
                            None => Err("the GPU surface has no target".to_string()),
                        }
                    } else {
                        p.draw(
                            &mut canvas,
                            display.width() as u32,
                            display.height() as u32,
                            display.stride() as u32,
                            (&background_id, &theme_id),
                            time_ms,
                        )
                        .and_then(|()| display.show_cpu(&canvas))
                    };
                    if let Err(why) = drew {
                        if refusals.is_multiple_of(REFUSAL_QUIET_FRAMES) {
                            journal.error(&format!(
                                "`{background_id}/{theme_id}`: {why} (said once every \
                                 {REFUSAL_QUIET_FRAMES} frames while it keeps happening)"
                            ));
                        }
                        refusals += 1;
                    } else if refusals != 0 {
                        journal.info(&format!(
                            "`{background_id}/{theme_id}` is drawing again after {refusals} \
                             refused frames"
                        ));
                        refusals = 0;
                    }
                }
                // No pack: black behind the cards, which is Aura's own default
                // picture. The sentence explaining it was written once at
                // start-up and is not repeated thirty times a second.
                None => {
                    canvas.fill(0);
                    let _ = display.show_cpu(&canvas);
                }
            }
            let background_done = Instant::now();

            let (screen_w, screen_h) = (display.width() as f32, display.height() as f32);
            // **The widgets are a SUBSURFACE now, and are drawn only when they
            // change** — `rivet_sdk::overlay`, 2026-09-21.
            //
            // Everything below draws into `widget_canvas` rather than into the
            // background's `canvas`, and the whole block is skipped on a frame
            // where nothing about the widgets is different. The compositor
            // keeps showing the last one for free, which is what a compositor
            // is for and what this program was paying 11.2 ms a frame not to
            // use.
            //
            // **`ARGB8888`, premultiplied**, which is why `overlay_target` is
            // built with `Target::argb` and not `::xrgb`: this layer is drawn
            // OVER the background, so its fourth byte is coverage and the
            // drawing primitives have to maintain it. Built with `::xrgb` the
            // whole layer would come out transparent and look exactly like
            // widgets that had stopped being drawn.
            let widget_canvas = if widgets_dirty {
                match display.overlay() {
                    Ok(bytes) => Some(bytes),
                    Err(why) => {
                        if refusals.is_multiple_of(REFUSAL_QUIET_FRAMES) {
                            journal.error(&format!("no widget layer to draw into: {why}"));
                        }
                        None
                    }
                }
            } else {
                None
            };
            if let Some(widget_canvas) = widget_canvas {
            // **Erased and not cleared.** Most of this surface is nothing at
            // all — the background has to show through everywhere a card is
            // not — and `draw::clear` paints a COLOUR. Without this the
            // previous minute's clock stays underneath the new one.
            rivet_sdk::draw::erase(widget_canvas, &overlay_target);
            // Shadowed so the widget calls below name the same `canvas` and
            // `target` they did when they drew into the background's buffer —
            // the change is WHERE they draw, and nothing about what they draw.
            let canvas = &mut *widget_canvas;
            let target = &overlay_target;
            if let Some(card) = store.get(Slot::Primary).filter(|_| show_widgets) {
                let (time, date) = clock_strings(card);
                widgets::clock(
                    canvas,
                    target,
                    &bold,
                    &regular,
                    &time,
                    &date,
                    clock_corner,
                    screen_w,
                    screen_h,
                    grey,
                    pad,
                    card_theme.as_ref(),
                );
            }
            // **The weather panel** — `SPEC-04` §5.8, built 2026-09-02.
            //
            // `secondary`'s composition is `DrawWeatherPanel`: a `pair` is its
            // two columns, a lone `reading` is one column, and a `statement` is
            // wrapped and centred under whichever it got. Which composition a
            // slot draws is `SPEC-04` §11 (12) and is open in general; this one
            // is settled because the card that occupies the slot is Aura's own
            // and the maintainer has ruled its appearance fixed.
            // `show_weather` is *"the whole panel, indoor column included"* —
            // the schema's own words, and the C++ skips the draw rather than
            // emptying the card. The card stays in the store either way, so
            // turning it back on needs no round trip to the controller.
            if let Some(card) = store
                .get(Slot::Secondary)
                .filter(|_| show_weather && show_widgets)
            {
                let (left, right, statement) = weather_panel_parts(card);
                widgets::weather_panel(
                    canvas,
                    target,
                    &bold,
                    &regular,
                    &left,
                    &right,
                    &statement,
                    clock_corner,
                    screen_w,
                    screen_h,
                    grey,
                    pad,
                    card_theme.as_ref(),
                );
            }
            // **Slots three and four, through the generic composition** —
            // `crate::layout`, built 2026-09-02 (`SPEC-04` §11 (12)).
            //
            // Until then a card here was held, journalled and thrown away,
            // which made the vocabulary a vocabulary in name only: an addon
            // could send a shape the protocol admits and nothing would draw it.
            //
            // **This does not put a picture that is not Aura's on the screen**,
            // which is the outcome the maintainer named as worst and the reason
            // the old generic path was struck. It is the other way round: the
            // *composition* is generic — blocks stacked in the card's own order
            // — and every *number* is `widgets`', which is `widgets.hpp`'s. A
            // slot Aura has no widget for has no fixed appearance to violate.
            //
            // Aura's own corner reading card — a value, a unit and a trend —
            // is drawn here by `layout`'s headline piece, the original
            // renderer's reading card at its own constants; anything else in
            // these slots takes the generic composition, which is what a THIRD
            // PARTY's card falls to and what it is for.
            //
            // The corners are the C++'s own: its corner rule for the reading
            // card is a quarter turn from the clock, and the fourth is
            // opposite that. The clock and
            // the panel own one diagonal, these two own the other, and
            // `the_three_widgets_occupy_three_different_corners_at_every_rotation`
            // is the property from the other side.
            for (slot, corner) in [
                (Slot::Tertiary, clock_corner.quarter_turn()),
                (Slot::Quaternary, clock_corner.quarter_turn().opposite()),
            ] {
                if let Some(card) = store.get(slot).filter(|_| show_widgets) {
                    let drawn = layout::card(
                        canvas,
                        target,
                        &bold,
                        &regular,
                        icon_face.as_ref(),
                        card,
                        corner,
                        screen_w,
                        screen_h,
                        grey,
                        pad,
                        card_theme.as_ref(),
                    );
                    if !drawn && store.is_dirty() {
                        // Held, and genuinely nothing to draw: every block is a
                        // shape whose content this build cannot render — a
                        // reading with no value and one of the two unported
                        // glyphs, say. Said once per card rather than thirty
                        // times a second, for `unmarked_state`'s reason.
                        journal.warn(&format!(
                            "{}: a card is held and drew NOTHING — every block on it is empty or \
                             carries only a glyph this build has no drawing for (SPEC-04 §5.6.1). \
                             The card is not lost; there is nothing on it to show",
                            slot.as_str()
                        ));
                    }
                }
            }

            }
            // **Committed only when it was redrawn.** A subsurface that were
            // recommitted every frame would put the widget layer back on the
            // compositor's work list for nothing, which is most of what moving
            // it here was meant to stop.
            if widgets_dirty
                && let Err(why) = display.present_overlay()
            {
                if refusals.is_multiple_of(REFUSAL_QUIET_FRAMES) {
                    journal.error(&format!("the widget layer was not handed over: {why}"));
                }
                refusals += 1;
            }

            let widgets_done = Instant::now();
            // **The background was already handed over** — by `show_cpu` on
            // the CPU path, or by the pack drawing straight into this
            // surface on the GPU one. What is left is the swap, which is a
            // no-op on `wl_shm` because `blit` commits.
            if let Err(why) = display.finish() {
                if refusals.is_multiple_of(REFUSAL_QUIET_FRAMES) {
                    journal.error(&format!("the frame was not handed over: {why}"));
                }
                refusals += 1;
            }
            let blit_done = Instant::now();
            store.clean();

            cost.record(
                background_done - drawing_began,
                widgets_done - background_done,
                blit_done - widgets_done,
                blit_done - frame_began,
            );
            if cost.frames >= REPORT_EVERY_FRAMES {
                let selection = format!("{background_id}/{theme_id}");
                journal.info(&cost.report(started.elapsed(), &selection));
                // **And the same fact as a health signal** — `ADR-045`.
                //
                // **`report` and never `Answering::serve`.** This pushes a
                // datagram to `NOTIFY_SOCKET` and returns; the pull path,
                // where the supervisor CONNECTS to an application and waits,
                // is the exact mechanism that stops this machine's supervisor
                // roughly 34 hours into every boot (`STATE.md` §0 —
                // `ask_health` blocked in `connect` on an application whose
                // backlog filled because nothing accepted). A renderer that
                // held a health socket would be a second candidate for that
                // outage, and this one holds the display.
                //
                // **Why this exists at all**: the 111% mean of 2026-09-20 sat
                // in this journal for months, in a line nobody reads, on a
                // wall running at 20 fps that looked fine. A number nothing
                // watches is not a measurement.
                let state = if cost.over_budget() {
                    rivet_sdk::health::Reported::Degraded
                } else {
                    rivet_sdk::health::Reported::Healthy
                };
                let _ = rivet_sdk::health::report(state, Some(&cost.caption(&selection)));
                cost = FrameCost::default();
            }

            // ~~The application takes its own picture, because nothing else
            // can while it holds the lease.~~ **Removed with the lease** —
            // `ADR-058` §5.
            //
            // That sentence was true and is the whole argument against keeping
            // this: what an application could photograph was **its own frame**,
            // never the screen, and it only looked like a screenshot because
            // nothing else was on the screen. `rivet-displayd` composites
            // everything and can be asked for the real thing at any moment, by
            // an operator, without this program's cooperation — which is the
            // capture that is worth having when a program is wedged.
            //
            // So Aura no longer writes `screen.qoi`, no longer spends a frame
            // every `CAPTURE_EVERY` encoding one, and
            // `cargo rivet screenshot` gets a better picture than it used to.
            let _ = &mut next_capture;
        }

        // Sleep what is left of the budget, not a constant after the work.
        let spent = frame_began.elapsed();
        if let Some(left) = FRAME_BUDGET.checked_sub(spent) {
            std::thread::sleep(left);
        }
    }
}

/// **What a frame costs, in four phases**, accumulated over a window.
///
/// # Why the phases are these four
///
/// `background` and `widgets` are the CPU drawing, and their sum is the number
/// that decides whether this fits: it is what the program spends before it is
/// allowed to hand anything over.
///
/// `blit` is **not** a copy. `Screen::blit` copies into the back page and then
/// **waits for the page flip to complete**, so on a double-buffered display it
/// is mostly the wait for the next vertical blank — which is the panel pacing
/// the loop and is exactly what it is supposed to do. Reading it as drawing
/// cost is the mistake this split exists to prevent: the 35.7 ms figure the
/// skeleton reported was a whole frame including that wait, on a nearly static
/// screen, and it was read as though it were work.
///
/// `total` is the whole frame. On a 60 Hz panel a frame that misses one vblank
/// lands near 33 ms whatever the drawing cost, so `total` is a rate and the two
/// drawing phases are a cost. Both are reported because either alone misleads.
///
/// **Maxima, not just means.** A mean hides the one frame in a hundred that
/// stutters, and a stutter is what a person sees.
#[derive(Default)]
struct FrameCost {
    frames: u32,
    background: Phase,
    widgets: Phase,
    blit: Phase,
    total: Phase,
}

#[derive(Default)]
struct Phase {
    sum: Duration,
    max: Duration,
}

impl Phase {
    fn add(&mut self, d: Duration) {
        self.sum += d;
        self.max = self.max.max(d);
    }
    fn mean_ms(&self, frames: u32) -> f64 {
        if frames == 0 {
            return 0.0;
        }
        self.sum.as_secs_f64() * 1000.0 / f64::from(frames)
    }
    fn max_ms(&self) -> f64 {
        self.max.as_secs_f64() * 1000.0
    }
}

impl FrameCost {
    fn record(&mut self, background: Duration, widgets: Duration, blit: Duration, total: Duration) {
        self.frames += 1;
        self.background.add(background);
        self.widgets.add(widgets);
        self.blit.add(blit);
        self.total.add(total);
    }

    /// **What this window's drawing cost, against the budget.**
    ///
    /// The mean and not the max, because one frame that overran is a hiccup
    /// and a mean that overruns is a picture this board cannot draw. The
    /// board ran at 20 fps for months on a 111% mean; no max would have said
    /// so any more clearly than that did.
    fn over_budget(&self) -> bool {
        let drawing = self.background.mean_ms(self.frames) + self.widgets.mean_ms(self.frames);
        drawing > FRAME_BUDGET.as_secs_f64() * 1000.0
    }

    /// One sentence for a health caption — `ADR-045` §5 caps it at 120 bytes.
    fn caption(&self, selection: &str) -> String {
        let frames = self.frames;
        let drawing = self.background.mean_ms(frames) + self.widgets.mean_ms(frames);
        let budget_ms = FRAME_BUDGET.as_secs_f64() * 1000.0;
        format!(
            "{selection}: drawing {drawing:.1}ms of {budget_ms:.1}ms ({:.0}%)",
            drawing / budget_ms * 100.0,
        )
    }

    fn report(&self, uptime: Duration, selection: &str) -> String {
        let frames = self.frames;
        let drawing = self.background.mean_ms(frames) + self.widgets.mean_ms(frames);
        let budget_ms = FRAME_BUDGET.as_secs_f64() * 1000.0;
        format!(
            "cost over {frames} frames at {:.0}s up drawing `{selection}`: \
             background {:.2}/{:.2}ms, widgets {:.2}/{:.2}ms, \
             DRAWING {drawing:.2}ms of the {budget_ms:.1}ms budget ({:.0}%), \
             blit-and-flip-wait {:.2}/{:.2}ms, frame {:.2}/{:.2}ms \
             (mean/max; blit is mostly the wait for vblank, not work)",
            uptime.as_secs_f64(),
            self.background.mean_ms(frames),
            self.background.max_ms(),
            self.widgets.mean_ms(frames),
            self.widgets.max_ms(),
            drawing / budget_ms * 100.0,
            self.blit.mean_ms(frames),
            self.blit.max_ms(),
            self.total.mean_ms(frames),
            self.total.max_ms(),
        ) + &thermals()
    }
}

/// **What the drawing is costing the SoC**, appended to every cost report.
///
/// The maintainer asked to watch thermals while the renderer runs, and this is
/// the honest place for it: the same line already says what a frame cost, so
/// the temperature sits beside the work that produced it and the journal
/// carries the pair. **No second program.** A soak probe of its own would
/// sample the same two files and be a process to install, enable and reason
/// about; `get thermal` already answers on demand, and what was missing was a
/// RECORD OVER TIME rather than a reading.
///
/// Zone 0 and no search, for `examples/aura/aura-vitals-test`'s reason: a Pi
/// has one zone and it is the SoC. A machine without one — QEMU — says so
/// rather than showing a number from nowhere.
fn thermals() -> String {
    let millidegrees = std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()
        .and_then(|t| t.trim().parse::<i64>().ok());
    let khz = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .ok()
        .and_then(|t| t.trim().parse::<i64>().ok());
    match (millidegrees, khz) {
        (Some(m), Some(k)) => format!(
            ", SoC {}.{}°C at {} MHz",
            m / 1000,
            (m % 1000) / 100,
            k / 1000
        ),
        (Some(m), None) => format!(", SoC {}.{}°C", m / 1000, (m % 1000) / 100),
        // Not a fault and not worth a warning every report: a machine with no
        // thermal zone is QEMU, and the rest of the line is still true.
        (None, _) => String::new(),
    }
}

/// The first reading on this card whose state asks for a mark this build cannot
/// draw, if any. See the call site.
fn unmarked_state(card: &aura_card::Card) -> Option<&'static str> {
    let unmarked = |r: &aura_card::Reading| {
        (r.state != aura_card::State::Ok).then(|| r.state.as_str())
    };
    card.blocks.iter().find_map(|block| match block {
        Block::Reading(r) => unmarked(r),
        Block::Pair(p) => unmarked(&p.left).or_else(|| unmarked(&p.right)),
        _ => None,
    })
}

/// The two columns and the statement, out of the card in `secondary`.
///
/// Read back off the card for [`clock_strings`]'s reason: the picture is drawn
/// from what went through [`store::Store::apply`] and not from a second copy
/// that could disagree with it.
///
/// A `pair` gives both columns; a lone `reading` gives the left one and leaves
/// the right empty, which the panel centres for. Anything else in the card —
/// a `list` in this slot, say — contributes nothing, because there is no
/// arrangement in `DrawWeatherPanel` for it and the alternative is inventing
/// one.
fn weather_panel_parts(card: &aura_card::Card) -> (widgets::Column, widgets::Column, String) {
    let mut left = widgets::Column::default();
    let mut right = widgets::Column::default();
    let mut statement = String::new();
    for block in &card.blocks {
        match block {
            Block::Pair(pair) => {
                left = widgets::Column::from_reading(&pair.left);
                right = widgets::Column::from_reading(&pair.right);
            }
            Block::Reading(reading) => left = widgets::Column::from_reading(reading),
            Block::Statement(s) => statement.clone_from(&s.text),
            Block::List(_) => {}
        }
    }
    (left, right, statement)
}

/// The time and the date out of the clock card.
///
/// Read back off the card rather than kept beside it, so the picture is drawn
/// from what went through [`store::Store::apply`] and not from a second copy
/// that could disagree with it.
fn clock_strings(card: &aura_card::Card) -> (String, String) {
    let mut time = String::new();
    let mut date = String::new();
    for block in &card.blocks {
        if let Block::Reading(reading) = block {
            if let Some(value) = &reading.value {
                time.clone_from(value);
            }
            if let Some(caption) = &reading.caption {
                date.clone_from(caption);
            }
        }
    }
    (time, date)
}
