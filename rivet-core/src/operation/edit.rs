//! `rivet apply edit <path> --expect <text> --replace <text> confirm` — **the
//! operation an agent uses to change a file, and the refusals that make it
//! safe.**
//!
//! Ruled by the maintainer, 2026-08-24. `ADR-006` §8.9 carries the decision;
//! `ADR-012` §8 carries why it is the ADR about agents that needed it.
//!
//! # The gap this closes, stated as it was put
//!
//! `ADR-012` §2 says an agent administering a Rivet host needs structured
//! output, typed errors, a discoverable surface, and plan-before-apply — and
//! that Rivet *"is not being made AI-friendly, it is being made legible"*. All
//! of that was true of every operation in this catalogue and none of it was true
//! of **changing a file**, because there was no operation for it. There is no
//! `sed` in this image, no `cat`, no editor, and `rivet` is the login shell,
//! precisely so that nothing else can be run.
//!
//! An agent asked to change a line of `config.nu` therefore had exactly one
//! move: improvise in nushell.
//!
//! ```text
//! open config.nu | str replace "show_banner = true" "show_banner = false" | save -f config.nu
//! ```
//!
//! That has **no atomicity** — the pipeline truncates the file before it writes
//! it, so a machine that loses power mid-pipe loses the file. It has **no
//! check** that the text it matched is the text the agent believed was there. It
//! will happily rewrite a **projected view** that the next boot regenerates, and
//! report success. And it leaves **no record at all** that anything happened.
//! That is not a first-class operator; that is improvising.
//!
//! # The safety primitive is replace-exactly-once, and it is the whole story
//!
//! [`perform`] refuses when `--expect` matches **zero** times and refuses when
//! it matches **more than once**. One rule, and what it buys is that *an agent
//! physically cannot blind-clobber a file*: to change anything it must state
//! what it believes is there and be correct.
//!
//! **Ambiguity is a refusal, never a guess.** Two matches is not "the first
//! one"; it is a request this operation cannot perform, and the refusal names
//! the line of every match so the next attempt can be specific. `AGENTS.md` §5 —
//! *refuse rather than guess* — and the reason it bites harder here than
//! anywhere else is that a wrong guess does not fail. It succeeds, and writes the
//! wrong line.
//!
//! # Two operations, and the bar for a third is deliberately very high
//!
//! `--expect`/`--replace`, and `--append`. That is the whole vocabulary, and
//! `ADR-006` §8.9 rules that this must grow **refusals** rather than
//! expressiveness. Every extra verb here — insert-at-line, delete-matching,
//! regular expressions — is a step towards being `sed`, and `sed` is the tool
//! this exists to avoid: it has no notion of what the caller believed was there.
//!
//! **What is deliberately absent, so it is not mistaken for an oversight:**
//!
//! * **No regular expressions.** A literal is what an agent can be *correct*
//!   about; a pattern is what it can be *nearly* correct about, and the two
//!   failure modes are not comparable.
//! * **No replace-all.** It is the exact opposite of this operation's rule.
//! * **No delete-file, no rename, no chmod.** Each is a different act with a
//!   different plan; none of them is an edit.
//! * **No reading.** `apply edit` does not print the file. That is `get`'s verb
//!   and no subject of it reads a file yet — see `PLAN.md`'s note, because the
//!   asymmetry is real: an agent can change a file it cannot read back.
//!
//! # What it refuses, and why each one is the same kind of refusal
//!
//! | Refused | Because |
//! | :-- | :-- |
//! | A **view** — anything `rivet explain` answers for, and the whole of `/etc` | `ADR-007` §3.6.2: a view is a read-only projection of state owned elsewhere. The refusal says what to change instead, in `explain`'s own words |
//! | A **mechanism path** — [`reserved_paths`] | The file is consumed by a mechanism; use the mechanism. `/state/deploy-request` written by hand forges a principal and bypasses `rivet apply install` |
//! | A file that is **not UTF-8** | A text edit on bytes that are not text is a guess about an encoding |
//! | A file **larger than [`FILE_MAX_BYTES`]** | This runs in the login shell and reads the file whole |
//! | `--expect` matching **0 or >1** times | The primitive above |
//! | `--append` text that would **continue an unterminated last line** | Joining is almost never meant, and a line joined onto a comment disappears into it while the file still parses; inserting a newline would be a guess |
//!
//! **The mechanism-path refusal is NOT a security boundary, and says so.** Same
//! root, same filesystem: anything that can run this command could write the
//! file with nushell's `save`. It is the same *class* of refusal as a view —
//! *this file is consumed by a mechanism, use the mechanism* — and the refusal
//! carries that sentence rather than implying an authority it does not have.
//! `ADR-028` §4.5 makes the identical point about the install request's
//! principal.
//!
//! # One source of truth with `explain`, not two
//!
//! `explain` is the read half of the question *may I write this path*, and this
//! is the write half. They answer from the same table: [`perform`] calls
//! [`super::explain::perform`] and refuses with the explanation's own
//! `change-with` sentence. A second list of unwritable paths here is exactly the
//! second authority `ADR-011` §3 is about, and it would be read at the moment
//! somebody is already confused.
//!
//! # Time-of-check to time-of-use, and why the primitive is also the answer
//!
//! A file can change between the plan and the confirmation, and nothing here
//! locks it. **The confirmed path re-reads and re-counts**, so a file whose
//! `--expect` no longer occurs exactly once is refused at `confirm` — the guard
//! against a stale plan is the same rule as the guard against a blind clobber,
//! and there is no second mechanism to get wrong.
//!
//! `--append` has no such guard and does not need one: appending is
//! order-independent, and the only thing that can change is whether the file had
//! to be created. If it appeared between the plan and the confirmation, the
//! confirmation appends to it and the result says `created: false` — which is
//! what happened.

use super::apply::{ACCEPT_FLAG, CONFIRMATION, confirmed};
use super::machine::{Board, MachineError, Write as FileWrite};
use super::plan::{Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::deploy::{self, Principal};
use crate::etc::ETC_MOUNT_POINT;
use crate::service::APP_ROOT;
use crate::state::LOG_DIR;
use crate::value::Value;

/// The subject. `ADR-006` §6.4's nouns are values, not catalogue keys.
pub const SUBJECT: &str = "edit";

/// The text that must already be there, exactly once.
pub const EXPECT_FLAG: &str = "--expect";
/// What to put in its place.
pub const REPLACE_FLAG: &str = "--replace";
/// Text to add to the end of the file, creating it if it is not there.
pub const APPEND_FLAG: &str = "--append";

/// The mode a file this operation **creates** is created with.
///
/// `0600`, ruled with the operation. `crate::deploy::REQUEST_MODE`'s reasoning
/// applies unchanged and is worth restating because the direction is the
/// surprising one: a new file starts **private and is widened deliberately**,
/// never the reverse. An operation that created world-readable files and left an
/// operator to notice would put a secret in one exactly once, which is once too
/// often (`ADR-020`).
///
/// **A file that already exists keeps the mode it has.** This operation changes
/// contents; changing permissions is a different act with a different plan, and
/// it is not one of the two this build performs.
pub const NEW_FILE_MODE: u32 = 0o600;

/// The mode a **directory** this operation creates is created with.
///
/// `0700`, matching `crate::state::LOG_DIR_MODE` for its reason: `ADR-015`
/// §5.1's application identities are not uid 0 and must not be able to read into
/// a tree an operator created for uid 0's own files.
pub const NEW_DIRECTORY_MODE: u32 = 0o700;

/// The most any one of `--expect`, `--replace` and `--append` may carry.
///
/// **64 KiB, and the number is about what a change *is* rather than about
/// memory.** A configuration file is measured in kilobytes; a change of 64 KiB
/// is not an edit, it is a file, and this operation has no form that writes one
/// whole. Saying so is a refusal an agent can act on, where an `E2BIG` from
/// `execve(2)` — which is what the alternative eventually produces — is not.
pub const TEXT_MAX_BYTES: usize = 64 * 1024;

/// The largest file this operation will read.
///
/// **1 MiB.** It reads the file whole, in the process that is the login shell,
/// on a machine with no swap. A cap that refuses is better
/// than an allocation that succeeds on a workstation and takes the board's
/// memory; and a file this size is not one anybody is editing a line of.
///
/// The same number as `crate::state::LOG_FILE_MAX_BYTES`, and the agreement is a
/// coincidence rather than a dependency — the journal's cap is an SD-wear
/// decision (`ADR-022` §8) and this one is a memory decision.
pub const FILE_MAX_BYTES: usize = 1024 * 1024;

/// How many matching lines a refusal names before it stops listing them.
const MATCHES_LISTED: usize = 8;

// ---------------------------------------------------------------------------
// The reserved set
// ---------------------------------------------------------------------------

/// One path this operation refuses because a **mechanism** owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reserved {
    /// The exact path, or the directory whose contents are reserved.
    pub path: &'static str,
    /// `true` when everything **under** [`Self::path`] is reserved too.
    pub tree: bool,
    /// What consumes it.
    pub mechanism: &'static str,
    /// What to do instead. Never "edit it anyway".
    pub instead: &'static str,
}

/// **Every path this operation refuses because a mechanism owns it.**
///
/// Deliberately short — `ADR-006` §8.9 says a *small* reserved set — and every
/// entry is a path some other constant in this crate already names, so there is
/// no second spelling of any of them to go stale (`ADR-011`).
///
/// **What is NOT here, said rather than left to be inferred:**
///
/// * **`/state/config`, the desired-state document.** It is the file an agent
///   most wants to change and `ADR-006` §6.4's `set` and `apply <document>` do
///   not exist, so reserving it would leave no way to change it at all.
///   `PLAN.md` carries the question of what happens when they do.
/// * **`/state/clock`.** It is `rivet-store`'s path and `rivet-core` may not
///   depend on that crate (`ADR-014` §3), so naming it here would be the second
///   spelling this function exists to avoid. It is text, tiny, and nobody edits
///   it by hand. Recorded as a known gap in `PLAN.md` rather than papered over.
pub fn reserved_paths() -> Vec<Reserved> {
    vec![
        Reserved {
            path: deploy::REQUEST_PATH,
            tree: false,
            mechanism: "the install channel — cmd/rivet-sysd reads this file on its next \
                        convergence pass and hands the payload it names to the installer",
            instead: "`rivet apply install <path.rvt> confirm`, which writes it with the \
                      session that asked recorded in it (ADR-028 §4.5). A request written by \
                      hand would name whoever it liked",
        },
        Reserved {
            path: deploy::REQUEST_TEMP_PATH,
            tree: false,
            mechanism: "the install channel's half-written request, which is renamed into place",
            instead: "`rivet apply install <path.rvt> confirm`",
        },
        Reserved {
            path: LOG_DIR,
            tree: true,
            mechanism: "this machine's journal (ADR-022 §7), which the supervisor writes",
            instead: "`rivet get log` to read it. A record written by hand is a record of \
                      something that did not happen, and the journal is the only account this \
                      machine keeps of what it did",
        },
        Reserved {
            path: APP_ROOT,
            tree: true,
            mechanism: "the installer, which unpacks an application here after verifying every \
                        member's digest (ADR-015 §7, ADR-029 §11)",
            instead: "`rivet apply install <path.rvt> confirm` with a rebuilt package. An \
                      application edited in place no longer matches what was signed, and the \
                      next install replaces it",
        },
    ]
}

/// The reserved entry a path falls under, if any.
///
/// # The application tree is THREE ZONES and not one — amended 2026-08-29
///
/// [`reserved_paths`]'s [`APP_ROOT`] entry is `tree: true`, and until this date
/// that meant *every* path under `/state/app` was refused. **The maintainer
/// ruled that too broad**, and `ADR-006` §8.10.4.4 carries the argument:
///
/// > permit, and inform — do not refuse.
///
/// The reason is that the control plane is the only channel. A blanket refusal
/// on the application tree recreates exactly the gap §8.10 exists to close — what
/// was once done by hand has to be doable by the surface. And what actually differs
/// between paths in that tree is **trust and persistence**, which is a matter
/// for the *plan* rather than for a refusal: a program is digest-covered and
/// re-verified at every launch, a file in a version directory is silently
/// discarded by the next update, and `data` persists (`ADR-015` §4.2.1, §7,
/// §5.1.1).
///
/// **Three paths are still reserved** — the application root, an application's
/// own directory, and the `current` link — and each names the mechanism that
/// owns it. The rest is permitted, and
/// [`super::filesystem::app_zone_clause`] is what every writing subject's plan
/// carries instead of a refusal.
///
/// **The `mechanism` and `instead` sentences come from
/// [`super::filesystem::app_zone`]**, so there is one account of what owns each
/// of the three rather than one here and one there (`ADR-011`).
pub(super) fn reserved_for(path: &str) -> Option<Reserved> {
    if let Some(zone) = super::filesystem::app_zone(path) {
        return match zone {
            super::filesystem::AppZone::Mechanism(mechanism) => Some(Reserved {
                path: APP_ROOT,
                tree: true,
                mechanism,
                instead: "`rivet apply install <path.rvt> confirm` to add or replace an \
                          application, and `rivet apply uninstall <name> confirm` to remove \
                          one. Files INSIDE an installed version are not refused — `rivet get \
                          file` reads them and the writing subjects of ADR-006 §8.10 change \
                          them, each saying in its plan whether the change will survive the \
                          next update",
            }),
            // Permitted, and the plan says what follows. See this function's
            // documentation.
            _ => None,
        };
    }
    reserved_paths().into_iter().find(|r| {
        r.path == path || (r.tree && path.starts_with(r.path) && path.as_bytes().get(r.path.len()) == Some(&b'/'))
    })
}

// ---------------------------------------------------------------------------
// The catalogue's half
// ---------------------------------------------------------------------------

