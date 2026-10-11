//! `rivet apply rename <from> <to> confirm` — **move a path, and refuse to
//! replace anything doing it.**
//!
//! `ADR-006` §8.10.11, ruled by the maintainer, Joel Lindell, 2026-08-29.
//!
//! # The refusal that is this operation's whole safety story
//!
//! **`rename(2)` replaces an existing destination silently.** That is a delete
//! nothing planned, performed by a verb whose name says *move* — the exact shape
//! of failure `super::edit`'s replace-exactly-once exists to prevent, in a
//! different costume. So **an existing `<to>` is refused**, before anything, and
//! the confirmed path checks again.
//!
//! Everything else follows from it. There is no `--force`, there is no
//! overwrite, and there is no form of this that removes something: if the
//! operator wants what is at `<to>` gone, `apply remove-file` is the act, and it
//! has its own plan saying what it would destroy.
//!
//! # It moves directories too, and the plan says how much is moving
//!
//! A rename is atomic and destroys nothing whichever it is, so refusing a
//! directory would be caution without a hazard. What an operator needs is to
//! **see the size of what moves**, which is why the plan carries the entry count
//! for a directory and the byte count for a file.
//!
//! # No directory is created on the way
//!
//! `<to>`'s parent must already exist and be a directory. `apply edit` creates
//! missing parents and names every one of them in its plan; doing that here
//! would fold two acts into one, which is exactly what `ADR-006` §8.9 says not
//! to do. `apply create-directory` is the other act.
//!
//! # Both paths pass the whole gate, and both are checked for links
//!
//! `ADR-006` §8.10.4: a rename that could not leave the permitted root through
//! its source could still leave through its destination, so `<to>` is gated as
//! hard as `<from>` — and `Editor::can_change_path` walks both for symbolic
//! links, which is the half a test on a string cannot make.

use super::apply::{ACCEPT_FLAG, CONFIRMATION, Hazard};
use super::filesystem::{
    check_writable_path, confirmation_words, describe_stat, directory_of, hazard_for, read_tail,
    refused, unsafe_for,
};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::value::Value;

/// The subject. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "rename";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `rename` here.",
    },
    Field {
        name: "path",
        description: "Where it was, exactly as it was given. Nothing is there now.",
    },
    Field {
        name: "to",
        description: "Where it is now, exactly as it was given.",
    },
    Field {
        name: "type",
        description: "What moved: `file` or `directory`. A symbolic link is never moved by this \
                      build (ADR-006 §8.10.4.3).",
    },
    Field {
        name: "before-bytes",
        description: "`file` ONLY: how large it is. The move does not change it — this is here \
                      so the record says what was moved and not only that something was.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked, in a sentence (ADR-028 §4.5). \
                      ATTRIBUTION AND NOT AUTHORITY: nothing checked it.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of this move reached this machine's journal. FALSE DOES \
                      NOT MEAN NOTHING MOVED.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of this move. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle.",
    },
];

