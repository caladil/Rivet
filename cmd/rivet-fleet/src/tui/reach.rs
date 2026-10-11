//! **Whether a machine answers, and whether it is the machine we think it is** —
//! asked without blocking the interface.
//!
//! The maintainer asked for it on 2026-08-27: the device list should be able to
//! indicate whether each device pings up or down (.rulings/ R-327).
//!
//! # It is a handshake, not a TCP connection, and that is the decision
//!
//! A bare TCP connection to the control plane's port would need none of ICMP's
//! privileges and would be the obvious probe. It is the wrong question, and the
//! reason is worth stating because the obvious answer is so nearly right.
//!
//! The control plane's trust is a **pin** — the machine's public key, as
//! `crate::register::Device::pin` records it — and a TCP connection tests none
//! of it. *Can I open a session* and *do I trust this machine* are one question
//! here, and only a handshake asks it.
//!
//! So a probe that connected to 7777 and reported **up** would say *reachable*
//! about a machine presenting a key this operator does not pin. That is either a
//! device whose identity was replaced or something else wearing its address, and
//! both are worse news than *down* — a green dot beside them would be the exact
//! failure this pane's founding rule names, **a claim the tool has not
//! checked**.
//!
//! # Four states, because two would be a lie and three would still be one
//!
//! [`Reach::Unknown`] is not a placeholder. An interface that shows
//! *unreachable* before it has asked is stating something it does not know, and
//! the moment after start-up is exactly when a person is looking. So every
//! device begins unknown and becomes something else when an answer arrives.
//!
//! [`Reach::Stranger`] is the state this rewrite exists for. It is drawn in the
//! error style and it does **not** read as *down*, because the two need
//! different things done about them: a machine that is down needs switching on,
//! and a machine that answers with the wrong key needs a person to find out
//! why before anything is sent to it.
//!
//! [`Reach::Unasked`] is *this end could not ask* — no `--identity`, or a
//! device whose record holds no pin. It is a fact about the workstation and
//! saying **down** about the machine would be attributing it to the wrong end.
//!
//! # What "up" does not mean
//!
//! It means **the control plane answered and presented the key the register
//! records**. It does not mean Rivet's supervisor is healthy, that the services
//! are running, or — and this one is worth stating because it is easy to assume
//! — **that the machine accepts THIS operator**. TLS 1.3 lets a client finish
//! its side before the server has judged the client certificate, so a board that
//! will refuse this identity still completes the handshake here and refuses at
//! the first request. `rivet_remote::tls::Tls::handshake` carries that caveat at
//! the point it is created; the stronger claims belong on the device screen,
//! where the tool has actually asked.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rivet_remote::tls::Handshake;

/// How long to wait for the machine before calling it down.
///
/// **Two seconds**, which is far longer than a household network needs and
/// short enough that a whole fleet resolves while a person is still reading the
/// first row. It is not a measure of the machine — a board answers in under a
/// millisecond — it is how long to wait before saying so.
///
/// It bounds the handshake as well as the connection, which a probe that only
/// opened a socket would not have to think about: a port that accepts a
/// connection and then says nothing would otherwise hold a probe thread for as
/// long as the operating system lets a read block.
pub const TIMEOUT: Duration = Duration::from_secs(2);

// **There is no `PORT` constant here, deliberately.** The port is
// `crate::register::Device::listen`'s (`ADR-034` §6.1), so one written here
// would be a second answer to *how do I reach this machine*. The probe takes a
// transport that has already been pointed at the right place.

/// What is known about a machine's reachability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Nobody has asked yet, or the answer has not come back.
    Unknown,
    /// The control plane answered and presented the key the register records.
    /// See the module header for the three things this deliberately does not
    /// mean.
    Up,
    /// **Something answered and it is not the machine the register describes.**
    /// A key that is not pinned, or a program on that port that is not this
    /// control plane at all.
    Stranger,
    /// Nothing answered within [`TIMEOUT`], or the name did not resolve.
    Down,
    /// **This end could not ask**: no `--identity`, or no pin in the record.
    /// Nothing is known about the machine.
    Unasked,
}

