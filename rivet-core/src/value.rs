//! The structured value every operation produces.
//!
//! `ADR-006` §4 is the whole of this module's reason to exist:
//!
//! > **Every command produces structured data. Human-readable text is
//! > *rendered from* that structure, never produced instead of it.**
//!
//! and, one line further down, the sentence that decides where it lives:
//!
//! > **`--output json` is not a feature bolted onto a text tool.** If the human
//! > rendering and the structured form can ever disagree, the rendering is
//! > reading from the wrong source and that is a defect.
//!
//! Two renderings cannot disagree if neither has a source of its own, and the
//! only way to guarantee that across *several* adapters is for the source to be
//! upstream of all of them. `ADR-006` §3 puts it there in as many words —
//! "every administrative action is a typed request producing a typed result" —
//! so the result is the core's and rendering it is the adapter's.
//!
//! # It was in `cmd/rivet` until 2026-08-23
//!
//! This enum was written in `cmd/rivet/src/value.rs` alongside the two
//! renderers, when the CLI was the only thing that produced a value and
//! `rivet-core` exposed no operation at all (`ADR-006` §8.1). It moved here
//! with the first operation, because a second adapter declaring its own value
//! type is `ADR-011` §3 exactly: two things that both look authoritative for
//! what an operation returned.
//!
//! **The renderers did not move.** `json` and `text` are in `cmd/rivet`,
//! because `ADR-014` §3 forbids `core → rivet-render` and a renderer compiled
//! into the core would be that edge with the crate boundary left off.
//!
//! # Why it is closed, and hand-serialised at the edge
//!
//! Five variants, no derive, no generic data model, no borrowed lifetimes.
//! `cmd/rivet-sysd` reached the same conclusion for the journal's
//! line-delimited JSON and recorded it in its `Cargo.toml`; `rivet-config`'s
//! records why serde is kept out of this workspace at all. Here the argument is
//! sharper still: `rivet-core`'s `[dependencies]` table is empty, and that
//! emptiness is what makes `ADR-014` §3's forbidden edges fail to compile.
//!
//! # What is deliberately not here
//!
//! **Floating point, and a null.** Nothing this workspace produces is either.
//! A `Value::Null` in particular would give a caller two ways to say "no value"
//! — the null and the absent key — which is `ADR-011` §3 in miniature; the
//! convention already established by `cmd/rivet`'s refusals is that an absent
//! field is omitted rather than emitted as null, and
//! `a_refusal_with_no_subject_omits_the_field_rather_than_emitting_null` is the
//! test that holds it.
//!
//! **A dynamic object key.** [`Value::Object`] keys are `&'static str`, which is
//! every key anything produces today: an operation's declared outputs
//! (`crate::operation`) and the configuration schema's declared keys
//! (`rivet_config::schema`) are both fixed lists. The day an operation returns
//! a value keyed by something the operator named — `get network.eth0` returning
//! one entry per interface — this widens to `String`. It is left narrow until
//! then rather than widened speculatively, because the narrow form is what makes
//! `ADR-015` §4's reproducible bytes free: there is nothing to sort and no map
//! to iterate.

/// One structured value. Closed on purpose — see the module documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    /// A whole number. `i64` rather than `u64` because the one number the
    /// schema emits today is a schema version and the one an operation emits is
    /// a file mode, and a type that cannot represent a negative number would be
    /// a second decision — about what a number *is* — smuggled in beside the
    /// representation.
    Integer(i64),
    Bool(bool),
    List(Vec<Value>),
    /// Ordered, because the order is part of what a human reads and a `BTreeMap`
    /// would sort the keys into an order nobody chose. `ADR-015` §4 also wants
    /// the same inputs to produce the same bytes, and a `HashMap` here would
    /// quietly cost that.
    Object(Vec<(&'static str, Value)>),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }

    pub fn object(fields: Vec<(&'static str, Value)>) -> Self {
        Self::Object(fields)
    }

    /// A list of strings, which is the shape most catalogued outputs take.
    pub fn strings<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::List(items.into_iter().map(Self::str).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_of_strings_is_the_same_value_however_it_was_built() {
        assert_eq!(
            Value::strings(["a", "b"]),
            Value::List(vec![Value::str("a"), Value::str("b")])
        );
    }

    #[test]
    fn an_empty_list_of_strings_is_an_empty_list_and_not_an_absent_one() {
        assert_eq!(Value::strings(Vec::<&str>::new()), Value::List(Vec::new()));
    }

    /// The property `ADR-015` §4 wants and this enum makes free: no map is
    /// iterated and nothing is sorted, so an equal value is equal every time.
    #[test]
    fn the_same_fields_in_the_same_order_are_the_same_value() {
        let build = || {
            Value::object(vec![
                ("name", Value::str("rivet")),
                ("mode", Value::Integer(0o644)),
                ("writable", Value::Bool(false)),
            ])
        };
        assert_eq!(build(), build());
    }

    /// And the order is *part of* the value, rather than something a renderer
    /// chooses — which is what stops two adapters printing the same object two
    /// ways.
    #[test]
    fn two_objects_with_the_same_fields_in_a_different_order_are_different_values() {
        let a = Value::object(vec![("a", Value::Bool(true)), ("b", Value::Bool(false))]);
        let b = Value::object(vec![("b", Value::Bool(false)), ("a", Value::Bool(true))]);
        assert_ne!(a, b);
    }
}
