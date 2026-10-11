//! **Answer when the supervisor asks whether this application is doing its
//! job** — `ADR-045` §3.
//!
//! The supervisor asks. The application answers. It is handed a descriptor it
//! did not open, it answers `get status` on it, and the supervisor decides how
//! often to ask and how long to wait.
//!
//! ```no_run
//! use rivet_sdk::health::{Answering, Reported};
//! use std::sync::{Arc, Mutex};
//!
//! // What this application currently believes about itself. Its own loop keeps
//! // it current; the supervisor's poll reads whatever it last concluded.
//! let opinion = Arc::new(Mutex::new((Reported::Healthy, None::<String>)));
//!
//! let asked = Answering::handed().expect("every application is handed one");
//! let mine = Arc::clone(&opinion);
//! std::thread::spawn(move || {
//!     asked.serve(move || mine.lock().map_or(
//!         (Reported::Down, Some("this application's own lock is poisoned".to_string())),
//!         |o| o.clone(),
//!     ));
//! });
//! ```
//!
//! # Why asking, and not telling
//!
//! **The case it catches is WEDGED BUT ALIVE.** A crashed process is already
//! visible — the supervisor holds the pid and counts the restarts. A deadlocked
//! one keeps its pid and simply never speaks again, and under a telling design
//! that is indistinguishable from a healthy application with nothing new to say.
//!
//! **And an application that chose WHEN to speak would be setting policy.** One
//! that reported every ten minutes would have set the fleet's monitoring
//! resolution; one that reported at startup and then went quiet would have
//! declared itself permanently healthy. `ADR-045` §4 takes severity away from
//! application authors for the same reason, and §3.1.1 closes the side door.
//!
//! # It says how it IS; it does not say how bad that is
//!
//! Whether a degraded Trends is worth waking somebody is fleet policy and not an
//! application's opinion of its own importance. *An author who can set their own
//! severity will set it high, and every author will.* There is deliberately no
//! way to send one from here.
//!
//! # The assessment is yours; the cadence is not
//!
//! Keep your own conclusion current — on your own loop, at your own threshold
//! (`ADR-045` §8: *thirty minutes* is TempMon's number, about itself, and no
//! layer above could know it). This module reads whatever that conclusion is
//! when the supervisor asks. **An application that wedges stops updating the
//! conclusion AND stops answering, and both are visible.**
//!
//! # `unknown` is not yours to say
//!
//! [`Reported`] has three variants where the machine-level vocabulary has four.
//! `unknown` is what an OBSERVER concludes about a thing that has said nothing
//! (`ADR-045` §4), so a service claiming it would be claiming to have not
//! spoken, while speaking. **The type makes it unrepresentable** rather than a
//! parser rejecting it after the fact.
//!
//! # The caption is for a person, and only for a person
//!
//! Four states are enough for *"is this healthy"* and are not enough for *"the
//! battery is getting low"*, which is useless without a sensor's name in it. So
//! pass a short sentence. It is capped at [`CAPTION_MAX_BYTES`] and
//! **truncated rather than refused** — a caption must never be the reason a
//! service stops.
//!
//! **Nothing downstream will branch on its contents**, deliberately. Write it
//! for a human being reading a dashboard at midnight, not for a program.
//!
//! # Why this re-declares the vocabulary instead of importing it
//!
//! `ADR-036` §8.3: **the SDK links no Rivet crate.** `rivetos-core` is a
//! dev-dependency and nothing else, so an application depending on the SDK never
//! pulls in the operating system's internals. The words and the wire form below
//! are therefore a second copy of `rivet_core::health`'s — and
//! `tests/agreement.rs` asserts a **round trip** through the real parser, which
//! is what buys the right to re-declare.
//!
//! That test is not ceremony. A drifted constant fails loudly; a drifted
//! *encoding* does not — a health line the supervisor cannot parse is one an
//! application believes it sent.

