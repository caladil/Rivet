//! The two ways to read a [`Value`], and nothing else.
//!
//! `ADR-006` §4 is the whole of this module's reason to exist:
//!
//! > **Every command produces structured data. Human-readable text is
//! > *rendered from* that structure, never produced instead of it.**
//!
//! and, one line further down, the sentence that decides the shape:
//!
//! > **`--output json` is not a feature bolted onto a text tool.** If the human
//! > rendering and the structured form can ever disagree, the rendering is
//! > reading from the wrong source and that is a defect.
//!
//! So there is exactly one producer — a [`Value`] — and [`json`] and [`text`]
//! are both *readers of it*. Neither can say anything the other cannot, because
//! neither has access to anything else. That property is asserted by
//! `every_key_in_the_json_is_also_in_the_text`, which is the only kind of test
//! that can hold it.
//!
//! # The value moved to `rivet-core` on 2026-08-23; the renderers did not
//!
//! The enum was declared here when this crate was the only thing that produced
//! a value and `rivet-core` exposed no operation at all (`ADR-006` §8.1). It
//! is `rivet_core::Value` now, because an operation's result is the core's
//! (`ADR-006` §3) and a second adapter declaring its own would be two things
//! that both look authoritative for what an operation returned.
//!
//! **Rendering stayed out of core, and that is `ADR-014` §3's forbidden edge
//! `core → rivet-render` being honoured** rather than a division of
//! convenience. That paragraph used to end *"when `rivet-render` is created
//! (`ADR-014` §2 lists it and it does not exist), these two functions are what
//! moves into it, and nothing in `rivet_core` has to change for that to happen
//! — which is the test of whether the boundary was drawn in the right place."*
//!
//! **It was created on 2026-08-30 and the prediction held: nothing in
//! `rivet-core` changed.** What forced it was `ADR-035` §8A.1.2's break-glass
//! console, which puts an operation endpoint in `cmd/rivet-initd` — so a second
//! binary needed to render a `Value`, and the choices were a second renderer
//! (`ADR-011` forbids), moving these into core (§3 forbids), or building the
//! crate §2 had already named. Two prohibitions pointing at one answer is what
//! a well-drawn boundary looks like from the inside.
//!
//! # Why this is hand-written and not `serde_json`
//!
//! The value enum is **closed**: five variants, no derive, no generic data
//! model, no borrowed lifetimes. `cmd/rivet-sysd` reached the same
//! conclusion for the journal's line-delimited JSON and recorded it in its
//! `Cargo.toml`; `rivet-config`'s records why serde is kept out of this
//! workspace at all. The escaping is the part that is easy to get wrong, so it
//! is the part that is tested against the characters that actually bite.

use std::fmt::Write as _;

pub use rivet_core::Value;

/// The machine form. `ADR-006` §4 makes this the primary one.
///
/// No trailing newline: `main` adds exactly one, so that a caller
/// reading a stream of these gets one object per line and never a blank one.
pub fn json(value: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &Value) {
    match value {
        Value::Str(s) => write_json_string(out, s),
        // RFC 8259 §6 permits any number a reader can represent; an `i64`
        // written in decimal is one, and there is no float here to lose
        // precision on.
        Value::Integer(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            for (i, (key, item)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(out, key);
                out.push(':');
                write_json(out, item);
            }
            out.push('}');
        }
    }
}

