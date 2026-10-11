//! **What a thing says about whether it is doing its job** — `ADR-045`.
//!
//! `R-146` ruled that the commit gate's blindness is repaired by a general
//! mechanism rather than by teaching the gate about kiosks, and `ADR-045`
//! specifies it. This module is that ADR's §10 first bullet: *"`rivet-core`
//! gains one small pure module — the vocabulary, the fold, and what `critical`
//! means. No I/O, testable with no machine."*
//!
//! **This is the first hop** (`ADR-045` §11 steps 1 and 1a): the supervisor
//! ASKS, the application answers, and `get status` reports it — together with
//! [`machine`], §4.4's fold over the `critical` services. The commit gate (§7)
//! and the fleet server's absence handling (§6) are later steps and are
//! deliberately not here.
//!
//! # The supervisor asks; the application does not tell
//!
//! ~~The application states its own health over the notify socket.~~
//! **Reversed 2026-09-08** (`ADR-045` §3, and its `## History`). The case
//! that decided it is **wedged but alive**: a deadlocked process keeps its pid
//! and simply never speaks again, which under a telling design is
//! indistinguishable from a healthy application with nothing new to say.
//!
//! So the wire form below is now an ANSWER to [`REQUEST`], read off the
//! descriptor the supervisor handed over under [`CHANNEL_NAME`]. [`render`] and
//! [`parse`] are unchanged, because the line is the same line — what changed is
//! who speaks first.
//!
//! # The vocabulary is CLOSED, and closing it is the point
//!
//! `ADR-006` §6.4 rules closed vocabularies. `ADR-045` §4 argues this one: if
//! the states are not a small fixed set defined once, there will be forty
//! severity words within a year and every reader will have to guess at all of
//! them. **Closed means this enum is the only health type in the system, no
//! layer accepts a state as free text, and adding a fifth state is an amendment
//! to `ADR-045` rather than a patch.**
//!
//! # `unknown` is unrepresentable in what a thing says about itself
//!
//! `ADR-045` §4: *a thing that can speak can say `healthy`, `degraded` or
//! `down`; only its observer can conclude it has said nothing.*
//!
//! That rule is held by [`Reported`] having three variants and [`State`] four,
//! with a conversion in one direction only. **A rule a type makes
//! unrepresentable cannot be forgotten at a call site**, which is the same move
//! `ADR-040` made for Trends' plaintext listener. There is deliberately no
//! `From<State> for Reported`.
//!
//! # The caption travels WITH the state, in one message
//!
//! `ADR-045` §5: four states are the right vocabulary for *"is TempMon
//! healthy"* and are not enough for *"the battery is getting low"*, because
//! that sentence is useless without a sensor's name in it.
//!
//! So [`render`] puts both on one line and [`parse`] takes them off it
//! together. **Two keys would let a caption outlive the state it described** —
//! which is the shape of `PLAN.md` §133's defect, a descriptor outliving the
//! selection that produced it, and it is not a shape worth rebuilding.
//!
//! # What nothing may do with a caption
//!
//! Carry it verbatim. **Never branch on its contents** (`ADR-045` §5): the
//! moment a reader matches on the words, applications have an accidental
//! protocol nobody designed and nobody can change. It is display text, and
//! every renderer escapes it, because it is a string an application chose and a
//! portal will put in a document.

/// **The four states, and there are four** — `ADR-045` §4.
///
/// A reader may receive any of these. A thing may only *say* the three in
/// [`Reported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Doing its job.
    Healthy,
    /// Working, but something is wrong that a person should eventually know.
    Degraded,
    /// Not doing its job. Also what a supervisor concludes about a service it
    /// has marked failed.
    Down,
    /// **Has not said, and it is too early or too quiet to tell.** Produced by
    /// the layer above a silent thing and never by the thing itself.
    Unknown,
}

/// **What a thing may say about ITSELF** — `ADR-045` §4, as a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported {
    Healthy,
    Degraded,
    Down,
}

impl From<Reported> for State {
    fn from(r: Reported) -> Self {
        match r {
            Reported::Healthy => Self::Healthy,
            Reported::Degraded => Self::Degraded,
            Reported::Down => Self::Down,
        }
    }
}

/// The word for each state. **One spelling, defined here and nowhere else.**
impl State {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Down => "down",
            Self::Unknown => "unknown",
        }
    }
}

impl Reported {
    #[must_use]
    pub const fn word(self) -> &'static str {
        State::from_reported(self).word()
    }
}

