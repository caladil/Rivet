//! **Taking a press from something with no hands** — `ADR-012` §9.
//!
//! The receiving half of the surface `cmd/rivet/src/press.rs` writes to. It is
//! twenty lines of work and one paragraph of reason, and the paragraph is the
//! important part.
//!
//! # Why the launcher, of all programs
//!
//! `ADR-012` is called *The Agent as a First-Class Operator* and until today it
//! was not true. An agent could install to a machine, reconfigure it, roll it
//! back and reboot it, and could not press ENTER on it. The lease mechanism in
//! `PLAN.md` §106 was designed, written and shipped by an agent that then had to
//! ask a person in another room to push a button so it could learn whether its
//! own work functioned.
//!
//! `ADR-012` §9.3 settles where the surface belongs by elimination: without a
//! launcher there are no kiosk applications, and without kiosk applications
//! there is nothing an input could mean. So the launcher is not *a* place to put
//! it — it is the only one.
//!
//! # Why this needs no new permission
//!
//! It is a `provides` socket (`ADR-015` §5.3). The supervisor binds it, and
//! because nothing on the machine declares `connects = ["rivet-launcher:input"]`
//! it is granted to nobody and stays `0600 root:root`. `rivet` reaches it as
//! root over `ADR-035`'s control plane and nothing unprivileged can reach it at
//! all. The grant mechanism protects this by *declining to grant*, which is a
//! better answer than a permission invented for the occasion.
//!
//! # What it deliberately cannot do
//!
//! One line, one action, closed vocabulary, no payload. There is no form of
//! this that types text, because `ADR-012` §9.3 rules parity with a person and
//! a person at a Rivet kiosk has buttons and no keyboard.

use std::io::Read as _;
use std::os::unix::net::UnixListener;

use crate::input::Action;

/// What the journal calls a press that arrived over the control plane.
///
/// **Deliberately not a device path.** `ADR-012` §4 makes the actor a property
/// of every request, and §9.3 rules that an agent's press must be tellable from
/// a person's. Two people reading one journal — `PLAN.md` §104.18 — must not
/// have to guess which of them pushed the button.
pub const ACTOR: &str = "an agent (control plane)";

/// The socket, if the supervisor handed one over.
///
/// `None` is the ordinary case for a launcher started any other way, and it is
/// not an error: the machine simply has no agent input surface, which is
/// exactly what an absent socket means at the other end too.
pub struct Presses(UnixListener);

/// How long one input connection may take to say what it came to say.
///
/// **Measured against the draw loop, not against a network.** Both ends are on
/// this machine, over a Unix socket, and `rivet apply input` writes its words
/// and closes in one go — so this is enormous for the legitimate case. What it
/// bounds is the illegitimate one: at `TICK` of 50 ms, this is the most frames
/// a caller that connects and then says nothing can cost, and the answer is two
/// rather than all of them.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_millis(100);

/// The most one input connection may say.
///
/// A press is a word from a closed vocabulary and a burst is a few of them.
/// This is thousands, so nothing an operator or an agent legitimately sends is
/// near it, and a caller that streams for ever is cut off rather than buffered.
const MAX_PRESS_BYTES: u64 = 4096;

impl Presses {
    /// Take the listener the supervisor bound for `provides = ["socket:input"]`.
    ///
    /// The descriptor arrives by [`rivet_sdk::listen`] rather than by opening a
    /// path, because that is the SDK's first article (`ADR-036` §2): **an
    /// application is handed what it needs; it does not go and take one.** The
    /// launcher obeying that rule in its own source is worth more than the
    /// fifteen lines it saves.
    #[must_use]
    pub fn offered() -> Option<Self> {
        let socket = rivet_sdk::listen::socket(rivet_core::input::ENDPOINT)?;
        // `None` if this process already owns that descriptor, which folds into
        // the same `Option` as "it was never offered" — both mean this launcher
        // has no input endpoint to accept on, and both are already handled by
        // the caller as a machine with no agent input.
        let listener = socket.into_unix_listener()?;
        // **Non-blocking, because this is a draw loop.** A blocking accept here
        // would stop the clock — and the clock is what animates the menu and
        // what times a long press.
        listener.set_nonblocking(true).ok()?;
        println!(
            "rivet-launcher: accepting input on the {} endpoint (ADR-012 §9)",
            rivet_core::input::ENDPOINT
        );
        Some(Self(listener))
    }

