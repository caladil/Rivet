//! **What a machine tells an application about itself** — the facts call,
//! ruled 2026-09-19 (`.rulings/` R-199 D4).
//!
//! An application asks; the supervisor answers, from memory it already holds.
//! `ADR-057` is the decision and the argument; this module is the fact set and
//! the wire form, both halves, in one place.
//!
//! # Why a call and not a file
//!
//! An earlier design put a world-readable projection under `/run/rivet`,
//! rewritten every second. It was refused on the shape: a projection exists for
//! a C library that reads a fixed path and cannot be told otherwise, and
//! `ADR-039` already calls `/etc` a compatibility surface rather than a
//! configuration one. A Rivet application is the opposite case — `SPEC-03` §2
//! hands it what it needs.
//!
//! Two properties follow from the shape, and both are why it is worth having:
//!
//! * **It costs nothing when nobody asks.** A file is serialized on a cadence
//!   whether or not anything is watching. This is paid for at the asking, and
//!   a machine with no dashboard selected pays nothing at all.
//! * **Silence is the signal.** A bounded call that does not answer says *the
//!   supervisor has stopped*, in two seconds, unambiguously. A stale file
//!   cannot: one written by a wedged supervisor is byte-identical to one
//!   written by a healthy supervisor that has not ticked yet, and telling them
//!   apart needs a threshold somebody has to pick.
//!
//! # It is a DATAGRAM, and that is not an implementation detail
//!
//! `crate::kiosk` and `crate::operation::status` are both stream sockets the
//! supervisor `accept`s on, and both are correct for what they do. This one is
//! a bound `UnixDatagram`, and the reason is the failure this surface exists to
//! make visible.
//!
//! **A stream socket's listen backlog is what wedged this supervisor.** The
//! 2026-09-18 incident was caught under instrument parked in
//! `unix_wait_for_peer` — where a task sleeps when a listening peer's backlog
//! is full and nothing is accepting — and `.rulings/` R-195 D5's invariant is
//! that a supervisor a service can block is a design defect. An instrument
//! built on the mechanism that causes the failure would go blind at the moment
//! it mattered, in both directions: the application's `connect` would block
//! with no bound, and the supervisor's `accept` would be one more queue to
//! service.
//!
//! A datagram has no backlog and no connection. The caller writes one line and
//! waits with a deadline it set; the supervisor reads one line and writes one
//! answer, both without blocking, with no per-caller state to keep between
//! wakeups. **There is nothing here a caller can hold the supervisor with**,
//! which is the property that lets this socket be open to every application on
//! the machine rather than chowned to one.
//!
//! # What crosses, and what deliberately does not
//!
//! [`Facts`] is what the supervisor **already knows** — the fold it composes
//! for `crate::operation::Status` every time an operator asks `get status`,
//! plus the two strings it read once at boot. Nothing here makes it open a
//! file, walk a directory or ask a service a question. `ADR-057` §4 is the
//! rule and its consequences: no thermal reading, no storage figures, no
//! journal records, and no uptime.
//!
//! # Every fact carries how old it is
//!
//! An age rather than an instant, computed by the supervisor as it answers.
//! Nothing absolute crosses, so the two processes need no agreement about a
//! clock and no skew can make an age negative. The caller adds its own elapsed
//! time on top, which is what turns *last updated 4 s ago* into *6 minutes
//! ago* without anything having to arrive to say so.
//!
//! Three classes of fact, and they age differently:
//!
//! * **Fixed for the life of the boot** — [`Facts::hostname`],
//!   [`Facts::version`], [`Facts::slot`]. These carry no age because they
//!   cannot have one, and stamping them with a zero would invite a reader to
//!   render *0 s ago* beside a fact that has been true since the machine came
//!   up.
//! * **As of the supervisor's last pass** — everything else on [`Facts`],
//!   under [`Facts::view_age_ms`].
//! * **As of the last health poll, per service** —
//!   [`ServiceFact::health_age_ms`]. The supervisor asks every thirty seconds
//!   (`ADR-045` §3), so this is the field most likely to be drawn as live when
//!   it is half a minute old, and it carries its own age for that reason.

