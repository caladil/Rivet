//! `rivet apply service-stop <name> confirm` and `rivet apply service-start
//! <name> confirm` — **pause one application without uninstalling it, and let
//! it run again.** `ADR-017` §6. *(`.rulings/` R-183.)*
//!
//! # What a stop is here
//!
//! A stop is a **hold** the supervisor keeps on one service. While it is held
//! the supervisor stops it, and does not start it again: not as a crash
//! restart, not on a backoff expiring, not on a reload. `service-start`
//! releases the hold and the service goes back to whatever the document says.
//!
//! **The hold lives in the supervisor's memory and nowhere else.** A reboot or a
//! restart of the supervisor releases it, and the machine comes back to its
//! desired state — the document still enables the service, because a stop
//! never writes `enabled`. That is the conservative reading of what an
//! operation is (`ADR-017` §6): it changes what the machine is doing, never
//! what it should be, so a machine nobody is watching returns to the record
//! on its next boot.
//!
//! # What it refuses
//!
//! * a name the supervisor does not know;
//! * a service in the reachability set — the control plane and the interface
//!   bring-up. Stopping one of those cuts the operator off with the hold still
//!   in place, and the only way back would be a power cycle;
//! * any other service from the image. Those are changed through the document,
//!   and this operation exists for applications;
//! * a service the document does not enable, which has nothing to stop;
//! * a stop of a service that is already held, and a start of one that is not.
//!   Neither reports success for having done nothing.
//!
//! # What it never does
//!
//! **It never touches `/state/app/<name>`.** The application's versions and its
//! data stay exactly where they are; only the process stops. `apply uninstall`
//! is the operation that removes files, and the two share no code path.
//!
//! # The wire format, both halves, here
//!
//! `super::service_restart`'s rule: tab-separated lines, writer and reader in
//! one module, round trip asserted. The request grammar is
//! [`crate::operation::service_restart::Request`], which is the supervisor
//! socket's whole vocabulary.

use super::machine::Board;
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, MachineError, OperationError, Verb};
use crate::value::Value;

/// The word that stops one service. Part of the interface contract.
pub const STOP_SUBJECT: &str = "service-stop";

/// The word that releases a stop. Part of the interface contract.
pub const START_SUBJECT: &str = "service-start";

/// Which of the two acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Stop the service and hold it stopped.
    Stop,
    /// Release the hold.
    Start,
}

impl Kind {
    /// The subject an operator types, and the wire word.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => STOP_SUBJECT,
            Self::Start => START_SUBJECT,
        }
    }

    /// Read a subject word, exactly.
    pub fn parse(word: &str) -> Option<Self> {
        [Self::Stop, Self::Start]
            .into_iter()
            .find(|k| k.as_str() == word)
    }

    /// The other one — named in every refusal that means *you wanted the
    /// other command*.
    pub const fn other(self) -> Self {
        match self {
            Self::Stop => Self::Start,
            Self::Start => Self::Stop,
        }
    }
}

impl core::fmt::Display for Kind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What taking or releasing the hold does from the state the supervisor found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doing {
    /// It is running, so it is signalled to stop, and it is not started again.
    Stop,
    /// It is not running, so nothing is signalled — and it is kept from
    /// starting, which a backoff or a failure would otherwise have led to.
    Hold,
    /// The hold is released and the document enables it, so the supervisor
    /// starts it.
    Start,
    /// The hold is released or the service is already going down, and nothing
    /// further is asked of it.
    Nothing,
}

impl Doing {
    /// The wire word. Never renamed.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Hold => "hold",
            Self::Start => "start",
            Self::Nothing => "nothing",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        [Self::Stop, Self::Hold, Self::Start, Self::Nothing]
            .into_iter()
            .find(|d| d.as_str() == word)
    }

    /// What it means, for a plan an operator reads.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Stop => "it is running, so it is signalled to stop and is NOT started again — \
                           not as a crash restart, not after a backoff, not on a reload",
            Self::Hold => "it is not running, so nothing is signalled; it is kept from starting \
                           until the hold is released",
            Self::Start => "the hold is released and the document enables it, so the supervisor \
                            starts it on its next pass",
            Self::Nothing => "nothing further is asked of it: either the hold is released and the \
                              document does not enable it, or it is already stopping",
        }
    }
}

