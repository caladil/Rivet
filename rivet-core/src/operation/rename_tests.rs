//! `apply rename`'s tests.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};

fn rename(fake: &mut FakeFiles, words: &[&str]) -> Result<Renamed, OperationError> {
    perform(words, fake)
}

fn refusal(result: Result<Renamed, OperationError>) -> OperationError {
    result.expect_err("that command must be refused")
}

#[test]
fn a_file_is_planned_with_its_size_and_then_moved_and_nothing_is_read() {
    let mut fake = FakeFiles::holding("/state/a.txt", "hello");

    let planned = refusal(rename(&mut fake, &[SUBJECT, "/state/a.txt", "/state/b.txt"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("5 bytes"), "{}", plan.narrative());
    assert!(plan.narrative().contains("not a copy"), "{}", plan.narrative());
    assert!(fake.has("/state/a.txt"));

    let done = rename(
        &mut fake,
        &[SUBJECT, "/state/a.txt", "/state/b.txt", CONFIRMATION],
    )
    .expect("a move must be possible");
    assert_eq!(done.to(), "/state/b.txt");
    assert!(!fake.has("/state/a.txt"));
    assert_eq!(fake.nodes["/state/b.txt"].contents, b"hello");
}

/// **The refusal this operation exists around.** `rename(2)` replaces silently,
/// which is a delete nothing planned.
#[test]
fn an_occupied_destination_is_refused_and_names_the_verb_that_would_clear_it() {
    let mut fake = FakeFiles::new()
        .with("/state/a", Node::file("one"))
        .with("/state/b", Node::file("two"))
        .with("/state/dir", Node::directory());

    let over_file = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/a", "/state/b", CONFIRMATION],
    ));
    assert_eq!(over_file.code(), ErrorCode::Unsafe);
    assert!(
        over_file.message().contains(super::super::remove::FILE_SUBJECT),
        "{}",
        over_file.message()
    );
    assert!(over_file.message().contains("no `--force`"), "{}", over_file.message());
    assert_eq!(fake.nodes["/state/b"].contents, b"two");

    let over_dir = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/a", "/state/dir", CONFIRMATION],
    ));
    assert!(
        over_dir
            .message()
            .contains(super::super::remove::DIRECTORY_SUBJECT),
        "{}",
        over_dir.message()
    );
    assert!(fake.did.is_empty());
}

/// **Directories move**, and the plan says how much is moving so the operator
/// can see it.
#[test]
fn a_directory_moves_and_its_plan_carries_the_entry_count() {
    let mut fake = FakeFiles::new()
        .with("/state/old", Node::directory())
        .with("/state/old/a", Node::file("x"))
        .with("/state/old/b", Node::file("y"));

    let planned = refusal(rename(&mut fake, &[SUBJECT, "/state/old", "/state/new"]));
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("2 entries"), "{}", plan.narrative());

    rename(
        &mut fake,
        &[SUBJECT, "/state/old", "/state/new", CONFIRMATION],
    )
    .expect("a directory must be movable");
    assert!(fake.has("/state/new"));
}

/// **No directory is created on the way** — folding two acts into one is what
/// `ADR-006` §8.9 says not to do.
#[test]
fn a_destination_whose_parent_is_missing_is_refused_and_names_the_other_act() {
    let mut fake = FakeFiles::holding("/state/a", "x");
    let refused = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/a", "/state/deep/down/a", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(
        refused.message().contains("create-directory"),
        "{}",
        refused.message()
    );
    assert!(fake.did.is_empty());
}

#[test]
fn a_source_that_is_not_there_and_a_move_onto_itself_are_both_refused() {
    let mut fake = FakeFiles::holding("/state/a", "x");

    let missing = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/gone", "/state/b", CONFIRMATION],
    ));
    assert_eq!(missing.code(), ErrorCode::Unsafe);

    let itself = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/a", "/state/a", CONFIRMATION],
    ));
    assert_eq!(itself.code(), ErrorCode::Arguments);
    assert!(fake.did.is_empty());
}