/// **What an application may say about ITSELF.**
///
/// Three words where the machine-level vocabulary has four; see the module note
/// on `unknown`. Agreement with `rivet_core::health::Reported` is asserted by
/// `tests/agreement.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported {
    /// Doing its job.
    Healthy,
    /// Working, but something is wrong that a person should eventually know.
    Degraded,
    /// Not doing its job.
    Down,
}

impl Reported {
    /// The word this state is spelled with on the wire.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Down => "down",
        }
    }
}

/// **The caption's cap, in BYTES of UTF-8** — `ADR-045` §5.
///
/// Long enough for *"no broadcast from GVH5075_2222 in 34 minutes"*, short
/// enough for a toast and for one line of `rivet get status`. A second copy of
/// `rivet_core::health::CAPTION_MAX_BYTES`, held honest by `tests/agreement.rs`.
pub const CAPTION_MAX_BYTES: usize = 120;

/// The notify-socket key a health report rides on.
pub const KEY: &str = "HEALTH";

/// The environment variable the supervisor sets to the path of this service's
/// socket. The name is `sd_notify`'s, because the protocol on it is too
/// (`ADR-017` §3.4) — `KEY=value` lines, of which `HEALTH=` is one.
#[cfg(unix)]
const NOTIFY_SOCKET: &str = "NOTIFY_SOCKET";

/// What happened to a report — the answer to *"did anybody hear that?"*.
///
/// **A named answer rather than a `bool`**, because the two failures want
/// different reactions: [`Self::NoChannel`] is a machine that never offered one
/// and is not an error at all, while [`Self::Failed`] is a channel that exists
/// and did not take the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    /// The report was written to the supervisor's socket.
    Yes,
    /// **This process was handed no `NOTIFY_SOCKET`, and that is not a fault.**
    /// A program running on a workstation, in a test, or under a supervisor
    /// that does not want reports gets this. Ignoring it is correct; there is
    /// nothing an application can do about it and nothing it should stop for.
    NoChannel,
    /// The socket was there and the write did not land.
    Failed(String),
}

/// **Ask the supervisor to poll this application NOW** — `ADR-045` §3.
///
/// **This is a hint and NOT a health report.** It was the mechanism until
/// 2026-09-08 and is now an interrupt: what it means on the wire is *ask me
/// now*, and the state passed here is **discarded by the supervisor**. The poll
/// is the truth.
///
/// Call it when this application has just concluded something a person should
/// not wait up to [`INTERVAL_HINT_MS`] to hear — a data-integrity failure at
/// 03:00 is the case it exists for. Then answer the question that follows, on
/// [`Answering`].
///
/// **Calling it is never required.** An application that only answers is
/// completely correct and is the ordinary case; this exists so that *sooner*
/// is available, not so that *at all* is.
///
/// Returns [`Delivered`], which most callers can ignore.
///
/// # Why the state is still a parameter
///
/// Because `NOTIFY_SOCKET` speaks `KEY=value` and `HEALTH=` needs a value —
/// `ADR-017` §3.4 keeps `sd_notify`'s protocol untouched so that a foreign
/// daemon speaking it works here unchanged. Passing what you are about to
/// answer is the honest thing to put there. **Nothing reads it**, and an
/// application that relied on it being read would be declaring its own health
/// on its own schedule, which is what the reversal took away.
pub fn report(state: Reported, caption: Option<&str>) -> Delivered {
    send(&line(state, caption))
}

/// **How often the supervisor asks, in milliseconds** — as of 2026-09-08.
///
/// **A hint about somebody else's behaviour, not a promise to an application.**
/// The cadence is the supervisor's and it may change without this constant
/// changing (`rivet_core::health::INTERVAL_MS` is the real one, and the SDK
/// links no Rivet crate). It is here so that an author sizing their own
/// assessment loop has a number to reason about rather than a guess: assessing
/// far more often than this is work nobody reads, and far less often is a
/// conclusion the supervisor reports while it is already stale.
pub const INTERVAL_HINT_MS: u64 = 30_000;

