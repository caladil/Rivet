//! `rivet apply service-restart <name> confirm` — **ask the supervisor to
//! discard one service's current state and converge again.** `ADR-017` §6.3.2,
//! built 2026-08-29.
//!
//! `PLAN.md` §102.3 item 4. This is **the second thing ever to cross the
//! supervisor's socket**, and the first that changes anything. `ADR-032` §3.2
//! wrote the sentence this module finishes:
//!
//! > It is a whole operation and not the bare subject … a request of
//! > `get status` is a Rivet command, and **the day a second one is ruled across
//! > it needs no new grammar.**
//!
//! So there is no new grammar here.
//! [`Request`](service_restart::Request) is the socket's whole
//! vocabulary — `get status`, and `plan` or `apply` of `service-restart`,
//! `service-stop` and `service-start` with one name — and each is a Rivet
//! command spelled the way an operator would type it. The two hold subjects
//! answer in `super::service_hold`'s wire format.
//!
//! # CONVERGE ONLY, which is the whole ruling
//!
//! `ADR-017` §6.3.2:
//!
//! > **Ruled: the supervisor's control surface may only ask it to CONVERGE,
//! > never to diverge.**
//!
//! * **Permitted** — restart a service; start an *enabled* service that is idle
//!   or failed. Both are requests to re-converge toward what the document
//!   already says.
//! * **Refused** — starting a service the document disables, and stopping one it
//!   enables. The refusal names `apply edit` and `apply reload`, because
//!   changing what the machine should be running is a change to the document by
//!   definition.
//!
//! **The two permitted things are one subject here, and that is a departure the
//! ADR amendment records.** §6.3.2's own sentence is the argument for it —
//! *"Both are requests to re-converge toward what the document already says"* —
//! and a separate `service-start` would have had to answer *what does starting
//! an already-running service do?* Either *nothing*, which is a command that
//! reports success for having done nothing, or *restart it*, which is this
//! subject under a second name. One subject, one refusal path, one guard.
//!
//! # A stop is a different subject, and a restart never becomes one
//!
//! `super::service_hold` stops one application and holds it stopped until it
//! is started again or the machine reboots (`ADR-017` §6). That is a separate
//! request on the wire with its own refusals. A restart of a held service is
//! refused rather than quietly releasing the hold, so the only thing that
//! releases one is the command that says so.
//!
//! # Why `service-restart` and not `restart`
//!
//! **`apply restart` already means *restart the machine*** (`super::stop`), and
//! `ADR-006` §6.4's vocabulary is worth exactly as much as its unambiguity. A
//! subject that differed from it only by taking an extra word would mean that
//! `rivet apply restart confirm` — a whole-machine reboot — is what a truncated
//! or mistyped per-service restart lands on. So the two share no prefix, they
//! sort apart, and `the_two_restarts_cannot_be_confused` asserts it rather
//! than leaving it to review.
//!
//! # The wire format, both halves, here
//!
//! `super::status`'s rule and its reasons, unchanged: this workspace has no JSON
//! parser (a ruling, not a gap), so the format is tab-separated lines and
//! **both the writer and the reader are in this module** with the round trip
//! asserted. A service name cannot contain a tab, so there is no escaping.
//!
//! **The socket's own constants — the path and the two size bounds — are still
//! in `super::status`**, where the socket was born. They are the *socket's* and
//! not that subject's, now that a second operation crosses it (`ADR-017`
//! §6.3.1: *"the socket becomes the supervisor's control surface"*), and moving
//! them is a rename this change deliberately did not fold in. Recorded here so
//! that it is a known place rather than a discovered one.

use super::machine::Board;
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, MachineError, OperationError, Verb};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "service-restart";

/// **What a caller may send over the supervisor's control surface.**
///
/// `ADR-017` §6 made `ADR-032` §3's query socket the supervisor's control
/// surface, and this is that surface's whole vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// `get status` — `super::status`, and the only thing that crossed until
    /// 2026-08-29.
    GetStatus,
    /// `plan service-restart <name>` or `apply service-restart <name>`.
    ///
    /// **`plan` is `ADR-006` §6.4's own verb**, and §6.5 makes the dry run a
    /// property of the framework rather than a word an operator types — so
    /// nothing exposes `plan service-restart` on the CLI. It is what
    /// `RestartService::plan` sends when the confirmation is absent, which is
    /// how the operator gets the supervisor's *own* answer about what would
    /// happen instead of the CLI's guess at it.
    ServiceRestart {
        service: String,
        /// `true` for `apply`, `false` for `plan`.
        apply: bool,
    },
    /// `plan service-stop <name>`, `apply service-stop <name>`, and the same
    /// for `service-start` — `super::service_hold`.
    ServiceHold {
        kind: super::service_hold::Kind,
        service: String,
        /// `true` for `apply`, `false` for `plan`.
        apply: bool,
    },
    /// `plan clock-floor-reset` or `apply clock-floor-reset` —
    /// `super::clock_floor`. Names no service: the floor is the machine's.
    ClockFloorReset {
        /// `true` for `apply`, `false` for `plan`.
        apply: bool,
    },
}

impl Request {
    /// The wire form. One line, ending in a newline, exactly as a caller types
    /// the command.
    pub fn render(&self) -> String {
        let verb = |apply: bool| if apply { Verb::Apply } else { Verb::Plan };
        match self {
            Self::GetStatus => super::status::REQUEST.to_string(),
            Self::ServiceRestart { service, apply } => {
                format!("{verb} {SUBJECT} {service}\n", verb = verb(*apply))
            }
            Self::ServiceHold {
                kind,
                service,
                apply,
            } => format!("{verb} {kind} {service}\n", verb = verb(*apply)),
            Self::ClockFloorReset { apply } => format!(
                "{verb} {subject}\n",
                verb = verb(*apply),
                subject = super::clock_floor::SUBJECT
            ),
        }
    }