/// **Both paths pass the whole gate**, because a move that could not leave
/// through its source could leave through its destination.
#[test]
fn the_gate_is_applied_to_the_destination_as_hard_as_to_the_source() {
    let mut fake = FakeFiles::holding("/state/a", "x");
    for to in [
        "/system/a",
        "/etc/a",
        "/state/../etc/a",
        "/state/config/machine.toml",
        "/state/deploy-request",
    ] {
        let refused = refusal(rename(&mut fake, &[SUBJECT, "/state/a", to, CONFIRMATION]));
        assert!(
            matches!(refused.code(), ErrorCode::Unsafe | ErrorCode::Arguments),
            "{to}: {:?}",
            refused.code()
        );
        assert!(fake.did.is_empty(), "{to} reached the machine");
    }
}

#[test]
fn a_symbolic_link_and_a_mount_point_are_both_refused_as_sources() {
    let mut fake = FakeFiles::new()
        .with("/state/link", Node::symlink("/state/x"))
        .with("/state/vol", Node::directory().mounted());

    let link = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/link", "/state/other", CONFIRMATION],
    ));
    assert!(link.message().contains("symlink"), "{}", link.message());

    let mount = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/vol", "/state/other", CONFIRMATION],
    ));
    assert!(mount.message().contains("MOUNTED"), "{}", mount.message());
    assert!(fake.did.is_empty());
}

/// Moving something ONTO a program path is the case where a file starts being
/// verified that never was — `ADR-012` §7.3's second tier.
#[test]
fn a_move_onto_a_program_path_needs_the_acknowledgement() {
    let mut fake = FakeFiles::new()
        .with("/state/app/thing/0.2.0/bin", Node::directory())
        .with("/state/staged", Node::file("ELF"));
    let bare = refusal(rename(
        &mut fake,
        &[
            SUBJECT,
            "/state/staged",
            "/state/app/thing/0.2.0/bin/thing",
            CONFIRMATION,
        ],
    ));
    assert_eq!(bare.code(), ErrorCode::Unsafe);
    assert!(bare.message().contains("<WARNING>"), "{}", bare.message());
    assert!(fake.has("/state/staged"));
}

/// **The check that matters runs again at the moment of the act**: a
/// destination that became occupied between the plan and the confirmation is
/// refused rather than replaced. That is the refusal this whole operation is
/// built around, so it is the one worth proving is not trusted from the plan.
#[test]
fn a_destination_that_became_occupied_is_refused_at_the_moment_of_the_act() {
    let mut fake = FakeFiles::holding("/state/a", "x");
    let planned = refusal(rename(&mut fake, &[SUBJECT, "/state/a", "/state/b"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    // And the plan is honest about what it cannot promise.
    assert!(
        planned
            .plan()
            .expect("a plan")
            .narrative()
            .contains("DOES NOT GUARANTEE"),
        "the plan over-promises"
    );

    fake.nodes
        .insert("/state/b".to_string(), Node::file("somebody else's"));

    let refused = refusal(rename(
        &mut fake,
        &[SUBJECT, "/state/a", "/state/b", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(fake.has("/state/a"));
    assert_eq!(fake.nodes["/state/b"].contents, b"somebody else's");
}

#[test]
fn naming_fewer_than_two_paths_is_refused_by_name() {
    let mut fake = FakeFiles::new();
    for words in [vec![SUBJECT], vec![SUBJECT, "/state/a"]] {
        let refused = refusal(rename(&mut fake, &words));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{words:?}");
    }
}

#[test]
fn the_declared_field_order_survives_both_shapes() {
    let mut fake = FakeFiles::new()
        .with("/state/f", Node::file("x"))
        .with("/state/d", Node::directory());
    let a = rename(&mut fake, &[SUBJECT, "/state/f", "/state/f2", CONFIRMATION]).expect("a file");
    let b = rename(&mut fake, &[SUBJECT, "/state/d", "/state/d2", CONFIRMATION])
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