/// **The `LISTEN_FDNAMES` entry the answering socket arrives under** —
/// `ADR-045` §3.
///
/// A manifest does not declare it and cannot: `unix:rivet-health` is refused at
/// packaging time, because two descriptors claiming the same name would be a
/// collision an application could not see. Every application is handed one.
pub const CHANNEL_NAME: &str = "rivet-health";

/// **What the supervisor writes on it.** The same words an operator types at
/// the control plane, which is why it is a whole operation rather than a bare
/// word.
pub const REQUEST: &str = "get status\n";

/// **The answering socket, handed over** — `ADR-045` §3.
///
/// It is a listening unix socket the supervisor bound, `listen`ed on, and
/// passed as one of `LISTEN_FDS`. This application never opened it and never
/// binds one: *an application declares what it needs and is handed it* is
/// `SPEC-03` §2's first article, and this is the case where it did not even
/// have to declare.
#[cfg(unix)]
pub struct Answering {
    listener: std::os::unix::net::UnixListener,
    /// The mode [`Answering::set_nonblocking`] last put the LISTENER in.
    ///
    /// **Re-applied to every stream [`Answering::answer_one`] accepts.**
    /// Linux's `accept(2)` does not inherit `O_NONBLOCK` from the listener, so
    /// an accepted stream is blocking regardless of the listener's own mode
    /// unless told otherwise — see that method. [`Answering::serve`] never
    /// changes this, so it stays at its default of blocking, which is what an
    /// accepted stream already is with nothing done to it.
    nonblocking: std::cell::Cell<bool>,
}

#[cfg(unix)]
impl Answering {
    /// Take the socket this application was handed, by its reserved name.
    ///
    /// `None` when there is none, which is **not an error**: a program run by
    /// hand on a workstation, in a test, or under a supervisor older than this
    /// convention gets it, and there is nothing an application can do about it
    /// and nothing it should stop for. It is the same answer
    /// [`Delivered::NoChannel`] gives for the other direction.
    #[must_use]
    pub fn handed() -> Option<Self> {
        let socket = crate::listen::socket(CHANNEL_NAME)?;
        Some(Self {
            listener: socket.into_unix_listener()?,
            nonblocking: std::cell::Cell::new(false),
        })
    }

    /// **Answer questions until the socket goes away.** Blocking; run it on a
    /// thread of its own.
    ///
    /// `opinion` is called once per question and must return **what this
    /// application currently believes about itself** — it is a read of a
    /// conclusion your own loop keeps current, not the place to compute one.
    ///
    /// # This closure must not block, and that is the whole contract
    ///
    /// The supervisor gives up after about two seconds and reports *it did not
    /// answer*, which `get status` shows as `unknown`. So an `opinion` that
    /// opens a database, takes a lock another thread holds, or makes a network
    /// call will make a healthy application look wedged — and it will do it
    /// intermittently, which is worse. **Read a value somebody else keeps
    /// fresh.**
    ///
    /// # It answers everything, including a request it does not recognise
    ///
    /// The only caller is the supervisor and the only question is `get status`,
    /// so a line that is not that one is a version skew rather than an
    /// intruder — the socket is `0600` and root-owned. Answering anyway is the
    /// forgiving direction and costs nothing; refusing would make an
    /// application built against a newer supervisor report itself unhealthy for
    /// a reason that has nothing to do with its health.
    pub fn serve<F>(&self, opinion: F)
    where
        F: Fn() -> (Reported, Option<String>),
    {
        while let Ok((stream, _)) = self.listener.accept() {
            let (state, caption) = opinion();
            Self::answer(stream, state, caption.as_deref());
        }
    }

