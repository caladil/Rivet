//! `rivet apply create-file <path> [<text>] confirm` and
//! `rivet apply create-directory <path> confirm` — **the two subjects that make
//! something that was not there.**
//!
//! `ADR-006` §8.10.8, ruled by the maintainer, Joel Lindell, 2026-08-29.
//!
//! # `create-file` closes a gap `apply edit` names against itself
//!
//! `super::edit`'s refusal for an empty `--append` says it in as many words:
//!
//! > If the intent was to CREATE an empty file, this build has no operation for
//! > that and saying so is better than writing one you did not ask for.
//!
//! This is that operation. `apply edit --append` still creates a file as a side
//! effect of adding text to it, and that is not the same act: an append to a
//! file that turns out to exist is an append, and a *creation* that turns out to
//! exist is a mistake. Which is the whole of the difference between them —
//! **creating refuses anything already at the path.**
//!
//! # Creating is not clobbering, and that is the one refusal that matters
//!
//! Replacing what is in a file is `apply edit`, which makes the caller state
//! what it believes is there and be right. A `create` that overwrote would be
//! the blind clobber §8.9.2 exists to prevent, reached by a different word. So
//! **anything at the path — a file, a directory, a link, a device node — is a
//! refusal**, and the refusal says which of the two the operator probably meant.
//!
//! # A directory that is already there is a refusal too, and it is deliberate
//!
//! `ADR-006` §6.2 wants idempotence and this is the second place in the
//! catalogue that departs from it, on §8.9.2's argument: a subject that reported
//! success for an act it did not perform is asserting something that did not
//! happen. The refusal says it is already there, which is what the caller needed
//! to know and is strictly more information than `Ok`.
//!
//! # Modes are §8.9.3's, unchanged
//!
//! A new file is **0600** and a new directory **0700** — private first, widened
//! deliberately, never the reverse. `apply set-mode` is how it is widened, and
//! that is a second act with its own plan.

use super::apply::{ACCEPT_FLAG, CONFIRMATION, Hazard};
use super::filesystem::{
    check_writable_path, confirmation_words, describe_mode, describe_stat, hazard_for, read_tail,
    refused, unsafe_for,
};
use super::machine::{Board, PathChange, PathStat, Write as FileWrite};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::value::Value;

/// The subject that makes one file.
pub const FILE_SUBJECT: &str = "create-file";
/// The subject that makes one directory.
pub const DIRECTORY_SUBJECT: &str = "create-directory";

/// What both subjects' results carry. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "Which creation this was: `create-file` or `create-directory`.",
    },
    Field {
        name: "path",
        description: "What was created, exactly as it was given.",
    },
    Field {
        name: "type",
        description: "What was made: `file` or `directory`.",
    },
    Field {
        name: "after-bytes",
        description: "`create-file` ONLY: how many bytes it holds. ZERO IS THE ORDINARY CASE — \
                      a file created with no text is an empty file, which is the gap this \
                      subject exists to close.",
    },
    Field {
        name: "directories",
        description: "Every parent directory that had to be created, outermost first, each mode \
                      0700. ABSENT when none were — a creation is never silent (ADR-006 §8.9).",
    },
    Field {
        name: "mode",
        description: "The mode it was created with, octal. 0600 for a file and 0700 for a \
                      directory: private first, widened deliberately, never the reverse \
                      (ADR-006 §8.9.3). `rivet apply set-mode` is how it is widened.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked, in a sentence (ADR-028 §4.5). \
                      ATTRIBUTION AND NOT AUTHORITY: nothing checked it.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of this creation reached the journal. FALSE DOES NOT \
                      MEAN NOTHING WAS CREATED.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of it. Absent when it has one.",
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

    const fn word(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }

    /// The mode this subject creates with. `ADR-006` §8.9.3's two numbers, read
    /// from `super::edit` rather than spelled again.
    const fn mode(self) -> u32 {
        match self {
            Self::File => super::edit::NEW_FILE_MODE,
            Self::Directory => super::edit::NEW_DIRECTORY_MODE,
        }
    }
}

