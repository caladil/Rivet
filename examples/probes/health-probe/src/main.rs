//! **Answer the supervisor when it asks how this application is** — `ADR-045`
//! §3.1, from the application's side.
//!
//! ```text
//! supervisor  --connect + "get status"-->  this probe
//! this probe  --"HEALTH=degraded no reading in 7s"-->  supervisor
//! ```
//!
//! The socket arrives under the reserved `LISTEN_FDNAMES` entry `rivet-health`.
//! **Nothing in this program's manifest asked for it** — §3.1.2: every
//! application is handed one, on the mechanism that already exists, because the
//! defensible number is the count of ways an application is handed things and
//! not the count of things it is handed.
//!
//! # The two behaviours, and why they are one program
//!
//! Chosen by the name this was INSTALLED under:
//!
//! * **`health-probe`** — healthy, then `degraded` with a sentence naming what
//!   is wrong. `ADR-045` §11 step 1.
//! * **anything containing `wedge`** — healthy, and then it **stops itself**
//!   with `SIGSTOP`. The process is still there, its pid is unchanged, its
//!   restart count is zero, and it never speaks again. §11.1 step 1a, and the
//!   case a telling design reports as `healthy` for ever.
//!
//! One program with a mode word is `tools/qemu/testapp`'s convention. Two
//! crates would have made the two boots differ by a whole build rather than by
//! the one thing under test.
//!
//! # What it is NOT allowed to say
//!
//! **How bad any of it is.** `ADR-045` §4 puts that with the operator, in the
//! machine's document, under `critical` — and `ADR-045` §8 is the sentence
//! this probe is shaped by: an application can say *"nothing has been
//! uploaded in seven days"*, and cannot say whether that is a problem.
//!
//! # The threshold is this application's, about itself
//!
//! [`DEGRADE_AFTER`] is the shape of TempMon's thirty minutes (`ADR-045` §8):
//! a fact about how long this program can go without doing its job before it is
//! not doing its job. Nothing above it could know that number, and nothing
//! above it is told.
//!
//! **On a real application it would be an `ADR-038` setting**, changeable on a
//! running board. It is a constant here because a proof that had to write a
//! settings file to reach its own assertion would be proving two things at
//! once.

use rivet_sdk::health::{Answering, Reported};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long this probe considers itself to be doing its job before it decides
/// it is not. See the module note: it is a threshold about ITSELF.
const DEGRADE_AFTER: Duration = Duration::from_secs(5);

/// How long the wedging variant runs before it stops itself.
///
/// **Longer than the supervisor's first question and shorter than its
/// second**, deliberately: the proof needs this application to have answered
/// once — so that what follows is a service which STOPPED answering rather than
/// one that never answered — and then to be silent when the next question
/// comes.
const WEDGE_AFTER: Duration = Duration::from_secs(6);