    /// [`Self::render`]'s inverse. **A request this build does not answer is an
    /// error rather than a default** — `ADR-032` §3.3 item 1 rules that the
    /// socket carries an operation, and one that silently ignored an unknown
    /// word would be a second command language whose vocabulary nobody could
    /// discover.
    pub fn parse(line: &str) -> Result<Self, RequestError> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["get", "status"] => Ok(Self::GetStatus),
            [verb @ ("plan" | "apply"), subject, service]
                if *subject == SUBJECT || super::service_hold::Kind::parse(subject).is_some() =>
            {
                if crate::service::ServiceName::new(service).is_none() {
                    return Err(RequestError::NotAServiceName {
                        word: (*service).to_string(),
                    });
                }
                let service = (*service).to_string();
                let apply = *verb == "apply";
                Ok(match super::service_hold::Kind::parse(subject) {
                    Some(kind) => Self::ServiceHold {
                        kind,
                        service,
                        apply,
                    },
                    None => Self::ServiceRestart { service, apply },
                })
            }
            [verb @ ("plan" | "apply"), subject] if *subject == super::clock_floor::SUBJECT => {
                Ok(Self::ClockFloorReset {
                    apply: *verb == "apply",
                })
            }
            _ => Err(RequestError::Unknown {
                line: line.trim().to_string(),
            }),
        }
    }
}

/// A request the supervisor will not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// Not one of the three commands this build answers.
    Unknown { line: String },
    /// A `service-restart` whose third word could not be a service name.
    ///
    /// **Checked here rather than by the supervisor looking it up**, because the
    /// two are different answers: *that is not a name* and *this machine has no
    /// service by that name* need different next moves, and a name with a `/` in
    /// it must never reach a lookup that builds a cgroup path from it.
    NotAServiceName { word: String },
}

impl core::fmt::Display for RequestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown { line } => write!(
                f,
                "this socket answers `{status}`, `{plan}` or `{apply}` of `{SUBJECT}`, `{stop}` \
                 or `{start}` followed by one service, and `{plan}` or `{apply}` of `{floor}`, in \
                 this build (ADR-017 §6.3.1), and {line:?} is none of them. `rivet --version` \
                 publishes what this machine implements.",
                status = super::status::REQUEST.trim(),
                plan = Verb::Plan,
                apply = Verb::Apply,
                stop = super::service_hold::STOP_SUBJECT,
                start = super::service_hold::START_SUBJECT,
                floor = super::clock_floor::SUBJECT,
            ),
            Self::NotAServiceName { word } => write!(
                f,
                "{word:?} cannot be a service name, so nothing was looked up and NOTHING WAS \
                 DONE. A service name has no path separator and no control characters."
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The answer
// ---------------------------------------------------------------------------

/// **What the supervisor will do, or has done, about one service.**
///
/// The same type on both paths: `plan` answers what *would* happen and `apply`
/// answers what *did*, in the same words, so the two are comparable. That is
/// `ADR-006` §6.5's whole shape — a plan an operator confirms should be the
/// thing they are confirming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restarting {
    pub service: String,
    /// The state the supervisor found the service in, **in the supervisor's own
    /// word** — `idle`, `started`, `adopted`, `done`, `backoff`, `failed`,
    /// `stopping`. `AGENTS.md` §5, and it is the same vocabulary
    /// [`super::status::ServiceStatus::state`] carries, from one definition.
    pub was: String,
    /// What converging this service takes from here. See [`Doing`].
    pub doing: Doing,
    /// **Whether this service is one of those that keep the machine
    /// reachable**, as the supervisor's own list defines it —
    /// `cmd/rivet-sysd`'s `definitions::REACHABILITY_SET`.
    ///
    /// The supervisor is the authority and answers it; this crate composes the
    /// sentence ([`Restarting::caveat`]) and never holds the list. `ADR-014`
    /// §3 forbids the core naming anything in a binary crate, and a second copy
    /// of the list here would be `ADR-011` §3 exactly.
    pub reachability: bool,
}

/// What converging one service takes. **Three words, and none of them is a
/// stop**: the stop in `Doing::Stop` is the *first half* of a restart on a
/// service the document enables, which the supervisor performs and then
/// converges back out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doing {
    /// It is running: `SIGTERM`, then the supervisor starts it again because
    /// the document says it should be running. `ADR-017` §3.5's deadline and a
    /// `SIGKILL` if it will not go.
    Stop,
    /// It is not running: the supervisor starts it. This is the `idle`,
    /// `backoff`, `failed` and `done` case — the backoff and the give-up latch
    /// are discarded, which is what *discard current state* means.
    Start,
    /// It is already stopping, so it is already going where a restart would put
    /// it, and it comes back on its own because the document enables it.
    Nothing,
}

impl Doing {
    /// The wire word. Part of the interface contract; never renamed.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Start => "start",
            Self::Nothing => "nothing",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        [Self::Stop, Self::Start, Self::Nothing]
            .into_iter()
            .find(|d| d.as_str() == word)
    }

    /// What it means, for a plan an operator reads.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Stop => "it is running, so it is signalled to stop and the supervisor starts \
                           it again on its next convergence pass — the document says it should \
                           be running and nothing here changes that",
            Self::Start => "it is not running, so the supervisor starts it. Any backoff it was \
                            waiting out and any give-up latch it had reached are discarded, \
                            which is what discarding current state means",
            Self::Nothing => "it is already stopping, which is where a restart would put it, and \
                              it comes back on its own because the document enables it. Nothing \
                              further is asked",
        }
    }
}

