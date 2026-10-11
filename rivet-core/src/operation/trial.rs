//! `rivet apply trial-boot <slot> [confirm]` — **the first operation that
//! changes a machine.**
//!
//! `ADR-018` §5 is the mechanism and the firmware owns it: a one-shot flag boots
//! the other slot exactly once and returns to the previous slot on the next reset
//! if nothing commits, *"with no userspace involvement on the failure path at
//! all"*. `rivet_adapter_image::BootSlots::arm_trial_boot` is the primitive.
//! **It had no caller at all until this operation**, which is why `ADR-018` §5.3
//! item 2 — that a `[tryboot]` section selects a different `boot_partition` — has
//! never been observed on any board: nothing could ask for it.
//!
//! # Why the CLI, and why that is not the CLI deciding anything
//!
//! `ADR-028` §4, ruled 2026-08-23 (`.rulings/` R-274): an authorised user,
//! local or remote, supplies the authority to order an update, and the signed
//! manifest is what verifies the artifact's authenticity.
//! **Authority** comes from an authenticated principal acting through the
//! administration surface — and `ADR-035` makes the control plane exactly
//! that surface. So the operator whose `rivet` invocation reaches this machine
//! that way **is** §4's principal, and nothing else in this repository is one
//! today.
//!
//! What the CLI does not do is own any of this. `ADR-006` §3 — *"every interface
//! is an adapter containing no logic"* — is why the operation is here, and why
//! the same words will reach it from the network API of `ADR-006` §5.1 without
//! anything below changing.
//!
//! # This is an **apply**, and it takes the `apply` verb
//!
//! `ADR-006` §6.4's verb set is closed and the catalogue is keyed by verb, so
//! taking one is not free. `apply` is the defensible choice and the record says
//! so twice: `ADR-012` §7.1 is titled *confirmation before apply*, and `ADR-028`
//! §4 classes an update as *"`ADR-006` §6 already requires a `plan` the principal
//! confirms. An update is that, not a special case."*
//!
//! **What that forecloses, stated rather than discovered.** `ADR-006` §6 spells
//! the desired-state apply `rivet apply`, with no subject — *"make the machine
//! match the document"*. That operation does not exist (§8.1: plan and apply are
//! not built, and §6.5's dry-run framework is a larger ruling), and when it lands
//! it will have to share this verb with the trial boot rather than replace it.
//! **That is a real cost and it is the maintainer's to accept or overturn**; the
//! shape chosen here keeps it cheap, because `subject` is a declared input rather
//! than a second axis of the catalogue and a second subject is an added match arm.
//! The alternative — a second catalogue key — is a change to `ADR-006` §6.4 and
//! `AGENTS.md` §1 says that is not an agent's to make.
//!
//! # Confirmation, and what is honestly missing
//!
//! `ADR-012` §7.1 requires confirmation before apply and makes unattended apply
//! *"a scope on a credential (§5), never a default and never a flag anyone can
//! simply pass"*. There are no credentials in Rivet and no scopes, so **there
//! is no way to grant unattended apply and this operation does not offer one.**
//!
//! ~~`ADR-006` §6's plan/apply pair is likewise not built, and §6.5's framework
//! — a mutating operation as *"a type that computes its diff separately from
//! applying it"* — does not exist. So this operation supplies the property by
//! hand, for itself, and says which of the two it is~~ — **struck 2026-08-26:
//! §6.5's framework is built and this operation is one of its seven callers.**
//! [`super::plan::Change`] is the type, [`super::plan::resolve`] is the driver,
//! and [`ArmTrialBoot`] below is this subject's implementation of it. `ADR-006`
//! §6's *desired-state* apply is still not built; that half of the struck
//! sentence is unchanged.
//!
//! * **Without the confirmation word it is a plan.** It reads the card, runs
//!   both of `ADR-018` §5.3's interlocks, changes nothing, and refuses with
//!   [`ErrorCode::ConfirmationRequired`] carrying what would change and the exact
//!   command that would do it.
//! * **With it, it applies** — and the interlocks run first there too, because
//!   [`super::plan::resolve`] plans on both paths. **That is a change of
//!   behaviour on the confirmed path**, which used to call `arm_trial_boot`
//!   without asking `can_arm_trial_boot` first. One more read of the card, and
//!   the same check the plan the operator confirmed was computed from.
//!
//! ~~**The one thing that is worse than the framework would be**: the plan
//! travels in the refusal's human string rather than as a typed diff, because
//! `OperationError` carries a code, a subject and a message and nothing else.
//! An agent can branch on the code and re-issue the command; it cannot read the
//! diff structurally.~~ **Struck the same day.** [`OperationError::plan`]
//! carries [`super::plan::Plan`] in `ADR-006` §4.1's structured-payload slot,
//! and the human string is *generated from it* — so the two cannot disagree.
//!
//! **Why confirmation is a second invocation rather than a prompt.** `ADR-005`
//! §5.1 rules `nushell` the interactive login shell and explicitly rejects making
//! `rivet` a shell; `cmd/rivet` refuses an interactive session outright. The
//! operator sends one whole command to the machine and gets one answer back,
//! with no way to be asked a question in between — so the answer is carried in
//! the next command.
//!
//! # It does not reboot, and ~~nothing in Rivet can~~ something now asks
//!
//! [`Machine::arm_trial_boot`] returns which restart carries the arming and
//! stops. That separation is `ADR-018` §5's and it is real — on a UEFI machine
//! the arming is in NVRAM and any restart picks it up. **This operation still
//! never reboots**, and that half is unchanged.
//!
//! ~~**On this hardware the operator cannot presently perform that restart, and
//! that is a gap in the record rather than in this operation.** `ADR-017`
//! §3.1.5 item 6 gives shutdown to PID 1, which restarts on `SIGINT` and powers
//! off on `SIGTERM`; `RebootHow::TrialRestart` exists on its `Kernel` port and
//! **nothing sends it**, and the image contains no program that can signal PID 1
//! at all. So [`Reboot::OneShot`]'s requirement is a true statement about what
//! must happen and not yet an instruction anybody can follow.~~
//!
//! **Struck 2026-08-23**, and struck rather than edited because `AGENTS.md` §7 is
//! about exactly this paragraph — it was written the same day it stopped being
//! true. `ADR-017` §3.1.10 gave PID 1 a third signal and [`super::stop`] gave the
//! operator a way to send it: `rivet apply trial-restart confirm`.
//! [`Reboot::OneShot`]'s requirement is now an instruction somebody can follow,
//! and the two commands are deliberately separate acts.
//!
//! **What has still never happened is the thing itself.** `ADR-026` §4 puts
//! `tryboot` in the column QEMU does not emulate at all, and
//! `reboot(2)` with `LINUX_REBOOT_CMD_RESTART2` and `"0 tryboot"` has been
//! executed exactly once anywhere — in a throwaway PID namespace on a
//! development host, which proves the call is well-formed and accepted by *a*
//! Linux kernel and says nothing whatever about whether Raspberry Pi firmware
//! honours it. `ADR-018` §5.3 item 2 is one experiment away, and no test suite
//! can shorten it.