/// RFC 8259 §7, and the three parts of it that are usually got wrong.
///
/// The two mandatory escapes (`"` and `\`) are the ones everybody remembers.
/// The others are the ones that produce a JSON document that parses and is
/// wrong: **every** code point below `0x20` must be escaped, not just the five
/// with short forms, and a literal control character inside a string is a parse
/// error in every conforming reader.
///
/// `DEL` (`0x7f`) is deliberately *not* escaped: it is legal unescaped, and
/// escaping it would be inventing a rule.
fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                // `write!` to a String cannot fail; the result is discarded
                // rather than unwrapped because `unwrap`/`expect` are denied in
                // this crate.
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The human form — a *view* of the same value, per `ADR-006` §4.
///
/// Deliberately plain: `key: value` one per line, nested objects indented, lists
/// as `- item`. There is no table, no colour and no column alignment, because
/// inventing a layout for values that do not exist yet would be designing
/// against imagination.
///
/// No trailing newline, for [`json`]'s reason.
pub fn text(value: &Value) -> String {
    let mut out = String::new();
    write_text(&mut out, value, 0);
    // One trailing newline is produced by the last line written; strip it so the
    // two renderers have the same contract and `emit` owns the newline.
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// The one place a scalar becomes a line of text.
///
/// **A number and a boolean render as themselves**, not as prose: `true` rather
/// than "yes", `1` rather than "one". The two renderings may not disagree
/// (`ADR-006` §4), and a human rendering that translated a value into English
/// would be saying something the machine form does not.
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.clone()),
        Value::Integer(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::List(_) | Value::Object(_) => None,
    }
}

