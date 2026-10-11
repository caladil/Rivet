//! `rivet apply reload confirm` — **ask this machine to re-read its
//! configuration.** `ADR-032` §3.2, and the sending half of it.
//!
//! `PLAN.md` §28.5 Phase 1 item 4, finished 2026-08-26. `cmd/rivet-sysd`'s
//! `reload` module is the receiving half and was built first; until this
//! existed, **nothing on a Rivet board could send the signal** — there is no
//! shell, no `kill`, and `cmd/rivet` executes nothing. That
//! is exactly the state `ADR-017` §3.1.10 found the three stop signals in and
//! called *receivable*.
//!
//! # Why an operation, and not a bare word
//!
//! **Ruled by the maintainer 2026-08-26**: the command line is the source of the
//! operation, and an MCP server made for an agent would be another place to
//! reach it, so the request has to be reachable from there too (`.rulings/`
//! R-361).
//!
//! That is `ADR-006` §3.1, arrived at from the outside. An adapter is a
//! projection of `crate::operation::CATALOGUE` — §3.3's MCP row has no logic of
//! its own — so a reload delivered by a word `cmd/rivet` matched itself would be
//! reachable from the login shell and from nowhere else, which §3.1 names as
//! *"a defect in that interface, not a feature of it"*. As a subject of `apply`
//! it costs one match arm and every adapter gets it at once.
//!
//! `crate::operation::machine::Machine::request_reload` is the port. **What
//! carries the request is the adapter's**, and deliberately not described here:
//! the domain knows *ask the supervisor to re-read*, and whether that is a
//! signal, a socket or a file is a platform's business (`AGENTS.md` §3.1).
//!
//! # It is a subject of `apply`, and that is the eighth
//!
//! `ADR-006` §6.4's verb set is closed and `reload` is not in it. §8.4 priced
//! this shape and it has now fired eight times: *"`subject` is a declared input
//! rather than a second axis of the catalogue, so a second subject is an added
//! match arm."*
//!
//! # What its plan has to say, and why the list is long
//!
//! A reload applies **less** than the document says (`ADR-032` §3.2.1), and the
//! whole point of `ADR-006` §6.5's plan is that an operator finds that out
//! *before* confirming rather than by reading the journal afterwards. So the
//! withheld set is `untouched` entries rather than a sentence — one per reason,
//! because an agent branching on one should not have to parse the others.

use super::apply::confirmed;
use super::machine::Board;
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, MachineError, OperationError, Verb};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "reload";

/// The service the request reaches. **The supervisor's name, in the domain's
/// vocabulary** — `crate::deploy::SERVICE_NAME` is the installer's and this is
/// not that.
pub const SUPERVISOR: &str = "rivet-sysd";

/// What this subject's result carries.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `reload` here.",
    },
    Field {
        name: "requested",
        description: "Always true in a result. It means the request was DELIVERED, not that the \
                      document has been re-read (ADR-006 §4.2: a request that was not delivered \
                      is a refusal, never a result with a false in it).",
    },
    Field {
        name: "applied",
        description: "Always false. The supervisor re-reads on its next pass, and this operation \
                      cannot observe that — ADR-032 §3.2's signal carries no reply.",
    },
    Field {
        name: "withheld",
        description: "What a reload does NOT apply, and it is not an error: the services that \
                      keep this machine reachable, and memory caps. ADR-032 §3.2.1 is the \
                      ruling and each entry says what to do instead.",
    },
    Field {
        name: "next",
        description: "Where the outcome appears. A signal cannot answer, so the journal is the \
                      only account there is.",
    },
];

/// **What a reload will not do**, in the operator's words rather than the
/// supervisor's. One entry per reason.
///
/// The supervisor journals each of these again when it declines to apply one —
/// `cmd/rivet-sysd`'s `document::apply_on`. That is two statements of one rule
/// and it is deliberate rather than a duplication: this one is *before*, so an
/// operator can choose differently, and that one is *after*, naming the value
/// actually seen. `ADR-011` §2's second clause — the system says which one, at
/// the moment the answer is needed — and the moments are different.
pub fn withheld() -> Vec<String> {
    vec![
        format!(
            "the services that keep this machine reachable — a reload will not stop the CONTROL \
             PLANE or the interface bring-up, because doing so takes away the means of undoing \
             it (ADR-032 §3.2.1, ADR-018 §8). Their `enabled` is taken at the next boot; \
             `rivet {apply} restart {confirm}` is how to take it now",
            apply = Verb::Apply,
            confirm = super::apply::CONFIRMATION,
        ),
        // **Corrected 2026-08-29.** This said *"Each takes effect the next time
        // its service starts"*, which was true only while nothing could start a
        // service without a reboot. `ADR-017` §6.3.2's `apply service-restart`
        // can, and it writes the cap the DEFINITION holds — which a reload
        // deliberately did not update. Struck rather than reworded is not
        // available in a runtime string, so the correction is here and the
        // stale sentence is quoted in this comment (`AGENTS.md` §7).
        format!(
            "memory caps — a cap is written into a cgroup when a service starts (ADR-017 \
             §3.1.3), so changing it here would report a limit the running process does not \
             have. Each is taken AT THE NEXT BOOT, not at the next start of its service: \
             `{apply} {SERVICE_RESTART}` writes the cap the definition already holds, which is \
             the old one. `rivet {apply} restart {confirm}` is how to take a new cap now",
            apply = Verb::Apply,
            SERVICE_RESTART = super::service_restart::SUBJECT,
            confirm = super::apply::CONFIRMATION,
        ),
    ]
}