use super::apply::confirmed;
use super::machine::{Board, MachineError, Reboot};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::state::Slot;
use crate::value::Value;

/// The subject this build applies. `ADR-006` §6.4's nouns *are* schema paths and
/// this is not one — see the module documentation for the cost of that and whose
/// call it is.
pub const SUBJECT: &str = "trial-boot";

/// The fields this subject's result carries.
///
/// ~~The catalogue entry, declared beside the implementation so the two cannot
/// drift — `super`'s module documentation, property 1.~~ **Amended 2026-08-23**,
/// and struck rather than deleted because the property it names still holds and
/// its *scope* changed: `apply` acquired three more subjects
/// ([`super::stop`]), so the entry belongs to the verb and lives in
/// [`super::apply`]. What is declared here is this subject's own output list,
/// which [`super::apply::subject_table`] names and from which the published
/// union is generated — see [`super::log::OUTPUTS`] for the note and the date.
/// The confirmation word and its rule moved to [`super::apply`] for the same
/// reason: four subjects sharing one word is one definition, not four.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied.",
    },
    Field {
        name: "slot",
        description: "The slot the next boot will try once.",
    },
    Field {
        name: "running-slot",
        description: "The slot this machine is running, as the firmware reported it.",
    },
    Field {
        name: "armed",
        description: "Always true in a result. An arming that did not happen is a refusal, \
                      never a result with a false in it (ADR-006 §4.2).",
    },
    Field {
        name: "rebooted",
        description: "Always false. Arming and restarting are two acts and this is the \
                      first (ADR-018 §5).",
    },
    Field {
        name: "reboot",
        description: "Which kind of restart carries the arming — `one-shot` or `ordinary`.",
    },
    Field {
        name: "next",
        description: "What has to happen for the trial boot to occur at all.",
    },
    Field {
        name: "rollback",
        description: "What happens if the trial slot is never committed. Nothing has to be \
                      done for this; it is the failure path (ADR-018 §5.1).",
    },
];

/// An arming that happened.
///
/// A typed result rather than a `Value`, for [`super::Explanation`]'s reason: a
/// future adapter reading this operation programmatically gets fields rather
/// than a document it has to index into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Armed {
    slot: Slot,
    running: Slot,
    reboot: Reboot,
}

impl Armed {
    /// The slot the next boot will try once.
    pub const fn slot(&self) -> Slot {
        self.slot
    }

    /// The slot this machine is running.
    pub const fn running_slot(&self) -> Slot {
        self.running
    }

    /// Which restart carries the arming.
    pub const fn reboot(&self) -> Reboot {
        self.reboot
    }