/// What an edit's result carries.
///
/// **The order is constrained rather than chosen** — `super::install`'s
/// `GET_OUTPUTS` records why: `super::apply`'s published union is generated by
/// merging the subjects' lists, and two subjects that order the same pair of
/// fields differently produce no valid union at all. The two names shared with
/// the other subjects, `subject` and `next`, appear here in that same relative
/// order, and `every_subjects_fields_survive_the_generated_union` in `super` is
/// what fails if a later edit forgets.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `edit` here.",
    },
    Field {
        name: "path",
        description: "The file that was replaced, exactly as it was given.",
    },
    Field {
        name: "change",
        description: "Which of the two changes this was: `replace` or `append`.",
    },
    Field {
        name: "created",
        description: "Whether there was no file at this path before. A created file is mode \
                      0600 — see `mode`.",
    },
    Field {
        name: "line",
        description: "`replace` ONLY: the line the matched text began on, counting from 1. \
                      Absent for an append.",
    },
    Field {
        name: "before-bytes",
        description: "How many bytes the file held before. 0 for a file that was created.",
    },
    Field {
        name: "after-bytes",
        description: "How many bytes it holds now. This operation replaces the file WHOLE, so \
                      these two are the entire change.",
    },
    Field {
        name: "directories",
        description: "Every parent directory that had to be created, outermost first, each mode \
                      0700. ABSENT when none were — a creation is never silent (ADR-006 §8.9).",
    },
    Field {
        name: "mode",
        description: "The mode a CREATED file was created with, octal. Absent when the file was \
                      already there, because this operation does not change permissions.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked, in a sentence — over \
                      ADR-035's control plane the caller's PINNED ED25519 KEY, and where a \
                      login session carried the request instead, that session (ADR-028 §4.5). \
                      It is ATTRIBUTION AND NOT AUTHORITY: nothing checked it, and anything \
                      running as uid 0 could have written the file itself.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of this edit reached this machine's journal. FALSE DOES \
                      NOT MEAN THE EDIT FAILED — the file is written either way, and `unrecorded` \
                      says what stopped the record.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of this edit. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle, and where to read it back.",
    },
];

// ---------------------------------------------------------------------------
// The result
// ---------------------------------------------------------------------------

/// Which of the two changes was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// `--expect` matched exactly once and `--replace` took its place.
    Replace,
    /// `--append` was added to the end of the file.
    Append,
}

impl Change {
    /// The wire form, and the word the record carries. Part of the interface
    /// contract; never renamed.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Append => "append",
        }
    }
}

impl core::fmt::Display for Change {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One file that was replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    path: String,
    change: Change,
    created: bool,
    /// The line the match began on. `None` for an append.
    line: Option<usize>,
    before: usize,
    after: usize,
    directories: Vec<String>,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Edited {
    /// The file that was replaced.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Which change was made.
    pub const fn change(&self) -> Change {
        self.change
    }

    /// Whether this operation created the file.
    pub const fn created(&self) -> bool {
        self.created
    }

    /// Whether a record of it reached the journal.
    pub const fn journalled(&self) -> bool {
        self.unrecorded.is_none()
    }

    /// The structured result, whose keys are a subsequence of `OUTPUTS` — in
    /// `OUTPUTS`'s order, checked here rather than only in a test because the
    /// catalogue naming a field no result carries is `ADR-006` §7's *"trusted
    /// and wrong"*.
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("path", Value::str(self.path.clone())),
            ("change", Value::str(self.change.as_str())),
            ("created", Value::Bool(self.created)),
        ];
        if let Some(line) = self.line {
            fields.push(("line", Value::Integer(count(line))));
        }
        fields.push(("before-bytes", Value::Integer(count(self.before))));
        fields.push(("after-bytes", Value::Integer(count(self.after))));
        if !self.directories.is_empty() {
            fields.push((
                "directories",
                Value::strings(self.directories.iter().map(String::as_str)),
            ));
        }
        if self.created {
            fields.push(("mode", Value::str(format!("{NEW_FILE_MODE:04o}"))));
        }
        fields.push(("principal", Value::str(describe_who(&self.principal))));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push(("next", Value::str(next_after_edit(self.journalled()))));
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "an edit result's keys must be a subsequence of OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// Who asked, in this operation's framing of [`Principal::describes`].
///
/// **One notion of who is speaking, two acts.** `crate::deploy::Principal` was
/// built for the install request and its `describe` says *"the request was
/// written by `rivet apply install`"*, which would be false here — so this
/// supplies the three words that differ and nothing else. Everything about what
/// a client address is worth, what a key label is and is not, and that none of
/// it was checked, is written once in that module (`ADR-011`).
///
/// The forgery sentence is the one worth reading twice: an edit is *more*
/// forgeable than an install request, because there is no mechanism to bypass —
/// anything running as uid 0 writes the file with `save` and leaves no record.
/// `ADR-028` §4.5's point exactly, and saying it here is what stops a reader
/// promoting the record into a permission.
fn describe_who(principal: &Principal) -> String {
    principal.describes(
        "the file",
        format!("`rivet {apply} {SUBJECT}`", apply = Verb::Apply).as_str(),
        "anything running as uid 0 could have written this file another way and left no record \
         at all — there is no mechanism here to bypass, which is why this refuses a mechanism's \
         paths rather than pretending to guard them",
    )
}

/// A count that has to become an `i64` for [`Value::Integer`].
///
/// Saturating rather than `as`, on `cmd/rivet-deploy`'s reasoning: a length
/// that wrapped negative would make the one field saying how big the file is
/// unreadable, and this crate denies the lint that would have caught it.
fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// What the operator should do and expect after a file has been replaced.
fn next_after_edit(journalled: bool) -> String {
    let get = Verb::Get;
    let record = if journalled {
        format!(
            "THIS EDIT IS IN THE JOURNAL, naming the session that asked (ADR-028 §4.5) — \
             `rivet {get} log` is where it appears, as a record of service `rivet`."
        )
    } else {
        format!(
            "THIS EDIT IS NOT IN THE JOURNAL and `unrecorded` says why. The file is written \
             either way; what is missing is the account of it. `rivet {get} log` will not \
             show this change."
        )
    };
    format!(
        "the file on this machine is now what this answer describes, and the write was atomic \
         — nothing could have read it half-replaced. {record} NOTHING HAS BEEN RELOADED: a \
         service reading this file picks the change up when it next reads it, and the \
         supervisor converges on its own pass (ADR-017 §3.2). This operation does not restart \
         anything and cannot say whether the change is any good."
    )
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// What the caller asked for, once the words have been read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request<'a> {
    path: &'a str,
    change: Wanted<'a>,
    /// `ADR-012` §7.3's acknowledgement, where one was given.
    accept: Option<&'a str>,
    confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Wanted<'a> {
    Replace { expect: &'a str, replace: &'a str },
    Append(&'a str),
}

/// An argument refusal, spelled once.
fn arguments(subject: impl Into<String>, message: impl Into<String>) -> OperationError {
    OperationError::about(ErrorCode::Arguments, subject, message)
}

/// The two forms, in one sentence, for every refusal that has to show them.
fn forms() -> String {
    format!(
        "`rivet {apply} {SUBJECT} <path> {EXPECT_FLAG} <text> {REPLACE_FLAG} <text> \
         [{CONFIRMATION}]` or `rivet {apply} {SUBJECT} <path> {APPEND_FLAG} <text> \
         [{CONFIRMATION}]`",
        apply = Verb::Apply,
    )
}

/// Read the words after `edit`.
///
/// **Every word is accounted for.** There is no position here that is ignored,
/// and an unknown word is refused by name rather than skipped — which is
/// `super::apply::confirmed`'s rule about `confrim`, applied to a command with
/// more places to make that mistake in. It is also the check that replaces
/// `super::perform`'s arity ceiling, which this operation gives up by declaring
/// a several-word input (`super::Input::several`).
fn read_arguments<'a>(arguments_in: &'a [&'a str]) -> Result<Request<'a>, OperationError> {
    let Some(path) = arguments_in.get(1).copied() else {
        return Err(arguments(
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the ABSOLUTE path of the file to change, as the word \
                 after `{SUBJECT}`. {forms}. There is no default and nothing is searched for: a \
                 file is named.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };

    let mut expect: Option<&str> = None;
    let mut replace: Option<&str> = None;
    let mut append: Option<&str> = None;
    // **`ADR-012` §7.3's acknowledgement, added 2026-08-29 — and it is NOT an
    // expressiveness.** `ADR-006` §8.9.3 rules that this operation grows
    // refusals rather than verbs, and this grows neither: it changes nothing
    // about what an operator may *ask for*. It is the confirmation model, which
    // is `ADR-012`'s and applies to every subject uniformly, and `edit` needs it
    // because §8.10.4.4 made an application's program editable — the one path
    // where a change stops the machine working, later, at a launch nobody is
    // watching.
    let mut accept: Option<&str> = None;
    let mut confirmation: Option<&str> = None;

    let mut i = 2usize;
    while i < arguments_in.len() {
        let word = arguments_in[i];
        let flag = match word {
            EXPECT_FLAG => Some(&mut expect),
            REPLACE_FLAG => Some(&mut replace),
            APPEND_FLAG => Some(&mut append),
            ACCEPT_FLAG => Some(&mut accept),
            _ => None,
        };
        if let Some(slot) = flag {
            if slot.is_some() {
                return Err(arguments(
                    word,
                    format!(
                        "`{word}` is given twice. This operation makes ONE change, so there is \
                         no form of it that takes two — and reading the second as the one that \
                         counts would silently discard the first."
                    ),
                ));
            }
            let Some(text) = arguments_in.get(i + 1).copied() else {
                return Err(arguments(
                    word,
                    format!("`{word}` needs the text after it, and this is the last word."),
                ));
            };
            // **A flag word, or the confirmation, where the text should be, is a
            // FORGOTTEN VALUE and not a literal.** `--append confirm` would
            // otherwise append the word `confirm` to a file and report success —
            // a plausible-looking result for a command that meant something
            // else. `super::install` refuses a payload spelled `confirm` for
            // exactly this reason. Any OTHER text beginning with `--` is
            // accepted, because a configuration file legitimately contains such
            // lines and refusing them would make whole files uneditable.
            if matches!(
                text,
                EXPECT_FLAG | REPLACE_FLAG | APPEND_FLAG | ACCEPT_FLAG | CONFIRMATION
            ) {
                return Err(arguments(
                    text,
                    format!(
                        "`{word}` is followed by `{text}`, which is one of this command's own \
                         words rather than text — so `{word}` has no value. If `{text}` really \
                         is the text you mean, this build has no way to say so and that is a \
                         refusal rather than a guess."
                    ),
                ));
            }
            *slot = Some(text);
            i += 2;
            continue;
        }

        // **`--expect=text` is refused rather than accepted as a second
        // spelling.** `Verb::parse`'s rule: a surface with two spellings of one
        // thing must be memorised rather than understood (`ADR-006` §6.4).
        if let Some((name, _)) = word.split_once('=')
            && matches!(name, EXPECT_FLAG | REPLACE_FLAG | APPEND_FLAG)
        {
            return Err(arguments(
                word,
                format!(
                    "this build spells it `{name} <text>`, with a space, and has no `=` form. \
                     One spelling, so that a command can be predicted rather than memorised."
                ),
            ));
        }

        if word.starts_with('-') {
            return Err(arguments(
                word,
                format!(
                    "`{apply} {SUBJECT}` takes {EXPECT_FLAG}, {REPLACE_FLAG}, {APPEND_FLAG} and \
                     — where this machine warns about a path (ADR-012 §7.3) — {ACCEPT_FLAG}, \
                     and nothing else. {forms}.",
                    apply = Verb::Apply,
                    forms = forms(),
                ),
            ));
        }

        // The only bare word this command has is the confirmation, and it is the
        // LAST word. Anything else here is refused rather than ignored.
        if i + 1 != arguments_in.len() {
            return Err(arguments(
                word,
                format!(
                    "`{word}` is a word this command has no meaning for. The only bare word \
                     `{apply} {SUBJECT}` takes is `{CONFIRMATION}`, as the LAST word. {forms}.",
                    apply = Verb::Apply,
                    forms = forms(),
                ),
            ));
        }
        confirmation = Some(word);
        i += 1;
    }

    let change = read_change(expect, replace, append)?;
    Ok(Request {
        path,
        change,
        accept,
        confirmed: confirmed(confirmation)?,
    })
}

/// Which of the two changes the flags describe, or a refusal naming the two
/// forms.
fn read_change<'a>(
    expect: Option<&'a str>,
    replace: Option<&'a str>,
    append: Option<&'a str>,
) -> Result<Wanted<'a>, OperationError> {
    if append.is_some() && (expect.is_some() || replace.is_some()) {
        return Err(arguments(
            APPEND_FLAG,
            format!(
                "`{APPEND_FLAG}` adds text to the end of a file and \
                 `{EXPECT_FLAG}`/`{REPLACE_FLAG}` changes text inside it. They are two \
                 different changes and this operation makes one. {forms}.",
                forms = forms(),
            ),
        ));
    }
    match (expect, replace, append) {
        (Some(expect), Some(replace), None) => Ok(Wanted::Replace { expect, replace }),
        (Some(_), None, None) => Err(arguments(
            EXPECT_FLAG,
            format!(
                "`{EXPECT_FLAG}` says what must already be there and `{REPLACE_FLAG}` says what \
                 to put in its place; without the second there is no change to make. To DELETE \
                 the text, say so: `{REPLACE_FLAG} \"\"`."
            ),
        )),
        (None, Some(_), None) => Err(arguments(
            REPLACE_FLAG,
            format!(
                "`{REPLACE_FLAG}` needs `{EXPECT_FLAG}` to say what it is replacing. THAT IS THE \
                 WHOLE SAFETY RULE of this operation (ADR-006 §8.9): a change must state what it \
                 believes is there and be right, and a replace with nothing to match is a blind \
                 clobber."
            ),
        )),
        (_, _, Some(append)) => Ok(Wanted::Append(append)),
        (None, None, None) => Err(arguments(
            SUBJECT,
            format!(
                "this names a file and no change to make. {forms}.",
                forms = forms(),
            ),
        )),
    }
}

// ---------------------------------------------------------------------------
// The path
// ---------------------------------------------------------------------------

/// Everything that is true of the **path** rather than of the file.
///
/// Answered before anything is read, so a refusal here has touched nothing at
/// all.
fn check_path(path: &str) -> Result<(), OperationError> {
    if path.is_empty() {
        return Err(arguments(
            path,
            format!(
                "this names no file at all. {forms}.",
                forms = forms(),
            ),
        ));
    }

    if !path.starts_with('/') {
        return Err(arguments(
            path,
            format!(
                "a file is named by an ABSOLUTE path. `{path}` is relative, and this process's \
                 working directory is whatever the thing that started it happened to be in — \
                 over ADR-035's control plane that is the control-plane daemon's, and it is \
                 nowhere the caller chose. So it would resolve somewhere neither you nor this \
                 command chose. The failure mode is the bad one: it resolves to SOMETHING often \
                 enough that the answer would name a file you never typed."
            ),
        ));
    }

    // **A control character in a path is refused, and a newline is the one that
    // matters.** `AGENTS.md` §5 asks for what the operator said to be reported
    // rather than paraphrased, and a path carrying a newline cannot be reported:
    // it is one line of a plan that reads as two, and one line of a journal
    // record that reads as an operator having edited a file that does not exist.
    // The refusal carries the bytes back verbatim, which is the one place they
    // are safe to put — a refusal is already the exceptional path.
    // The predicate is `super::filesystem`'s since 2026-08-29, and it is the ONE
    // thing this gate shares with `ADR-006` §8.10's — a fact whose answer could
    // not sensibly differ between two subjects. The sentence below stays this
    // operation's, because a refusal has to say what to do next and that differs
    // (`super::filesystem`'s module documentation).
    if let Some((at, byte)) = super::filesystem::control_character(path) {
        return Err(arguments(
            path,
            format!(
                "this path carries a control character — byte {byte:#04x} at offset {at} — and \
                 this operation will not write a file whose name it cannot report back on one \
                 line. A newline is the one that matters: a plan and a journal record are both \
                 lines, and a path that breaks one of them describes an edit that did not \
                 happen. Rivet creates no such names, so this is almost always a quoting \
                 mistake in the command that produced it."
            ),
        ));
    }

    if path.ends_with('/') {
        return Err(arguments(
            path,
            "this names a directory rather than a file. `apply edit` replaces the contents of \
             one file and creates no directories except the parents of the file it is writing.",
        ));
    }

    // **A `.`, `..` or empty (`//`) component is refused — a second spelling
    // of one path, reaching a file every check above tested a different
    // string for.** The adapter that decides whether new content must parse
    // as the machine document does it by comparing this path, verbatim, to
    // one constant; `/state/config/./machine.toml` names the same file on
    // Linux and would reach that adapter having skipped the comparison. One
    // spelling per path is what makes a check on the spelling a check on the
    // path — `super::filesystem`'s own reason for the identical predicate.
    if let Some(component) = super::filesystem::traversal_component(path) {
        return Err(arguments(
            path,
            format!(
                "this path has a `{component}` in it, and `apply edit` reads and writes a path \
                 LITERALLY — it resolves nothing. Say the path you mean."
            ),
        ));
    }

    Ok(())
}

/// Refuse a path Rivet generates, in `explain`'s own words.
///
/// **One source of truth with `explain`, and this function is it.** See this
/// module's documentation.
/// **A view is refused, with no exceptions.**
///
/// This operation once carried an exception list, for exactly one path whose
/// only mechanism was to edit it by hand. There is no such path now: every
/// projection this build can name has a source an operator can change, and
/// `explain` names it. An exception here would be an edit that is accepted,
/// appears to work, and is gone at the next boot — the failure `explain` exists
/// to prevent, reintroduced by the operation that reads it.
fn check_not_a_view(path: &str) -> Result<(), OperationError> {
    match view_refusal(path) {
        Some(refusal) => Err(refusal),
        None => Ok(()),
    }
}

/// **The view refusal itself, without `apply edit`'s one exception.**
///
/// Split out of [`check_not_a_view`] on 2026-08-29 for `ADR-006` §8.10: the
/// file-management subjects refuse a view too, from **this** table, so that a
/// path cannot be a view to one subject and not to another (`ADR-011`).
///
/// `None` means *not a view*, which is the ordinary answer.
pub(super) fn view_refusal(path: &str) -> Option<OperationError> {
    if let Ok(explanation) = super::explain::perform(&[path]) {
        if explanation.writable() {
            return None;
        }
        return Some(OperationError::about(
            ErrorCode::Unsafe,
            path,
            format!(
                "REFUSED {path} is generated by Rivet — {kind}, ADR-007 §3.6.2 — and \
                 NOTHING HAS BEEN WRITTEN. It comes from {from}, and {consequence}. \
                 CHANGE IT BY: {instead}. \
                 `rivet {explain} {path}` is the same answer in full.",
                kind = explanation.kind_word().to_ascii_uppercase(),
                consequence = explanation.consequence(),
                from = explanation.derived_from(),
                instead = explanation.changed_by(),
                explain = Verb::Explain,
            ),
        ));
    }

    // The rest of `/etc`. **Not a view of anything in particular**, which is why
    // `explain` refuses to answer for it — but the whole tree is Rivet's
    // runtime projection (`ADR-007` §3.5.1) on a filesystem that is remounted
    // read-only once it is rendered, so a write there is lost at the next boot
    // in the best case and refused by the kernel in the ordinary one.
    if path.starts_with(ETC_MOUNT_POINT)
        && path.as_bytes().get(ETC_MOUNT_POINT.len()) == Some(&b'/')
    {
        return Some(OperationError::about(
            ErrorCode::Unsafe,
            path,
            format!(
                "REFUSED the whole of {ETC_MOUNT_POINT} is Rivet's runtime projection \
                 (ADR-007 §3.5.1), rendered before any service starts and remounted read-only \
                 once it is — so NOTHING HAS BEEN WRITTEN, and a write here would be discarded \
                 at the next boot even where the kernel allowed it. Rivet does not have an \
                 /etc full of formats (ADR-006 §6.4): what a Rivet machine is, is the \
                 desired-state document. `rivet {explain} <path>` says which paths are \
                 generated and what to change instead.",
                explain = Verb::Explain,
            ),
        ));
    }

    None
}

/// Refuse a path a mechanism owns. See [`reserved_paths`].
///
/// `pub(super)` since 2026-08-29: `ADR-006` §8.10's file-management subjects
/// refuse a mechanism's path from **this** table, so a path cannot be a
/// mechanism's to one subject and not to another (`ADR-011`).
pub(super) fn check_not_reserved(path: &str) -> Result<(), OperationError> {
    let Some(reserved) = reserved_for(path) else {
        return Ok(());
    };
    let what = if reserved.tree && reserved.path != path {
        format!("{path} is inside {}, which belongs to", reserved.path)
    } else {
        format!("{path} belongs to")
    };
    Err(OperationError::about(
        ErrorCode::Unsafe,
        path,
        format!(
            "REFUSED {what} {mechanism}, and NOTHING HAS BEEN WRITTEN. USE THE MECHANISM: \
             {instead}. THIS IS NOT A SECURITY BOUNDARY and must not be read as one — same \
             root, same filesystem, and anything that can run this command could write the file \
             another way. It is the same kind of refusal as a view (ADR-007 §3.6.2): this file \
             is consumed by a mechanism, so use the mechanism, because a file written by hand \
             makes a claim nothing produced.",
            mechanism = reserved.mechanism,
            instead = reserved.instead,
        ),
    ))
}

// ---------------------------------------------------------------------------
// Performing it
// ---------------------------------------------------------------------------

/// The one place a [`MachineError`] becomes this operation's refusal.
///
/// The same mapping `super::stop`, `super::trial` and `super::install` use, for
/// the same reason — an operator does two different things next — and **nothing
/// is reworded on the way through**.
fn refused(error: MachineError, path: &str) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, path, error.why())
}

/// Where every occurrence of `needle` begins in `haystack`.
///
/// **Overlapping**, deliberately, and `str::match_indices` is not: for `aa` in
/// `aaa` this answers twice and that answers once. The stricter count is the
/// right one here, because the question this operation asks is *how many places
/// in this file look like what you said* — and a caller who meant one of two
/// overlapping places has an ambiguity whether or not a scanner would have
/// skipped it.
///
/// An empty needle is refused before this is reached; it would otherwise match
/// at every position, which is the definition of the ambiguity this refuses.
fn occurrences(haystack: &str, needle: &str) -> Vec<usize> {
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let mut at = 0usize;
    while at + needle.len() <= haystack.len() {
        if haystack.is_char_boundary(at) && haystack[at..].starts_with(needle) {
            found.push(at);
        }
        at += 1;
    }
    found
}

/// Which line a byte offset falls on, counting from 1.
fn line_of(text: &str, at: usize) -> usize {
    1 + text[..at].matches('\n').count()
}

/// Perform `apply edit`.
///
/// `arguments` is the whole of what followed the verb, so `arguments[0]` is the
/// subject and `arguments[1]` is the path. Everything after that is read by
/// [`read_arguments`], which accounts for every word.
///
/// # The order of the refusals is part of the design
///
/// Everything that is true of the **request** is refused before the file is
/// read, and everything true of the **path** before that. An agent that typed a
/// view's path gets *"that is a view, change this instead"* whether or not the
/// file is there, whether or not it is UTF-8, and on a machine that has no such
/// filesystem at all — which is the property that makes those refusals
/// reproducible from a workstation.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Edited, OperationError> {
    let request = read_arguments(arguments_in)?;
    let path = request.path;
    check_path(path)?;
    // **Reserved before view, and the order was the other way until 2026-08-25.**
    // Both refuse; the question is which sentence the operator reads. A reserved
    // path names the *mechanism that consumes it* — `rivet apply install` for
    // `/state/deploy-request` — and a view names *what to change instead*. While
    // `explain` knew only `/etc` the two sets could not intersect, so the order
    // could not matter and was not chosen. `rivet_core::generated` made
    // `explain` answer for `/state/deploy-request`, `/state/log` and
    // `/state/app`, and with the old order all three lost their specific
    // sentence for the generic one — which additionally *told the operator
    // something false*, that the next boot would regenerate a file nothing
    // regenerates. The more specific refusal wins.
    check_not_reserved(path)?;
    check_not_a_view(path)?;
    check_text(&request.change)?;

