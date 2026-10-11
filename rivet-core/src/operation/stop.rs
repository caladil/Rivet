//! `rivet apply power-off|restart|trial-restart confirm` — **the three ways to
//! stop a Rivet machine.**
//!
//! `ADR-017` §3.1.10 built the receiving half on 2026-08-23: pid 1 handles
//! `SIGTERM` (power off), `SIGINT` (restart) and `SIGUSR1` (restart once into the
//! armed slot), and its `pid1` module carries the three-row table that is the
//! contract. **This is the other half**, named in as many words when the
//! signal side was built:
//!
//! > **Two things are deliberately not done by that change, and neither is an
//! > oversight.** The `rivet apply` subjects that would *send* these signals —
//! > `power-off`, `restart`, `trial-restart` — are a separate change in
//! > `cmd/rivet`; what exists today is that the signals are **receivable**.
//!
//! Until this module existed, **nothing on a Rivet board could stop it.** There
//! is no shell and no `kill`, the image's six binaries signal nothing, and a board
//! could be stopped only by removing power — which does not carry `ADR-018` §5.1's
//! one-shot flag, so the whole of the trial-boot mechanism could be armed and
//! never exercised.
//!
//! # Why these are subjects of `apply` and not verbs
//!
//! `ADR-006` §6.4's verb set is closed — nine words — and *"is extended only by
//! amending this ADR"*. `reboot` and `shutdown` are not among the nine and adding
//! them is not an agent's to do (`AGENTS.md` §1). `apply` is where the record
//! already puts a confirmed change to a machine: `ADR-012` §7.1 is titled
//! *confirmation before apply*, and `ADR-006` §8.4 chose it for the trial boot on
//! the same argument. [`super::apply`] carries the shape and its costs.
//!
//! # It stops nothing itself, and that is the point
//!
//! [`Machine::request_stop`] delivers a request to the process `ADR-017` §3.1.5
//! item 6 gives shutdown to, and returns. Everything that makes a shutdown a
//! shutdown rather than a syscall — stopping the supervisor, waiting for it,
//! `ADR-021` §3.3's clock write, syncing the card — happens in pid 1 afterwards,
//! on a console this process cannot read.
//!
//! **So a result here reports a request that was accepted, and says so in those
//! words.** `AGENTS.md` §5: *report what the system said, not what it was
//! expected to say.* What the system said is "received".
//!
//! **And the answer may not reach the operator.** A caller arrives over
//! `ADR-035`'s control plane, so the connection carrying this answer is
//! `rivet-mcpd`'s — and pid 1 stops the supervisor first, which stops
//! `rivet-mcpd`. **The transport is deliberately not named in [`next`]**: what
//! is true is that the answer RACES the shutdown whichever channel carried it,
//! either outcome is ordinary, and the console is the authority for what
//! happened after it. Naming a channel there would tell some caller about a
//! connection they had not opened.
//!
//! **Corrected 2026-08-23, by a board.** This said the answer *probably does not*
//! arrive and told the operator to *expect a dropped connection rather than an
//! exit status*. In four stop requests on a Pi 4 — `ADR-018` §5.3.3 — the answer
//! arrived and the caller exited **0** every time: pid 1 stopping the
//! supervisor, waiting for it and syncing is slower than writing one JSON object
//! is. Telling an operator to expect a drop makes the ordinary case look like
//! a fault — the same defect `ADR-017` §3.1.11 fixed on the console, since a
//! message that cries wolf teaches its reader to stop believing it.
//!
//! # `trial-restart` does **not** check that something was armed, and here is why
//!
//! It is the obvious interlock and it was considered, because a one-shot restart
//! into a slot the operator did not choose is a surprise. It is not implemented,
//! for a reason specific to this hardware rather than for cost:
//!
//! **There is no "nothing is armed" state to detect.** `ADR-018` §5.1's arming
//! lives in `autoboot.txt`'s `[tryboot]` section, and `cmd/rivet-image` writes
//! that section on every card it builds — naming the slot that is *not* the
//! default. So on every Rivet card that has ever existed, `[tryboot]` names a
//! slot from the moment it is written; `rivet apply trial-boot` only ever
//! **moves** it. `cmd/rivet`'s
//! `arming_a_slot_the_override_already_names_succeeds_and_writes_nothing` is that
//! fact as a test.
//!
//! An interlock reading "was this armed *by an operator*" would therefore need a
//! second record — a flag written at arm time saying that somebody meant it. That
//! is precisely the second source of truth `ADR-011` forbids, and it would be
//! **wrong as well as duplicated**: the firmware honours `[tryboot]` whatever
//! Rivet believes about who wrote it, so a board would refuse a restart it was
//! going to perform anyway if the flag were lost, and permit one if the flag
//! outlived the file. `AGENTS.md` §5 — *derive, never store* — points the same way.
//!
//! **What is done instead**, and it is weaker on purpose: the plan for
//! `trial-restart` says in as many words that this operation does not check what
//! is armed, names the command that does, and names the failure path. The
//! operator reads the arming from the authority that owns it — `rivet apply
//! trial-boot <slot>`, whose *plan* reports the current record without changing
//! anything.
//!
//! **That paragraph was not true when it was written, and is true as of
//! 2026-08-26.** The sentence it describes lived in [`next`] — a field of the
//! **result**, which an operator reads *after* pid 1 has been asked to restart.
//! A warning that arrives once the request is gone is not a warning. `ADR-006`
//! §6.5's typed plan is where an operator decides, so [`RequestStop::plan`] now
//! carries it as an `untouched` entry and the result carries it still; both come
//! from this module and neither is a second account. `AGENTS.md` §7 in its
//! harder direction — a claim about the code that outlived nothing, because it
//! was never so.
//!
//! **The shape this would take if the maintainer wants it** is recorded rather
//! than left implicit: `rivet_adapter_image::BootSlots` would gain an
//! `armed_slot(&mut dyn BootMedium) -> Result<SlotId, String>` reading the same
//! `[tryboot]` line `arm_trial_boot` writes — one record, read at the point of use
//! — and `Machine` would gain a domain method over it. That is a derivation and
//! not a second authority. It is not built because it puts a FAT mount on the
//! shutdown path, and a machine that cannot be stopped because its card would not
//! mount is a worse failure than the surprise it prevents.

