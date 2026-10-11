//! **`aura-vitals`** — what the machine can say about itself, as a card.
//!
//! `SPEC-04`, `PLAN.md` §112.2. An ordinary Aura addon: it declares
//! `connects = ["aura-renderer:tertiary"]`, is handed a socket, and writes
//! cards. There is no privileged path and nothing in `aura-renderer` knows this
//! program exists.
//!
//! # This is one of two, and the pair is the point
//!
//! `examples/aura/aura-vitals-go` is the same producer in Go. They claim the
//! **same endpoint**, read the **same files** and write the **same bytes** —
//! checked by both test suites against one shared expectation,
//! `examples/aura/vitals-fixture/expected-card.txt`.
//!
//! `ADR-036` §6.2B ruled two producers because *"`provides` is an interface, and
//! two implementations are what prove it"*. The consumer declares `connects`
//! once and either language satisfies it, with **no change to the consumer** —
//! which is `PLAN.md` §105.5's language-neutrality claim as a working
//! demonstration instead of a sentence.
//!
//! **Swap one for the other and the renderer cannot tell.** Both cannot run at
//! once and that is the platform's doing, not Aura's: two applications claiming
//! one endpoint is refused at launch by name and the second is not started
//! (`SPEC-03` §3.4).
//!
//! # Why vitals, and not weather
//!
//! `PLAN.md` §112.2 rules weather out and gives the reason: `aura-controller`
//! already fetches it, owns every unit conversion and is authoritative for it,
//! so a second weather client would be a second authority for one fact
//! (`ADR-011`). A machine's own temperature and uptime have **no other
//! authority** — the machine is the only thing that can say them — so a
//! producer for them competes with nothing.
//!
//! It also exercises the path this was built for: a `reading` and a `list` in
//! `tertiary`, which is `SPEC-04` §5.9's generic composition, which is what an
//! addon's card actually gets.
//!
//! # It fetches nothing and writes nothing
//!
//! `network = "none"` and `state = "none"`, and neither is a placeholder. It
//! reads `/proc` and `/sys`, which every process can, and sends. A reading kept
//! across a restart would be a number nobody is vouching for (`ADR-036` §8.5).

mod card;
mod vitals;

use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use vitals::Vitals;

/// Where the facts are read from. `/` on a machine; a fixture in a test.
const ROOT: &str = "/";

/// How long to wait before trying the socket again after a refusal.
///
/// **Shorter than the card's TTL, deliberately.** `ADR-017` §3.5's backoff is
/// for a process that keeps dying; this is a process that stays up and finds a
/// consumer that is not listening yet, which is the ordinary case at boot when
/// the renderer has not finished taking its display.
const RECONNECT_AFTER: Duration = Duration::from_secs(5);

fn main() {
    let journal = rivet_sdk::journal::Journal::for_service("aura-vitals-test");

    let Some(renderer) = rivet_sdk::app::endpoint("aura-renderer", card::SLOT.as_str()) else {
        // An ordinary state and not a crash: the grant was not made. Naming the
        // endpoint is what an operator needs, and the supervisor's own refusal
        // names the other half.
        journal.error(&format!(
            "aura-renderer:{} was not granted, so there is nowhere to send a card. \
             RIVET_CONNECT carries: {}",
            card::SLOT.as_str(),
            granted()
        ));
        return;
    };
    journal.info(&format!(
        "publishing machine vitals to {} every {}s, ttl {}s",
        renderer.path.display(),
        card::TICK.as_secs(),
        card::TTL_SECONDS
    ));

    let mut stream: Option<UnixStream> = None;
    let mut last_sent: Option<String> = None;

    loop {
        let began = Instant::now();
        let text = match card::encode(&Vitals::read(Path::new(ROOT))) {
            Ok(text) => text,
            Err(why) => {
                // The encoder ran the same rules the renderer will run, so this
                // is a defect in this program rather than a transport problem —
                // and sending it anyway would only move the refusal.
                journal.error(&format!("this producer composed a card it cannot send: {why}"));
                sleep_rest_of(began, card::TICK);
                continue;
            }
        };

        if stream.is_none() {
            match UnixStream::connect(&renderer.path) {
                Ok(fresh) => {
                    journal.info("connected to the renderer");
                    stream = Some(fresh);
                }
                Err(why) => {
                    journal.warn(&format!(
                        "{} is not accepting connections ({why}); the card on the wall expires \
                         on its own ttl and the slot empties, which is the correct outcome for \
                         a producer nobody can hear",
                        renderer.path.display()
                    ));
                    std::thread::sleep(RECONNECT_AFTER);
                    continue;
                }
            }
        }

        // **Sent every tick even when the bytes have not changed.** The card
        // carries its own expiry, so silence is indistinguishable from a
        // producer that died — which is the confusion `Card::ttl_seconds`
        // exists to resolve, and a keepalive is how a live producer answers it.
        if let Some(open) = stream.as_mut() {
            match open.write_all(text.as_bytes()).and_then(|()| open.flush()) {
                Ok(()) => {
                    if last_sent.as_deref() != Some(text.as_str()) {
                        journal.debug("the vitals changed");
                        last_sent = Some(text.clone());
                    }
                }
                Err(why) => {
                    journal.warn(&format!(
                        "the renderer's socket refused the card ({why}); reconnecting on the \
                         next tick"
                    ));
                    stream = None;
                }
            }
        }

        sleep_rest_of(began, card::TICK);
    }
}

/// Sleep whatever is left of the interval after the work, rather than a
/// constant after it — so the real period is the period and not `work +
/// constant`, which silently stretches. The same argument `aura-renderer`'s
/// frame budget makes, for the same reason.
fn sleep_rest_of(began: Instant, interval: Duration) {
    if let Some(left) = interval.checked_sub(began.elapsed()) {
        std::thread::sleep(left);
    }
}

/// What WAS granted, for the refusal above.
///
/// Named endpoints rather than raw `RIVET_CONNECT`, because the paths are noise
/// to somebody reading a journal at three in the morning and the names are the
/// thing they can compare against a manifest.
fn granted() -> String {
    let endpoints = rivet_sdk::app::endpoints();
    if endpoints.is_empty() {
        return "nothing".to_string();
    }
    endpoints
        .iter()
        .map(|e| format!("{}:{}", e.owner, e.name))
        .collect::<Vec<_>>()
        .join(", ")
}
