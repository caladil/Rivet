//! **Every `FieldKind` produces its control — a `Secret` a password input that
//! is stored, not sent to the addon — bounds are enforced before a `Configure`,
//! and a field set outside this module's own vocabulary renders anyway.**
//!
//! The last of those is the one that matters most: [`unseen_fields`] below is
//! not any installed source, is not a real addon at all, and every assertion in
//! this file still holds against it. That is the whole test of whether
//! `crate::portal::schema` special-cases anything.

use super::*;

fn bool_field() -> Field {
    Field {
        key: "alerts_enabled".to_string(),
        label: "Enable alerts".to_string(),
        help: "Page when a reading is out of range.".to_string(),
        kind: FieldKind::Bool { default: true },
    }
}

fn number_field() -> Field {
    Field {
        key: "poll_seconds".to_string(),
        label: "Poll interval".to_string(),
        help: String::new(),
        kind: FieldKind::Number {
            default: 30.0,
            min: 5.0,
            max: 300.0,
            unit: "s".to_string(),
        },
    }
}

fn choice_field() -> Field {
    Field {
        key: "alert_level".to_string(),
        label: "Alert level".to_string(),
        help: String::new(),
        kind: FieldKind::Choice {
            default: "warn".to_string(),
            options: vec![
                ("info".to_string(), "Informational".to_string()),
                ("warn".to_string(), "Warning".to_string()),
                ("crit".to_string(), "Critical".to_string()),
            ],
        },
    }
}

fn text_field() -> Field {
    Field {
        key: "pager_note".to_string(),
        label: "Pager note".to_string(),
        help: String::new(),
        kind: FieldKind::Text {
            default: String::new(),
            max_len: 40,
        },
    }
}

fn secret_field() -> Field {
    Field {
        key: "pager".to_string(),
        label: "Pager credential".to_string(),
        help: String::new(),
        kind: FieldKind::Secret {
            name: "pager.env".to_string(),
        },
    }
}

/// **A field set that is invented for this test file alone, and belongs to no
/// real addon.** Every test in this file that wants to prove genericity runs
/// against this rather than against any installed source's fields, because a
/// test that only ever exercised one addon's shape would not distinguish *this
/// module is generic* from *this module happens to work for that addon*.
fn unseen_fields() -> Vec<Field> {
    vec![
        bool_field(),
        number_field(),
        choice_field(),
        text_field(),
        secret_field(),
    ]
}

// ---- Each `FieldKind` produces its control ---------------------------------

#[test]
fn a_bool_field_renders_a_checkbox() {
    let rendered = render_fields(&[bool_field()], &Map::new(), &[]);
    assert!(rendered.contains("type=\"checkbox\""), "{rendered}");
    assert!(rendered.contains("name=\"field.alerts_enabled\""));
    assert!(rendered.contains("Enable alerts"));
    // The default is `true`, and nothing has been saved yet.
    assert!(rendered.contains("checked"), "{rendered}");
}

#[test]
fn a_number_field_renders_an_input_bounded_by_its_own_min_and_max() {
    let rendered = render_fields(&[number_field()], &Map::new(), &[]);
    assert!(rendered.contains("type=\"number\""), "{rendered}");
    assert!(rendered.contains("min=\"5\""), "{rendered}");
    assert!(rendered.contains("max=\"300\""), "{rendered}");
    assert!(rendered.contains("name=\"field.poll_seconds\""));
}

#[test]
fn a_choice_field_renders_a_select_of_exactly_its_own_options() {
    let rendered = render_fields(&[choice_field()], &Map::new(), &[]);
    assert!(rendered.contains("<select"), "{rendered}");
    for (value, label) in [("info", "Informational"), ("warn", "Warning"), ("crit", "Critical")] {
        assert!(rendered.contains(&format!("value=\"{value}\"")), "{rendered}");
        assert!(rendered.contains(label), "{rendered}");
    }
    // The default is `warn`, and nothing has been saved yet.
    assert!(
        rendered.contains("<option value=\"warn\" selected>"),
        "{rendered}"
    );
}

#[test]
fn a_text_field_renders_an_input_bounded_by_its_own_max_len() {
    let rendered = render_fields(&[text_field()], &Map::new(), &[]);
    assert!(rendered.contains("type=\"text\""), "{rendered}");
    assert!(rendered.contains("maxlength=\"40\""), "{rendered}");
}