use super::apply::{CONFIRMATION, confirmed};
use super::machine::{Board, MachineError, Stop};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::value::Value;

/// The fields a stop result carries. A subsequence of [`super::apply`]'s
/// published outputs, in the published order, and tested to be one.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "Which stop was requested.",
    },
    Field {
        name: "requested",
        description: "Always true in a result. It means the request was DELIVERED, not that the \
                      machine stopped (ADR-006 §4.2: a request that was not delivered is a \
                      refusal, never a result with a false in it).",
    },
    Field {
        name: "rebooted",
        description: "Always false. Nothing here waits for a machine to come back, and nothing \
                      here could observe it if it did.",
    },
    Field {
        name: "means",
        description: "What the machine will do, in a sentence.",
    },
    Field {
        name: "next",
        description: "What to watch, and why this command's own answer may not arrive.",
    },
];

/// Read a stop from a subject word, exactly and without aliases.
///
/// **Case-sensitive and without synonyms**, for `Verb::parse`'s reason:
/// `ADR-006` §6.4's value is that *"a reader who knows one command can predict
/// the others"*, and a surface where `poweroff`, `halt`, `shutdown` and
/// `power-off` all work is one that must be memorised. The near-misses are what
/// a person types from habit, and they are refused loudly with the list.
pub(super) fn parse_subject(word: &str) -> Option<Stop> {
    Stop::ALL.iter().copied().find(|s| s.as_str() == word)
}

/// A stop request that was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopping {
    how: Stop,
}

impl Stopping {
    /// Which stop was requested.
    pub const fn how(&self) -> Stop {
        self.how
    }

    /// The structured result, whose keys are `OUTPUTS` — checked here rather
    /// than only in a test, because the catalogue naming a field no result
    /// carries is `ADR-006` §7's "trusted and wrong".
    pub fn value(&self) -> Value {
        let fields = vec![
            ("subject", Value::str(self.how.as_str())),
            // Said rather than implied: what happened is that a request was
            // accepted by the thing that stops this machine.
            ("requested", Value::Bool(true)),
            ("rebooted", Value::Bool(false)),
            ("means", Value::str(self.how.means())),
            ("next", Value::str(next(self.how))),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "a stop result's keys must be exactly OUTPUTS, in the declared order"
        );
        Value::object(fields)
    }
}