/// **Where the supervisor listens.** Beside `crate::kiosk::SOCKET_PATH` and
/// `crate::operation::QUERY_SOCKET_PATH`, and deliberately neither of them.
///
/// `/run/rivet` is root-owned and already in every application's ruleset with
/// read and write and **no create** (`crate::confinement::RUNTIME_DIR`), so an
/// application can talk on this socket and cannot leave one of its own beside
/// it. `/run` is a tmpfs, so it cannot outlive the boot — which is correct for
/// a surface whose whole subject is what is true right now.
pub const SOCKET_PATH: &str = "/run/rivet/facts";

/// **What a caller sends.** One line, and the socket answers nothing else.
///
/// It is a whole operation rather than a bare word for
/// `crate::operation::status::REQUEST`'s reason: a request of `facts` alone
/// would be a second command language with one word in it. `get facts` is a
/// Rivet command, and the day a second question is ruled across this socket it
/// needs no new grammar.
pub const REQUEST: &str = "get facts\n";

/// The most a request may be. A datagram longer than this is dropped unread:
/// the one request this build answers is eleven bytes, so anything near the
/// bound is a caller that is confused or hostile and both are answered the same
/// way.
pub const REQUEST_MAX_BYTES: usize = 256;

/// **The most an answer may be**, and the number is a datagram's constraint
/// rather than a taste.
///
/// An answer is one datagram, so it must fit in one: a Unix datagram socket's
/// default send buffer on Linux is far above this, and a machine whose fact set
/// exceeded it would silently lose the whole answer rather than part of it.
/// Sixty-four kilobytes is about six hundred services at the longest name
/// `crate::service::ServiceName` admits, which is far past any Rivet — and
/// [`Facts::render`] stops before the bound and says so with
/// [`Facts::truncated`] rather than emitting something that cannot be sent.
pub const ANSWER_MAX_BYTES: usize = 64 * 1024;

/// **The answer's own version, and the only thing in this protocol a reader
/// must check before trusting the rest.**
///
/// It is on the FIRST line, not the last, so a reader that cannot understand
/// the answer knows it before it has parsed a field. `.rulings/` R-201 D1's
/// rule about a list's order being a weak contract is why this is a number
/// carried in the answer rather than a shape two builds are assumed to agree
/// about.
pub const PROTOCOL: u32 = 1;

/// The word an absent value is written as, in a format with no quoting.
///
/// A service name, a hostname, a slot and a version string can none of them be
/// this — `crate::service::ServiceName` and `crate::hostname` both require a
/// leading alphanumeric — so it cannot be mistaken for a value.
const NOTHING: &str = "-";

/// **What one service looks like to a screen.**
///
/// A narrower thing than `crate::operation::ServiceStatus`, and the narrowing
/// is `ADR-057` §4: this carries what a tile draws and not what an operator
/// reads. The two fields left behind are named in that section — the
/// supervisor's `reason` sentence and the service's own health caption — and
/// both are prose one program wrote for a person, which is the control plane's
/// to carry and not a kiosk screen's in a room with other people in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFact {
    /// Its name, as the supervisor holds it.
    pub name: String,
    /// What the document and the definition together say (`ADR-017` §6).
    pub enabled: bool,
    /// The supervisor's own word — `idle`, `started`, `done`, `failed`.
    /// **Carried through unchanged** (`AGENTS.md` §5): a second vocabulary
    /// here would be a second answer.
    pub state: String,
    /// `None` unless it is running. Absent rather than zero, because zero is a
    /// pid.
    pub pid: Option<u32>,
    /// **How many times it has been RESTARTED** — `0` for a service that
    /// started first time and is still running. The same counter
    /// `crate::operation::ServiceStatus::restarts` reports, and named for what
    /// it counts rather than for the journal's neighbouring `attempt` column,
    /// which is this number plus one.
    pub restarts: u32,
    /// Whether it has declared readiness over `NOTIFY_SOCKET` (`ADR-017`
    /// §3.3). **`false` for a service that never declares one**, which is most
    /// of them — this is not a health check.
    pub ready: bool,
    /// One of the four words in `crate::health::State` (`ADR-045` §4), and
    /// `unknown` until there is a successful answer on record.
    pub health: crate::health::State,
    /// **How old that health word is, in milliseconds**, and `None` when no
    /// answer has arrived to age.
    ///
    /// The supervisor polls on a thirty-second cadence, so a tile that drew
    /// this word without its age would be presenting a reading up to half a
    /// minute old as the present. `None` and a `health` of `unknown` arrive
    /// together and mean *nothing has ever answered*; a `None` beside any
    /// other word would be a supervisor that recorded an answer without
    /// recording when, and this build cannot produce one.
    pub health_age_ms: Option<u64>,
}

