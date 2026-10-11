//! `apply set-mode` and `apply set-owner`'s tests.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};

fn mode(fake: &mut FakeFiles, words: &[&str]) -> Result<Changed, OperationError> {
    perform(Kind::Mode, words, fake)
}

fn owner(fake: &mut FakeFiles, words: &[&str]) -> Result<Changed, OperationError> {
    perform(Kind::Owner, words, fake)
}

fn refusal(result: Result<Changed, OperationError>) -> OperationError {
    result.expect_err("that command must be refused")
}

// -- set-mode ----------------------------------------------------------------

#[test]
fn a_mode_is_planned_in_octal_and_in_letters_and_then_set() {
    let mut fake = FakeFiles::new().with("/state/key", Node::file("secret").moded(0o644));

    let planned = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/key", "0600"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("0644 (rw-r--r--)"), "{}", plan.narrative());
    assert!(plan.narrative().contains("0600 (rw-------)"), "{}", plan.narrative());
    assert_eq!(fake.nodes["/state/key"].mode, 0o644);

    mode(&mut fake, &[MODE_SUBJECT, "/state/key", "0600", CONFIRMATION])
        .expect("a mode must be settable");
    assert_eq!(fake.nodes["/state/key"].mode, 0o600);
}

/// **Widening is the direction worth a sentence**, because it is the one that
/// is invisible afterwards.
#[test]
fn widening_a_mode_says_who_it_opens_the_path_to() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").moded(0o600));
    let planned = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", "0644"]));
    let plan = planned.plan().expect("a plan");
    assert!(
        plan.narrative().contains("EVERY uid on this machine"),
        "{}",
        plan.narrative()
    );

    let narrowing = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", "0400"]));
    assert!(
        narrowing
            .plan()
            .expect("a plan")
            .narrative()
            .contains("does not open"),
        "narrowing must not claim to open anything"
    );
}

/// **A symbolic mode is a pattern over a value the caller did not state**, and
/// `ADR-006` §8.9.2's rule refuses exactly that.
#[test]
fn a_symbolic_mode_is_refused_by_name_and_says_what_to_write_instead() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x"));
    for given in ["u+x", "go-rwx", "a=r", "+x", "rwxr-xr-x"] {
        let refused = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", given, CONFIRMATION]));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{given}");
        assert!(refused.message().contains("SYMBOLIC"), "{given}: {}", refused.message());
        assert!(fake.did.is_empty(), "{given}");
    }
}

#[test]
fn something_that_is_not_octal_is_refused_and_says_why_a_decimal_is_dangerous() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x"));
    for given in ["", "8", "0888", "12345", "0o644"] {
        let refused = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", given, CONFIRMATION]));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{given:?}");
        assert!(fake.did.is_empty(), "{given:?}");
    }
}

/// **setuid, setgid and the sticky bit are refused**, and the refusal says it
/// is a decision rather than a limitation.
#[test]
fn the_three_special_bits_are_refused_and_named() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x"));
    for (given, name) in [("4755", "setuid"), ("2755", "setgid"), ("1777", "sticky")] {
        let refused = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", given, CONFIRMATION]));
        assert_eq!(refused.code(), ErrorCode::Unsafe, "{given}");
        assert!(refused.message().contains(name), "{given}: {}", refused.message());
        assert!(fake.did.is_empty(), "{given}");
    }
    // And a mode that carries one is still REPORTED, which is the other half of
    // the decision.
    assert!(super::super::filesystem::render_mode(0o4755).contains("setuid"));
}

/// `super::edit`'s rule for `--expect` equal to `--replace`.
#[test]
fn setting_the_mode_that_is_already_there_is_refused() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").moded(0o600));
    let refused = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", "0600", CONFIRMATION]));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("ALREADY"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

// -- set-owner ---------------------------------------------------------------

/// **Names are resolved before the confirmation**, so an operator never
/// confirms an identifier they have not seen resolved.
#[test]
fn an_owner_is_resolved_in_the_plan_and_then_set() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").owned(0, 0));

    let planned = refusal(owner(&mut fake, &[OWNER_SUBJECT, "/state/f", "app"]));
    let plan = planned.plan().expect("a plan");
    assert!(plan.narrative().contains("0 (root)"), "{}", plan.narrative());
    assert!(plan.narrative().contains("1000 (app)"), "{}", plan.narrative());

    let done = owner(&mut fake, &[OWNER_SUBJECT, "/state/f", "app", CONFIRMATION])
        .expect("an owner must be settable");
    assert_eq!(fake.nodes["/state/f"].uid, 1000);
    assert_eq!(fake.nodes["/state/f"].gid, 0, "the group was not named");
    assert_eq!(done.path(), "/state/f");
    assert!(done.journalled());
}

