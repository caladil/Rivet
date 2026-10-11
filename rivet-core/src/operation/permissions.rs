//! `rivet apply set-mode <path> <octal> confirm` and
//! `rivet apply set-owner <path> <user>[:<group>] confirm` — **the two subjects
//! that change who may reach a path.**
//!
//! `ADR-006` §8.10.9 and §8.10.10, ruled by the maintainer, Joel Lindell,
//! 2026-08-29. `apply edit` deliberately does not change permissions — *"a file
//! that already exists keeps the mode it has"* — and `apply create-file` makes
//! things private. These are how either is widened, and widening is a separate
//! act because it has a separate consequence.
//!
//! # No symbolic mode, and the refusal is by name
//!
//! `u+x`, `go-rwx` and `a=r` are **patterns over the mode that is currently
//! there**. This family's rule is `super::edit`'s: a caller states what it
//! believes and is right. `set-mode 0644` says what the mode will be; `u+x` says
//! what to do to a mode nobody has looked at, and its result depends on a value
//! the caller did not state. So it is refused, by name, with the octal form it
//! would have to be written as.
//!
//! That is exactly `ADR-006` §8.9.3's argument against regular expressions in
//! `apply edit`: *"a literal is what an agent can be correct about; a pattern is
//! what it can be nearly correct about, and the two failure modes are not
//! comparable."*
//!
//! # setuid, setgid and the sticky bit are refused, and it is a decision
//!
//! **Rivet ships no setuid binaries.** A setuid bit set through the
//! administrative surface is a privilege-escalation primitive nobody asked this
//! operation for, and the one place it could ever be wanted — a program in an
//! image — is a build-time decision rather than a runtime one on a board. If it
//! is ever needed it needs its own ruling and its own subject, which is this
//! whole section's shape (`AGENTS.md` §11.1).
//!
//! **The bits are still *reported* wherever a mode is** — `super::filesystem`'s
//! renderer appends `+setuid` — because refusing to set one is not a reason to
//! hide one that is there.
//!
//! # A no-op is refused
//!
//! `super::edit`'s rule for `--expect` equal to `--replace`, and the same
//! argument: a change that changes nothing still puts a record in the journal
//! saying something happened, and a caller who asked for it has misunderstood
//! the state they are in — which is worth telling them.
//!
//! # `set-owner` does not recurse, and the plan says so
//!
//! A directory's contents keep their owner. Recursive ownership is
//! `ADR-006` §8.10.13's problem in a different costume: an act whose size the
//! plan cannot show, and therefore one an operator cannot check before saying
//! yes.

use super::apply::{ACCEPT_FLAG, CONFIRMATION, Hazard};
use super::filesystem::{
    check_writable_path, confirmation_words, describe_id, describe_mode, describe_stat,
    hazard_for, read_tail, refused, unsafe_for,
};
use super::machine::{Board, OwnerIds, PathChange, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::value::Value;

/// The subject that changes a mode.
pub const MODE_SUBJECT: &str = "set-mode";
/// The subject that changes an owner or a group.
pub const OWNER_SUBJECT: &str = "set-owner";

/// The bits this build refuses to set. See this module's documentation.
pub const REFUSED_BITS: u32 = 0o7000;

/// What both subjects' results carry. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "Which change this was: `set-mode` or `set-owner`.",
    },
    Field {
        name: "path",
        description: "What was changed, exactly as it was given. A symbolic link is never \
                      followed (ADR-006 §8.10.4.3).",
    },
    Field {
        name: "type",
        description: "What is at the path: `file` or `directory`. Reported because the change \
                      applies to THAT and to nothing inside it.",
    },
    Field {
        name: "mode",
        description: "`set-mode` ONLY: the mode it has now, four octal digits.",
    },
    Field {
        name: "mode_was",
        description: "`set-mode` ONLY: the mode it had before. Both are reported because a \
                      record that said only the new one could not answer 'what did I change'.",
    },
    Field {
        name: "uid",
        description: "`set-owner` ONLY: the owning user's number now.",
    },
    Field {
        name: "user",
        description: "`set-owner` ONLY: the name for `uid`, where this machine has one. ABSENT \
                      is a fact about the identity table and not about the path.",
    },
    Field {
        name: "gid",
        description: "`set-owner` ONLY: the owning group's number now.",
    },
    Field {
        name: "group",
        description: "`set-owner` ONLY: the name for `gid`. See `user`.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked, in a sentence (ADR-028 §4.5). \
                      ATTRIBUTION AND NOT AUTHORITY: nothing checked it.",
    },
    Field {
        name: "journalled",
        description: "Whether a record reached the journal. FALSE DOES NOT MEAN NOTHING \
                      CHANGED.",
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

/// What was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wanted {
    Mode(u32),
    Owner { user: Option<String>, group: Option<String> },
}

