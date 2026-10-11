//! **The internal handlers: what the login shell answers to that is not a verb.**
//!
//! `ADR-032` §4.2, ruled 2026-08-25 and built 2026-08-26 (`PLAN.md` §28.5 Phase 1
//! item 5, §32).
//!
//! # What was ruled, and it is not one more bare word
//!
//! > **For `root`: a declared table of internal handlers.** `rivet mcp` is a
//! > **handler the login shell runs itself**, so `ADR-005` §5.4's *"it executes
//! > nothing"* is untouched. What was ruled is not one more bare word but the
//! > **table**: `schema` and `measure` become the first two entries of a declared
//! > set, and `mcp` is the third.
//! >
//! > The reason is `AGENTS.md` §3's, applied to a pattern rather than a
//! > boundary: the fourth special case is where somebody adds a fifth without
//! > noticing it has become a pattern.
//!
//! # The table ships with ONE entry, and the ruling named two
//!
//! Said plainly rather than quietly built as a one-row table: **`schema` was
//! retired the same day §4.2 was written.** `ADR-006` §7.4 made it
//! `rivet get schema` — a subject of a verb, where the operation catalogue can
//! see it — so of the two entries §4.2 names, one is no longer a bare word at
//! all and `crate::cli` refuses its old spelling by name.
//!
//! [`MCP`] is not here either, because it does not exist. `AGENTS.md` §3: a
//! stub refuses and never fakes success, and a table entry for a handler that
//! answered *unimplemented* would be a published claim about a surface this
//! build does not have.
//!
//! # What one entry still buys, which is the reason to build it anyway
//!
//! **`measure` is invisible today.** `rivet --version` publishes
//! `rivet_core::operation::CATALOGUE`, which is keyed by `ADR-006` §6.4's verbs —
//! and `measure` is not a verb. So an agent reading everything this machine says
//! about itself finds no mention of a word the login shell answers to, which is
//! precisely the failure `ADR-006` §7 exists to prevent:
//!
//! > If they are hand-maintained documents, they will be wrong within a month
//! > and worse than absent, because they will be trusted.
//!
//! A catalogue that is *silently incomplete* is the same defect with a quieter
//! failure. [`value`] is published beside the operations, and
//! `every_handler_is_reachable_and_every_reachable_word_is_listed` is what makes
//! the two agree.

use rivet_core::Value;

pub use crate::measure::MEASURE;

/// `ADR-032` §4.2's `rivet mcp` — **built 2026-08-27.**
///
/// It was declared here as a name and kept out of [`HANDLERS`] from 2026-08-25,
/// *"so that the day it is built the table is the only thing that has to
/// change and the word is already spelled once."* That day came, and the table
/// was the only thing that changed.
pub const MCP: &str = "mcp";

/// One word the login shell answers to itself.
///
/// **Not an operation** — an operation is `rivet-core`'s (`ADR-006` §3) and is
/// reachable from every adapter. A handler is this binary's, reachable only
/// through the login shell, and the distinction is the one `crate::refusal`
/// already draws for error codes: *an operation's failures are catalogued in the
/// core; an adapter's own invocation contract is catalogued by that adapter.*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handler {
    /// The word an operator types. Part of the interface contract.
    pub name: &'static str,
    pub summary: &'static str,
    /// What follows it, in the shape the catalogue's `inputs` use — so an agent
    /// reading this and reading an operation is reading one vocabulary.
    pub takes: &'static str,
    /// **Why this is not a verb**, in a sentence.
    ///
    /// A field rather than a comment because it is the question an agent will
    /// have: `ADR-006` §6.4's verb set is closed and a word outside it needs a
    /// reason, not an exemption. Publishing the reason is what stops the table
    /// becoming the place special cases go to be legitimised.
    pub why_not_a_verb: &'static str,
}

