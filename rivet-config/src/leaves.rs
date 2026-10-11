//! **A document's text, as leaves** — the reader for [`rivet_core::leaf`].
//!
//! The types and the comparison live in `rivet-core`, which depends on nothing
//! that touches the world and therefore cannot parse TOML. This is the half
//! that needs `toml_edit`, in the crate that already carries it and already
//! owns the schema.
//!
//! # It reads the TEXT and not the schema, deliberately
//!
//! [`crate::Document::parse`] is the authority on whether a document is *valid*
//! and every caller of this should run it too — that is where an unknown key,
//! and therefore a credential, is refused (`ADR-007` §7.3). But a valid
//! document parsed into typed values has lost the thing a diff needs: the
//! **path** each value came from. `[services.rivet-mcpd] enabled = true` has to
//! be reportable as `services.rivet-mcpd.enabled`, by name, for a refusal to be
//! worth reading.
//!
//! So this is a second pass over the same bytes with a different question, and
//! it is not a second parser: `toml_edit` is the same one this crate's schema
//! parser uses.
//!
//! Moved here from `cmd/rivet-fleet/src/diff.rs` on 2026-09-05, when `ADR-044`
//! gave it a second caller.

use rivet_core::leaf::{Leaf, Leaves, Scalar};
use toml_edit::{Item, Value};

/// Read a TOML document into its leaves.
///
/// # An empty table is a leaf, and it has to be
///
/// `[accounts.deploy.keys]` with nothing under it is an account nobody can log
/// into, which `crate::serialise` renders deliberately and which the machine's
/// parser reads as a real declaration. Flattened as *no leaves at all* it would
/// be invisible to a diff, and applying a document that dropped the account
/// would look like a no-op. So a table with no entries becomes
/// `Leaf::Many(vec![])` at its own path.
///
/// # Errors
///
/// `toml_edit`'s own words when the text is not TOML.
pub fn leaves(text: &str) -> Result<Leaves, String> {
    let document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("this is not TOML: {e}"))?;
    let mut out = Leaves::new();
    let mut path = Vec::new();
    table(&mut path, document.as_table(), &mut out);
    Ok(out)
}

fn table(path: &mut Vec<String>, source: &toml_edit::Table, out: &mut Leaves) {
    if source.is_empty() && !path.is_empty() {
        out.insert(path.clone(), Leaf::Many(Vec::new()));
        return;
    }
    for (name, item) in source.iter() {
        path.push(name.to_owned());
        entry(path, item, out);
        path.pop();
    }
}

fn entry(path: &mut Vec<String>, item: &Item, out: &mut Leaves) {
    match item {
        // `Item::None` is a key `toml_edit` holds a place for and nothing
        // occupies. It is not a value on the machine, so it is not a loss.
        Item::None => {}
        Item::Table(inner) => table(path, inner, out),
        Item::ArrayOfTables(array) => {
            // Nothing `rivet-config` renders uses `[[x]]`, so this arm is here
            // for a document written by hand rather than for the canonical
            // form. Indexed, because position is the only identity an element
            // of one has.
            for (index, inner) in array.iter().enumerate() {
                path.push(format!("[{index}]"));
                table(path, inner, out);
                path.pop();
            }
        }
        Item::Value(value) => scalar_or_group(path, value, out),
    }
}

fn scalar_or_group(path: &mut Vec<String>, value: &Value, out: &mut Leaves) {
    match value {
        Value::InlineTable(inner) => {
            if inner.is_empty() {
                out.insert(path.clone(), Leaf::Many(Vec::new()));
                return;
            }
            for (name, item) in inner.iter() {
                path.push(name.to_owned());
                scalar_or_group(path, item, out);
                path.pop();
            }
        }
        Value::Array(array) => {
            if array
                .iter()
                .all(|element| !element.is_array() && !element.is_inline_table())
            {
                out.insert(path.clone(), Leaf::Many(array.iter().map(typed).collect()));
            } else {
                // A nested or table-valued array. Nothing in this schema has
                // one; rendering it whole compares it as a single value rather
                // than inventing an identity for its elements, which is the
                // honest answer for a shape nobody has ruled on.
                //
                // **Bare, not string** — `rendered` is never what a TOML
                // string contains verbatim, so it is never the side of the
                // bare-versus-quoted collision that must keep
                // [`rivet_core::leaf::Kind::String`].
                //
                // A nested or table-valued array compares by its whole
                // rendered text, so the TOML kinds of its OWN elements are
                // not distinguished from one another inside that one
                // rendered value — see `rendered`.
                out.insert(path.clone(), Leaf::One(Scalar::bare(rendered(value))));
            }
        }
        other => {
            out.insert(path.clone(), Leaf::One(typed(other)));
        }
    }
}