/// One thing that was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    path: String,
    kind: Kind,
    bytes: usize,
    directories: Vec<String>,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Created {
    /// What was created.
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
            ("type", Value::str(self.kind.word())),
        ];
        if self.kind == Kind::File {
            fields.push((
                "after-bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ));
        }
        if !self.directories.is_empty() {
            fields.push((
                "directories",
                Value::strings(self.directories.iter().map(String::as_str)),
            ));
        }
        fields.push(("mode", Value::str(format!("{:04o}", self.kind.mode()))));
        fields.push(("principal", Value::str(describe_who(&self.principal, self.kind))));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push(("next", Value::str(next_after(self))));
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "a creation's keys must be a subsequence of OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// Who asked, in this operation's framing of [`Principal::describes`].
fn describe_who(principal: &Principal, kind: Kind) -> String {
    principal.describes(
        "the path",
        format!(
            "`rivet {apply} {subject}`",
            apply = Verb::Apply,
            subject = kind.subject()
        )
        .as_str(),
        "anything running as uid 0 could have created it another way and left no record at all",
    )
}

/// What the operator should expect afterwards.
fn next_after(created: &Created) -> String {
    let record = if created.journalled() {
        "THIS CREATION IS IN THE JOURNAL, naming the session that asked (ADR-028 §4.5)."
    } else {
        "THIS CREATION IS NOT IN THE JOURNAL and `unrecorded` says why. It exists either way."
    };
    let widen = format!(
        "IT IS PRIVATE: mode {mode}, reachable by uid 0 and by nothing else. `rivet {apply} \
         set-mode {path} <octal> {CONFIRMATION}` widens it, and `rivet {apply} set-owner` \
         changes who owns it — each is a separate act with its own plan.",
        mode = describe_mode(created.kind.mode()),
        apply = Verb::Apply,
        path = created.path,
    );
    format!("IT EXISTS NOW. {widen} {record} NOTHING HAS BEEN RELOADED and no service was told.")
}

// ---------------------------------------------------------------------------
// Performing it
// ---------------------------------------------------------------------------

/// The form, for every refusal that has to show it.
fn forms(kind: Kind) -> String {
    match kind {
        Kind::File => format!(
            "`rivet {apply} {FILE_SUBJECT} <path> [<text>] [{ACCEPT_FLAG} <text>] \
             [{CONFIRMATION}]` — the text is OPTIONAL and one word; without it the file is \
             empty",
            apply = Verb::Apply,
        ),
        Kind::Directory => format!(
            "`rivet {apply} {DIRECTORY_SUBJECT} <path> [{ACCEPT_FLAG} <text>] [{CONFIRMATION}]`",
            apply = Verb::Apply,
        ),
    }
}

/// Perform one of the two creations.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a malformed command, [`ErrorCode::Unsafe`] for a
/// path this must not create or one that is occupied,
/// [`ErrorCode::ConfirmationRequired`] carrying the plan, and
/// [`ErrorCode::MachineRefused`] when the machine could not do it.
pub(super) fn perform(
    kind: Kind,
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Created, OperationError> {
    let Some(path) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            kind.subject(),
            format!(
                "`{apply} {subject}` needs the ABSOLUTE path to create, as the word after \
                 `{subject}`. {forms}.",
                apply = Verb::Apply,
                subject = kind.subject(),
                forms = forms(kind),
            ),
        ));
    };

    // **`create-file`'s optional text is the ONE positional this family has**,
    // and it is read before the tail so that a text of `confirm` cannot be
    // mistaken for one — `super::edit` refuses that spelling for the same
    // reason and `super::install` refuses a payload spelled `confirm`.
    let (text, tail_from) = match kind {
        Kind::Directory => ("", 2),
        Kind::File => match arguments.get(2).copied() {
            Some(word) if word != CONFIRMATION && word != ACCEPT_FLAG => (word, 3),
            _ => ("", 2),
        },
    };
    if text.len() > super::edit::TEXT_MAX_BYTES {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            kind.subject(),
            format!(
                "the text carries {given} bytes and the most this build accepts is {most} — \
                 `apply {edit}`'s cap, and the same argument: a change that size is not one \
                 word of a command.",
                given = text.len(),
                most = super::edit::TEXT_MAX_BYTES,
                edit = super::edit::SUBJECT,
            ),
        ));
    }

    let tail = read_tail(kind.subject(), &forms(kind), arguments, tail_from)?;
    check_writable_path(&format!("{} {}", Verb::Apply, kind.subject()), path)?;
    let hazard = hazard_for(path);
    super::apply::accepted(tail.accept, hazard.as_ref(), tail.confirmed)?;

    resolve(
        Creation {
            kind,
            path,
            text,
            typed: arguments,
            hazard,
        },
        tail.confirmed,
        board,
    )
}