/// What the operator should do and expect, per stop.
///
/// **The first sentence of all three is the same and it is the surprising one**:
/// the machine has not stopped when this answers, and the answer is racing the
/// shutdown of whatever server is carrying it.
///
/// **It names no transport, since 2026-08-29.** It used to name the one channel
/// there was, and a caller who had arrived by another read a sentence about a
/// connection they had not opened. `rivet-core` may not name a transport at all
/// (`ADR-014` §3, and this module's `operation.rs` sibling says so), so the fix
/// is not to swap one transport for another — it is to describe the channel by
/// its role.
///
/// **It does not promise which way that race goes.** It used to promise a dropped
/// connection, and a board disproved it four times out of four — `ADR-018` §5.3.3,
/// and the module documentation above.
fn next(how: Stop) -> String {
    let common = "the machine has NOT stopped yet — pid 1 stops the supervisor, waits for it, \
                  syncs the filesystems and only then asks the kernel (ADR-017 §3.1.10). This \
                  answer travels back over WHATEVER CHANNEL CARRIED THE REQUEST — ADR-035's \
                  control plane, or a session on the machine itself — and the shutdown is about \
                  to close that channel too, so the answer is racing it: the client may print \
                  this object and exit 0, or may report a dropped connection, and BOTH are \
                  normal. The console is the authority for what happened after this answer.";
    match how {
        Stop::PowerOff => format!(
            "{common} Watch for `rivet-initd: SIGTERM received; shutting down to power off`, \
             then `rivet-initd: syncing filesystems`."
        ),
        Stop::Restart => format!(
            "{common} Watch for `rivet-initd: SIGINT received; shutting down to reboot`. The \
             machine comes back on the DEFAULT slot; if a trial boot was armed, this restart \
             does not take it — use `{apply} {trial}` for that.",
            apply = Verb::Apply,
            trial = Stop::TrialRestart,
        ),
        Stop::TrialRestart => format!(
            "{common} Watch for `rivet-initd: SIGUSR1 received; shutting down to reboot into \
             the trial slot`. THIS OPERATION DOES NOT CHECK WHAT IS ARMED: the `[tryboot]` \
             section of autoboot.txt always names a slot on a Rivet card, so there is no \
             unarmed state to refuse — run `{apply} trial-boot <a|b>` WITHOUT `{CONFIRMATION}` \
             first to read what it names. If the slot that comes up is not committed, the next \
             reset returns to the default with no userspace involvement (ADR-018 §5.1).",
            apply = Verb::Apply,
        ),
    }
}

/// Perform one of the three stop subjects.
///
/// `arguments` is the whole of what followed the verb, so `arguments[0]` is the
/// subject already parsed into `how` and `arguments[1]` is the confirmation.
/// **Nothing may follow it** — see [`confirmed`] for the rule and why.
/// **The change one of the three stop subjects is**, as `ADR-006` §6.5 defines
/// one. All three share it, because all three are one request with a different
/// word in it — the same reason [`OUTPUTS`] is one declaration for three
/// subjects.
struct RequestStop {
    how: Stop,
}