/// [`scalar`]'s text, plus the TOML kind [`rivet_core::leaf::Leaf`] equality
/// needs to tell `60` from `"60"`. Kept apart from `scalar` because
/// `rendered`'s own recursion needs the text alone: a nested value's kind is
/// not tracked at all (see the caller above).
fn typed(value: &Value) -> Scalar {
    if matches!(value, Value::String(_)) {
        Scalar::string(scalar(value))
    } else {
        Scalar::bare(scalar(value))
    }
}

/// A scalar as an operator would type it — **unquoted**, because these values
/// go into command lines a refusal composes.
fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.value().clone(),
        Value::Boolean(flag) => flag.value().to_string(),
        Value::Integer(number) => number.value().to_string(),
        Value::Float(number) => number.value().to_string(),
        Value::Datetime(stamp) => stamp.value().to_string(),
        container @ (Value::Array(_) | Value::InlineTable(_)) => rendered(container),
    }
}

/// A container as one comparable string.
///
/// # It is written out rather than taken from `toml_edit`, and the reason is a
/// FEATURE and not a preference
///
/// This module's first home — `cmd/rivet-fleet` — spelled both of these
/// `value.to_string()`, which is `toml_edit`'s `Display`. **`rivet-config`
/// takes `toml_edit` with `default-features = false, features = ["parse"]`**,
/// and its manifest gives the reason: `display` preserves the layout of the
/// document it parsed, which is the opposite of what `ADR-007` §3.1 asks of a
/// canonical serialiser, so this crate emits its own form and keeps the writer
/// out of the build.
///
/// The trap that makes this worth a paragraph: **cargo unifies features**, so
/// the moved code compiled perfectly in any build that also included
/// `cmd/rivet-fleet` — which is every workspace build and therefore
/// `tools/test-all.sh`. `cargo test -p rivetos-config --lib` on its own is what
/// found it. A test suite that only ever runs the whole workspace cannot see
/// this class of defect at all.
///
/// The output is not `toml_edit`'s and does not need to be. Nothing parses it:
/// it exists so that two of these compare equal when they say the same thing,
/// and so that a report can print one.
fn rendered(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let inside: Vec<String> = items.iter().map(rendered).collect();
            format!("[{}]", inside.join(", "))
        }
        Value::InlineTable(table) => {
            // **In the document's own order**, like every other path here. A
            // sorted rendering would make two differently-written tables
            // compare equal, which is a judgement this function has no
            // business making on a shape the schema does not have.
            let inside: Vec<String> = table
                .iter()
                .map(|(key, item)| format!("{key} = {}", rendered(item)))
                .collect();
            format!("{{{}}}", inside.join(", "))
        }
        plain => scalar(plain),
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::leaf::{Scalar, between};

    /// **The board's own document, as it really is** — captured over the
    /// control plane from `rivet-01` on 2026-09-05, not composed here.
    /// `AGENTS.md` §4: a vector captured from real hardware is worth more than
    /// a hand-written one, and the difference must be visible in the file.
    ///
    /// It is kept for one property in particular: it is **not** canonically
    /// serialised, because every `apply edit --append` that has ever run
    /// against it has left its own formatting. Ten consecutive blank lines and
    /// two tables with no blank line between them are in this text on purpose.
    const BOARD: &str = "\
# Rivet's configuration document. Written by Rivet and canonically
# serialised (ADR-007 3.1); change it with the rivet tools, not an editor.
version = 1
hostname = \"rivet-01\"
timezone = \"America/Chicago\"

[services.rivet-mcpd]
enabled = true

[network.eth0]
method = \"static\"
address = \"192.168.50.66/24\"
gateway = \"192.168.50.1\"

[control-plane]
listen = \"0.0.0.0:7777\"

