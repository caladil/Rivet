// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::schema::{Kind, Property, Requirement};
use rivet_core::operation::plan::TakesEffect;

/// **`docs/PLAN-Feature-Set-2026-09-12.md` §5.2's own words, captured before
/// the hand-written list that produced them is deleted**: `cmd/rivet-provision`'s
/// old `questions()` asked, in this order and numbered 1 through 9,
/// `Hostname`, `Addressing`, `Address`, `Gateway`, `Mode`, `Fleet`,
/// `Timezone`, `Auto-run`, `Identity`. `Identity` is not a `rivet-config`
/// schema key — it names where `cargo rivet setup` put the board's identity,
/// a fact about the workstation's build, never about the document — so it is
/// not generated here; `cmd/rivet-provision` still appends it itself. The
/// other eight are exactly what a schema-generated interview must reproduce.
const OLD_LABELS_IN_ORDER: [&str; 8] = [
    "Hostname", "Addressing", "Address", "Gateway", "Mode", "Fleet", "Timezone", "Auto-run",
];

/// **Day one reproduces today's order and question numbers exactly.**
/// `docs/PLAN-Feature-Set-2026-09-12.md` §5.2: break this by moving one
/// `DOCUMENT` entry and watch it fail (`AGENTS.md` §4).
#[test]
fn the_generated_interview_matches_the_old_hand_written_order() {
    let generated: Vec<&str> = generate(schema::DOCUMENT).iter().map(|q| q.label).collect();
    assert_eq!(generated, OLD_LABELS_IN_ORDER);
}

/// The same claim, by schema key rather than by display label — the
/// generator's actual contract, since a label is cosmetic and a key is what
/// composing the document reads back.
#[test]
fn the_generated_interview_keys_match_the_old_interview_fields() {
    let keys: Vec<&str> = generate(schema::DOCUMENT).iter().map(|q| q.key).collect();
    assert_eq!(
        keys,
        ["hostname", "method", "address", "gateway", "mode", "fleet-role", "timezone", "auto-run"]
    );
}

/// `version` carries a pinned constant and is never asked — its own `prompt`
/// says so ("not asked; always the current one").
#[test]
fn version_is_never_a_question() {
    assert!(!generate(schema::DOCUMENT).iter().any(|q| q.key == "version"));
}

/// `ssid` and `locked` are real, declared, schema properties — and neither
/// produces a question, on purpose. See the module documentation for why
/// `network` and `kiosk` are curated rather than walked generically.
#[test]
fn ssid_and_locked_are_declared_but_not_asked() {
    assert!(schema::INTERFACE.iter().any(|p| p.name == "ssid"), "fixture assumption");
    assert!(schema::KIOSK.iter().any(|p| p.name == "locked"), "fixture assumption");
    let keys: Vec<&str> = generate(schema::DOCUMENT).iter().map(|q| q.key).collect();
    assert!(!keys.contains(&"ssid"));
    assert!(!keys.contains(&"locked"));
}

/// **Conditions**: `address` and `gateway` gate on `method == "static"` —
/// `ADR-007` §7, republished by `crate::schema::gate` rather than a second
/// copy of the rule.
#[test]
fn address_and_gateway_gate_on_method_equals_static() {
    let questions = generate(schema::DOCUMENT);
    for key in ["address", "gateway"] {
        let q = questions.iter().find(|q| q.key == key).expect(key);
        assert_eq!(q.gate, Some(Gate { on: "method", when: "static" }), "{key}");
    }
    for key in ["hostname", "method", "mode", "fleet-role", "timezone", "auto-run"] {
        let q = questions.iter().find(|q| q.key == key).expect(key);
        assert_eq!(q.gate, None, "{key} should not be gated");
    }
}

/// A question whose condition is false is not asked — `Question::applies`,
/// evaluated against an operator's own answers, e.g. the gateway under DHCP
/// (`docs/PLAN-Feature-Set-2026-09-12.md` §5.2 item A).
#[test]
fn a_gated_question_does_not_apply_when_its_condition_is_false() {
    let questions = generate(schema::DOCUMENT);
    let gateway = questions.iter().find(|q| q.key == "gateway").expect("declared");

    assert!(!gateway.applies(&[("method", "dhcp")]));
    assert!(gateway.applies(&[("method", "static")]));

    let hostname = questions.iter().find(|q| q.key == "hostname").expect("declared");
    assert!(hostname.applies(&[("method", "dhcp")]), "an ungated question always applies");
}

/// `method`, `mode` and `fleet-role` are closed choices, in the schema's own
/// vocabulary; `hostname`, `address`, `gateway`, `timezone` and `auto-run`
/// are free text.
#[test]
fn choices_and_free_text_match_the_schema_kinds() {
    let questions = generate(schema::DOCUMENT);
    let of = |key: &str| questions.iter().find(|q| q.key == key).expect(key).answer;
    assert_eq!(of("method"), Answer::Choice(&["dhcp", "static"]));
    assert_eq!(of("mode"), Answer::Choice(&["headless", "kiosk"]));
    assert_eq!(of("fleet-role"), Answer::Choice(&["server", "member"]));
    for key in ["hostname", "address", "gateway", "timezone", "auto-run"] {
        assert_eq!(of(key), Answer::Text, "{key}");
    }
}

/// A generated prompt is never empty and never the bare description —
/// `ADR-007` §7 already requires this of the schema; this asserts the
/// interview actually carries it through rather than a copy.
#[test]
fn every_question_carries_a_non_empty_prompt() {
    for q in generate(schema::DOCUMENT) {
        assert!(!q.prompt.is_empty(), "{} has no prompt", q.key);
        assert_ne!(q.prompt, q.description, "{}'s prompt repeats its description", q.key);
    }
}

// ---------------------------------------------------------------------------
// The fixture: proving "adding a schema key in the right position is the
// only change needed to add a question" against an array this test controls,
// rather than against the production schema.
// ---------------------------------------------------------------------------

const FIXTURE_EXTRA: Property = Property {
    name: "zip-code",
    requirement: Requirement::Optional,
    kind: Kind::Text { one_of: &[] },
    takes_effect: TakesEffect::OnNextRestart,
    description: "A fixture key, not a real document property.",
    rule: None,
    prompt: "A fixture question:",
};

fn fixture_property(name: &'static str) -> Property {
    schema::DOCUMENT.iter().find(|p| p.name == name).copied().expect(name)
}

/// **The acceptance test**: a new top-level scalar key, inserted between
/// `hostname` and `network` — the same position a real schema edit would use
/// — produces a question at that position and disturbs nothing else. No
/// change anywhere in this module is needed beyond the array itself.
#[test]
fn a_fixture_key_becomes_a_question_in_position() {
    let fixture: Vec<Property> =
        vec![fixture_property("version"), fixture_property("hostname"), FIXTURE_EXTRA, fixture_property("network")];
    let fixture: &'static [Property] = Box::leak(fixture.into_boxed_slice());

    let keys: Vec<&str> = generate(fixture).iter().map(|q| q.key).collect();
    assert_eq!(keys, ["hostname", "zip-code", "method", "address", "gateway"]);
}

/// A key with no `LEGACY_LABELS` entry gets a generated label rather than
/// being left blank or panicking.
#[test]
fn a_key_with_no_legacy_label_gets_a_generated_one() {
    assert_eq!(label_for("zip-code"), "Zip Code");
    assert_eq!(label_for("fleet-role"), "Fleet", "a legacy label wins over the generated one");
}