/// **The one thing this operation cannot promise, said before it is confirmed.**
///
/// `None` for an ordinary service. `Some` when the supervisor answered that this
/// service is in its reachability set — which today is the control plane and
/// the interface bring-up.
///
/// # Why this is a caveat and not a refusal
///
/// Recorded here because it is the decision a reader will want to check.
/// **`ADR-017` §6.3.2's rule is about CONVERGENCE, and a restart converges.**
/// The reachability set exists so that a *reload* cannot durably stop the
/// services that carry the way in (`ADR-032` §3.2.1) — a stop that persists
/// removes the means of undoing itself. A restart is not that: the definition
/// and the document are unchanged, so the supervisor brings the service back on
/// the same pass by the same rules, and there is nothing to undo.
///
/// **And refusing it would be incoherent with what this surface already
/// permits.** `apply restart confirm` reboots the whole machine, which stops the
/// control plane along with everything else, and `super::stop` documents that
/// the answer *"RACES the shutdown whichever channel carried it"*. A surface
/// that refused the small act while permitting the large one that contains it
/// would be protecting nothing.
///
/// What is honest to say — and what the plan therefore says — is that **this
/// command may not be able to report its own outcome**, because on an ordinary
/// board the process carrying the answer is the one being restarted.
pub fn caveat(service: &str) -> String {
    format!(
        "`{service}` is one of the services that keep this machine reachable, and if it is the \
         one carrying this request then THE ANSWER TO THIS COMMAND MAY NOT ARRIVE. The \
         supervisor writes its reply before it signals anything, so the reply usually wins — but \
         it is a race and a dropped connection is an ORDINARY OUTCOME here, not a failure. It is \
         permitted rather than refused because a restart CONVERGES: the document is unchanged, \
         so the supervisor starts it again on the same pass (ADR-017 §6.3.2). `rivet {get} \
         {status}` is the authority for what happened; ask again once it answers.",
        get = Verb::Get,
        status = super::status::SUBJECT,
    )
}

impl Restarting {
    /// [`caveat`] for this service, or `None`.
    pub fn caveat(&self) -> Option<String> {
        self.reachability.then(|| caveat(&self.service))
    }

    /// The wire form: one line, tab-separated.
    ///
    /// ```text
    /// restarting\tphoto-index\tstarted\tstop\tfalse
    /// ```
    pub fn render(&self) -> String {
        format!(
            "{RESTARTING}\t{}\t{}\t{}\t{}\n",
            self.service,
            self.was,
            self.doing.as_str(),
            self.reachability,
        )
    }

    /// The structured result, whose keys are `OUTPUTS`.
    pub fn value(&self, applied: bool) -> Value {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("service", Value::str(self.service.clone())),
            ("was", Value::str(self.was.clone())),
            ("doing", Value::str(self.doing.as_str())),
            ("applied", Value::Bool(applied)),
            (
                "next",
                Value::str(format!(
                    "{caveat}the supervisor converges on its next pass and `rivet {get} {status}` \
                     is what says it did — this operation reports that the REQUEST was taken, \
                     not that the service is back up. `rivet {get} log {service}` carries the \
                     stop, the spawn and the pid.",
                    caveat = self
                        .caveat()
                        .map_or_else(String::new, |c| format!("{c} ")),
                    get = Verb::Get,
                    status = super::status::SUBJECT,
                    service = self.service,
                )),
            ),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "a service-restart result's keys must be exactly OUTPUTS, in the declared order"
        );
        Value::object(fields)
    }
}

/// The fields this subject's result carries.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `service-restart` here.",
    },
    Field {
        name: "service",
        description: "The service the supervisor acted on, echoed back.",
    },
    Field {
        name: "was",
        description: "The state the supervisor found it in, in the supervisor's own vocabulary — \
                      the same words `get status` uses.",
    },
    Field {
        name: "doing",
        description: "`stop` (it is running and is signalled, then started again), `start` (it \
                      is not running and is started, discarding any backoff or give-up latch), \
                      or `nothing` (it is already stopping and comes back on its own).",
    },
    Field {
        name: "applied",
        description: "Whether this was the confirmed apply. It means the supervisor TOOK the \
                      request, not that the service is back up — convergence happens on its next \
                      pass and ADR-017 §3.5's stop deadline is underneath it.",
    },
    Field {
        name: "next",
        description: "Where the outcome appears, and — for a service that keeps this machine \
                      reachable — that the answer to this command may not arrive at all.",
    },
];

// ---------------------------------------------------------------------------
// The refusals — ADR-017 §6.3.2's other half
// ---------------------------------------------------------------------------

/// **Why the supervisor would not do it.**
///
/// The variants carry facts and nothing else; [`Self::describe`] composes the
/// sentence, in this crate, from one definition — so the supervisor's journal
/// and the operator's terminal cannot end up saying different things about the
/// same refusal. `ADR-011`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No service by that name. Carries what there is, because the next move is
    /// to type one of them.
    UnknownService {
        service: String,
        known: Vec<String>,
    },
    /// **§6.3.2's central refusal.** The document disables this service, so
    /// starting it would manufacture machine state that contradicts the record.
    Disabled { service: String },
    /// The machine is shutting down. Converging toward *running* is not
    /// something this supervisor is doing any more.
    ShuttingDown { service: String },
    /// **The service is stopped by `apply service-stop`.** A restart would
    /// release that stop as a side effect, and the only thing that releases one
    /// is the command that says so (`super::service_hold`).
    Held { service: String },
}