/// **The change one of these subjects is**, as `ADR-006` §6.5 defines one.
struct Creation<'a> {
    kind: Kind,
    path: &'a str,
    text: &'a str,
    typed: &'a [&'a str],
    hazard: Option<Hazard>,
}

/// What planning a creation worked out.
struct Prepared {
    directories: Vec<String>,
    principal: Principal,
}

impl Change for Creation<'_> {
    type Prepared = Prepared;
    type Applied = Created;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path;
        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        self.check_nothing_is_there(&stat)?;

        // The parents come from the reader, which is the one thing that knows
        // them — `FileRead::missing_parents`, ordered outermost first, and the
        // plan names every one because a creation is never silent.
        let read = board.read_file(path).map_err(|e| refused(e, path))?;
        let directories = read.missing_parents.clone();

        let principal = board.principal();
        match self.kind {
            Kind::File => board
                .can_write_file(path, self.text)
                .map_err(|e| refused(e, path))?,
            Kind::Directory => board
                .can_change_path(&PathChange::CreateDirectory {
                    path,
                    parents: &directories,
                    mode: Kind::Directory.mode(),
                })
                .map_err(|e| refused(e, path))?,
        }

        let zone = super::filesystem::app_zone_clause(path);
        let warning = self
            .hazard
            .as_ref()
            .map_or_else(String::new, |h| format!(" {}", h.warning()));

        let mut plan = Plan::new(
            path,
            format!("creates a {what} on this machine", what = self.kind.word()),
        );
        for directory in &directories {
            plan = plan.doing(
                Effect::create(directory.clone()).noting(format!(
                    "a PARENT that is not there — mode {mode}",
                    mode = describe_mode(super::edit::NEW_DIRECTORY_MODE),
                )),
            );
        }

        let effect = match self.kind {
            Kind::File => {
                let diff = super::diff::unified(
                    "",
                    self.text,
                    "nothing (this path is empty)",
                    path,
                );
                Effect::create(path)
                    .becomes(format!("{} bytes", self.text.len()))
                    .noting(format!(
                        "mode {mode}, and NOTHING IS THERE NOW — this operation refuses a path \
                         that is occupied, because creating is not clobbering.{zone}{warning} \
                         {diff}",
                        mode = describe_mode(Kind::File.mode()),
                        diff = if self.text.is_empty() {
                            "IT WOULD BE EMPTY: no text was given, which is a form of this \
                             command and not an omission."
                                .to_string()
                        } else {
                            diff.note()
                        },
                    ))
            }
            Kind::Directory => Effect::create(path).noting(format!(
                "mode {mode}, and EMPTY. Nothing is put in it.{zone}{warning}",
                mode = describe_mode(Kind::Directory.mode()),
            )),
        };

        plan = plan
            .doing(effect)
            .leaving(
                "nothing that already exists is touched — this operation creates, and refuses \
                 anything already at the path",
            )
            .taking_effect(TakesEffect::NotAsserted {
                because: "this creates a path you chose and nothing in this build says what \
                          reads it — the PATH exists as soon as this returns",
            })
            .leaving(
                "THE PATH IS EXAMINED AGAIN AT THE MOMENT THIS ACTS: if something has appeared \
                 there, the creation is REFUSED rather than replacing it. That check is the \
                 whole of what makes creating not clobbering, and it is re-run rather than \
                 trusted from this plan — but note that nothing carries a plan from one command \
                 to the next, so a `confirm` typed later is checked against the machine THEN",
            )
            .confirmed_by(&confirmation_words(self.typed, self.hazard.as_ref()));

        if directories.is_empty() {
            plan = plan.leaving("no directory is created");
        }

        Ok((
            plan,
            Prepared {
                directories,
                principal,
            },
        ))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Created, OperationError> {
        let path = self.path;
        // The guard against a stale plan. **It is the whole safety story of a
        // creation**: without it, a file that appeared between the plan and the
        // confirmation would be replaced by one nobody described.
        let now = board.stat_path(path).map_err(|e| refused(e, path))?;
        self.check_nothing_is_there(&now)?;

        let wrote = match self.kind {
            Kind::File => board
                .write_file(&FileWrite {
                    path,
                    contents: self.text,
                    directories: &prepared.directories,
                    creating: true,
                    how: "create",
                    was: 0,
                })
                .map_err(|e| refused(e, path))?,
            Kind::Directory => board
                .change_path(&PathChange::CreateDirectory {
                    path,
                    parents: &prepared.directories,
                    mode: Kind::Directory.mode(),
                })
                .map_err(|e| refused(e, path))?,
        };

        Ok(Created {
            path: path.to_string(),
            kind: self.kind,
            bytes: self.text.len(),
            directories: prepared.directories,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Creation<'_> {
    /// **The refusal that makes creating not clobbering.** See this module's
    /// documentation.
    fn check_nothing_is_there(&self, stat: &PathStat) -> Result<(), OperationError> {
        let Some(there) = stat.kind else {
            return Ok(());
        };
        let path = self.path;
        let advice = if there == super::machine::PathKind::File && self.kind == Kind::File {
            format!(
                "TO CHANGE WHAT IS IN IT, `rivet {apply} {edit} {path} --expect <text> \
                 --replace <text> {CONFIRMATION}`, which makes you state what you believe is \
                 there and be right (ADR-006 §8.9.2). TO SEE WHAT IS IN IT FIRST, `rivet {get} \
                 {file} {path}`. There is no form of this command that replaces a file, on \
                 purpose: that would be the blind clobber §8.9.2 exists to prevent, reached by \
                 a different word.",
                apply = Verb::Apply,
                edit = super::edit::SUBJECT,
                get = Verb::Get,
                file = super::file::SUBJECT,
            )
        } else if there == super::machine::PathKind::Directory && self.kind == Kind::Directory {
            "IT IS ALREADY A DIRECTORY, so there is nothing to do — and this is a refusal \
             rather than a quiet success on purpose (ADR-006 §8.10.8): reporting that this \
             operation created something it did not create would be asserting an act that did \
             not happen, and 'it was already there' is what you needed to know."
                .to_string()
        } else {
            format!(
                "`rivet {get} {file} {path}` says what is there.",
                get = Verb::Get,
                file = super::file::SUBJECT,
            )
        };
        Err(unsafe_for(
            path,
            format!(
                "REFUSED there is already a {there} at {path} and NOTHING HAS BEEN CREATED. It \
                 is {what}. {advice}",
                what = describe_stat(stat),
            ),
        ))
    }
}

#[cfg(test)]
#[path = "create_tests.rs"]
mod tests;