impl State {
    /// `const` conversion, because [`Reported::word`] is `const` and `From` is
    /// not usable there.
    #[must_use]
    const fn from_reported(r: Reported) -> Self {
        match r {
            Reported::Healthy => Self::Healthy,
            Reported::Degraded => Self::Degraded,
            Reported::Down => Self::Down,
        }
    }
}

/// The key a health line is spelled with, on the answering channel and on the
/// notify socket alike.
///
/// **On `NOTIFY_SOCKET` its only meaning is *ask me now*** — `ADR-045` §3.
/// A push that arrives is not a health report; it is a request for one, and the
/// supervisor's next poll is the truth. The key is the same word in both places
/// because it names the same subject, not because the two are the same message.
pub const KEY: &str = "HEALTH";

/// **The reserved `LISTEN_FDNAMES` entry the answering socket arrives under** —
/// `ADR-045` §3.
///
/// The manifest does not declare it and every service is handed one. It is a
/// **handed descriptor and not a sixth idiom**: an application is given things
/// through the environment, file descriptors, stdout and the filesystem, plus
/// `NOTIFY_SOCKET` because that is `sd_notify`'s protocol, and the number a fair
/// critic counts is the number of MECHANISMS rather than the number of things
/// handed. So this reuses the one that already exists.
///
/// **It is refused as a manifest socket name** —
/// `crate::package::application::parse_socket` — because an application
/// declaring `unix:rivet-health` would collide with the entry the supervisor
/// appends, and the collision would be a descriptor the application accepts on
/// believing it is its own.
pub const CHANNEL_NAME: &str = "rivet-health";

/// Where the supervisor binds the answering socket, one per service.
///
/// Beside `crate::service::LISTEN_DIR` and `/run/rivet/notify`, because it is
/// the same kind of thing: a per-service socket on a tmpfs that cannot outlive
/// a boot.
pub const CHANNEL_DIR: &str = "/run/rivet/health";

/// This service's answering socket. The supervisor binds it, hands the
/// descriptor over, and connects to this path to ask.
#[must_use]
pub fn channel_path(service: &str) -> String {
    format!("{CHANNEL_DIR}/{service}")
}

/// **What the supervisor writes on the answering channel.**
///
/// The same words an operator types at the control plane
/// (`crate::operation::status::REQUEST`), and a test asserts they have not
/// drifted apart. `ADR-032` §3.3 item 1's argument carries here: a request of
/// one bare word would be a second command language with one word in it.
pub const REQUEST: &str = "get status\n";

/// The most an answer may be. A health line is a word and at most
/// [`CAPTION_MAX_BYTES`] of sentence; this is several times that, so a wedged or
/// hostile application cannot make the supervisor allocate.
pub const ANSWER_MAX_BYTES: usize = 512;

/// **How often the supervisor asks, in milliseconds** — `ADR-045` §3's
/// *"the supervisor decides how often to ask and how long to wait"*.
///
/// Thirty seconds. It is a judgement rather than a measurement and this
/// constant is where it is changed. The bound that matters is the one it sits
/// between: often enough that a fleet poll (`ADR-044`) never reads an answer
/// from a previous minute, rare enough that a machine with twenty services makes
/// forty connections a minute rather than four hundred.
pub const INTERVAL_MS: u64 = 30_000;

/// **How long the supervisor waits for an answer before concluding there was
/// none**, in milliseconds.
///
/// Two seconds — `KIOSK_PATIENCE`'s argument at four times the size. Both ends
/// are local and a unix socket round trip is microseconds, so this is a ceiling
/// on a stall rather than a working timeout, and the thing it is a ceiling on is
/// **the case the whole design exists for**: an application that has stopped
/// answering must be reported as not answering, and that verdict cannot be
/// reached without a bound.
pub const PATIENCE_MS: u64 = 2_000;

/// **The fold: what the MACHINE answers** — `ADR-045` §4, written out because
/// *"the worst of them"* is wrong.
///
/// Takes one entry per service: whether the operator's document marks it
/// `critical`, and what that service's state is.
///
/// # The order, and why `unknown` outranks `degraded`
///
/// ```text
/// down  >  unknown  >  degraded  >  healthy
/// ```
///
/// A critical service that cannot answer is nearer to a dead one than to a live
/// one that is telling you it is unwell. **A `degraded` service is
/// participating; an `unknown` one has stopped.**
///
/// # Non-critical services do not move this at all
///
/// A `degraded` weather probe must not make a machine `degraded` when everything
/// that matters is fine — that is what makes the machine-level answer worth
/// reading. Their states stay individually visible in `get status`; they simply
/// do not roll up.
///
/// Two consequences, stated by `ADR-045` §4 rather than discovered:
///
/// * **A machine whose only broken services are non-critical answers
///   `healthy`.** The remedy available to an operator is to mark what matters
///   `critical`, and no per-service state is hidden.
/// * **A machine with no critical services at all answers `healthy`.** An
///   operator who has told the machine that nothing matters gets the honest
///   answer that nothing it was told to watch is wrong.
#[must_use]
pub fn machine(services: impl IntoIterator<Item = (bool, State)>) -> State {
    services
        .into_iter()
        .filter(|(critical, _)| *critical)
        .map(|(_, state)| state)
        .fold(State::Healthy, |worst, state| {
            if rank(state) > rank(worst) {
                state
            } else {
                worst
            }
        })
}