/// The wire word for a refusal's kind. Part of the interface contract.
impl Refusal {
    /// The wire word. Never renamed.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownService { .. } => "unknown-service",
            Self::Disabled { .. } => "disabled",
            Self::ShuttingDown { .. } => "shutting-down",
            Self::Held { .. } => "held",
        }
    }

    /// The service it is about.
    pub fn service(&self) -> &str {
        match self {
            Self::UnknownService { service, .. }
            | Self::Disabled { service }
            | Self::ShuttingDown { service }
            | Self::Held { service } => service,
        }
    }

    /// **The sentence.** Every one of them says `REFUSED` first, cites the
    /// ruling, and says what to do instead — `cmd/rivet-sysd`'s
    /// `every_refusal_names_itself_and_cites_something` is the shape and
    /// `every_refusal_names_itself_and_cites_something` here holds this one to
    /// it.
    pub fn describe(&self) -> String {
        match self {
            Self::UnknownService { service, known } => format!(
                "REFUSED to restart `{service}`: this machine's supervisor has no service by \
                 that name, so NOTHING WAS DONE. It knows {list}. `rivet {get} {status}` lists \
                 them with what each is doing (ADR-017 §6.3.2).",
                list = if known.is_empty() {
                    "none at all, which means it has not finished starting or has no \
                     definitions".to_string()
                } else {
                    known
                        .iter()
                        .map(|n| format!("`{n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                get = Verb::Get,
                status = super::status::SUBJECT,
            ),
            Self::Disabled { service } => format!(
                "REFUSED to restart `{service}`: THE DOCUMENT DISABLES IT, and this control \
                 surface may only ask the supervisor to CONVERGE toward the document, never to \
                 diverge from it (ADR-017 §6.3.2). Starting it would make this machine run \
                 something its own desired state says it should not, and that state would \
                 vanish at the next boot with nothing to say why. NOTHING WAS DONE. **To make \
                 it run, change what this machine should be**: `rivet {apply} {edit} \
                 /state/config/machine.toml --expect '<the services.{service} table>' --replace \
                 '<it, with enabled = true>' {confirm}`, then `rivet {apply} {reload} \
                 {confirm}`. That is a change to the document, which is what turning a service \
                 on is by definition.",
                apply = Verb::Apply,
                edit = super::edit::SUBJECT,
                reload = super::reload::SUBJECT,
                confirm = super::apply::CONFIRMATION,
            ),
            Self::ShuttingDown { service } => format!(
                "REFUSED to restart `{service}`: this machine is SHUTTING DOWN, so the \
                 supervisor is bringing every service down rather than converging toward the \
                 document (ADR-017 §3.2). A restart taken now would be a stop with no start \
                 after it, which is the one thing this surface may never produce (ADR-017 \
                 §6.3.2). NOTHING WAS DONE — ask again once the machine is back up.",
            ),
            Self::Held { service } => format!(
                "REFUSED to restart `{service}`: it is STOPPED by `{apply} {stop}`, and a restart \
                 would start it again as a side effect. NOTHING WAS DONE. `rivet {apply} {start} \
                 {service} {confirm}` releases the stop, which is what starting it means here \
                 (ADR-017 §6).",
                apply = Verb::Apply,
                stop = super::service_hold::STOP_SUBJECT,
                start = super::service_hold::START_SUBJECT,
                confirm = super::apply::CONFIRMATION,
            ),
        }
    }

    /// How a refusal reaches an operation: **the machine objecting to this
    /// request as it stands**, or the machine reporting what it found.
    ///
    /// `Unsafe` for the two that will not change on a retry, which is
    /// [`ErrorCode::Unsafe`]'s exact definition — *"an agent that gets this must
    /// stop rather than retry"*. `Refused` for an unknown name, where the
    /// machine has been read and has said why.
    pub fn as_machine_error(&self) -> MachineError {
        match self {
            Self::UnknownService { .. } => MachineError::Refused(self.describe()),
            Self::Disabled { .. } | Self::ShuttingDown { .. } | Self::Held { .. } => {
                MachineError::Unsafe(self.describe())
            }
        }
    }

    /// The wire form: one line, tab-separated. The fourth field is the known
    /// list for [`Self::UnknownService`] and empty otherwise.
    pub fn render(&self) -> String {
        format!(
            "{REFUSED}\t{}\t{}\t{}\n",
            self.service(),
            self.code(),
            match self {
                Self::UnknownService { known, .. } => known.join(" "),
                _ => String::new(),
            },
        )
    }
}

const RESTARTING: &str = "restarting";
const REFUSED: &str = "refused";

/// **What the supervisor answered**, read back off the socket.
///
/// [`Restarting::render`] and [`Refusal::render`] are the writers; this is the
/// reader, and the round trip is asserted. `super::status`'s argument for
/// keeping both halves in one module, unchanged.
///
/// # Errors
///
/// A line this build cannot read is refused rather than guessed at — an older
/// CLI talking to a newer supervisor must say so, which is `ADR-006` §7's
/// *trusted and wrong* in the one place a version skew produces it.
pub fn parse_answer(text: &str) -> Result<Result<Restarting, Refusal>, AnswerError> {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or(AnswerError::Empty)?;
    let fields: Vec<&str> = line.split('\t').collect();
    match fields.first().copied().unwrap_or_default() {
        RESTARTING => {
            let [_, service, was, doing, reachability] = fields.as_slice() else {
                return Err(AnswerError::Fields {
                    kind: RESTARTING,
                    found: fields.len(),
                    wanted: 5,
                });
            };
            Ok(Ok(Restarting {
                service: (*service).to_string(),
                was: (*was).to_string(),
                doing: Doing::parse(doing).ok_or_else(|| AnswerError::Value {
                    field: "doing",
                    found: (*doing).to_string(),
                })?,
                reachability: match *reachability {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(AnswerError::Value {
                            field: "reachability",
                            found: other.to_string(),
                        });
                    }
                },
            }))
        }
        REFUSED => {
            let [_, service, code, rest] = fields.as_slice() else {
                return Err(AnswerError::Fields {
                    kind: REFUSED,
                    found: fields.len(),
                    wanted: 4,
                });
            };
            let service = (*service).to_string();
            match *code {
                "unknown-service" => Ok(Err(Refusal::UnknownService {
                    service,
                    known: rest
                        .split_whitespace()
                        .map(ToString::to_string)
                        .collect(),
                })),
                "disabled" => Ok(Err(Refusal::Disabled { service })),
                "shutting-down" => Ok(Err(Refusal::ShuttingDown { service })),
                "held" => Ok(Err(Refusal::Held { service })),
                other => Err(AnswerError::Value {
                    field: "code",
                    found: other.to_string(),
                }),
            }
        }
        other => Err(AnswerError::UnknownLine {
            kind: other.to_string(),
        }),
    }
}

/// An answer that could not be read. [`parse_answer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerError {
    Empty,
    UnknownLine { kind: String },
    Fields { kind: &'static str, found: usize, wanted: usize },
    Value { field: &'static str, found: String },
}

impl core::fmt::Display for AnswerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str(
                "the supervisor answered nothing at all, so NOTHING IS ASSERTED about what it \
                 did. `rivet get status` is where to look.",
            ),
            Self::UnknownLine { kind } => write!(
                f,
                "the answer begins with `{kind}`, which this build does not know. The supervisor \
                 is newer than this CLI, or that is not an answer to this command."
            ),
            Self::Fields { kind, found, wanted } => write!(
                f,
                "the answer is a `{kind}` with {found} fields and this build wants {wanted}."
            ),
            Self::Value { field, found } => write!(
                f,
                "the answer's `{field}` is `{found}`, which is not a value it may take."
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The operation
// ---------------------------------------------------------------------------

/// **The change this subject is**, as `ADR-006` §6.5's framework defines one.
///
/// The interesting property: **the plan is the supervisor's, not this crate's.**
/// [`Change::plan`] asks the machine what it *would* do and builds the plan from
/// the answer, so a plan cannot describe a service in a state the supervisor is
/// not actually holding it in, and the refusals happen on the plan path too —
/// which is `ADR-017` §6.3.2's *"the refusals are also trivially writable"*
/// cashed out where the operator can still choose differently.
struct RestartService {
    service: String,
}

impl Change for RestartService {
    type Prepared = Restarting;
    type Applied = Restarting;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Restarting), OperationError> {
        let preview = board
            .can_request_service_restart(&self.service)
            .map_err(refused)?;

        let mut plan = Plan::new(
            SUBJECT,
            format!(
                "discards `{}`'s current state and lets the supervisor converge to the SAME \
                 document",
                self.service
            ),
        )
        .doing(
            Effect::request(
                self.service.clone(),
                format!(
                    "restart it — the supervisor found it `{was}`, so {what}",
                    was = preview.was,
                    what = preview.doing.describe(),
                ),
            )
            .noting(
                "this changes nothing about what this machine SHOULD be running. ADR-017 §6.3.2 \
                 rules that this surface may only ask the supervisor to converge toward the \
                 document, never to diverge from it",
            ),
        )
        .leaving(
            "the desired-state document — `enabled` is configuration and a restart is an \
             operation (ADR-017 §6). To change what this machine should be running, use `apply \
             edit` and `apply reload`",
        )
        .leaving(format!(
            "every other service — this names one, and `rivet {apply} {restart} {confirm}` is \
             the whole machine",
            apply = Verb::Apply,
            restart = super::machine::Stop::Restart.as_str(),
            confirm = super::apply::CONFIRMATION,
        ))
        // **The memory cap, said because the obvious reading is wrong.** A cap
        // is written into a cgroup at the spawn that creates it (`ADR-017`
        // §3.1.3), so a restart does write one — but it writes the cap the
        // supervisor's DEFINITION holds, and a `SIGHUP` reload deliberately
        // does not update that (`ADR-032` §3.2.1). So this is not the way to
        // take a cap an operator has just edited, and `super::reload`'s
        // withheld list was corrected on the same day to stop implying it was.
        .leaving(
            "a memory cap the document has changed since this machine booted — a restart writes \
             the cap this service's DEFINITION holds, which a reload does not update (ADR-032 \
             §3.2.1). A new cap is taken at the next boot",
        )
        .leaving("NOTHING HAS BEEN ASKED — the request is sent only if this is confirmed")
        // **Immediately**, and the exception is the `untouched` entry above.
        // What a restart takes, it takes at once.
        .taking_effect(TakesEffect::Immediately)
        .confirmed_by(&[SUBJECT, &self.service]);

        if let Some(caveat) = preview.caveat() {
            plan = plan.leaving(caveat);
        }
        Ok((plan, preview))
    }

    fn apply(self, _: Restarting, board: &mut dyn Board) -> Result<Restarting, OperationError> {
        board
            .request_service_restart(&self.service)
            .map_err(refused)
    }
}

/// Perform it. `arguments[0]` is the subject.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Restarting, OperationError> {
    debug_assert_eq!(arguments.first().copied(), Some(SUBJECT));

    let Some(service) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the name of ONE service. There is no form of it that \
                 restarts everything — that is `{apply} {restart} {confirm}`, which restarts the \
                 machine. `rivet {get} {status}` lists what this machine is running.",
                apply = Verb::Apply,
                restart = super::machine::Stop::Restart.as_str(),
                confirm = super::apply::CONFIRMATION,
                get = Verb::Get,
                status = super::status::SUBJECT,
            ),
        ));
    };

    // **The confirmation is the third word and a fourth is refused**, which is
    // `super::reload`'s rule and the same hazard: an operator who typed a second
    // service name must not have one of them silently restarted.
    if let Some(extra) = arguments.get(3).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {SUBJECT}` takes one service and then the confirmation. `{extra}` is a \
                 fourth word, and there is no form of this command that restarts two services: \
                 run it twice, so that each one is confirmed on its own.",
                apply = Verb::Apply,
            ),
        ));
    }

    let confirmed = super::apply::confirmed(arguments.get(2).copied())?;
    resolve(
        RestartService {
            service: service.to_string(),
        },
        confirmed,
        board,
    )
}

/// The one place a [`MachineError`] becomes this operation's refusal — the same
/// mapping every other subject uses, and **nothing is reworded on the way
/// through**.
fn refused(error: MachineError) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, SUBJECT, error.why())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::operation::machine::{Machine, Reboot, Stop};
    use crate::operation::{Act, apply::CONFIRMATION};
    use crate::state::Slot;

    /// A board that answers this operation's two methods and refuses the rest.
    struct FakeBoard {
        answer: Result<Restarting, Refusal>,
        planned: u32,
        applied: u32,
    }

    impl FakeBoard {
        fn saying(answer: Result<Restarting, Refusal>) -> Self {
            Self {
                answer,
                planned: 0,
                applied: 0,
            }
        }

        fn healthy(service: &str, was: &str, doing: Doing) -> Self {
            Self::saying(Ok(Restarting {
                service: service.to_string(),
                was: was.to_string(),
                doing,
                reachability: false,
            }))
        }
    }

    fn nope() -> MachineError {
        MachineError::Refused("this fake answers only the service-restart methods".to_string())
    }

    crate::operation::machine::refuses_to_edit!(FakeBoard);
    crate::operation::machine::refuses_to_deploy!(FakeBoard);
    crate::operation::machine::refuses_to_observe!(FakeBoard);

    impl Machine for FakeBoard {
        fn running_slot(&mut self) -> Result<Slot, MachineError> {
            Err(nope())
        }
        fn can_arm_trial_boot(&mut self, _slot: Slot) -> Result<(), MachineError> {
            Err(nope())
        }
        fn arm_trial_boot(&mut self, _slot: Slot) -> Result<Reboot, MachineError> {
            Err(nope())
        }
        fn can_request_stop(&mut self, _how: Stop) -> Result<(), MachineError> {
            Err(nope())
        }
        fn request_stop(&mut self, _how: Stop) -> Result<(), MachineError> {
            Err(nope())
        }
        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            Err(nope())
        }
        fn request_reload(&mut self) -> Result<(), MachineError> {
            Err(nope())
        }
        fn can_request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<Restarting, MachineError> {
            self.planned += 1;
            self.answer
                .clone()
                .map_err(|refusal| refusal.as_machine_error())
        }
        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<Restarting, MachineError> {
            self.applied += 1;
            self.answer
                .clone()
                .map_err(|refusal| refusal.as_machine_error())
        }
    }

    // -----------------------------------------------------------------------
    // The naming, which is the thing an operator's fingers get wrong
    // -----------------------------------------------------------------------

    /// **`apply restart` restarts the MACHINE and `apply service-restart`
    /// restarts one service**, and the two must not be reachable from each
    /// other by a slip of the fingers. Asserted rather than reviewed, because
    /// the failure is a board rebooting when somebody meant to bounce a daemon.
    #[test]
    fn the_two_restarts_cannot_be_confused() {
        let machine = Stop::Restart.as_str();
        assert_ne!(SUBJECT, machine);
        assert!(
            !SUBJECT.starts_with(machine) && !machine.starts_with(SUBJECT),
            "`{SUBJECT}` and `{machine}` share a prefix, so a truncated command reaches the \
             wrong one"
        );
        // And the ONE-word command is the machine: `apply service-restart` with
        // no service is an argument error rather than a reboot.
        let mut board = FakeBoard::healthy("a", "started", Doing::Stop);
        let refusal = perform(&[SUBJECT], &mut board).expect_err("a bare subject");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert_eq!(board.applied, 0);
        assert!(refusal.message().contains(machine), "{}", refusal.message());
    }

    // -----------------------------------------------------------------------
    // Converge only — ADR-017 §6.3.2
    // -----------------------------------------------------------------------

    /// **A service the document disables cannot be started by this surface**,
    /// on either path, and the refusal names both remedies. `ADR-017` §6.3.2's
    /// central rule.
    #[test]
    fn a_service_the_document_disables_cannot_be_started() {
        for confirmation in [None, Some(CONFIRMATION)] {
            let mut board = FakeBoard::saying(Err(Refusal::Disabled {
                service: "photo-index".to_string(),
            }));
            let mut argv = vec![SUBJECT, "photo-index"];
            argv.extend(confirmation);
            let refusal = perform(&argv, &mut board).expect_err("a disabled service");
            assert_eq!(
                refusal.code(),
                ErrorCode::Unsafe,
                "a refusal that will not change on a retry is Unsafe: {}",
                refusal.message()
            );
            let why = refusal.message();
            assert!(why.contains("THE DOCUMENT DISABLES IT"), "{why}");
            assert!(why.contains("apply edit"), "{why}");
            assert!(why.contains("apply reload"), "{why}");
            assert!(why.contains("NOTHING WAS DONE"), "{why}");
        }
    }

    /// **The refusal happens on the PLAN path too**, which is what stops an
    /// operator from confirming something that was never going to work. The
    /// count is the assertion: the plan asked the machine, and the machine's
    /// answer ended it.
    #[test]
    fn the_supervisor_refuses_before_the_confirmation_is_ever_typed() {
        let mut board = FakeBoard::saying(Err(Refusal::Disabled {
            service: "photo-index".to_string(),
        }));
        let _ = perform(&[SUBJECT, "photo-index"], &mut board).expect_err("disabled");
        assert_eq!(board.planned, 1, "the plan did not consult the supervisor");
        assert_eq!(board.applied, 0, "the apply path ran on a refused plan");
    }

    /// **The socket's grammar is exactly what is listed, and nothing near it.**
    /// A stop is `service-stop` and only that: the words an operator types from
    /// habit on another Linux — `stop`, `disable`, `kill` — are not requests,
    /// and nothing can spell a change to `enabled`. `ADR-017` §6.
    #[test]
    fn the_socket_answers_its_listed_requests_and_nothing_near_them() {
        for word in ["stop", "start", "disable", "service-disable", "kill", "service-pause"] {
            for verb in ["apply", "plan", "get", "set", "remove"] {
                let line = format!("{verb} {word} photo-index\n");
                assert!(
                    matches!(Request::parse(&line), Err(RequestError::Unknown { .. })),
                    "{line:?} was read as a request"
                );
            }
        }
        // Only `plan` and `apply` carry a subject; `get service-stop` is nothing.
        for word in [SUBJECT, "service-stop", "service-start"] {
            let line = format!("get {word} photo-index\n");
            assert!(
                matches!(Request::parse(&line), Err(RequestError::Unknown { .. })),
                "{line:?} was read as a request"
            );
        }
        let s = || "photo-index".to_string();
        let mut every = vec![Request::GetStatus];
        for apply in [true, false] {
            every.push(Request::ServiceRestart { service: s(), apply });
            for kind in [
                super::super::service_hold::Kind::Stop,
                super::super::service_hold::Kind::Start,
            ] {
                every.push(Request::ServiceHold {
                    kind,
                    service: s(),
                    apply,
                });
            }
            every.push(Request::ClockFloorReset { apply });
        }
        for request in &every {
            let rendered = request.render();
            assert!(
                !rendered.contains("enabled"),
                "{rendered:?} can express a change to the document"
            );
            assert_eq!(&Request::parse(&rendered).expect("round trip"), request);
        }
    }

    /// **A restart of a held service is refused**, so a restart can never
    /// release a stop as a side effect.
    #[test]
    fn a_held_service_is_not_restarted_and_the_refusal_names_service_start() {
        let mut board = FakeBoard::saying(Err(Refusal::Held {
            service: "photo-index".to_string(),
        }));
        let refusal = perform(&[SUBJECT, "photo-index", CONFIRMATION], &mut board)
            .expect_err("a held service");
        assert_eq!(refusal.code(), ErrorCode::Unsafe);
        assert!(
            refusal.message().contains("apply service-start photo-index confirm"),
            "{}",
            refusal.message()
        );
        assert_eq!(board.applied, 0);
    }

    /// A machine that is going down refuses, because a restart taken then would
    /// be a stop with no start after it.
    #[test]
    fn a_machine_that_is_shutting_down_refuses_and_says_why() {
        let mut board = FakeBoard::saying(Err(Refusal::ShuttingDown {
            service: "rivet-mcpd".to_string(),
        }));
        let refusal =
            perform(&[SUBJECT, "rivet-mcpd", CONFIRMATION], &mut board).expect_err("going down");
        assert_eq!(refusal.code(), ErrorCode::Unsafe);
        assert!(refusal.message().contains("SHUTTING DOWN"), "{}", refusal.message());
    }

    /// An unknown name is refused with the list, because the next move is to
    /// type one of them.
    #[test]
    fn an_unknown_service_is_refused_with_what_there_is() {
        let mut board = FakeBoard::saying(Err(Refusal::UnknownService {
            service: "photoindex".to_string(),
            known: vec!["photo-index".to_string(), "rivet-netd".to_string()],
        }));
        let refusal =
            perform(&[SUBJECT, "photoindex", CONFIRMATION], &mut board).expect_err("no such");
        assert_eq!(refusal.code(), ErrorCode::MachineRefused);
        assert!(refusal.message().contains("photo-index"), "{}", refusal.message());
    }

    // -----------------------------------------------------------------------
    // The permitted path
    // -----------------------------------------------------------------------

    /// Without the confirmation it is a plan, and nothing is asked.
    #[test]
    fn without_the_confirmation_nothing_is_asked() {
        let mut board = FakeBoard::healthy("photo-index", "started", Doing::Stop);
        let refusal =
            perform(&[SUBJECT, "photo-index"], &mut board).expect_err("it restarted unconfirmed");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert_eq!(board.applied, 0, "the request was sent without a yes");
        assert_eq!(board.planned, 1);

        let plan = refusal.plan().expect("a confirmation refusal carries its plan");
        let [effect] = plan.effects() else {
            panic!("one service, one effect: {:?}", plan.effects());
        };
        assert_eq!(effect.act, Act::Request);
        assert_eq!(effect.target, "photo-index");
        // The plan repeats the SUPERVISOR's word for what it found, not a guess.
        let asked = effect.to.clone().unwrap_or_default();
        assert!(asked.contains("started"), "{asked}");
        assert_eq!(
            plan.confirm_with(),
            format!("rivet apply {SUBJECT} photo-index {CONFIRMATION}")
        );
        assert_eq!(plan.takes_effect().requires_reboot(), Some(false));
        // And it says the document is untouched, naming the two remedies.
        let untouched = plan.untouched().join("\n");
        assert!(untouched.contains("apply edit"), "{untouched}");
        assert!(untouched.contains("apply reload"), "{untouched}");
    }

    /// With the word, the request is sent and the result reports what the
    /// supervisor said.
    #[test]
    fn with_the_confirmation_the_request_is_sent() {
        let mut board = FakeBoard::healthy("photo-index", "failed", Doing::Start);
        let done = perform(&[SUBJECT, "photo-index", CONFIRMATION], &mut board)
            .expect("a healthy board");
        assert_eq!(board.applied, 1);
        assert_eq!(board.planned, 1, "the interlock did not run before the act");
        assert_eq!(done.doing, Doing::Start);
        let Value::Object(fields) = done.value(true) else {
            panic!("a result is an object");
        };
        assert!(fields.iter().any(|(k, v)| *k == "was" && *v == Value::str("failed")));
        assert!(fields.iter().any(|(k, v)| *k == "applied" && *v == Value::Bool(true)));
    }

    /// A fourth word is refused rather than ignored: two service names in one
    /// command must not restart one of them silently.
    #[test]
    fn a_second_service_name_is_refused_rather_than_ignored() {
        let mut board = FakeBoard::healthy("photo-index", "started", Doing::Stop);
        let refusal = perform(
            &[SUBJECT, "photo-index", CONFIRMATION, "rivet-netd"],
            &mut board,
        )
        .expect_err("a fourth word");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert_eq!(refusal.subject(), Some("rivet-netd"));
        assert_eq!(board.applied, 0);
    }

    // -----------------------------------------------------------------------
    // The reachability caveat
    // -----------------------------------------------------------------------

    /// **A service that carries the way in is PERMITTED and the plan says what
    /// it cannot promise.** `ADR-017` §6.3.2's amendment: a restart converges,
    /// so the reachability set — which exists to stop a RELOAD durably
    /// disabling these — does not reach it. What it does reach is the answer.
    #[test]
    fn restarting_the_control_plane_is_permitted_and_the_plan_says_the_answer_may_not_arrive() {
        let mut board = FakeBoard::saying(Ok(Restarting {
            service: "rivet-mcpd".to_string(),
            was: "started".to_string(),
            doing: Doing::Stop,
            reachability: true,
        }));
        let refusal = perform(&[SUBJECT, "rivet-mcpd"], &mut board).expect_err("a plan");
        let plan = refusal.plan().expect("a plan");
        let untouched = plan.untouched().join("\n");
        assert!(
            untouched.contains("THE ANSWER TO THIS COMMAND MAY NOT ARRIVE"),
            "{untouched}"
        );
        assert!(untouched.contains("ORDINARY OUTCOME"), "{untouched}");
        assert!(untouched.contains("rivet get status"), "{untouched}");

        // And it is permitted: confirming it sends the request.
        let mut board = FakeBoard::saying(Ok(Restarting {
            service: "rivet-mcpd".to_string(),
            was: "started".to_string(),
            doing: Doing::Stop,
            reachability: true,
        }));
        let done = perform(&[SUBJECT, "rivet-mcpd", CONFIRMATION], &mut board)
            .expect("restarting the control plane is permitted");
        assert_eq!(board.applied, 1);
        assert!(done.caveat().is_some());
        // The caveat travels in the RESULT too, because the operator who saw it
        // in the plan is not necessarily the agent reading the result.
        let Value::Object(fields) = done.value(true) else {
            panic!("an object");
        };
        let next = fields
            .iter()
            .find(|(k, _)| *k == "next")
            .map(|(_, v)| format!("{v:?}"))
            .expect("a next field");
        assert!(next.contains("MAY NOT ARRIVE"), "{next}");
    }

    /// An ordinary service gets no caveat — the rare warning is worth nothing
    /// if it is on everything (`ADR-017` §3.1.11).
    #[test]
    fn an_ordinary_service_carries_no_caveat() {
        let restarting = Restarting {
            service: "photo-index".to_string(),
            was: "started".to_string(),
            doing: Doing::Stop,
            reachability: false,
        };
        assert_eq!(restarting.caveat(), None);
    }

    // -----------------------------------------------------------------------
    // The wire
    // -----------------------------------------------------------------------

    /// **The round trip, which is why both halves live here.** The supervisor
    /// renders and the CLI parses; if these two disagree a board does something
    /// and reports that the answer was malformed.
    #[test]
    fn what_the_supervisor_writes_is_what_a_reader_reads() {
        for doing in [Doing::Stop, Doing::Start, Doing::Nothing] {
            for reachability in [true, false] {
                let answer = Restarting {
                    service: "rivet-netd".to_string(),
                    was: "done".to_string(),
                    doing,
                    reachability,
                };
                assert_eq!(parse_answer(&answer.render()), Ok(Ok(answer)));
            }
        }
        for refusal in [
            Refusal::UnknownService {
                service: "nope".to_string(),
                known: vec!["a".to_string(), "b".to_string()],
            },
            Refusal::UnknownService {
                service: "nope".to_string(),
                known: Vec::new(),
            },
            Refusal::Disabled {
                service: "photo-index".to_string(),
            },
            Refusal::ShuttingDown {
                service: "photo-index".to_string(),
            },
            Refusal::Held {
                service: "photo-index".to_string(),
            },
        ] {
            assert_eq!(parse_answer(&refusal.render()), Ok(Err(refusal)));
        }
    }

    /// The request round-trips too, and a `plan` is not an `apply`.
    #[test]
    fn a_plan_request_is_not_an_apply_request() {
        let planning = Request::ServiceRestart {
            service: "photo-index".to_string(),
            apply: false,
        };
        let applying = Request::ServiceRestart {
            service: "photo-index".to_string(),
            apply: true,
        };
        assert_ne!(planning.render(), applying.render());
        assert_eq!(Request::parse(&planning.render()), Ok(planning));
        assert_eq!(Request::parse(&applying.render()), Ok(applying));
        assert_eq!(
            Request::parse(super::super::status::REQUEST),
            Ok(Request::GetStatus)
        );
    }

    /// **A name with a path separator never reaches a lookup.** The supervisor
    /// builds cgroup and notify paths out of a service name, so this is refused
    /// at the door with its own diagnostic rather than as *no such service*.
    #[test]
    fn a_word_that_cannot_be_a_service_name_is_refused_at_the_door() {
        for bad in ["../../etc", "a/b", "", "a\tb"] {
            let line = format!("apply {SUBJECT} {bad}\n");
            let refused = Request::parse(&line).expect_err("a bad name");
            assert!(
                matches!(
                    refused,
                    RequestError::NotAServiceName { .. } | RequestError::Unknown { .. }
                ),
                "{bad:?} parsed as {refused:?}"
            );
        }
    }

    /// An answer this build cannot read is refused, never guessed at — the
    /// version-skew case.
    #[test]
    fn an_answer_this_build_does_not_know_is_refused_rather_than_skipped() {
        assert_eq!(
            parse_answer("rebuilt\tphoto-index\n"),
            Err(AnswerError::UnknownLine {
                kind: "rebuilt".to_string()
            })
        );
        assert_eq!(parse_answer(""), Err(AnswerError::Empty));
        assert!(matches!(
            parse_answer("restarting\ta\tstarted\n"),
            Err(AnswerError::Fields { .. })
        ));
        assert!(matches!(
            parse_answer("restarting\ta\tstarted\tdemolish\tfalse\n"),
            Err(AnswerError::Value { field: "doing", .. })
        ));
    }

    /// **Every refusal says so first, cites the ruling, and names what to do
    /// instead.** `cmd/rivet-sysd/src/verify.rs`'s test of the same name is the
    /// shape this follows.
    #[test]
    fn every_refusal_names_itself_and_cites_something() {
        let refusals = [
            Refusal::UnknownService {
                service: "s".to_string(),
                known: vec!["a".to_string()],
            },
            Refusal::Disabled {
                service: "s".to_string(),
            },
            Refusal::ShuttingDown {
                service: "s".to_string(),
            },
            Refusal::Held {
                service: "s".to_string(),
            },
        ];
        for refusal in refusals {
            let text = refusal.describe();
            assert!(
                text.starts_with("REFUSED"),
                "a refusal must say so first: {text}"
            );
            assert!(text.contains("ADR-0"), "{text} cites nothing");
            assert!(text.contains("NOTHING WAS DONE"), "{text}");
            assert!(!refusal.code().is_empty());
            assert_eq!(refusal.service(), "s");
            // A refusal must not contain a tab, or its own wire form would be
            // unreadable — the sentence is composed on the far side from the
            // fields, but this is the property that lets that be true.
            assert!(!text.contains('\t'), "{text}");
        }
    }

    /// **The one refusal that must name both remedies**, checked on the
    /// sentence rather than on the code, because it is the sentence an operator
    /// reads. `ADR-017` §6.3.2 rules exactly this.
    #[test]
    fn the_converge_only_refusal_names_apply_edit_and_apply_reload() {
        let text = Refusal::Disabled {
            service: "photo-index".to_string(),
        }
        .describe();
        assert!(text.contains("apply edit"), "{text}");
        assert!(text.contains("apply reload"), "{text}");
        assert!(text.contains("CONVERGE"), "{text}");
        assert!(text.contains("§6.3.2"), "{text}");
    }
}