/// **Everything a machine hands an application about itself**, in one answer.
///
/// Composed from `crate::operation::Status` — the same fold `get status`
/// returns — so a dashboard and an operator asking over the control plane
/// cannot disagree about what is running. `ADR-011` in the ordinary case:
/// there is one derivation of these facts and this is a second *reader* of it,
/// never a second author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The answer's own version. See [`PROTOCOL`].
    pub protocol: u32,
    /// **This machine's name**, as `crate::hostname` settled it at boot, and
    /// `None` on a machine that has none.
    ///
    /// Fixed for the life of the boot, so it carries no age.
    pub hostname: Option<String>,
    /// **The running image's own version** — the same string `rivet --version`
    /// carries (`ADR-027` §10). `None` only on a supervisor built without one,
    /// which this build cannot be.
    ///
    /// Fixed for the life of the boot.
    pub version: Option<String>,
    /// **Which A/B slot is running**, `a` or `b`, and `None` on a machine with
    /// no `/system` in its mount table — a development host or the emulator,
    /// which is not a fault (`ADR-018` §5).
    ///
    /// Read from the kernel's mount table at boot and never from hashing a
    /// binary, which two slots holding one build would make a coin toss.
    /// Fixed for the life of the boot.
    pub slot: Option<String>,
    /// What the supervisor is doing as a whole — `running`, `shutting-down`
    /// or `finished` (`ADR-032` §8); a build older than that names `running`
    /// `converging`. Its own word, carried through.
    pub phase: String,
    /// **Whether this machine is doing its job** — `ADR-045` §4's fold over
    /// the services the document marks `critical`, computed by the supervisor
    /// and carried rather than folded again here.
    pub machine: crate::health::State,
    /// What this machine is for — `kiosk`, `headless` or `unknown`
    /// (`crate::operation::status::MODE_KIOSK` and its two siblings).
    ///
    /// **The three-valued word, not a boolean**: `unknown` is the answer for a
    /// machine whose document could not be read, and a screen that rendered
    /// that as `headless` would be stating something confidently about a
    /// document nobody parsed.
    pub mode: String,
    /// **How old the supervisor's own view was when it answered**, in
    /// milliseconds — the time since the convergence pass that last refreshed
    /// everything above except the three boot-fixed strings.
    ///
    /// A healthy supervisor answers a small number here. A large one is a
    /// supervisor that is still answering and has stopped turning, which is a
    /// state worth being able to draw and is not the same thing as silence.
    pub view_age_ms: u64,
    /// One entry per service the supervisor holds, image-defined and installed
    /// alike, in the order it holds them.
    pub services: Vec<ServiceFact>,
    /// **How many services were left out because the answer reached
    /// [`ANSWER_MAX_BYTES`]**, and `0` in every ordinary case.
    ///
    /// Reported rather than silently dropped: a dashboard that drew eleven
    /// tiles for a machine running four hundred services must be able to say
    /// that it is drawing part of the answer. A datagram that did not fit
    /// would otherwise arrive as no answer at all, which reads as a wedge.
    pub truncated: u32,
}