/// The fold's ordering, as a number. **Defined once**, here, so that no reader
/// invents a second one — which is the whole reason `ADR-045` §4 is normative
/// text rather than a description.
const fn rank(state: State) -> u8 {
    match state {
        State::Healthy => 0,
        State::Degraded => 1,
        State::Unknown => 2,
        State::Down => 3,
    }
}

/// **The caption's cap, in BYTES of UTF-8** — `ADR-045` §5.
///
/// Long enough for *"no broadcast from GVH5075_2222 in 34 minutes"*, short
/// enough for a toast and for one line of `get status`. It is a judgement and
/// `ADR-045` §11 says so; nothing has yet rendered one.
pub const CAPTION_MAX_BYTES: usize = 120;

/// Why a health line could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// A word outside the closed vocabulary.
    UnknownState { found: String },
    /// **`unknown`, which a thing may not say about itself** — `ADR-045` §4.
    /// Refused by name rather than lumped in with a typo, because the two need
    /// different fixes: one is a misspelling, the other is a misunderstanding
    /// of whose answer `unknown` is.
    SelfReportedUnknown,
    /// Nothing before the first space.
    Empty,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownState { found } => write!(
                f,
                "`{found}` is not a health state. This build knows `healthy`, `degraded` and \
                 `down`, and the set is CLOSED (ADR-045 §4) — a new word is an amendment to that \
                 ADR, not a value a caller may invent."
            ),
            Self::SelfReportedUnknown => write!(
                f,
                "`unknown` is not something a service may say about itself (ADR-045 §4). It is \
                 what an OBSERVER concludes when nothing has been said, so a service claiming it \
                 is claiming to have not spoken while speaking. Say `degraded` if it is working \
                 badly, or `down` if it is not working."
            ),
            Self::Empty => write!(f, "a health report with no state word in it"),
        }
    }
}