    /// Answer exactly one question, if one is waiting.
    ///
    /// For an application that already has a poll loop of its own and would
    /// rather not spend a thread. Set the descriptor non-blocking, put
    /// [`Answering::fd`] in the poll set, and call this when it is readable.
    ///
    /// Returns whether a question was answered.
    ///
    /// # The accepted stream does not inherit non-blocking from the listener
    ///
    /// Linux's `accept(2)` hands back a stream in blocking mode regardless of
    /// the listener's own mode. A caller that trusted inheritance would have
    /// this call read from that stream exactly as `answer` always has — and a
    /// peer that connects and then says nothing would hold this call, and the
    /// poll loop it was meant not to block, until that peer spoke or closed.
    /// So the mode [`Answering::set_nonblocking`] last recorded is applied to
    /// the accepted stream here, explicitly, before `answer` ever reads from
    /// it.
    pub fn answer_one(&self, state: Reported, caption: Option<&str>) -> bool {
        match self.listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(self.nonblocking.get());
                Self::answer(stream, state, caption);
                true
            }
            Err(_) => false,
        }
    }

    /// The raw descriptor, to `poll`. **Reading a number is not owning it** —
    /// this type owns the socket and closes it when it is dropped.
    #[must_use]
    pub fn fd(&self) -> i32 {
        use std::os::fd::AsRawFd as _;
        self.listener.as_raw_fd()
    }

    /// Put the socket in or out of non-blocking mode.
    ///
    /// [`Answering::serve`] wants it blocking, which is the default it arrives
    /// in; [`Answering::answer_one`] inside somebody else's poll loop wants it
    /// non-blocking. Said out loud because `listen::Socket` hands the
    /// descriptor over unchanged and the other SDK adopters have already been
    /// caught by the same question once.
    ///
    /// **Remembered, and re-applied to every stream [`Answering::answer_one`]
    /// accepts** — see that method. Setting the LISTENER non-blocking only
    /// changes what `accept` itself does; Linux does not carry the mode
    /// through to the stream `accept` hands back, so this crate has to.
    ///
    /// # Errors
    ///
    /// Whatever `fcntl` said. The mode is only remembered once the listener
    /// itself accepted it.
    pub fn set_nonblocking(&self, yes: bool) -> std::io::Result<()> {
        self.listener.set_nonblocking(yes)?;
        self.nonblocking.set(yes);
        Ok(())
    }

    /// **Write the line and close.** The supervisor reads to the newline or to
    /// end-of-file, so closing is what finishes an answer that had no newline
    /// — and the newline is written anyway, because a protocol that depended on
    /// a close would break the moment anything kept the connection.
    ///
    /// **A write that fails is dropped in silence, deliberately.** The peer
    /// that went away is the supervisor, so there is nowhere to report it that
    /// anybody would read; and `ADR-045` §5's containment is the rule —
    /// answering a health question must never be the reason a service stops.
    fn answer(mut stream: std::os::unix::net::UnixStream, state: Reported, caption: Option<&str>) {
        use std::io::{Read as _, Write as _};

        // **The question is READ before the answer is written, and that is not
        // politeness.** Closing a unix stream socket with unread bytes still in
        // its receive queue sends RST, and the peer's pending inbound data goes
        // with it — so an answer written and then abandoned arrives at the
        // supervisor as `ECONNRESET`, which it correctly reports as *this
        // service did not answer*. Found by running it.
        //
        // Bounded, and short: the request is one line of a dozen bytes, and a
        // reader with no bound is a reader a peer can make allocate. Reading
        // the bytes and not inspecting them is deliberate — see the note on
        // `serve` about a request this build does not recognise.
        let mut swallow = [0u8; 256];
        // **The caller already put this stream in the right mode.** `serve`
        // never asks for non-blocking, so a stream reaching it here is
        // blocking by default; `answer_one` sets it explicitly first, because
        // Linux does not carry `O_NONBLOCK` through `accept(2)` the way the
        // listener's own mode might suggest. Either way, a read that would
        // block (or a non-blocking read that comes back empty) means *the
        // request has not arrived yet*, which is not worth waiting for: the
        // answer is what the supervisor is here for.
        let _ = stream.read(&mut swallow);

        let mut answer = line(state, caption);
        answer.push('\n');
        let _ = stream.write_all(answer.as_bytes());
        let _ = stream.flush();
    }
}