/// A reload request that was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reloading;

impl Reloading {
    /// The structured result, whose keys are [`OUTPUTS`] — checked here rather
    /// than only in a test, because the catalogue naming a field no result
    /// carries is `ADR-006` §7's "trusted and wrong".
    pub fn value(self) -> Value {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("requested", Value::Bool(true)),
            ("applied", Value::Bool(false)),
            ("withheld", Value::strings(withheld())),
            (
                "next",
                Value::str(format!(
                    "the request was DELIVERED and the document may not have been re-read yet. \
                     ADR-032 §3.2's signal carries no reply, so the journal is the whole of the \
                     answer: `rivet {get} log {SUPERVISOR}` shows what was re-read, what changed \
                     and what was withheld. A document that will not parse leaves this machine \
                     on the last good one and says so at error severity.",
                    get = Verb::Get,
                )),
            ),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "a reload result's keys must be exactly OUTPUTS, in the declared order"
        );
        Value::object(fields)
    }
}

/// **The change this subject is**, as `ADR-006` §6.5's framework defines one.
struct RequestReload;

impl Change for RequestReload {
    /// Nothing. A reload takes no arguments at all — it is the whole document or
    /// none of it, which is `ADR-007` §3's desired-state model and the reason
    /// this subject has no noun.
    type Prepared = ();
    type Applied = Reloading;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, ()), OperationError> {
        board.can_request_reload().map_err(refused)?;

        let mut plan = Plan::new(SUBJECT, "makes this machine re-read its configuration")
            .doing(
                Effect::request(SUPERVISOR, "re-read the desired-state document").noting(
                    "the supervisor applies it on its next convergence pass; a document that \
                     will not parse leaves this machine on the last good one and says so at \
                     error severity (ADR-032 §3.2)",
                ),
            )
            .leaving("NOTHING HAS BEEN SENT — the request is delivered only if this is confirmed")
            // **Immediately, and the exceptions are the `untouched` list.** What
            // a reload takes, it takes at once; what it withholds it does not
            // take at all until the next boot. Answering `not-asserted` for the
            // whole operation would hide the half that IS immediate, which is
            // the half an operator confirmed it for.
            .taking_effect(TakesEffect::Immediately)
            .confirmed_by(&[SUBJECT]);

        for line in withheld() {
            plan = plan.leaving(line);
        }
        Ok((plan, ()))
    }

    fn apply(self, (): (), board: &mut dyn Board) -> Result<Reloading, OperationError> {
        board.request_reload().map_err(refused)?;
        Ok(Reloading)
    }
}

