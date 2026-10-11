//! `rivet apply clock-floor-reset confirm` — **put this machine's clock floor
//! back to its running clock.** `ADR-021` §3.
//!
//! # What the floor is, and why an operator needs to move it down
//!
//! A Raspberry Pi has no real-time clock, so the supervisor writes the running
//! clock to the card every hour and restores it at the next boot as a
//! **floor**: the machine never believes it is earlier than when it last ran.
//! The floor only ever rises. That is the property, and it is also the hazard:
//! a clock stepped once to a wildly wrong future time — the year 2200, from one
//! time server that lied — is written to the card at the next hour, and every
//! boot after that restores 2200 again. Nothing on the machine can tell a
//! future it really reached from one it was told about, so nothing on the
//! machine may lower it on its own.
//!
//! **This operation is the operator lowering it**, deliberately: the floor
//! becomes the running clock, both in the supervisor's memory and on the card,
//! and from then on rises from there.
//!
//! # Its plan says both numbers
//!
//! Without the confirmation nothing is written, and the plan carries the floor
//! the supervisor holds and the running clock it would be replaced with. A
//! floor that is already at or below the running clock is said to be so — the
//! reset then changes nothing a boot would notice, which is worth knowing
//! before confirming.
//!
//! # The wire format, both halves, here
//!
//! `super::service_restart`'s rule: tab-separated lines, writer and reader in
//! one module. The request is
//! [`crate::operation::service_restart::Request::ClockFloorReset`].

use super::machine::Board;
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, MachineError, OperationError, Verb};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "clock-floor-reset";

/// Where the floor is kept between boots.
pub const STORED_AT: &str = "/state/clock";

/// **What the supervisor holds, and what the reset makes it.** The same type on
/// the plan path and the apply path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Floor {
    /// The floor before the reset, in Unix milliseconds — `None` on a boot that
    /// established none (no stored clock, or one that could not be read).
    pub was_ms: Option<u64>,
    /// The running clock, which the reset makes the floor.
    pub now_ms: u64,
}

const FLOOR: &str = "floor";
const REFUSED: &str = "refused";

impl Floor {
    /// The wire form: one line, tab-separated, `-` for no floor.
    ///
    /// ```text
    /// floor\t7258118400000\t1790000000000
    /// ```
    pub fn render(&self) -> String {
        format!(
            "{FLOOR}\t{}\t{}\n",
            self.was_ms.map_or_else(|| "-".to_string(), |ms| ms.to_string()),
            self.now_ms
        )
    }

    /// Whether the reset moves the floor DOWN, which is what it exists for.
    #[must_use]
    pub fn lowers(&self) -> bool {
        self.was_ms.is_some_and(|was| was > self.now_ms)
    }

    /// The structured result, whose keys are `OUTPUTS`.
    pub fn value(&self, applied: bool) -> Value {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            (
                "was",
                Value::str(
                    self.was_ms
                        .map_or_else(|| "none".to_string(), crate::journal::rfc3339),
                ),
            ),
            ("now", Value::str(crate::journal::rfc3339(self.now_ms))),
            ("lowers", Value::Bool(self.lowers())),
            ("applied", Value::Bool(applied)),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "a clock-floor-reset result's keys must be exactly OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// The fields a result carries.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `clock-floor-reset` here.",
    },
    Field {
        name: "was",
        description: "The floor the supervisor held before, as RFC 3339, or `none` on a boot that \
                      established none.",
    },
    Field {
        name: "now",
        description: "The running clock, as RFC 3339, which is the floor afterwards.",
    },
    Field {
        name: "lowers",
        description: "Whether the reset moves the floor down — the case it exists for. False is \
                      not a failure: the floor already was at or below the running clock.",
    },
    Field {
        name: "applied",
        description: "Whether this was the confirmed apply. True means the supervisor holds the \
                      new floor and wrote it to the card.",
    },
];

