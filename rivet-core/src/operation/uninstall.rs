//! `rivet apply uninstall <name> [purge] confirm` — remove an installed
//! application. Ruled 2026-08-25.
//!
//! `crate::uninstall` carries the mechanism and the two arguments that shaped
//! it: why this is the **supervisor's** and not the installer's, and why `data`
//! survives unless the operator says otherwise. This module is the operation.
//!
//! # It delivers a request and returns, exactly as `install` does
//!
//! And for a sharper reason than that one's. `install` returns early because an
//! update writes 640 MiB; this returns early because **the machine may have to
//! stop a running service first**, with `ADR-017` §3.5's deadline and its
//! `SIGKILL` escalation. An answer that waited would hold a caller's connection
//! open across a stop that is allowed to take ten seconds and is allowed to end
//! in a kill.
//!
//! So `requested: true` means *the request was written*. `rivet get log` is
//! where the removal reports, on a record whose `svc` is `rivet-sysd` — that
//! operation has no service filter, so the name is never a word to give it.

use super::{ErrorCode, Field, OperationError, Verb};
use crate::operation::machine::{Board, MachineError};
use crate::operation::plan::{Change, Effect, Plan, TakesEffect, resolve};
use crate::uninstall::PURGE;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "uninstall";

/// The literal `ADR-012` §7.1 requires as the last word.
///
/// `pub(crate)` so that `crate::service::reserved_application_names`
/// (`.rulings/` R-210 D21) reads this grammar's own words, this one among
/// them, rather than a second hand-written copy.
pub(crate) const CONFIRMATION: &str = "confirm";

/// What this subject's result carries.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `uninstall` in this build.",
    },
    Field {
        name: "application",
        description: "The application named, carried back verbatim.",
    },
    Field {
        name: "requested",
        description: "Always true in a result. It means the request was DELIVERED, not that \
                      anything has been removed (ADR-006 §4.2).",
    },
    Field {
        name: "removed",
        description: "Always false. NOTHING IS GONE when this answers — the supervisor stops \
                      the application first if it is running, which ADR-017 §3.5 allows to \
                      take a deadline and to end in a SIGKILL.",
    },
    Field {
        name: "purge",
        description: "Whether the application's private data goes too. FALSE unless the word \
                      `purge` was typed, ruled 2026-08-25: removing a program is not consent \
                      to discard what it wrote.",
    },
    Field {
        name: "keeps",
        description: "Everything this LEAVES BEHIND, named rather than implied, so nobody has to \
                      guess what is still there — and each entry names the command that removes \
                      it. Two things survive a plain uninstall: the application's private data, \
                      and the `[services.<name>]` table in this machine's document. `purge` \
                      removes the first and NEITHER the word nor the verb touches the second, \
                      which is why this field is present either way.",
    },
    Field {
        name: "next",
        description: "Where the outcome appears, and what to watch.",
    },
];

/// Everything an operator must be told, whether this is a plan or a result.
fn answer(name: &str, purge: bool) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("application", Value::str(name.to_string())),
        ("requested", Value::Bool(true)),
        ("removed", Value::Bool(false)),
        ("purge", Value::Bool(purge)),
    ];
    let mut kept: Vec<String> = Vec::new();
    if !purge {
        kept.push(format!(
            "{}/{name}/{} — this application's private data (ADR-015 §3.2's `state = \
             \"private\"`). It is the operator's, not the program's, so removing the program \
             does not remove it. `{} {SUBJECT} {name} {PURGE} {CONFIRMATION}` is the command \
             that does.",
            crate::service::APP_ROOT,
            crate::service::DATA_DIR,
            Verb::Apply,
        ));
    }
    // **The second thing this keeps, and `purge` does not reach it.** The
    // document is what the operator asked this machine to be, so removing a
    // program is not an answer to whether it was wanted — `ADR-017` §6. Left
    // standing with `enabled = true` it is an ORPHANED DECLARATION: the machine
    // is asking its supervisor for a program that is not there.
    kept.push(format!(
        "{doc}'s `[services.{name}]` table, if the document has one — REMOVING A PROGRAM IS NOT \
         AN ANSWER TO WHETHER IT WAS WANTED (ADR-017 §6), and `{PURGE}` does not reach it \
         either. A table left standing with `enabled = true` is an ORPHANED DECLARATION: this \
         machine goes on asking its supervisor for a program that is not installed, and gets a \
         warning per boot instead of a service. `{apply} {edit} {doc} --expect '<the \
         services.{name} table>' --replace '' {CONFIRMATION}` is the command that removes it, \
         and `{apply} {reload} {CONFIRMATION}` takes the change now.",
        doc = crate::state::MACHINE_DOCUMENT,
        apply = Verb::Apply,
        edit = super::edit::SUBJECT,
        reload = super::reload::SUBJECT,
    ));
    fields.push(("keeps", Value::str(kept.join(" "))));
    fields.push((
        "next",
        Value::str(
            "the request was DELIVERED and nothing has been removed yet. The supervisor stops \
             the application first if it is running — ADR-017 §3.5's deadline, and a SIGKILL \
             if it will not go — and removes the tree after. Watch it with `rivet get log` — \
             that operation has no service filter, so select on the record whose `svc` is \
             `rivet-sysd`. An application that was not installed is a refusal from the \
             supervisor, in the journal, not from this command: this one checks the NAME and \
             the machine checks what is on it.",
        ),
    ));
    fields
}