/// **Every word this login shell answers to that is not a verb.**
///
/// Two entries. See the module documentation for why that is fewer than
/// `ADR-032` §4.2 names and why it is still worth being a table.
///
/// **Each entry is declared by the module that implements it**, never here —
/// `rivet_core::operation::CATALOGUE`'s rule, and this is its adapter-side
/// counterpart. So a handler's description cannot drift from the handler,
/// because there is nowhere for it to drift *to*.
pub const HANDLERS: &[Handler] = &[crate::measure::HANDLER, crate::mcp::HANDLER];

/// The handler a word names, if this build has one.
///
/// The one place a word becomes a handler. `crate::cli` dispatches through it
/// rather than matching the word itself, which is what makes the table the
/// authority instead of a description of a `match` somewhere else.
pub fn parse(word: &str) -> Option<&'static Handler> {
    HANDLERS.iter().find(|h| h.name == word)
}

/// The table as the value `rivet --version` publishes, beside the operations.
pub fn value() -> Value {
    Value::List(
        HANDLERS
            .iter()
            .map(|h| {
                Value::object(vec![
                    ("name", Value::str(h.name)),
                    ("summary", Value::str(h.summary)),
                    ("takes", Value::str(h.takes)),
                    ("why-not-a-verb", Value::str(h.why_not_a_verb)),
                ])
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::cli::{Request, parse as parse_args};

    /// **The anti-drift property, and it is the reason §4.2 ruled a table.**
    ///
    /// Every listed word reaches a handler, and every word that reaches one is
    /// listed. `rivet_core::operation::CATALOGUE` has the same pair of
    /// properties for the same reason; this is the adapter's half.
    #[test]
    fn every_handler_is_reachable_and_every_reachable_word_is_listed() {
        for handler in HANDLERS {
            let parsed = parse_args(&[handler.name.to_string()])
                .unwrap_or_else(|e| panic!("{} is listed and not reachable: {e:?}", handler.name));
            match parsed.request {
                Request::Handler { name, .. } => assert_eq!(
                    name, handler.name,
                    "{} parsed as a different handler",
                    handler.name
                ),
                other => panic!("{} did not parse as a handler: {other:?}", handler.name),
            }
        }
        // And the reverse: a word that is not listed is not a handler.
        //
        // **`mcp` used to be asserted ABSENT here**, because it was declared as
        // a name and deliberately kept out of the table — the assertion made
        // the absence real rather than letting the constant look like it was
        // enough. It was built on 2026-08-27 and moved into `HANDLERS`, so the
        // loop above now covers it and this line would be false.
        assert!(parse("schema").is_none(), "schema is a subject of `get`");
        assert!(parse("get").is_none(), "a verb is not a handler");
    }

    /// Every entry says what it is, what it takes, and why it is not a verb.
    /// A table whose rows do not answer those is a list of exemptions.
    #[test]
    fn every_handler_publishes_what_it_is_and_why_it_is_not_a_verb() {
        for handler in HANDLERS {
            assert!(!handler.summary.is_empty(), "{} has no summary", handler.name);
            assert!(!handler.takes.is_empty(), "{} says nothing about its words", handler.name);
            assert!(
                !handler.why_not_a_verb.is_empty(),
                "{} is outside ADR-006 §6.4's closed set and gives no reason",
                handler.name
            );
        }
    }

    /// The published form carries every row, so an agent reading
    /// `rivet --version` learns what the login shell answers to.
    #[test]
    fn the_published_table_names_every_handler() {
        let Value::List(rows) = value() else {
            panic!("the table is a list");
        };
        assert_eq!(rows.len(), HANDLERS.len());
        for (row, handler) in rows.iter().zip(HANDLERS) {
            let Value::Object(fields) = row else {
                panic!("a row is an object");
            };
            assert_eq!(
                fields.iter().find(|(k, _)| *k == "name").map(|(_, v)| v),
                Some(&Value::str(handler.name))
            );
        }
    }
}