/// **The one that changed on 2026-09-22.** A `Secret` is now a typed input —
/// the maintainer's ruling, reversing R-197 D3. It renders a password control,
/// says whether one is stored (none is, in this test), and no longer prints a
/// CLI command.
#[test]
fn a_secret_field_renders_a_password_input() {
    let rendered = render_fields(&[secret_field()], &Map::new(), &[]);
    assert!(
        rendered.contains("type=\"password\""),
        "a Secret field must render a password input: {rendered}"
    );
    assert!(
        rendered.contains("not stored"),
        "no secret was stored in this test, and the page must say so: {rendered}"
    );
    assert!(
        !rendered.contains("install-secret"),
        "the old CLI-command hint must be gone: {rendered}"
    );
}

// ---- Current values and defaults -------------------------------------------

#[test]
fn a_saved_value_is_shown_rather_than_the_field_s_default() {
    let mut current = Map::new();
    current.insert("poll_seconds".to_string(), serde_json::json!(120.0));
    let rendered = render_fields(&[number_field()], &current, &[]);
    assert!(rendered.contains("value=\"120\""), "{rendered}");
    assert!(!rendered.contains("value=\"30\""), "{rendered}");
}

#[test]
fn an_unsaved_field_falls_back_to_its_own_default() {
    let rendered = render_fields(&[number_field()], &Map::new(), &[]);
    assert!(rendered.contains("value=\"30\""), "{rendered}");
}

// ---- A field can start hidden, by key, and it is opt-in --------------------

/// **Naming a key in `hidden` starts its control at `display:none`, and
/// nothing else changes about it** — the input, the label and the help text
/// are all still there, so a caller's own script can reveal it without
/// asking the server again.
#[test]
fn a_field_named_in_hidden_starts_with_display_none() {
    let rendered = render_fields(&[text_field()], &Map::new(), &["pager_note"]);
    assert!(
        rendered.contains("<div class=\"field\" style=\"display:none\">"),
        "{rendered}"
    );
    assert!(rendered.contains("name=\"field.pager_note\""), "{rendered}");
}

/// **A key nobody named is rendered exactly as before** — this is the
/// default, proven directly rather than trusted by omission.
#[test]
fn a_field_not_named_in_hidden_renders_with_no_style_attribute_at_all() {
    let rendered = render_fields(&[text_field()], &Map::new(), &[]);
    assert!(rendered.contains("<div class=\"field\">"), "{rendered}");
    assert!(!rendered.contains("display:none"), "{rendered}");
}

/// **`hidden` names a [`Field::key`], never a position** — with two fields
/// and only the second one's key listed, the first must render ordinarily.
#[test]
fn hidden_names_the_matching_key_and_no_other_field_in_the_same_set() {
    let rendered = render_fields(&[number_field(), text_field()], &Map::new(), &["pager_note"]);
    assert!(
        rendered.contains("<div class=\"field\">\n<label for=\"field-poll_seconds\">"),
        "an unrelated field must not be hidden by another field's key: {rendered}"
    );
    assert!(
        rendered.contains("<div class=\"field\" style=\"display:none\">\n\
                            <label for=\"field-pager_note\">"),
        "{rendered}"
    );
}

// ---- Bounds are enforced before a Configure --------------------------------

#[test]
fn a_number_outside_its_own_range_is_refused() {
    let submitted = Form::parse("field.poll_seconds=1000");
    let err = validate_fields(&[number_field()], &submitted).expect_err("out of range");
    assert!(err.contains("Poll interval"), "{err}");
    assert!(err.contains("5") && err.contains("300"), "{err}");
}

#[test]
fn a_number_in_range_is_accepted_and_carries_its_own_key() {
    let submitted = Form::parse("field.poll_seconds=45");
    let parsed = validate_fields(&[number_field()], &submitted).expect("in range");
    assert_eq!(parsed["poll_seconds"], serde_json::json!(45.0));
}

#[test]
fn a_number_that_does_not_parse_is_refused_rather_than_defaulted() {
    let submitted = Form::parse("field.poll_seconds=not-a-number");
    let err = validate_fields(&[number_field()], &submitted).expect_err("not a number");
    assert!(err.contains("Poll interval"), "{err}");
}

#[test]
fn a_choice_outside_its_own_options_is_refused() {
    let submitted = Form::parse("field.alert_level=made-up");
    let err = validate_fields(&[choice_field()], &submitted).expect_err("not an option");
    assert!(err.contains("Alert level"), "{err}");
}