/// **What the supervisor will do, or has done, about one hold.** The same type
/// on the plan path and the apply path, so the two are comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub kind: Kind,
    pub service: String,
    /// The state the supervisor found the service in, in the words `get status`
    /// uses.
    pub was: String,
    pub doing: Doing,
}

const HELD: &str = "held";
const REFUSED: &str = "refused";

impl Hold {
    /// The wire form: one line, tab-separated.
    ///
    /// ```text
    /// held\tservice-stop\tphoto-index\tstarted\tstop
    /// ```
    pub fn render(&self) -> String {
        format!(
            "{HELD}\t{}\t{}\t{}\t{}\n",
            self.kind.as_str(),
            self.service,
            self.was,
            self.doing.as_str(),
        )
    }

    /// The structured result, whose keys are `OUTPUTS`.
    pub fn value(&self, applied: bool) -> Value {
        let fields = vec![
            ("subject", Value::str(self.kind.as_str())),
            ("service", Value::str(self.service.clone())),
            ("was", Value::str(self.was.clone())),
            ("doing", Value::str(self.doing.as_str())),
            ("applied", Value::Bool(applied)),
            ("next", Value::str(next(self.kind, &self.service))),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "a service-stop or service-start result's keys must be exactly OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// Where the outcome appears, per act.
fn next(kind: Kind, service: &str) -> String {
    match kind {
        Kind::Stop => format!(
            "the supervisor stops it on its next pass, with ADR-017 §3's stop deadline and a \
             SIGKILL if it will not go, and `rivet {get} {status}` then shows it `idle` with the \
             hold as its reason. IT STAYS STOPPED until `rivet {apply} {START_SUBJECT} {service} \
             {confirm}`. A reboot, or a restart of the supervisor, also releases it, because the \
             document still enables it. Its files and data under /state/app are untouched.",
            get = Verb::Get,
            status = super::status::SUBJECT,
            apply = Verb::Apply,
            confirm = super::apply::CONFIRMATION,
        ),
        Kind::Start => format!(
            "the hold is gone, and the supervisor converges on its next pass by the document \
             alone. `rivet {get} {status}` is what says whether it is running; `rivet {get} log \
             {service}` carries the spawn and the pid.",
            get = Verb::Get,
            status = super::status::SUBJECT,
        ),
    }
}

/// The fields a result carries.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied: `service-stop` or `service-start`.",
    },
    Field {
        name: "service",
        description: "The service the supervisor acted on, echoed back.",
    },
    Field {
        name: "was",
        description: "The state the supervisor found it in, in the words `get status` uses.",
    },
    Field {
        name: "doing",
        description: "`stop` (running, signalled, not started again), `hold` (not running, kept \
                      from starting), `start` (released and started because the document \
                      enables it) or `nothing`.",
    },
    Field {
        name: "applied",
        description: "Whether this was the confirmed apply. It means the supervisor TOOK the \
                      request, not that the process has exited or started.",
    },
    Field {
        name: "next",
        description: "Where the outcome appears, and what releases a stop.",
    },
];

// ---------------------------------------------------------------------------
// The refusals
// ---------------------------------------------------------------------------

/// **Why the supervisor would not take or release the hold.** Facts only;
/// [`Self::describe`] composes the sentence here, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No service by that name.
    UnknownService { kind: Kind, service: String, known: Vec<String> },
    /// The machine is shutting down.
    ShuttingDown { kind: Kind, service: String },
    /// **A service that keeps the machine reachable.** Stopping it would cut the
    /// operator off with the hold still in place.
    CarriesTheWayIn { service: String },
    /// A service from the image rather than an installed application.
    NotAnApplication { service: String },
    /// The document does not enable it, so nothing is running to stop.
    NotEnabled { service: String },
    /// A stop of a service that is already held.
    AlreadyStopped { service: String },
    /// A start of a service that is not held.
    NotStopped { service: String },
}

