// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use rivet_core::Value;

/// **`rivet_config::schema::schema()`, as the JSON this module actually
/// reads.**
///
/// [`walk`] is written against the wire form on purpose — see the module
/// documentation — so a test that fed it `rivet_config`'s own `Value` tree
/// directly would be exercising a shape this module never sees. This
/// converts structurally rather than through a text round trip: both `Value`
/// and `Json` are closed five-variant enums with the same shape, so there is
/// nothing a text encoder could catch that a field-by-field match would not,
/// and the real schema — not a hand-written fixture — is what every test
/// below walks (the brief's own requirement: a fixture schema can drift from
/// what `rivet-config` emits, and the whole point of this module is to
/// never drift from what a machine actually publishes).
fn value_to_json(v: &Value) -> Json {
    match v {
        Value::Str(s) => Json::Str(s.clone()),
        Value::Integer(n) => Json::Integer(*n),
        Value::Bool(b) => Json::Bool(*b),
        Value::List(items) => Json::Array(items.iter().map(value_to_json).collect()),
        Value::Object(fields) => {
            Json::Object(fields.iter().map(|(k, v)| ((*k).to_owned(), value_to_json(v))).collect())
        }
    }
}

fn real_schema() -> Json {
    value_to_json(&rivet_config::schema::schema())
}

/// One field, by key, out of an already-walked list — the assertion a test
/// wants ("does `network` carry a `method`") rather than the position it
/// wants, which is what the ordering tests check separately.
fn field<'a>(fields: &'a [Field], key: &str) -> &'a Field {
    fields.iter().find(|f| f.key == key).unwrap_or_else(|| panic!("no field named {key:?}"))
}

/// **The top-level order [`walk`] reports is `DOCUMENT`'s own declaration
/// order, exactly** — the property this whole module exists to prove.
///
/// Ground truth is `rivet_config::schema::DOCUMENT` itself, walked by name,
/// not a copy of the list re-typed here: if a key is ever reordered in
/// `rivet-config`, this test's expectation moves with it automatically and
/// only a REAL ordering defect in [`walk`] can fail it.
#[test]
fn top_level_fields_follow_the_documents_declaration_order() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let got: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    let expected: Vec<&str> = rivet_config::schema::DOCUMENT.iter().map(|p| p.name).collect();
    assert_eq!(got, expected);
}

/// **`network.<name>.address` and `.gateway` are gated on `method ==
/// "static"`, and `method` and `ssid` carry no gate** — `ADR-007` §7,
/// proved against the schema `rivet-config` really emits rather than a
/// hand-written fixture.
#[test]
fn the_interface_gate_matches_the_real_conditional() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let network = field(&fields, "network");
    let Shape::NamedTables { fields: entry } = &network.shape else {
        panic!("`network` is not NamedTables: {:?}", network.shape);
    };
    let expected_gate = Some(Gate { on: "method".to_owned(), when: "static".to_owned() });
    assert_eq!(field(entry, "address").gate, expected_gate);
    assert_eq!(field(entry, "gateway").gate, expected_gate);
    assert_eq!(field(entry, "method").gate, None);
    assert_eq!(field(entry, "ssid").gate, None);
    // Declaration order within the entry, same proof as the top level.
    let got: Vec<&str> = entry.iter().map(|f| f.key.as_str()).collect();
    let expected: Vec<&str> = rivet_config::schema::INTERFACE.iter().map(|p| p.name).collect();
    assert_eq!(got, expected);
}

/// **Breaking either conditional's extraction is exactly what this test
/// exists to catch** — `AGENTS.md` §4: this asserts the SPECIFIC gate values
/// `rivet_config::schema::named_gate` already publishes for the same two
/// keys, so a change to either side that stops agreeing with the other
/// fails here rather than only in a browser nobody is watching.
#[test]
fn the_extracted_gates_agree_with_rivet_configs_own_named_gate() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let network = field(&fields, "network");
    let Shape::NamedTables { fields: entry } = &network.shape else {
        panic!("`network` is not NamedTables");
    };
    for key in ["method", "address", "gateway", "ssid"] {
        let want = rivet_config::schema::named_gate("network", key)
            .map(|(on, when)| Gate { on: on.to_owned(), when: when.to_owned() });
        assert_eq!(field(entry, key).gate, want, "{key}");
    }
}

/// **`kiosk` is a fixed `Table`, its own fields in order, and none is
/// gated** — `[kiosk]` carries no conditional in this schema, which is
/// exactly R-164's point: `mode` and `[kiosk]` disagreement is not expressed
/// as a schema gate at all. `launcher` and `compositor` name the two display
/// roles and default to the image's own.
#[test]
fn kiosk_is_an_ungated_table_with_auto_run_locked_launcher_and_compositor() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let kiosk = field(&fields, "kiosk");
    let Shape::Table { fields: nested } = &kiosk.shape else {
        panic!("`kiosk` is not a Table: {:?}", kiosk.shape);
    };
    let got: Vec<&str> = nested.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(got, ["auto-run", "locked", "launcher", "compositor"]);
    assert!(nested.iter().all(|f| f.gate.is_none()));
}