/// Which of the two this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mode,
    Owner,
}

impl Kind {
    /// The subject word, if this is one of them.
    pub(super) fn parse(subject: &str) -> Option<Self> {
        match subject {
            MODE_SUBJECT => Some(Self::Mode),
            OWNER_SUBJECT => Some(Self::Owner),
            _ => None,
        }
    }

    const fn subject(self) -> &'static str {
        match self {
            Self::Mode => MODE_SUBJECT,
            Self::Owner => OWNER_SUBJECT,
        }
    }
}

/// One permission change that was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    path: String,
    kind: Kind,
    type_word: &'static str,
    mode: Option<u32>,
    mode_was: Option<u32>,
    owner: OwnerIds,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Changed {
    /// What was changed.
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
            ("type", Value::str(self.type_word)),
        ];
        if let Some(mode) = self.mode {
            fields.push(("mode", Value::str(format!("{mode:04o}"))));
        }
        if let Some(was) = self.mode_was {
            fields.push(("mode_was", Value::str(format!("{was:04o}"))));
        }
        if let Some(uid) = self.owner.uid {
            fields.push(("uid", Value::Integer(i64::from(uid))));
        }
        if let Some(user) = &self.owner.user {
            fields.push(("user", Value::str(user.clone())));
        }
        if let Some(gid) = self.owner.gid {
            fields.push(("gid", Value::Integer(i64::from(gid))));
        }
        if let Some(group) = &self.owner.group {
            fields.push(("group", Value::str(group.clone())));
        }
        fields.push(("principal", Value::str(describe_who(&self.principal, self.kind))));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push(("next", Value::str(next_after(self))));
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "a permission change's keys must be a subsequence of OUTPUTS, in order"
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
        "anything running as uid 0 could have changed it another way and left no record at all",
    )
}