impl Reach {
    /// The word a person reads, beside the glyph.
    ///
    /// **Every state has one**, because `crate::tui::theme`'s rule is that no
    /// glyph carries meaning alone.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Unknown => "checking",
            Self::Up => "up",
            Self::Stranger => "STRANGER",
            Self::Down => "down",
            Self::Unasked => "not asked",
        }
    }

    // **`is_trouble` is deliberately not here.** It would read well and nothing
    // would call it: `crate::tui::fleet` matches every variant exhaustively to
    // pick a glyph AND a style, which is stricter — a variant added later fails
    // the build there rather than falling into a default. A predicate would
    // have given it somewhere to fall.
    //
    // What that match encodes, since it is a decision rather than a rendering:
    // `Stranger` draws in the WARN style and `Down` in the down style, because
    // the two need different things done about them; `Unasked` draws faint,
    // because a run with no identity has learned nothing bad about the machine
    // and colouring it as trouble would attribute a missing flag to the board.

    /// Read one handshake's answer.
    #[must_use]
    pub const fn of(answer: &Handshake) -> Self {
        match answer {
            Handshake::Trusted => Self::Up,
            Handshake::Untrusted(_) => Self::Stranger,
            Handshake::Unreachable(_) => Self::Down,
            Handshake::Unusable(_) => Self::Unasked,
        }
    }
}

/// One answer, on its way back to the interface.
#[derive(Debug, Clone)]
pub struct Answer {
    /// Which device, by the identifier — **not the label**, because a label can
    /// change while a probe is in flight and `ADR-025` §4.1 says which of the
    /// two is allowed to.
    pub id: String,
    pub reach: Reach,
    /// **Whatever this end or the far end said**, kept verbatim for a screen
    /// that shows detail. `None` when there was nothing to say.
    ///
    /// A [`Reach::Stranger`] with no detail would be the pane telling a person
    /// something is wrong and refusing to say what — which is the shape
    /// `AGENTS.md` §5 exists to prevent.
    pub detail: Option<String>,
}

/// Probes running in the background, and the answers they have sent.
///
/// **The interface never waits.** A probe that has not answered leaves its
/// device [`Reach::Unknown`], the screen draws, and the row changes when the
/// answer arrives — which is why this is a channel and not a function that
/// returns a map.
pub struct Probes {
    to_ui: Sender<Answer>,
    from_probes: Receiver<Answer>,
}

impl Probes {
    #[must_use]
    pub fn new() -> Self {
        let (to_ui, from_probes) = channel();
        Self { to_ui, from_probes }
    }

    /// **Ask about one device**, over a transport already pointed at it.
    ///
    /// Spawns a thread and returns immediately. **The thread is detached and
    /// its answer may never arrive** — if the interface has exited by then the
    /// send fails and the thread ends, which is the whole of the cleanup this
    /// needs.
    ///
    /// The transport is built by the caller rather than here, because building
    /// one reads this operator's identity off the disk and that is the
    /// application's business, not a probe's.
    pub fn ask(&self, id: &str, transport: rivet_remote::tls::Tls) {
        let (id, to_ui) = (id.to_owned(), self.to_ui.clone());
        let named = transport.host.clone();
        // A named thread, so a stuck probe is identifiable in a debugger rather
        // than being one of several anonymous ones.
        let _ = std::thread::Builder::new()
            .name(format!("reach:{named}"))
            .spawn(move || {
                let answer = transport.handshake(TIMEOUT);
                // The receiver is gone when the interface has exited. That is
                // not an error and there is nothing to report it to.
                let _ = to_ui.send(Answer {
                    id,
                    reach: Reach::of(&answer),
                    detail: answer.detail().map(str::to_owned),
                });
            });
    }

