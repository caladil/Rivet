//! `rivet apply remove-file <path> confirm` and
//! `rivet apply remove-directory <path> confirm` — **the two subjects that
//! destroy something.**
//!
//! `ADR-006` §8.10.12, ruled by the maintainer, Joel Lindell, 2026-08-29. These
//! are the pair the ruling was actually for:
//!
//! > at `ADR-035` phase 5 the control plane is the only way in and **nothing
//! > can delete a stale payload from `/state`**, which is today done by hand.
//!
//! An update payload is roughly 683 MiB and they accumulate. A surface that can
//! install one and cannot clean up after itself is the same *improvising*
//! `ADR-006` §8.9.1 was written to end, pointing the other way.
//!
//! # Two subjects and not one, and the asymmetry is the safety property
//!
//! `ADR-006` §8.9 says *"each is a different act with a different plan"*, and
//! these two prove it: a file's plan carries its **size**, because that is the
//! one fact nobody can recover afterwards; a directory's plan carries that it
//! is **empty**, because that is the whole of what makes removing it safe. One
//! subject covering both would have a plan that said neither exactly.
//!
//! # `remove-file` removes REGULAR FILES ONLY, and that one rule does a lot
//!
//! A directory is refused and names `remove-directory`. A symbolic link is
//! refused (`ADR-006` §8.10.4.3). A device node, a socket or a fifo is refused
//! by name. **A mount point is a directory**, so it cannot be reached here at
//! all. Four hazards, one sentence in the refusal, and an operator reads it at
//! the moment they are already wrong.
//!
//! # A path that is not there is a refusal, not a success
//!
//! `super::edit`'s departure from `ADR-006` §6.2's idempotence, and the same
//! argument: *"nothing was there"* and *"I removed it"* are different facts, and
//! a caller that got the path wrong needs to know which one happened. A verb
//! that answered `Ok` either way would let a typo look like a success for ever.
//!
//! # Time of check to time of use, stated exactly rather than optimistically
//!
//! **Every refusal above is re-run at the moment this acts**, not trusted from
//! the planning phase — `AGENTS.md` §5's *the kernel is the authority, asked at
//! the point of use*. The stat taken while planning is compared to the stat
//! taken while applying, and a difference refuses.
//!
//! **What that does NOT do is compare against the plan an operator read.** This
//! process ends when it answers, and nothing carries a plan from one command to
//! the next, so a `confirm` typed a minute later removes whatever is at the path
//! *then*. `super::edit` is the one subject in this catalogue that does better,
//! and only because its `--expect` **travels in the command** — the operator
//! carries the state they believe in, in their own hands.
//!
//! **An `--expect-bytes` here would close it**, and it is deliberately not
//! built: the routine case this ruling exists for is removing a stale payload
//! after `rivet get storage` said the card is full, and putting a byte count in
//! that command is friction on exactly the act §8.10.1 was written to make
//! possible. `ADR-006` §8.10.16 records it as the maintainer's to rule, and the
//! plan says plainly what the guard is and is not rather than implying more.
//!
//! # Recursive removal is NOT here, and `ADR-006` §8.10.13 says why
//!
//! `remove-directory` removes an **empty** directory and the kernel's `rmdir(2)`
//! refuses a non-empty one under the domain's own check. A recursive form is
//! specified in §8.10.13 and deliberately unbuilt: it is the one act in this
//! family whose danger is not bounded by the operator's attention, and
//! `AGENTS.md` §11.1 says an agent does not decide that.

use super::apply::{ACCEPT_FLAG, CONFIRMATION, Hazard};
use super::filesystem::{
    check_writable_path, confirmation_words, describe_stat, hazard_for, read_tail, refused,
    unsafe_for,
};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::value::Value;

/// The subject that removes one regular file.
pub const FILE_SUBJECT: &str = "remove-file";
/// The subject that removes one empty directory.
pub const DIRECTORY_SUBJECT: &str = "remove-directory";

