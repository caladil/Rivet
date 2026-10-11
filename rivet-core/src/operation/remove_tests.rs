//! `apply remove-file` and `apply remove-directory`'s tests.
//!
//! **The one thing every test here has in common**: when a refusal is asserted,
//! the fake is asserted to have removed nothing. A refusal that had already
//! destroyed something would be the worst defect this operation can have, and
//! asserting the message without asserting the fake would not catch it.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};

const PAYLOAD: &str = "/state/rivet-0.1.8-aarch64-rpi4-update.rvt";

fn file(fake: &mut FakeFiles, words: &[&str]) -> Result<Removed, OperationError> {
    perform(Kind::File, words, fake)
}

fn directory(fake: &mut FakeFiles, words: &[&str]) -> Result<Removed, OperationError> {
    perform(Kind::Directory, words, fake)
}

fn refusal(result: Result<Removed, OperationError>) -> OperationError {
    result.expect_err("that command must be refused")
}

// -- the case the ruling was for ---------------------------------------------

/// **`ADR-006` §8.10.1's driving case, end to end.** At `ADR-035` phase 5 there
/// is no way in but the control plane, and a stale payload has to be removable
/// by the machine's own surface.
#[test]
fn a_stale_payload_is_planned_with_its_size_and_then_removed() {
    let mut fake = FakeFiles::new().with(PAYLOAD, Node::file("x".repeat(4096)));

    // The plan first, because that is the safety property.
    let planned = refusal(file(&mut fake, &[FILE_SUBJECT, PAYLOAD]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a confirmation refusal carries a plan");
    assert!(plan.narrative().contains("4096 bytes"), "{}", plan.narrative());
    assert!(plan.narrative().contains("CANNOT BE UNDONE"), "{}", plan.narrative());
    assert_eq!(
        plan.confirm_with(),
        format!("rivet apply {FILE_SUBJECT} {PAYLOAD} confirm")
    );
    assert!(fake.has(PAYLOAD), "the plan removed the file");

    let done = file(&mut fake, &[FILE_SUBJECT, PAYLOAD, CONFIRMATION])
        .expect("a stale payload must be removable");
    assert_eq!(done.path(), PAYLOAD);
    assert!(!fake.has(PAYLOAD));
    assert!(done.journalled());
    assert_eq!(fake.did, vec![format!("remove-file {PAYLOAD}")]);
}

/// **Without the literal word, the plan and nothing else.** `ADR-012` §7.1.
#[test]
fn a_misspelled_confirmation_is_refused_and_never_read_as_absence() {
    let mut fake = FakeFiles::new().with(PAYLOAD, Node::file("x"));
    for word in ["confrim", "yes", "y", "--confirm", "CONFIRM"] {
        let refused = refusal(file(&mut fake, &[FILE_SUBJECT, PAYLOAD, word]));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{word}");
        assert!(fake.has(PAYLOAD), "{word} removed the file");
    }
    assert!(fake.did.is_empty());
}

// -- what it refuses ---------------------------------------------------------

/// **Regular files only**, and each kind gets a sentence naming what to do
/// instead.
#[test]
fn everything_that_is_not_a_regular_file_is_refused_and_named() {
    let mut fake = FakeFiles::new()
        .with("/state/dir", Node::directory())
        .with("/state/link", Node::symlink("/state/dir"));

    let dir = refusal(file(&mut fake, &[FILE_SUBJECT, "/state/dir", CONFIRMATION]));
    assert_eq!(dir.code(), ErrorCode::Unsafe);
    assert!(dir.message().contains(DIRECTORY_SUBJECT), "{}", dir.message());

    let link = refusal(file(&mut fake, &[FILE_SUBJECT, "/state/link", CONFIRMATION]));
    assert_eq!(link.code(), ErrorCode::Unsafe);
    assert!(
        link.message().contains("NO SYMBOLIC LINK"),
        "{}",
        link.message()
    );
    assert!(link.message().contains("/state/dir"), "{}", link.message());

    assert!(fake.did.is_empty());
}

/// **Nothing there is a refusal and not a success.** A mistyped path must not
/// look like a success for ever.
#[test]
fn a_path_with_nothing_at_it_is_refused_rather_than_reported_as_removed() {
    let mut fake = FakeFiles::new();
    let refused = refusal(file(
        &mut fake,
        &[FILE_SUBJECT, "/state/typo", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("different facts"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

/// The gate, asserted through this subject rather than only through
/// `filesystem`'s own tests — because the thing that matters is that this
/// operation reaches it.
#[test]
fn the_gate_refuses_before_the_machine_is_asked_anything() {
    let mut fake = FakeFiles::new();
    for path in [
        "/system/bin/rivet-sysd",
        "/etc/passwd",
        "/state/../etc/passwd",
        "state/relative",
        "/state/config/machine.toml",
        "/state/deploy-request",
        "/state/home/root",
        "/state/app/thing",
    ] {
        let refused = refusal(file(&mut fake, &[FILE_SUBJECT, path, CONFIRMATION]));
        assert!(
            matches!(refused.code(), ErrorCode::Unsafe | ErrorCode::Arguments),
            "{path}: {:?}",
            refused.code()
        );
        assert!(fake.did.is_empty(), "{path} reached the machine");
        assert!(fake.asked.is_empty(), "{path} reached the machine");
    }
}

/// **A link on the way out of `/state` is the machine's refusal**, and it is
/// the half a test on a string cannot make. `ADR-006` §8.10.4.3.
#[test]
fn a_symbolic_link_in_the_path_is_refused_by_the_machine() {
    let mut fake = FakeFiles::new()
        .with("/state/scratch", Node::symlink("/system"))
        .with("/state/scratch/bin/x", Node::file("ELF"));
    let refused = refusal(file(
        &mut fake,
        &[FILE_SUBJECT, "/state/scratch/bin/x", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("symbolic link"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

#[test]
fn a_mount_point_is_refused_even_when_the_kind_is_right() {
    let mut fake = FakeFiles::new().with("/state/volume", Node::directory().mounted());
    let refused = refusal(directory(
        &mut fake,
        &[DIRECTORY_SUBJECT, "/state/volume", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("MOUNTED"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

// -- the directory half ------------------------------------------------------

#[test]
fn an_empty_directory_is_planned_as_empty_and_then_removed() {
    let mut fake = FakeFiles::new().with("/state/old", Node::directory());
    let planned = refusal(directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/old"]));
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("empty"), "{}", plan.narrative());

    directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/old", CONFIRMATION])
        .expect("an empty directory must be removable");
    assert!(!fake.has("/state/old"));
}

/// **There is no recursive removal in this build**, and the refusal says so and
/// says where the design for one lives.
#[test]
fn a_directory_that_is_not_empty_is_refused_and_names_what_is_in_it() {
    let mut fake = FakeFiles::new()
        .with("/state/full", Node::directory())
        .with("/state/full/a", Node::file("x"))
        .with("/state/full/b", Node::file("y"));
    let refused = refusal(directory(
        &mut fake,
        &[DIRECTORY_SUBJECT, "/state/full", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("NOT EMPTY"), "{}", refused.message());
    assert!(refused.message().contains("a, b"), "{}", refused.message());
    assert!(refused.message().contains("8.10.13"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

#[test]
fn each_subject_refuses_the_other_ones_kind() {
    let mut fake = FakeFiles::new()
        .with("/state/f", Node::file("x"))
        .with("/state/d", Node::directory());

    let a = refusal(directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/f", CONFIRMATION]));
    assert!(a.message().contains(FILE_SUBJECT), "{}", a.message());
    let b = refusal(file(&mut fake, &[FILE_SUBJECT, "/state/d", CONFIRMATION]));
    assert!(b.message().contains(DIRECTORY_SUBJECT), "{}", b.message());
    assert!(fake.did.is_empty());
}

// -- time of check to time of use --------------------------------------------

/// **The limit of the guard, asserted so nobody reads more into it.**
///
/// A plan does not survive between commands: this process ends when it answers,
/// so a `confirm` typed after the file changed removes what is there **then**.
/// `super::edit` is the one subject that does better, and only because its
/// `--expect` travels in the command.
///
/// This test exists because the first version of this module claimed the
/// stronger property in its plan text. **The claim was false and the test that
/// was written to prove it is what showed that** — kept, pointing the other way,
/// because a plan that over-promises is worse than one that does not promise.
#[test]
fn a_plan_does_not_survive_between_commands_and_the_plan_says_so() {
    let mut fake = FakeFiles::new().with("/state/log.txt", Node::file("small"));
    let planned = refusal(file(&mut fake, &[FILE_SUBJECT, "/state/log.txt"]));
    let plan = planned.plan().expect("a plan");
    assert!(
        plan.narrative().contains("DOES NOT GUARANTEE"),
        "{}",
        plan.narrative()
    );
    assert!(
        plan.narrative().contains("removes whatever is at"),
        "{}",
        plan.narrative()
    );

    // Something else on the machine rewrites it, and the confirmation removes
    // what is there NOW. That is the honest behaviour and it is what happens.
    fake.nodes.insert(
        "/state/log.txt".to_string(),
        Node::file("small and then a great deal more"),
    );
    let done = file(
        &mut fake,
        &[FILE_SUBJECT, "/state/log.txt", CONFIRMATION],
    )
    .expect("the confirmation acts on what is there now");
    assert!(!fake.has("/state/log.txt"));
    // And the record says the size that actually went, not the planned one.
    assert_eq!(done.bytes, 32);
}

/// **What the re-read DOES buy**: every refusal runs at the moment of the act
/// rather than being trusted from the planning phase. A directory that filled up
/// in between is refused, which is `AGENTS.md` §5's *asked at the point of use*.
#[test]
fn every_refusal_is_re_run_at_the_moment_of_the_act_and_not_trusted_from_the_plan() {
    let mut fake = FakeFiles::new().with("/state/box", Node::directory());
    let planned = refusal(directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/box"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);

    fake.nodes
        .insert("/state/box/new".to_string(), Node::file("appeared"));

    let refused = refusal(directory(
        &mut fake,
        &[DIRECTORY_SUBJECT, "/state/box", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("NOT EMPTY"), "{}", refused.message());
    assert!(fake.has("/state/box"));
}

// -- the second confirmation tier --------------------------------------------

/// **`ADR-012` §7.3**: a program is the machine's chance to object.
#[test]
fn removing_a_program_needs_the_acknowledgement_and_confirm_alone_will_not_do() {
    const PROGRAM: &str = "/state/app/thing/0.2.0/bin/thing";
    let mut fake = FakeFiles::new().with(PROGRAM, Node::file("ELF"));

    // The plan carries the warning in the maintainer's own rendering, and the
    // command it says to repeat carries the acknowledgement.
    let planned = refusal(file(&mut fake, &[FILE_SUBJECT, PROGRAM]));
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("<WARNING>"), "{}", plan.narrative());
    assert!(plan.narrative().contains("(N/yes)"), "{}", plan.narrative());
    assert!(
        plan.confirm_with().contains("--accept 'the application will not start"),
        "{}",
        plan.confirm_with()
    );

    // `confirm` alone is refused.
    let bare = refusal(file(&mut fake, &[FILE_SUBJECT, PROGRAM, CONFIRMATION]));
    assert_eq!(bare.code(), ErrorCode::Unsafe);
    assert!(bare.message().contains("<WARNING>"), "{}", bare.message());
    assert!(fake.has(PROGRAM));

    // A wrong acknowledgement is refused and shows both strings.
    let wrong = refusal(file(
        &mut fake,
        &[FILE_SUBJECT, PROGRAM, ACCEPT_FLAG, "yes really", CONFIRMATION],
    ));
    assert_eq!(wrong.code(), ErrorCode::Arguments);
    assert!(fake.has(PROGRAM));

    // The right one, which cannot be produced without having read the plan.
    file(
        &mut fake,
        &[
            FILE_SUBJECT,
            PROGRAM,
            ACCEPT_FLAG,
            "the application will not start until it is reinstalled",
            CONFIRMATION,
        ],
    )
    .expect("the acknowledgement must be accepted");
    assert!(!fake.has(PROGRAM));
}

/// **A superfluous acknowledgement is refused**, which is what stops a caller
/// appending it to everything and turning the tier back into the first one.
#[test]
fn an_acknowledgement_on_a_change_with_no_warning_is_refused_by_name() {
    let mut fake = FakeFiles::new().with(PAYLOAD, Node::file("x"));
    let refused = refusal(file(
        &mut fake,
        &[FILE_SUBJECT, PAYLOAD, ACCEPT_FLAG, "anything", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(refused.message().contains("NO WARNING"), "{}", refused.message());
    assert!(fake.has(PAYLOAD));
}

/// **The doctrine, asserted.** The maintainer, 2026-08-29 (`.rulings/` R-030):
/// the WARNING tier is reserved for the exceptional case. A stale
/// payload and a config file in a version directory are not exceptional, and a
/// tier that fired on them would become wallpaper (`PLAN.md` §94.2).
#[test]
fn the_routine_cases_do_not_reach_the_second_tier() {
    for path in [
        PAYLOAD,
        "/state/app/thing/0.2.0/config.toml",
        "/state/app/thing/data/store.db",
        "/state/scratch/notes",
    ] {
        let mut fake = FakeFiles::new().with(path, Node::file("x"));
        file(&mut fake, &[FILE_SUBJECT, path, CONFIRMATION])
            .unwrap_or_else(|e| panic!("{path} needed more than `confirm`: {}", e.message()));
        assert!(!fake.has(path), "{path}");
    }
}

// -- arguments ---------------------------------------------------------------

#[test]
fn a_word_this_command_has_no_meaning_for_is_refused_by_name() {
    let mut fake = FakeFiles::new().with(PAYLOAD, Node::file("x"));
    for words in [
        vec![FILE_SUBJECT],
        vec![FILE_SUBJECT, PAYLOAD, "--force", CONFIRMATION],
        vec![FILE_SUBJECT, PAYLOAD, "please", CONFIRMATION],
        vec![FILE_SUBJECT, PAYLOAD, ACCEPT_FLAG],
    ] {
        let refused = refusal(file(&mut fake, &words));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{words:?}");
        assert!(fake.has(PAYLOAD), "{words:?}");
    }
}

/// The catalogue naming a field no result carries is `ADR-006` §7's *"trusted
/// and wrong"*; the `debug_assert` in `value` holds it and this exercises both
/// shapes.
#[test]
fn both_result_shapes_keep_the_declared_field_order() {
    let mut fake = FakeFiles::new()
        .with("/state/f", Node::file("x"))
        .with("/state/d", Node::directory());
    let a = file(&mut fake, &[FILE_SUBJECT, "/state/f", CONFIRMATION]).expect("a file");
    let b = directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/d", CONFIRMATION])
        .expect("a directory");
    for value in [a.value(), b.value()] {
        let Value::Object(fields) = &value else {
            panic!("not an object");
        };
        let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
        let mut at = 0usize;
        for (key, _) in fields {
            let found = declared
                .iter()
                .skip(at)
                .position(|d| d == key)
                .unwrap_or_else(|| panic!("`{key}` is out of order or undeclared"));
            at += found + 1;
        }
    }
}