/// Why an answer could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactsError {
    /// The first line is not a `facts` line, so this is not an answer.
    NotAnAnswer,
    /// The answer came from a supervisor speaking a protocol this build does
    /// not know. **Refused rather than parsed hopefully** — a reader that
    /// guessed would draw a number from the wrong field.
    Protocol { found: u32, wanted: u32 },
    /// The answer stopped before its `end` line, which is what a datagram that
    /// was cut short looks like from this side.
    Unterminated,
    /// A field that should have been a number, a boolean or a closed word and
    /// was not.
    Value {
        line: usize,
        field: &'static str,
        found: String,
    },
    /// A line with the wrong number of fields for its kind.
    Fields {
        line: usize,
        kind: String,
        found: usize,
        wanted: usize,
    },
}

impl core::fmt::Display for FactsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnAnswer => write!(
                f,
                "this does not begin with a `facts` line, so it is not an answer from a Rivet \
                 supervisor"
            ),
            Self::Protocol { found, wanted } => write!(
                f,
                "the supervisor answered facts protocol {found} and this build reads {wanted}. \
                 The machine is newer than the application running on it."
            ),
            Self::Unterminated => write!(
                f,
                "the answer carries no `end` line, so it was cut short. Every whole answer ends \
                 with one, precisely so that a partial one cannot be read as a complete machine."
            ),
            Self::Value { line, field, found } => write!(
                f,
                "line {line}'s `{field}` is `{found}`, which is not a value it may take."
            ),
            Self::Fields {
                line,
                kind,
                found,
                wanted,
            } => write!(
                f,
                "line {line} is a `{kind}` with {found} fields and this build wants {wanted}."
            ),
        }
    }
}

/// What a request could not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    /// Anything that is not [`REQUEST`].
    NotTheRequest,
    /// Longer than [`REQUEST_MAX_BYTES`], or not text.
    TooLong,
}

impl core::fmt::Display for RequestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotTheRequest => write!(
                f,
                "this socket answers `get facts` and nothing else. It is deliberately one \
                 question wide: the operator's control surface is ADR-032 §3's query socket \
                 and an application is not given that"
            ),
            Self::TooLong => write!(f, "a request is at most {REQUEST_MAX_BYTES} bytes of text"),
        }
    }
}

/// **The one request this socket takes**, read off the wire.
///
/// There is one question, so this returns nothing on success: a `Request` type
/// with a single variant would be a vocabulary pretending to have a choice in
/// it. The day a second question is ruled, it becomes one.
///
/// # Errors
///
/// [`RequestError`] when the line is not [`REQUEST`].
pub fn read_request(line: &str) -> Result<(), RequestError> {
    if line.len() > REQUEST_MAX_BYTES {
        return Err(RequestError::TooLong);
    }
    if line.trim_end_matches(['\r', '\n']) == REQUEST.trim_end() {
        Ok(())
    } else {
        Err(RequestError::NotTheRequest)
    }
}

const FACTS: &str = "facts";
const HOST: &str = "host";
const VERSION: &str = "version";
const SLOT: &str = "slot";
const PHASE: &str = "phase";
const MACHINE: &str = "machine";
const MODE: &str = "mode";
const VIEW: &str = "view";
const SVC: &str = "svc";
const END: &str = "end";

/// How many fields a `svc` line carries after its kind.
const SVC_FIELDS: usize = 8;

impl Facts {
    /// **Write the answer.** The supervisor's half of one definition of one
    /// format; [`Facts::parse`] is the other, and the round trip is asserted
    /// below.
    ///
    /// **Bounded, and it stops on a service boundary.** A `svc` line is only
    /// written if the whole line and the `end` line after it still fit in
    /// [`ANSWER_MAX_BYTES`]; the rest are counted into [`Facts::truncated`].
    /// A renderer that produced an over-long answer would produce a datagram
    /// that cannot be sent, and no answer at all reads as a wedged supervisor
    /// — which is the one thing this surface must never fake.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{FACTS} {}\n", self.protocol));
        out.push_str(&format!(
            "{HOST} {}\n",
            self.hostname.as_deref().unwrap_or(NOTHING)
        ));
        out.push_str(&format!(
            "{VERSION} {}\n",
            self.version.as_deref().unwrap_or(NOTHING)
        ));
        out.push_str(&format!(
            "{SLOT} {}\n",
            self.slot.as_deref().unwrap_or(NOTHING)
        ));
        out.push_str(&format!("{PHASE} {}\n", token(&self.phase)));
        out.push_str(&format!("{MACHINE} {}\n", self.machine.word()));
        out.push_str(&format!("{MODE} {}\n", token(&self.mode)));
        out.push_str(&format!("{VIEW} {}\n", self.view_age_ms));