/// What both subjects' results carry. `ADR-006` §7.
///
/// **One declaration for two subjects**, which is `super::stop`'s precedent:
/// they produce the same shape, so they name the same list rather than two
/// copies that could disagree. What differs between them is the *plan* and the
/// *refusals*, which is where §8.9 puts the difference.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "Which removal this was: `remove-file` or `remove-directory`.",
    },
    Field {
        name: "path",
        description: "What was removed, exactly as it was given.",
    },
    Field {
        name: "type",
        description: "What was there: `file` or `directory`. It is reported because the two \
                      subjects refuse each other's kind, so an answer that carries it can be \
                      checked without re-reading the command.",
    },
    Field {
        name: "before-bytes",
        description: "`remove-file` ONLY: how many bytes are now gone. THIS IS THE ONE FACT \
                      NOBODY CAN RECOVER, which is why it is a field and not only prose.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked, in a sentence (ADR-028 §4.5). \
                      It is ATTRIBUTION AND NOT AUTHORITY: nothing checked it, and anything \
                      running as uid 0 could have removed the path itself.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of this removal reached this machine's journal. FALSE \
                      DOES NOT MEAN NOTHING WAS REMOVED — the path is gone either way, and \
                      `unrecorded` says what stopped the record.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of this removal. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle.",
    },
];

/// Which of the two this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}

impl Kind {
    /// The subject word, if this is one of them.
    pub(super) fn parse(subject: &str) -> Option<Self> {
        match subject {
            FILE_SUBJECT => Some(Self::File),
            DIRECTORY_SUBJECT => Some(Self::Directory),
            _ => None,
        }
    }

    const fn subject(self) -> &'static str {
        match self {
            Self::File => FILE_SUBJECT,
            Self::Directory => DIRECTORY_SUBJECT,
        }
    }

    /// The kind of thing this subject removes.
    const fn removes(self) -> PathKind {
        match self {
            Self::File => PathKind::File,
            Self::Directory => PathKind::Directory,
        }
    }

    /// The other subject, for a refusal that redirects.
    const fn sibling(self) -> &'static str {
        match self {
            Self::File => DIRECTORY_SUBJECT,
            Self::Directory => FILE_SUBJECT,
        }
    }
}