    /// Everything pressed since the last pass.
    ///
    /// Drains rather than taking one, so a burst does not smear across frames.
    /// A connection that says something unparseable is **reported and dropped**:
    /// silence would be the failure this project keeps naming, and a launcher
    /// that exits because somebody wrote a bad word is worse than one that says
    /// so.
    pub fn drain(&self) -> Vec<Action> {
        let mut out = Vec::new();
        loop {
            match self.0.accept() {
                Ok((stream, _)) => {
                    // **The ACCEPTED stream is not non-blocking, and the
                    // listener being non-blocking does not make it so.** Linux
                    // does not carry `O_NONBLOCK` across `accept(2)`, so
                    // `offered`'s reasoning a few lines up — *"a blocking
                    // accept here would stop the clock"* — held for the accept
                    // and was lost the moment one succeeded. A caller that
                    // connected and never closed its end held the render loop
                    // for ever: no frames, no clock, and no long-press timer,
                    // from an endpoint any local process can reach. Found in
                    // the audit of 2026-09-03 (`PLAN.md` §123).
                    //
                    // A deadline rather than a poll registration, for the
                    // reason Wave 2.2 gives at the kiosk socket: this is one
                    // tiny message from a client that has already written it,
                    // and a state machine for that would be more moving parts
                    // than the problem has.
                    if let Err(why) = stream.set_read_timeout(Some(READ_DEADLINE)) {
                        println!(
                            "rivet-launcher: an input connection could not be given a deadline, \
                             so it was dropped rather than risked: {why}"
                        );
                        continue;
                    }
                    let mut said = String::new();
                    // **And bounded.** `read_to_string` on a socket reads until
                    // the peer closes; the deadline stops a silent caller but
                    // not a chatty one, and a press is a handful of words.
                    if let Err(why) = stream.take(MAX_PRESS_BYTES).read_to_string(&mut said) {
                        println!("rivet-launcher: an input connection said nothing: {why}");
                        continue;
                    }
                    for word in said.split_whitespace() {
                        match rivet_core::input::Action::parse(word) {
                            Some(wire) => out.push(here(wire)),
                            None => println!(
                                "rivet-launcher: `{word}` is not an action this build knows, so \
                                 nothing was pressed. One of: {}",
                                rivet_core::input::Action::vocabulary()
                            ),
                        }
                    }
                }
                // Nothing waiting. The ordinary case, every frame.
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return out,
                Err(why) => {
                    println!("rivet-launcher: the input endpoint could not be read: {why}");
                    return out;
                }
            }
        }
    }
}

/// **The wire vocabulary, in this program's words.**
///
/// Two enums, on purpose. `rivet_core::input::Action` is the closed vocabulary
/// an operator types and both ends agree on; [`crate::input::Action`] is what
/// this launcher's event loop deals in, and it is named for the hardware — the
/// hold is `BackHeld` here because that is what the timer produced.
///
/// **Written out rather than derived**, and exhaustive so it cannot rot: adding
/// a word to the vocabulary stops this compiling until somebody decides what the
/// launcher does with it. A `_ =>` arm would turn that decision into silence,
/// which is the shape of most of the defects this project has had to chase.
const fn here(wire: rivet_core::input::Action) -> Action {
    use rivet_core::input::Action as Wire;
    match wire {
        Wire::Up => Action::Up,
        Wire::Down => Action::Down,
        Wire::Left => Action::Left,
        Wire::Right => Action::Right,
        Wire::Enter => Action::Enter,
        Wire::Back => Action::Back,
        // The one that is not a rename: a hold is a DIFFERENT event, and each
        // end says so in its own words.
        Wire::LongBack => Action::BackHeld,
        Wire::Capture => Action::Capture,
    }
}