fn write_text(out: &mut String, value: &Value, depth: usize) {
    let pad = "  ".repeat(depth);
    if let Some(scalar) = scalar(value) {
        let _ = writeln!(out, "{pad}{scalar}");
        return;
    }
    match value {
        Value::List(items) => {
            if items.is_empty() {
                let _ = writeln!(out, "{pad}(none)");
            }
            for item in items {
                match scalar(item) {
                    Some(s) => {
                        let _ = writeln!(out, "{pad}- {s}");
                    }
                    None => {
                        let _ = writeln!(out, "{pad}-");
                        write_text(out, item, depth + 1);
                    }
                }
            }
        }
        Value::Object(fields) => {
            if fields.is_empty() {
                let _ = writeln!(out, "{pad}(none)");
            }
            for (key, item) in fields {
                match scalar(item) {
                    Some(s) => {
                        let _ = writeln!(out, "{pad}{key}: {s}");
                    }
                    None => {
                        let _ = writeln!(out, "{pad}{key}:");
                        write_text(out, item, depth + 1);
                    }
                }
            }
        }
        // Every scalar was handled above; this arm keeps the match total
        // without an `unreachable!`, which would be a panic in the binary
        // `/etc/shells` names.
        Value::Str(_) | Value::Integer(_) | Value::Bool(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        Value::object(vec![
            ("name", Value::str("rivet")),
            ("version", Value::str("0.0.1")),
            (
                "implements",
                Value::List(vec![Value::str("login-shell"), Value::str("--version")]),
            ),
        ])
    }

    #[test]
    fn an_object_serialises_in_the_order_its_fields_were_given() {
        assert_eq!(
            json(&sample()),
            r#"{"name":"rivet","version":"0.0.1","implements":["login-shell","--version"]}"#
        );
    }

    /// `ADR-015` §4 wants reproducible bytes and this is the cheap half of it:
    /// nothing here may iterate a hash map.
    #[test]
    fn the_same_value_serialises_to_the_same_bytes_every_time() {
        assert_eq!(json(&sample()), json(&sample()));
        assert_eq!(text(&sample()), text(&sample()));
    }

    /// **The property that makes `ADR-006` §4 true rather than intended.** The
    /// human rendering and the machine form must not be able to disagree, and
    /// the only way they cannot is by both reading the same value. If a future
    /// change gives `text` a source of its own, this is what fails.
    #[test]
    fn every_key_and_string_in_the_json_is_also_in_the_text() {
        let t = text(&sample());
        for needle in ["name", "rivet", "version", "0.0.1", "implements"] {
            assert!(t.contains(needle), "the text view dropped {needle}:\n{t}");
        }
    }

    #[test]
    fn the_two_mandatory_escapes_are_applied() {
        assert_eq!(json(&Value::str(r#"a"b"#)), r#""a\"b""#);
        assert_eq!(json(&Value::str(r"a\b")), r#""a\\b""#);
    }

    /// The escapes that produce a document which *parses* and is wrong if they
    /// are missed. A raw control character inside a JSON string is a parse error
    /// in every conforming reader, so this is the case that turns a refusal
    /// message into an unreadable one — and a refusal message is exactly where a
    /// stray control character comes from, since it may quote what the operator
    /// typed.
    #[test]
    fn every_control_character_is_escaped_and_not_only_the_five_with_short_forms() {
        assert_eq!(json(&Value::str("a\nb")), r#""a\nb""#);
        assert_eq!(json(&Value::str("a\tb")), r#""a\tb""#);
        assert_eq!(json(&Value::str("a\rb")), r#""a\rb""#);
        assert_eq!(json(&Value::str("a\u{08}b")), r#""a\bb""#);
        assert_eq!(json(&Value::str("a\u{0c}b")), r#""a\fb""#);
        // The ones with no short form: U+0001 through U+001F less the five
        // above. A naive escaper emits these raw and the document stops
        // parsing in every conforming reader.
        //
        // The expected text is built from a `char` rather than written out as
        // an escape sequence, so that this source file contains no literal
        // control byte for an editor or a tool to silently normalise.
        let esc = '\\';
        assert_eq!(json(&Value::str("a\u{01}b")), format!("\"a{esc}u0001b\""));
        assert_eq!(json(&Value::str("\u{1f}")), format!("\"{esc}u001f\""));
        for c in 0u32..0x20 {
            let raw = char::from_u32(c).unwrap_or('?');
            let out = json(&Value::str(raw.to_string()));
            assert!(
                !out.contains(raw),
                "U+{c:04X} reached the output unescaped: {out:?}"
            );
        }
    }

    /// DEL is legal unescaped. Escaping it would be inventing a rule, which is
    /// the same class of mistake as failing to apply a real one.
    #[test]
    fn del_is_left_alone_because_the_specification_leaves_it_alone() {
        assert_eq!(json(&Value::str("\u{7f}")), "\"\u{7f}\"");
    }

    #[test]
    fn a_non_ascii_string_is_passed_through_rather_than_escaped_to_ascii() {
        assert_eq!(json(&Value::str("µ§")), "\"µ§\"");
    }

    #[test]
    fn an_empty_list_reads_as_none_rather_than_as_a_blank_line() {
        assert_eq!(text(&Value::List(vec![])), "(none)");
        assert_eq!(json(&Value::List(vec![])), "[]");
    }

    #[test]
    fn neither_rendering_ends_in_a_newline_because_emit_owns_that() {
        assert!(!json(&sample()).ends_with('\n'));
        assert!(!text(&sample()).ends_with('\n'));
    }

    #[test]
    fn a_nested_object_is_indented_rather_than_flattened() {
        let v = Value::object(vec![(
            "outer",
            Value::object(vec![("inner", Value::str("x"))]),
        )]);
        assert_eq!(text(&v), "outer:\n  inner: x");
    }

    // -- the two scalars the schema needs -------------------------------------

    /// `rivet schema` emits `"additionalProperties": false` and a `const`.
    /// A boolean quoted as a string would produce a JSON Schema that no
    /// validator reads the way it is meant, and the failure would be invisible
    /// here (`ADR-006` §7's "trusted and wrong").
    #[test]
    fn a_boolean_and_a_number_are_json_literals_rather_than_strings() {
        assert_eq!(json(&Value::Bool(false)), "false");
        assert_eq!(json(&Value::Bool(true)), "true");
        assert_eq!(json(&Value::Integer(1)), "1");
        assert_eq!(json(&Value::Integer(-1)), "-1");
        assert_eq!(json(&Value::Integer(0)), "0");
    }

    /// And the human rendering says the same thing rather than translating it.
    #[test]
    fn the_text_rendering_of_a_scalar_says_what_the_json_says() {
        let v = Value::object(vec![
            ("writable", Value::Bool(false)),
            ("version", Value::Integer(1)),
        ]);
        assert_eq!(text(&v), "writable: false\nversion: 1");
    }

    /// A list of non-strings must not collapse: it was a list in the machine
    /// form and it has to look like one here.
    #[test]
    fn a_list_of_numbers_renders_one_per_line_like_a_list_of_strings() {
        let v = Value::List(vec![Value::Integer(1), Value::Bool(true)]);
        assert_eq!(text(&v), "- 1\n- true");
        assert_eq!(json(&v), "[1,true]");
    }
}