/// Perform it. `arguments[0]` is the subject.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Reloading, OperationError> {
    debug_assert_eq!(arguments.first().copied(), Some(SUBJECT));

    // **A reload takes no noun**, so a second word is not a misplaced argument —
    // it is a word this command has no meaning for. Refused rather than ignored,
    // which is `super::stop`'s rule and the same hazard: `rivet apply reload
    // rivet-netd confirm` must not quietly reload everything while looking like it
    // reloaded one service.
    if let Some(extra) = arguments.get(2).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {SUBJECT}` takes only the confirmation after it. `{extra}` is a third \
                 word, and there is no form of this command that reloads part of a document: \
                 the document is the desired state of the whole machine (ADR-007 §3).",
                apply = Verb::Apply,
            ),
        ));
    }

    let confirmed = confirmed(arguments.get(1).copied())?;
    resolve(RequestReload, confirmed, board)
}

/// The one place a [`MachineError`] becomes this operation's refusal — the same
/// mapping every other subject uses, and nothing is reworded on the way through.
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
    use crate::operation::machine::Machine;
    use crate::operation::{Act, apply::CONFIRMATION};
    use crate::state::Slot;

    /// A board that answers this operation's two methods and refuses the rest.
    #[derive(Default)]
    struct FakeBoard {
        interlock: Option<MachineError>,
        deliver: Option<MachineError>,
        asked: u32,
        planned: u32,
    }

    fn nope() -> MachineError {
        MachineError::Refused("this fake answers only the reload methods".to_string())
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
        fn arm_trial_boot(
            &mut self,
            _slot: Slot,
        ) -> Result<crate::operation::machine::Reboot, MachineError> {
            Err(nope())
        }
        fn can_request_stop(
            &mut self,
            _how: crate::operation::machine::Stop,
        ) -> Result<(), MachineError> {
            Err(nope())
        }
        fn request_stop(
            &mut self,
            _how: crate::operation::machine::Stop,
        ) -> Result<(), MachineError> {
            Err(nope())
        }
        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            self.planned += 1;
            self.interlock.clone().map_or(Ok(()), Err)
        }
        fn request_reload(&mut self) -> Result<(), MachineError> {
            self.asked += 1;
            self.deliver.clone().map_or(Ok(()), Err)
        }
        fn can_request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(nope())
        }
        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(nope())
        }
    }

    /// Without the confirmation it is a plan, and nothing is sent.
    #[test]
    fn without_the_confirmation_nothing_is_asked() {
        let mut board = FakeBoard::default();
        let refusal = perform(&[SUBJECT], &mut board).expect_err("it reloaded unconfirmed");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert_eq!(board.asked, 0, "the request was delivered without a yes");
        assert_eq!(board.planned, 1, "the interlock did not run on the plan path");
    }

    /// **The plan names everything a reload will NOT do.** `ADR-032` §3.2.1,
    /// and it is the reason this subject's plan is worth reading: a reload
    /// applies less than the document says, and an operator must find that out
    /// before confirming rather than in the journal afterwards.
    #[test]
    fn the_plan_says_what_a_reload_withholds_and_why() {
        let mut board = FakeBoard::default();
        let refusal = perform(&[SUBJECT], &mut board).expect_err("a plan");
        let plan = refusal.plan().expect("a confirmation refusal carries its plan");

        let [effect] = plan.effects() else {
            panic!("a reload asks for exactly one thing: {:?}", plan.effects());
        };
        assert_eq!(effect.act, Act::Request);
        assert_eq!(effect.target, SUPERVISOR);
        assert_eq!(plan.takes_effect().requires_reboot(), Some(false));
        assert_eq!(plan.confirm_with(), format!("rivet apply reload {CONFIRMATION}"));

        // One entry per reason, and each names what to do instead.
        let untouched = plan.untouched().join("\n");
        for expected in ["reachable", "memory caps"] {
            assert!(untouched.contains(expected), "{expected} is not named: {untouched}");
        }
        assert!(untouched.contains("next boot"), "{untouched}");
    }

    /// With the word, the request is delivered.
    #[test]
    fn with_the_confirmation_the_request_is_delivered() {
        let mut board = FakeBoard::default();
        let requested = perform(&[SUBJECT, CONFIRMATION], &mut board).expect("a healthy board");
        assert_eq!(board.asked, 1);
        // The interlock runs before the act it guards — `super::plan::resolve`
        // plans on both paths.
        assert_eq!(board.planned, 1);
        // `applied` is false and says so: a signal carries no reply.
        let Value::Object(fields) = requested.value() else {
            panic!("a result is an object");
        };
        assert!(
            fields
                .iter()
                .any(|(k, v)| *k == "applied" && *v == Value::Bool(false)),
            "{fields:?}"
        );
    }

    /// A second word is refused rather than ignored. There is no form of this
    /// that reloads part of a document.
    #[test]
    fn a_word_after_the_confirmation_is_refused_rather_than_ignored() {
        let mut board = FakeBoard::default();
        let refusal = perform(&[SUBJECT, CONFIRMATION, "rivet-netd"], &mut board)
            .expect_err("a third word must be refused");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert_eq!(refusal.subject(), Some("rivet-netd"));
        assert_eq!(board.asked, 0);
    }

    /// A machine with no supervisor to ask refuses, and the refusal is the
    /// machine's own words.
    #[test]
    fn a_machine_with_no_supervisor_refuses_and_says_what_it_found() {
        let mut board = FakeBoard {
            interlock: Some(MachineError::Unsafe("pid 1 is `systemd`".to_string())),
            ..FakeBoard::default()
        };
        let refusal = perform(&[SUBJECT, CONFIRMATION], &mut board).expect_err("no supervisor");
        assert_eq!(refusal.code(), ErrorCode::Unsafe);
        assert!(refusal.message().contains("systemd"), "{}", refusal.message());
        assert_eq!(board.asked, 0, "it asked anyway");
    }
}