impl Refusal {
    /// The wire word. Never renamed.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownService { .. } => "unknown-service",
            Self::ShuttingDown { .. } => "shutting-down",
            Self::CarriesTheWayIn { .. } => "carries-the-way-in",
            Self::NotAnApplication { .. } => "not-an-application",
            Self::NotEnabled { .. } => "not-enabled",
            Self::AlreadyStopped { .. } => "already-stopped",
            Self::NotStopped { .. } => "not-stopped",
        }
    }

    /// The act it refused.
    pub const fn kind(&self) -> Kind {
        match self {
            Self::UnknownService { kind, .. } | Self::ShuttingDown { kind, .. } => *kind,
            Self::CarriesTheWayIn { .. }
            | Self::NotAnApplication { .. }
            | Self::NotEnabled { .. }
            | Self::AlreadyStopped { .. } => Kind::Stop,
            Self::NotStopped { .. } => Kind::Start,
        }
    }

    /// The service it is about.
    pub fn service(&self) -> &str {
        match self {
            Self::UnknownService { service, .. }
            | Self::ShuttingDown { service, .. }
            | Self::CarriesTheWayIn { service }
            | Self::NotAnApplication { service }
            | Self::NotEnabled { service }
            | Self::AlreadyStopped { service }
            | Self::NotStopped { service } => service,
        }
    }

    /// **The sentence.** Every one says `REFUSED` first, cites the ruling, says
    /// `NOTHING WAS DONE`, and names what to do instead.
    pub fn describe(&self) -> String {
        let apply = Verb::Apply;
        let confirm = super::apply::CONFIRMATION;
        let get = Verb::Get;
        let status = super::status::SUBJECT;
        match self {
            Self::UnknownService { kind, service, known } => format!(
                "REFUSED to {kind} `{service}`: this machine's supervisor has no service by that \
                 name, so NOTHING WAS DONE. It knows {list}. `rivet {get} {status}` lists them \
                 (ADR-017 §6).",
                list = if known.is_empty() {
                    "none at all".to_string()
                } else {
                    known
                        .iter()
                        .map(|n| format!("`{n}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            ),
            Self::ShuttingDown { kind, service } => format!(
                "REFUSED to {kind} `{service}`: this machine is SHUTTING DOWN and every service is \
                 already coming down (ADR-017 §3). NOTHING WAS DONE — ask again once the machine \
                 is back up.",
            ),
            Self::CarriesTheWayIn { service } => format!(
                "REFUSED to stop `{service}`: it is one of the services that keep this machine \
                 REACHABLE. Stopping it would cut off the operator who asked, with the stop still \
                 held and nothing left that could release it but a power cycle (ADR-017 §6). \
                 NOTHING WAS DONE. To bounce it without losing the way in, `rivet {apply} \
                 {restart} {service} {confirm}` — a restart brings it back on the same pass.",
                restart = super::service_restart::SUBJECT,
            ),
            Self::NotAnApplication { service } => format!(
                "REFUSED to stop `{service}`: it is part of the IMAGE, not an installed \
                 application, and this operation pauses applications only (ADR-017 §6). NOTHING \
                 WAS DONE. What an image service does is the document's to say: `rivet {apply} \
                 {edit} /state/config/machine.toml ...` then `rivet {apply} {reload} {confirm}`.",
                edit = super::edit::SUBJECT,
                reload = super::reload::SUBJECT,
            ),
            Self::NotEnabled { service } => format!(
                "REFUSED to stop `{service}`: nothing enables it, so it is not running and there \
                 is nothing to stop (ADR-017 §6). NOTHING WAS DONE. `rivet {get} {status}` says \
                 what it is doing, and a display application runs only while it holds the \
                 screen.",
            ),
            Self::AlreadyStopped { service } => format!(
                "REFUSED to stop `{service}`: it is ALREADY STOPPED by `{apply} {STOP_SUBJECT}`, \
                 so NOTHING WAS DONE (ADR-017 §6). `rivet {apply} {START_SUBJECT} {service} \
                 {confirm}` releases it.",
            ),
            Self::NotStopped { service } => format!(
                "REFUSED to start `{service}`: it is NOT STOPPED by `{apply} {STOP_SUBJECT}`, and \
                 releasing that stop is all this operation does, so NOTHING WAS DONE (ADR-017 §6). \
                 To start an enabled service that is idle, failed or backing off, `rivet {apply} \
                 {restart} {service} {confirm}`.",
                restart = super::service_restart::SUBJECT,
            ),
        }
    }

    /// `Refused` for an unknown name — the machine was read and said why —
    /// and `Unsafe` for the rest, none of which changes on a retry.
    pub fn as_machine_error(&self) -> MachineError {
        match self {
            Self::UnknownService { .. } => MachineError::Refused(self.describe()),
            _ => MachineError::Unsafe(self.describe()),
        }
    }

    /// The wire form: one line, tab-separated — `refused`, the act, the
    /// service, the code, and the known list for an unknown name.
    pub fn render(&self) -> String {
        format!(
            "{REFUSED}\t{}\t{}\t{}\t{}\n",
            self.kind().as_str(),
            self.service(),
            self.code(),
            match self {
                Self::UnknownService { known, .. } => known.join(" "),
                _ => String::new(),
            },
        )
    }
}

/// **What the supervisor answered**, read back. [`Hold::render`] and
/// [`Refusal::render`] are the writers.
///
/// # Errors
///
/// A line this build cannot read is refused rather than guessed at.
pub fn parse_answer(text: &str) -> Result<Result<Hold, Refusal>, AnswerError> {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or(AnswerError::Empty)?;
    let fields: Vec<&str> = line.split('\t').collect();
    match fields.first().copied().unwrap_or_default() {
        HELD => {
            let [_, kind, service, was, doing] = fields.as_slice() else {
                return Err(AnswerError::Fields {
                    kind: HELD,
                    found: fields.len(),
                    wanted: 5,
                });
            };
            Ok(Ok(Hold {
                kind: parse_kind(kind)?,
                service: (*service).to_string(),
                was: (*was).to_string(),
                doing: Doing::parse(doing).ok_or_else(|| AnswerError::Value {
                    field: "doing",
                    found: (*doing).to_string(),
                })?,
            }))
        }
        REFUSED => {
            let [_, kind, service, code, rest] = fields.as_slice() else {
                return Err(AnswerError::Fields {
                    kind: REFUSED,
                    found: fields.len(),
                    wanted: 5,
                });
            };
            let kind = parse_kind(kind)?;
            let service = (*service).to_string();
            let refusal = match *code {
                "unknown-service" => Refusal::UnknownService {
                    kind,
                    service,
                    known: rest.split_whitespace().map(ToString::to_string).collect(),
                },
                "shutting-down" => Refusal::ShuttingDown { kind, service },
                "carries-the-way-in" => Refusal::CarriesTheWayIn { service },
                "not-an-application" => Refusal::NotAnApplication { service },
                "not-enabled" => Refusal::NotEnabled { service },
                "already-stopped" => Refusal::AlreadyStopped { service },
                "not-stopped" => Refusal::NotStopped { service },
                other => {
                    return Err(AnswerError::Value {
                        field: "code",
                        found: other.to_string(),
                    });
                }
            };
            Ok(Err(refusal))
        }
        other => Err(AnswerError::UnknownLine {
            kind: other.to_string(),
        }),
    }
}

fn parse_kind(word: &str) -> Result<Kind, AnswerError> {
    Kind::parse(word).ok_or_else(|| AnswerError::Value {
        field: "subject",
        found: word.to_string(),
    })
}

/// An answer that could not be read. The same four shapes
/// `super::service_restart` reads.
pub use super::service_restart::AnswerError;

// ---------------------------------------------------------------------------
// The operation
// ---------------------------------------------------------------------------

/// **The change one of the two subjects is.** The plan is the supervisor's
/// own answer, so the refusals happen before the confirmation is typed.
struct HoldService {
    kind: Kind,
    service: String,
}

impl Change for HoldService {
    type Prepared = Hold;
    type Applied = Hold;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Hold), OperationError> {
        let preview = board
            .can_request_service_hold(self.kind, &self.service)
            .map_err(|e| refused(e, self.kind))?;

        let summary = match self.kind {
            Kind::Stop => format!(
                "stops `{}` and holds it stopped until it is started again",
                self.service
            ),
            Kind::Start => format!(
                "releases the stop on `{}` and lets the document decide again",
                self.service
            ),
        };
        let verb = match self.kind {
            Kind::Stop => "stop it",
            Kind::Start => "start it",
        };
        let mut plan = Plan::new(self.kind.as_str(), summary)
            .doing(
                Effect::request(
                    self.service.clone(),
                    format!(
                        "{verb} — the supervisor found it `{was}`, so {what}",
                        was = preview.was,
                        what = preview.doing.describe(),
                    ),
                )
                .noting(
                    "the hold lives in the supervisor's memory: a reboot or a restart of the \
                     supervisor releases it, and the machine returns to what the document says \
                     (ADR-017 §6)",
                ),
            )
            .leaving(
                "the desired-state document — `enabled` is configuration and this is an \
                 operation, so nothing about what this machine SHOULD run is changed",
            );
        if self.kind == Kind::Stop {
            plan = plan.leaving(format!(
                "/state/app/{} — the application's installed versions and its data are not \
                 touched; only the process stops. `apply uninstall` is what removes files",
                self.service
            ));
        }
        let plan = plan
            .leaving("every other service — this names one")
            .leaving("NOTHING HAS BEEN ASKED — the request is sent only if this is confirmed")
            .taking_effect(TakesEffect::Immediately)
            .confirmed_by(&[self.kind.as_str(), &self.service]);
        Ok((plan, preview))
    }

    fn apply(self, _: Hold, board: &mut dyn Board) -> Result<Hold, OperationError> {
        board
            .request_service_hold(self.kind, &self.service)
            .map_err(|e| refused(e, self.kind))
    }
}

/// Perform one of the two. `arguments[0]` is the subject already read into
/// `kind`.
pub(super) fn perform(
    kind: Kind,
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Hold, OperationError> {
    let Some(service) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            kind.as_str(),
            format!(
                "`{apply} {kind}` needs the name of ONE application. There is no form of it that \
                 acts on everything. `rivet {get} {status}` lists what this machine runs.",
                apply = Verb::Apply,
                get = Verb::Get,
                status = super::status::SUBJECT,
            ),
        ));
    };
    if let Some(extra) = arguments.get(3).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {kind}` takes one service and then the confirmation. `{extra}` is a \
                 fourth word: run it once per service, so each is confirmed on its own.",
                apply = Verb::Apply,
            ),
        ));
    }
    if crate::service::ServiceName::new(service).is_none() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            service,
            format!(
                "{service:?} cannot be a service name — it has a path separator or a control \
                 character — so nothing was asked."
            ),
        ));
    }
    let confirmed = super::apply::confirmed(arguments.get(2).copied())?;
    resolve(
        HoldService {
            kind,
            service: service.to_string(),
        },
        confirmed,
        board,
    )
}

fn refused(error: MachineError, kind: Kind) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, kind.as_str(), error.why())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::operation::machine::{Machine, Reboot, Stop};
    use crate::operation::{Act, apply::CONFIRMATION};
    use crate::state::Slot;

    /// A board that answers the hold port as scripted and refuses the rest.
    struct FakeBoard {
        answer: Result<Hold, Refusal>,
        planned: Vec<Kind>,
        applied: Vec<Kind>,
    }

    impl FakeBoard {
        fn saying(answer: Result<Hold, Refusal>) -> Self {
            Self {
                answer,
                planned: Vec::new(),
                applied: Vec::new(),
            }
        }

        fn healthy(kind: Kind, was: &str, doing: Doing) -> Self {
            Self::saying(Ok(Hold {
                kind,
                service: "photo-index".to_string(),
                was: was.to_string(),
                doing,
            }))
        }
    }

    fn nope() -> MachineError {
        MachineError::Refused("this fake answers only the service hold".to_string())
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
        ) -> Result<super::super::Restarting, MachineError> {
            Err(nope())
        }
        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<super::super::Restarting, MachineError> {
            Err(nope())
        }
        fn can_request_service_hold(
            &mut self,
            kind: Kind,
            _service: &str,
        ) -> Result<Hold, MachineError> {
            self.planned.push(kind);
            self.answer.clone().map_err(|r| r.as_machine_error())
        }
        fn request_service_hold(&mut self, kind: Kind, _service: &str) -> Result<Hold, MachineError> {
            self.applied.push(kind);
            self.answer.clone().map_err(|r| r.as_machine_error())
        }
    }

    /// **The naming.** Neither subject shares a word with a machine stop, and
    /// neither is a prefix of the other, so a truncated command reaches
    /// nothing rather than the wrong thing.
    #[test]
    fn the_two_subjects_cannot_be_confused_with_each_other_or_with_the_machine() {
        for kind in [Kind::Stop, Kind::Start] {
            let word = kind.as_str();
            assert_eq!(Kind::parse(word), Some(kind));
            for machine in Stop::ALL {
                assert!(
                    !word.starts_with(machine.as_str()) && !machine.as_str().starts_with(word),
                    "{word} and {machine} share a prefix"
                );
            }
            assert_ne!(word, super::super::service_restart::SUBJECT);
        }
        assert!(!STOP_SUBJECT.starts_with(START_SUBJECT));
        assert!(!START_SUBJECT.starts_with(STOP_SUBJECT));
        for near in ["stop", "start", "service-pause", "Service-Stop", "service_stop", ""] {
            assert_eq!(Kind::parse(near), None, "{near:?} was accepted");
        }
    }

    /// Without the confirmation it is a plan: nothing is asked, and the plan
    /// says the application's files are untouched and what releases the hold.
    #[test]
    fn without_the_confirmation_nothing_is_asked_and_the_plan_says_what_is_untouched() {
        let mut board = FakeBoard::healthy(Kind::Stop, "started", Doing::Stop);
        let refusal =
            perform(Kind::Stop, &[STOP_SUBJECT, "photo-index"], &mut board).expect_err("a plan");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert_eq!(board.planned, vec![Kind::Stop]);
        assert!(board.applied.is_empty(), "a stop was sent without a yes");

        let plan = refusal.plan().expect("a confirmation refusal carries its plan");
        let [effect] = plan.effects() else {
            panic!("one service, one effect: {:?}", plan.effects());
        };
        assert_eq!(effect.act, Act::Request);
        assert_eq!(effect.target, "photo-index");
        assert!(effect.to.clone().unwrap_or_default().contains("started"));
        assert_eq!(
            plan.confirm_with(),
            format!("rivet apply {STOP_SUBJECT} photo-index {CONFIRMATION}")
        );
        assert_eq!(plan.takes_effect().requires_reboot(), Some(false));
        let untouched = plan.untouched().join("\n");
        assert!(untouched.contains("/state/app/photo-index"), "{untouched}");
        assert!(untouched.contains("desired-state document"), "{untouched}");
    }

    /// With the word, the request is sent once, after the plan ran.
    #[test]
    fn with_the_confirmation_the_request_is_sent_after_the_plan() {
        for (kind, doing) in [(Kind::Stop, Doing::Stop), (Kind::Start, Doing::Start)] {
            let mut board = FakeBoard::healthy(kind, "idle", doing);
            let done = perform(kind, &[kind.as_str(), "photo-index", CONFIRMATION], &mut board)
                .expect("a healthy board");
            assert_eq!(board.planned, vec![kind]);
            assert_eq!(board.applied, vec![kind]);
            assert_eq!(done.doing, doing);
            let Value::Object(fields) = done.value(true) else {
                panic!("a result is an object");
            };
            let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
            let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
            assert_eq!(keys, declared);
        }
    }

    /// **A stop's result says it stays stopped and names what releases it.**
    #[test]
    fn a_stop_result_says_it_stays_stopped_and_what_releases_it() {
        let text = next(Kind::Stop, "photo-index");
        assert!(text.contains("STAYS STOPPED"), "{text}");
        assert!(text.contains("apply service-start photo-index confirm"), "{text}");
        assert!(text.contains("reboot"), "{text}");
        assert!(text.contains("untouched"), "{text}");
    }

    /// Every refusal happens on the plan path, so nothing is ever sent.
    #[test]
    fn every_refusal_stops_the_plan_and_nothing_is_sent() {
        for refusal in every_refusal() {
            let kind = refusal.kind();
            let mut board = FakeBoard::saying(Err(refusal.clone()));
            let got = perform(kind, &[kind.as_str(), "photo-index", CONFIRMATION], &mut board)
                .expect_err("refused");
            let wanted = match refusal {
                Refusal::UnknownService { .. } => ErrorCode::MachineRefused,
                _ => ErrorCode::Unsafe,
            };
            assert_eq!(got.code(), wanted, "{refusal:?}");
            assert!(board.applied.is_empty(), "{refusal:?} still sent the request");
        }
    }

    /// Arguments: a bare subject, a fourth word, and a name that is a path.
    #[test]
    fn arguments_are_refused_before_the_machine_is_asked() {
        for kind in [Kind::Stop, Kind::Start] {
            let mut board = FakeBoard::healthy(kind, "started", Doing::Stop);
            let bare = perform(kind, &[kind.as_str()], &mut board).expect_err("bare");
            assert_eq!(bare.code(), ErrorCode::Arguments);
            let fourth = perform(
                kind,
                &[kind.as_str(), "photo-index", CONFIRMATION, "other"],
                &mut board,
            )
            .expect_err("fourth");
            assert_eq!(fourth.code(), ErrorCode::Arguments);
            assert_eq!(fourth.subject(), Some("other"));
            let path = perform(kind, &[kind.as_str(), "../../etc", CONFIRMATION], &mut board)
                .expect_err("a path");
            assert_eq!(path.code(), ErrorCode::Arguments);
            assert!(board.planned.is_empty(), "the machine was asked");
        }
    }

    fn every_refusal() -> Vec<Refusal> {
        let s = || "photo-index".to_string();
        vec![
            Refusal::UnknownService {
                kind: Kind::Stop,
                service: s(),
                known: vec!["a".to_string(), "b".to_string()],
            },
            Refusal::UnknownService {
                kind: Kind::Start,
                service: s(),
                known: Vec::new(),
            },
            Refusal::ShuttingDown {
                kind: Kind::Stop,
                service: s(),
            },
            Refusal::ShuttingDown {
                kind: Kind::Start,
                service: s(),
            },
            Refusal::CarriesTheWayIn { service: s() },
            Refusal::NotAnApplication { service: s() },
            Refusal::NotEnabled { service: s() },
            Refusal::AlreadyStopped { service: s() },
            Refusal::NotStopped { service: s() },
        ]
    }

    /// **Every refusal says so first, cites the ruling, and says nothing was
    /// done** — and contains no tab, or its own wire form would break.
    #[test]
    fn every_refusal_names_itself_and_cites_something() {
        for refusal in every_refusal() {
            let text = refusal.describe();
            assert!(text.starts_with("REFUSED"), "{text}");
            assert!(text.contains("ADR-017"), "{text}");
            assert!(text.contains("NOTHING WAS DONE"), "{text}");
            assert!(!text.contains('\t'), "{text}");
        }
    }

    /// **The control plane's refusal names what to do instead**, which is a
    /// restart: the one way to bounce it that brings it back.
    #[test]
    fn stopping_the_way_in_is_refused_and_points_at_a_restart() {
        let text = Refusal::CarriesTheWayIn {
            service: "rivet-mcpd".to_string(),
        }
        .describe();
        assert!(text.contains("REACHABLE"), "{text}");
        assert!(text.contains("apply service-restart rivet-mcpd confirm"), "{text}");
    }

    /// **The round trip, both kinds, every answer.**
    #[test]
    fn what_the_supervisor_writes_is_what_a_reader_reads() {
        for kind in [Kind::Stop, Kind::Start] {
            for doing in [Doing::Stop, Doing::Hold, Doing::Start, Doing::Nothing] {
                let hold = Hold {
                    kind,
                    service: "photo-index".to_string(),
                    was: "backoff".to_string(),
                    doing,
                };
                assert_eq!(parse_answer(&hold.render()), Ok(Ok(hold)));
            }
        }
        for refusal in every_refusal() {
            assert_eq!(parse_answer(&refusal.render()), Ok(Err(refusal)));
        }
    }

    /// An answer this build cannot read is refused, never guessed at.
    #[test]
    fn an_answer_this_build_does_not_know_is_refused() {
        assert_eq!(parse_answer(""), Err(AnswerError::Empty));
        assert!(matches!(
            parse_answer("paused\tphoto-index\n"),
            Err(AnswerError::UnknownLine { .. })
        ));
        assert!(matches!(
            parse_answer("held\tservice-stop\tphoto-index\n"),
            Err(AnswerError::Fields { .. })
        ));
        assert!(matches!(
            parse_answer("held\tservice-pause\tphoto-index\tidle\thold\n"),
            Err(AnswerError::Value { field: "subject", .. })
        ));
        assert!(matches!(
            parse_answer("refused\tservice-stop\tphoto-index\tvanished\t\n"),
            Err(AnswerError::Value { field: "code", .. })
        ));
    }
}