/// Perform it. `arguments[0]` is the subject.
pub(super) fn perform(arguments: &[&str], board: &mut dyn Board) -> Result<Value, OperationError> {
    let Some(name) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the name of the application to remove, as the word \
                 after `{SUBJECT}` — `rivet {apply} {SUBJECT} <name> {CONFIRMATION}`. The \
                 name is the directory under {root}, which is what `rivet get log` calls the \
                 service.",
                apply = Verb::Apply,
                root = crate::service::APP_ROOT,
            ),
        ));
    };

    // A word that is only the confirmation is a forgotten name — `install`'s
    // precedent, and the same failure it prevents: reporting a delivered
    // request for a command that named nothing.
    if name == CONFIRMATION || name == PURGE {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            name,
            format!(
                "`{name}` is a keyword of this command and cannot be the application's name, \
                 so this names nothing to remove. The name goes first: `rivet {apply} \
                 {SUBJECT} <name> [{PURGE}] {CONFIRMATION}`.",
                apply = Verb::Apply,
            ),
        ));
    }

    // The words after the name, in any order, and every one accounted for.
    let mut purge = false;
    let mut confirmed = false;
    for word in arguments.iter().skip(2).copied() {
        match word {
            PURGE => purge = true,
            CONFIRMATION => confirmed = true,
            other => {
                return Err(OperationError::about(
                    ErrorCode::Arguments,
                    other,
                    format!(
                        "`{other}` is not a word this command takes. After the name come \
                         `{PURGE}` and `{CONFIRMATION}`, and nothing else — a MISSPELLING OF \
                         EITHER IS REFUSED rather than read as absent (ADR-012 §7.1), because \
                         the two mistakes it hides are removing data nobody asked to remove \
                         and removing nothing while reporting success."
                    ),
                ));
            }
        }
    }

    // **The name is checked before the machine is touched**, so a plan and a
    // confirmed run refuse an impossible name identically — and so nothing that
    // could traverse reaches a path.
    if crate::service::ServiceName::new(name).is_none() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            name,
            format!(
                "`{name}` is not a usable application name (ADR-017 §3.1.3), so no directory \
                 under {root} could correspond to it. Names are what `rivet get log` shows \
                 in its `svc` column.",
                root = crate::service::APP_ROOT,
            ),
        ));
    }

    // Everything above this line is argument parsing; the plan/apply split is
    // `super::plan`'s driver — `ADR-006` §6.5.
    resolve(RequestUninstall { name, purge }, confirmed, board)
}

/// **The change `apply uninstall` is**, as `ADR-006` §6.5 defines one.
struct RequestUninstall<'a> {
    name: &'a str,
    purge: bool,
}