    // **`ADR-012` §7.3's second tier, added 2026-08-29.** This operation reaches
    // it for exactly one reason: `ADR-006` §8.10.4.4 made an application's
    // program and its manifest editable, and those are the two paths in the
    // whole file surface where a change stops the machine working *later*, at a
    // launch nobody is watching. Everything else this operation touches is the
    // ordinary `confirm`, and §7.3.3's doctrine is why — a warning that fires
    // often is not a warning.
    let hazard = super::filesystem::hazard_for(path);
    super::apply::accepted(request.accept, hazard.as_ref(), request.confirmed)?;

    // Everything above this line is true of the REQUEST and refuses without
    // touching a filesystem — see this function's documentation for why that
    // ordering is part of the design. Everything below it needs the file, and
    // that is `super::plan`'s driver rather than this function's.
    resolve(
        EditFile {
            path,
            wanted: request.change,
            typed: arguments_in,
            hazard,
        },
        request.confirmed,
        board,
    )
}

/// **The change `apply edit` is**, as `ADR-006` §6.5 defines one — and **the
/// operation the associated type exists for.**
struct EditFile<'a> {
    path: &'a str,
    wanted: Wanted<'a>,
    /// What the operator typed, for the command that repeats it.
    typed: &'a [&'a str],
    /// `ADR-012` §7.3's warning, where this path has one.
    hazard: Option<super::apply::Hazard>,
}

/// What planning an edit worked out, and what performing it needs.
///
/// Every field here was computed to *describe* the change: the whole new file,
/// where the match was, what the file weighed, which directories are missing.
/// Recomputing them after the confirmation would be a second read of the file
/// and a window between the two in which it could differ from the one described
/// — so [`super::plan::Change::Prepared`] carries them across instead.
struct Prepared {
    change: Change,
    /// The file as it would be after. Written whole and atomically.
    after: String,
    before: usize,
    line: Option<usize>,
    creating: bool,
    directories: Vec<String>,
    principal: Principal,
}