impl Change for RequestStop {
    /// Nothing. Stopping needs only the word the operator typed, which
    /// [`Self::how`] already is.
    type Prepared = ();
    type Applied = Stopping;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, ()), OperationError> {
        // **The interlock, and it now runs on the confirmed path too** — this
        // module used to argue for running it on the plan path and `super::plan`
        // now owns that argument for every subject.
        board
            .can_request_stop(self.how)
            .map_err(|e| refused(e, self.how))?;

        let mut plan = Plan::new(self.how.as_str(), "stops the machine")
            .doing(
                Effect::request("pid 1", self.how.means()).noting(
                    "ADR-017 §3.1.5 item 6 gives shutdown to pid 1; this operation delivers the \
                     request and returns, and everything that makes a shutdown a shutdown \
                     happens afterwards on a console this process cannot read",
                ),
            )
            .leaving(
                "NOTHING HAS BEEN SENT — the request is delivered only if this is confirmed",
            )
            .taking_effect(TakesEffect::Immediately)
            .confirmed_by(&[self.how.as_str()]);

        // **`trial-restart`'s caveat belongs in the plan, and until 2026-08-26
        // it was only in the result.** This module's documentation says the
        // plan *"says in as many words that this operation does not check what
        // is armed"* — and it did not: the sentence lived in [`next`], which is
        // a field of a result the operator reads **after** the machine has been
        // asked to restart. A warning that arrives once the request is gone is
        // not a warning, and `ADR-006` §6.5's typed plan is precisely the place
        // an operator decides. It stays in the result too; both are generated
        // from this one match.
        if self.how == Stop::TrialRestart {
            plan = plan.leaving(
                "WHAT IS ARMED IS NOT CHECKED — the `[tryboot]` section of autoboot.txt always \
                 names a slot on a Rivet card, so there is no unarmed state to refuse. Run \
                 `rivet apply trial-boot <a|b>` WITHOUT the confirmation first to read what it \
                 names",
            );
        }

        Ok((plan, ()))
    }

    fn apply(self, (): (), board: &mut dyn Board) -> Result<Stopping, OperationError> {
        board
            .request_stop(self.how)
            .map_err(|e| refused(e, self.how))?;
        Ok(Stopping { how: self.how })
    }
}