// ---------------------------------------------------------------------------
// The refusals
// ---------------------------------------------------------------------------

/// **Why the supervisor would not reset the floor.** Facts only;
/// [`Self::describe`] composes the sentence here, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// There is no readable wall clock, so there is nothing to reset it to.
    NoWallClock,
    /// The new floor could not be written to the card. The floor the supervisor
    /// held is unchanged, in memory and on the card.
    NotWritten { detail: String },
}

impl Refusal {
    /// The wire word. Never renamed.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NoWallClock => "no-wall-clock",
            Self::NotWritten { .. } => "not-written",
        }
    }

    /// **The sentence.** Every one says `REFUSED` first and `NOTHING WAS
    /// CHANGED`.
    pub fn describe(&self) -> String {
        match self {
            Self::NoWallClock => format!(
                "REFUSED to reset the clock floor: this machine has no readable wall clock, so \
                 there is no time to reset it to (ADR-021 §3). NOTHING WAS CHANGED. `rivet {get} \
                 log latest all` carries what the supervisor said about the clock at boot.",
                get = Verb::Get,
            ),
            Self::NotWritten { detail } => format!(
                "REFUSED to reset the clock floor: the new floor could not be written to \
                 {STORED_AT} ({detail}), so NOTHING WAS CHANGED — the supervisor keeps the floor it \
                 had, in memory and on the card (ADR-021 §3)."
            ),
        }
    }

    /// `Refused` either way: the machine was asked and said why.
    pub fn as_machine_error(&self) -> MachineError {
        MachineError::Refused(self.describe())
    }

    /// The wire form: `refused`, the subject, the code, and the detail.
    pub fn render(&self) -> String {
        format!(
            "{REFUSED}\t{SUBJECT}\t{}\t{}\n",
            self.code(),
            match self {
                Self::NoWallClock => String::new(),
                // A tab or a newline would end the field early; an errno's
                // sentence carries neither, and this keeps that true.
                Self::NotWritten { detail } => detail.replace(['\t', '\n'], " "),
            }
        )
    }
}

/// **What the supervisor answered**, read back. [`Floor::render`] and
/// [`Refusal::render`] are the writers.
///
/// # Errors
///
/// A line this build cannot read is refused rather than guessed at.
pub fn parse_answer(text: &str) -> Result<Result<Floor, Refusal>, AnswerError> {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or(AnswerError::Empty)?;
    let fields: Vec<&str> = line.split('\t').collect();
    let number = |field: &'static str, word: &str| {
        word.parse::<u64>().map_err(|_| AnswerError::Value {
            field,
            found: word.to_string(),
        })
    };
    match fields.first().copied().unwrap_or_default() {
        FLOOR => {
            let [_, was, now] = fields.as_slice() else {
                return Err(AnswerError::Fields {
                    kind: FLOOR,
                    found: fields.len(),
                    wanted: 3,
                });
            };
            Ok(Ok(Floor {
                was_ms: if *was == "-" {
                    None
                } else {
                    Some(number("was", was)?)
                },
                now_ms: number("now", now)?,
            }))
        }
        REFUSED => {
            let [_, subject, code, detail] = fields.as_slice() else {
                return Err(AnswerError::Fields {
                    kind: REFUSED,
                    found: fields.len(),
                    wanted: 4,
                });
            };
            if *subject != SUBJECT {
                return Err(AnswerError::Value {
                    field: "subject",
                    found: (*subject).to_string(),
                });
            }
            Ok(Err(match *code {
                "no-wall-clock" => Refusal::NoWallClock,
                "not-written" => Refusal::NotWritten {
                    detail: (*detail).to_string(),
                },
                other => {
                    return Err(AnswerError::Value {
                        field: "code",
                        found: other.to_string(),
                    });
                }
            }))
        }
        other => Err(AnswerError::UnknownLine {
            kind: other.to_string(),
        }),
    }
}

/// An answer that could not be read — `super::service_restart`'s four shapes.
pub use super::service_restart::AnswerError;