/// **Cut a caption to [`CAPTION_MAX_BYTES`] on a character boundary.**
///
/// Returns the text and whether anything was removed.
///
/// **Truncated and reported, never refused** — `ADR-038` §4.4's containment,
/// which `ADR-045` §5 extends here: *a caption must never be the reason a
/// service stops*. An author who writes a long sentence gets a short one and a
/// journal line, not an outage.
#[must_use]
pub fn truncate_caption(text: &str) -> (&str, bool) {
    if text.len() <= CAPTION_MAX_BYTES {
        return (text, false);
    }
    let mut end = CAPTION_MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Replace the characters the notify protocol is built from.
///
/// That protocol is newline-separated `KEY=value`, so a caption containing a
/// newline would split its own record in two and the second half would be
/// silently dropped as a line with no `=`. **Replaced rather than refused**, for
/// the reason `operation::status::scrub` gives about a service's `reason`: a
/// sentence written for a person may contain anything a sentence contains.
fn scrub(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// **The wire form: `HEALTH=<word>[ <caption>]`.**
///
/// The state is one word from a closed set that contains no spaces, so the
/// caption is simply the rest of the line and needs no escaping or quoting —
/// which is why this is one key rather than two.
#[must_use]
pub fn render(state: Reported, caption: Option<&str>) -> String {
    match caption {
        Some(text) if !text.trim().is_empty() => {
            let (cut, _) = truncate_caption(text.trim());
            format!("{KEY}={} {}", state.word(), scrub(cut))
        }
        _ => format!("{KEY}={}", state.word()),
    }
}

/// **The value half of a health line, read back.** [`render`]'s inverse.
///
/// Takes what follows `HEALTH=`. An over-long caption is truncated here too:
/// the sender is supposed to have done it, and a receiver that trusted the
/// sender's cap would be a receiver a buggy sender can grow without limit.
///
/// # Errors
///
/// [`ParseError`] when the state word is not one a service may say.
pub fn parse(value: &str) -> Result<(Reported, Option<String>), ParseError> {
    let value = value.trim_start();
    let (word, rest) = value.split_once(' ').unwrap_or((value, ""));
    let state = match word {
        "healthy" => Reported::Healthy,
        "degraded" => Reported::Degraded,
        "down" => Reported::Down,
        "unknown" => return Err(ParseError::SelfReportedUnknown),
        "" => return Err(ParseError::Empty),
        other => {
            return Err(ParseError::UnknownState {
                found: other.to_string(),
            })
        }
    };
    let caption = rest.trim();
    if caption.is_empty() {
        return Ok((state, None));
    }
    let (cut, _) = truncate_caption(caption);
    Ok((state, Some(scrub(cut))))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The round trip, with and without the sentence.**
    #[test]
    fn what_is_rendered_is_what_is_read_back() {
        for (state, caption) in [
            (Reported::Healthy, None),
            (Reported::Degraded, Some("no broadcast from GVH5075_2222 in 34 minutes")),
            (Reported::Down, Some("the database will not open")),
        ] {
            let line = render(state, caption);
            let value = line.strip_prefix("HEALTH=").expect("the key");
            let (back, text) = parse(value).expect("a health line");
            assert_eq!(back, state, "{line}");
            assert_eq!(text.as_deref(), caption, "{line}");
        }
    }

    /// **A service may not say `unknown` about itself** — `ADR-045` §4, and
    /// the refusal names the section rather than saying *invalid*.
    #[test]
    fn a_service_cannot_claim_to_have_said_nothing() {
        assert_eq!(parse("unknown"), Err(ParseError::SelfReportedUnknown));
        let said = ParseError::SelfReportedUnknown.to_string();
        assert!(said.contains("ADR-045 §4"), "{said}");
        assert!(said.contains("OBSERVER"), "{said}");
    }

    /// **And the type says so before the parser does.** `Reported` has no
    /// `Unknown` variant, so a call site cannot express it — this test exists
    /// to fail if somebody adds one.
    #[test]
    fn the_reported_vocabulary_is_three_words_and_unknown_is_not_one() {
        for r in [Reported::Healthy, Reported::Degraded, Reported::Down] {
            assert_ne!(r.word(), "unknown");
            assert_eq!(State::from(r).word(), r.word(), "the two spellings drifted");
        }
    }

    /// **A word outside the set is refused, and the refusal says the set is
    /// closed** — otherwise a reader concludes their build is old.
    #[test]
    fn a_word_outside_the_vocabulary_is_refused_and_says_the_set_is_closed() {
        let e = parse("wobbly").expect_err("a refusal");
        assert_eq!(e, ParseError::UnknownState { found: "wobbly".to_string() });
        let said = e.to_string();
        assert!(said.contains("CLOSED"), "{said}");
        assert!(said.contains("ADR-045"), "{said}");
    }

    /// **A caption is cut on a character boundary, never through one.**
    ///
    /// The naive `&text[..120]` panics here rather than truncating, so this is
    /// a test about not crashing on a sentence somebody typed in another
    /// language.
    #[test]
    fn an_over_long_caption_is_cut_on_a_character_boundary() {
        let long = "é".repeat(200);
        let (cut, did) = truncate_caption(&long);
        assert!(did);
        assert!(cut.len() <= CAPTION_MAX_BYTES);
        // The whole point: still valid UTF-8 and a prefix of the original.
        assert!(long.starts_with(cut));
        assert!(!cut.is_empty());
    }

    /// **A caption never stops a service** — `ADR-045` §5, `ADR-038` §4.4's
    /// containment. Over-length is a shorter sentence, not a refusal.
    #[test]
    fn an_over_long_caption_is_truncated_rather_than_refused() {
        let line = render(Reported::Degraded, Some(&"x".repeat(500)));
        let value = line.strip_prefix("HEALTH=").expect("the key");
        let (state, caption) = parse(value).expect("not a refusal");
        assert_eq!(state, Reported::Degraded);
        assert_eq!(caption.expect("a caption").len(), CAPTION_MAX_BYTES);
    }

    /// **A newline in a caption cannot split its own record.**
    ///
    /// The notify socket is newline-separated `KEY=value`; the half after a
    /// newline would be dropped as a line with no `=`, taking the sentence's
    /// meaning with it.
    #[test]
    fn a_newline_in_a_caption_cannot_split_the_record() {
        let line = render(Reported::Down, Some("first\nsecond\rthird"));
        assert_eq!(line.lines().count(), 1, "{line:?}");
        let value = line.strip_prefix("HEALTH=").expect("the key");
        let (_, caption) = parse(value).expect("a health line");
        assert_eq!(caption.as_deref(), Some("first second third"));
    }

    /// An empty or whitespace-only caption is no caption, not an empty one —
    /// so a reader never has to decide what a blank sentence means.
    #[test]
    fn a_blank_caption_is_absent_rather_than_empty() {
        assert_eq!(render(Reported::Healthy, Some("   ")), "HEALTH=healthy");
        assert_eq!(parse("healthy   ").expect("a line").1, None);
    }

    /// **The whole ordering of `ADR-045` §4, one pair at a time.**
    ///
    /// Written as pairs rather than as a sorted list, because the claim the ADR
    /// makes is about which of two states wins — and `unknown` beating
    /// `degraded` is the one a reader is most likely to get backwards.
    #[test]
    fn the_fold_orders_down_above_unknown_above_degraded_above_healthy() {
        use State::{Degraded, Down, Healthy, Unknown};
        for (a, b, worse) in [
            (Down, Unknown, Down),
            (Down, Degraded, Down),
            (Down, Healthy, Down),
            (Unknown, Degraded, Unknown),
            (Unknown, Healthy, Unknown),
            (Degraded, Healthy, Degraded),
        ] {
            assert_eq!(machine([(true, a), (true, b)]), worse, "{a:?} vs {b:?}");
            assert_eq!(machine([(true, b), (true, a)]), worse, "order must not matter");
        }
    }

    /// **`unknown` ranks worse than `degraded`**, alone, because it is the
    /// clause `ADR-045` §4 argues for and the one an ordering change would
    /// silently invert. *A `degraded` service is participating; an `unknown` one
    /// has stopped.*
    #[test]
    fn a_critical_service_that_cannot_answer_is_worse_than_one_saying_it_is_unwell() {
        assert_eq!(
            machine([(true, State::Degraded), (true, State::Unknown)]),
            State::Unknown
        );
    }

    /// **Non-critical services do not move the machine's answer** — §4.4, and
    /// the first of its two stated consequences.
    #[test]
    fn a_machine_whose_only_broken_services_are_not_critical_answers_healthy() {
        assert_eq!(
            machine([
                (false, State::Down),
                (false, State::Unknown),
                (false, State::Degraded),
                (true, State::Healthy),
            ]),
            State::Healthy
        );
    }

    /// The second consequence: **nothing marked critical is `healthy`, not
    /// `unknown`.** An operator who has told the machine that nothing matters
    /// gets the honest answer that nothing it was told to watch is wrong.
    #[test]
    fn a_machine_with_no_critical_services_at_all_answers_healthy() {
        assert_eq!(machine([]), State::Healthy);
        assert_eq!(machine([(false, State::Down)]), State::Healthy);
    }

    /// **The request is the operator's own words**, not a private one-word
    /// protocol — `ADR-032` §3.3 item 1, carried to this channel. If the two
    /// ever drift, an application written against one is answering the other.
    #[test]
    fn the_answering_channel_asks_the_same_question_the_control_plane_does() {
        assert_eq!(REQUEST, crate::operation::status::REQUEST);
    }

    /// **The reserved name is refused as a manifest socket name**, or an
    /// application could declare `unix:rivet-health` and be handed two
    /// descriptors that both claim to be it.
    #[test]
    fn the_reserved_channel_name_cannot_be_declared_by_a_manifest() {
        let manifest = |socket: &str| {
            format!(
                "kind = \"daemon\"\nrestart = \"never\"\nready = \"none\"\nnetwork = \"none\"\n\
                 state = \"private\"\nlisten = [\"unix:{socket}\"]\nprovides = []\n\
                 connects = []\nlibrary = []\ndevices = []\n"
            )
        };
        // The control: an ordinary name parses, so a refusal below is about the
        // NAME rather than about the manifest being malformed.
        assert!(
            crate::package::application::Manifest::parse(manifest("api").as_bytes()).is_ok(),
            "an ordinary unix socket name must still parse"
        );
        assert!(
            crate::package::application::Manifest::parse(manifest(CHANNEL_NAME).as_bytes())
                .is_err(),
            "a manifest may not claim the reserved health channel name"
        );
    }

    /// The bound on an answer is comfortably above the longest legitimate one,
    /// so the cap can never be what truncates a caption.
    #[test]
    fn the_answer_bound_is_larger_than_the_longest_legitimate_answer() {
        let longest = render(Reported::Degraded, Some(&"x".repeat(CAPTION_MAX_BYTES)));
        assert!(longest.len() < ANSWER_MAX_BYTES, "{}", longest.len());
    }
}