impl super::plan::Change for EditFile<'_> {
    type Prepared = Prepared;
    type Applied = Edited;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path;
        let read = board.read_file(path).map_err(|e| refused(e, path))?;
        let creating = read.contents.is_none();
        let before = match &read.contents {
            None => String::new(),
            Some(bytes) => readable(path, bytes)?,
        };

        let (after, line) = match self.wanted {
            Wanted::Append(text) => {
                // Planning runs on both paths, so a confirmation refuses too.
                starts_a_line(path, &before, text)?;
                (format!("{before}{text}"), None)
            }
            Wanted::Replace { expect, replace } => {
                // **The refusal that is this operation's whole safety story**,
                // and it runs while planning — so it runs on both paths.
                // `ADR-006` §8.9.2's replace-exactly-once.
                let at = single_match(path, &before, expect, replace, creating)?;
                let mut after = String::with_capacity(before.len() + replace.len());
                after.push_str(&before[..at]);
                after.push_str(replace);
                after.push_str(&before[at + expect.len()..]);
                (after, Some(line_of(&before, at)))
            }
        };

        let change = match self.wanted {
            Wanted::Append(_) => Change::Append,
            Wanted::Replace { .. } => Change::Replace,
        };
        let principal = board.principal();
        board.can_write_file(path, &after).map_err(|e| refused(e, path))?;

        let mut plan = Plan::new(path, "changes a file on this machine");

        // The directories first, in the order they would be made, because that
        // is the order they happen in and because a path that creates four of
        // them is usually a typo (`ADR-006` §8.9).
        for directory in &read.missing_parents {
            plan = plan.doing(
                Effect::create(directory.clone())
                    .noting(format!("mode {mode:04o}", mode = NEW_DIRECTORY_MODE)),
            );
        }

        // The note is assembled once rather than layered, because
        // `Effect::noting` REPLACES: how the text lands, whether the file is
        // being made, and — `ADR-028` §4.5 — who would be recorded as having
        // done it. **That last one rides on the effect rather than beside it**:
        // it is a fact about the write and not something the change leaves
        // alone, which is what `untouched` means.
        let how = match change {
            Change::Replace => format!(
                "the text `{EXPECT_FLAG}` names is at LINE {line}, exactly once, and would be \
                 replaced there and nowhere else",
                line = line.unwrap_or(0),
            ),
            Change::Append => format!(
                "the text `{APPEND_FLAG}` names would be added to the END of the file, exactly \
                 as given, on a line of its own — no newline is added and none is removed"
            ),
        };
        let creation = if creating {
            format!(
                ". THE FILE DOES NOT EXIST AND WOULD BE CREATED, mode {mode:04o}",
                mode = NEW_FILE_MODE,
            )
        } else {
            ". the file is written WHOLE and atomically — beside itself, flushed, and renamed \
             over it"
                .to_string()
        };
        // **The diff, and it is `ADR-006` §8.10.7's highest-value placement of
        // one.** `ADR-012` §7.1 makes the plan the safety mechanism, and *"3
        // bytes will change"* is a much weaker one than the change itself: an
        // agent whose `--expect` matched the wrong one of two similar stanzas
        // cannot tell from a byte count and can tell instantly from a diff.
        //
        // It is OUTPUT, not input, so §8.9.3's rule that this operation grows
        // no expressiveness is untouched — nothing about what may be *asked
        // for* has changed.
        let shown = super::diff::unified(&before, &after, path, format!("{path} (after)").as_str());
        let zone = super::filesystem::app_zone_clause(path);
        let warning = self
            .hazard
            .as_ref()
            .map_or_else(String::new, |h| format!(" {}", h.warning()));
        let note = format!(
            "{how}{creation}. WHO WOULD BE RECORDED: {who}{zone}{warning} {diff}",
            who = describe_who(&principal),
            diff = shown.note(),
        );

        let file = if creating {
            Effect::create(path).becomes(format!("{} bytes", after.len()))
        } else {
            Effect::set(
                path,
                format!("{} bytes", before.len()),
                format!("{} bytes", after.len()),
            )
        };

        plan = plan
            .doing(file.noting(note))
            .leaving(
                "NOTHING HAS BEEN WRITTEN — the file is replaced only if this is confirmed",
            )
            .leaving(format!(
                "THE FILE IS READ AGAIN WHEN YOU CONFIRM: if it has changed in the meantime so \
                 that `{EXPECT_FLAG}` no longer matches exactly once, the confirmation is \
                 refused rather than applied to a file this plan did not describe"
            ))
            // **The `NotAsserted` this whole three-state exists for.**
            // `ADR-034` §5.3 wants a change that is not live to say so, and this
            // operation writes a path the operator chose: its safety story is
            // replace-exactly-once and a reserved deny-list (`ADR-006` §8.9),
            // not a schema, so nothing in this build knows what reads the file.
            // Claiming either answer would be the omission §5.3 forbids,
            // pointing the other way.
            .taking_effect(TakesEffect::NotAsserted {
                because: "this writes a path you chose and nothing in this build says what reads \
                          it — the FILE changes as soon as this returns",
            })
            // **The acknowledgement is spliced in where there is one**, so the
            // line an operator pastes back is complete — `ADR-012` §7.3's token
            // is meant to require *reading* the plan, not transcribing it out
            // of prose.
            .confirmed_by(&match &self.hazard {
                None => vec![retype(self.typed)],
                Some(hazard) => {
                    let mut words = vec![retype(&without_acknowledgement(self.typed))];
                    words.extend(hazard.accept_words());
                    words
                }
            });

        if read.missing_parents.is_empty() {
            plan = plan.leaving("no directory is created");
        }

        Ok((
            plan,
            Prepared {
                change,
                after,
                before: before.len(),
                line,
                creating,
                directories: read.missing_parents,
                principal,
            },
        ))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Edited, OperationError> {
        let wrote = board
            .write_file(&FileWrite {
                path: self.path,
                contents: &prepared.after,
                directories: &prepared.directories,
                creating: prepared.creating,
                how: prepared.change.as_str(),
                was: count(prepared.before).unsigned_abs(),
            })
            .map_err(|e| refused(e, self.path))?;

        Ok(Edited {
            path: self.path.to_string(),
            change: prepared.change,
            created: prepared.creating,
            line: prepared.line,
            before: prepared.before,
            after: prepared.after.len(),
            directories: prepared.directories,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

/// Everything that is true of the **text** the caller supplied.
fn check_text(change: &Wanted<'_>) -> Result<(), OperationError> {
    let (longest, which) = match change {
        Wanted::Append(text) => (text.len(), APPEND_FLAG),
        Wanted::Replace { expect, replace } => {
            if expect.len() >= replace.len() {
                (expect.len(), EXPECT_FLAG)
            } else {
                (replace.len(), REPLACE_FLAG)
            }
        }
    };
    if longest > TEXT_MAX_BYTES {
        return Err(arguments(
            which,
            format!(
                "`{which}` carries {longest} bytes and the most this build accepts is \
                 {TEXT_MAX_BYTES}. A change that size is not an edit, it is a file, and this \
                 operation has no form that writes one whole."
            ),
        ));
    }

    match change {
        Wanted::Append("") => Err(arguments(
            APPEND_FLAG,
            format!(
                "`{APPEND_FLAG}` was given no text, so nothing would be added. If the intent was \
                 to CREATE an empty file, this build has no operation for that and saying so is \
                 better than writing one you did not ask for."
            ),
        )),
        // **An empty `--expect` matches at every position in every file**, which
        // is precisely the ambiguity this operation exists to refuse. Refused
        // by name rather than by the counting below, because "your text matched
        // 4193 times" is a true answer that would send an agent looking at the
        // file instead of at its own command.
        Wanted::Replace { expect: "", .. } => Err(arguments(
            EXPECT_FLAG,
            format!(
                "`{EXPECT_FLAG}` was given no text. An empty string occurs everywhere in every \
                 file, so there is no file for which this could match exactly once — which is \
                 the rule this operation is built on. Say the text that is actually there."
            ),
        )),
        Wanted::Replace { expect, replace } if expect == replace => Err(arguments(
            REPLACE_FLAG,
            format!(
                "`{EXPECT_FLAG}` and `{REPLACE_FLAG}` carry the SAME text, so this change would \
                 rewrite the file to exactly what it already holds. Refused rather than \
                 performed, because a write that changes nothing still replaces the file, still \
                 costs the card a block, and still puts a record in the journal saying an edit \
                 happened."
            ),
        )),
        _ => Ok(()),
    }
}

/// **An append that would continue the file's last line is refused.**
///
/// The text is added exactly as given and nothing is inserted, so appending to
/// a file that does not end in a newline, with text that does not begin with
/// one, joins the two onto one line. That is almost never what was meant, and
/// the case that matters fails silently: when the last line is a comment, an
/// appended table header becomes part of the comment and the document still
/// parses, so the machine is not what was written and nothing says so.
///
/// Inserting a newline would be a guess about what the caller meant
/// (`AGENTS.md` §5), so the ambiguous case is refused and the refusal names
/// both unambiguous spellings. The last line is described rather than quoted:
/// this operation also appends to secret files, and a refusal is not a reader.
fn starts_a_line(path: &str, before: &str, text: &str) -> Result<(), OperationError> {
    let file_ends_a_line = before.is_empty() || before.ends_with(['\n', '\r']);
    let text_starts_a_line = text.starts_with(['\n', '\r']);
    if file_ends_a_line || text_starts_a_line {
        return Ok(());
    }
    Err(OperationError::about(
        ErrorCode::Unsafe,
        path,
        format!(
            "REFUSED {path} does not end with a newline and the `{APPEND_FLAG}` text does not \
             begin with one, so the text would CONTINUE the file's last line instead of \
             starting a line of its own. NOTHING HAS BEEN WRITTEN. `apply {SUBJECT}` adds \
             exactly the text it is given and inserts nothing, so it refuses rather than guess \
             which was meant — and when that last line is a comment, appended text becomes part \
             of the comment while the file still reads as valid. To add a line, begin the text \
             with a newline. To change the last line itself, use `{EXPECT_FLAG}` and \
             `{REPLACE_FLAG}`."
        ),
    ))
}

/// The file's bytes as text, or the refusal that says why they are not.
fn readable(path: &str, bytes: &[u8]) -> Result<String, OperationError> {
    if bytes.len() > FILE_MAX_BYTES {
        return Err(OperationError::about(
            ErrorCode::Unsafe,
            path,
            format!(
                "REFUSED {path} holds {held} bytes and this operation reads a file whole; the \
                 most it will read is {FILE_MAX_BYTES}. NOTHING HAS BEEN WRITTEN. This runs in \
                 the machine's login shell on a board with no swap, and a \
                 file this size is not one anybody is changing a line of.",
                held = bytes.len(),
            ),
        ));
    }
    core::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|e| {
            OperationError::about(
                ErrorCode::Unsafe,
                path,
                format!(
                    "REFUSED {path} is not UTF-8 — the first byte this build cannot read is at \
                     offset {at} — so it is not text, and NOTHING HAS BEEN WRITTEN. `apply \
                     {SUBJECT}` matches and replaces TEXT; doing that to bytes that are not text \
                     means guessing an encoding, and a wrong guess here does not fail, it \
                     silently corrupts the file (AGENTS.md §5).",
                    at = e.valid_up_to(),
                )
            )
        })
}

/// **The safety primitive.** Where `expect` occurs, if it occurs exactly once.
///
/// Zero and many are both refusals and they are deliberately *different*
/// refusals, because the next thing to do is different: a caller with zero
/// matches has the wrong text or has already made the change, and a caller with
/// several needs a longer one.
fn single_match(
    path: &str,
    text: &str,
    expect: &str,
    replace: &str,
    creating: bool,
) -> Result<usize, OperationError> {
    if creating {
        return Err(OperationError::about(
            ErrorCode::Unsafe,
            path,
            format!(
                "REFUSED there is no file at {path}, so there is nothing for `{EXPECT_FLAG}` to \
                 match and NOTHING HAS BEEN WRITTEN. `{EXPECT_FLAG}` describes text that must \
                 ALREADY be there. To create the file, `{APPEND_FLAG} <text>` does it — and says \
                 so in its plan, with the mode and every directory it would create."
            ),
        ));
    }

    let found = occurrences(text, expect);
    match found.len() {
        1 => Ok(found[0]),
        0 => {
            // **The one case worth a second sentence**, because it is the one an
            // agent hits after a retry: the change is already made, and the
            // honest answer is that rather than "your text is not there".
            let already = if occurrences(text, replace).len() == 1 && !replace.is_empty() {
                format!(
                    " THE REPLACEMENT TEXT IS ALREADY THERE, EXACTLY ONCE — this edit looks like \
                     one that has already been made. `apply {SUBJECT}` is not idempotent and \
                     cannot be: a second run has no way to tell 'already done' from 'the file is \
                     not what you think', so it refuses and says both."
                )
            } else {
                String::new()
            };
            Err(OperationError::about(
                ErrorCode::Unsafe,
                path,
                format!(
                    "REFUSED `{EXPECT_FLAG}` matches NOTHING in {path}, so NOTHING HAS BEEN \
                     WRITTEN. The file holds {bytes} bytes on {lines} lines. Retrying will not \
                     help: either the text is not what is in the file — whitespace and \
                     indentation are part of it — or this is not the file you meant.{already}",
                    bytes = text.len(),
                    lines = text.lines().count(),
                ),
            ))
        }
        many => {
            let mut lines: Vec<String> = found
                .iter()
                .take(MATCHES_LISTED)
                .map(|at| line_of(text, *at).to_string())
                .collect();
            if many > MATCHES_LISTED {
                lines.push(format!("and {} more", many - MATCHES_LISTED));
            }
            Err(OperationError::about(
                ErrorCode::Unsafe,
                path,
                format!(
                    "REFUSED `{EXPECT_FLAG}` matches {many} places in {path} — lines {lines} — \
                     and NOTHING HAS BEEN WRITTEN. THIS OPERATION REPLACES EXACTLY ONE \
                     OCCURRENCE AND NEVER GUESSES WHICH (ADR-006 §8.9): picking the first would \
                     succeed, and would write the wrong line. Say more text: include the line \
                     above it, or the whole stanza, until it names one place.",
                    lines = lines.join(", "),
                ),
            ))
        }
    }
}

/// The words the operator typed, minus any acknowledgement they gave.
///
/// The plan shows the one that would be **accepted**, which may not be the one
/// that was typed — a caller who guessed the phrase must see the right one
/// rather than their own guess echoed back at them.
fn without_acknowledgement<'a>(words: &'a [&'a str]) -> Vec<&'a str> {
    match words.iter().position(|word| *word == ACCEPT_FLAG) {
        Some(at) => words[..at].to_vec(),
        None => words
            .iter()
            .filter(|word| **word != CONFIRMATION)
            .copied()
            .collect(),
    }
}

/// The command to repeat, spelled so it can be **typed back**.
///
/// Words are joined with spaces — the confirmation is one invocation, not
/// several — and each word is emitted bare or single-quoted, never bare when
/// it is not safe to be. **A word is left bare only when every character in
/// it is one of `[A-Za-z0-9_./:=@%+,-]`**: an ordinary path, a digest, or a
/// flag such as `--append`. Every other word is single-quoted, which is the
/// right quoting for both shells that can reach this command — a POSIX `sh`
/// and `nushell` both treat a single-quoted string as literal, with no
/// escapes and no expansion, so `$HOME`, a glob character, a shell
/// metacharacter (`;`, `|`, `&`, a backtick), a double quote, a backslash and
/// an embedded newline all survive as the byte the operator typed, rather
/// than being expanded, split, or read as syntax when the command is typed
/// back.
///
/// A word containing a single quote cannot be quoted this way and is **said**
/// rather than mangled: there is no spelling of it this function could
/// produce that both shells read alike, and printing a broken command would
/// be worse than admitting one.
fn retype(words: &[&str]) -> String {
    let mut out = String::new();
    for word in words {
        if !out.is_empty() {
            out.push(' ');
        }
        if word.contains('\'') {
            out.push_str("<the text you typed, which contains a ' and cannot be quoted here>");
        } else if is_bare_safe(word) {
            out.push_str(word);
        } else {
            out.push('\'');
            out.push_str(word);
            out.push('\'');
        }
    }
    out
}

/// Whether every character of `word` is safe to print unquoted in the command
/// [`retype`] builds — the allowlist named in its doc comment.
///
/// An allowlist rather than a denylist: the set of bytes that are ordinary in
/// a path, a digest or a flag is small and closed, while the set that is
/// active syntax in some shell somewhere is not — a denylist has to be
/// complete to be safe, and an allowlist only has to be correct about what it
/// admits. An empty word is never bare: `''` is how `retype` spells one, and
/// a bare empty word would vanish from the line instead.
fn is_bare_safe(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:=@%+,-".contains(c))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::deploy::Origin;
    use crate::etc::PASSWD;
    use crate::operation::machine::{Editor, FileRead, Wrote};
    use std::collections::BTreeMap;

    // **This operation edits a file and does nothing else**, and since
    // 2026-08-26 it reaches `super::plan::resolve`, which takes the whole
    // `Board`. The other three ports refuse rather than pretend, so an edit
    // test that started stopping the machine fails loudly (`AGENTS.md` §3).
    crate::operation::machine::refuses_to_manage!(FakeEditor);
    crate::operation::machine::refuses_to_deploy!(FakeEditor);
    crate::operation::machine::refuses_to_observe!(FakeEditor);

    /// A filesystem in a map, and a record of what it was asked to write.
    struct FakeEditor {
        files: BTreeMap<String, Vec<u8>>,
        directories: Vec<String>,
        missing_parents: Vec<String>,
        read_fails: Option<MachineError>,
        interlock: Option<MachineError>,
        write_fails: Option<MachineError>,
        unrecorded: Option<String>,
        /// Every write that was performed, in order.
        wrote: Vec<(String, String, bool, Vec<String>)>,
        /// Whether the plan path asked the interlock.
        asked: Vec<String>,
    }

    impl FakeEditor {
        fn new() -> Self {
            Self {
                files: BTreeMap::new(),
                directories: Vec::new(),
                missing_parents: Vec::new(),
                read_fails: None,
                interlock: None,
                write_fails: None,
                unrecorded: None,
                wrote: Vec::new(),
                asked: Vec::new(),
            }
        }

        fn holding(path: &str, contents: &str) -> Self {
            let mut fake = Self::new();
            fake.files
                .insert(path.to_string(), contents.as_bytes().to_vec());
            fake
        }

        fn contents(&self, path: &str) -> String {
            String::from_utf8(self.files.get(path).cloned().unwrap_or_default())
                .expect("the fake was asked to write bytes that are not text")
        }
    }

    impl Editor for FakeEditor {
        fn principal(&mut self) -> Principal {
            Principal::new(
                Origin::pinned(&"0123456789abcdef".repeat(4)).expect("64 hex characters"),
            )
        }

        fn read_file(&mut self, path: &str) -> Result<FileRead, MachineError> {
            if let Some(why) = &self.read_fails {
                return Err(why.clone());
            }
            Ok(FileRead {
                contents: self.files.get(path).cloned(),
                missing_parents: self.missing_parents.clone(),
            })
        }

        // `_contents` because this double answers on the PATH alone - what a
        // real machine checks about the bytes is its own business, and a
        // fake that inspected them would be testing this file's idea of
        // that rather than the machine's.
        fn can_write_file(&mut self, path: &str, _contents: &str) -> Result<(), MachineError> {
            self.asked.push(path.to_string());
            match &self.interlock {
                Some(why) => Err(why.clone()),
                None => Ok(()),
            }
        }

        fn write_file(&mut self, write: &FileWrite<'_>) -> Result<Wrote, MachineError> {
            if let Some(why) = &self.write_fails {
                return Err(why.clone());
            }
            self.directories.extend(write.directories.iter().cloned());
            self.files
                .insert(write.path.to_string(), write.contents.as_bytes().to_vec());
            self.wrote.push((
                write.path.to_string(),
                write.contents.to_string(),
                write.creating,
                write.directories.to_vec(),
            ));
            Ok(Wrote {
                unrecorded: self.unrecorded.clone(),
            })
        }

        // **`ADR-006` §8.10's four refuse here**, added 2026-08-29. This
        // operation changes a file's contents and does nothing else; a test in
        // this module that started removing or renaming one fails loudly rather
        // than passing on a fiction (`AGENTS.md` §3). `super::super::filesystem`
        // carries the fake that answers.
        fn stat_path(
            &mut self,
            _path: &str,
        ) -> Result<crate::operation::machine::PathStat, MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn resolve_owner(
            &mut self,
            _user: Option<&str>,
            _group: Option<&str>,
        ) -> Result<crate::operation::machine::OwnerIds, MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn can_change_path(
            &mut self,
            _change: &crate::operation::machine::PathChange<'_>,
        ) -> Result<(), MachineError> {
            Err(Self::not_this_ports_business())
        }

        fn change_path(
            &mut self,
            _change: &crate::operation::machine::PathChange<'_>,
        ) -> Result<Wrote, MachineError> {
            Err(Self::not_this_ports_business())
        }
    }

    impl FakeEditor {
        fn not_this_ports_business() -> MachineError {
            MachineError::Refused(
                "this fake edits files and does nothing else; an `apply edit` test that reached \
                 ADR-006 §8.10's port has changed what it is about"
                    .to_string(),
            )
        }
    }

    fn edit(fake: &mut FakeEditor, words: &[&str]) -> Result<Edited, OperationError> {
        perform(words, fake)
    }

    fn refusal(fake: &mut FakeEditor, words: &[&str]) -> OperationError {
        edit(fake, words).expect_err("that command must be refused")
    }

    const NU: &str = "/state/home/root/.config/nushell/config.nu";

    // -- the safety primitive --------------------------------------------------

    /// **The whole operation, in one test.** The maintainer's own example: a
    /// line that occurs exactly once is replaced, once, and the rest of the file
    /// is untouched.
    #[test]
    fn a_line_that_occurs_exactly_once_is_replaced_and_nothing_else_changes() {
        let mut fake = FakeEditor::holding(
            NU,
            "# nushell\n$env.config.show_banner = true\n$env.config.edit_mode = \"vi\"\n",
        );
        let done = edit(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "$env.config.show_banner = true",
                REPLACE_FLAG,
                "$env.config.show_banner = false",
                CONFIRMATION,
            ],
        )
        .expect("one match is the case this operation is for");

        assert_eq!(
            fake.contents(NU),
            "# nushell\n$env.config.show_banner = false\n$env.config.edit_mode = \"vi\"\n"
        );
        assert_eq!(done.change(), Change::Replace);
        assert!(!done.created());
        assert_eq!(done.line, Some(2));
        assert_eq!(done.before, 70);
        assert_eq!(done.after, 71);
    }

    /// **Zero matches is a refusal and never a write.** An agent that guessed
    /// the text wrong must be stopped, not helped along.
    #[test]
    fn text_that_is_not_there_is_refused_with_e_unsafe_and_writes_nothing() {
        let mut fake = FakeEditor::holding(NU, "$env.config.show_banner = true\n");
        let refused = refusal(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "show_banner = TRUE",
                REPLACE_FLAG,
                "x",
                CONFIRMATION,
            ],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert_eq!(refused.subject(), Some(NU));
        assert!(refused.message().contains("matches NOTHING"), "{}", refused.message());
        assert!(
            refused.message().contains("NOTHING HAS BEEN WRITTEN"),
            "{}",
            refused.message()
        );
        assert!(fake.wrote.is_empty(), "{:?}", fake.wrote);
        assert_eq!(fake.contents(NU), "$env.config.show_banner = true\n");
    }

    /// **Two matches is a refusal, never the first one.** Picking one would
    /// succeed and write the wrong line, which is the failure this whole
    /// operation exists to make impossible.
    #[test]
    fn text_that_occurs_twice_is_refused_and_the_refusal_names_both_lines() {
        let mut fake = FakeEditor::holding(NU, "enabled = true\nname = x\nenabled = true\n");
        let refused = refusal(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "enabled = true",
                REPLACE_FLAG,
                "enabled = false",
                CONFIRMATION,
            ],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(refused.message().contains("matches 2 places"), "{}", refused.message());
        assert!(refused.message().contains("lines 1, 3"), "{}", refused.message());
        assert!(refused.message().contains("NEVER GUESSES"), "{}", refused.message());
        assert!(fake.wrote.is_empty());
    }

    /// And a longer `--expect` disambiguates, which is what the refusal tells
    /// the caller to do. The two tests are a pair on purpose: the advice in the
    /// refusal has to actually work.
    #[test]
    fn a_longer_expect_narrows_two_matches_to_one_and_succeeds() {
        let mut fake = FakeEditor::holding(NU, "enabled = true\nname = x\nenabled = true\n");
        edit(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "name = x\nenabled = true",
                REPLACE_FLAG,
                "name = x\nenabled = false",
                CONFIRMATION,
            ],
        )
        .expect("the longer text names one place");
        assert_eq!(
            fake.contents(NU),
            "enabled = true\nname = x\nenabled = false\n"
        );
    }

    /// Overlapping occurrences are counted, and `str::match_indices` would not
    /// count them. The stricter answer is the right one — see [`occurrences`].
    #[test]
    fn overlapping_occurrences_are_ambiguous_rather_than_one_match() {
        assert_eq!(occurrences("aaa", "aa"), vec![0, 1]);
        let mut fake = FakeEditor::holding("/state/x", "aaa");
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "aa", REPLACE_FLAG, "b", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(refused.message().contains("matches 2 places"), "{}", refused.message());
    }

    /// **The retry an agent will actually make.** Having already applied the
    /// change, `--expect` no longer matches — and the refusal says the
    /// replacement is already there rather than leaving the agent to conclude it
    /// has the wrong file.
    #[test]
    fn a_change_that_has_already_been_made_is_refused_and_says_so() {
        let mut fake = FakeEditor::holding(NU, "show_banner = false\n");
        let refused = refusal(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "show_banner = true",
                REPLACE_FLAG,
                "show_banner = false",
                CONFIRMATION,
            ],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(
            refused.message().contains("ALREADY THERE"),
            "{}",
            refused.message()
        );
        assert!(
            refused.message().contains("not idempotent"),
            "{}",
            refused.message()
        );
    }

    // -- the plan --------------------------------------------------------------

    /// **Without `confirm`, nothing is written.** `ADR-012` §7.1, and the plan
    /// carries every fact the ruling named.
    #[test]
    fn without_the_confirmation_it_plans_and_writes_nothing() {
        let mut fake = FakeEditor::holding(NU, "a\nshow_banner = true\nb\n");
        let planned = refusal(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "show_banner = true",
                REPLACE_FLAG,
                "show_banner = false",
            ],
        );
        assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
        assert_eq!(planned.subject(), Some(NU));
        let said = planned.message();
        assert!(said.contains("LINE 2"), "where it matched: {said}");
        assert!(said.contains("NOTHING HAS BEEN WRITTEN"), "{said}");
        assert!(said.contains(CONFIRMATION), "{said}");

        // **The byte counts and the directory answer are FIELDS since
        // 2026-08-26**, not `23 bytes to 24` and `No directory would be
        // created` in a sentence. `ADR-006` §6.5: this operation is the one that
        // made the framework worth building, because an install's plan is one
        // sentence and an edit's is a diff.
        let typed = planned.plan().expect("a confirmation refusal carries its plan");
        let [file] = typed.effects() else {
            panic!("one file, no directories: {:?}", typed.effects());
        };
        assert_eq!(file.act, crate::operation::Act::Set);
        assert_eq!(file.target, NU);
        assert_eq!(file.from.as_deref(), Some("23 bytes"));
        assert_eq!(file.to.as_deref(), Some("24 bytes"));
        assert!(
            typed.untouched().iter().any(|l| l == "no directory is created"),
            "{:?}",
            typed.untouched()
        );
        // `ADR-034` §5.3's third state, and this is the operation it exists for:
        // an arbitrary path is one Rivet cannot say is live anywhere.
        assert_eq!(typed.takes_effect().requires_reboot(), None);
        assert_eq!(typed.takes_effect().as_str(), "not-asserted");
        // **...and the command to repeat can actually be typed back.** The
        // first version of this joined the words with spaces and produced a
        // command whose `--expect` took one word of four; a guest found it.
        assert!(
            said.contains(
                "rivet apply edit /state/home/root/.config/nushell/config.nu \
                 --expect 'show_banner = true' --replace 'show_banner = false' confirm"
            ),
            "{said}"
        );
        assert!(fake.wrote.is_empty());
        // The interlock ran on the plan path — `super::stop`'s rule.
        assert_eq!(fake.asked, vec![NU.to_string()]);
    }

    /// A plan that would create the file says the mode, and a plan that would
    /// create directories names **every one of them**, in order. `ADR-006`
    /// §8.9: a creation reads as intent rather than as a silent `mkdir -p`.
    #[test]
    fn a_plan_that_creates_names_the_mode_and_every_directory() {
        let mut fake = FakeEditor::new();
        fake.missing_parents = vec![
            "/state/home/root/.config".to_string(),
            "/state/home/root/.config/nushell".to_string(),
        ];
        let planned = refusal(&mut fake, &[SUBJECT, NU, APPEND_FLAG, "$env.x = 1\n"]);
        assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
        let said = planned.message();
        assert!(said.contains("WOULD BE CREATED, mode 0600"), "{said}");

        // **Every directory is its own `create` effect, in the order it
        // happens**, rather than a comma-separated list inside one sentence.
        // That is what an agent needs to see four of them and stop.
        let typed = planned.plan().expect("a confirmation refusal carries its plan");
        let created: Vec<&str> = typed
            .effects()
            .iter()
            .filter(|e| e.act == crate::operation::Act::Create)
            .map(|e| e.target.as_str())
            .collect();
        assert_eq!(
            created,
            [
                "/state/home/root/.config",
                "/state/home/root/.config/nushell",
                NU,
            ],
            "the directories come first, in the order they are made, and the file last"
        );
        assert!(
            typed.effects()[0]
                .note
                .as_deref()
                .is_some_and(|n| n.contains("0700")),
            "{:?}",
            typed.effects()[0]
        );
    }

    /// The plan says who would be recorded, **before** anything is written. An
    /// operator who discovers their session in the journal afterwards was not
    /// told.
    #[test]
    fn the_plan_says_who_would_be_recorded() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let planned = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b"],
        );
        assert!(
            planned.message().contains(&"0123456789abcdef".repeat(4)),
            "{}",
            planned.message()
        );
        assert!(
            planned.message().contains("WHO WOULD BE RECORDED"),
            "{}",
            planned.message()
        );
    }

    /// A misspelled confirmation is refused rather than read as absence —
    /// `super::apply::confirmed`'s rule, reached through this operation's own
    /// argument reader.
    #[test]
    fn a_misspelled_confirmation_is_refused_and_not_taken_as_a_plan() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        for word in ["confrim", "yes", "--confirm", "CONFIRM"] {
            let refused = refusal(
                &mut fake,
                &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", word],
            );
            assert_eq!(refused.code(), ErrorCode::Arguments, "{word}");
            assert!(fake.wrote.is_empty());
        }
    }

    // -- creating --------------------------------------------------------------

    /// `--append` on a file that is not there creates it, at [`NEW_FILE_MODE`],
    /// and the result says both.
    #[test]
    fn append_creates_a_missing_file_and_reports_the_mode_it_created_it_with() {
        let mut fake = FakeEditor::new();
        fake.missing_parents = vec!["/state/home/root/.config".to_string()];
        let done = edit(
            &mut fake,
            &[SUBJECT, NU, APPEND_FLAG, "$env.config.show_banner = false\n", CONFIRMATION],
        )
        .expect("a creation");
        assert!(done.created());
        assert_eq!(done.before, 0);
        assert_eq!(fake.contents(NU), "$env.config.show_banner = false\n");
        assert_eq!(fake.directories, vec!["/state/home/root/.config".to_string()]);

        let Value::Object(fields) = done.value() else {
            panic!("a result is an object");
        };
        assert!(fields.iter().any(|(k, v)| *k == "mode" && *v == Value::str("0600")));
        assert!(fields.iter().any(|(k, _)| *k == "directories"));
    }

    /// **`--expect` cannot create.** It describes text that must already be
    /// there, and a file that is not there has none.
    #[test]
    fn expect_against_a_file_that_does_not_exist_is_refused_and_names_append() {
        let mut fake = FakeEditor::new();
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/new", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(refused.message().contains("there is no file"), "{}", refused.message());
        assert!(refused.message().contains(APPEND_FLAG), "{}", refused.message());
    }

    /// An append adds **exactly** the bytes it was given. Nothing is inserted:
    /// text without a trailing newline leaves the file without one.
    #[test]
    fn an_append_adds_exactly_what_it_was_given_and_invents_no_newline() {
        let mut fake = FakeEditor::holding("/state/x", "first\n");
        edit(&mut fake, &[SUBJECT, "/state/x", APPEND_FLAG, "second", CONFIRMATION])
            .expect("an append");
        assert_eq!(fake.contents("/state/x"), "first\nsecond");
    }

    /// **Text that would continue an unterminated last line is refused**, at
    /// the plan and at the confirmation alike, and nothing is written. The
    /// refusal names both unambiguous spellings.
    #[test]
    fn an_append_that_would_continue_the_last_line_is_refused() {
        for words in [
            vec![SUBJECT, "/state/x", APPEND_FLAG, "second"],
            vec![SUBJECT, "/state/x", APPEND_FLAG, "second", CONFIRMATION],
        ] {
            let mut fake = FakeEditor::holding("/state/x", "first");
            let refused = refusal(&mut fake, &words);
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{}", refused.message());
            assert!(refused.message().contains("CONTINUE"), "{}", refused.message());
            assert!(refused.message().contains(EXPECT_FLAG), "{}", refused.message());
            assert!(fake.wrote.is_empty());
            assert_eq!(fake.contents("/state/x"), "first");
        }
    }

    /// **The shape that darkened a screen**: a desired-state document whose
    /// last line is a comment with no newline after it, and a table appended
    /// without a leading newline. Joined, the table header would sit inside
    /// the comment and its key would land in the table before it — a document
    /// that still parses. Refused instead, and the refusal does not echo the
    /// file's content.
    #[test]
    fn a_table_appended_onto_a_trailing_comment_is_refused() {
        const DOCUMENT: &str = "version = 1\n[kiosk]\nauto-run = \"aura-renderer\"\n# trailing";
        let mut fake = FakeEditor::holding("/state/config/machine.toml", DOCUMENT);
        let refused = refusal(
            &mut fake,
            &[
                SUBJECT,
                "/state/config/machine.toml",
                APPEND_FLAG,
                "[services.rivet-displayd]\nenabled = true\n",
                CONFIRMATION,
            ],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe, "{}", refused.message());
        assert!(!refused.message().contains("# trailing"), "{}", refused.message());
        assert!(fake.wrote.is_empty());
        assert_eq!(fake.contents("/state/config/machine.toml"), DOCUMENT);
    }

    /// The spellings that are NOT ambiguous all go through: text beginning
    /// with a newline (LF or CRLF), a file that ends with one (LF or CRLF),
    /// and an empty file, which has no last line to continue.
    #[test]
    fn an_append_that_starts_a_line_is_written() {
        for (held, text, written) in [
            ("first", "\nsecond", "first\nsecond"),
            ("first", "\r\nsecond", "first\r\nsecond"),
            ("first\r\n", "second", "first\r\nsecond"),
            ("", "second", "second"),
        ] {
            let mut fake = FakeEditor::holding("/state/x", held);
            edit(&mut fake, &[SUBJECT, "/state/x", APPEND_FLAG, text, CONFIRMATION])
                .unwrap_or_else(|e| panic!("{held:?} + {text:?}: {}", e.message()));
            assert_eq!(fake.contents("/state/x"), written, "{held:?} + {text:?}");
        }
    }

    /// **Through the whole planner, not just `diff::unified` in isolation.**
    /// Appending only a newline changes the file's bytes — this operation's
    /// own contract just above, "invents no newline", cuts both ways — and
    /// the plan an operator reads before confirming must not claim nothing
    /// would change just because a line-based diff sees the same one line
    /// either way.
    #[test]
    fn appending_only_a_newline_is_never_planned_as_identical() {
        let mut fake = FakeEditor::holding("/state/x", "first");
        let planned = refusal(&mut fake, &[SUBJECT, "/state/x", APPEND_FLAG, "\n"]);
        assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
        let plan = planned.plan().expect("a confirmation refusal carries a plan");
        let narrative = plan.narrative();
        assert!(!narrative.contains("IDENTICAL"), "{narrative}");
        assert!(narrative.to_lowercase().contains("line ending"), "{narrative}");
        assert!(fake.wrote.is_empty());
    }

    // -- what it refuses about the path ---------------------------------------

    /// **A generated path is refused in `explain`'s own words.** One source of
    /// truth: the refusal repeats what `rivet explain` says to change instead,
    /// and it names the *kind* rather than calling everything a view.
    ///
    /// The last path is why the wording changed on 2026-08-25. A socket is not
    /// a view, and until `rivet_core::generated` existed this operation could
    /// not have been asked about it.
    #[test]
    fn a_generated_path_is_refused_and_says_what_to_change_instead() {
        let mut fake = FakeEditor::new();
        let cases = [
            (PASSWD, "VIEW"),
            ("/etc/shells", "VIEW"),
            ("/etc/hosts", "VIEW"),
            (crate::journal::INGEST_SOCKET_PATH, "MECHANISM"),
        ];
        for (path, kind) in cases {
            let refused = refusal(
                &mut fake,
                &[SUBJECT, path, APPEND_FLAG, "x\n", CONFIRMATION],
            );
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{path}");
            assert_eq!(refused.subject(), Some(path));
            assert!(
                refused.message().contains(kind),
                "{path}: the refusal does not say {kind}: {}",
                refused.message()
            );
            let explanation =
                super::super::explain::perform(&[path]).expect("that path is generated");
            assert!(
                refused.message().contains(explanation.changed_by()),
                "{path}: the refusal does not repeat explain's answer: {}",
                refused.message()
            );
            assert!(
                refused.message().contains(explanation.consequence()),
                "{path}: the refusal does not say what a write would do: {}",
                refused.message()
            );
        }
        assert!(fake.wrote.is_empty());
    }

    /// **There is no exception list, and this is the test that says so.**
    ///
    /// Every path `explain` reports as not writable is refused by this
    /// operation. An exception would be an edit that is accepted, appears to
    /// work, and is gone at the next boot.
    #[test]
    fn no_path_explain_calls_unwritable_is_editable() {
        let mut fake = FakeEditor::new();
        for def in crate::generated::GENERATED_PATHS {
            if def.writable {
                continue;
            }
            let refused = refusal(
                &mut fake,
                &[SUBJECT, def.path, APPEND_FLAG, "x\n", CONFIRMATION],
            );
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{}", def.path);
        }
    }

    /// The rest of `/etc` is refused too — it is the runtime projection, not a
    /// place to keep a file — and the refusal points at `explain` rather than
    /// pretending to know what the path was for.
    #[test]
    fn a_path_in_etc_that_is_not_generated_is_still_refused() {
        let mut fake = FakeEditor::new();
        // `/etc/motd` was generated between 2026-08-26 and 2026-09-13
        // (`ADR-035` §9B.1's `[greeting]`, removed by `.rulings/` R-168.1) —
        // it is back in this list now that nothing renders it.
        for path in [
            "/etc/motd",
            "/etc/nsswitch.conf",
            "/etc/fstab",
            "/etc/nested/thing.conf",
        ] {
            let refused = refusal(&mut fake, &[SUBJECT, path, APPEND_FLAG, "x\n", CONFIRMATION]);
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{path}");
            assert!(
                refused.message().contains("runtime projection"),
                "{path}: {}",
                refused.message()
            );
        }
    }

    /// `/etc` itself is not `/etc/...`, and a path that merely starts with the
    /// same letters is not inside it.
    #[test]
    fn a_path_that_only_looks_like_etc_is_not_refused_as_etc() {
        let mut fake = FakeEditor::holding("/etcetera/x", "a\n");
        edit(
            &mut fake,
            &[SUBJECT, "/etcetera/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
        )
        .expect("/etcetera is not /etc");
    }

    /// **Every reserved path is refused, and every refusal says it is not a
    /// security boundary.** `ADR-006` §8.9: it is the same class of refusal as a
    /// view, and implying an authority it does not have would be worse than not
    /// refusing at all.
    #[test]
    fn every_reserved_path_is_refused_and_none_of_them_claims_to_be_a_boundary() {
        let mut fake = FakeEditor::new();
        for reserved in reserved_paths() {
            let path = if reserved.tree {
                format!("{}/anything", reserved.path)
            } else {
                reserved.path.to_string()
            };
            let refused = refusal(
                &mut fake,
                &[SUBJECT, &path, APPEND_FLAG, "x\n", CONFIRMATION],
            );
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{path}");
            assert!(
                refused.message().contains("USE THE MECHANISM"),
                "{path}: {}",
                refused.message()
            );
            assert!(
                refused.message().contains("NOT A SECURITY BOUNDARY"),
                "{path}: {}",
                refused.message()
            );
            // **`reserved_for`, not the table row**, since 2026-08-29. The
            // application tree's row covers three paths that name three
            // different mechanisms (`ADR-006` §8.10.4.4), so the entry a path
            // actually falls under is the authority for what the refusal says —
            // which is what `reserved_for` answers.
            let entry = reserved_for(&path).expect("a reserved path falls under an entry");
            assert!(
                refused.message().contains(entry.instead),
                "{path}: the refusal does not say what to use instead: {}",
                refused.message()
            );
        }
        assert!(fake.wrote.is_empty());
    }

    /// **The plan shows the change, not a byte count** — `ADR-006` §8.10.7,
    /// ruled 2026-08-29. `ADR-012` §7.1 makes the plan the safety mechanism, and
    /// a safety mechanism that says *"3 bytes will change"* is a weak one.
    #[test]
    fn a_plan_shows_the_change_as_a_diff_and_not_only_as_a_byte_count() {
        let mut fake = FakeEditor::holding(NU, "alpha\nbanner = true\ngamma\n");
        let planned = refusal(
            &mut fake,
            &[
                SUBJECT,
                NU,
                EXPECT_FLAG,
                "banner = true",
                REPLACE_FLAG,
                "banner = false",
            ],
        );
        assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
        let plan = planned.plan().expect("a confirmation refusal carries a plan");
        let narrative = plan.narrative();
        // **The property `ADR-006` §8.10.7 was ruled for**: an operator whose
        // `--expect` matched the wrong stanza can see it here and cannot see it
        // in a byte count.
        assert!(narrative.contains("-banner = true"), "{narrative}");
        assert!(narrative.contains("+banner = false"), "{narrative}");
        assert!(narrative.contains(" alpha"), "{narrative}");
        assert!(fake.wrote.is_empty());
    }

    /// **`ADR-012` §7.3 reaches this operation too**, and for one reason:
    /// §8.10.4.4 made an application's program editable, and that is the path
    /// where a change stops the machine working *later*.
    ///
    /// Keeping `edit` on the old rule while its six siblings took the new one
    /// would have produced a surface where a program can be removed only with an
    /// acknowledgement and rewritten without one, which nobody could predict.
    #[test]
    fn editing_a_program_needs_the_acknowledgement_and_editing_its_neighbour_does_not() {
        const PROGRAM: &str = "/state/app/thing/0.2.0/bin/thing";
        const CONFIG: &str = "/state/app/thing/0.2.0/config.toml";
        const PHRASE: &str = "the application will not start until it is reinstalled";

        let mut fake = FakeEditor::holding(PROGRAM, "old\n");
        // The plan warns in the maintainer's own rendering and hands back a
        // command that already carries the acknowledgement.
        let planned = refusal(&mut fake, &[SUBJECT, PROGRAM, APPEND_FLAG, "x\n"]);
        let plan = planned.plan().expect("a plan");
        assert!(plan.narrative().contains("<WARNING>"), "{}", plan.narrative());
        assert!(plan.narrative().contains("(N/yes)"), "{}", plan.narrative());
        assert!(
            plan.confirm_with().contains(&format!("{ACCEPT_FLAG} '{PHRASE}'")),
            "{}",
            plan.confirm_with()
        );

        // `confirm` alone is refused, and nothing is written.
        let bare = refusal(&mut fake, &[SUBJECT, PROGRAM, APPEND_FLAG, "x\n", CONFIRMATION]);
        assert_eq!(bare.code(), ErrorCode::Unsafe);
        assert!(bare.message().contains("<WARNING>"), "{}", bare.message());
        assert!(fake.wrote.is_empty());

        // The acknowledgement, which cannot be produced without having read.
        edit(
            &mut fake,
            &[
                SUBJECT,
                PROGRAM,
                APPEND_FLAG,
                "x\n",
                ACCEPT_FLAG,
                PHRASE,
                CONFIRMATION,
            ],
        )
        .expect("the acknowledgement must be accepted");
        assert_eq!(fake.contents(PROGRAM), "old\nx\n");

        // And the neighbouring config file is the ordinary `confirm`, with the
        // consequence in the plan's PROSE — `ADR-012` §7.3.3's doctrine.
        let mut beside = FakeEditor::holding(CONFIG, "a = 1\n");
        let ordinary = refusal(&mut beside, &[SUBJECT, CONFIG, APPEND_FLAG, "b = 2\n"]);
        let prose = ordinary.plan().expect("a plan").narrative();
        assert!(!prose.contains("<WARNING>"), "the tier fired on a routine path: {prose}");
        assert!(prose.contains("NEXT UPDATE DISCARDS IT"), "{prose}");
        edit(
            &mut beside,
            &[SUBJECT, CONFIG, APPEND_FLAG, "b = 2\n", CONFIRMATION],
        )
        .expect("a config file beside a program is routine");
    }

    /// A superfluous acknowledgement is refused here too, which is what stops a
    /// caller appending it to every command and collapsing the two tiers.
    #[test]
    fn an_acknowledgement_on_an_ordinary_edit_is_refused_by_name() {
        let mut fake = FakeEditor::holding(NU, "x\n");
        let refused = refusal(
            &mut fake,
            &[SUBJECT, NU, APPEND_FLAG, "y\n", ACCEPT_FLAG, "anything", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(refused.message().contains("NO WARNING"), "{}", refused.message());
        assert!(fake.wrote.is_empty());
    }

    /// **The application tree is three zones and not one** — amended by the
    /// maintainer, 2026-08-29, `ADR-006` §8.10.4.4.
    ///
    /// Three paths stay reserved and each names its own mechanism; everything
    /// deeper is **permitted**, because a blanket refusal would recreate the gap
    /// §8.10 exists to close — the control plane is the only channel, and what was
    /// done by hand has to be doable by the surface.
    #[test]
    fn the_three_mechanism_paths_of_the_application_tree_are_refused_and_the_rest_is_not() {
        let mut fake = FakeEditor::new();
        for path in [
            APP_ROOT,
            "/state/app/thing",
            "/state/app/thing/current",
            "/state/app/thing/current/bin/thing",
        ] {
            let refused = refusal(
                &mut fake,
                &[SUBJECT, path, APPEND_FLAG, "x\n", CONFIRMATION],
            );
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{path}");
            assert!(
                refused.message().contains("USE THE MECHANISM"),
                "{path}: {}",
                refused.message()
            );
        }

        // And these are files an operator legitimately reaches after phase 5.
        for path in [
            "/state/app/thing/0.2.0/bin/thing",
            "/state/app/thing/0.2.0/manifest.toml",
            "/state/app/thing/0.2.0/config.toml",
            "/state/app/thing/data/notes",
        ] {
            assert!(
                reserved_for(path).is_none(),
                "{path} is still refused as a mechanism's"
            );
            assert!(check_not_reserved(path).is_ok(), "{path}");
        }
        assert!(fake.wrote.is_empty());
    }

    /// **The install request is the one the ruling named first**, because
    /// writing it by hand forges a principal.
    #[test]
    fn the_install_request_is_reserved_and_names_apply_install() {
        let mut fake = FakeEditor::new();
        let refused = refusal(
            &mut fake,
            &[SUBJECT, deploy::REQUEST_PATH, APPEND_FLAG, "x\n", CONFIRMATION],
        );
        assert!(
            refused.message().contains("rivet apply install"),
            "{}",
            refused.message()
        );
    }

    /// A reserved *tree* reserves what is under it and not a sibling whose name
    /// merely starts the same way.
    #[test]
    fn a_reserved_tree_does_not_reserve_a_sibling_with_a_longer_name() {
        let mut fake = FakeEditor::holding("/state/logging.conf", "a\n");
        edit(
            &mut fake,
            &[
                SUBJECT,
                "/state/logging.conf",
                EXPECT_FLAG,
                "a",
                REPLACE_FLAG,
                "b",
                CONFIRMATION,
            ],
        )
        .expect("/state/logging.conf is not inside /state/log");
    }

    /// A relative path is refused, for `super::install`'s reason: it would
    /// resolve somewhere neither the operator nor this process chose.
    #[test]
    fn a_relative_path_is_refused_before_anything_is_read() {
        let mut fake = FakeEditor::new();
        for path in ["config.nu", "./config.nu", "../x"] {
            let refused = refusal(&mut fake, &[SUBJECT, path, APPEND_FLAG, "x", CONFIRMATION]);
            assert_eq!(refused.code(), ErrorCode::Arguments, "{path}");
            assert!(refused.message().contains("ABSOLUTE"), "{path}");
        }
    }

    /// **A path with a newline in it is refused**, because a plan and a journal
    /// record are both lines — and the refusal carries the bytes back verbatim,
    /// which `AGENTS.md` §5 asks for.
    #[test]
    fn a_path_with_a_control_character_is_refused_and_carried_back_verbatim() {
        let mut fake = FakeEditor::new();
        for path in ["/state/two\nlines", "/state/tab\there", "/state/nul\0byte"] {
            let refused = refusal(&mut fake, &[SUBJECT, path, APPEND_FLAG, "x", CONFIRMATION]);
            assert_eq!(refused.code(), ErrorCode::Arguments, "{path:?}");
            assert_eq!(refused.subject(), Some(path));
            assert!(
                refused.message().contains("control character"),
                "{path:?}: {}",
                refused.message()
            );
        }
    }

    /// **B03: a `.`, `..` or repeated-separator spelling of a path is refused
    /// before anything is read, on both the plan and the confirm path.**
    ///
    /// The trigger this closes: `crate::edit`'s adapter decides whether new
    /// content must parse as the machine document by comparing the path
    /// STRING to `rivet_core::state::MACHINE_DOCUMENT` exactly, so
    /// `/state/config/./machine.toml` used to reach it having skipped that
    /// comparison. This test does not need the adapter or a TOML fixture to
    /// prove the fix — every one of these spellings must never even reach a
    /// board, for any file.
    #[test]
    fn a_path_with_a_traversal_component_is_refused_before_anything_is_read() {
        let mut fake = FakeEditor::new();
        for path in [
            "/state/config/./machine.toml",
            "/state/config//machine.toml",
            "/state/config/../config/machine.toml",
        ] {
            for confirmed in [&[][..], &[CONFIRMATION][..]] {
                let mut words = vec![SUBJECT, path, APPEND_FLAG, "x"];
                words.extend_from_slice(confirmed);
                let refused = refusal(&mut fake, &words);
                assert_eq!(refused.code(), ErrorCode::Arguments, "{path}");
                assert_eq!(refused.subject(), Some(path));
                assert!(
                    refused.message().contains("resolves nothing"),
                    "{path}: {}",
                    refused.message()
                );
            }
        }
        assert!(fake.asked.is_empty(), "a traversal alias reached the interlock: {:?}", fake.asked);
        assert!(fake.wrote.is_empty(), "a traversal alias reached a write: {:?}", fake.wrote);
    }

    // -- what it refuses about the file ---------------------------------------

    /// A file that is not text is refused, and the refusal says where the first
    /// unreadable byte is so a caller can tell a binary from a mis-encoded
    /// config.
    #[test]
    fn a_file_that_is_not_utf8_is_refused_and_says_where() {
        let mut fake = FakeEditor::new();
        fake.files
            .insert("/state/x".to_string(), vec![b'a', b'b', 0xff, 0xfe]);
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "ab", REPLACE_FLAG, "cd", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(refused.message().contains("not UTF-8"), "{}", refused.message());
        assert!(refused.message().contains("offset 2"), "{}", refused.message());
        assert!(fake.wrote.is_empty());
    }

    /// A file larger than this operation will read is refused rather than read.
    #[test]
    fn a_file_larger_than_the_cap_is_refused_rather_than_read_whole() {
        let mut fake = FakeEditor::new();
        fake.files
            .insert("/state/big".to_string(), vec![b'a'; FILE_MAX_BYTES + 1]);
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/big", EXPECT_FLAG, "aa", REPLACE_FLAG, "b", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(refused.message().contains("whole"), "{}", refused.message());
    }

    /// A read that fails is a refusal carrying what the machine said, and never
    /// an empty file — reading the second as the first would replace a file
    /// whose contents were never seen.
    #[test]
    fn a_file_that_cannot_be_read_is_refused_rather_than_treated_as_absent() {
        let mut fake = FakeEditor::new();
        fake.read_fails = Some(MachineError::Refused("Input/output error".to_string()));
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", APPEND_FLAG, "x", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::MachineRefused);
        assert!(refused.message().contains("Input/output error"));
        assert!(fake.wrote.is_empty());
    }

    /// The interlock's refusal reaches the operator with its own code and its
    /// own words — `E_UNSAFE` says *stop*, `E_MACHINE_REFUSED` says *look at the
    /// hardware*, and nothing is reworded on the way through.
    #[test]
    fn an_interlock_refusal_keeps_its_code_and_its_words_on_both_paths() {
        for (why, code) in [
            (
                MachineError::Unsafe("this is a windows build".to_string()),
                ErrorCode::Unsafe,
            ),
            (
                MachineError::Refused("Read-only file system".to_string()),
                ErrorCode::MachineRefused,
            ),
        ] {
            let mut fake = FakeEditor::holding("/state/x", "a\n");
            fake.interlock = Some(why.clone());
            fake.write_fails = Some(why.clone());
            let planned = refusal(
                &mut fake,
                &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b"],
            );
            let confirmed = refusal(
                &mut fake,
                &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
            );
            assert_eq!(planned.code(), code);
            assert_eq!(confirmed.code(), code);
            assert_eq!(planned.message(), why.why());
            assert_eq!(confirmed.message(), why.why());
        }
    }

    // -- the record ------------------------------------------------------------

    /// **A journal that could not be written is reported and is not a refusal.**
    /// The file is replaced either way, and saying "nothing was written" about a
    /// file that was would be the worse lie.
    #[test]
    fn an_edit_that_could_not_be_journalled_still_happened_and_says_so() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        fake.unrecorded = Some("/state/log is not there".to_string());
        let done = edit(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
        )
        .expect("the write succeeded");
        assert!(!done.journalled());
        assert_eq!(fake.contents("/state/x"), "b\n");

        let Value::Object(fields) = done.value() else {
            panic!("a result is an object");
        };
        assert!(fields.iter().any(|(k, v)| *k == "journalled" && *v == Value::Bool(false)));
        let (_, unrecorded) = fields
            .iter()
            .find(|(k, _)| *k == "unrecorded")
            .expect("an unjournalled edit says why");
        assert_eq!(*unrecorded, Value::str("/state/log is not there"));
    }

    /// A journalled edit omits `unrecorded` rather than carrying an empty one —
    /// `crate::value` has no null, so absence is how a value says it has no such
    /// thing.
    #[test]
    fn a_journalled_edit_omits_the_reason_field_entirely() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let done = edit(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
        )
        .expect("the write succeeded");
        assert!(done.journalled());
        let Value::Object(fields) = done.value() else {
            panic!("a result is an object");
        };
        assert!(!fields.iter().any(|(k, _)| *k == "unrecorded"), "{fields:?}");
    }

    /// The result names who asked, and says in the same breath that it is
    /// attribution rather than authority — `ADR-028` §4.5.
    #[test]
    fn the_result_says_who_asked_and_that_nothing_checked_it() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let done = edit(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
        )
        .expect("an edit");
        let Value::Object(fields) = done.value() else {
            panic!("a result is an object");
        };
        let (_, who) = fields
            .iter()
            .find(|(k, _)| *k == "principal")
            .expect("a result says who asked");
        let Value::Str(text) = who else {
            panic!("the principal is a sentence")
        };
        assert!(text.contains(&"0123456789abcdef".repeat(4)), "{text}");
        assert!(text.contains("not a proof") || text.contains("NOT"), "{text}");
    }

    // -- the arguments ---------------------------------------------------------

    /// Every way of naming no change, and each is refused with the two forms.
    #[test]
    fn a_command_that_names_no_change_or_half_a_change_is_refused_with_the_forms() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        for words in [
            vec![SUBJECT, "/state/x"],
            vec![SUBJECT, "/state/x", CONFIRMATION],
            vec![SUBJECT, "/state/x", EXPECT_FLAG, "a"],
            vec![SUBJECT, "/state/x", REPLACE_FLAG, "b"],
        ] {
            let refused = refusal(&mut fake, &words);
            assert_eq!(refused.code(), ErrorCode::Arguments, "{words:?}");
            assert!(fake.wrote.is_empty());
        }
        // `--replace` alone is the one worth its own assertion: it is a blind
        // clobber, which is the thing this operation exists to refuse.
        let refused = refusal(&mut fake, &[SUBJECT, "/state/x", REPLACE_FLAG, "b"]);
        assert!(
            refused.message().contains("blind clobber"),
            "{}",
            refused.message()
        );
    }

    /// The two changes are two changes, and asking for both is refused.
    #[test]
    fn append_and_replace_together_are_refused_rather_than_one_winning() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let refused = refusal(
            &mut fake,
            &[
                SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", APPEND_FLAG, "c",
                CONFIRMATION,
            ],
        );
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(refused.message().contains("two different changes"), "{}", refused.message());
    }

    /// A flag given twice is refused rather than the second silently winning.
    #[test]
    fn a_flag_given_twice_is_refused() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", APPEND_FLAG, "a", APPEND_FLAG, "b", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(refused.message().contains("twice"), "{}", refused.message());
    }

    /// **A flag with no value is refused rather than eating the next word.**
    /// `--append confirm` would otherwise append the word `confirm` and report
    /// success, which is `super::install`'s payload-called-confirm defect in a
    /// new place.
    #[test]
    fn a_flag_followed_by_one_of_this_commands_own_words_is_a_missing_value() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        for words in [
            vec![SUBJECT, "/state/x", APPEND_FLAG, CONFIRMATION],
            vec![SUBJECT, "/state/x", EXPECT_FLAG, REPLACE_FLAG, "b"],
            vec![SUBJECT, "/state/x", APPEND_FLAG],
        ] {
            let refused = refusal(&mut fake, &words);
            assert_eq!(refused.code(), ErrorCode::Arguments, "{words:?}");
            assert!(fake.wrote.is_empty(), "{words:?}");
        }
    }

    /// **Text that begins with `--` is still text**, because configuration files
    /// contain such lines and refusing them would make whole files uneditable.
    #[test]
    fn text_that_looks_like_a_flag_but_is_not_one_of_this_commands_is_accepted() {
        let mut fake = FakeEditor::holding("/state/x", "--verbose\n");
        edit(
            &mut fake,
            &[
                SUBJECT, "/state/x", EXPECT_FLAG, "--verbose", REPLACE_FLAG, "--quiet",
                CONFIRMATION,
            ],
        )
        .expect("a config line that begins with dashes is text");
        assert_eq!(fake.contents("/state/x"), "--quiet\n");
    }

    /// One spelling, so a command can be predicted rather than memorised —
    /// `Verb::parse`'s rule (`ADR-006` §6.4).
    #[test]
    fn the_equals_spelling_of_a_flag_is_refused_and_names_the_one_that_works() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let refused = refusal(&mut fake, &[SUBJECT, "/state/x", "--append=b", CONFIRMATION]);
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(refused.message().contains("with a space"), "{}", refused.message());
    }

    /// An unknown option is refused by name, and so is a stray bare word. This
    /// is the check that replaces `super::perform`'s arity ceiling — see
    /// `super::Input::several`.
    #[test]
    fn an_unknown_word_is_refused_by_name_rather_than_ignored() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        for words in [
            vec![SUBJECT, "/state/x", "--all", APPEND_FLAG, "b", CONFIRMATION],
            vec![SUBJECT, "/state/x", APPEND_FLAG, "b", "please", CONFIRMATION],
            vec![SUBJECT, "/state/x", "wat", APPEND_FLAG, "b"],
        ] {
            let refused = refusal(&mut fake, &words);
            assert_eq!(refused.code(), ErrorCode::Arguments, "{words:?}");
            assert!(fake.wrote.is_empty(), "{words:?}");
        }
    }

    /// Text longer than this build accepts is refused with the number, so a
    /// caller learns the limit rather than meeting `E2BIG` from `execve(2)`.
    #[test]
    fn text_larger_than_the_cap_is_refused_with_the_cap_in_the_message() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let enormous = "x".repeat(TEXT_MAX_BYTES + 1);
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", APPEND_FLAG, &enormous, CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(
            refused.message().contains(&TEXT_MAX_BYTES.to_string()),
            "{}",
            refused.message()
        );
        // ...and exactly the cap is accepted, so the boundary is where it says.
        let allowed = "x".repeat(TEXT_MAX_BYTES);
        edit(
            &mut fake,
            &[SUBJECT, "/state/x", APPEND_FLAG, &allowed, CONFIRMATION],
        )
        .expect("the cap itself is allowed");
    }

    /// An empty append is refused: nothing would be added, and creating an empty
    /// file is an operation this build does not have.
    #[test]
    fn an_empty_append_is_refused_rather_than_writing_the_file_unchanged() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let refused = refusal(&mut fake, &[SUBJECT, "/state/x", APPEND_FLAG, "", CONFIRMATION]);
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(fake.wrote.is_empty());
    }

    /// An empty `--expect` is refused **by name**, because "it matched 4193
    /// times" would send an agent to look at the file instead of at its command.
    #[test]
    fn an_empty_expect_is_refused_by_name_and_not_by_the_match_count() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "", REPLACE_FLAG, "b", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(refused.message().contains("everywhere"), "{}", refused.message());
    }

    /// A replacement identical to the expectation is refused: the write would
    /// still replace the file, still cost the card a block, and still journal an
    /// edit that changed nothing.
    #[test]
    fn a_change_that_would_change_nothing_is_refused() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let refused = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "a", CONFIRMATION],
        );
        assert_eq!(refused.code(), ErrorCode::Arguments);
        assert!(fake.wrote.is_empty());
    }

    /// Deleting text is `--replace ""`, and it is a real change rather than a
    /// no-op — so it is allowed where an identical replacement is not.
    #[test]
    fn deleting_text_with_an_empty_replacement_is_allowed() {
        let mut fake = FakeEditor::holding("/state/x", "keep\ndrop\nkeep\n");
        edit(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "drop\n", REPLACE_FLAG, "", CONFIRMATION],
        )
        .expect("a deletion is a change");
        assert_eq!(fake.contents("/state/x"), "keep\nkeep\n");
    }

    // -- time of check to time of use -----------------------------------------

    /// **The primitive is also the answer to a stale plan.** A file that changed
    /// between the plan and the confirmation no longer matches exactly once, and
    /// the confirmation is refused rather than applied to a file the plan did
    /// not describe.
    #[test]
    fn a_file_that_changed_since_the_plan_is_refused_at_the_confirmation() {
        let mut fake = FakeEditor::holding("/state/x", "banner = true\n");
        let words = [
            SUBJECT, "/state/x", EXPECT_FLAG, "banner = true", REPLACE_FLAG, "banner = false",
        ];
        let planned = refusal(&mut fake, &words);
        assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);

        // Somebody else edits the file in the meantime.
        fake.files
            .insert("/state/x".to_string(), b"banner = maybe\n".to_vec());

        let mut confirmed: Vec<&str> = words.to_vec();
        confirmed.push(CONFIRMATION);
        let refused = refusal(&mut fake, &confirmed);
        assert_eq!(refused.code(), ErrorCode::Unsafe);
        assert!(refused.message().contains("matches NOTHING"), "{}", refused.message());
        assert_eq!(fake.contents("/state/x"), "banner = maybe\n");
    }

    /// And the plan says that this is what will happen, so an operator is not
    /// surprised by it.
    #[test]
    fn the_plan_says_the_file_is_read_again_at_the_confirmation() {
        let mut fake = FakeEditor::holding("/state/x", "a\n");
        let planned = refusal(
            &mut fake,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b"],
        );
        assert!(
            planned.message().contains("READ AGAIN"),
            "{}",
            planned.message()
        );
    }

    // -- the catalogue's half --------------------------------------------------

    /// Every declared output is one some result actually carries, and the two
    /// changes between them carry all of them. A catalogue naming a field
    /// nothing produces is `ADR-006` §7's "trusted and wrong".
    #[test]
    fn between_them_the_two_changes_produce_every_declared_output() {
        let mut created = FakeEditor::new();
        created.missing_parents = vec!["/state/deep".to_string()];
        created.unrecorded = Some("no journal here".to_string());
        let creation = created
            .files
            .is_empty()
            .then(|| {
                edit(
                    &mut created,
                    &[SUBJECT, "/state/deep/x", APPEND_FLAG, "a\n", CONFIRMATION],
                )
                .expect("a creation")
            })
            .expect("a creation");

        let mut existing = FakeEditor::holding("/state/x", "a\n");
        let replacement = edit(
            &mut existing,
            &[SUBJECT, "/state/x", EXPECT_FLAG, "a", REPLACE_FLAG, "b", CONFIRMATION],
        )
        .expect("a replacement");

        let mut seen: Vec<&str> = Vec::new();
        for value in [creation.value(), replacement.value()] {
            let Value::Object(fields) = value else {
                panic!("a result is an object")
            };
            for (key, _) in fields {
                if !seen.contains(&key) {
                    seen.push(key);
                }
            }
        }
        for declared in OUTPUTS {
            assert!(
                seen.contains(&declared.name),
                "{} is declared and no result carries it",
                declared.name
            );
        }
        for key in &seen {
            assert!(
                OUTPUTS.iter().any(|f| f.name == *key),
                "{key} is produced and not declared"
            );
        }
    }

    /// The reserved table names a mechanism and an alternative for every entry,
    /// and no entry is listed twice.
    #[test]
    fn every_reserved_entry_is_complete_and_distinct() {
        let mut seen: Vec<&str> = Vec::new();
        for reserved in reserved_paths() {
            assert!(reserved.path.starts_with('/'), "{}", reserved.path);
            assert!(!reserved.mechanism.is_empty(), "{}", reserved.path);
            assert!(!reserved.instead.is_empty(), "{}", reserved.path);
            assert!(!seen.contains(&reserved.path), "{} twice", reserved.path);
            seen.push(reserved.path);
        }
    }

    /// **The re-type line, on its own**, because it is the one part of a plan an
    /// operator acts on by copying — and it was wrong in the first version.
    #[test]
    fn the_command_to_repeat_quotes_every_word_that_needs_it_and_nothing_else() {
        assert_eq!(
            retype(&["edit", "/state/x", EXPECT_FLAG, "a b", REPLACE_FLAG, "c"]),
            "edit /state/x --expect 'a b' --replace c"
        );
        // An empty word is quoted too: `--replace ''` is how a deletion is
        // spelled, and an unquoted empty word would vanish from the line.
        assert_eq!(retype(&[REPLACE_FLAG, ""]), "--replace ''");
        // And a word this cannot quote is said rather than mangled.
        let awkward = retype(&[APPEND_FLAG, "it's here"]);
        assert!(awkward.contains("cannot be quoted"), "{awkward}");
        assert!(!awkward.contains("'it's here'"), "{awkward}");
    }

    /// **Every character that could mean something to the shell reading the
    /// command back is quoted; nothing else is.** The two bare words here are
    /// exactly the shapes `retype`'s allowlist keeps unquoted; every other
    /// word has at least one character outside it and must come back
    /// single-quoted to survive being typed back rather than expanded, split,
    /// or read as syntax.
    #[test]
    fn retype_quotes_every_word_a_shell_could_read_as_something_else() {
        assert_eq!(retype(&["$HOME"]), "'$HOME'");
        assert_eq!(retype(&["a*b"]), "'a*b'");
        assert_eq!(retype(&["x;y"]), "'x;y'");
        assert_eq!(retype(&["a|b"]), "'a|b'");
        assert_eq!(retype(&["one\ntwo"]), "'one\ntwo'");
        assert_eq!(retype(&["\""]), "'\"'");
        assert_eq!(retype(&["\\"]), "'\\'");
        assert_eq!(retype(&[""]), "''");
        assert_eq!(retype(&["/state/app/x.txt"]), "/state/app/x.txt");
        assert_eq!(retype(&["--append"]), "--append");
    }

    /// **The same property, through the planner rather than `retype` in
    /// isolation**: a literal argument a shell would expand is not the
    /// literal argument the confirmation prints back.
    #[test]
    fn a_dollar_prefixed_literal_is_quoted_in_the_command_to_repeat() {
        let mut fake = FakeEditor::holding("/state/x", "old\n");
        let planned = refusal(&mut fake, &[SUBJECT, "/state/x", APPEND_FLAG, "$HOME"]);
        assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
        let plan = planned.plan().expect("a confirmation refusal carries its plan");
        assert!(
            plan.confirm_with()
                .contains(&format!("{APPEND_FLAG} '$HOME' {CONFIRMATION}")),
            "{}",
            plan.confirm_with()
        );
        assert!(fake.wrote.is_empty());
    }

    /// A created file's mode is `0600` and it is reported in the octal every
    /// Unix tool prints — `explain`'s rule, for its reason: `384` would be read
    /// as decimal by the one reader that matters, a person at 2am.
    #[test]
    fn the_created_mode_is_reported_in_octal() {
        assert_eq!(format!("{NEW_FILE_MODE:04o}"), "0600");
        assert_eq!(format!("{NEW_DIRECTORY_MODE:04o}"), "0700");
        assert_eq!(NEW_FILE_MODE & 0o077, 0, "a new file is not group-readable");
    }
}
