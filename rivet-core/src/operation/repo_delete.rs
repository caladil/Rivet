//! `rivet apply repo-delete <name> <id> confirm` — **destroy a repository.**
//!
//! Ruled 2026-08-26. `crate::repo_delete` carries the mechanism and the two
//! interlocks; this module is the operation.
//!
//! # It delivers a request and returns
//!
//! `uninstall`'s shape, for a reason of its own: **the supervisor owns the
//! repository root.** `crate::repo`'s projection converges that tree on every
//! boot and every reload, and a second process removing a directory underneath
//! it would be two authorities over one tree.
//!
//! So `requested: true` means *the request was written*. `rivet get log
//! rivet-sysd` is where the removal reports — including the two refusals this
//! command cannot make on the operator's behalf, because only the machine can
//! see its own disk:
//!
//! * the repository is **still declared**, so the next convergence would
//!   recreate it;
//! * the directory carries a **different identifier** from the one asked for.

use super::{ErrorCode, Field, OperationError, Verb};
use crate::operation::machine::{Board, MachineError};
use crate::operation::plan::{Change, Effect, Plan, TakesEffect, resolve};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "repo-delete";

/// The literal `ADR-012` §7.1 requires as the last word.
const CONFIRMATION: &str = "confirm";

/// What this subject's result carries.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `repo-delete` in this build.",
    },
    Field {
        name: "repository",
        description: "The repository named, carried back verbatim.",
    },
    Field {
        name: "id",
        description: "The identifier the operator expects to destroy. THE MACHINE COMPARES IT \
                      to the one in the directory and refuses if they differ — a name is a \
                      label and labels get reused (ADR-025 §4.1).",
    },
    Field {
        name: "requested",
        description: "Always true in a result. It means the request was DELIVERED, not that \
                      anything has been removed (ADR-006 §4.2).",
    },
    Field {
        name: "removed",
        description: "Always false. NOTHING IS GONE when this answers — and the supervisor may \
                      yet refuse, because two of the three checks can only be made by the \
                      machine that has the disk.",
    },
    Field {
        name: "next",
        description: "Where the outcome appears, and the two ways it can still be refused.",
    },
];

/// Everything an operator must be told, whether this is a plan or a result.
fn answer(name: &str, id: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("subject", Value::str(SUBJECT)),
        ("repository", Value::str(name.to_string())),
        ("id", Value::str(id.to_string())),
        ("requested", Value::Bool(true)),
        ("removed", Value::Bool(false)),
        (
            "next",
            Value::str(format!(
                "the request was DELIVERED and nothing has been removed yet. Watch it with \
                 `rivet get log` and look at the records whose `svc` is `rivet-sysd` — that  \
                 operation has no service filter. THE SUPERVISOR CAN STILL REFUSE, in two ways this \
                 command cannot check because only the machine can see its own disk: if the \
                 repository is STILL DECLARED in `[repos]` the next convergence would recreate \
                 it, so undeclare it first (`rivet-fleet remove repo`); and if the directory \
                 carries a DIFFERENT identifier from `{id}` it is not the repository you meant \
                 and nothing is touched."
            )),
        ),
    ]
}