/// One thing that was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    path: String,
    kind: Kind,
    bytes: u64,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Removed {
    /// What was removed.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether a record of it reached the journal.
    pub const fn journalled(&self) -> bool {
        self.unrecorded.is_none()
    }

    /// The structured result, whose keys are a subsequence of `OUTPUTS`.
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(self.kind.subject())),
            ("path", Value::str(self.path.clone())),
            ("type", Value::str(self.kind.removes().as_str())),
        ];
        if self.kind == Kind::File {
            fields.push((
                "before-bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ));
        }
        fields.push(("principal", Value::str(describe_who(&self.principal, self.kind))));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push(("next", Value::str(next_after(self))));
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "a removal's keys must be a subsequence of OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// Who asked, in this operation's framing of [`Principal::describes`].
///
/// `super::edit`'s function with three words changed, and the forgery sentence
/// is sharper here than anywhere: there is no mechanism to bypass, and after a
/// removal there is nothing left to compare against.
fn describe_who(principal: &Principal, kind: Kind) -> String {
    principal.describes(
        "the path",
        format!(
            "`rivet {apply} {subject}`",
            apply = Verb::Apply,
            subject = kind.subject()
        )
        .as_str(),
        "anything running as uid 0 could have removed it another way and left no record at all \
         — and after a removal there is nothing left to compare against, which is why the \
         record carries the size that is now gone",
    )
}

/// What the operator should expect afterwards.
fn next_after(removed: &Removed) -> String {
    let record = if removed.journalled() {
        "THIS REMOVAL IS IN THE JOURNAL, naming the session that asked (ADR-028 §4.5) — `rivet \
         get log` is where it appears, as a record of service `rivet` at severity `warn`."
    } else {
        "THIS REMOVAL IS NOT IN THE JOURNAL and `unrecorded` says why. The path is gone either \
         way; what is missing is the account of it."
    };
    format!(
        "THE PATH IS GONE AND THIS CANNOT BE UNDONE — there is no trash on this machine and no \
         snapshot. {record} NOTHING HAS BEEN RELOADED and nothing was stopped: a service \
         holding this file open still holds it, and the space it used is not released until it \
         closes it (`rivet get storage` is where that shows). This operation cannot say whether \
         anything needed the path."
    )
}

// ---------------------------------------------------------------------------
// Performing it
// ---------------------------------------------------------------------------

/// The two forms, for every refusal that has to show them.
fn forms(kind: Kind) -> String {
    format!(
        "`rivet {apply} {subject} <path> [{ACCEPT_FLAG} <text>] [{CONFIRMATION}]`",
        apply = Verb::Apply,
        subject = kind.subject(),
    )
}

/// Perform one of the two removals.
///
/// # The order of the refusals is part of the design
///
/// Everything true of the **request** — the path's shape, the permitted root,
/// the views, the mechanisms, the load-bearing set — is refused before anything
/// is examined, so a refusal here has touched nothing at all and is reproducible
/// from a workstation. Only then is the machine asked what is there.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a malformed command, [`ErrorCode::Unsafe`] for a
/// path this must not remove, [`ErrorCode::ConfirmationRequired`] carrying the
/// plan when the confirmation is absent, and [`ErrorCode::MachineRefused`] when
/// the machine could not do it.
pub(super) fn perform(
    kind: Kind,
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Removed, OperationError> {
    let Some(path) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            kind.subject(),
            format!(
                "`{apply} {subject}` needs the ABSOLUTE path to remove, as the word after \
                 `{subject}`. {forms}. There is no default, nothing is searched for, and \
                 nothing is expanded: one path is named, and it is the only thing that is \
                 removed.",
                apply = Verb::Apply,
                subject = kind.subject(),
                forms = forms(kind),
            ),
        ));
    };

    let tail = read_tail(kind.subject(), &forms(kind), arguments, 2)?;
    check_writable_path(&format!("{} {}", Verb::Apply, kind.subject()), path)?;
    let hazard = hazard_for(path);
    super::apply::accepted(tail.accept, hazard.as_ref(), tail.confirmed)?;

    resolve(
        Removal {
            kind,
            path,
            typed: arguments,
            hazard,
        },
        tail.confirmed,
        board,
    )
}

/// **The change one of these subjects is**, as `ADR-006` §6.5 defines one.
struct Removal<'a> {
    kind: Kind,
    path: &'a str,
    typed: &'a [&'a str],
    hazard: Option<Hazard>,
}

/// What planning a removal worked out, and what performing it must re-check.
struct Prepared {
    /// **The whole reading the plan described.** Carried across so the confirmed
    /// path can compare rather than re-describe — see this module's
    /// documentation on time of check to time of use.
    stat: PathStat,
    principal: Principal,
}