fn main() {
    let name = installed_name();
    let wedging = name.contains("wedge");
    println!("health-probe: running as {name}, pid {}", std::process::id());

    // **What this application currently believes about itself.** Its own loop
    // keeps it current; the supervisor's poll reads whatever it last concluded
    // (`ADR-045` §8 — *the tick and the answer come apart, which is better*).
    let opinion: Arc<Mutex<(Reported, Option<String>)>> =
        Arc::new(Mutex::new((Reported::Healthy, None)));

    // **The handed descriptor.** `None` means nothing handed one over, which is
    // a program run by hand rather than a fault — but under a supervisor it
    // means the reserved entry did not arrive, and this probe exists to notice
    // that, so it says so loudly and keeps running.
    match Answering::handed() {
        Some(answering) => {
            println!(
                "health-probe: handed the answering socket on fd {} — LISTEN_FDNAMES entry \
                 `rivet-health` (ADR-045 §3)",
                answering.fd()
            );
            let mine = Arc::clone(&opinion);
            std::thread::spawn(move || {
                answering.serve(move || {
                    // **This closure must not block.** The supervisor gives up
                    // after about two seconds; a lock somebody else holds for
                    // longer would make a healthy application look wedged, and
                    // intermittently, which is worse than a failure.
                    mine.lock().map_or_else(
                        |_| {
                            (
                                Reported::Down,
                                Some("this probe's own lock is poisoned".to_string()),
                            )
                        },
                        |held| held.clone(),
                    )
                });
                println!("health-probe: the answering socket closed; nothing will ask again");
            });
        }
        None => {
            println!(
                "health-probe: NO answering socket was handed to this process. Under a Rivet \
                 that is a defect — every application gets one — and `rivet get status` will \
                 report this service as `unknown` for the life of the boot"
            );
        }
    }

    // **Readiness, which is a different question from health** — `ADR-017`
    // §3.3. It says this program is serving; health says whether it is doing
    // its job, and a service can be ready and unwell.
    //
    // Written out here because the SDK has no readiness call: `ADR-017` §3.4
    // adopts `sd_notify`'s protocol whole, which is newline-separated
    // `KEY=value` on the datagram socket named by `NOTIFY_SOCKET`, and this is
    // the one line of it this probe needs. `rivet_sdk::health::report` is the
    // same socket carrying `HEALTH=`.
    notify("READY=1");
    println!("health-probe: declared ready");

    if wedging {
        // **The wedge, on a thread of its own**, so that the answering loop is
        // free to answer the first question before this fires.
        std::thread::spawn(|| {
            std::thread::sleep(WEDGE_AFTER);
            println!(
                "health-probe: WEDGING NOW — SIGSTOP to my own process group. My pid does not \
                 change, I do not exit, my restart count stays zero, and I will never answer \
                 again. Under ADR-045's telling design this machine would go on reporting the \
                 last thing I said, which was `healthy`"
            );
            // Flushed before the stop, or the sentence above never leaves the
            // buffer and the console shows a process that vanished silently.
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
            // SAFETY: `getpid` and `kill` take no pointers and touch no memory
            // this program owns. `SIGSTOP` cannot be caught or ignored and
            // stops the whole thread group, which is the point — a wedge that
            // left the answering thread running would not be a wedge.
            #[allow(unsafe_code)]
            unsafe {
                libc::kill(libc::getpid(), libc::SIGSTOP);
            }
        });
    }

    // **The assessment loop.** `ADR-039` §4: an application that needs
    // something to happen on a cadence is a service and has its own loop, which
    // it can already do, at no cost, with no platform involvement.
    let started = Instant::now();
    let mut said_degraded = false;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if wedging || said_degraded {
            continue;
        }
        let waited = started.elapsed();
        if waited < DEGRADE_AFTER {
            continue;
        }
        // **A fact about its own ability to do its job**, with the subject in
        // it — `ADR-045` §5: four states are enough for *"is this healthy"*
        // and are not enough for *"the battery is getting low"*.
        let caption = format!("no reading in {}s", waited.as_secs());
        if let Ok(mut held) = opinion.lock() {
            *held = (Reported::Degraded, Some(caption.clone()));
        }
        said_degraded = true;
        println!("health-probe: I am degraded — {caption}");
        // **`ADR-045` §3's *ask me now*, and it is a HINT.** The state
        // passed here is discarded by the supervisor; what it does is bring the
        // next poll forward, so a conclusion reached at 03:00 is seen before
        // the interval is up rather than up to thirty seconds later. The poll
        // is the truth, and the answer above is what it will read.
        let _ = rivet_sdk::health::report(Reported::Degraded, Some(&caption));
    }
}

/// Write one `KEY=value` line to the supervisor's notification socket.
///
/// A datagram either arrives whole or does not arrive, and a report nobody
/// heard is not something an application should stop for — so every failure
/// here is silent by design.
fn notify(line: &str) {
    let Ok(path) = std::env::var("NOTIFY_SOCKET") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    if let Ok(socket) = std::os::unix::net::UnixDatagram::unbound() {
        let _ = socket.send_to(line.as_bytes(), &path);
    }
}

/// The name this application was installed under.
///
/// `RIVET_APP_DIR` is `<volume>/app/<name>/current` (`SPEC-03` §3.2), so the
/// name is its parent's file name. **Read rather than compiled in**, because
/// the two behaviours are chosen by the installed name and a constant here
/// would make them one.
fn installed_name() -> String {
    std::env::var("RIVET_APP_DIR")
        .ok()
        .and_then(|dir| {
            std::path::Path::new(&dir)
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "health-probe".to_string())
}