/// Perform it. `arguments[0]` is the subject.
pub(super) fn perform(arguments: &[&str], board: &mut dyn Board) -> Result<Value, OperationError> {
    let Some(name) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the repository to destroy and the identifier it \
                 should have — `rivet {apply} {SUBJECT} <name> <id> {CONFIRMATION}`. \
                 `rivet-fleet list repos` prints both.",
                apply = Verb::Apply,
            ),
        ));
    };

    // A word that is only the confirmation is a forgotten name — `uninstall`'s
    // precedent, and the same failure it prevents.
    if name == CONFIRMATION {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            name,
            format!(
                "`{name}` is a keyword of this command and cannot be the repository's name, so \
                 this names nothing to destroy. The name goes first: `rivet {apply} {SUBJECT} \
                 <name> <id> {CONFIRMATION}`.",
                apply = Verb::Apply,
            ),
        ));
    }

    // **Checked before the machine is touched**, so a plan and a confirmed run
    // refuse an impossible name identically — and so nothing that could
    // traverse reaches a path.
    if crate::repo::RepoName::new(name).is_none() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            name,
            format!(
                "`{name}` is not a usable repository name, so no directory under {root} could \
                 correspond to it; it must be {rule}",
                root = crate::git::REPOSITORY_ROOT,
                rule = crate::repo::RepoName::RULE,
            ),
        ));
    }

    let Some(id) = arguments.get(2).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT} {name}` names a repository and not WHICH repository. The \
                 identifier goes after the name — `rivet {apply} {SUBJECT} {name} <id> \
                 {CONFIRMATION}` — and the machine refuses if the directory carries a \
                 different one. **A name is a label and labels get reused**: undeclare \
                 `{name}`, create a new `{name}`, run a command you typed ten minutes ago, and \
                 without this you would destroy the wrong one. `rivet-fleet list repos` prints \
                 the identifier.",
                apply = Verb::Apply,
            ),
        ));
    };
    if id == CONFIRMATION {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            id,
            format!(
                "the identifier is missing: `{CONFIRMATION}` is where it should be. \
                 `rivet {apply} {SUBJECT} {name} <id> {CONFIRMATION}`.",
                apply = Verb::Apply,
            ),
        ));
    }
    if !crate::identifier::is_identifier(id) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            id,
            format!(
                "`{id}` is not an identifier; {}. `rivet-fleet list repos` prints the one this \
                 repository has.",
                crate::identifier::RULE
            ),
        ));
    }

    let mut confirmed = false;
    for word in arguments.iter().skip(3).copied() {
        if word == CONFIRMATION {
            confirmed = true;
        } else {
            return Err(OperationError::about(
                ErrorCode::Arguments,
                word,
                format!(
                    "`{word}` is not a word this command takes. After the name and the \
                     identifier comes `{CONFIRMATION}`, and nothing else — A MISSPELLING OF IT \
                     IS REFUSED rather than read as absent (ADR-012 §7.1), because what it \
                     would hide is destroying a repository nobody confirmed."
                ),
            ));
        }
    }

    resolve(RequestRepoDelete { name, id }, confirmed, board)
}

/// **The change `apply repo-delete` is**, as `ADR-006` §6.5 defines one.
struct RequestRepoDelete<'a> {
    name: &'a str,
    id: &'a str,
}

impl Change for RequestRepoDelete<'_> {
    type Prepared = ();
    type Applied = Value;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, ()), OperationError> {
        board.can_request_repo_delete(self.name).map_err(refused)?;

        let dir = crate::git::repository_dir(self.name);
        let plan = Plan::new(
            format!("{SUBJECT} {} {}", self.name, self.id),
            "destroys a repository and everything in it",
        )
        .doing(
            Effect::remove(dir.clone()).was(format!(
                "the repository `{}` and EVERY COMMIT IN IT. There is no copy on this machine \
                 and nothing keeps a backup — if the only clone is here, this is the end of it",
                self.name
            )),
        )
        .doing(
            Effect::request(
                // The supervisor rather than a path: it owns this tree, and it
                // is the only thing that can see the two conditions below.
                "the supervisor",
                format!(
                    "check that `{}` is no longer declared and that the directory carries \
                     `{}`, then remove it",
                    self.name, self.id
                ),
            )
            .noting(
                "BOTH CHECKS ARE THE MACHINE'S and neither can be made from here: this process \
                 cannot see the machine's document or its disk. A still-declared repository is \
                 refused, because the next convergence would recreate it and the delete would \
                 look like it had failed.",
            ),
        )
        .leaving("NOTHING HAS BEEN REMOVED — the request is delivered only if this is confirmed")
        .taking_effect(TakesEffect::Immediately)
        .confirmed_by(&[
            SUBJECT.to_string(),
            self.name.to_string(),
            self.id.to_string(),
        ]);
        Ok((plan, ()))
    }

    fn apply(self, (): (), board: &mut dyn Board) -> Result<Value, OperationError> {
        board.request_repo_delete(self.name, self.id).map_err(refused)?;
        Ok(Value::object(answer(self.name, self.id)))
    }
}

/// The one place a [`MachineError`] becomes this operation's refusal.
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

    const ID: &str = "4f2c9a01bd7e3865";
    const OTHER: &str = "a1b2c3d4e5f60718";

    /// **A board that answers on this operation's two methods and refuses on
    /// every other**, so a test here that started depending on something else
    /// fails loudly rather than passing on a fiction (`AGENTS.md` §3).
    #[derive(Default)]
    struct FakeBoard {
        asked: Vec<(String, String)>,
    }

    fn nope() -> MachineError {
        MachineError::Refused("this fake answers only the repo-delete methods".to_string())
    }

    impl Machine for FakeBoard {
        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            Err(nope())
        }
        fn request_reload(&mut self) -> Result<(), MachineError> {
            Err(nope())
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
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(nope())
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
        fn can_request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(nope())
        }
        fn request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(nope())
        }
        fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
            Err(nope())
        }
        fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
            Err(nope())
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
        fn can_request_repo_delete(&mut self, _name: &str) -> Result<(), MachineError> {
            Ok(())
        }
        fn request_repo_delete(&mut self, name: &str, id: &str) -> Result<(), MachineError> {
            self.asked.push((name.to_string(), id.to_string()));
            Ok(())
        }
    }

    crate::operation::machine::refuses_to_edit!(FakeBoard);

    /// **What reached the machine is what was typed**, identifier included.
    #[test]
    fn the_request_that_reaches_the_machine_carries_both_words() {
        let mut board = FakeBoard::default();
        perform(&[SUBJECT, "example-repo", ID, CONFIRMATION], &mut board).expect("delivered");
        assert_eq!(board.asked, vec![("example-repo".to_string(), ID.to_string())]);
    }

    /// A plan asks the machine nothing.
    #[test]
    fn a_plan_delivers_no_request_at_all() {
        let mut board = FakeBoard::default();
        let _ = perform(&[SUBJECT, "example-repo", ID], &mut board);
        assert!(board.asked.is_empty(), "{:?}", board.asked);
    }

    /// **The identifier is not optional, and the refusal says why.** This is
    /// the interlock that stops a stale command destroying a reused name.
    #[test]
    fn a_command_that_names_a_repository_and_not_which_one_is_refused() {
        let mut board = FakeBoard::default();
        let e = perform(&[SUBJECT, "example-repo"], &mut board).expect_err("refused");
        assert_eq!(e.code(), ErrorCode::Arguments);
        assert!(e.message().contains("labels get reused"), "{}", e.message());
        assert!(board.asked.is_empty());
    }

    /// The confirmation in the identifier's place is a missing identifier, and
    /// is told so rather than being read as one.
    #[test]
    fn the_confirmation_in_the_identifiers_place_is_a_missing_identifier() {
        let mut board = FakeBoard::default();
        let e = perform(&[SUBJECT, "example-repo", CONFIRMATION], &mut board)
            .expect_err("refused");
        assert_eq!(e.code(), ErrorCode::Arguments);
        assert!(board.asked.is_empty());
    }

    /// A name that could traverse never reaches the board.
    #[test]
    fn a_name_that_could_traverse_is_refused_before_the_machine_is_touched() {
        let mut board = FakeBoard::default();
        for bad in ["../../system", "..", "a/b", "-rf", "Example"] {
            let e = perform(&[SUBJECT, bad, ID, CONFIRMATION], &mut board).expect_err(bad);
            assert_eq!(e.code(), ErrorCode::Arguments, "{bad}");
        }
        assert!(board.asked.is_empty(), "the board was touched: {:?}", board.asked);
    }

    /// **A misspelt confirmation is refused, never read as absent** — what it
    /// would hide is destroying a repository nobody confirmed.
    #[test]
    fn a_misspelt_confirmation_is_refused_rather_than_read_as_absent() {
        let mut board = FakeBoard::default();
        let e = perform(&[SUBJECT, "example-repo", ID, "confrim"], &mut board)
            .expect_err("refused");
        assert_eq!(e.code(), ErrorCode::Arguments);
        assert!(board.asked.is_empty());
    }

    /// An identifier that is not one is refused before the machine is touched.
    #[test]
    fn an_identifier_that_is_not_one_is_refused() {
        let mut board = FakeBoard::default();
        for bad in ["4f2c", "NOTHEX0000000000", "example-repo"] {
            assert!(
                perform(&[SUBJECT, "example-repo", bad, CONFIRMATION], &mut board).is_err(),
                "{bad}"
            );
        }
        assert!(board.asked.is_empty());
    }

    /// **The answer never claims anything is gone.** Two of the three checks
    /// belong to the machine, and the result says which.
    #[test]
    fn the_answer_says_nothing_has_been_removed_and_who_can_still_refuse() {
        let mut board = FakeBoard::default();
        let value = perform(&[SUBJECT, "example-repo", ID, CONFIRMATION], &mut board)
            .expect("delivered");
        let rendered = format!("{value:?}");
        assert!(rendered.contains("example-repo"), "{rendered}");
        assert!(rendered.contains(ID), "{rendered}");
        assert!(!rendered.contains(OTHER));
        // `removed` is false, and the sentence says the supervisor can refuse.
        assert!(rendered.contains("STILL DECLARED"), "{rendered}");
        assert!(rendered.contains("DIFFERENT identifier"), "{rendered}");
    }
}