#[test]
fn a_choice_that_is_one_of_its_own_options_is_accepted() {
    let submitted = Form::parse("field.alert_level=crit");
    let parsed = validate_fields(&[choice_field()], &submitted).expect("a real option");
    assert_eq!(parsed["alert_level"], serde_json::json!("crit"));
}

#[test]
fn text_longer_than_its_own_max_len_is_refused() {
    let long = "x".repeat(41);
    let submitted = Form::parse(&format!("field.pager_note={long}"));
    let err = validate_fields(&[text_field()], &submitted).expect_err("too long");
    assert!(err.contains("Pager note"), "{err}");
}

#[test]
fn an_absent_bool_is_false_exactly_as_an_unchecked_checkbox_is() {
    let submitted = Form::parse("");
    let parsed = validate_fields(&[bool_field()], &submitted).expect("bools never fail");
    assert_eq!(parsed["alerts_enabled"], serde_json::json!(false));
}

/// **A `Secret` contributes nothing to the addon's settings map.** Its input is
/// read by `store_secrets`, not `validate_fields`, so a submission validates
/// the same whether or not a credential was typed, and never fails on it.
#[test]
fn a_secret_field_contributes_nothing_and_never_fails() {
    let submitted = Form::parse("");
    let parsed = validate_fields(&[secret_field()], &submitted).expect("a Secret never fails");
    assert!(!parsed.contains_key("pager"), "{parsed:?}");
}

/// **`store_secrets` leaves a stored secret alone when the control is blank.**
/// The empty-field case needs no data directory, because nothing is written —
/// which is what lets the rest of a form save without re-entering a credential.
#[test]
fn store_secrets_skips_a_blank_field() {
    assert_eq!(store_secrets(&[secret_field()], &Form::parse("field.pager=")), Ok(false));
    assert_eq!(store_secrets(&[secret_field()], &Form::parse("")), Ok(false));
}

// ---- A field set this module has never seen --------------------------------

/// **This is the test the whole module exists to pass.** `unseen_fields` is
/// no real source and names nothing this file has ever special-cased — a Bool,
/// a Number, a Choice, a Text and a Secret from a fictional pager addon — and
/// every one of them renders its own control with no change to this module.
#[test]
fn an_addon_this_module_has_never_seen_renders_correctly() {
    let rendered = render_fields(&unseen_fields(), &Map::new(), &[]);
    assert!(rendered.contains("type=\"checkbox\""), "{rendered}");
    assert!(rendered.contains("type=\"number\""), "{rendered}");
    assert!(rendered.contains("<select"), "{rendered}");
    assert!(rendered.contains("type=\"text\""), "{rendered}");
    assert!(rendered.contains("type=\"password\""), "{rendered}");
    assert!(rendered.contains("not stored"), "{rendered}");
    // Five fields, five controls' worth of markers — nothing dropped, nothing
    // duplicated. Four `class="field"` wrappers (number, choice, text,
    // secret) plus the one `toggle-row` the bool renders instead.
    assert_eq!(rendered.matches("class=\"field\"").count() + rendered.matches("toggle-row").count(), 5, "{rendered}");
}

/// And it validates the same way: a submission against fields this module has
/// never seen is checked against exactly the bounds THEY state, not against
/// anything a real addon happens to use.
#[test]
fn a_submission_against_an_unseen_field_set_is_validated_against_its_own_bounds() {
    let submitted = Form::parse("field.poll_seconds=9999&field.alert_level=warn&field.alerts_enabled=1");
    let err = validate_fields(&unseen_fields(), &submitted).expect_err("9999s is out of range");
    assert!(err.contains("Poll interval"), "{err}");

    let submitted = Form::parse("field.poll_seconds=60&field.alert_level=crit&field.alerts_enabled=1&field.pager_note=hi");
    let parsed = validate_fields(&unseen_fields(), &submitted).expect("every value in range");
    assert_eq!(parsed["poll_seconds"], serde_json::json!(60.0));
    assert_eq!(parsed["alert_level"], serde_json::json!("crit"));
    assert_eq!(parsed["alerts_enabled"], serde_json::json!(true));
    assert_eq!(parsed["pager_note"], serde_json::json!("hi"));
    assert!(!parsed.contains_key("pager"), "the Secret field must not appear");
}
