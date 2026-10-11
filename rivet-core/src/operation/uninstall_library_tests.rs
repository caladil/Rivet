//! `apply uninstall-library`'s words, its plan and its answer, against a board
//! that answers on this operation's two methods and refuses on every other.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::Act;
use crate::package::library::Reference;
use crate::uninstall_library::{Acceptor, Refusal, Selector, Unread};

/// **A board that answers on this operation's two methods and refuses on every
/// other** (`AGENTS.md` §3), with the removal it reports scripted by the test.
struct FakeBoard {
    /// What the machine finds, or the refusal it gives.
    answers: Result<Removal, MachineError>,
    /// Every selector the plan asked about.
    planned: Vec<Selector>,
    /// Every selector a request was delivered for.
    asked: Vec<Selector>,
}

impl FakeBoard {
    fn finding(removal: Removal) -> Self {
        Self {
            answers: Ok(removal),
            planned: Vec::new(),
            asked: Vec::new(),
        }
    }

    fn refusing(error: MachineError) -> Self {
        Self {
            answers: Err(error),
            planned: Vec::new(),
            asked: Vec::new(),
        }
    }
}

fn nope() -> MachineError {
    MachineError::Refused("this fake answers only the library-uninstall methods".to_string())
}

impl crate::operation::machine::Deployer for FakeBoard {
    fn can_request_install(
        &mut self,
        _digest: &crate::artifact::Digest,
    ) -> Result<crate::operation::machine::Held, MachineError> {
        Err(nope())
    }
    fn request_install(
        &mut self,
        _digest: &crate::artifact::Digest,
    ) -> Result<crate::operation::machine::Held, MachineError> {
        Err(nope())
    }
    fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
        Err(nope())
    }
    fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
        Err(nope())
    }
    fn can_request_repo_delete(&mut self, _name: &str) -> Result<(), MachineError> {
        Err(nope())
    }
    fn request_repo_delete(&mut self, _name: &str, _id: &str) -> Result<(), MachineError> {
        Err(nope())
    }
    fn can_request_library_uninstall(&mut self, selector: &Selector) -> Result<Removal, MachineError> {
        self.planned.push(selector.clone());
        self.answers.clone()
    }
    fn request_library_uninstall(&mut self, selector: &Selector) -> Result<Removal, MachineError> {
        self.asked.push(selector.clone());
        self.answers.clone()
    }
}

crate::operation::machine::refuses_to_edit!(FakeBoard);
crate::operation::machine::refuses_to_observe!(FakeBoard);
crate::operation::machine::refuses_to_manage!(FakeBoard);

fn build(product: &str, version: &str, byte: u8) -> Reference {
    Reference::compose(product, version, &[byte; 32]).expect("a reference")
}