        let mut dropped = 0_u32;
        for svc in &self.services {
            let line = format!(
                "{SVC} {} {} {} {} {} {} {} {}\n",
                token(&svc.name),
                word(svc.enabled),
                token(&svc.state),
                svc.pid
                    .map_or_else(|| NOTHING.to_owned(), |p| p.to_string()),
                svc.restarts,
                word(svc.ready),
                svc.health.word(),
                svc.health_age_ms
                    .map_or_else(|| NOTHING.to_owned(), |a| a.to_string()),
            );
            // The `end` line is part of the budget, because an answer without
            // one is refused by every reader and is therefore no answer.
            let tail = format!("{END} {} {dropped}\n", self.services.len());
            if out.len() + line.len() + tail.len() + 16 > ANSWER_MAX_BYTES {
                dropped = dropped.saturating_add(1);
                continue;
            }
            out.push_str(&line);
        }
        out.push_str(&format!(
            "{END} {} {}\n",
            self.services.len(),
            dropped.max(self.truncated)
        ));
        out
    }

    /// **Read an answer**, the application's half.
    ///
    /// # Errors
    ///
    /// [`FactsError`] for an answer that is not one, is from a supervisor
    /// speaking a protocol this build does not know, was cut short, or carries
    /// a field that is not a value it may take.
    pub fn parse(text: &str) -> Result<Self, FactsError> {
        let mut out = Self {
            protocol: PROTOCOL,
            hostname: None,
            version: None,
            slot: None,
            phase: String::new(),
            machine: crate::health::State::Unknown,
            mode: crate::operation::status::MODE_UNKNOWN.to_owned(),
            view_age_ms: 0,
            services: Vec::new(),
            truncated: 0,
        };
        let mut saw_end = false;
        let mut first = true;

        for (n, line) in text.lines().enumerate() {
            let number = n + 1;
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split(' ');
            let kind = fields.next().unwrap_or("");
            if first {
                if kind != FACTS {
                    return Err(FactsError::NotAnAnswer);
                }
                let found = number_field(number, "protocol", fields.next())?;
                let found = u32::try_from(found).unwrap_or(u32::MAX);
                if found != PROTOCOL {
                    return Err(FactsError::Protocol {
                        found,
                        wanted: PROTOCOL,
                    });
                }
                out.protocol = found;
                first = false;
                continue;
            }
            match kind {
                HOST => out.hostname = optional(fields.next()),
                VERSION => out.version = optional(fields.next()),
                SLOT => out.slot = optional(fields.next()),
                PHASE => out.phase = fields.next().unwrap_or("").to_owned(),
                MACHINE => out.machine = health_word(number, "machine", fields.next())?,
                MODE => {
                    out.mode = fields
                        .next()
                        .unwrap_or(crate::operation::status::MODE_UNKNOWN)
                        .to_owned();
                }
                VIEW => out.view_age_ms = number_field(number, "view", fields.next())?,
                SVC => {
                    let rest: Vec<&str> = fields.collect();
                    if rest.len() != SVC_FIELDS {
                        return Err(FactsError::Fields {
                            line: number,
                            kind: SVC.to_owned(),
                            found: rest.len(),
                            wanted: SVC_FIELDS,
                        });
                    }
                    out.services.push(ServiceFact {
                        name: rest[0].to_owned(),
                        enabled: boolean(number, "enabled", rest[1])?,
                        state: rest[2].to_owned(),
                        pid: match rest[3] {
                            NOTHING => None,
                            text => Some(
                                u32::try_from(parse_u64(number, "pid", text)?)
                                    .map_err(|_| value(number, "pid", text))?,
                            ),
                        },
                        restarts: u32::try_from(parse_u64(number, "restarts", rest[4])?)
                            .unwrap_or(u32::MAX),
                        ready: boolean(number, "ready", rest[5])?,
                        health: health_word(number, "health", Some(rest[6]))?,
                        health_age_ms: match rest[7] {
                            NOTHING => None,
                            text => Some(parse_u64(number, "health_age_ms", text)?),
                        },
                    });
                }
                END => {
                    // The first field is how many services the supervisor
                    // holds and the second is how many did not fit. The count
                    // is not compared against the rows: `truncated` already
                    // says they disagree, and a reader that refused the answer
                    // would have thrown away the part that arrived.
                    let _held = fields.next();
                    out.truncated = u32::try_from(number_field(number, "truncated", fields.next())?)
                        .unwrap_or(u32::MAX);
                    saw_end = true;
                }
                // **An unknown line is skipped, where an unknown protocol is
                // refused**, and the difference is deliberate. The protocol
                // number is what says *this answer is not shaped the way you
                // think*; within one protocol, a line kind this build does not
                // know can only be a field added by a supervisor that still
                // promises everything above, and refusing the whole answer
                // over it would take a working dashboard down at an upgrade.
                _ => {}
            }
        }
        if first {
            return Err(FactsError::NotAnAnswer);
        }
        if !saw_end {
            return Err(FactsError::Unterminated);
        }
        Ok(out)
    }
}