    /// **Say that this end could not ask**, without spawning anything.
    ///
    /// A device with no pin, or a run with no `--identity`, has an answer
    /// already — and it is one this end knows without a socket. Sent through
    /// the same channel so the interface has one place answers arrive.
    pub fn cannot_ask(&self, id: &str, why: String) {
        let _ = self.to_ui.send(Answer {
            id: id.to_owned(),
            reach: Reach::Unasked,
            detail: Some(why),
        });
    }

    /// Every answer that has arrived since the last call. Never blocks.
    pub fn collected(&self) -> Vec<Answer> {
        self.from_probes.try_iter().collect()
    }
}

impl Default for Probes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **Every state has a word.** `crate::tui::theme`'s rule is that no glyph
    /// carries meaning alone, and this is the other half of it.
    #[test]
    fn every_state_says_itself_in_words() {
        let all = [
            Reach::Unknown,
            Reach::Up,
            Reach::Stranger,
            Reach::Down,
            Reach::Unasked,
        ];
        for state in all {
            assert!(!state.word().is_empty(), "{state:?}");
        }
        // And the five words are five words. A pair that read the same would be
        // two states a person cannot tell apart, which is the whole defect this
        // module was rewritten to fix.
        for (n, one) in all.iter().enumerate() {
            for other in &all[n + 1..] {
                assert_ne!(one.word(), other.word(), "{one:?} and {other:?}");
            }
        }
    }

    /// **Not knowing reads as not knowing.** The word a person sees before an
    /// answer arrives must not be `down`, which would be a claim.
    #[test]
    fn the_word_before_an_answer_is_not_a_claim_about_the_machine() {
        assert_eq!(Reach::Unknown.word(), "checking");
    }

    /// **`ADR-034` §6.1's neighbour, as a test.** A machine that answers with a
    /// key this end does not pin must not read as `up` — the failure this
    /// rewrite exists for — and must not read as `down` either, because the two
    /// need different things done about them.
    #[test]
    fn a_machine_answering_with_the_wrong_key_is_neither_up_nor_down() {
        let stranger = Reach::of(&Handshake::Untrusted("not pinned".to_owned()));
        assert_eq!(stranger, Reach::Stranger);
        assert_ne!(stranger, Reach::Up);
        assert_ne!(stranger, Reach::Down);
    }

    /// Each of the transport's four answers maps to exactly one state, and no
    /// two of them collapse together.
    #[test]
    fn each_handshake_answer_is_its_own_state() {
        let said = "why".to_owned();
        assert_eq!(Reach::of(&Handshake::Trusted), Reach::Up);
        assert_eq!(Reach::of(&Handshake::Untrusted(said.clone())), Reach::Stranger);
        assert_eq!(Reach::of(&Handshake::Unreachable(said.clone())), Reach::Down);
        assert_eq!(Reach::of(&Handshake::Unusable(said)), Reach::Unasked);
    }

    /// **The interface never waits.** `collected` returns what has arrived and
    /// nothing else, so a draw between probes is a draw and not a stall.
    #[test]
    fn collecting_answers_never_blocks() {
        let probes = Probes::new();
        assert!(probes.collected().is_empty());
        // The assertion is that this RETURNS, which it would not if it waited.
        let _ = probes.collected();
    }

    /// An answer names the device by its identifier, because a label can change
    /// while a probe is in flight and `ADR-025` §4.1 says which of the two may.
    #[test]
    fn an_answer_is_addressed_by_identifier_and_not_by_label() {
        let probes = Probes::new();
        probes.cannot_ask("4f2c9a01bd7e3865", "no identity was given".to_owned());
        let answers = probes.collected();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].id, "4f2c9a01bd7e3865");
        assert_eq!(answers[0].reach, Reach::Unasked);
    }

    /// **A state that is bad news must say why.** A red dot with nothing behind
    /// it tells a person something is wrong and refuses to say what.
    #[test]
    fn an_answer_this_end_composed_carries_its_reason() {
        let probes = Probes::new();
        probes.cannot_ask("an-id", "the record holds no pin".to_owned());
        let answers = probes.collected();
        assert_eq!(answers[0].detail.as_deref(), Some("the record holds no pin"));
    }
}