/// **`mode` itself is a plain gated-nothing scalar carrying its own
/// `enum`.** Proves `Shape::Scalar`'s `enum_values` is read off the real
/// schema rather than assumed empty.
#[test]
fn mode_is_a_scalar_with_a_nonempty_enum() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let mode = field(&fields, "mode");
    let Shape::Scalar { json_type, enum_values } = &mode.shape else {
        panic!("`mode` is not a Scalar: {:?}", mode.shape);
    };
    assert_eq!(json_type, "string");
    assert!(enum_values.contains(&"kiosk".to_owned()), "{enum_values:?}");
    assert!(enum_values.contains(&"headless".to_owned()), "{enum_values:?}");
}

/// **`applications` is a `NamedTables` whose entry declares no properties at
/// all** — `rivet_config::schema::Kind::NamedTablesOfStrings` publishes an
/// open string map on purpose (the shape comes from a signed manifest this
/// crate cannot see), and `walk` must report that as zero fields rather than
/// erroring or inventing one.
#[test]
fn applications_is_a_named_table_with_no_fields_of_its_own() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let applications = field(&fields, "applications");
    let Shape::NamedTables { fields: entry } = &applications.shape else {
        panic!("`applications` is not NamedTables: {:?}", applications.shape);
    };
    assert!(entry.is_empty(), "{entry:?}");
}

/// **Prompt and description flow through unchanged**, for one key that is
/// easy to eyeball wrong: `hostname`'s prompt is a short question, its
/// description is the long, ABSENT-IS-NOT-NAMELESS paragraph, and the two
/// must not have been swapped or concatenated.
#[test]
fn prompt_and_description_are_read_from_the_right_keys_and_kept_apart() {
    let fields = walk(&real_schema()).expect("the real schema walks");
    let hostname = field(&fields, "hostname");
    assert_eq!(hostname.prompt, "The machine's name on the network:");
    assert!(hostname.description.starts_with("What this machine calls itself."));
    assert_ne!(hostname.prompt, hostname.description);
}

/// **A schema object with no `properties` is refused, not misread as
/// empty** — the failure a machine running a build with a broken schema
/// generator would produce, and the form must say so rather than drawing
/// nothing and looking finished.
#[test]
fn a_schema_object_with_no_properties_is_refused() {
    let broken = Json::Object(vec![("type".to_owned(), Json::Str("object".to_owned()))]);
    let why = walk(&broken).expect_err("refused");
    assert!(why.contains("properties"), "{why}");
}

/// And a schema that is not an object at all — the shape `get schema` would
/// never send, but a form reading an untrusted machine's answer must not
/// panic on it.
#[test]
fn a_schema_that_is_not_an_object_is_refused_not_panicked_on() {
    let why = walk(&Json::Str("not a schema".to_owned())).expect_err("refused");
    assert!(why.contains("a string"), "{why}");
}

// ---------------------------------------------------------------------------
// `applies` — the same evaluation `rivet_config::interview::Question::applies`
// performs, read by a form instead of an interview.
// ---------------------------------------------------------------------------

#[test]
fn an_ungated_field_always_applies() {
    assert!(applies(None, &[]));
    assert!(applies(None, &[("method", "dhcp")]));
}

#[test]
fn a_gated_field_applies_only_when_its_sibling_matches() {
    let gate = Gate { on: "method".to_owned(), when: "static".to_owned() };
    assert!(applies(Some(&gate), &[("method", "static")]));
    assert!(!applies(Some(&gate), &[("method", "dhcp")]));
    assert!(!applies(Some(&gate), &[]));
}

// ---------------------------------------------------------------------------
// `kiosk_mode_notice` — R-164, R-165.
// ---------------------------------------------------------------------------

/// **The one mismatch R-164 asks to be made visible, and only that one.**
#[test]
fn the_notice_fires_only_for_kiosk_mode_with_no_kiosk_table() {
    assert!(kiosk_mode_notice(Some("kiosk"), false).is_some());
    assert!(kiosk_mode_notice(Some("kiosk"), true).is_none());
    assert!(kiosk_mode_notice(Some("headless"), false).is_none());
    // R-165: a headless machine may carry a `[kiosk]` table it does not use,
    // and that is not a contradiction either.
    assert!(kiosk_mode_notice(Some("headless"), true).is_none());
    // No `mode` at all means what it means today — `[kiosk]`'s own presence
    // decides, and there is nothing for this notice to add.
    assert!(kiosk_mode_notice(None, false).is_none());
    assert!(kiosk_mode_notice(None, true).is_none());
}