/// One path that was moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    from: String,
    to: String,
    kind: PathKind,
    bytes: u64,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Renamed {
    /// Where it is now.
    pub fn to(&self) -> &str {
        &self.to
    }

    /// Whether a record of it reached the journal.
    pub const fn journalled(&self) -> bool {
        self.unrecorded.is_none()
    }

    /// The structured result, whose keys are a subsequence of `OUTPUTS`.
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("path", Value::str(self.from.clone())),
            ("to", Value::str(self.to.clone())),
            ("type", Value::str(self.kind.as_str())),
        ];
        if self.kind == PathKind::File {
            fields.push((
                "before-bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ));
        }
        fields.push(("principal", Value::str(describe_who(&self.principal))));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push(("next", Value::str(next_after(self))));
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "a rename's keys must be a subsequence of OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// Who asked, in this operation's framing of [`Principal::describes`].
fn describe_who(principal: &Principal) -> String {
    principal.describes(
        "the path",
        format!("`rivet {apply} {SUBJECT}`", apply = Verb::Apply).as_str(),
        "anything running as uid 0 could have moved it another way and left no record at all",
    )
}

/// What the operator should expect afterwards.
fn next_after(renamed: &Renamed) -> String {
    let record = if renamed.journalled() {
        "THIS MOVE IS IN THE JOURNAL, naming the session that asked (ADR-028 §4.5)."
    } else {
        "THIS MOVE IS NOT IN THE JOURNAL and `unrecorded` says why. The path moved either way."
    };
    format!(
        "THE PATH HAS MOVED AND NOTHING WAS REPLACED — this operation refuses an existing \
         destination, so whatever was at `to` before is a question that did not arise. \
         {record} NOTHING HAS BEEN RELOADED: a service that reads the OLD path now finds \
         nothing there, and the supervisor converges on its own pass (ADR-017 §3.2). This \
         operation cannot say whether anything was reading it."
    )
}

// ---------------------------------------------------------------------------
// Performing it
// ---------------------------------------------------------------------------

/// The form, for every refusal that has to show it.
fn forms() -> String {
    format!(
        "`rivet {apply} {SUBJECT} <from> <to> [{ACCEPT_FLAG} <text>] [{CONFIRMATION}]`",
        apply = Verb::Apply,
    )
}

/// Perform `apply rename`.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a malformed command, [`ErrorCode::Unsafe`] for a
/// path this must not move or a destination that is occupied,
/// [`ErrorCode::ConfirmationRequired`] carrying the plan, and
/// [`ErrorCode::MachineRefused`] when the machine could not do it.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Renamed, OperationError> {
    let (Some(from), Some(to)) = (arguments.get(1).copied(), arguments.get(2).copied()) else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs TWO ABSOLUTE paths — where it is now, then where it \
                 should be. {forms}. There is no default destination and nothing is derived \
                 from the source: both are named, so that neither can be a surprise.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };

    let tail = read_tail(SUBJECT, &forms(), arguments, 3)?;
    let command = format!("{} {SUBJECT}", Verb::Apply);
    check_writable_path(&command, from)?;
    check_writable_path(&command, to)?;

    if from == to {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "the two paths are the same — {from} — so this names no move. Refused rather \
                 than performed as a no-op, because a rename onto itself would still put a \
                 record in the journal saying something moved."
            ),
        ));
    }

    // **The destination's hazard, not the source's.** What a rename can break is
    // what it puts somewhere; moving a program *away* leaves a missing file,
    // which the launch check reports as plainly as a changed one — and moving a
    // file *onto* a program path is the case where something starts being
    // verified that never was.
    let hazard = hazard_for(to).or_else(|| hazard_for(from));
    super::apply::accepted(tail.accept, hazard.as_ref(), tail.confirmed)?;

    resolve(
        Move {
            from,
            to,
            typed: arguments,
            hazard,
        },
        tail.confirmed,
        board,
    )
}

/// **The change `apply rename` is**, as `ADR-006` §6.5 defines one.
struct Move<'a> {
    from: &'a str,
    to: &'a str,
    typed: &'a [&'a str],
    hazard: Option<Hazard>,
}

/// What planning a move worked out, and what performing it must re-check.
struct Prepared {
    source: PathStat,
    principal: Principal,
}

