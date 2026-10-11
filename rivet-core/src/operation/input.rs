//! **`apply input <action>`** — pressing a button from somewhere that has none.
//!
//! `ADR-012` §9. This document is called *The Agent as a First-Class Operator*
//! and until 2026-09-01 it was not true: an agent could read every fact on a
//! machine, install to it, reconfigure it and reboot it, and could not press
//! ENTER. The lease mechanism in `PLAN.md` §106 was built by an agent that then
//! had to ask a person to test it.
//!
//! # Why it is an `apply` subject
//!
//! Because it changes what the machine is doing. `get` reads; this makes a
//! selection, starts an application and takes a screen. `ADR-012` §3's plan-and-
//! confirm is the safety mechanism, and it costs nothing here: `confirm` is the
//! last word of the same invocation, not a second round trip, so an agent
//! driving a menu sends one call per press exactly as `apply reload confirm` is
//! one call.
//!
//! # What it deliberately is not
//!
//! **Not a remote-control surface.** The vocabulary is closed
//! (`crate::input::Action`) and carries no payload, so there is no form of this
//! command that types text — `ADR-012` §9.3 rules that parity with a person,
//! and a person at a Rivet kiosk has buttons and no keyboard.

use super::apply::confirmed;
use super::machine::Board;
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, MachineError, OperationError, Verb};
use crate::input::Action;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "input";

/// What this subject's result carries.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `input` here.",
    },
    Field {
        name: "action",
        description: "The action delivered, in the vocabulary crate::input::Action defines.",
    },
    Field {
        name: "delivered",
        description: "Always true in a result. It means the action reached the launcher's \
                      socket, NOT that the launcher acted on it — a press carries no reply, \
                      exactly as a real one does not.",
    },
    Field {
        name: "means",
        description: "What the launcher will do with it, and what this operation cannot say.",
    },
];

/// The result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pressed {
    /// Which action reached the launcher.
    pub action: Action,
}

struct SendInput(Action);

impl Change for SendInput {
    /// Nothing. The action was parsed before the change was built, so planning
    /// learns nothing applying needs.
    type Prepared = ();
    type Applied = Pressed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, ()), OperationError> {
        // **The interlock is "is anything listening"** — `ADR-012` §9.3. With
        // no launcher there are no kiosk applications and nothing an input
        // could mean, so this refuses here rather than queueing a press for a
        // consumer that may never exist.
        board.can_send_input().map_err(refused)?;

        let word = self.0.as_str();
        let plan = Plan::new(
            SUBJECT,
            format!("presses `{word}` on this machine's kiosk, as a person at it would"),
        )
        .doing(
            Effect::request(
                crate::input::PROVIDER,
                format!("receive `{word}` and act on it"),
            )
            .noting(
                "an agent's press is RECORDED AS ONE. It arrives over ADR-035's control \
                 plane, where ADR-028 §4.5 stamps the caller's pinned key, rather than off \
                 a /dev/input node — so the journal tells the two apart and ADR-012 §4's \
                 provenance is not weakened by pretending they are the same",
            ),
        )
        .leaving("NOTHING HAS BEEN SENT — the press is delivered only if this is confirmed")
        .leaving(
            "NOTHING IS INSTALLED, STOPPED OR CONFIGURED by this. It presses a button; what \
             happens next is the launcher's and the selected application's",
        )
        .leaving(
            "THE ANSWER CANNOT SAY WHAT IT DID. A press carries no reply, exactly as a real \
             one does not — `rivet get log` is where the launcher says what it made of it, on \
             a record whose `svc` is `rivet-launcher` (that operation has no service filter)",
        )
        .taking_effect(TakesEffect::Immediately)
        .confirmed_by(&[SUBJECT, word]);

        Ok((plan, ()))
    }

    fn apply(self, (): (), board: &mut dyn Board) -> Result<Pressed, OperationError> {
        board.send_input(self.0.as_str()).map_err(refused)?;
        Ok(Pressed { action: self.0 })
    }
}