#[test]
fn a_group_alone_and_a_pair_are_both_accepted() {
    let mut fake = FakeFiles::new()
        .with("/state/a", Node::file("x").owned(0, 0))
        .with("/state/b", Node::file("x").owned(0, 0));

    owner(&mut fake, &[OWNER_SUBJECT, "/state/a", ":app", CONFIRMATION])
        .expect("a group alone must be settable");
    assert_eq!(fake.nodes["/state/a"].uid, 0);
    assert_eq!(fake.nodes["/state/a"].gid, 1000);

    owner(
        &mut fake,
        &[OWNER_SUBJECT, "/state/b", "app:app", CONFIRMATION],
    )
    .expect("a pair must be settable");
    assert_eq!(fake.nodes["/state/b"].uid, 1000);
    assert_eq!(fake.nodes["/state/b"].gid, 1000);
}

/// A number this machine has no name for is still a valid identity — the
/// filesystem holds numbers.
#[test]
fn a_numeric_identity_is_accepted_even_where_there_is_no_name_for_it() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").owned(0, 0));
    owner(&mut fake, &[OWNER_SUBJECT, "/state/f", "4242", CONFIRMATION])
        .expect("a number must be accepted");
    assert_eq!(fake.nodes["/state/f"].uid, 4242);
}

#[test]
fn an_unknown_name_is_refused_by_the_machine_and_nothing_changes() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").owned(0, 0));
    let refused = refusal(owner(
        &mut fake,
        &[OWNER_SUBJECT, "/state/f", "nobody", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::MachineRefused);
    assert_eq!(fake.nodes["/state/f"].uid, 0);
    assert!(fake.did.is_empty());
}

#[test]
fn setting_the_owner_that_is_already_there_is_refused() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").owned(0, 0));
    let refused = refusal(owner(
        &mut fake,
        &[OWNER_SUBJECT, "/state/f", "root:root", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
}

#[test]
fn naming_neither_a_user_nor_a_group_is_refused() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x"));
    for given in ["", ":"] {
        let refused = refusal(owner(&mut fake, &[OWNER_SUBJECT, "/state/f", given, CONFIRMATION]));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{given:?}");
    }
}

// -- both --------------------------------------------------------------------

#[test]
fn a_path_with_nothing_at_it_and_a_symbolic_link_are_both_refused() {
    let mut fake = FakeFiles::new().with("/state/link", Node::symlink("/state/f"));

    let missing = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/gone", "0600", CONFIRMATION]));
    assert_eq!(missing.code(), ErrorCode::Unsafe);

    let link = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/link", "0600", CONFIRMATION]));
    assert_eq!(link.code(), ErrorCode::Unsafe);
    assert!(link.message().contains("does not follow"), "{}", link.message());
    assert!(fake.did.is_empty());
}

#[test]
fn the_gate_refuses_before_the_machine_is_asked_anything() {
    let mut fake = FakeFiles::new();
    for path in ["/system/bin/x", "/etc/passwd", "/state/../etc/x", "/state/config"] {
        let refused = refusal(mode(&mut fake, &[MODE_SUBJECT, path, "0600", CONFIRMATION]));
        assert!(
            matches!(refused.code(), ErrorCode::Unsafe | ErrorCode::Arguments),
            "{path}: {:?}",
            refused.code()
        );
        assert!(fake.did.is_empty(), "{path}");
    }
}

/// The plan says exactly what it does and does not promise, and the no-op check
/// is re-run at the moment of the act rather than trusted from the plan.
#[test]
fn the_plan_does_not_over_promise_and_the_no_op_check_runs_again() {
    let mut fake = FakeFiles::new().with("/state/f", Node::file("x").moded(0o644));
    let planned = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", "0600"]));
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    assert!(
        planned
            .plan()
            .expect("a plan")
            .narrative()
            .contains("DOES NOT GUARANTEE"),
        "the plan over-promises"
    );

    // Something else sets the mode this command was going to set. Re-running
    // the no-op check at the moment of the act catches it.
    fake.nodes
        .insert("/state/f".to_string(), Node::file("x").moded(0o600));
    let refused = refusal(mode(&mut fake, &[MODE_SUBJECT, "/state/f", "0600", CONFIRMATION]));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("ALREADY"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

#[test]
fn changing_a_programs_mode_needs_the_acknowledgement() {
    const PROGRAM: &str = "/state/app/thing/0.2.0/bin/thing";
    let mut fake = FakeFiles::new().with(PROGRAM, Node::file("ELF").moded(0o755));
    let bare = refusal(mode(&mut fake, &[MODE_SUBJECT, PROGRAM, "0700", CONFIRMATION]));
    assert_eq!(bare.code(), ErrorCode::Unsafe);
    assert!(bare.message().contains("<WARNING>"), "{}", bare.message());
    assert_eq!(fake.nodes[PROGRAM].mode, 0o755);
}

#[test]
fn both_result_shapes_keep_the_declared_field_order() {
    let mut fake = FakeFiles::new()
        .with("/state/a", Node::file("x").moded(0o644).owned(0, 0))
        .with("/state/b", Node::file("x").moded(0o644).owned(0, 0));
    let a = mode(&mut fake, &[MODE_SUBJECT, "/state/a", "0600", CONFIRMATION]).expect("a mode");
    let b = owner(&mut fake, &[OWNER_SUBJECT, "/state/b", "app", CONFIRMATION]).expect("an owner");
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