/// **The wire form: `HEALTH=<word>[ <caption>]`.**
///
/// The state is one word from a closed set containing no spaces, so the caption
/// is simply the rest of the line and needs no escaping — which is why this is
/// one key rather than two, and why a caption cannot arrive without the state it
/// describes.
///
/// Public so `tests/agreement.rs` can put it through the real parser.
#[must_use]
pub fn line(state: Reported, caption: Option<&str>) -> String {
    match caption.map(str::trim).filter(|c| !c.is_empty()) {
        Some(text) => format!("{KEY}={} {}", state.word(), scrub(truncate(text))),
        None => format!("{KEY}={}", state.word()),
    }
}

/// Cut a caption to [`CAPTION_MAX_BYTES`] **on a character boundary**.
///
/// The naive `&text[..120]` panics on a multi-byte character rather than
/// truncating, which would turn a long sentence in somebody's own language into
/// a crash.
fn truncate(text: &str) -> &str {
    if text.len() <= CAPTION_MAX_BYTES {
        return text;
    }
    let mut end = CAPTION_MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Replace the characters the notify protocol is built from.
///
/// That protocol is newline-separated `KEY=value`, so a newline in a caption
/// would split its own record and the remainder would be dropped as a line with
/// no `=`. **Replaced rather than refused**, because a sentence written for a
/// person may contain anything a sentence contains.
fn scrub(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

#[cfg(unix)]
fn send(line: &str) -> Delivered {
    send_to(std::env::var(NOTIFY_SOCKET).ok(), line)
}

/// **The decision, separated from the environment read**, so a test can take
/// the "nobody is supervising this" path without mutating the process's
/// environment — which is `unsafe` in current Rust, forbidden in this crate,
/// and observable by a parallel test.
#[cfg(unix)]
fn send_to(path: Option<String>, line: &str) -> Delivered {
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Delivered::NoChannel;
    };
    // **Unbound, and a new socket per report.** The supervisor's end is a bound
    // `UnixDatagram` and never replies, so there is nothing to keep open and no
    // ordering to preserve — a datagram either arrives whole or does not arrive.
    // Holding one open for a process's life would be a descriptor to lose track
    // of in exchange for nothing.
    let sock = match std::os::unix::net::UnixDatagram::unbound() {
        Ok(s) => s,
        Err(e) => return Delivered::Failed(e.to_string()),
    };
    match sock.send_to(line.as_bytes(), &path) {
        Ok(_) => Delivered::Yes,
        Err(e) => Delivered::Failed(e.to_string()),
    }
}

/// Off Unix there is no socket to write to, which is the same answer as a
/// machine that offered none — see [`Delivered::NoChannel`]. This exists so the
/// SDK still compiles on the development host.
#[cfg(not(unix))]
fn send(_line: &str) -> Delivered {
    Delivered::NoChannel
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **A program nobody is supervising gets an answer, not a panic.**
    /// The workstation case, and the one an author meets first.
    #[cfg(unix)]
    #[test]
    fn with_no_socket_a_report_says_so_and_does_not_fail() {
        assert_eq!(super::send_to(None, "HEALTH=healthy"), Delivered::NoChannel);
    }

    /// **An empty `NOTIFY_SOCKET` is no socket, not a path of length zero.**
    /// Without this the write fails and reports `Failed`, sending an author to
    /// look for a broken socket rather than an absent one.
    #[cfg(unix)]
    #[test]
    fn an_empty_socket_path_is_no_channel_rather_than_a_failure() {
        assert_eq!(
            super::send_to(Some(String::new()), "HEALTH=healthy"),
            Delivered::NoChannel
        );
    }

    /// **A real socket takes a real report.** The supervisor's end is a bound
    /// `UnixDatagram`; this stands one up and reads back exactly the bytes an
    /// application would have sent, which is the only way to know the two halves
    /// agree about the wire form.
    #[cfg(unix)]
    #[test]
    fn a_report_arrives_at_a_bound_socket_byte_for_byte() {
        let dir = std::env::temp_dir().join(format!("rivet-health-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("notify");
        let server = std::os::unix::net::UnixDatagram::bind(&path).expect("the supervisor's end");

        let sent = super::send_to(
            Some(path.to_string_lossy().into_owned()),
            &line(Reported::Degraded, Some("one sensor is quiet")),
        );
        assert_eq!(sent, Delivered::Yes);

        let mut buf = [0_u8; 512];
        let n = server.recv(&mut buf).expect("the datagram");
        assert_eq!(
            std::str::from_utf8(&buf[..n]).expect("utf-8"),
            "HEALTH=degraded one sensor is quiet"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A newline in a caption cannot split its own record**, or the half after
    /// it is dropped as a line with no `=`.
    #[test]
    fn a_newline_in_a_caption_cannot_split_the_record() {
        let l = line(Reported::Down, Some("first\nsecond\rthird"));
        assert_eq!(l.lines().count(), 1, "{l:?}");
        assert_eq!(l, "HEALTH=down first second third");
    }

    /// **Cut on a character boundary, never through one.**
    #[test]
    fn an_over_long_caption_is_cut_on_a_character_boundary() {
        let l = line(Reported::Degraded, Some(&"é".repeat(200)));
        let caption = l.strip_prefix("HEALTH=degraded ").expect("a caption");
        assert!(caption.len() <= CAPTION_MAX_BYTES);
        assert!("é".repeat(200).starts_with(caption));
    }

    /// A blank caption is no caption, so no reader has to decide what an empty
    /// sentence means.
    #[test]
    fn a_blank_caption_is_absent_rather_than_empty() {
        assert_eq!(line(Reported::Healthy, Some("   ")), "HEALTH=healthy");
        assert_eq!(line(Reported::Healthy, None), "HEALTH=healthy");
    }

    /// **The whole answering exchange, over a real socket pair.**
    ///
    /// A supervisor's end: bind, connect, write `get status`, read the line
    /// back. This asserts the thing an author actually depends on — that a
    /// question asked on the handed descriptor produces the line the
    /// application's own opinion says, terminated so a reader knows it is
    /// whole.
    #[cfg(unix)]
    #[test]
    fn a_question_on_the_handed_socket_is_answered_with_the_applications_own_opinion() {
        use std::io::{Read as _, Write as _};

        let dir = std::env::temp_dir().join(format!("rivet-answering-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("health");
        // The supervisor's half: it binds and listens, and the application is
        // handed the descriptor. Constructed directly here because
        // `Answering::handed` reads `LISTEN_FDS`, which a test may not set —
        // mutating this process's environment is `unsafe` and observable by a
        // parallel test.
        let answering = Answering {
            listener: std::os::unix::net::UnixListener::bind(&path).expect("the bound socket"),
            nonblocking: std::cell::Cell::new(false),
        };

        let asked = std::thread::spawn(move || {
            let mut client =
                std::os::unix::net::UnixStream::connect(&path).expect("the supervisor connects");
            client
                .write_all(REQUEST.as_bytes())
                .expect("the question goes out");
            let mut said = String::new();
            client.read_to_string(&mut said).expect("an answer");
            said
        });

        assert!(answering.answer_one(Reported::Degraded, Some("one sensor is quiet")));
        let said = asked.join().expect("the asking thread");
        assert_eq!(said, "HEALTH=degraded one sensor is quiet\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **An answer is one line and is newline-terminated**, so a reader that
    /// stops at the newline gets a whole answer and one that reads to
    /// end-of-file gets the same one.
    #[cfg(unix)]
    #[test]
    fn an_answer_is_one_terminated_line_whatever_the_caption_contains() {
        let mut answer = line(Reported::Down, Some("first\nsecond\rthird"));
        answer.push('\n');
        assert_eq!(answer.lines().count(), 1, "{answer:?}");
        assert!(answer.ends_with('\n'));
    }

    /// **`answer_one` must not block reading a peer that connected and said
    /// nothing, once `Answering` is non-blocking.** Linux does not carry
    /// `O_NONBLOCK` through `accept(2)`, so the accepted stream is blocking
    /// unless this crate sets it explicitly — the bounded read inside
    /// `answer` would otherwise wait for the peer, stalling whatever poll
    /// loop called this.
    ///
    /// Built directly, as
    /// `a_question_on_the_handed_socket_is_answered_with_the_applications_own_opinion`
    /// already does, because `Answering::handed` reads `LISTEN_FDS`, which a
    /// test may not set. **The listener itself stays blocking**, so `accept`
    /// below waits for the connection deterministically rather than racing
    /// it; only the recorded mode — what `answer_one` applies to the
    /// ACCEPTED stream — is what this test exercises.
    #[cfg(unix)]
    #[test]
    fn answer_one_does_not_block_reading_a_silent_peer_once_nonblocking() {
        let dir = std::env::temp_dir().join(format!("rivet-health-nb-silent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("health");

        let answering = Answering {
            listener: std::os::unix::net::UnixListener::bind(&path).expect("the bound socket"),
            nonblocking: std::cell::Cell::new(true),
        };

        // A peer that connects and then says nothing at all.
        let peer = std::os::unix::net::UnixStream::connect(&path).expect("the peer connects");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let answered = answering.answer_one(Reported::Healthy, None);
            let _ = tx.send((answered, started.elapsed()));
        });

        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok((answered, elapsed)) => {
                assert!(answered, "a queued connection should be answered");
                assert!(
                    elapsed < std::time::Duration::from_millis(500),
                    "answer_one took {elapsed:?} reading a silent peer"
                );
            }
            Err(_) => {
                // Release a call still stuck in the OLD blocking read, so the
                // failure below is a clear assertion rather than a test
                // process left hanging.
                drop(peer);
                panic!("answer_one did not return within the watchdog bound");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A peer that sends its request normally still gets the answer**, once
    /// `Answering` is non-blocking — the ordinary case the fix above must not
    /// break.
    ///
    /// The client signals over a channel once its write has landed, so this
    /// calls `answer_one` only after the bytes are already sitting in the
    /// accepted stream's receive buffer — otherwise a non-blocking read here
    /// could race the client's write and answer before reading anything,
    /// which is allowed (see `answer`'s own note) but would make this test
    /// assert nothing about reading the request.
    #[cfg(unix)]
    #[test]
    fn a_nonblocking_answering_still_answers_a_peer_that_sends_its_request() {
        use std::io::{Read as _, Write as _};

        let dir = std::env::temp_dir().join(format!("rivet-health-nb-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("health");

        let answering = Answering {
            listener: std::os::unix::net::UnixListener::bind(&path).expect("the bound socket"),
            nonblocking: std::cell::Cell::new(true),
        };

        let (sent_tx, sent_rx) = std::sync::mpsc::channel();
        let asked = std::thread::spawn(move || {
            let mut client =
                std::os::unix::net::UnixStream::connect(&path).expect("the supervisor connects");
            client.write_all(REQUEST.as_bytes()).expect("the question goes out");
            let _ = sent_tx.send(());
            let mut said = String::new();
            client.read_to_string(&mut said).expect("an answer");
            said
        });

        sent_rx.recv_timeout(std::time::Duration::from_secs(2)).expect("the peer to have written");
        assert!(answering.answer_one(Reported::Degraded, Some("one sensor is quiet")));
        let said = asked.join().expect("the asking thread");
        assert_eq!(said, "HEALTH=degraded one sensor is quiet\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
