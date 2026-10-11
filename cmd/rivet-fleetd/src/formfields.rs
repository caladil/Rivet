//! **The pure half of the portal's configuration screen** — plan
//! `docs/PLAN-Feature-Set-2026-09-12.md` §5.3, `.rulings/` R-162 (Wave 3c).
//!
//! # What this module is, and what it is deliberately not
//!
//! `web/config.js` draws the form. This module carries the two things worth
//! proving with a real test rather than trusting a browser to get right:
//! **the order and the conditions**. It walks a JSON Schema object shaped
//! exactly like the one `rivet_core::operation::schema` (`get schema`) hands
//! back — the wire form, not `rivet_config::schema::DOCUMENT`'s Rust types —
//! because the form this screen draws has to follow whatever the *target
//! machine* published, which may be a different build than this server's own
//! (plan §5.3: *"asks the target machine `get schema`… not the fleet
//! server's own compiled-in schema"*). Reading the wire form here rather than
//! importing `rivet_config`'s types is what keeps that promise honest — this
//! function would work identically against an older or newer machine's
//! schema, because it never assumes anything beyond what JSON Schema and
//! `ADR-007` §7's own extension keywords already say.
//!
//! **This program never calls [`walk`] at run time.** `rivet-fleetd` still
//! grows no new route (plan §5.3) — the browser fetches the schema straight
//! from the machine over the existing `POST /mcp/<device>` and walks it
//! itself, in `web/config.js`. What lives here is the same algorithm,
//! transliterated, kept where `cargo test` can hold it against the schema
//! `rivet-config` really emits. The two are proved to agree only by having
//! one obviously short algorithm and writing it twice; there is no automatic
//! check that `web/config.js` has not drifted from this file, which is the
//! honest limit of a page with no JS test harness — see
//! `cmd/rivet-fleetd/src/page.rs`'s own tests for the structural check that
//! still applies.
//!
//! # Declaration order
//!
//! A JSON object's own key order is what [`walk`] reads — `rivet_core::json`
//! parses one into `Json::Object(Vec<(String, Json)>)`, ordered and not
//! reordered, for exactly this reason, and a browser's `JSON.parse` gives the
//! same guarantee for string keys. `rivet_config::schema::schema` emits
//! `properties` in `DOCUMENT`'s own declaration order (`ADR-007` §7), so
//! walking the object in the order it parsed *is* walking the document in
//! the order it was declared, with nothing here re-sorting or re-deriving it.
//!
//! # Conditions
//!
//! `rivet_config::schema::conditional_schema` publishes each sibling rule as
//! JSON Schema's own `if`/`then`/`else` rather than only in prose
//! (`ADR-007` §7). [`extract_gates`] reads that structurally: a property
//! named in `then.required` or in `else.not.anyOf[].required` is asked, and
//! written, only when the `if` block's one named sibling equals its `const`
//! — the same rule `rivet_config::schema::gate` encodes on the writing side,
//! read back rather than re-invented.
//!
//! # `mode` and `[kiosk]` — R-164, R-165
//!
//! Neither ruling changed the schema: `mode` carries no `if`/`then`/`else`
//! naming `kiosk`, on purpose (`.rulings/` R-164: *"no refusal or tolerance
//! rule was added to the parser"*). So [`kiosk_mode_notice`] is not a gate —
//! it is a separate, document-*values* judgement a form makes after it has
//! read what the operator's current document actually holds, and it never
//! hides the `[kiosk]` table or refuses to let it be edited. It exists only
//! to say plainly what R-164 asks to be made visible: `mode = "kiosk"` with
//! no `[kiosk]` table is a mode asked for and not configured.

use rivet_core::json::Json;

/// A property gated on a sibling's value — republishes one entry of an
/// `allOf` block's `if`/`then`/`else`, never invents a rule of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    /// The sibling property this one is gated on.
    pub on: String,
    /// The value `on` must carry for this property to be asked or written.
    pub when: String,
}