    /// The structured result, whose keys are `OUTPUTS` — checked here rather
    /// than only in a test, because the catalogue naming a field no result
    /// carries is `ADR-006` §7's "trusted and wrong".
    pub fn value(&self) -> Value {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("slot", Value::str(slot_word(self.slot))),
            ("running-slot", Value::str(slot_word(self.running))),
            ("armed", Value::Bool(true)),
            // Said in the result rather than assumed, because the whole design
            // of ADR-018 §5 rests on the two being separate acts.
            ("rebooted", Value::Bool(false)),
            ("reboot", Value::str(self.reboot.as_str())),
            ("next", Value::str(self.reboot.requirement())),
            (
                "rollback",
                Value::str(format!(
                    "if slot {} is not committed while it is running, the next reset returns to \
                     slot {} with no userspace involvement (ADR-018 §5.1). Withholding the \
                     commit is the rollback; nothing has to be done to cause it.",
                    slot_word(self.slot),
                    slot_word(self.running),
                )),
            ),
        ];
        debug_assert!(
            fields
                .iter()
                .map(|(key, _)| *key)
                .eq(OUTPUTS.iter().map(|f| f.name)),
            "an arming's keys must be exactly OUTPUTS, in the declared order"
        );
        Value::object(fields)
    }
}

/// **The change this subject is**, as `ADR-006` §6.5's framework defines one.
///
/// Nothing but the slot, because that is the whole of what an operator chose.
/// `word` is carried beside it for the refusals, which name what was typed
/// rather than what it parsed to (`AGENTS.md` §5).
struct ArmTrialBoot<'a> {
    slot: Slot,
    word: &'a str,
}

impl Change for ArmTrialBoot<'_> {
    /// The running slot, which the plan reads to say what stays the default and
    /// which the result then reports. Reading it once is the point of the
    /// associated type: the two paths used to read it separately and a card that
    /// answered differently between them would have produced a result
    /// contradicting the plan the operator confirmed.
    type Prepared = Slot;
    type Applied = Armed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Slot), OperationError> {
        let running = board.running_slot().map_err(|e| refused(e, self.word))?;
        // Both interlocks run here, and `resolve` runs this on the confirmed
        // path too — so the ordering the hand-written version had to argue for
        // is now the driver's property. `super::plan` carries the argument.
        board
            .can_arm_trial_boot(self.slot)
            .map_err(|e| refused(e, self.word))?;

        let plan = Plan::new(
            format!("{SUBJECT} {}", self.word),
            "changes what the machine does on its next reset",
        )
        .doing(
            Effect::set(
                "autoboot.txt [tryboot] boot_partition",
                format!("slot {}", slot_word(running)),
                format!("slot {}", slot_word(self.slot)),
            )
            .noting("on ADR-018 §4's first boot partition; the firmware reads it once and \
                     clears it"),
        )
        .leaving(format!(
            "the `[all]` default, so slot {} stays the default and an ordinary reset still \
             boots it",
            slot_word(running)
        ))
        .leaving(
            "NOTHING IS RESTARTED — arming and restarting are two acts and this is only the \
             first",
        )
        .taking_effect(TakesEffect::OnNextRestart)
        .confirmed_by(&[SUBJECT, self.word]);

        Ok((plan, running))
    }

    fn apply(self, running: Slot, board: &mut dyn Board) -> Result<Armed, OperationError> {
        let reboot = board
            .arm_trial_boot(self.slot)
            .map_err(|e| refused(e, self.word))?;
        Ok(Armed {
            slot: self.slot,
            running,
            reboot,
        })
    }
}