/// What the operator should expect afterwards.
fn next_after(changed: &Changed) -> String {
    let record = if changed.journalled() {
        "IT IS IN THE JOURNAL, naming the session that asked (ADR-028 §4.5)."
    } else {
        "IT IS NOT IN THE JOURNAL and `unrecorded` says why. The change was made either way."
    };
    format!(
        "THE CHANGE IS LIVE FOR ANYTHING THAT OPENS THE PATH FROM NOW ON, and NOT for anything \
         that already has it open — a process holding an open file keeps the access it was \
         granted when it opened it, so a service does not lose or gain anything until it \
         reopens. NOTHING INSIDE A DIRECTORY IS CHANGED: this operation does not recurse. \
         {record} Nothing was stopped and nothing was reloaded."
    )
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// The form, for every refusal that has to show it.
fn forms(kind: Kind) -> String {
    match kind {
        Kind::Mode => format!(
            "`rivet {apply} {MODE_SUBJECT} <path> <octal> [{ACCEPT_FLAG} <text>] \
             [{CONFIRMATION}]` — the mode is THREE OR FOUR OCTAL DIGITS, such as 0644",
            apply = Verb::Apply,
        ),
        Kind::Owner => format!(
            "`rivet {apply} {OWNER_SUBJECT} <path> <user>[:<group>] [{ACCEPT_FLAG} <text>] \
             [{CONFIRMATION}]` — either half may be a NAME or a NUMBER, and `:group` alone \
             changes only the group",
            apply = Verb::Apply,
        ),
    }
}

/// Read a mode, or the refusal that says why this is not one.
fn read_mode(given: &str) -> Result<u32, OperationError> {
    let arguments = |message: String| OperationError::about(ErrorCode::Arguments, given, message);

    // **The symbolic forms, refused by name.** See this module's documentation.
    if given.contains(['+', '-', '=']) || given.bytes().any(|b| b.is_ascii_alphabetic()) {
        return Err(arguments(format!(
            "`{given}` looks like a SYMBOLIC mode and this build takes octal only. A symbolic \
             mode is a PATTERN OVER THE MODE THAT IS ALREADY THERE — its result depends on a \
             value you did not state, and this family's whole rule is that a caller states what \
             it believes and is right (ADR-006 §8.9.2, §8.10.9). Say the mode you want: \
             `rivet {get} {file} <path>` reports the one it has now, in octal and in letters.",
            get = Verb::Get,
            file = super::file::SUBJECT,
        )));
    }

    if given.is_empty() || given.len() > 4 || given.bytes().any(|b| !(b'0'..=b'7').contains(&b)) {
        return Err(arguments(format!(
            "`{given}` is not a mode. A mode is THREE OR FOUR OCTAL DIGITS — each 0 to 7 — such \
             as `0644` or `700`. Nothing else is accepted, and in particular a decimal number \
             like `644` read as decimal would be a different mode entirely, which is why this \
             refuses rather than interpreting."
        )));
    }

    let mode = u32::from_str_radix(given, 8).map_err(|_| {
        arguments(format!("`{given}` is not a mode this build can read as octal."))
    })?;

    if mode & REFUSED_BITS != 0 {
        let which: Vec<&str> = [
            (0o4000, "setuid"),
            (0o2000, "setgid"),
            (0o1000, "the sticky bit"),
        ]
        .into_iter()
        .filter(|(bit, _)| mode & bit != 0)
        .map(|(_, name)| name)
        .collect();
        return Err(OperationError::about(
            ErrorCode::Unsafe,
            given,
            format!(
                "REFUSED `{given}` sets {which}, and NOTHING HAS BEEN CHANGED. Rivet ships no \
                 setuid binaries: a program that runs as somebody other than whoever started it \
                 is a privilege-escalation primitive, and setting one through the \
                 administrative surface is not something anybody has asked this operation for. \
                 THE PLACE FOR IT, IF IT IS EVER NEEDED, IS THE IMAGE — a build-time decision \
                 rather than a runtime one on a board — and it would need its own ruling \
                 (ADR-006 §8.10.9). The bits are still REPORTED wherever a mode is shown, \
                 because refusing to set one is no reason to hide one that is there.",
                which = which.join(" and "),
            ),
        ));
    }

    Ok(mode)
}

/// Read a `<user>[:<group>]`, or the refusal that says why this is not one.
fn read_owner(given: &str) -> Result<(Option<String>, Option<String>), OperationError> {
    let arguments = |message: String| OperationError::about(ErrorCode::Arguments, given, message);

    let (user, group) = match given.split_once(':') {
        Some((user, group)) => (user, Some(group)),
        None => (given, None),
    };
    let user = (!user.is_empty()).then(|| user.to_string());
    let group = group.and_then(|g| (!g.is_empty()).then(|| g.to_string()));

    if user.is_none() && group.is_none() {
        return Err(arguments(format!(
            "`{given}` names neither a user nor a group, so there is no change to make. \
             {forms}.",
            forms = forms(Kind::Owner),
        )));
    }
    Ok((user, group))
}

// ---------------------------------------------------------------------------
// Performing it
// ---------------------------------------------------------------------------

/// Perform one of the two permission changes.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a malformed command, [`ErrorCode::Unsafe`] for a
/// path this must not change or a mode it will not set,
/// [`ErrorCode::ConfirmationRequired`] carrying the plan, and
/// [`ErrorCode::MachineRefused`] when the machine could not do it.
pub(super) fn perform(
    kind: Kind,
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Changed, OperationError> {
    let (Some(path), Some(given)) = (
        arguments_in.get(1).copied(),
        arguments_in.get(2).copied(),
    ) else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            kind.subject(),
            format!(
                "`{apply} {subject}` needs an ABSOLUTE path and then what to set it to. \
                 {forms}.",
                apply = Verb::Apply,
                subject = kind.subject(),
                forms = forms(kind),
            ),
        ));
    };

    let wanted = match kind {
        Kind::Mode => Wanted::Mode(read_mode(given)?),
        Kind::Owner => {
            let (user, group) = read_owner(given)?;
            Wanted::Owner { user, group }
        }
    };

    let tail = read_tail(kind.subject(), &forms(kind), arguments_in, 3)?;
    check_writable_path(&format!("{} {}", Verb::Apply, kind.subject()), path)?;
    let hazard = hazard_for(path);
    super::apply::accepted(tail.accept, hazard.as_ref(), tail.confirmed)?;

    resolve(
        Permissions {
            kind,
            path,
            wanted,
            typed: arguments_in,
            hazard,
        },
        tail.confirmed,
        board,
    )
}