/// What a property's value may hold, read off the JSON Schema wire form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// A leaf — `type` is `string`, `boolean`, `integer` or `array`, and
    /// `enum_values` is non-empty exactly when the schema published one.
    Scalar {
        json_type: String,
        enum_values: Vec<String>,
    },
    /// `type: object` with a fixed `properties` list of its own — `[kiosk]`,
    /// `[control-plane]`. Configured once, by name.
    Table { fields: Vec<Field> },
    /// `type: object` whose `additionalProperties` is itself a table schema
    /// — `network`, `services`, `volumes`. The operator names the entries;
    /// `fields` is the shape every one of them takes.
    NamedTables { fields: Vec<Field> },
}

/// One property, in the position it was declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The schema's own key — never a display label.
    pub key: String,
    /// The short form a form asks with — `Property::prompt`.
    pub prompt: String,
    /// The long form — `Property::description`.
    pub description: String,
    /// Asked, and composed into the document, only when this is satisfied
    /// against the operator's other answers. `None` for an ungated property.
    pub gate: Option<Gate>,
    pub shape: Shape,
}

/// **Walk a JSON Schema object into an ordered field list.**
///
/// `schema` is either the whole document schema (`get schema`'s own
/// `"schema"` value) or, recursively, one property's own sub-schema for a
/// `Table`/`NamedTables` kind — both shapes carry `properties` and, where it
/// applies, `allOf`, which is the whole of what this function reads.
///
/// # Errors
///
/// A sentence, when `schema` is not an object or carries no `properties` —
/// which means the machine answered something this form cannot have been
/// built from, not that a value inside it was wrong.
pub fn walk(schema: &Json) -> Result<Vec<Field>, String> {
    let Json::Object(top) = schema else {
        return Err(format!(
            "a schema object was expected here and this is {} instead",
            schema.kind()
        ));
    };
    let Some(Json::Object(properties)) = find(top, "properties") else {
        return Err("this schema object carries no `properties`".to_owned());
    };
    let gates = extract_gates(find(top, "allOf"));
    let mut out = Vec::with_capacity(properties.len());
    for (key, sub) in properties {
        out.push(field_from(key, sub, gates.get(key).cloned())?);
    }
    Ok(out)
}

/// One field of an object's own `("key", value)` pairs, by name.
fn find<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn text(node: &Json, key: &str) -> String {
    node.get(key).and_then(Json::as_str).unwrap_or_default().to_owned()
}

