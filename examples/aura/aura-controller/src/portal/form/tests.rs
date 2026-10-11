//! **The decoder, the repeats, and the two splitters.**

use super::*;

#[test]
fn a_flat_form_decodes_to_its_pairs() {
    let form = Form::parse("brightness=60&screensaver=matrix");
    assert_eq!(form.value("brightness"), "60");
    assert_eq!(form.value("screensaver"), "matrix");
}

/// **`days=Mon&days=Fri` is how a set of checkboxes arrives.** A parser keeping
/// only the last value would silently turn *Monday and Friday* into *Friday*.
#[test]
fn a_repeated_key_keeps_every_value_in_order() {
    let form = Form::parse("days=Mon&days=Wed&days=Fri");
    assert_eq!(form.all("days"), ["Mon", "Wed", "Fri"]);
    // `value` still answers with the first, which is what every other field
    // wants.
    assert_eq!(form.value("days"), "Mon");
}

/// An unchecked checkbox is absent and an emptied text field is empty, and both
/// mean *not set* — so both answer with `""` rather than one of them panicking.
#[test]
fn an_absent_key_and_an_empty_value_read_the_same() {
    let form = Form::parse("zip_code=");
    assert_eq!(form.value("zip_code"), "");
    assert_eq!(form.value("use_24_hour"), "");
    assert!(form.all("use_24_hour").is_empty());
}

/// `+` is a space, and it is the one difference between form encoding and
/// ordinary percent encoding.
#[test]
fn a_plus_is_a_space() {
    assert_eq!(Form::parse("indoor_sensor=Living+Room").value("indoor_sensor"), "Living Room");
}

#[test]
fn percent_escapes_decode_including_the_ones_a_password_contains() {
    let form = Form::parse("field.secret=a%24b%26c%3Dd%25e");
    assert_eq!(form.value("field.secret"), "a$b&c=d%e");
}

/// UTF-8 arrives as its bytes, and a sensor called *Küche* must survive.
#[test]
fn multi_byte_utf8_survives_the_decode() {
    assert_eq!(Form::parse("indoor_sensor=K%C3%BCche").value("indoor_sensor"), "Küche");
}

/// **A malformed escape is kept as written**, not dropped: a password may
/// contain a literal `%`, and a decoder that ate it would shorten it in exactly
/// the way `crate::credential` exists to have stopped.
#[test]
fn a_malformed_escape_is_kept_rather_than_eaten() {
    assert_eq!(Form::parse("p=100%").value("p"), "100%");
    assert_eq!(Form::parse("p=50%zz").value("p"), "50%zz");
    assert_eq!(Form::parse("p=%").value("p"), "%");
}

/// A pair with no `=` is a key with an empty value, which is what a browser
/// sends in a few corners and is not an error a person could act on.
#[test]
fn a_pair_with_no_equals_is_a_key_with_no_value() {
    let form = Form::parse("flag&brightness=10");
    assert_eq!(form.value("flag"), "");
    assert_eq!(form.value("brightness"), "10");
}

#[test]
fn an_empty_body_is_an_empty_form() {
    assert_eq!(Form::parse(""), Form::default());
    assert_eq!(Form::parse("&&").pairs.len(), 0);
}

/// The action rows are numbered, and the reader walks them until the first gap.
#[test]
fn numbered_action_fields_are_reachable_by_name() {
    let form = Form::parse(
        "action.0.type=brightness&action.0.value=20&action.1.type=weather&action.1.value=off",
    );
    assert_eq!(form.value("action.0.type"), "brightness");
    assert_eq!(form.value("action.1.value"), "off");
    assert_eq!(form.value("action.2.type"), "");
}

// ---- The target splitters -------------------------------------------------

#[test]
fn a_target_splits_into_a_path_and_a_query() {
    assert_eq!(path_of("/schedule/edit?index=3"), "/schedule/edit");
    assert_eq!(query_of("/schedule/edit?index=3"), "index=3");
}

#[test]
fn a_target_with_no_query_is_all_path() {
    assert_eq!(path_of("/"), "/");
    assert_eq!(query_of("/"), "");
}

/// A `?` inside the query does not start a second one.
#[test]
fn only_the_first_question_mark_separates() {
    assert_eq!(path_of("/?a=1?2"), "/");
    assert_eq!(query_of("/?a=1?2"), "a=1?2");
}
