//! `rivet get diff`'s tests.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};

fn diff_of(fake: &mut FakeFiles, path: &str, against: &str) -> Value {
    perform(&[SUBJECT, path, against], fake).expect("this comparison must answer")
}

fn refusal(fake: &mut FakeFiles, words: &[&str]) -> OperationError {
    perform(words, fake).expect_err("that command must be refused")
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.iter().find(|(key, _)| *key == name).map(|(_, v)| v),
        _ => panic!("the answer is not an object"),
    }
}

fn text(value: &Value, name: &str) -> String {
    match field(value, name) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{name} is {other:?}"),
    }
}

fn flag(value: &Value, name: &str) -> bool {
    match field(value, name) {
        Some(Value::Bool(b)) => *b,
        other => panic!("{name} is {other:?}"),
    }
}

/// The case an agent is actually in: it is about to change one line and wants
/// to see that it is the line it meant.
#[test]
fn one_changed_line_answers_with_a_unified_diff_of_exactly_that_line() {
    let mut fake = FakeFiles::holding("/state/c.nu", "# nu\nbanner = true\nmode = vi\n");
    let answer = diff_of(&mut fake, "/state/c.nu", "# nu\nbanner = false\nmode = vi\n");

    assert!(flag(&answer, "present"));
    assert!(flag(&answer, "changed"));
    assert_eq!(field(&answer, "added_lines"), Some(&Value::Integer(1)));
    assert_eq!(field(&answer, "removed_lines"), Some(&Value::Integer(1)));
    assert!(!flag(&answer, "truncated"));
    assert_eq!(text(&answer, "algorithm"), "lines");
    let d = text(&answer, "diff");
    assert!(d.contains("--- /state/c.nu"), "{d}");
    assert!(d.contains("-banner = true"), "{d}");
    assert!(d.contains("+banner = false"), "{d}");
}

/// **An absent file is diffed against empty, not refused** — the answer is
/// useful where a refusal would only have been correct.
#[test]
fn a_path_with_nothing_at_it_diffs_against_empty_and_says_it_is_a_creation() {
    let mut fake = FakeFiles::new();
    let answer = diff_of(&mut fake, "/state/new.toml", "a = 1\nb = 2\n");

    assert!(!flag(&answer, "present"));
    assert!(flag(&answer, "changed"));
    assert_eq!(field(&answer, "added_lines"), Some(&Value::Integer(2)));
    assert_eq!(field(&answer, "removed_lines"), Some(&Value::Integer(0)));
    assert!(text(&answer, "next").contains("create-file"));
}

#[test]
fn identical_text_says_nothing_would_change_and_says_edit_would_refuse_it() {
    let mut fake = FakeFiles::holding("/state/x", "same\n");
    let answer = diff_of(&mut fake, "/state/x", "same\n");

    assert!(!flag(&answer, "changed"));
    assert_eq!(text(&answer, "diff"), "");
    assert!(text(&answer, "next").contains("ALREADY HOLDS"));
}

/// Reading is not confined to the permitted root — `ADR-006` §8.10.4's shorter
/// gate for reads.
#[test]
fn a_path_outside_the_permitted_root_is_compared_rather_than_refused() {
    let mut fake = FakeFiles::new().with("/system/etc/x", Node::file("a\n"));
    let answer = diff_of(&mut fake, "/system/etc/x", "b\n");
    assert!(flag(&answer, "changed"));
}

// -- refusals ---------------------------------------------------------------

#[test]
fn a_directory_and_a_symbolic_link_are_both_refused_and_each_says_which_it_is() {
    let mut fake = FakeFiles::new()
        .with("/state/d", Node::directory())
        .with("/state/l", Node::symlink("/state/d"));

    let dir = refusal(&mut fake, &[SUBJECT, "/state/d", "x"]);
    assert_eq!(dir.code(), ErrorCode::Unsafe);
    assert!(dir.message().contains("is a directory"), "{}", dir.message());

    let link = refusal(&mut fake, &[SUBJECT, "/state/l", "x"]);
    assert_eq!(link.code(), ErrorCode::Unsafe);
    assert!(link.message().contains("NOT FOLLOWED"), "{}", link.message());
}

/// **Never a guess about an encoding**, and the refusal names the offset.
#[test]
fn a_file_that_is_not_utf8_is_refused_rather_than_rendered() {
    let mut fake = FakeFiles::new().with("/state/blob", Node::file([0x41, 0xff]));
    let refused = refusal(&mut fake, &[SUBJECT, "/state/blob", "A"]);
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("offset 1"), "{}", refused.message());
}

#[test]
fn a_file_or_a_text_larger_than_the_caps_is_refused_with_the_number() {
    let big = vec![b'x'; super::super::edit::FILE_MAX_BYTES + 1];
    let mut fake = FakeFiles::new().with("/state/big", Node::file(&big));
    let file = refusal(&mut fake, &[SUBJECT, "/state/big", "x"]);
    assert_eq!(file.code(), ErrorCode::Unsafe);

    let long = "y".repeat(super::super::edit::TEXT_MAX_BYTES + 1);
    let text_too_big = refusal(&mut fake, &[SUBJECT, "/state/small", &long]);
    assert_eq!(text_too_big.code(), ErrorCode::Arguments);
}

/// **FS4: an over-cap file is refused from the STAT, and never read at
/// all.** `stat_path` already knows the size before `readable` would ever
/// be called; this proves the refusal happens on that number rather than on
/// the whole file having been read in first and measured afterward.
#[test]
fn a_file_over_the_cap_is_refused_without_being_read() {
    let big = vec![b'x'; super::super::edit::FILE_MAX_BYTES + 1];
    let mut fake = FakeFiles::new().with("/state/big", Node::file(&big));
    let refused = refusal(&mut fake, &[SUBJECT, "/state/big", "x"]);
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(
        refused
            .message()
            .contains(&(super::super::edit::FILE_MAX_BYTES + 1).to_string()),
        "{}",
        refused.message()
    );
    assert!(
        fake.whole_reads.is_empty(),
        "the whole-file reader was called before the size refusal: {:?}",
        fake.whole_reads
    );
}

#[test]
fn missing_and_extra_words_are_both_refused_by_name() {
    let mut fake = FakeFiles::new();
    for words in [
        vec![SUBJECT],
        vec![SUBJECT, "/state/x"],
        vec![SUBJECT, "/state/x", "a", "b"],
    ] {
        let refused = refusal(&mut fake, &words);
        assert_eq!(refused.code(), ErrorCode::Arguments, "{words:?}");
    }
}

#[test]
fn the_declared_field_order_survives_every_shape_this_subject_produces() {
    let mut fake = FakeFiles::holding("/state/x", "a\n");
    for (path, against) in [("/state/x", "b\n"), ("/state/x", "a\n"), ("/state/gone", "c\n")] {
        let answer = diff_of(&mut fake, path, against);
        let Value::Object(fields) = &answer else {
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