fn value(line: usize, field: &'static str, found: &str) -> FactsError {
    FactsError::Value {
        line,
        field,
        found: found.to_owned(),
    }
}

fn parse_u64(line: usize, field: &'static str, text: &str) -> Result<u64, FactsError> {
    text.parse().map_err(|_| value(line, field, text))
}

fn number_field(line: usize, field: &'static str, text: Option<&str>) -> Result<u64, FactsError> {
    parse_u64(line, field, text.unwrap_or(""))
}

fn boolean(line: usize, field: &'static str, text: &str) -> Result<bool, FactsError> {
    match text {
        "yes" => Ok(true),
        "no" => Ok(false),
        other => Err(value(line, field, other)),
    }
}

fn health_word(
    line: usize,
    field: &'static str,
    text: Option<&str>,
) -> Result<crate::health::State, FactsError> {
    use crate::health::State;
    match text.unwrap_or("") {
        "healthy" => Ok(State::Healthy),
        "degraded" => Ok(State::Degraded),
        "down" => Ok(State::Down),
        "unknown" => Ok(State::Unknown),
        other => Err(value(line, field, other)),
    }
}

const fn word(yes: bool) -> &'static str {
    if yes { "yes" } else { "no" }
}

fn optional(text: Option<&str>) -> Option<String> {
    match text {
        None | Some(NOTHING) | Some("") => None,
        Some(value) => Some(value.to_owned()),
    }
}

