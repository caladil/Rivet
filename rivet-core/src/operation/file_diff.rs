//! `rivet get diff <path> <text>` — **what would change if this text were
//! written here.**
//!
//! `ADR-006` §8.10.6, ruled by the maintainer, Joel Lindell, 2026-08-29. The
//! read-side answer to the question `apply edit`'s `--expect` otherwise makes a
//! caller guess at: an agent that can diff before it writes does not have to be
//! lucky.
//!
//! # Why it is its own subject rather than a flag on `get file`
//!
//! `rivet get file` answers *what is at this path*. This answers *what would
//! change*, which is a different question with a different shape of answer and
//! different refusals — a directory is a perfectly good answer to the first and
//! a refusal to the second. `ADR-006` §6.4's one-spelling rule cuts the other
//! way too: a `get file` that changed shape when a second word appeared would
//! be two operations wearing one name.
//!
//! **It is a READ.** Nothing is written, no confirmation is taken, and the
//! shorter gate of §8.10.4 applies — the path need not be under the permitted
//! root, because diffing `/system/bin/rivet-sysd` against a text is a strange
//! thing to want and not a dangerous one.
//!
//! # An absent file is diffed against empty, not refused
//!
//! `present: false`, and every line of the supplied text is an addition. That is
//! exactly what `apply create-file` would write, so the answer is *useful* where
//! a refusal would only have been *correct*.
//!
//! # What it refuses, and each is the same kind of refusal
//!
//! | Refused | Because |
//! | :-- | :-- |
//! | a **directory**, a **symbolic link**, or anything that is not a regular file | there is nothing to compare. A link is not followed (§8.10.4.3) |
//! | a file that is **not UTF-8** | a byte diff is not a diff a person reads, and rendering one as text is the encoding guess `AGENTS.md` §5 forbids |
//! | a file **larger than `super::edit::FILE_MAX_BYTES`** | it reads the file whole, in the login shell, on a board with no swap |
//! | text **larger than `super::edit::TEXT_MAX_BYTES`** | `apply edit`'s cap, and the same argument: a change that size is a file rather than an edit |

use super::diff;
use super::machine::{Editor, PathKind};
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The subject. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "diff";

/// How this subject is named in a refusal — the words an operator typed.
const COMMAND: &str = "get diff";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `diff` in this build.",
    },
    Field {
        name: "path",
        description: "The file compared against, exactly as it was given.",
    },
    Field {
        name: "present",
        description: "Whether there is a file at this path. FALSE IS NOT AN ERROR: the diff is \
                      then against an empty file, so every line of the supplied text is an \
                      addition — which is exactly what `rivet apply create-file` would write.",
    },
    Field {
        name: "changed",
        description: "Whether the file and the supplied text differ at all. FALSE means writing \
                      this text would change nothing, which `apply edit` refuses to do.",
    },
    Field {
        name: "added_lines",
        description: "Lines in the supplied text and not in the file. THIS IS THE WHOLE COUNT \
                      even when `truncated` is true.",
    },
    Field {
        name: "removed_lines",
        description: "Lines in the file and not in the supplied text. See `added_lines`.",
    },
    Field {
        name: "truncated",
        description: "Whether `diff` was cut short. A cut diff is NOT the whole change and this \
                      is how to tell; the two counts above still are.",
    },
    Field {
        name: "algorithm",
        description: "`lines` when the difference was lined up minimally, or `coarse` when it \
                      was too large to and became one hunk of everything removed and everything \
                      added. Reported rather than hidden, because a coarse diff is honest about \
                      WHAT changed and not about WHERE.",
    },
    Field {
        name: "diff",
        description: "The unified diff. `---` is the file on this machine and `+++` is the text \
                      you supplied. EMPTY when nothing changed.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle, and what to run to make the \
                      change it describes.",
    },
];