// ---------------------------------------------------------------------------
// The operation
// ---------------------------------------------------------------------------

/// **The change this subject is.** The plan is the supervisor's own answer.
struct ResetFloor;

impl Change for ResetFloor {
    type Prepared = Floor;
    type Applied = Floor;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Floor), OperationError> {
        let preview = board.can_request_clock_floor_reset().map_err(refused)?;
        let was = preview
            .was_ms
            .map_or_else(|| "no floor".to_string(), crate::journal::rfc3339);
        let now = crate::journal::rfc3339(preview.now_ms);
        let summary = if preview.lowers() {
            format!("lowers this machine's clock floor from {was} to the running clock, {now}")
        } else {
            format!(
                "sets this machine's clock floor to the running clock, {now}; it is {was} now, so \
                 this lowers nothing"
            )
        };
        let plan = Plan::new(SUBJECT, summary)
            .doing(
                Effect::set(STORED_AT, was, now.clone()).noting(
                    "the floor is what the next boot restores the clock to at the least; from \
                     here it rises again, an hour at a time, as it always does (ADR-021 §3)",
                ),
            )
            .leaving(
                "the running clock — this sets the floor TO it and does not move it. If the \
                 running clock is itself wrong, correct it first: the floor only follows it",
            )
            .leaving("NOTHING HAS BEEN WRITTEN — the floor is reset only if this is confirmed")
            .taking_effect(TakesEffect::Immediately)
            .confirmed_by(&[SUBJECT]);
        Ok((plan, preview))
    }

    fn apply(self, _: Floor, board: &mut dyn Board) -> Result<Floor, OperationError> {
        board.request_clock_floor_reset().map_err(refused)
    }
}