impl Change for RequestUninstall<'_> {
    /// Nothing. The name and the `purge` flag are the whole request, and both
    /// were settled by argument parsing before this type was built.
    type Prepared = ();
    /// A [`Value`] rather than a struct, because this subject never had one:
    /// [`answer`] builds the object directly, and inventing a typed result here
    /// would be a change this ruling did not ask for.
    type Applied = Value;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, ()), OperationError> {
        board.can_request_uninstall(self.name).map_err(refused)?;

        let root = crate::service::APP_ROOT;
        let data = format!(
            "{root}/{name}/{dir}",
            name = self.name,
            dir = crate::service::DATA_DIR
        );

        let mut plan = Plan::new(
            format!("{SUBJECT} {}", self.name),
            "removes an application from the machine",
        )
        .doing(
            Effect::remove(format!("{root}/{}", self.name)).was(format!(
                "every installed version of `{}`, its `current` link and its manifest",
                self.name
            )),
        );

        // **The one irreversible thing in this catalogue**, and the `purge` word
        // is the whole of what decides it — so it is its own effect rather than
        // a clause. An agent reading the plan sees a second `remove` or does
        // not, which is a stronger signal than a sentence that differs.
        plan = if self.purge {
            plan.doing(Effect::remove(data.clone()).noting(format!(
                "`{PURGE}` was asked for, so what this application wrote is removed with it and \
                 there is no copy anywhere"
            )))
        } else {
            plan.leaving(format!(
                "its data at {data} — a plain uninstall removes the program and leaves what it \
                 wrote (ADR-015 §3.2). Add `{PURGE}` before the confirmation to remove that too"
            ))
        };

        // **Said on BOTH branches**, because `purge` does not reach the
        // document — a plan that named the data twice and the declaration never
        // was enumerating one of the two things this verb keeps. `ADR-017` §6.
        plan = plan.leaving(format!(
            "the `[services.{name}]` table in {doc}, if the document has one — the document is \
             what this machine was asked to be, and no uninstall rewrites it. `enabled = true` \
             there after this becomes an ORPHANED DECLARATION and the supervisor warns about it \
             every boot; `{apply} {edit} {doc}` is what clears it",
            name = self.name,
            doc = crate::state::MACHINE_DOCUMENT,
            apply = Verb::Apply,
            edit = super::edit::SUBJECT,
        ));

        let mut words = vec![SUBJECT.to_string(), self.name.to_string()];
        if self.purge {
            words.push(PURGE.to_string());
        }

        Ok((
            plan.doing(
                Effect::request(
                    // The supervisor rather than a path: `Act::Request`'s target is
                    // WHO is asked, and this one is asked because an application
                    // may be running and nothing else on the machine can stop a
                    // service. `rivet get log` is where it answers, on a record
                    // whose `svc` is `rivet-sysd` (corrected 2026-08-30: that
                    // operation has no service filter).
                    "the supervisor",
                    format!("stop `{}` first if it is running, then remove it", self.name),
                )
                .noting(
                    "the supervisor disables it and waits, with ADR-017 §3.5's deadline and a \
                     SIGKILL if it will not go — so this is not instantaneous and this command \
                     does not wait for it",
                ),
            )
            .leaving("NOTHING HAS BEEN REMOVED — the request is delivered only if this is confirmed")
            .taking_effect(TakesEffect::Immediately)
            .confirmed_by(&words),
            (),
        ))
    }

    fn apply(self, (): (), board: &mut dyn Board) -> Result<Value, OperationError> {
        board
            .request_uninstall(self.name, self.purge)
            .map_err(refused)?;
        Ok(Value::object(answer(self.name, self.purge)))
    }
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
    use crate::operation::machine::{
        Deployer, JournalRead, Machine, MemoryRead, Observed, Reboot, Stop, StorageRead,
    };
    use crate::state::Slot;

    /// **A board that answers on this operation's two methods and refuses on
    /// every other**, so a test here that started depending on something else
    /// fails loudly rather than passing on a fiction (`AGENTS.md` §3).
    #[derive(Default)]
    struct FakeBoard {
        asked: Vec<(String, bool)>,
    }

    fn nope() -> MachineError {
        MachineError::Refused("this fake answers only the uninstall methods".to_string())
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

    impl Machine for FakeBoard {
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
    }

    impl Observed for FakeBoard {
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
    fn read_memory(&mut self) -> Result<MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. This fake asks no
        /// supervisor and refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(MachineError::Refused(
                "this fake answers no supervisor; an operation that reached this port has changed"
                    .to_string(),
            ))
        }

        fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
            Err(nope())
        }
        fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
            Err(nope())
        }
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }
    }

    impl Deployer for FakeBoard {
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
            Err(nope())
        }
        fn request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(nope())
        }
        fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
            Ok(())
        }
        fn request_uninstall(&mut self, name: &str, purge: bool) -> Result<(), MachineError> {
            self.asked.push((name.to_string(), purge));
            Ok(())
        }
        fn can_request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(nope())
        }
        fn request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(nope())
        }
    }

    crate::operation::machine::refuses_to_edit!(FakeBoard);

    fn run(words: &[&str]) -> Result<Value, OperationError> {
        let mut board = FakeBoard::default();
        perform(words, &mut board)
    }

    /// **What reached the machine is what was typed.** Asserted separately from
    /// the rendered answer, because a result that says `purge: true` while
    /// having asked for `false` is the defect worth catching.
    #[test]
    fn the_request_that_reaches_the_machine_carries_the_word_that_was_typed() {
        for (words, expected) in [
            (vec![SUBJECT, "photo-index", CONFIRMATION], false),
            (vec![SUBJECT, "photo-index", PURGE, CONFIRMATION], true),
        ] {
            let mut board = FakeBoard::default();
            perform(&words, &mut board).expect("delivered");
            assert_eq!(
                board.asked,
                vec![("photo-index".to_string(), expected)],
                "{words:?}"
            );
        }
    }

    /// A plan asks the machine nothing.
    #[test]
    fn a_plan_delivers_no_request_at_all() {
        let mut board = FakeBoard::default();
        let _ = perform(&[SUBJECT, "photo-index"], &mut board);
        assert!(board.asked.is_empty(), "{:?}", board.asked);
    }

    fn field(value: &Value, key: &str) -> Value {
        match value {
            Value::Object(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("no {key}")),
            other => panic!("not an object: {other:?}"),
        }
    }

    /// **`purge` is off unless typed**, which is the ruling of 2026-08-25 as one
    /// assertion.
    #[test]
    fn a_plain_uninstall_keeps_the_data_and_says_where_it_is() {
        let got = run(&[SUBJECT, "photo-index", CONFIRMATION]).expect("delivered");
        assert_eq!(field(&got, "purge"), Value::Bool(false));
        let keeps = field(&got, "keeps");
        let Value::Str(text) = keeps else {
            panic!("keeps is not a string")
        };
        assert!(text.contains("/state/app/photo-index/data"), "{text}");
        assert!(text.contains(PURGE), "the way to remove it must be named: {text}");
    }

    /// **Both things this verb keeps are enumerated, not one of them.** The
    /// answer named the data directory and said nothing about the
    /// `[services.<name>]` table, so an operator who followed it exactly was
    /// left with a document asking for a program that is no longer installed —
    /// `ADR-017` §6, found on hardware 2026-09-07.
    #[test]
    fn the_service_declaration_is_named_as_kept_and_so_is_the_command_that_clears_it() {
        for words in [
            vec![SUBJECT, "photo-index", CONFIRMATION],
            vec![SUBJECT, "photo-index", PURGE, CONFIRMATION],
        ] {
            let got = run(&words).expect("delivered");
            let Value::Str(text) = field(&got, "keeps") else {
                panic!("keeps is not a string for {words:?}")
            };
            assert!(
                text.contains("[services.photo-index]"),
                "the declaration this leaves behind is not named: {text}"
            );
            assert!(
                text.contains(crate::state::MACHINE_DOCUMENT),
                "the document holding it is not named: {text}"
            );
            assert!(
                text.contains(super::super::edit::SUBJECT),
                "the command that clears it is not named: {text}"
            );
        }
    }

    /// **`purge` removes the data and still keeps the declaration**, so the
    /// field is present either way. It used to be absent here, on the promise
    /// that a purge kept nothing — which was never true of the document.
    #[test]
    fn purge_removes_the_data_and_says_so_without_claiming_to_keep_nothing() {
        let got = run(&[SUBJECT, "photo-index", PURGE, CONFIRMATION]).expect("delivered");
        assert_eq!(field(&got, "purge"), Value::Bool(true));
        let Value::Str(text) = field(&got, "keeps") else {
            panic!("keeps is not a string")
        };
        assert!(
            !text.contains("/state/app/photo-index/data"),
            "a purge claimed to keep the data it removes: {text}"
        );
    }

    /// **Nothing is removed when this answers.** The field exists so an agent
    /// cannot read `requested` as `removed`.
    #[test]
    fn the_answer_says_nothing_has_been_removed_yet() {
        let got = run(&[SUBJECT, "photo-index", CONFIRMATION]).expect("delivered");
        assert_eq!(field(&got, "requested"), Value::Bool(true));
        assert_eq!(field(&got, "removed"), Value::Bool(false));
    }

    /// Without the confirmation it is a plan and the machine is not asked to do
    /// anything — **and the data this keeps is a field, not a sentence.**
    ///
    /// These two tests read the typed plan since 2026-08-26 rather than grepping
    /// the message for `IS KEPT` and `AND ITS DATA`. That is `ADR-006` §6.5's
    /// point restated as a test: the difference between keeping an
    /// application's data and destroying it is the difference between a second
    /// `remove` effect and an `untouched` entry, and an agent that had to tell
    /// those apart by matching capital letters in prose was doing string
    /// matching on the safety mechanism (`PLAN.md` §22.3).
    #[test]
    fn without_the_confirmation_it_plans_and_removes_nothing() {
        let refusal = run(&[SUBJECT, "photo-index"]).expect_err("a plan");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        let plan = refusal.plan().expect("a confirmation refusal carries its plan");

        let removals: Vec<&str> = plan
            .effects()
            .iter()
            .filter(|e| e.act == crate::operation::Act::Remove)
            .map(|e| e.target.as_str())
            .collect();
        assert_eq!(
            removals,
            ["/state/app/photo-index"],
            "a plain uninstall removes the tree and nothing else"
        );
        assert!(
            plan.untouched()
                .iter()
                .any(|line| line.contains("/state/app/photo-index/data")),
            "the data it keeps is not named: {:?}",
            plan.untouched()
        );
    }

    /// A plan for a purge says what a purge does — **as a second removal**,
    /// which is the one irreversible act in this catalogue.
    #[test]
    fn a_purge_plan_says_the_data_goes_too() {
        let refusal = run(&[SUBJECT, "photo-index", PURGE]).expect_err("a plan");
        let plan = refusal.plan().expect("a confirmation refusal carries its plan");

        let removals: Vec<&str> = plan
            .effects()
            .iter()
            .filter(|e| e.act == crate::operation::Act::Remove)
            .map(|e| e.target.as_str())
            .collect();
        assert_eq!(
            removals,
            ["/state/app/photo-index", "/state/app/photo-index/data"],
            "a purge removes the data too, and says so as an effect"
        );
        assert!(
            !plan
                .untouched()
                .iter()
                .any(|line| line.contains("/state/app/photo-index/data")),
            "a purge must not also claim to keep the data: {:?}",
            plan.untouched()
        );
        // The word that caused it is in the command that repeats it.
        assert_eq!(
            plan.confirm_with(),
            format!("rivet apply {SUBJECT} photo-index {PURGE} {CONFIRMATION}")
        );
    }

    /// **A misspelling is refused, never read as absence.** Both halves matter:
    /// `purgee` would silently keep data the operator meant to remove, and
    /// `confrim` would silently remove nothing while looking like it worked.
    #[test]
    fn a_misspelled_keyword_is_refused_rather_than_ignored() {
        for wrong in ["purgee", "confrim", "--purge", "yes"] {
            let refusal = run(&[SUBJECT, "photo-index", wrong]).expect_err(wrong);
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{wrong}");
        }
    }

    /// A name that could escape `/state/app` never reaches the machine.
    #[test]
    fn a_name_that_is_not_one_is_refused_before_the_machine_is_asked() {
        for hostile in ["..", "a/b", "/etc", ".hidden"] {
            let refusal = run(&[SUBJECT, hostile, CONFIRMATION]).expect_err(hostile);
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{hostile}");
        }
    }

    /// The keywords cannot be the name, so a forgotten name is a refusal rather
    /// than a request to remove an application called `confirm`.
    #[test]
    fn a_keyword_cannot_be_the_application_name() {
        for word in [CONFIRMATION, PURGE] {
            let refusal = run(&[SUBJECT, word]).expect_err(word);
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{word}");
        }
    }

    /// Every declared output is produced, and nothing else is.
    #[test]
    fn the_declared_shape_is_the_produced_shape() {
        let got = run(&[SUBJECT, "photo-index", CONFIRMATION]).expect("delivered");
        let Value::Object(fields) = &got else {
            panic!("not an object")
        };
        let produced: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
        for output in OUTPUTS {
            assert!(
                produced.contains(&output.name),
                "{} is declared and absent",
                output.name
            );
        }
        for key in &produced {
            assert!(
                OUTPUTS.iter().any(|o| o.name == *key),
                "{key} is produced and undeclared"
            );
        }
    }
}
