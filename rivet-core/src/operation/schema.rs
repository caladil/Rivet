//! `rivet get schema` — **the document this machine will accept.**
//!
//! `ADR-006` §7's self-description. It was `rivet schema`, a bare word beside
//! `--help` and `--version`, until the maintainer ruled it a subject of `get` on
//! 2026-08-25; [`crate::operation::machine::Observed::configuration_schema`]
//! carries the whole argument and the two things that decided it — that the old
//! spelling had **no verb in it**, and that it was **undiscoverable from the
//! catalogue** the machine publishes about itself.
//!
//! # It is the other half of `explain`
//!
//! `ADR-007` §3.6 splits a Rivet's files in two and this verb pair is that
//! split made askable:
//!
//! | | |
//! | :-- | :-- |
//! | `rivet explain <path>` | *this file is written FOR you — here is from what, and what to change instead* |
//! | `rivet get schema` | *this document is YOURS to write — here is what may be in it* |
//!
//! An operator who has both never has to guess which kind of file they are
//! looking at, which is the confusion `ADR-007` §3.6.2 exists to remove.
//!
//! # The result is an object, and the schema is inside it
//!
//! `rivet schema` emitted the JSON Schema document **bare**. This emits
//! `{"subject": "schema", "schema": { … }}`, and the wrapper is not
//! decoration: every other result on this surface is an object whose keys the
//! catalogue declares, and a subject that answered with a foreign document
//! would be exactly the sort of exception this move existed to delete.
//!
//! **It is a wire change and it is stated rather than discovered**: anything
//! reading the old form wants `| get schema` on the end now.

use super::{ErrorCode, Field, OperationError};
use crate::operation::machine::Observed;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "schema";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `schema` in this build.",
    },
    Field {
        name: "schema",
        description: "The JSON Schema of THIS MACHINE's desired-state document — the thing an \
                      operator writes, at /state/config and on the boot partition for a first \
                      boot (ADR-004 §6). It carries its own dialect and its own version in \
                      `$schema` and `title`, so it needs no second spelling of either here. \
                      ASK THE MACHINE rather than assuming: on a fleet with A/B slots two \
                      boards can run different builds and accept different documents, which is \
                      why this is a fact read off a machine and not a property of the tool.",
    },
];

/// Perform it. Nothing can fail — see the port.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    // **A second word is refused, never ignored.** `storage` set this
    // precedent and the reason is the same: an operator who typed
    // `get schema network` meant something, and answering the whole document
    // as though they had not asked is the shape `AGENTS.md` §5 refuses. There
    // is no per-section selector today; if one is wanted it is a new input.
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole document is \
                 the answer; select part of it with a JSON tool rather than with an argument \
                 this build does not have."
            ),
        ));
    }

    Ok(Value::object(vec![
        ("subject", Value::str(SUBJECT)),
        ("schema", observed.configuration_schema()),
    ]))
}
