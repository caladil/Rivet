//! `apply create-file` and `apply create-directory`'s tests.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};

fn make_file(fake: &mut FakeFiles, words: &[&str]) -> Result<Created, OperationError> {
    perform(Kind::File, words, fake)
}

fn make_directory(fake: &mut FakeFiles, words: &[&str]) -> Result<Created, OperationError> {
    perform(Kind::Directory, words, fake)
}

fn refusal(result: Result<Created, OperationError>) -> OperationError {
    result.expect_err("that command must be refused")
}

// -- the gap this closes -----------------------------------------------------

/// **`apply edit`'s own words**: *"If the intent was to CREATE an empty file,
/// this build has no operation for that."* This is that operation.
#[test]
fn a_file_can_be_created_with_no_text_at_all_and_the_plan_says_it_would_be_empty() {
    let mut fake = FakeFiles::new();

    let planned = refusal(make_file(&mut fake, &[FILE_SUBJECT, "/state/marker"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("WOULD BE EMPTY"), "{}", plan.narrative());
    assert!(plan.narrative().contains("0600"), "{}", plan.narrative());
    assert!(!fake.has("/state/marker"));

    let done = make_file(&mut fake, &[FILE_SUBJECT, "/state/marker", CONFIRMATION])
        .expect("an empty file must be creatable");
    assert_eq!(done.path(), "/state/marker");
    assert!(fake.has("/state/marker"));
    assert_eq!(fake.nodes["/state/marker"].contents, b"");
    assert!(done.journalled());
}

/// With text, the plan shows the whole content as a diff against nothing —
/// `ADR-006` §8.10.7's placement of a diff where it does the most work.
#[test]
fn a_file_created_with_text_shows_that_text_as_a_diff_in_its_plan() {
    let mut fake = FakeFiles::new();
    let planned = refusal(make_file(
        &mut fake,
        &[FILE_SUBJECT, "/state/new.toml", "a = 1\nb = 2\n"],
    ));
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("+a = 1"), "{}", plan.narrative());
    assert!(plan.narrative().contains("+b = 2"), "{}", plan.narrative());

    make_file(
        &mut fake,
        &[FILE_SUBJECT, "/state/new.toml", "a = 1\nb = 2\n", CONFIRMATION],
    )
    .expect("a file with text must be creatable");
    assert!(fake.has("/state/new.toml"));
}

#[test]
fn a_directory_is_created_at_0700_and_the_plan_says_it_would_be_empty() {
    let mut fake = FakeFiles::new();
    let planned = refusal(make_directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/box"]));
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("0700"), "{}", plan.narrative());

    make_directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/box", CONFIRMATION])
        .expect("a directory must be creatable");
    assert!(fake.has("/state/box"));
}

// -- creating is not clobbering ----------------------------------------------

/// **The one refusal that matters.** A `create` that overwrote would be the
/// blind clobber `ADR-006` §8.9.2 exists to prevent, reached by a different
/// word.
#[test]
fn anything_already_at_the_path_is_refused_and_the_refusal_names_the_right_verb() {
    let mut fake = FakeFiles::new()
        .with("/state/there", Node::file("existing"))
        .with("/state/dir", Node::directory());

    let over_file = refusal(make_file(
        &mut fake,
        &[FILE_SUBJECT, "/state/there", "new", CONFIRMATION],
    ));
    assert_eq!(over_file.code(), ErrorCode::Unsafe);
    assert!(over_file.message().contains("--expect"), "{}", over_file.message());
    assert_eq!(fake.nodes["/state/there"].contents, b"existing");

    let over_dir = refusal(make_directory(
        &mut fake,
        &[DIRECTORY_SUBJECT, "/state/dir", CONFIRMATION],
    ));
    assert_eq!(over_dir.code(), ErrorCode::Unsafe);
    assert!(
        over_dir.message().contains("ALREADY A DIRECTORY"),
        "{}",
        over_dir.message()
    );
    assert!(fake.did.is_empty());
}

/// **The guard against a stale plan is the whole safety story of a creation**:
/// without it, a file that appeared in between would be replaced by one nobody
/// described.
#[test]
fn a_path_that_became_occupied_between_the_plan_and_the_confirmation_is_refused() {
    let mut fake = FakeFiles::new();
    let planned = refusal(make_file(&mut fake, &[FILE_SUBJECT, "/state/race", "mine"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);

    fake.nodes
        .insert("/state/race".to_string(), Node::file("somebody else's"));

    let refused = refusal(make_file(
        &mut fake,
        &[FILE_SUBJECT, "/state/race", "mine", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert_eq!(fake.nodes["/state/race"].contents, b"somebody else's");
}

// -- refusals ----------------------------------------------------------------

#[test]
fn the_gate_refuses_before_the_machine_is_asked_anything() {
    let mut fake = FakeFiles::new();
    for path in [
        "/system/new",
        "/etc/new",
        "/state/../etc/new",
        "relative",
        "/state/config/other.toml",
        "/state/app/newthing",
    ] {
        let refused = refusal(make_file(&mut fake, &[FILE_SUBJECT, path, CONFIRMATION]));
        assert!(
            matches!(refused.code(), ErrorCode::Unsafe | ErrorCode::Arguments),
            "{path}: {:?}",
            refused.code()
        );
        assert!(fake.did.is_empty(), "{path} reached the machine");
    }
}

/// **`confirm` where the text should be is a forgotten value, not a literal** —
/// `super::edit`'s rule and `super::install`'s, applied to the one positional
/// this family has.
#[test]
fn the_confirmation_word_is_never_read_as_the_text_of_a_new_file() {
    let mut fake = FakeFiles::new();
    make_file(&mut fake, &[FILE_SUBJECT, "/state/f", CONFIRMATION])
        .expect("this creates an EMPTY file, not one holding the word `confirm`");
    assert_eq!(fake.nodes["/state/f"].contents, b"");
}

#[test]
fn text_larger_than_the_cap_is_refused_with_the_number() {
    let mut fake = FakeFiles::new();
    let long = "y".repeat(super::super::edit::TEXT_MAX_BYTES + 1);
    let refused = refusal(make_file(
        &mut fake,
        &[FILE_SUBJECT, "/state/big", &long, CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(fake.did.is_empty());
}

// -- the application tree ----------------------------------------------------

/// Creating a program is exceptional; creating a config file beside it is not.
/// `ADR-012` §7.3's doctrine.
#[test]
fn creating_a_program_needs_the_acknowledgement_and_a_config_file_does_not() {
    let mut fake = FakeFiles::new();
    let bare = refusal(make_file(
        &mut fake,
        &[
            FILE_SUBJECT,
            "/state/app/thing/0.2.0/bin/thing",
            "ELF",
            CONFIRMATION,
        ],
    ));
    assert_eq!(bare.code(), ErrorCode::Unsafe);
    assert!(bare.message().contains("<WARNING>"), "{}", bare.message());

    // And the plan of the ordinary case says what nothing else would tell them.
    let planned = refusal(make_file(
        &mut fake,
        &[FILE_SUBJECT, "/state/app/thing/0.2.0/config.toml", "a = 1\n"],
    ));
    let plan = planned.plan().expect("a plan");
    assert!(
        plan.narrative().contains("NEXT UPDATE DISCARDS IT"),
        "{}",
        plan.narrative()
    );
    make_file(
        &mut fake,
        &[
            FILE_SUBJECT,
            "/state/app/thing/0.2.0/config.toml",
            "a = 1\n",
            CONFIRMATION,
        ],
    )
    .expect("a config file beside a program is the routine case");
}

#[test]
fn both_result_shapes_keep_the_declared_field_order() {
    let mut fake = FakeFiles::new();
    let a = make_file(&mut fake, &[FILE_SUBJECT, "/state/f", "x", CONFIRMATION]).expect("a file");
    let b = make_directory(&mut fake, &[DIRECTORY_SUBJECT, "/state/d", CONFIRMATION])
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