/// Perform it. `arguments[0]` is the subject.
pub(super) fn perform(arguments: &[&str], board: &mut dyn Board) -> Result<Floor, OperationError> {
    if let Some(extra) = arguments.get(2).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {SUBJECT}` takes nothing but the confirmation: the floor is reset to the \
                 running clock and to nothing else. `{extra}` is a word too many.",
                apply = Verb::Apply,
            ),
        ));
    }
    let confirmed = super::apply::confirmed(arguments.get(1).copied())?;
    resolve(ResetFloor, confirmed, board)
}

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

    const FUTURE: u64 = 7_258_118_400_000; // 2200-01-01T00:00:00Z
    const NOW: u64 = 1_790_000_000_000;

    /// A board that answers the floor port as scripted and refuses the rest.
    struct FakeBoard {
        answer: Result<Floor, Refusal>,
        planned: usize,
        applied: usize,
    }

    fn nope() -> MachineError {
        MachineError::Refused("this fake answers only the clock floor".to_string())
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
        fn can_request_clock_floor_reset(&mut self) -> Result<Floor, MachineError> {
            self.planned += 1;
            self.answer.clone().map_err(|r| r.as_machine_error())
        }
        fn request_clock_floor_reset(&mut self) -> Result<Floor, MachineError> {
            self.applied += 1;
            self.answer.clone().map_err(|r| r.as_machine_error())
        }
    }

    fn board(answer: Result<Floor, Refusal>) -> FakeBoard {
        FakeBoard {
            answer,
            planned: 0,
            applied: 0,
        }
    }

    /// **Without the confirmation nothing is asked to change**, and the plan
    /// carries both numbers — the house shape of every `apply`.
    #[test]
    fn without_the_confirmation_the_plan_says_both_times_and_nothing_is_reset() {
        let mut b = board(Ok(Floor {
            was_ms: Some(FUTURE),
            now_ms: NOW,
        }));
        let refusal = perform(&[SUBJECT], &mut b).expect_err("a plan");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert_eq!((b.planned, b.applied), (1, 0));
        let plan = refusal.plan().expect("a confirmation refusal carries its plan");
        let [effect] = plan.effects() else {
            panic!("one floor, one effect: {:?}", plan.effects());
        };
        assert_eq!(effect.act, Act::Set);
        assert_eq!(effect.target, STORED_AT);
        assert!(plan.consequence().contains("2200-01-01"), "{}", plan.consequence());
        assert!(plan.consequence().contains("lowers"), "{}", plan.consequence());
        assert_eq!(plan.confirm_with(), format!("rivet apply {SUBJECT} {CONFIRMATION}"));
    }

    #[test]
    fn with_the_confirmation_the_floor_is_reset_after_the_plan() {
        let mut b = board(Ok(Floor {
            was_ms: Some(FUTURE),
            now_ms: NOW,
        }));
        let done = perform(&[SUBJECT, CONFIRMATION], &mut b).expect("a confirmed reset");
        assert_eq!((b.planned, b.applied), (1, 1));
        assert!(done.lowers());
        let Value::Object(fields) = done.value(true) else {
            panic!("a result is an object");
        };
        let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
        let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
        assert_eq!(keys, declared);
    }

    #[test]
    fn a_refusal_stops_the_plan_and_nothing_is_asked_to_change() {
        let mut b = board(Err(Refusal::NoWallClock));
        let refusal = perform(&[SUBJECT, CONFIRMATION], &mut b).expect_err("refused");
        assert_eq!(refusal.code(), ErrorCode::MachineRefused);
        assert_eq!(b.applied, 0);
    }

    #[test]
    fn a_word_after_the_confirmation_is_refused_before_anything_is_asked() {
        let mut b = board(Err(Refusal::NoWallClock));
        let refusal = perform(&[SUBJECT, CONFIRMATION, "now"], &mut b).expect_err("refused");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert_eq!((b.planned, b.applied), (0, 0));
    }

    #[test]
    fn the_request_round_trips_both_ways() {
        use crate::operation::service_restart::Request;
        for apply in [false, true] {
            let request = Request::ClockFloorReset { apply };
            assert_eq!(Request::parse(&request.render()), Ok(request));
        }
    }

    #[test]
    fn what_the_supervisor_writes_is_what_a_reader_reads() {
        for floor in [
            Floor {
                was_ms: Some(FUTURE),
                now_ms: NOW,
            },
            Floor {
                was_ms: None,
                now_ms: NOW,
            },
        ] {
            assert_eq!(parse_answer(&floor.render()), Ok(Ok(floor)));
        }
        for refusal in [
            Refusal::NoWallClock,
            Refusal::NotWritten {
                detail: "No space left on device (os error 28)".to_string(),
            },
        ] {
            assert_eq!(parse_answer(&refusal.render()), Ok(Err(refusal)));
        }
    }

    #[test]
    fn a_floor_in_the_future_is_lowered_and_one_behind_is_not() {
        let future = Floor {
            was_ms: Some(FUTURE),
            now_ms: NOW,
        };
        assert!(future.lowers());
        let behind = Floor {
            was_ms: Some(NOW - 1),
            now_ms: NOW,
        };
        assert!(!behind.lowers());
        assert!(
            !Floor {
                was_ms: None,
                now_ms: NOW
            }
            .lowers()
        );
    }

    #[test]
    fn every_refusal_says_refused_and_that_nothing_changed() {
        for refusal in [
            Refusal::NoWallClock,
            Refusal::NotWritten {
                detail: "EIO".to_string(),
            },
        ] {
            let said = refusal.describe();
            assert!(said.starts_with("REFUSED"), "{said}");
            assert!(said.contains("NOTHING WAS CHANGED"), "{said}");
        }
    }

    #[test]
    fn an_answer_this_build_does_not_know_is_refused() {
        assert!(parse_answer("").is_err());
        assert!(parse_answer("floor\tsoon\t1\n").is_err());
        assert!(parse_answer("refused\tservice-stop\tno-wall-clock\t\n").is_err());
        assert!(parse_answer("something\n").is_err());
    }
}
