//! `rivet get document` — **the desired-state document this machine is running
//! from.**
//!
//! `ADR-035` phase 4's read surface, and the first subject of it. `PLAN.md` §90.3
//! called it *"probably the highest-value missing subject"*, for a reason that is
//! not about completeness: it is **the read half of `apply edit`**, and its
//! absence is load-bearing today.
//!
//! # The asymmetry this closes
//!
//! [`crate::operation::edit`] says it against itself:
//!
//! > **No reading.** `apply edit` does not print the file. That is `get`'s verb
//! > and no subject of it reads a file yet … an agent can change a file it
//! > cannot read back.
//!
//! That is not a tidiness complaint. `apply edit` is safe *because* it refuses
//! unless `--expect` matches exactly once — which requires the caller to state
//! what it believes is there and be correct. **A caller that cannot read the
//! file cannot form that belief**, so it either guesses or reaches for a shell.
//! The safety primitive and the missing read verb were pulling against each
//! other.
//!
//! It is also why `cargo rivet deploy --enable` refuses over `ADR-035`'s control
//! plane and says so by name: `--enable` appends `[services.<name>] enabled =
//! true` to the document, and it will not do that without reading first.
//!
//! # Why this is not `get file <path>`
//!
//! A general file reader is a different operation with a different refusal set —
//! it has to decide about `/state/secrets`, about device nodes, about a path
//! that is a directory, and about how much of a large file it will return. **This
//! subject reads exactly one file whose path this build knows**, so it needs none
//! of those decisions.
//!
//! `get file` is worth building and is on `docs/design-candidates/`'s list. It is
//! not a superset of this one: an operator asking *what is this machine
//! configured to be* should not have to know where the document lives, and a
//! subject that answers it is discoverable from the catalogue in a way that a
//! path argument never is.
//!
//! # It is safe to return whole, and that is a property rather than a hope
//!
//! The document is the **operator's** file — `ADR-007` §3.6 splits Rivet's files
//! in two and this is the half that is written *by* a human rather than *for*
//! one. Secrets are not in it: `ADR-020` §9 puts them under `/state/secrets`,
//! and the schema has no field that could hold one — `rivet-config`'s
//! `a_document_has_nowhere_to_put_a_secret_so_none_is_rendered` asserts that at
//! the end which writes the bytes, so it is a property of the type rather than
//! a habit of the operator.
//!
//! So this returns the bytes as they are, with no redaction pass. **A redaction
//! pass would be the more dangerous design**: it would have to know every shape
//! a secret could take in a document whose schema grows, and the failure mode of
//! getting that wrong is silent.

use super::{ErrorCode, Field, OperationError};
use crate::operation::machine::Observed;
use crate::state::MACHINE_DOCUMENT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "desired-state";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `document` in this build.",
    },
    Field {
        name: "path",
        description: "Where this machine keeps its desired-state document. Reported rather \
                      than assumed, because a caller that hard-codes it is a second authority \
                      for a fact the machine owns (ADR-011).",
    },
    Field {
        name: "present",
        description: "Whether there is a document at all. FALSE IS AN ORDINARY ANSWER and not \
                      an error: a machine boots from the provisioning document on the boot \
                      partition and writes /state/config only when it has something to write \
                      (ADR-004 §6, ADR-019 §3.1), so a first boot legitimately has none.",
    },
    Field {
        name: "contents",
        description: "The document, verbatim, as TOML text. EMPTY when `present` is false: \
                      this value model has no null, so `present` is what separates \"there is \
                      no document\" from \"the document is empty\", and a real machine \
                      reaches both. NOT PARSED AND NOT REDACTED — it is the bytes on disk, so \
                      a caller composing `apply edit --expect` matches against exactly what \
                      the file says. A parsed round-trip would normalise whitespace and drop \
                      comments, and `--expect` would then fail against text this verb had \
                      just shown the caller.",
    },
    Field {
        name: "bytes",
        description: "The document's length. Zero when it is absent, which is distinguishable \
                      from absent by `present`.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or if the document is there and cannot be read —
/// which includes it not being UTF-8, because the adapter reads it as text.
/// is not UTF-8.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    // A second word is refused, never ignored — `schema`'s precedent and
    // `AGENTS.md` §5. An operator who typed `get document services` meant
    // something, and there is no per-section selector in this build.
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole document is \
                 the answer; select part of it with a JSON tool rather than with an argument \
                 this build does not have. `get schema` says what may be in it."
            ),
        ));
    }

    // **A missing file is `Ok` with no contents**, which is the port's own rule
    // and the right one here: a first boot has no `/state/config` document and
    // reporting that as an error would make an ordinary state look like a fault.
    let read = observed.read_desired_state().map_err(|e| {
        OperationError::about(
            ErrorCode::MachineRefused,
            SUBJECT,
            format!(
                "{MACHINE_DOCUMENT} exists and could not be read: {e}. This is NOT the \
                 same as there being no document — that answers `present: false`. A file \
                 that is there and unreadable is a fault worth reporting as one."
            ),
        )
    })?;

    let Some(text) = read else {
        return Ok(Value::object(vec![
            ("subject", Value::str(SUBJECT)),
            ("path", Value::str(MACHINE_DOCUMENT)),
            ("present", Value::Bool(false)),
            ("contents", Value::str("")),
            ("bytes", Value::Integer(0)),
        ]));
    };

    let length = i64::try_from(text.len()).unwrap_or(i64::MAX);

    Ok(Value::object(vec![
        ("subject", Value::str(SUBJECT)),
        ("path", Value::str(MACHINE_DOCUMENT)),
        ("present", Value::Bool(true)),
        ("contents", Value::str(text)),
        ("bytes", Value::Integer(length)),
    ]))
}

#[cfg(test)]
#[path = "desired_state_tests.rs"]
mod tests;