/// **The change one of these subjects is**, as `ADR-006` §6.5 defines one.
struct Permissions<'a> {
    kind: Kind,
    path: &'a str,
    wanted: Wanted,
    typed: &'a [&'a str],
    hazard: Option<Hazard>,
}

/// What planning worked out, and what performing it must re-check.
struct Prepared {
    stat: PathStat,
    /// `set-owner` ONLY: the identities the plan showed, resolved.
    resolved: OwnerIds,
    principal: Principal,
}

impl Change for Permissions<'_> {
    type Prepared = Prepared;
    type Applied = Changed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path;
        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let kind = self.check_what_is_there(&stat)?;

        // **Names are resolved BEFORE the confirmation**, so an operator never
        // confirms an identifier they have not seen resolved (`ADR-006`
        // §8.10.10).
        let resolved = match &self.wanted {
            Wanted::Mode(_) => OwnerIds::default(),
            Wanted::Owner { user, group } => board
                .resolve_owner(user.as_deref(), group.as_deref())
                .map_err(|e| refused(e, path))?,
        };

        let change = self.change(&stat, &resolved);
        self.check_it_changes_something(&stat, &resolved)?;

        let principal = board.principal();
        board.can_change_path(&change).map_err(|e| refused(e, path))?;

        let zone = super::filesystem::app_zone_clause(path);
        let warning = self
            .hazard
            .as_ref()
            .map_or_else(String::new, |h| format!(" {}", h.warning()));

        let effect = match &self.wanted {
            Wanted::Mode(mode) => Effect::set(
                path,
                stat.mode.map_or_else(|| "an unreadable mode".to_string(), describe_mode),
                describe_mode(*mode),
            )
            .noting(format!(
                "it is {what}. {widening}{zone}{warning}",
                what = describe_stat(&stat),
                widening = widening_note(stat.mode, *mode, kind),
            )),
            Wanted::Owner { .. } => Effect::set(
                path,
                format!(
                    "{}:{}",
                    describe_id(stat.uid, stat.user.as_deref()),
                    describe_id(stat.gid, stat.group.as_deref()),
                ),
                format!(
                    "{}:{}",
                    describe_id(
                        resolved.uid.or(stat.uid),
                        resolved.user.as_deref().or(stat.user.as_deref())
                    ),
                    describe_id(
                        resolved.gid.or(stat.gid),
                        resolved.group.as_deref().or(stat.group.as_deref())
                    ),
                ),
            )
            .noting(format!(
                "it is {what}. THE NAMES ARE RESOLVED HERE, before you confirm, against \
                 ADR-007's projected identity tables — so the numbers above are the ones that \
                 would be written.{zone}{warning}",
                what = describe_stat(&stat),
            )),
        };

        let plan = Plan::new(path, "changes who may reach a path on this machine")
            .doing(effect)
            .leaving(
                "the contents are not read and not changed — this operation changes an \
                 attribute of the path and nothing about what is in it",
            )
            .leaving(
                "NOTHING INSIDE A DIRECTORY IS CHANGED: this does not recurse. A recursive form \
                 is an act whose size a plan cannot show, which is ADR-006 §8.10.13's problem \
                 in a different costume",
            )
            .leaving(
                "anything that ALREADY HAS THE PATH OPEN keeps the access it was granted when \
                 it opened it — a running service neither loses nor gains anything until it \
                 reopens",
            )
            .taking_effect(TakesEffect::Immediately)
            .leaving(
                "WHAT THIS PLAN DOES NOT GUARANTEE: it describes the path AS IT IS NOW, and this \
                 process ends when it answers you. Every refusal above is re-run at the moment \
                 this acts, but nothing carries a plan from one command to the next — so a \
                 `confirm` typed later applies to the path THEN",
            )
            .confirmed_by(&confirmation_words(self.typed, self.hazard.as_ref()));

        Ok((
            plan,
            Prepared {
                stat,
                resolved,
                principal,
            },
        ))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Changed, OperationError> {
        let path = self.path;
        let now = board.stat_path(path).map_err(|e| refused(e, path))?;
        let type_word = self.check_what_is_there(&now)?;
        if now.kind != prepared.stat.kind
            || now.mode != prepared.stat.mode
            || now.uid != prepared.stat.uid
            || now.gid != prepared.stat.gid
        {
            return Err(unsafe_for(
                path,
                format!(
                    "REFUSED {path} IS NOT WHAT THE PLAN DESCRIBED and NOTHING HAS BEEN \
                     CHANGED. The plan described {then}; it is now {now}. Something else on \
                     this machine changed it between the plan and the confirmation.",
                    then = describe_stat(&prepared.stat),
                    now = describe_stat(&now),
                ),
            ));
        }

        let change = self.change(&prepared.stat, &prepared.resolved);
        let wrote = board.change_path(&change).map_err(|e| refused(e, path))?;

        let (mode, mode_was) = match self.wanted {
            Wanted::Mode(mode) => (Some(mode), prepared.stat.mode),
            Wanted::Owner { .. } => (None, None),
        };
        let owner = match self.wanted {
            Wanted::Mode(_) => OwnerIds::default(),
            Wanted::Owner { .. } => OwnerIds {
                uid: prepared.resolved.uid.or(prepared.stat.uid),
                user: prepared
                    .resolved
                    .user
                    .clone()
                    .or_else(|| prepared.stat.user.clone()),
                gid: prepared.resolved.gid.or(prepared.stat.gid),
                group: prepared
                    .resolved
                    .group
                    .clone()
                    .or_else(|| prepared.stat.group.clone()),
            },
        };

        Ok(Changed {
            path: path.to_string(),
            kind: self.kind,
            type_word,
            mode,
            mode_was,
            owner,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Permissions<'_> {
    /// The port's description of this act.
    fn change<'b>(&'b self, stat: &PathStat, resolved: &OwnerIds) -> PathChange<'b> {
        match self.wanted {
            Wanted::Mode(mode) => PathChange::SetMode {
                path: self.path,
                mode,
                was: stat.mode.unwrap_or(0),
            },
            Wanted::Owner { .. } => PathChange::SetOwner {
                path: self.path,
                uid: resolved.uid,
                gid: resolved.gid,
                was_uid: stat.uid,
                was_gid: stat.gid,
            },
        }
    }

    /// What is at the path, or the refusal that says why this cannot apply to
    /// it.
    fn check_what_is_there(&self, stat: &PathStat) -> Result<&'static str, OperationError> {
        use super::machine::PathKind;
        let path = self.path;
        match stat.kind {
            None => Err(unsafe_for(
                path,
                format!(
                    "REFUSED there is nothing at {path}, so there is nothing to change and \
                     NOTHING HAS BEEN CHANGED. This operation changes an attribute of something \
                     that exists; `rivet {apply} create-file {path} {CONFIRMATION}` makes one, \
                     private, and this is how it is widened afterwards.",
                    apply = Verb::Apply,
                ),
            )),
            Some(PathKind::File) => Ok("file"),
            Some(PathKind::Directory) => Ok("directory"),
            Some(other) => Err(unsafe_for(
                path,
                format!(
                    "REFUSED {path} is a {other} and this build changes the permissions of \
                     files and directories only, so NOTHING HAS BEEN CHANGED. A symbolic link's \
                     own mode means nothing on Linux — what matters is the mode of what it \
                     points at, and this surface does not follow one (ADR-006 §8.10.4.3), so \
                     changing it here would be a change with no effect reported as a success."
                ),
            )),
        }
    }

    /// **A change that changes nothing is refused.** See this module's
    /// documentation.
    fn check_it_changes_something(
        &self,
        stat: &PathStat,
        resolved: &OwnerIds,
    ) -> Result<(), OperationError> {
        let path = self.path;
        match &self.wanted {
            Wanted::Mode(mode) if stat.mode == Some(*mode) => Err(unsafe_for(
                path,
                format!(
                    "REFUSED {path} is ALREADY {what} and NOTHING HAS BEEN CHANGED. Refused \
                     rather than performed, because a change that changes nothing still puts a \
                     record in the journal saying something happened — and a caller who asked \
                     for it has misunderstood the state they are in, which is worth being told.",
                    what = describe_mode(*mode),
                ),
            )),
            Wanted::Owner { .. } => {
                let uid_same = resolved.uid.is_none() || resolved.uid == stat.uid;
                let gid_same = resolved.gid.is_none() || resolved.gid == stat.gid;
                if uid_same && gid_same {
                    return Err(unsafe_for(
                        path,
                        format!(
                            "REFUSED {path} is ALREADY owned by {who} and NOTHING HAS BEEN \
                             CHANGED. `super::edit`'s rule: a change that changes nothing still \
                             puts a record in the journal saying something happened.",
                            who = format_args!(
                                "{}:{}",
                                describe_id(stat.uid, stat.user.as_deref()),
                                describe_id(stat.gid, stat.group.as_deref()),
                            ),
                        ),
                    ));
                }
                Ok(())
            }
            Wanted::Mode(_) => Ok(()),
        }
    }
}

/// The clause a mode change carries about what it opens up, if anything.
///
/// **Widening is the direction worth a sentence.** Narrowing a mode can stop a
/// service reading a file, which the plan's own effect already shows; widening
/// is the one that is invisible afterwards and is how a secret gets left where
/// something can read it (`ADR-020`).
fn widening_note(was: Option<u32>, becomes: u32, kind: &'static str) -> String {
    let opened = was.is_none_or(|was| becomes & !was & 0o077 != 0);
    if !opened {
        return "This does not open the path to anybody it was closed to.".to_string();
    }
    let who = match (becomes & 0o070 != 0, becomes & 0o007 != 0) {
        (_, true) => "EVERY uid on this machine",
        (true, false) => "every uid in the owning group",
        (false, false) => "nobody new",
    };
    format!(
        "THIS OPENS THE {kind} TO {who}, which it was closed to. Rivet creates things private \
         and widens deliberately (ADR-006 §8.9.3) — a secret put in a path that was widened \
         earlier is put there exactly once, which is once too often (ADR-020)."
    )
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
