//! **A kiosk application that draws, in about forty lines.** `ADR-036` §6.1.
//!
//! Everything below the `draw_frame` function is plumbing you should not have
//! to think about, and it is here so you can see what the platform does for
//! you rather than having to trust it:
//!
//! * The display is **handed** to this process, not taken. `ADR-036` §2's
//!   first article — *an application declares what it needs and is handed it;
//!   it does not go and take one.* The launcher stays DRM master and cuts this
//!   process a lease, which is what lets a person get back out.
//! * Nothing here opens `/dev/dri`. The manifest's `devices = ["display"]` is
//!   the whole request, and the supervisor answers it in the environment.
//! * **Take the picture yourself.** While you hold the lease, the launcher is
//!   not drawing and cannot see your pixels — asked for a capture it refuses,
//!   correctly. `Screen::capture` is how your work becomes visible to somebody
//!   who is not standing in front of the machine.
//! * **If you keep state, treat your own unparseable state as ABSENT**
//!   (`ADR-036` §8.5). A kiosk is switched off by the plug, so a file caught
//!   mid-write comes back truncated rather than merely stale. An application
//!   that exits over it is restarted, fails again, and becomes a fault splash
//!   on a machine nobody is standing at. Cold-start instead — see
//!   `rivet_sdk::app::data`.

use rivet_sdk::display;
use rivet_sdk::draw;
use rivet_sdk::screen;
use rivet_sdk::text;

/// Functional colours, and still deliberately not a brand.
///
/// ~~Rivet has no `brand.json` yet.~~ It does — `ADR-037`, 2026-09-01,
/// `docs/brand/brand.json`. The reason these are not those is better than the
/// old one anyway: **this template becomes somebody else's application**, and a
/// starting point that stamped Rivet's accent on it would be dressing their
/// program as ours.
const BACKGROUND: [u8; 3] = [0x0d, 0x11, 0x17];
const CARD: [u8; 3] = [0x16, 0x1b, 0x22];
const INK: [u8; 3] = [0xe6, 0xed, 0xf3];
const ACCENT: [u8; 3] = [0x3f, 0xb9, 0x8c];

/// **Your application goes here.** Everything else is scaffolding.
fn draw_frame(canvas: &mut [u8], t: &text::Target, face: Option<&text::Face>, tick: u64) {
    draw::fill(canvas, t, 0, 0, t.width, t.height, BACKGROUND);

    let w = (t.width * 3 / 5).min(t.width);
    let h = (t.height / 3).min(t.height);
    let x = (t.width - w) / 2;
    let y = (t.height - h) / 2;
    draw::fill_rounded(canvas, t, x, y, w, h, 20, CARD);
    draw::stroke_rounded(canvas, t, x, y, w, h, 20, 3, ACCENT);

    let Some(face) = face else {
        return;
    };
    let mut centred = |s: &str, px: f32, baseline: usize, colour: [u8; 3]| {
        let tw = face.width_of(s, px);
        face.draw(canvas, t, x + (w.saturating_sub(tw)) / 2, baseline, s, px, colour);
    };
    centred("template-kiosk", 44.0, y + h / 2, INK);
    centred(&format!("frame {tick}"), 24.0, y + h / 2 + 48, ACCENT);
}

fn main() {
    // **Handed, not taken.** `Err` here is an ordinary state: started on a
    // machine with no launcher there is no channel, and saying so beats
    // crashing.
    let handed = match screen::handed() {
        Ok(handed) => handed,
        Err(why) => {
            println!("template-kiosk: no display was handed over: {why}");
            return;
        }
    };
    // **Keep the channel. This is the half a kiosk application is obliged to
    // have and the half that is easy to leave out** — `ADR-036` §8.1.
    //
    // A display lease can be REVOKED: the launcher takes the screen back to
    // show its menu, and a revoked lease cannot be resumed — the descriptor
    // dies with it. Coming back means accepting a **new** descriptor, and it
    // arrives here. An application that dropped this would work perfectly until
    // the first time somebody pressed BACK, and then never draw again with
    // nothing in any log to say why.
    let channel = handed.channel;
    let mut display = handed.screen;
    println!(
        "template-kiosk: drawing at {}x{}",
        display.width, display.height
    );

    // The system font. `None` is survivable — draw shapes and say so, rather
    // than exiting and leaving a black screen nobody can diagnose.
    let face = match text::Face::load(text::SYSTEM_FONT) {
        Ok(face) => Some(face),
        Err(why) => {
            println!("template-kiosk: no type, drawing without it: {why}");
            None
        }
    };

    let t = text::Target::xrgb(display.width, display.height, display.stride);
    let mut canvas = vec![0u8; display.stride * display.height];

    for tick in 0u64.. {
        // **The re-issue check, once a frame, and this is the whole of it.**
        //
        // `try_next` does not block: it asks whether a descriptor is waiting and
        // says no immediately when none is. A program that used `next()` here
        // would stop drawing in order to find out whether it should be drawing.
        //
        // Assigning over `display` drops the old `Screen`, and that drop is
        // what frees the revoked descriptor and both dumb buffers — 16 MB at
        // 1080p. Before 2026-09-03 the SDK freed neither.
        //
        // **A real application re-checks the geometry here.** This template's
        // canvas and text target are sized from the first display; a re-issued
        // one is the same panel in practice, but an application that cannot
        // assume that should rebuild both from `display.stride`,
        // `display.width` and `display.height` after this block.
        match channel.try_next() {
            Ok(None) => {}
            Ok(Some(lease)) => match screen::take_leased(lease) {
                Ok(fresh) => {
                    display = fresh;
                    println!("template-kiosk: the screen came back: {}", display.what);
                }
                Err(why) => println!("template-kiosk: a re-issued display would not drive: {why}"),
            },
            // The launcher has gone and nothing more is coming. Exiting is the
            // right answer, not an error: `restart` in the manifest decides
            // whether this program comes back.
            Err(display::DisplayError::Closed) => {
                println!("template-kiosk: the display channel closed; nothing more is coming");
                return;
            }
            Err(why) => println!("template-kiosk: the display channel could not be read: {why}"),
        }

        draw_frame(&mut canvas, &t, face.as_ref(), tick);
        display.blit(&canvas);

        // One picture a second, so an operator or an agent can see this without
        // standing in front of it. Drop this in a real application, or write it
        // only when something changed — a capture per frame is a card filling
        // up with pictures nobody asked for.
        if tick % 30 == 0 {
            match display.capture(&canvas) {
                Ok(where_it_went) => println!("template-kiosk: captured {where_it_went}"),
                Err(why) => println!("template-kiosk: capture failed: {why}"),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(33));
    }
}