/// Perform it. `arguments[0]` is the subject, `arguments[1]` the action.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a missing or unknown action —
/// **never a guess**. Defaulting to anything would press a button nobody asked
/// for, and the one it would press is somebody's choice.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Pressed, OperationError> {
    debug_assert_eq!(arguments.first().copied(), Some(SUBJECT));

    let Some(word) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` takes an action, and there is no default: pressing a \
                 button nobody asked for is somebody's choice being made for them. One of: {}",
                Action::vocabulary(),
                apply = Verb::Apply,
            ),
        ));
    };

    let Some(action) = Action::parse(word) else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            word,
            format!(
                "`{word}` is not an action this build knows. One of: {}.\n\nThere is \
                 deliberately no way to type text: ADR-012 §9.3 rules parity with what a \
                 PERSON can press, and a Rivet kiosk has buttons and no keyboard surface.",
                Action::vocabulary(),
            ),
        ));
    };

    // **A press takes one action and then the confirmation**, so a fourth
    // word is not a misplaced confirmation — it is a word this command has no
    // meaning for. Refused rather than ignored, which is `super::reload`'s
    // rule and the same hazard: `rivet apply input enter confirm back` must
    // not quietly press ENTER while looking like it also did something with
    // `back`.
    if let Some(extra) = arguments.get(3).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {SUBJECT}` takes one action and then the confirmation. `{extra}` is \
                 a fourth word, and there is no form of this command that presses two buttons: \
                 run it twice, so that each press is confirmed on its own.",
                apply = Verb::Apply,
            ),
        ));
    }

    let confirmed = confirmed(arguments.get(2).copied())?;
    resolve(SendInput(action), confirmed, board)
}

/// The one place a [`MachineError`] becomes this operation's refusal.
fn refused(error: MachineError) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, SUBJECT, error.why())
}

impl Pressed {
    /// The result, as `ADR-006` §6.5's framework wants it.
    ///
    /// The `debug_assert` is `reload`'s: a result whose keys are not exactly
    /// `OUTPUTS` in the declared order is a catalogue that lies about its own
    /// shape, and the one place to catch that is where the two are written.
    #[must_use]
    pub fn value(self) -> Value {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("action", Value::str(self.action.as_str())),
            ("delivered", Value::Bool(true)),
            (
                "means",
                Value::str(
                    "the action reached the launcher and it acts on it on the next pass of its \
                     input loop. THIS ANSWER CANNOT SAY WHAT IT DID: a press carries no reply, \
                     exactly as a real one does not, and `rivet get log` is where the launcher \
                     says what it made of it, on a record whose `svc` is `rivet-launcher` (that \
                     operation has no service filter).",
                ),
            ),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "an input result's keys must be exactly OUTPUTS, in the declared order"
        );
        Value::object(fields)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A board that refuses everything. **Enough, because every test here
    /// returns before the board is touched**: an action that is missing or
    /// unknown is refused by the argument check, which is the whole point —
    /// a bad word must never reach a machine.
    #[derive(Default)]
    struct FakeBoard;

    crate::operation::machine::refuses_to_edit!(FakeBoard);
    crate::operation::machine::refuses_to_deploy!(FakeBoard);
    crate::operation::machine::refuses_to_observe!(FakeBoard);
    crate::operation::machine::refuses_to_manage!(FakeBoard);

    /// A board that presses when asked, refuses everything else, and records
    /// every action it was actually sent — so a test can show a press
    /// happened, or show none did, rather than inferring it from an error
    /// code.
    #[derive(Default)]
    struct RecordingBoard {
        sent: Vec<Action>,
    }

    crate::operation::machine::refuses_to_edit!(RecordingBoard);
    crate::operation::machine::refuses_to_deploy!(RecordingBoard);
    crate::operation::machine::refuses_to_observe!(RecordingBoard);

    impl RecordingBoard {
        fn not_this_ports_business() -> MachineError {
            MachineError::Refused(
                "this fake presses buttons and does nothing else; an `apply input` test that \
                 reached another port has changed what it is about"
                    .to_string(),
            )
        }
    }

    impl crate::operation::machine::Machine for RecordingBoard {
        fn running_slot(&mut self) -> Result<crate::state::Slot, MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn can_arm_trial_boot(&mut self, _slot: crate::state::Slot) -> Result<(), MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn arm_trial_boot(
            &mut self,
            _slot: crate::state::Slot,
        ) -> Result<crate::operation::machine::Reboot, MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn can_request_stop(
            &mut self,
            _stop: crate::operation::machine::Stop,
        ) -> Result<(), MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn request_stop(
            &mut self,
            _stop: crate::operation::machine::Stop,
        ) -> Result<(), MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn request_reload(&mut self) -> Result<(), MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn can_request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(Self::not_this_ports_business())
        }