/// Perform `get diff`.
///
/// # Errors
///
/// A path that is not absolute or is otherwise malformed; a missing or extra
/// word; a path that is not a regular file; a file that is too large or is not
/// text; text that is too large; or a machine that could not answer.
pub(super) fn perform(
    arguments: &[&str],
    editor: &mut dyn Editor,
) -> Result<Value, OperationError> {
    let (Some(path), Some(against)) = (arguments.get(1).copied(), arguments.get(2).copied()) else {
        return Err(arguments_refusal(format!(
            "`{COMMAND}` takes an ABSOLUTE path and then the WHOLE TEXT to compare it against, \
             as one word: `rivet {COMMAND} <path> <text>`. It compares a file on this machine \
             to text you supply and writes nothing."
        )));
    };
    if let Some(extra) = arguments.get(3) {
        return Err(arguments_refusal(format!(
            "`{COMMAND}` takes a path and one text, and {extra:?} is a third word. The text is \
             ONE argument — quote it. If it really contains the words you meant, this build has \
             no way to say so and that is a refusal rather than a guess."
        )));
    }
    if against.len() > super::edit::TEXT_MAX_BYTES {
        return Err(arguments_refusal(format!(
            "the text to compare against carries {given} bytes and the most this build accepts \
             is {most} — `apply edit`'s cap, because a change that size is not an edit, it is a \
             file.",
            given = against.len(),
            most = super::edit::TEXT_MAX_BYTES,
        )));
    }

    super::filesystem::check_readable_path(COMMAND, path)?;

    let stat = editor
        .stat_path(path)
        .map_err(|e| super::filesystem::refused(e, path))?;

    // Present and absent are the two ordinary cases; everything else is a
    // refusal, and each says which of them applied.
    let before = match stat.kind {
        None => String::new(),
        Some(PathKind::File) => {
            // **FS4: refused from the STAT, before the read.** `readable`
            // below still checks its own read's length too — belt and
            // braces against a file that grows between the two calls — but
            // the ordinary case is now refused without ever allocating an
            // over-cap file at all, which is what this comparison is for.
            let most = u64::try_from(super::edit::FILE_MAX_BYTES).unwrap_or(u64::MAX);
            if stat.size_bytes > most {
                return Err(too_large(path, stat.size_bytes));
            }
            readable(path, editor)?
        }
        Some(other) => {
            return Err(OperationError::about(
                ErrorCode::Unsafe,
                path,
                format!(
                    "REFUSED {path} is a {other} and not a regular file, so there is nothing to \
                     compare. {extra} `rivet get file {path}` says what is there.",
                    extra = if other == PathKind::Symlink {
                        "A symbolic link is NOT FOLLOWED by this surface (ADR-006 §8.10.4.3) — \
                         ask about the path it names instead."
                    } else {
                        "This build reads the contents of regular files only."
                    },
                ),
            ));
        }
    };

    let computed = diff::unified(
        &before,
        against,
        path,
        "the text you supplied (would become the whole file)",
    );

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("path", Value::str(path)),
        ("present", Value::Bool(stat.present())),
        ("changed", Value::Bool(computed.changed())),
        (
            "added_lines",
            Value::Integer(i64::try_from(computed.added).unwrap_or(i64::MAX)),
        ),
        (
            "removed_lines",
            Value::Integer(i64::try_from(computed.removed).unwrap_or(i64::MAX)),
        ),
        ("truncated", Value::Bool(computed.truncated)),
        (
            "algorithm",
            Value::str(if computed.coarse { "coarse" } else { "lines" }),
        ),
        ("diff", Value::str(computed.text.clone())),
        ("next", Value::str(next(&computed, path, stat.present()))),
    ];

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a `get diff` result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// An argument refusal, spelled once.
fn arguments_refusal(message: String) -> OperationError {
    OperationError::about(ErrorCode::Arguments, SUBJECT, message)
}