/// One property's [`Field`], recursing into a `Table`/`NamedTables` shape.
fn field_from(key: &str, schema: &Json, gate: Option<Gate>) -> Result<Field, String> {
    let prompt = text(schema, "prompt");
    let description = text(schema, "description");
    let json_type = text(schema, "type");

    let shape = if json_type == "object" {
        match schema.get("additionalProperties") {
            // `Kind::NamedTables` — `rivet_config::schema::property` splices
            // the entry table in as an OBJECT here; `Kind::Table` always
            // carries the boolean `false` in this position instead (see the
            // module documentation's cross-reference). Matching specifically
            // on an object, rather than any non-boolean value, means a
            // schema that ever widened `additionalProperties` to `true`
            // falls through to the `Table` arm below rather than being
            // misread as an entry shape with no properties.
            //
            // **An entry with no `properties` of its own is a `NamedTables`
            // whose fields are not enumerable at all** —
            // `rivet_config::schema::Kind::NamedTablesOfStrings`
            // (`applications`): every setting name comes from an installed
            // application's own manifest, so there is nothing here for a
            // generated form to walk into. `walk` refuses an object with no
            // `properties` for every OTHER caller, correctly — this is the
            // one shape where that absence is the answer rather than a
            // malformed schema, so it is read directly instead of handed to
            // `walk`, which would refuse it.
            Some(Json::Object(entry_fields)) if find(entry_fields, "properties").is_none() => {
                Shape::NamedTables { fields: Vec::new() }
            }
            Some(entry @ Json::Object(_)) => Shape::NamedTables { fields: walk(entry)? },
            _ => Shape::Table { fields: walk(schema)? },
        }
    } else {
        let enum_values = schema
            .get("enum")
            .and_then(Json::as_array)
            .map(|items| items.iter().filter_map(Json::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        Shape::Scalar { json_type, enum_values }
    };

    Ok(Field { key: key.to_owned(), prompt, description, gate, shape })
}

/// **Every gate an `allOf` block declares**, keyed by the property it gates.
///
/// Reads `rivet_config::schema::conditional_schema`'s own output structurally
/// — see the module documentation for why this is the writing side's rule
/// read back rather than a second copy of it. A property is gated when it is
/// named in `then.required` (shown when `on == when`) OR in
/// `else.not.anyOf[].required` (shown only when `on == when`, forbidden
/// otherwise) — either membership means the same thing to a form: do not ask
/// and do not write unless the condition holds.
fn extract_gates(all_of: Option<&Json>) -> std::collections::BTreeMap<String, Gate> {
    let mut out = std::collections::BTreeMap::new();
    let Some(Json::Array(items)) = all_of else {
        return out;
    };
    for item in items {
        let Some(Json::Object(if_props)) =
            item.get("if").and_then(|i| i.get("properties"))
        else {
            continue;
        };
        // Exactly one entry in every conditional this schema writes today —
        // `(on, {"const": when})` — but this walks whatever is there rather
        // than assuming the count, so a future multi-key `if` degrades to
        // "every named sibling gates the same way" instead of silently
        // reading only the first.
        for (on, value) in if_props {
            let Some(when) = value.get("const").and_then(Json::as_str) else {
                continue;
            };
            let gate = Gate { on: on.clone(), when: when.to_owned() };
            let then_required = item
                .get("then")
                .and_then(|t| t.get("required"))
                .and_then(Json::as_array)
                .into_iter()
                .flatten();
            let else_forbidden = item
                .get("else")
                .and_then(|e| e.get("not"))
                .and_then(|n| n.get("anyOf"))
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(|clause| clause.get("required"))
                .filter_map(Json::as_array)
                .flatten();
            for key in then_required.chain(else_forbidden) {
                if let Some(k) = key.as_str() {
                    out.insert(k.to_owned(), gate.clone());
                }
            }
        }
    }
    out
}

/// **Whether a gated field's condition is satisfied**, given the answers
/// gathered so far as `(key, value)` pairs — `rivet_config::interview`'s
/// `Question::applies`, the same rule, read by a form instead of an
/// interview.
#[must_use]
pub fn applies(gate: Option<&Gate>, answers: &[(&str, &str)]) -> bool {
    match gate {
        None => true,
        Some(Gate { on, when }) => answers.iter().any(|(k, v)| k == on && v == when),
    }
}

/// **R-164, R-165: the one mismatch a document may carry, made visible and
/// never refused.**
///
/// `mode = "kiosk"` with no `[kiosk]` table is a mode asked for and not
/// configured — tolerated, not an error, and this says so in one sentence a
/// form can show beside the `mode` field. Every other combination —
/// `headless` with a `[kiosk]` table present (kept, unused, per R-164),
/// `kiosk` with the table present, no `mode` at all, or a second mode's own
/// table coexisting per R-165 — has nothing to say here.
#[must_use]
pub fn kiosk_mode_notice(mode: Option<&str>, kiosk_table_present: bool) -> Option<&'static str> {
    (mode == Some("kiosk") && !kiosk_table_present).then_some(
        "`mode` is set to `kiosk`, but this document has no `[kiosk]` table yet — nothing is \
         configured for it. Configure `[kiosk]` below, or change `mode`.",
    )
}

#[cfg(test)]
#[path = "formfields_tests.rs"]
mod tests;