impl Change for Move<'_> {
    type Prepared = Prepared;
    type Applied = Renamed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let source = board
            .stat_path(self.from)
            .map_err(|e| refused(e, self.from))?;
        self.check(&source, board)?;

        let principal = board.principal();
        board
            .can_change_path(&PathChange::Rename {
                from: self.from,
                to: self.to,
            })
            .map_err(|e| refused(e, self.from))?;

        let size = match source.kind {
            Some(PathKind::Directory) => format!("{} entries", source.entry_count),
            _ => format!("{} bytes", source.size_bytes),
        };
        let zone = format!(
            "{}{}",
            super::filesystem::app_zone_clause(self.from),
            super::filesystem::app_zone_clause(self.to),
        );
        let warning = self
            .hazard
            .as_ref()
            .map_or_else(String::new, |h| format!(" {}", h.warning()));

        let plan = Plan::new(
            self.from,
            "MOVES something on this machine, and replaces nothing",
        )
        .doing(
            Effect::remove(self.from)
                .was(size.clone())
                .noting(format!(
                    "{what}. Nothing is at {to} — this operation REFUSES an occupied \
                     destination, because rename(2) would replace it silently and that is a \
                     delete nothing planned.{zone}{warning}",
                    what = describe_stat(&source),
                    to = self.to,
                )),
        )
        .doing(Effect::create(self.to).becomes(size))
        .leaving(
            "the contents are not read and not changed — a move is a rename and not a copy, so \
             nothing is rewritten and no space is used",
        )
        .leaving(format!(
            "no directory is created: {parent} must already exist, and `rivet {apply} \
             create-directory` is the act that makes one",
            parent = directory_of(self.to),
            apply = Verb::Apply,
        ))
        .taking_effect(TakesEffect::Immediately)
        .leaving(format!(
            "WHAT THIS PLAN DOES NOT GUARANTEE: it describes the machine AS IT IS NOW, and this \
             process ends when it answers you. Every refusal above is re-run at the moment this \
             acts — including that nothing is at {to} — but nothing carries a plan from one \
             command to the next, so a `{CONFIRMATION}` typed later acts on the machine THEN. \
             The destination check is the one that matters and it does run again",
            to = self.to,
        ))
        .confirmed_by(&confirmation_words(self.typed, self.hazard.as_ref()));

        Ok((plan, Prepared { source, principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Renamed, OperationError> {
        // The guard against a stale plan, on both paths — `super::remove`'s
        // rule and `ADR-006` §8.9.5's.
        let now = board
            .stat_path(self.from)
            .map_err(|e| refused(e, self.from))?;
        self.check(&now, board)?;
        if now.kind != prepared.source.kind || now.size_bytes != prepared.source.size_bytes {
            return Err(unsafe_for(
                self.from,
                format!(
                    "REFUSED {from} IS NOT WHAT THE PLAN DESCRIBED and NOTHING HAS MOVED. The \
                     plan described {then}; it is now {now}. Ask again and confirm the plan you \
                     actually read.",
                    from = self.from,
                    then = describe_stat(&prepared.source),
                    now = describe_stat(&now),
                ),
            ));
        }

        let wrote = board
            .change_path(&PathChange::Rename {
                from: self.from,
                to: self.to,
            })
            .map_err(|e| refused(e, self.from))?;

        Ok(Renamed {
            from: self.from.to_string(),
            to: self.to.to_string(),
            kind: prepared.source.kind.unwrap_or(PathKind::Other),
            bytes: prepared.source.size_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Move<'_> {
    /// Everything true of the two paths on the machine. Run while planning and
    /// again at the confirmation.
    fn check(&self, source: &PathStat, board: &mut dyn Board) -> Result<(), OperationError> {
        let Some(kind) = source.kind else {
            return Err(unsafe_for(
                self.from,
                format!(
                    "REFUSED there is nothing at {from}, so there is nothing to move and \
                     NOTHING HAS MOVED. `rivet {get} {file} {from}` says what is where you \
                     meant.",
                    from = self.from,
                    get = Verb::Get,
                    file = super::file::SUBJECT,
                ),
            ));
        };

        if kind == PathKind::Symlink || kind == PathKind::Other {
            return Err(unsafe_for(
                self.from,
                format!(
                    "REFUSED {from} is a {kind} and this build moves files and directories \
                     only, so NOTHING HAS MOVED. A symbolic link is never followed or moved by \
                     this surface (ADR-006 §8.10.4.3), and Rivet creates no device nodes, \
                     sockets or fifos under {root}.",
                    from = self.from,
                    root = super::filesystem::PERMITTED_ROOT,
                ),
            ));
        }

        if source.mount_point {
            return Err(unsafe_for(
                self.from,
                format!(
                    "REFUSED something is MOUNTED at {from} and NOTHING HAS MOVED. A mount \
                     point cannot be renamed while it is one, and a rename that appeared to \
                     succeed would have moved the directory underneath a filesystem that is \
                     still attached to the old name.",
                    from = self.from,
                ),
            ));
        }

        // **The destination, and this is the refusal the operation exists
        // around.** See this module's documentation.
        let destination = board.stat_path(self.to).map_err(|e| refused(e, self.to))?;
        if let Some(there) = destination.kind {
            return Err(unsafe_for(
                self.to,
                format!(
                    "REFUSED there is already a {there} at {to} and NOTHING HAS MOVED. \
                     rename(2) would REPLACE it silently — that is a delete nobody planned, \
                     performed by a verb whose name says move — so this refuses instead. It is \
                     {what}. If it should go, `rivet {apply} {remove} {to} {CONFIRMATION}` \
                     removes it and says what it is destroying first; there is no `--force` \
                     here and there is not going to be.",
                    to = self.to,
                    what = describe_stat(&destination),
                    apply = Verb::Apply,
                    remove = if there == PathKind::Directory {
                        super::remove::DIRECTORY_SUBJECT
                    } else {
                        super::remove::FILE_SUBJECT
                    },
                ),
            ));
        }

        // The destination's parent, because nothing here creates one.
        let parent = directory_of(self.to);
        let holding = board.stat_path(parent).map_err(|e| refused(e, parent))?;
        match holding.kind {
            Some(PathKind::Directory) => Ok(()),
            None => Err(unsafe_for(
                self.to,
                format!(
                    "REFUSED {parent} does not exist, so {to} cannot be inside it and NOTHING \
                     HAS MOVED. THIS OPERATION CREATES NO DIRECTORY: `apply {edit}` creates \
                     missing parents and names every one of them in its plan, and doing that \
                     here would fold two acts into one (ADR-006 §8.9). `rivet {apply} \
                     create-directory {parent} {CONFIRMATION}` is the other act.",
                    to = self.to,
                    edit = super::edit::SUBJECT,
                    apply = Verb::Apply,
                ),
            )),
            Some(other) => Err(unsafe_for(
                self.to,
                format!(
                    "REFUSED {parent} is a {other} and not a directory, so nothing can be \
                     inside it and NOTHING HAS MOVED."
                ),
            )),
        }
    }
}

#[cfg(test)]
#[path = "rename_tests.rs"]
mod tests;