pub(super) fn perform(
    how: Stop,
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Stopping, OperationError> {
    // A stop subject takes no slot, so the third word is not "a confirmation in
    // the wrong place" — it is a word this command has no meaning for at all.
    // Refused rather than ignored, because the alternative is `rivet apply
    // power-off b confirm` quietly powering the machine off.
    if let Some(extra) = arguments.get(2).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {how}` takes only the confirmation after it. `{extra}` is a third \
                 word, and the only subject that takes one is `trial-boot`, which needs a slot.",
                apply = Verb::Apply,
            ),
        ));
    }

    let confirmed = confirmed(arguments.get(1).copied())?;

    // Everything above this line is argument parsing; the plan/apply split below
    // is `super::plan`'s driver rather than this function's — `ADR-006` §6.5.
    resolve(RequestStop { how }, confirmed, board)
}

/// The one place a [`MachineError`] becomes this operation's refusal.
///
/// The same mapping `super::trial` uses and for the same reason — an operator
/// does two different things next — and **nothing is reworded on the way
/// through**. The interlock's own words are what say which machine this is and
/// what pid 1 turned out to be.
fn refused(error: MachineError, how: Stop) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, how.as_str(), error.why())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    // The operation itself talks to a `Board` now; the fake still has to
    // implement the narrow port the board is made of.
    use super::super::machine::Machine;
    use crate::state::Slot;

    /// A machine that answers as scripted and records what it was asked to do.
    ///
    /// **It never signals anything**, which is the property that makes the
    /// accept path testable at all: on a development host the thing this
    /// operation asks to stop is pid 1, and pid 1 on a development host is the
    /// host. `cmd/rivet`'s `shutdown` module carries the interlock that
    /// enforces that, and its own tests read the real `/proc`.
    struct FakeMachine {
        allowed: Result<(), MachineError>,
        delivered: Result<(), MachineError>,
        planned: Vec<Stop>,
        requested: Vec<Stop>,
    }

    impl FakeMachine {
        fn healthy() -> Self {
            Self {
                allowed: Ok(()),
                delivered: Ok(()),
                planned: Vec::new(),
                requested: Vec::new(),
            }
        }

        /// A machine the interlock refuses — a development host, in practice.
        fn not_a_rivet_machine() -> Self {
            let why = MachineError::Unsafe(
                "pid 1 on this machine is `systemd`, not `rivet-initd`".to_string(),
            );
            Self {
                allowed: Err(why.clone()),
                delivered: Err(why),
                planned: Vec::new(),
                requested: Vec::new(),
            }
        }
    }

    // **This operation stops a machine and does nothing else**, and since
    // 2026-08-26 it reaches `super::plan::resolve`, which takes the whole
    // `Board`. So the fake answers on the other three ports and refuses on all
    // of them — a stop test that started editing, installing or observing fails
    // loudly rather than passing on a fiction (`AGENTS.md` §3).
    crate::operation::machine::refuses_to_edit!(FakeMachine);
    crate::operation::machine::refuses_to_deploy!(FakeMachine);
    crate::operation::machine::refuses_to_observe!(FakeMachine);

    /// **The reload port refuses in every fake but `super::reload`'s.**
    /// `AGENTS.md` §3: a stub refuses and never fakes success, so a test in
    /// this module that started reloading fails loudly.
    fn no_reload() -> MachineError {
        MachineError::Refused(
            "this fake does not reload; an operation that reached this port has changed"
                .to_string(),
        )
    }

    impl Machine for FakeMachine {
        // **The two `ADR-032` §3.2 added on 2026-08-26.** This fake does not
        // reload anything, and refusing is the honest stand-in (`AGENTS.md` §3):
        // a test here that started reloading fails loudly rather than passing on
        // a fiction.
        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            Err(no_reload())
        }
        fn request_reload(&mut self) -> Result<(), MachineError> {
            Err(no_reload())
        }

        fn can_request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(no_reload())
        }

        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(no_reload())
        }

        // The `read_journal` and `read_filesystems` stubs that stood here were
        // removed on 2026-08-24: both moved to `Observed`, so this fake is
        // no longer obliged to have an opinion about them.
        fn running_slot(&mut self) -> Result<Slot, MachineError> {
            Err(MachineError::Refused(
                "these tests do not read a card".to_string(),
            ))
        }

        fn can_arm_trial_boot(&mut self, _slot: Slot) -> Result<(), MachineError> {
            Err(MachineError::Refused(
                "these tests do not read a card".to_string(),
            ))
        }

        fn arm_trial_boot(
            &mut self,
            _slot: Slot,
        ) -> Result<super::super::machine::Reboot, MachineError> {
            Err(MachineError::Refused(
                "these tests do not read a card".to_string(),
            ))
        }

        fn can_request_stop(&mut self, stop: Stop) -> Result<(), MachineError> {
            self.planned.push(stop);
            self.allowed.clone()
        }

        fn request_stop(&mut self, stop: Stop) -> Result<(), MachineError> {
            self.requested.push(stop);
            self.delivered.clone()
        }
    }

    fn fields(stopping: &Stopping) -> Vec<(&'static str, Value)> {
        match stopping.value() {
            Value::Object(fields) => fields,
            other => panic!("a result must be an object, got {other:?}"),
        }
    }

    fn field(stopping: &Stopping, key: &str) -> Value {
        match fields(stopping).into_iter().find(|(k, _)| *k == key) {
            Some((_, value)) => value,
            None => panic!("no {key} field"),
        }
    }

    // -- confirmation ----------------------------------------------------------

    /// **`ADR-012` §7.1, as the property that matters**: without the word,
    /// nothing is sent to anything.
    #[test]
    fn without_the_confirmation_word_no_stop_is_ever_requested() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            let refusal = perform(*how, &[how.as_str()], &mut m)
                .expect_err("it stopped the machine without confirmation");
            assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired, "{how}");
            assert!(
                m.requested.is_empty(),
                "{how} was requested: {:?}",
                m.requested
            );
            assert_eq!(
                m.planned,
                vec![*how],
                "{how}'s plan did not run the interlock"
            );
        }
    }

    /// And the refusal has to be a *plan*: what will happen, that nothing has
    /// been sent, and the exact next command.
    #[test]
    fn the_unconfirmed_refusal_says_what_happens_and_the_command_that_does_it() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            let refusal = perform(*how, &[how.as_str()], &mut m).expect_err("unconfirmed");
            let message = refusal.message();
            assert!(message.contains(how.means()), "{how}: {message}");
            assert!(message.contains("ADR-012 §7.1"), "{how}: {message}");
            assert!(
                message.contains("NOTHING HAS BEEN SENT"),
                "{how}: {message}"
            );
            assert!(
                message.contains(&format!("rivet apply {how} {CONFIRMATION}")),
                "{how} does not say what to type: {message}"
            );
            assert_eq!(refusal.subject(), Some(how.as_str()));
        }
    }

    /// **And the plan is fields rather than that sentence.** `ADR-006` §6.5.
    /// All three stops are one request, so all three carry one effect — and
    /// `trial-restart` carries the caveat the result used to carry alone.
    #[test]
    fn each_stop_carries_a_typed_plan_and_trial_restart_warns_before_it_is_too_late() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            let refusal = perform(*how, &[how.as_str()], &mut m).expect_err("unconfirmed");
            let plan = refusal.plan().expect("a confirmation refusal carries its plan");

            assert_eq!(plan.subject(), how.as_str());
            assert_eq!(
                plan.confirm_with(),
                format!("rivet apply {how} {CONFIRMATION}")
            );
            // Stopping IS the restart, so nothing is pending on one.
            assert_eq!(plan.takes_effect().requires_reboot(), Some(false), "{how}");

            let [effect] = plan.effects() else {
                panic!("{how} asks pid 1 for exactly one thing: {:?}", plan.effects());
            };
            assert_eq!(effect.act, crate::operation::Act::Request);
            assert_eq!(effect.target, "pid 1");
            assert_eq!(effect.to.as_deref(), Some(how.means()));

            let armed_caveat = plan
                .untouched()
                .iter()
                .any(|line| line.contains("WHAT IS ARMED IS NOT CHECKED"));
            assert_eq!(
                armed_caveat,
                *how == Stop::TrialRestart,
                "{how} carries the wrong armed-slot caveat: {:?}",
                plan.untouched()
            );
        }
    }

    /// With the word, the request is delivered — and it is the stop that was
    /// asked for, never one worked out.
    #[test]
    fn with_the_confirmation_word_the_named_stop_is_requested_and_only_that_one() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            let stopping =
                perform(*how, &[how.as_str(), CONFIRMATION], &mut m).expect("a healthy machine");
            assert_eq!(m.requested, vec![*how]);
            assert_eq!(stopping.how(), *how);
            // ~~The confirmed path does not re-run the plan: one interlock, at
            // the moment of the act.~~ **Inverted 2026-08-26**, and it is a
            // deliberate change rather than a test bending to the code:
            // `super::plan::resolve` plans on both paths, so the interlock now
            // runs BEFORE the act it guards rather than instead of a plan. The
            // old shape checked `can_request_stop` on the plan path and
            // `request_stop` on the confirmed one, which meant a confirmed stop
            // was the one path the interlock never covered.
            assert_eq!(
                m.planned,
                vec![*how],
                "{how} skipped the interlock on the confirmed path"
            );
        }
    }

    /// A misspelt confirmation is refused rather than treated as absent — the
    /// rule `super::apply::confirmed` owns, asserted here through this
    /// operation so that a subject that stopped using it would fail.
    #[test]
    fn a_word_that_is_not_the_confirmation_is_refused_rather_than_ignored() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            for word in ["confrim", "yes", "--confirm", "CONFIRM", ""] {
                let refusal = perform(*how, &[how.as_str(), word], &mut m)
                    .expect_err("only the exact confirmation word may confirm");
                assert_eq!(refusal.code(), ErrorCode::Arguments, "{how} {word:?}");
                assert_eq!(refusal.subject(), Some(word));
            }
            assert!(m.requested.is_empty());
        }
    }

    /// **A stop subject takes no slot.** `apply power-off b confirm` must not
    /// power the machine off, and it must not silently drop the `b` either.
    #[test]
    fn a_third_word_is_refused_because_no_stop_subject_takes_one() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            for extra in ["b", "confirm", "a", "--force"] {
                let refusal = perform(*how, &[how.as_str(), CONFIRMATION, extra], &mut m)
                    .expect_err("a third word must be refused");
                assert_eq!(refusal.code(), ErrorCode::Arguments, "{how} {extra:?}");
                assert_eq!(refusal.subject(), Some(extra));
                assert!(
                    refusal.message().contains("trial-boot"),
                    "{}",
                    refusal.message()
                );
            }
            assert!(
                m.requested.is_empty(),
                "{how} was requested with a third word"
            );
            assert!(m.planned.is_empty());
        }
    }

    // -- the interlock ---------------------------------------------------------

    /// **The refusal that matters most, through the operation.** A machine whose
    /// pid 1 is not Rivet's refuses on BOTH paths — the plan and the confirmed
    /// act — and nothing is ever delivered.
    #[test]
    fn a_machine_the_interlock_refuses_is_never_asked_to_stop_on_either_path() {
        for how in Stop::ALL {
            let mut m = FakeMachine::not_a_rivet_machine();
            for args in [
                vec![how.as_str()].as_slice(),
                vec![how.as_str(), CONFIRMATION].as_slice(),
            ] {
                let refusal = perform(*how, args, &mut m).expect_err("the interlock refuses");
                assert_eq!(refusal.code(), ErrorCode::Unsafe, "{how} {args:?}");
                assert!(
                    refusal.message().contains("rivet-initd"),
                    "the interlock's own words must survive: {}",
                    refusal.message()
                );
            }
        }
    }

    /// The two kinds of machine refusal stay two codes, because an operator does
    /// two different things next: stop, versus look at the hardware.
    #[test]
    fn an_interlock_and_a_failed_delivery_are_two_different_codes() {
        let mut refused_by_kernel = FakeMachine::healthy();
        refused_by_kernel.delivered = Err(MachineError::Refused("EPERM".to_string()));
        let refusal = perform(
            Stop::PowerOff,
            &[Stop::PowerOff.as_str(), CONFIRMATION],
            &mut refused_by_kernel,
        )
        .expect_err("the kernel refused");
        assert_eq!(refusal.code(), ErrorCode::MachineRefused);
        assert_eq!(refusal.message(), "EPERM");

        let mut interlocked = FakeMachine::not_a_rivet_machine();
        let refusal = perform(
            Stop::PowerOff,
            &[Stop::PowerOff.as_str(), CONFIRMATION],
            &mut interlocked,
        )
        .expect_err("the interlock refused");
        assert_eq!(refusal.code(), ErrorCode::Unsafe);
    }

    // -- the subject vocabulary ------------------------------------------------

    /// Both directions round-trip, so a result can be fed back in as a request.
    #[test]
    fn every_stop_has_a_word_and_every_word_is_a_stop() {
        for how in Stop::ALL {
            assert_eq!(parse_subject(how.as_str()), Some(*how));
        }
    }

    /// **Near-misses matter more than nonsense**, and here they matter more than
    /// usual: `halt`, `poweroff` and `shutdown` are what a person types from
    /// habit on any other Linux, and each of them must be refused rather than
    /// aliased onto a command that stops the machine.
    #[test]
    fn the_words_every_other_linux_uses_are_not_accepted_here() {
        for word in [
            "halt",
            "poweroff",
            "shutdown",
            "reboot",
            "power_off",
            "PowerOff",
            "POWER-OFF",
            "trial_restart",
            "tryboot",
            "",
            "restart-trial",
        ] {
            assert_eq!(parse_subject(word), None, "{word:?} was accepted as a stop");
        }
    }

    // -- the result ------------------------------------------------------------

    /// The result carries exactly this module's declared fields, in the declared
    /// order — `ADR-006` §7's "trusted and wrong" in the smallest available form.
    #[test]
    fn the_result_carries_exactly_the_fields_this_module_declares_in_order() {
        let mut m = FakeMachine::healthy();
        let stopping = perform(
            Stop::PowerOff,
            &[Stop::PowerOff.as_str(), CONFIRMATION],
            &mut m,
        )
        .expect("a healthy machine");
        let keys: Vec<&str> = fields(&stopping).iter().map(|(k, _)| *k).collect();
        let declared: Vec<&str> = OUTPUTS.iter().map(|o| o.name).collect();
        assert_eq!(keys, declared);
        // ...and the catalogue publishes every one of them.
        let entry = super::super::operation(Verb::Apply).expect("apply is catalogued");
        let entry_outputs = entry.outputs();
        let published: Vec<&str> = entry_outputs.iter().map(|o| o.name).collect();
        for key in keys {
            assert!(
                published.contains(&key),
                "{key} is not published: {published:?}"
            );
        }
    }

    /// **A result never says the machine stopped, and never says it rebooted.**
    /// Both would be claims nothing in this process can observe.
    #[test]
    fn a_result_reports_a_request_that_was_delivered_and_nothing_more() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            let stopping =
                perform(*how, &[how.as_str(), CONFIRMATION], &mut m).expect("a healthy machine");
            assert_eq!(field(&stopping, "requested"), Value::Bool(true), "{how}");
            assert_eq!(field(&stopping, "rebooted"), Value::Bool(false), "{how}");
            assert_eq!(field(&stopping, "subject"), Value::str(how.as_str()));
            assert_eq!(field(&stopping, "means"), Value::str(how.means()));
        }
    }

    /// **The surprising fact reaches the operator**: this answer probably does
    /// not arrive, because the shutdown closes the session carrying it.
    #[test]
    fn every_result_warns_that_its_own_answer_may_not_arrive() {
        for how in Stop::ALL {
            let mut m = FakeMachine::healthy();
            let stopping =
                perform(*how, &[how.as_str(), CONFIRMATION], &mut m).expect("a healthy machine");
            let next = match field(&stopping, "next") {
                Value::Str(s) => s,
                other => panic!("next is {other:?}"),
            };
            assert!(next.contains("has NOT stopped yet"), "{how}: {next}");
            assert!(next.contains("console"), "{how}: {next}");
            // Each names the console line to look for, and the three differ.
            assert!(next.contains("rivet-initd:"), "{how}: {next}");
        }
    }

    /// **Every stop describes the channel by its ROLE and never by its name.**
    ///
    /// All three used to say the answer *"travels over the session the shutdown
    /// is about to close"*, naming the one channel there was at the time — and a
    /// caller who had arrived another way read a sentence about a connection
    /// they had never opened, which teaches a reader to discount the rest of it,
    /// exactly the failure `ADR-017` §3.1.11 fixed on the console.
    ///
    /// **Stated positively, and that is forced rather than chosen.** The obvious
    /// guard is a list of transport names that must not appear, and a list like
    /// that is a tracked copy of the thing it exists to keep out. So this
    /// asserts the phrasing that IS correct — the role — which fails for any
    /// sentence that swapped it for a name, whatever the name turned out to be.
    /// `AGENTS.md` §7 is about the sibling copies a fix leaves behind, and this
    /// covers all three plus any new one.
    #[test]
    fn every_stop_names_the_channel_by_its_role_rather_than_by_a_transport() {
        for how in Stop::ALL {
            let guidance = next(*how);
            let lowered = guidance.to_lowercase();
            assert!(
                lowered.contains("whatever channel carried the request"),
                "{how} describes the channel as something other than the one that carried \
                 the request, which rivet-core may not know about (ADR-014 §3): {guidance}"
            );
            // …and still says what it is racing, so the fix is not a deletion.
            assert!(
                lowered.contains("racing"),
                "{how} lost the race it was warning about: {guidance}"
            );
        }
    }

    /// The three `next` sentences must not be interchangeable: each names the
    /// signal pid 1 will print, and reading the wrong one on a console is how an
    /// operator concludes a trial boot happened when a plain restart did.
    #[test]
    fn the_three_stops_name_three_different_console_lines() {
        let mut lines: Vec<String> = Vec::new();
        for how in Stop::ALL {
            let line = next(*how);
            assert!(
                !lines.contains(&line),
                "{how} repeats another stop's guidance"
            );
            lines.push(line);
        }
        assert!(next(Stop::PowerOff).contains("SIGTERM"));
        assert!(next(Stop::Restart).contains("SIGINT"));
        assert!(next(Stop::TrialRestart).contains("SIGUSR1"));
    }

    /// **The decision recorded in this module's documentation, as a test.**
    /// `trial-restart` does not check what is armed, so it has to *say* so where
    /// the operator will read it, and point at the command that does.
    #[test]
    fn the_trial_restart_says_it_does_not_check_what_is_armed_and_names_what_does() {
        let guidance = next(Stop::TrialRestart);
        assert!(
            guidance.contains("DOES NOT CHECK WHAT IS ARMED"),
            "{guidance}"
        );
        assert!(guidance.contains("apply trial-boot"), "{guidance}");
        assert!(guidance.contains("ADR-018 §5.1"), "{guidance}");
        // And the other two must not claim it, because they are not the one-shot.
        for how in [Stop::PowerOff, Stop::Restart] {
            assert!(
                !next(how).contains("tryboot"),
                "{how} mentions the one-shot: {}",
                next(how)
            );
        }
    }
}