impl Change for Removal<'_> {
    type Prepared = Prepared;
    type Applied = Removed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path;
        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        self.check_what_is_there(&stat, board)?;

        let principal = board.principal();
        let change = self.change(&stat);
        board.can_change_path(&change).map_err(|e| refused(e, path))?;

        let what = describe_stat(&stat);
        let irreversible = "THIS CANNOT BE UNDONE: there is no trash on this machine, no \
                            snapshot and no undo";
        let zone = super::filesystem::app_zone_clause(path);
        let warning = self
            .hazard
            .as_ref()
            .map_or_else(String::new, |h| format!(" {}", h.warning()));
        let note = format!("{what}. {irreversible}.{zone}{warning}");

        let effect = match self.kind {
            Kind::File => Effect::remove(path)
                .was(format!("{} bytes", stat.size_bytes))
                .noting(note),
            Kind::Directory => Effect::remove(path).was("empty").noting(note),
        };

        let mut plan = Plan::new(
            path,
            match self.kind {
                Kind::File => "DESTROYS a file on this machine",
                Kind::Directory => "DESTROYS a directory on this machine",
            },
        )
        .doing(effect)
        .leaving(match self.kind {
            Kind::File => "no directory is removed, and nothing else in the directory is \
                           touched — this removes exactly the one file named",
            Kind::Directory => "nothing inside it, because it is empty and this refuses a \
                                directory that is not. There is no recursive removal in this \
                                build (ADR-006 §8.10.13)",
        })
        .leaving(
            "nothing is stopped and nothing is reloaded. A service holding this path open keeps \
             it, and the space is not released until it closes it",
        )
        .taking_effect(TakesEffect::Immediately)
        .confirmed_by(&confirmation_words(self.typed, self.hazard.as_ref()));

        // **Said exactly, because the optimistic version would be false.** See
        // this module's documentation on time of check to time of use.
        plan = plan.leaving(format!(
            "WHAT THIS PLAN DOES NOT GUARANTEE: it describes {path} AS IT IS NOW, and this \
             process ends when it answers you. Nothing carries a plan from one command to the \
             next, so a `{CONFIRMATION}` typed later removes whatever is at {path} THEN. Every \
             refusal above is re-run at that moment — a directory that has become non-empty, a \
             file that has become a link — but this cannot compare against what you are reading. \
             `apply {edit}` is the one subject that can, and only because its `--expect` travels \
             in the command",
            edit = super::edit::SUBJECT,
        ));

        Ok((plan, Prepared { stat, principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Removed, OperationError> {
        let path = self.path;
        // **Re-run, not remembered** — `AGENTS.md` §5. This catches a change
        // between the planning read and this one, which is a window of
        // microseconds rather than of however long an operator took to read;
        // the module documentation is precise about which of the two this is.
        let now = board.stat_path(path).map_err(|e| refused(e, path))?;
        self.check_what_is_there(&now, board)?;
        if now.kind != prepared.stat.kind
            || now.size_bytes != prepared.stat.size_bytes
            || now.modified_ms != prepared.stat.modified_ms
        {
            return Err(unsafe_for(
                path,
                format!(
                    "REFUSED {path} IS NOT WHAT THE PLAN DESCRIBED and NOTHING HAS BEEN \
                     REMOVED. The plan described {then}; it is now {now}. Something else on \
                     this machine changed it between the plan and the confirmation, so this \
                     removal would destroy something nobody looked at. Ask again — `rivet {get} \
                     {file} {path}` — and confirm the plan you actually read.",
                    then = describe_stat(&prepared.stat),
                    now = describe_stat(&now),
                    get = Verb::Get,
                    file = super::file::SUBJECT,
                ),
            ));
        }

        let change = self.change(&prepared.stat);
        let wrote = board.change_path(&change).map_err(|e| refused(e, path))?;

        Ok(Removed {
            path: path.to_string(),
            kind: self.kind,
            bytes: prepared.stat.size_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Removal<'_> {
    /// The port's description of this act.
    fn change<'b>(&'b self, stat: &PathStat) -> PathChange<'b> {
        match self.kind {
            Kind::File => PathChange::RemoveFile {
                path: self.path,
                was_bytes: stat.size_bytes,
            },
            Kind::Directory => PathChange::RemoveDirectory { path: self.path },
        }
    }

    /// **Everything true of what is actually at the path.** Run while planning
    /// and again at the confirmation, because the machine is the authority and
    /// it is asked at the point of use (`AGENTS.md` §5).
    fn check_what_is_there(
        &self,
        stat: &PathStat,
        board: &mut dyn Board,
    ) -> Result<(), OperationError> {
        let path = self.path;
        let subject = self.kind.subject();

        let Some(kind) = stat.kind else {
            return Err(unsafe_for(
                path,
                format!(
                    "REFUSED there is nothing at {path}, so there is nothing to remove and \
                     NOTHING HAS BEEN REMOVED. This is a refusal rather than a success on \
                     purpose: 'nothing was there' and 'I removed it' are different facts, and a \
                     verb that answered the same way to both would let a mistyped path look \
                     like a success for ever. `rivet {get} {file} {path}` says what is where \
                     you meant.",
                    get = Verb::Get,
                    file = super::file::SUBJECT,
                ),
            ));
        };

        if kind != self.kind.removes() {
            let advice = match kind {
                PathKind::Directory => format!(
                    "`rivet {apply} {sibling} {path} {CONFIRMATION}` removes a directory, and \
                     only an EMPTY one",
                    apply = Verb::Apply,
                    sibling = self.kind.sibling(),
                ),
                PathKind::File => format!(
                    "`rivet {apply} {sibling} {path} {CONFIRMATION}` removes a file",
                    apply = Verb::Apply,
                    sibling = self.kind.sibling(),
                ),
                PathKind::Symlink => format!(
                    "THIS BUILD REMOVES NO SYMBOLIC LINK (ADR-006 §8.10.4.3). Removing one \
                     removes the LINK and not what it names, and that is a sentence an operator \
                     has to read correctly at exactly the moment they are in a hurry — so it is \
                     refused instead. It points at {target}",
                    target = stat.link_target.as_deref().unwrap_or("something unreadable"),
                ),
                PathKind::Other => "THIS BUILD REMOVES NO DEVICE NODE, SOCKET OR FIFO. Rivet \
                                    creates none under /state, so a path like this is either \
                                    something an application made deliberately or something \
                                    nobody meant"
                    .to_string(),
            };
            return Err(unsafe_for(
                path,
                format!(
                    "REFUSED {path} is a {kind} and `{subject}` removes a {removes}, so NOTHING \
                     HAS BEEN REMOVED. {advice}.",
                    removes = self.kind.removes(),
                ),
            ));
        }

        // **A mount point is refused even where the kind is right**, and for a
        // directory it is the check whose failure costs most: removing one is
        // either impossible or removes the *mount point* and leaves a
        // filesystem attached to nothing an operator can name.
        if stat.mount_point {
            return Err(unsafe_for(
                path,
                format!(
                    "REFUSED something is MOUNTED at {path} and NOTHING HAS BEEN REMOVED. What \
                     is under a mount point is a different filesystem, and removing the point \
                     itself does not remove what is mounted on it — it leaves a filesystem \
                     attached to nothing this machine can name. Unmounting is not an operation \
                     this build has."
                ),
            ));
        }

        if self.kind == Kind::Directory && stat.entry_count > 0 {
            let names: Vec<&str> = stat
                .entries
                .iter()
                .take(8)
                .map(|entry| entry.name.as_str())
                .collect();
            return Err(unsafe_for(
                path,
                format!(
                    "REFUSED {path} is NOT EMPTY — it holds {count} entries, beginning \
                     {names} — and NOTHING HAS BEEN REMOVED. `{subject}` removes an empty \
                     directory and there is NO RECURSIVE REMOVAL IN THIS BUILD: ADR-006 \
                     §8.10.13 specifies one and leaves it unbuilt deliberately, because it is \
                     the one act in this family whose danger is not bounded by what the \
                     operator can read in a plan. Remove what is in it first, or leave it.",
                    count = stat.entry_count,
                    names = names.join(", "),
                ),
            ));
        }

        // Nothing here needs the board today; it is a parameter so that a check
        // that has to ask the machine can be added without changing every
        // caller. Named `_board` would hide that intent.
        let _ = board;
        Ok(())
    }
}

#[cfg(test)]
#[path = "remove_tests.rs"]
mod tests;