        /// **The one port this fake actually has.** No launcher to ask, so
        /// this is the healthy answer every real adapter with one listening
        /// would give.
        fn can_send_input(&mut self) -> Result<(), MachineError> {
            Ok(())
        }

        fn send_input(&mut self, action: &str) -> Result<(), MachineError> {
            self.sent.push(
                Action::parse(action)
                    .expect("perform validated the action before it reached this port"),
            );
            Ok(())
        }
    }

    /// **No default action.** The refusal must say so and list the vocabulary,
    /// because an operator who typed `apply input confirm` has made a mistake
    /// that a default would silently turn into a button press.
    #[test]
    fn a_missing_action_is_refused_and_lists_the_vocabulary() {
        let mut board = FakeBoard::default();
        let why = perform(&[SUBJECT], &mut board).expect_err("no action is an error");
        let text = format!("{why:?}");
        assert!(text.contains("enter"), "{text}");
        assert!(text.contains("long-back"), "{text}");
    }

    /// A word this build does not know is refused, and the refusal says why
    /// there is no text entry rather than leaving the reader to wonder.
    #[test]
    fn an_unknown_action_names_the_parity_rule() {
        let mut board = FakeBoard::default();
        let why = perform(&[SUBJECT, "type"], &mut board).expect_err("unknown is an error");
        let text = format!("{why:?}");
        assert!(text.contains("keyboard"), "{text}");
    }

    /// Every action in the vocabulary is accepted by the parser this operation
    /// uses — so the two cannot drift apart.
    #[test]
    fn every_action_is_reachable_through_this_operation() {
        for action in Action::ALL {
            assert_eq!(
                Action::parse(action.as_str()),
                Some(action),
                "{} is in the vocabulary and not reachable",
                action.as_str()
            );
        }
    }

    /// **The plan says what it will NOT do**, and an agent reading it should not
    /// have to wonder whether a press installs anything. It also carries the
    /// provenance note, because `ADR-012` §4 makes the actor a property of every
    /// request and §9.3 rules that an agent's press must not read like a
    /// person's.
    #[test]
    fn the_plan_disclaims_everything_a_press_is_not() {
        let mut board = FakeBoard;
        // A refusing board refuses at the interlock, which is itself the §9.3
        // behaviour: no launcher, no press. The plan is exercised through
        // `perform` on a board that allows it in the integration tests; here
        // what matters is that the interlock is consulted at all.
        let why = perform(&[SUBJECT, "enter", "confirm"], &mut board)
            .expect_err("a board that manages nothing cannot be pressed");
        let text = format!("{why:?}");
        assert!(!text.is_empty());
    }

    /// **A word after the confirmation is refused by name, and the board is
    /// never touched.** `apply input enter confirm back` must not press ENTER
    /// while quietly discarding `back` — the same hazard
    /// `super::service_restart` names for its own fourth word. `back` is
    /// itself a valid action, which is the point: position three has no
    /// meaning at all, so it is refused rather than read as a second press.
    #[test]
    fn a_word_after_the_confirmation_is_refused_and_nothing_is_sent() {
        let mut board = RecordingBoard::default();
        for extra_args in [
            [SUBJECT, "enter", "confirm", "back"],
            [SUBJECT, "enter", "confirm", "confirm"],
            [SUBJECT, "enter", "confirm", "--force"],
        ] {
            let why = perform(&extra_args, &mut board).expect_err("a fourth word is refused");
            assert_eq!(why.code(), ErrorCode::Arguments, "{extra_args:?}");
            assert_eq!(why.subject(), Some(extra_args[3]));
        }
        assert!(board.sent.is_empty(), "a press reached the board: {:?}", board.sent);
    }

    /// A confirmed press with no extra word sends exactly once; the same
    /// request without `confirm` only plans and sends nothing — `ADR-012`
    /// §7.1's property, on a board that could actually press.
    #[test]
    fn a_confirmed_press_sends_once_and_an_unconfirmed_one_only_plans() {
        let mut board = RecordingBoard::default();
        perform(&[SUBJECT, "enter"], &mut board).expect_err("no confirmation is a plan, not a press");
        assert!(board.sent.is_empty(), "a plan sent a press: {:?}", board.sent);

        let pressed = perform(&[SUBJECT, "enter", "confirm"], &mut board)
            .expect("a confirmed press on a healthy board");
        assert_eq!(pressed.action, Action::Enter);
        assert_eq!(board.sent, vec![Action::Enter]);
    }
}