/// **Make a value one token**, because this format has no quoting.
///
/// Every field that reaches here is a single token by construction — a service
/// name, a state word, a mode word — so this changes nothing on any input this
/// machine produces. It is here because a value that ever *did* contain a space
/// would otherwise shift every field after it on the line, and a reader would
/// take one field for another rather than fail. `crate::kiosk`'s
/// `every_mode_word_is_a_single_token` is the guard on the other side of the
/// same trade.
fn token(text: &str) -> String {
    if text.is_empty() {
        return NOTHING.to_owned();
    }
    text.replace([' ', '\t', '\n', '\r'], "_")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use crate::health::State;

    fn svc(name: &str) -> ServiceFact {
        ServiceFact {
            name: name.to_owned(),
            enabled: true,
            state: "started".to_owned(),
            pid: Some(678),
            restarts: 0,
            ready: false,
            health: State::Healthy,
            health_age_ms: Some(4_120),
        }
    }

    fn machine() -> Facts {
        Facts {
            protocol: PROTOCOL,
            hostname: Some("rivet-06f5c".to_owned()),
            version: Some("0.1.91".to_owned()),
            slot: Some("b".to_owned()),
            phase: "converging".to_owned(),
            machine: State::Healthy,
            mode: "kiosk".to_owned(),
            view_age_ms: 212,
            services: vec![svc("aura-controller"), svc("rivet-proxyd")],
            truncated: 0,
        }
    }

    #[test]
    fn an_answer_round_trips() {
        let facts = machine();
        assert_eq!(Facts::parse(&facts.render()).expect("an answer"), facts);
    }

    /// **Every absence round-trips as an absence**, and not as the string `-`.
    /// A machine off a card has no slot, a machine with no name has no
    /// hostname, and a service nothing has ever asked has no health age.
    #[test]
    fn the_absences_are_absences_and_not_the_word_for_one() {
        let mut facts = machine();
        facts.hostname = None;
        facts.version = None;
        facts.slot = None;
        facts.services[0].pid = None;
        facts.services[0].health = State::Unknown;
        facts.services[0].health_age_ms = None;
        let read = Facts::parse(&facts.render()).expect("an answer");
        assert_eq!(read, facts);
        assert_eq!(read.hostname, None);
        assert_eq!(read.services[0].health_age_ms, None);
    }

    /// **A supervisor newer than the application is refused, not guessed at.**
    /// A reader that parsed a protocol it did not know would draw a number
    /// from whichever field happened to be in that position.
    #[test]
    fn an_answer_from_a_protocol_this_build_does_not_know_is_refused() {
        let text = machine().render().replacen("facts 1", "facts 2", 1);
        assert_eq!(
            Facts::parse(&text),
            Err(FactsError::Protocol {
                found: 2,
                wanted: PROTOCOL
            })
        );
    }

    /// **A cut-short answer is refused rather than read as a small machine.**
    ///
    /// This is the failure the `end` line exists for: a datagram truncated
    /// after four services parses perfectly as a machine running four
    /// services, and a dashboard would draw it without a hint that anything
    /// was missing.
    #[test]
    fn an_answer_without_its_end_line_is_refused() {
        let whole = machine().render();
        let cut = whole
            .lines()
            .take(whole.lines().count() - 1)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(Facts::parse(&cut), Err(FactsError::Unterminated));
    }

    #[test]
    fn something_that_is_not_an_answer_is_not_read_as_one() {
        assert_eq!(Facts::parse(""), Err(FactsError::NotAnAnswer));
        assert_eq!(
            Facts::parse("phase converging\nend 0 0\n"),
            Err(FactsError::NotAnAnswer)
        );
    }

    /// **A line this build does not know does not take the answer down**, so a
    /// dashboard keeps working across a supervisor that grew a field. The
    /// protocol number is what refuses a shape change; a new line within one
    /// protocol is an addition, by definition.
    #[test]
    fn a_line_kind_from_a_later_build_is_skipped_and_the_rest_still_reads() {
        let text = machine().render().replacen(
            "end ",
            "thermal 54200 no\nend ",
            1,
        );
        let read = Facts::parse(&text).expect("an answer");
        assert_eq!(read.services.len(), 2);
        assert_eq!(read.hostname.as_deref(), Some("rivet-06f5c"));
    }

    /// **One question, and the socket refuses every other line.** The
    /// narrowness is the point, so it is asserted rather than assumed: each of
    /// these is a verb the operator's socket has or the kiosk socket has, and
    /// none of them is reachable from here.
    #[test]
    fn nothing_but_the_one_request_is_accepted() {
        assert_eq!(read_request(REQUEST), Ok(()));
        assert_eq!(read_request("get facts"), Ok(()));
        for line in [
            "get status\n",
            "facts\n",
            "get facts please\n",
            "select aura-renderer\n",
            "restart\n",
            "apply service-restart rivet-mcpd\n",
            "\n",
            "GET FACTS\n",
        ] {
            assert_eq!(
                read_request(line),
                Err(RequestError::NotTheRequest),
                "{line:?} was accepted"
            );
        }
        assert_eq!(
            read_request(&"x".repeat(REQUEST_MAX_BYTES + 1)),
            Err(RequestError::TooLong)
        );
    }

    /// **An answer that would not fit in a datagram is cut on a service
    /// boundary and SAYS SO**, rather than being sent whole and lost.
    ///
    /// The count is what makes the truncation legible: a dashboard drawing
    /// part of a machine must be able to say it is drawing part of one.
    #[test]
    fn an_over_large_fact_set_is_bounded_and_reports_what_it_dropped() {
        let mut facts = machine();
        facts.services = (0..4000)
            .map(|i| svc(&format!("application-{i:04}")))
            .collect();
        let rendered = facts.render();
        assert!(
            rendered.len() <= ANSWER_MAX_BYTES,
            "an answer of {} bytes cannot be sent as one datagram",
            rendered.len()
        );
        let read = Facts::parse(&rendered).expect("a bounded answer still parses");
        assert!(read.truncated > 0, "the drop is silent");
        assert_eq!(
            read.services.len() + read.truncated as usize,
            4000,
            "every service is either drawn or counted"
        );
    }

    /// **An ordinary machine is nowhere near the bound**, which is what makes
    /// the test above a guard rather than the ordinary path.
    #[test]
    fn a_real_machines_answer_is_one_small_datagram() {
        let mut facts = machine();
        facts.services = (0..24)
            .map(|i| svc(&format!("application-{i:02}")))
            .collect();
        let rendered = facts.render();
        assert!(rendered.len() < 4096, "{} bytes", rendered.len());
        assert_eq!(Facts::parse(&rendered).expect("an answer").truncated, 0);
    }

    /// **The socket is not the operator's and is not the launcher's**, and a
    /// build that pointed two of them at one path would hand an application a
    /// surface nobody granted it.
    #[test]
    fn the_facts_socket_is_its_own_path() {
        assert_ne!(SOCKET_PATH, crate::operation::QUERY_SOCKET_PATH);
        assert_ne!(SOCKET_PATH, crate::kiosk::SOCKET_PATH);
        assert_ne!(SOCKET_PATH, crate::journal::INGEST_SOCKET_PATH);
        assert!(
            SOCKET_PATH.starts_with(&format!("{}/", crate::confinement::RUNTIME_DIR)),
            "a socket outside the runtime directory is one no application's ruleset reaches"
        );
    }

    /// **A value with a space in it cannot shift the fields after it.**
    ///
    /// Nothing this machine produces contains one — a service name and a state
    /// word are single tokens by construction — so this asserts the behaviour
    /// of the format rather than of today's data.
    #[test]
    fn a_field_that_somehow_held_a_space_cannot_move_its_neighbours() {
        let mut facts = machine();
        facts.phase = "shutting down".to_owned();
        facts.services[0].state = "not a word".to_owned();
        let read = Facts::parse(&facts.render()).expect("an answer");
        assert_eq!(read.phase, "shutting_down");
        assert_eq!(read.services[0].pid, Some(678));
        assert_eq!(read.services.len(), 2);
    }

    /// **The three boot-fixed facts carry no age and the rest do.**
    ///
    /// Asserted because it is a decision rather than an omission: stamping a
    /// hostname with `0 ms` would invite a screen to render *updated 0 s ago*
    /// beside a fact that has been true since the machine came up.
    #[test]
    fn the_ages_are_on_the_facts_that_can_have_one() {
        let facts = machine();
        let rendered = facts.render();
        for fixed in ["host rivet-06f5c", "version 0.1.91", "slot b"] {
            let line = rendered
                .lines()
                .find(|l| l.starts_with(fixed))
                .expect("the line is there");
            assert_eq!(
                line.split(' ').count(),
                2,
                "{line} carries an age, and a fact fixed for the boot cannot have one"
            );
        }
        assert!(rendered.contains("view 212"));
        assert!(rendered.contains(" healthy 4120"));
    }
}