/// **The refusal for a file over `super::edit::FILE_MAX_BYTES`, spelled once**
/// so the stat-based refusal in [`perform`] and the read-based one in
/// [`readable`] cannot drift into two different sentences for one limit.
/// `held` is whichever of the two already knows the file's size — a stat
/// taken moments ago, or the bytes this process just finished reading.
fn too_large(path: &str, held: u64) -> OperationError {
    OperationError::about(
        ErrorCode::Unsafe,
        path,
        format!(
            "REFUSED {path} holds {held} bytes and this operation reads a file whole; the \
             most it will read is {most}. It runs in the machine's login shell \
             on a board with no swap.",
            most = super::edit::FILE_MAX_BYTES,
        ),
    )
}

/// The file's bytes as text, or the refusal that says why they are not.
///
/// **`super::edit::FILE_MAX_BYTES` and its sentences, not a second set.** This
/// reads a file whole in the same process for the same reason, and two caps
/// would eventually be two numbers.
fn readable(path: &str, editor: &mut dyn Editor) -> Result<String, OperationError> {
    let read = editor
        .read_file(path)
        .map_err(|e| super::filesystem::refused(e, path))?;
    let Some(bytes) = read.contents else {
        // The stat said a regular file and the reader says nothing. Reported
        // rather than treated as empty: an empty file and a file that vanished
        // between two calls produce very different diffs.
        return Err(OperationError::about(
            ErrorCode::MachineRefused,
            path,
            format!(
                "REFUSED {path} held a regular file when it was examined and held nothing when \
                 it was read, a moment later. Something else on this machine removed it in \
                 between, and a diff against 'empty' would describe neither state."
            ),
        ));
    };
    // **Belt and braces against a file that grew between the stat and this
    // read** — `perform`'s own comparison against the earlier stat is what
    // stops the ordinary over-cap file from being read at all.
    if bytes.len() > super::edit::FILE_MAX_BYTES {
        return Err(too_large(path, u64::try_from(bytes.len()).unwrap_or(u64::MAX)));
    }
    core::str::from_utf8(&bytes)
        .map(str::to_string)
        .map_err(|e| {
            OperationError::about(
                ErrorCode::Unsafe,
                path,
                format!(
                    "REFUSED {path} is not UTF-8 — the first byte this build cannot read is at \
                     offset {at} — so it is not text and there is no line-by-line comparison to \
                     make. Rendering it as text anyway would be an encoding guess that does not \
                     fail, it just produces a plausible wrong answer (AGENTS.md §5).",
                    at = e.valid_up_to(),
                ),
            )
        })
}

/// What this answer settles, and what it does not.
fn next(computed: &diff::Diff, path: &str, present: bool) -> String {
    if !computed.changed() {
        return format!(
            "THE FILE ALREADY HOLDS EXACTLY THIS TEXT, so there is nothing to write — and \
             `rivet apply edit` would refuse a change that rewrote a file to what it already \
             holds. This is a reading taken at one moment: nothing holds {path} still."
        );
    }
    let creation = if present {
        String::new()
    } else {
        format!(
            " THERE IS NO FILE AT {path}: this diff is against an empty file, so it describes \
             what `rivet apply create-file {path} <text> confirm` would write."
        )
    };
    let caveat = if computed.truncated || computed.coarse {
        " THE DIFF ABOVE IS NOT THE WHOLE PICTURE — `truncated` and `algorithm` say which \
         limit applied; the two line counts are exact either way."
    } else {
        ""
    };
    format!(
        "NOTHING HAS BEEN WRITTEN — this operation compares and returns.{creation} To make this \
         change, `rivet apply edit {path} --expect <text> --replace <text> confirm` changes one \
         place in the file, which is a different act from replacing it whole: this build has no \
         operation that writes a whole file over an existing one, on purpose (ADR-006 \
         §8.9.3).{caveat}"
    )
}

#[cfg(test)]
#[path = "file_diff_tests.rs"]
mod tests;