/// Two builds of one addon product, one application joining both by class.
fn two_builds_one_acceptor() -> (Reference, Reference, Removal) {
    let a = build("aura-background-pack", "0.1.0", 0x11);
    let b = build("aura-background-pack", "0.2.0", 0x22);
    let removal = Removal {
        builds: vec![a.clone(), b.clone()],
        acceptors: vec![
            Acceptor {
                application: "aura-renderer".to_string(),
                class: "aura-background-pack".to_string(),
                reference: a.clone(),
            },
            Acceptor {
                application: "aura-renderer".to_string(),
                class: "aura-background-pack".to_string(),
                reference: b.clone(),
            },
        ],
        unread: Vec::new(),
    };
    (a, b, removal)
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

/// **What reached the machine is what was typed**, and the answer carries the
/// machine's account back.
#[test]
fn a_confirmed_request_carries_the_selector_typed_and_the_machines_account() {
    let (a, _, removal) = two_builds_one_acceptor();
    let mut board = FakeBoard::finding(removal);
    let got = perform(&[SUBJECT, "aura-background-pack", CONFIRMATION], &mut board)
        .expect("delivered");

    assert_eq!(
        board.asked,
        vec![Selector::Product("aura-background-pack".to_string())]
    );
    assert_eq!(field(&got, "requested"), Value::Bool(true));
    assert_eq!(field(&got, "removed"), Value::Bool(false));
    let Value::List(builds) = field(&got, "builds") else {
        panic!("builds is not a list")
    };
    assert!(builds.contains(&Value::str(a.as_str())), "{builds:?}");
    let Value::List(accepted) = field(&got, "accepted-by") else {
        panic!("accepted-by is not a list")
    };
    assert_eq!(accepted.len(), 2, "{accepted:?}");
    assert!(
        matches!(&accepted[0], Value::Str(s) if s.starts_with("aura-renderer accepts aura-background-pack from ")),
        "{accepted:?}"
    );
}

/// **The plan names every build as a removal and every acceptor beside it**,
/// and the command that confirms it is the one that was typed.
#[test]
fn the_plan_names_each_build_and_each_application_that_loses_it() {
    let (a, b, removal) = two_builds_one_acceptor();
    let mut board = FakeBoard::finding(removal);
    let refusal = perform(&[SUBJECT, "aura-background-pack"], &mut board).expect_err("a plan");
    assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
    let plan = refusal.plan().expect("a confirmation refusal carries its plan");

    let removals: Vec<&crate::operation::Effect> = plan
        .effects()
        .iter()
        .filter(|e| e.act == Act::Remove)
        .collect();
    assert_eq!(
        removals.iter().map(|e| e.target.as_str()).collect::<Vec<_>>(),
        [
            library_dir(BUILT_IN_VOLUME, &a).as_str(),
            library_dir(BUILT_IN_VOLUME, &b).as_str()
        ],
    );
    for effect in &removals {
        let note = effect.note.as_deref().expect("an acceptor is noted");
        assert!(note.contains("`aura-renderer`"), "{note}");
        assert!(note.contains("LOSES"), "{note}");
    }
    assert!(
        plan.effects().iter().any(|e| e.act == Act::Request),
        "the supervisor is asked, and the plan says so"
    );
    assert!(
        plan.untouched()
            .iter()
            .any(|line| line.contains("NOTHING IS RESTARTED")),
        "{:?}",
        plan.untouched()
    );
    assert_eq!(
        plan.confirm_with(),
        format!("rivet apply {SUBJECT} aura-background-pack {CONFIRMATION}")
    );
    // A plan asks the machine and delivers nothing.
    assert_eq!(board.planned.len(), 1);
    assert!(board.asked.is_empty(), "{:?}", board.asked);
}

/// A build whose class cannot be read says so in the plan rather than being
/// silently short of acceptors.
#[test]
fn the_plan_says_when_a_builds_acceptors_are_unknown() {
    let a = build("libgreet", "1.0.0", 0x11);
    let mut board = FakeBoard::finding(Removal {
        builds: vec![a.clone()],
        acceptors: Vec::new(),
        unread: vec![Unread {
            reference: a,
            detail: "no manifest".to_string(),
        }],
    });
    let refusal = perform(&[SUBJECT, "libgreet"], &mut board).expect_err("a plan");
    let plan = refusal.plan().expect("a plan");
    let note = plan.effects()[0].note.as_deref().expect("noted");
    assert!(note.contains("UNKNOWN"), "{note}");
}

/// **The machine's refusal passes through with its code**, unreworded: a
/// declared closure is `E_UNSAFE`, which an agent must stop at rather than
/// retry.
#[test]
fn a_declared_closure_from_the_machine_is_an_unsafe_refusal_on_both_paths() {
    let a = build("libgreet", "1.0.0", 0x11);
    let why = Refusal::Declared {
        holders: vec![crate::uninstall_library::Holder {
            application: "greeter".to_string(),
            reference: a,
        }],
    };
    for words in [
        vec![SUBJECT, "libgreet"],
        vec![SUBJECT, "libgreet", CONFIRMATION],
    ] {
        let mut board = FakeBoard::refusing(why.machine_error());
        let refusal = perform(&words, &mut board).expect_err("refused");
        assert_eq!(refusal.code(), ErrorCode::Unsafe, "{words:?}");
        assert_eq!(refusal.message(), why.why(), "{words:?}");
        assert!(board.asked.is_empty(), "a refused plan delivered a request");
    }
}

/// **A misspelling is refused, never read as absence**, and so is a word this
/// command does not take.
#[test]
fn a_misspelled_or_extra_word_is_refused_rather_than_ignored() {
    for wrong in ["confrim", "--confirm", "yes", "purge"] {
        let mut board = FakeBoard::finding(Removal {
            builds: Vec::new(),
            acceptors: Vec::new(),
            unread: Vec::new(),
        });
        let refusal = perform(&[SUBJECT, "libgreet", wrong], &mut board).expect_err(wrong);
        assert_eq!(refusal.code(), ErrorCode::Arguments, "{wrong}");
        assert!(board.planned.is_empty(), "{wrong} reached the machine");
    }
}

/// A selector that could escape `/state/lib` never reaches the machine, and
/// neither does a forgotten one.
#[test]
fn a_selector_that_is_not_one_is_refused_before_the_machine_is_asked() {
    for words in [
        vec![SUBJECT],
        vec![SUBJECT, CONFIRMATION],
        vec![SUBJECT, "..", CONFIRMATION],
        vec![SUBJECT, "a/b", CONFIRMATION],
        vec![SUBJECT, ".hidden"],
        vec![SUBJECT, "-x"],
    ] {
        let mut board = FakeBoard::refusing(nope());
        let refusal = perform(&words, &mut board).expect_err("refused");
        assert_eq!(refusal.code(), ErrorCode::Arguments, "{words:?}");
        assert!(board.planned.is_empty(), "{words:?} reached the machine");
    }
}

/// Every declared output is produced, and nothing else is.
#[test]
fn the_declared_shape_is_the_produced_shape() {
    let (_, _, removal) = two_builds_one_acceptor();
    let mut board = FakeBoard::finding(removal);
    let got = perform(&[SUBJECT, "aura-background-pack", CONFIRMATION], &mut board)
        .expect("delivered");
    let Value::Object(fields) = &got else {
        panic!("not an object")
    };
    let produced: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
    for output in OUTPUTS {
        assert!(produced.contains(&output.name), "{} is declared and absent", output.name);
    }
    for key in &produced {
        assert!(
            OUTPUTS.iter().any(|o| o.name == *key),
            "{key} is produced and undeclared"
        );
    }
}