/// Perform the operation, with the subject already recognised by
/// [`super::apply::perform`].
///
/// **Arity is this function's own to check, not an outer guard's.** `apply`
/// declares a several-word input for `edit`, so [`super::Input::several`]'s
/// cost falls on the whole verb: `Operation::word_ceiling` returns none for
/// it, and every subject dispatched under `apply` — including this one —
/// must refuse an extra word for itself. A fourth word here is refused by
/// name, the same as `reload`, `install`, `stop`, `service-restart` and the
/// two service-hold subjects.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Armed, OperationError> {
    // The subject is [`super::apply`]'s to recognise — it dispatches on it, and
    // a second copy of that decision here would be a second list of subjects.
    debug_assert_eq!(arguments.first().copied(), Some(SUBJECT));

    let word = arguments.get(1).copied().unwrap_or_default();
    let Some(slot) = parse_slot(word) else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            word,
            format!(
                "not a slot. ADR-018 §4 gives this machine two, and they are spelled `{}` and \
                 `{}`.",
                slot_word(Slot::A),
                slot_word(Slot::B),
            ),
        ));
    };

    // **A trial-boot takes one slot and then the confirmation**, so a fourth
    // word is not a misplaced confirmation — it is a word this command has no
    // meaning for. Refused rather than ignored, which is `super::reload`'s
    // rule and the same hazard: `rivet apply trial-boot b confirm a` must not
    // quietly arm slot b while looking like it also did something with `a`.
    if let Some(extra) = arguments.get(3).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {SUBJECT}` takes one slot and then the confirmation. `{extra}` is a \
                 fourth word, and there is no form of this command that arms two slots: run it \
                 twice, so that each arming is confirmed on its own.",
                apply = Verb::Apply,
            ),
        ));
    }

    // A third word that is not the confirmation is refused rather than ignored:
    // an operator who typed `confrim` must not be told the plan again as though
    // they had typed nothing, because the second refusal would look identical to
    // the first and they would repeat it. The rule and its message are
    // [`super::apply::confirmed`]'s, shared with the three stop subjects.
    let confirmed = confirmed(arguments.get(2).copied())?;

    // Everything above this line is argument parsing; everything below it is
    // `super::plan`'s driver. The plan/apply split is no longer this function's
    // to get right — `ADR-006` §6.5.
    resolve(ArmTrialBoot { slot, word }, confirmed, board)
}

/// The one place a [`MachineError`] becomes an operation's refusal.
///
/// The two variants map onto two codes because an operator does two different
/// things next, which is `ADR-006` §4.1's whole argument for codes over prose.
/// **Nothing is reworded on the way through** — `cmd/rivet::refusal` records
/// the same rule one layer out.
fn refused(error: MachineError, subject: &str) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, subject, error.why())
}

/// The wire spelling of a slot.
///
/// Here rather than on [`Slot`] because it is this operation's argument
/// vocabulary rather than a property of `ADR-018` §4's layout — `crate::state`
/// identifies a slot by its boot partition number, which is what the *firmware*
/// says, and `AGENTS.md` §10.2 keeps the two from becoming one word.
///
/// **`pub(super)` since 2026-08-24**, because [`super::install`] reports which
/// slot an update would land in and there must not be a second spelling of `a`
/// and `b` (`ADR-011`). It stays in *this* module because this is the operation
/// whose argument vocabulary it is — `apply trial-boot <a|b>` — and the other
/// caller only renders it.
pub(super) const fn slot_word(slot: Slot) -> &'static str {
    // **A projection since 2026-08-26, not a definition.** The word is
    // `Slot::as_str`'s now, because `cmd/rivet-sysd`'s boot record has to spell a
    // slot the same way this operation accepts one (`ADR-022` §12 item 1). Kept
    // as a name because this module reads far better for it.
    slot.as_str()
}

/// Read a slot from a word, exactly and without aliases.
///
/// Case-sensitive for `Verb::parse`'s reason: `ADR-006` §6.4's value is that a
/// reader who knows one command can predict the others, and a surface with two
/// spellings of one thing has to be memorised instead. Partition numbers are
/// **not** accepted — `1` and `2` are the firmware's vocabulary and letting them
/// in here would put a number an operator half-remembers next to a slot name.
fn parse_slot(word: &str) -> Option<Slot> {
    match word {
        "a" => Some(Slot::A),
        "b" => Some(Slot::B),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    // Named here rather than at the top of the module: the operation itself no
    // longer spells the confirmation word — `super::plan::Plan::confirmed_by`
    // assembles the line from the one definition — so the only thing left that
    // needs it is the tests that type it.
    use super::super::apply::CONFIRMATION;
    use super::super::machine::Machine;

    /// A machine that answers as scripted and records what it was asked.
    ///
    /// **`ADR-026` §4 puts `tryboot` and `/proc/device-tree/chosen/bootloader` in
    /// the column QEMU does not emulate at all**, so this is not a substitute for
    /// a board and is not offered as one — it is what makes every branch of the
    /// operation reachable on a development host, which is `ADR-014` §3.1's
    /// standing check (`cargo test -p rivet-core` needs no device).
    struct FakeMachine {
        running: Result<Slot, MachineError>,
        can_arm: Result<(), MachineError>,
        arm: Result<Reboot, MachineError>,
        armed: Vec<Slot>,
        planned: Vec<Slot>,
    }

    crate::operation::machine::refuses_to_edit!(FakeMachine);

    impl FakeMachine {
        /// A healthy board running slot A.
        fn healthy() -> Self {
            Self {
                running: Ok(Slot::A),
                can_arm: Ok(()),
                arm: Ok(Reboot::OneShot),
                armed: Vec::new(),
                planned: Vec::new(),
            }
        }
    }

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
            self.running.clone()
        }

        fn can_arm_trial_boot(&mut self, slot: Slot) -> Result<(), MachineError> {
            self.planned.push(slot);
            self.can_arm.clone()
        }

        fn arm_trial_boot(&mut self, slot: Slot) -> Result<Reboot, MachineError> {
            self.armed.push(slot);
            self.arm.clone()
        }

        /// **This operation stops nothing**, so a fake that could would be a
        /// fake with a capability the thing it stands in for does not use.
        /// `AGENTS.md` §3 — refuse, never pretend.
        fn can_request_stop(
            &mut self,
            _stop: crate::operation::machine::Stop,
        ) -> Result<(), MachineError> {
            Err(unreachable_for_an_arming())
        }

        fn request_stop(
            &mut self,
            _stop: crate::operation::machine::Stop,
        ) -> Result<(), MachineError> {
            Err(unreachable_for_an_arming())
        }
    }

    /// **This operation installs nothing**, and one test here reaches
    /// `apply::perform`, which since 2026-08-24 takes the whole
    /// [`crate::operation::Board`]. So the fake answers on the third port and
    /// refuses there too — the same posture as the two stop methods above and
    /// for the same reason (`AGENTS.md` §3).
    impl crate::operation::machine::Deployer for FakeMachine {
        /// **`crate::repo_delete`'s two, added 2026-08-26.** This fake destroys
        /// nothing and refusing is the honest stand-in (`AGENTS.md` §3): a test
        /// in this module that reached this port has changed what it is about.
        fn can_request_repo_delete(
            &mut self,
            _name: &str,
        ) -> Result<(), crate::operation::machine::MachineError> {
            Err(crate::operation::machine::MachineError::Refused(
                "this fake destroys no repository; an operation that reached this port has \
                 changed"
                    .to_string(),
            ))
        }
        fn request_repo_delete(
            &mut self,
            _name: &str,
            _id: &str,
        ) -> Result<(), crate::operation::machine::MachineError> {
            self.can_request_repo_delete(_name)
        }

        fn can_request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(unreachable_for_an_arming())
        }

        fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
            Ok(())
        }
        fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
            Ok(())
        }
        fn can_request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(unreachable_for_an_arming())
        }
        fn request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(unreachable_for_an_arming())
        }
        fn request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(unreachable_for_an_arming())
        }
    }

    /// Likewise the observation port, for the same one caller.
    impl crate::operation::machine::Observed for FakeMachine {
        /// **Refuses rather than answering that this machine trusts nobody.**
        /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
        /// `--no-trust` image produces -- and this fake has not taken it. The
        /// direction is the dangerous one: *"trusts nobody"* is the answer
        /// somebody acts on by re-signing an artifact.
        fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an idle machine.** Zeroed counters
        /// are what a machine that has run nothing since it booted looks like,
        /// and a load average of zero is what an idle one looks like — both are
        /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
        fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering that this machine has no block
        /// devices**, which is not a state a machine that booted off one can
        /// be in.
        fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an empty network.** No interfaces at
        /// all reads as a machine with none, which is not a state a running
        /// Linux is in.
        fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than naming a slot.** Naming the wrong one is how
        /// an update overwrites the running system, so a fake that observes
        /// nothing names none.
        fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an uptime of zero.** `0` would say
        /// this machine booted a moment ago, which is a reading nobody took.
        fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering with unset flags.** Reporting a
        /// `Throttle` of all-`false` would assert that this machine has never
        /// been under-volted since it booted — the one claim this subject must
        /// never make without a measurement (`AGENTS.md` §3, §5).
        fn read_thermal(
            &mut self,
        ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
            Err(MachineError::Refused(
                    "not this fake's port; unread throttle flags are never reported as false"
                        .to_string(),
            ))
        }
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(
        &mut self,
    ) -> Result<crate::operation::machine::MemoryRead, crate::operation::machine::MachineError> {
        Err(crate::operation::machine::MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, crate::operation::machine::MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. This fake asks no
        /// supervisor and refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(unreachable_for_an_arming())
        }

        fn read_journal(
            &mut self,
        ) -> Result<crate::operation::machine::JournalRead, MachineError> {
            Err(unreachable_for_an_arming())
        }

        /// The fake supplies no schema: `rivet-core` cannot name one
        /// (`ADR-014` §3.2), so a test that needs the real document belongs in
        /// the adapter that has both crates. Loud rather than empty, so an
        /// assertion against real schema content fails instead of passing.
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }

        fn read_filesystems(
            &mut self,
        ) -> Result<crate::operation::machine::StorageRead, MachineError> {
            Err(unreachable_for_an_arming())
        }
    }

    fn unreachable_for_an_arming() -> MachineError {
        MachineError::Refused(
            "arming a trial boot never stops a machine; if this was reached, ADR-018 §5's two \
             acts have become one"
                .to_string(),
        )
    }

    fn apply(machine: &mut dyn Board, args: &[&str]) -> Result<Armed, OperationError> {
        perform(args, machine)
    }

    fn fields(armed: &Armed) -> Vec<(&'static str, Value)> {
        match armed.value() {
            Value::Object(fields) => fields,
            other => panic!("a result must be an object, got {other:?}"),
        }
    }

    fn field(armed: &Armed, key: &str) -> Value {
        match fields(armed).into_iter().find(|(k, _)| *k == key) {
            Some((_, value)) => value,
            None => panic!("no {key} field"),
        }
    }

    // -- confirmation ----------------------------------------------------------

    /// **`ADR-012` §7.1, as the property that matters**: without the word, the
    /// machine is not touched at all.
    #[test]
    fn without_the_confirmation_word_nothing_is_armed() {
        let mut m = FakeMachine::healthy();
        let refusal = apply(&mut m, &[SUBJECT, "b"]).expect_err("it armed without confirmation");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert!(
            m.armed.is_empty(),
            "arm_trial_boot was called: {:?}",
            m.armed
        );
        assert_eq!(
            m.planned,
            vec![Slot::B],
            "the plan did not consult the card"
        );
    }

    /// And the refusal has to be a *plan*: what changes, what does not, that
    /// nothing restarts, and the exact next command.
    #[test]
    fn the_unconfirmed_refusal_carries_the_plan_and_the_command_that_performs_it() {
        let mut m = FakeMachine::healthy();
        let refusal = apply(&mut m, &[SUBJECT, "b"]).expect_err("it armed without confirmation");
        let message = refusal.message();
        assert!(message.contains("[tryboot]"), "{message}");
        assert!(message.contains("[all]"), "{message}");
        assert!(message.contains("ADR-012 §7.1"), "{message}");
        assert!(
            message.contains(&format!("rivet apply {SUBJECT} b {CONFIRMATION}")),
            "the refusal does not say what to type: {message}"
        );
        assert_eq!(refusal.subject(), Some(format!("{SUBJECT} b").as_str()));
    }

    /// **And it carries the plan as fields, not only as that sentence.**
    /// `ADR-006` §6.5, `ADR-034` §5.3. This is the property `PLAN.md` §22.3 said
    /// was missing: an agent asserting *this is the change I intended* against
    /// prose is doing string matching on the safety mechanism.
    #[test]
    fn the_refusal_carries_a_typed_plan_that_says_it_needs_a_restart() {
        let mut m = FakeMachine::healthy();
        let refusal = apply(&mut m, &[SUBJECT, "b"]).expect_err("it armed without confirmation");
        let plan = refusal.plan().expect("a confirmation refusal carries its plan");

        assert_eq!(plan.subject(), "trial-boot b");
        assert_eq!(plan.confirm_with(), "rivet apply trial-boot b confirm");
        // ADR-034 §5.3's whole point: arming is written now and is not live.
        assert_eq!(plan.takes_effect().requires_reboot(), Some(true));

        let [effect] = plan.effects() else {
            panic!("arming changes exactly one thing: {:?}", plan.effects());
        };
        assert_eq!(effect.act, crate::operation::Act::Set);
        assert!(effect.target.contains("boot_partition"), "{effect:?}");
        assert_eq!(effect.from.as_deref(), Some("slot a"));
        assert_eq!(effect.to.as_deref(), Some("slot b"));
        // The `[all]` default and the fact that nothing restarts are the two
        // things an operator gets wrong here, and both are fields rather than
        // sentences an agent would have to find.
        assert_eq!(plan.untouched().len(), 2, "{:?}", plan.untouched());
    }

    /// The interlock runs before the change on the confirmed path too, which is
    /// `super::plan::resolve` planning on both paths. It did not before
    /// 2026-08-26 — see this module's documentation.
    #[test]
    fn a_confirmed_arming_asks_the_interlock_before_it_arms() {
        let mut m = FakeMachine::healthy();
        apply(&mut m, &[SUBJECT, "b", CONFIRMATION]).expect("a healthy board");
        assert_eq!(m.planned, vec![Slot::B], "the interlock was skipped");
        assert_eq!(m.armed, vec![Slot::B]);
    }

    /// With the word, it arms — and asks the machine for the slot it was told,
    /// never for a slot it worked out.
    #[test]
    fn with_the_confirmation_word_the_named_slot_is_armed() {
        let mut m = FakeMachine::healthy();
        let armed = apply(&mut m, &[SUBJECT, "b", CONFIRMATION]).expect("a healthy board");
        assert_eq!(m.armed, vec![Slot::B]);
        assert_eq!(armed.slot(), Slot::B);
        assert_eq!(armed.running_slot(), Slot::A);
        assert_eq!(armed.reboot(), Reboot::OneShot);
    }

    /// A misspelt confirmation is refused rather than treated as absent. The two
    /// refusals would otherwise look identical and the operator would repeat the
    /// typo.
    #[test]
    fn a_third_word_that_is_not_the_confirmation_is_refused_rather_than_ignored() {
        let mut m = FakeMachine::healthy();
        for word in ["confrim", "yes", "--confirm", "CONFIRM", ""] {
            let refusal = apply(&mut m, &[SUBJECT, "b", word])
                .expect_err("only the exact confirmation word may confirm");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{word:?}");
            assert_eq!(refusal.subject(), Some(word));
        }
        assert!(m.armed.is_empty());
    }

    /// **A fourth word is refused by name, and the card is never even read.**
    /// `apply trial-boot b confirm a` must not quietly arm slot b while
    /// looking like it also did something with `a` — the same hazard
    /// `super::service_restart` names for its own fourth word. Checked before
    /// the card is consulted at all: neither `planned` nor `armed` gains an
    /// entry, which a misread of the confirmation word (the test above) does
    /// not promise, because that one is caught one step later.
    #[test]
    fn a_fourth_word_is_refused_and_nothing_is_planned_or_armed() {
        let mut m = FakeMachine::healthy();
        for args in [
            [SUBJECT, "b", CONFIRMATION, "a"],
            [SUBJECT, "b", CONFIRMATION, "--force"],
        ] {
            let refusal = apply(&mut m, &args).expect_err("a fourth word is refused");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{args:?}");
            assert_eq!(refusal.subject(), Some(args[3]));
        }
        assert!(m.planned.is_empty(), "the card was read: {:?}", m.planned);
        assert!(m.armed.is_empty(), "a slot was armed: {:?}", m.armed);
    }

    // -- the interlocks reach the operator ------------------------------------

    /// Interlock refusals are `E_UNSAFE` and machine failures are
    /// `E_MACHINE_REFUSED`, because an operator does two different things next.
    #[test]
    fn the_two_kinds_of_machine_refusal_become_two_different_codes() {
        let mut unsafe_card = FakeMachine::healthy();
        unsafe_card.arm = Err(MachineError::Unsafe("tryboot_a_b is not set".to_string()));
        unsafe_card.can_arm = Err(MachineError::Unsafe("tryboot_a_b is not set".to_string()));

        let mut broken = FakeMachine::healthy();
        broken.arm = Err(MachineError::Refused("EROFS".to_string()));
        broken.can_arm = Err(MachineError::Refused("EROFS".to_string()));

        for (machine, expected) in [
            (&mut unsafe_card, ErrorCode::Unsafe),
            (&mut broken, ErrorCode::MachineRefused),
        ] {
            for args in [
                vec![SUBJECT, "b"].as_slice(),
                vec![SUBJECT, "b", CONFIRMATION].as_slice(),
            ] {
                let refusal = apply(machine, args).expect_err("it should have refused");
                assert_eq!(refusal.code(), expected, "{args:?}");
            }
        }
    }

    /// **The message is carried out unchanged.** The interlocks say what to
    /// change, and an operation that improved on the wording would be a second
    /// account of the same refusal.
    #[test]
    fn a_machine_refusal_reaches_the_operator_word_for_word() {
        let why = "this machine is running boot partition 1; arm slot 2 instead";
        let mut m = FakeMachine::healthy();
        m.can_arm = Err(MachineError::Unsafe(why.to_string()));
        let refusal = apply(&mut m, &[SUBJECT, "a"]).expect_err("the interlock refuses");
        assert_eq!(refusal.message(), why);
    }

    /// A machine that cannot say which slot it is running refuses before
    /// anything else. Under an emulator this is every run (`ADR-026` §4).
    #[test]
    fn a_machine_that_cannot_report_its_slot_refuses_rather_than_assuming_one() {
        let mut m = FakeMachine::healthy();
        m.running = Err(MachineError::Refused(
            "cannot read /proc/device-tree/chosen/bootloader/partition".to_string(),
        ));
        for args in [
            vec![SUBJECT, "b"].as_slice(),
            vec![SUBJECT, "b", CONFIRMATION].as_slice(),
        ] {
            let refusal = apply(&mut m, args).expect_err("nothing may be armed blind");
            assert_eq!(refusal.code(), ErrorCode::MachineRefused, "{args:?}");
        }
        assert!(m.armed.is_empty());
    }

    // -- the argument surface --------------------------------------------------

    /// **Driven through [`super::apply::perform`], because that is what owns the
    /// subject.** It used to be this module's check; with four subjects behind
    /// one verb it moved, and the test moved with it rather than being deleted —
    /// what it asserts is a property of the surface, not of where the `if` lives.
    #[test]
    fn only_a_known_subject_is_applied_and_the_refusal_lists_the_ones_that_are() {
        let mut m = FakeMachine::healthy();
        for subject in [
            "",
            "config",
            "trial_boot",
            "Trial-Boot",
            "/state/config",
            "poweroff",
        ] {
            let refusal = super::super::apply::perform(&[subject, "b"], &mut m)
                .expect_err("a subject this build does not apply must be refused");
            assert_eq!(refusal.code(), ErrorCode::UnknownSubject, "{subject:?}");
            assert_eq!(refusal.subject(), Some(subject));
            for known in super::super::apply::subjects() {
                assert!(
                    refusal.message().contains(known),
                    "{known} is not listed: {}",
                    refusal.message()
                );
            }
        }
        assert!(m.armed.is_empty());
    }

    /// Both spellings and nothing else. A partition number is the firmware's
    /// vocabulary and is deliberately not accepted here.
    #[test]
    fn a_slot_is_a_or_b_and_a_partition_number_is_not_one() {
        let mut m = FakeMachine::healthy();
        for word in ["1", "2", "A", "B", "", "c", "boot-a", "3"] {
            let refusal =
                apply(&mut m, &[SUBJECT, word, CONFIRMATION]).expect_err("only a and b are slots");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{word:?}");
            assert_eq!(refusal.subject(), Some(word));
            assert!(refusal.message().contains("`a`"), "{}", refusal.message());
            assert!(refusal.message().contains("`b`"), "{}", refusal.message());
        }
        assert!(m.armed.is_empty(), "a bad slot reached the machine");
    }

    /// The two slots round-trip through the words this operation accepts, so a
    /// result can always be fed back in as a request.
    #[test]
    fn every_slot_has_a_word_and_every_word_is_a_slot() {
        for slot in [Slot::A, Slot::B] {
            assert_eq!(parse_slot(slot_word(slot)), Some(slot));
        }
        assert_ne!(slot_word(Slot::A), slot_word(Slot::B));
    }

    // -- the result ------------------------------------------------------------

    /// The catalogue may not name a field the result does not carry, and the
    /// declared order is the produced order — `ADR-006` §7's "trusted and wrong"
    /// in the smallest available form.
    #[test]
    fn the_result_carries_exactly_the_fields_the_catalogue_declares_in_order() {
        let mut m = FakeMachine::healthy();
        let armed = apply(&mut m, &[SUBJECT, "b", CONFIRMATION]).expect("a healthy board");
        let keys: Vec<&str> = fields(&armed).iter().map(|(k, _)| *k).collect();
        let declared: Vec<&str> = OUTPUTS.iter().map(|o| o.name).collect();
        assert_eq!(keys, declared);
        // ...and every one of them is published by the entry that describes this
        // subject, which is `apply`'s and covers three more.
        let entry =
            super::super::operation(super::super::Verb::Apply).expect("apply is catalogued");
        let entry_outputs = entry.outputs();
        let published: Vec<&str> = entry_outputs.iter().map(|o| o.name).collect();
        for key in keys {
            assert!(
                published.contains(&key),
                "{key} is not published: {published:?}"
            );
        }
    }

    /// **A result never says the machine restarted, and never says it did not
    /// arm.** Both are constants in the value and both are the point: `ADR-006`
    /// §4.2 makes a non-result a typed refusal, so there is no result carrying
    /// `armed: false` for a caller to misread.
    #[test]
    fn a_result_reports_an_arming_that_happened_and_a_restart_that_did_not() {
        let mut m = FakeMachine::healthy();
        let armed = apply(&mut m, &[SUBJECT, "b", CONFIRMATION]).expect("a healthy board");
        assert_eq!(field(&armed, "armed"), Value::Bool(true));
        assert_eq!(field(&armed, "rebooted"), Value::Bool(false));
        assert_eq!(field(&armed, "reboot"), Value::str("one-shot"));
    }

    /// The result has to say what still has to happen, because on this hardware
    /// an ordinary restart boots the default slot and trials nothing.
    #[test]
    fn the_result_says_what_restart_is_needed_and_what_a_rollback_looks_like() {
        let mut m = FakeMachine::healthy();
        let armed = apply(&mut m, &[SUBJECT, "b", CONFIRMATION]).expect("a healthy board");
        let next = match field(&armed, "next") {
            Value::Str(s) => s,
            other => panic!("next is {other:?}"),
        };
        assert!(next.contains("tryboot"), "{next}");
        let rollback = match field(&armed, "rollback") {
            Value::Str(s) => s,
            other => panic!("rollback is {other:?}"),
        };
        assert!(rollback.contains("ADR-018 §5.1"), "{rollback}");
        assert!(rollback.contains("slot a"), "{rollback}");
    }

    /// A UEFI machine, behind the same boot-slot port, would answer `Ordinary`, and
    /// the result must then say something different. Nothing produces this
    /// today; the operation must not have baked the Pi's answer in.
    #[test]
    fn an_ordinary_arming_reports_a_different_requirement_from_a_one_shot() {
        let mut m = FakeMachine::healthy();
        m.arm = Ok(Reboot::Ordinary);
        let armed = apply(&mut m, &[SUBJECT, "b", CONFIRMATION]).expect("a healthy board");
        assert_eq!(field(&armed, "reboot"), Value::str("ordinary"));
        assert_ne!(
            field(&armed, "next"),
            Value::str(Reboot::OneShot.requirement())
        );
    }
}