[services.aura-renderer]
enabled = true
[kiosk]
auto-run = \"aura-renderer\"
[network.resolver]
nameservers = [\"192.168.50.1\", \"1.1.1.1\"]




[services.rivet-fleetd]
enabled = true
";

    fn path(of: &str) -> Vec<String> {
        of.split('.').map(str::to_owned).collect()
    }

    /// Every nesting the real document uses reaches a leaf at the path a
    /// person would name it by.
    #[test]
    fn the_boards_own_document_reads_into_the_paths_an_operator_would_name() {
        let found = leaves(BOARD).expect("the board's document is TOML");
        assert_eq!(found.get(&path("hostname")), Some(&Leaf::One(Scalar::string("rivet-01"))));
        assert_eq!(
            found.get(&path("services.rivet-mcpd.enabled")),
            Some(&Leaf::One(Scalar::bare("true")))
        );
        assert_eq!(
            found.get(&path("control-plane.listen")),
            Some(&Leaf::One(Scalar::string("0.0.0.0:7777")))
        );
        assert_eq!(
            found.get(&path("network.resolver.nameservers")),
            Some(&Leaf::Many(vec![
                Scalar::string("192.168.50.1"),
                Scalar::string("1.1.1.1")
            ]))
        );
    }

    /// **Comments and blank lines are not values**, which is the property a
    /// poller's quietness rests on: the machine's file and a tidied copy of it
    /// compare equal, so nothing is re-written every cycle.
    #[test]
    fn formatting_is_not_a_difference() {
        let tidied = BOARD
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(tidied, BOARD, "the two texts must really differ");
        let a = leaves(BOARD).expect("board");
        let b = leaves(&tidied).expect("tidied");
        assert!(
            between(&a, &b).is_empty() && between(&b, &a).is_empty(),
            "a comment or a blank line was read as a value"
        );
    }

    /// A declared, empty table survives as a leaf — an account with no keys is
    /// a revocation, and losing it silently is the failure this guards.
    #[test]
    fn an_empty_table_is_a_leaf_of_its_own() {
        let found = leaves("version = 1\n[accounts.deploy.keys]\n").expect("TOML");
        assert_eq!(
            found.get(&path("accounts.deploy.keys")),
            Some(&Leaf::Many(Vec::new()))
        );
    }

    /// A quoted key containing a dot keeps its own segment, which is why the
    /// map is keyed by a path and not by a joined string.
    #[test]
    fn a_key_with_a_dot_in_it_is_one_segment() {
        let found = leaves("version = 1\n[services.\"a.b\"]\nenabled = true\n").expect("TOML");
        let expected: Vec<String> =
            vec!["services".to_owned(), "a.b".to_owned(), "enabled".to_owned()];
        assert_eq!(found.get(&expected), Some(&Leaf::One(Scalar::bare("true"))));
    }

    /// **A shape the schema does not have, rendered rather than dropped.**
    /// Nothing `rivet-config` emits uses an array of arrays or an array of
    /// tables, so this is about a document somebody wrote by hand — and a leaf
    /// that vanished from the reader would read as *this value is about to be
    /// removed* in one direction and as nothing at all in the other.
    ///
    /// It also compiles without `toml_edit`'s `display` feature, which is what
    /// this crate takes it without. See [`super::rendered`].
    #[test]
    fn a_nested_array_is_one_comparable_value() {
        let found = leaves("version = 1\nodd = [[1, 2], [3]]\n").expect("TOML");
        assert_eq!(
            found.get(&path("odd")),
            Some(&Leaf::One(Scalar::bare("[[1, 2], [3]]")))
        );
        let same = leaves("version = 1\nodd = [ [ 1,2 ],[ 3 ] ]\n").expect("TOML");
        assert!(
            between(&found, &same).is_empty(),
            "two spellings of one value compared unequal"
        );
    }

    /// Text that is not TOML is a refusal carrying what the parser said, never
    /// an empty set of leaves — an empty one would read as *every value on the
    /// machine is about to be removed*.
    #[test]
    fn text_that_is_not_toml_refuses_rather_than_reading_as_empty() {
        let why = leaves("this is not = = TOML\n").expect_err("refused");
        assert!(why.contains("not TOML"), "{why}");
    }
}
