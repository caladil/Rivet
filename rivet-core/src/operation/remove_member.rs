//! `rivet apply remove-member <id> [confirm]` — **retract one fleet member's
//! register record from this fleet server's register.** The exact inverse of
//! [`super::enroll_member`], `ADR-059` §6 — the control-plane tool for
//! taking a member OUT of a fleet, so an operator never hand-deletes a
//! `register/devices/<id>.toml` over a shell they were not meant to need.
//!
//! A fleet server (`rivet-fleetd`) keeps its register at
//! `/state/app/rivet-fleetd/data/register/`, and it only ever *reads* it —
//! `cmd/rivet-fleetd/src/fleet.rs` re-reads the whole directory on every
//! `GET /api/fleet`, discovering members by scanning `devices/<id>.toml`, with
//! no index in `register.toml`. So retracting a member is exactly the inverse
//! of enrolling one: take the one `devices/<id>.toml` away, and it is gone from
//! the next read.
//!
//! # Why a subject of its own, and not `remove-file`
//!
//! `remove-file` (`ADR-006` §8) removes any regular file under
//! `/state`, and could in principle reach this one — but it takes a PATH,
//! and this crate's whole file surface holds to one rule: `<id>` is the only
//! thing a caller supplies, and the path is composed, never given (see
//! [`super::enroll_member`]'s own documentation for the argument in full). A
//! caller that could name `register/devices/<anything>.toml` directly could
//! also mistype it, and `remove-file`'s refusals are generic where this
//! subject's are specific: `NO SUCH MEMBER` rather than *"nothing there"*,
//! and a plan that says `<id>` by name rather than a bare path.
//!
//! # The path is composed, never given
//!
//! `<id>` is a register identifier — sixteen lowercase hexadecimal characters
//! (`ADR-049` §8) — so it cannot contain a path separator, cannot be `.` or
//! `..`, and names exactly one file inside `register/devices/`. This is the
//! SAME rule [`super::enroll_member::is_identifier`] applies, reused rather
//! than redefined, so the two subjects can never disagree about what an id is.
//!
//! # What it does NOT do
//!
//! * **It does not create the register**, for the same reason `enroll-member`
//!   does not: a machine without `register/devices/` is not a fleet server, and
//!   this refuses rather than inventing one.
//! * **It restarts nothing.** `rivet-fleetd` re-reads the register on every
//!   request, so the member is gone from the next `GET /api/fleet`.
//! * **It checks nothing about WHY the member is being removed.** Whether the
//!   fleet still needs it, or whether something is still deployed to it, is the
//!   workstation's call — this subject carries out a decision already made, and
//!   does not re-litigate it.
//! * **A member that is not there is a refusal, not a success** — `super::remove`'s
//!   departure from `ADR-006` §6's idempotence, and the same argument:
//!   *"nothing was there"* and *"I removed it"* are different facts, and an
//!   operator who got the id wrong needs to know which one happened.

use super::apply::CONFIRMATION;
use super::enroll_member::{self, DEVICES_DIR, FLEET_SERVER_APP, ID_LEN, RECORD_SUFFIX, REGISTER_DIR};
use super::filesystem::{arguments, check_writable_path, directory_of, read_tail, refused, unsafe_for};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::service::{ServiceName, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "remove-member";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `remove-member`.",
    },
    Field {
        name: "id",
        description: "The member's register identifier, as it was given — the name of the \
                      record's file inside register/devices/.",
    },
    Field {
        name: "path",
        description: "The ABSOLUTE path that was removed. Composed from the id and never \
                      given, so no caller can reach a file outside this fleet server's \
                      register/devices/.",
    },
    Field {
        name: "before-bytes",
        description: "How many bytes the record held. THIS IS THE ONE FACT NOBODY CAN RECOVER, \
                      which is why it is a field and not only prose.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked (ADR-028 §4). ATTRIBUTION AND \
                      NOT AUTHORITY.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of this removal reached the journal. The record names \
                      the path and the length, never the content.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of it. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "That the member is gone from the register and rivet-fleetd will stop \
                      listing it on its next read — nothing was restarted, because it re-reads \
                      every request.",
    },
];

/// One member that was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    id: String,
    path: String,
    bytes: u64,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Removed {
    /// Whether a record of it reached the journal.
    #[must_use]
    pub const fn journalled(&self) -> bool {
        self.unrecorded.is_none()
    }

    /// The structured answer. `ADR-006` §4.
    #[must_use]
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("id", Value::str(self.id.clone())),
            ("path", Value::str(self.path.clone())),
            (
                "before-bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ),
        ];
        fields.push((
            "principal",
            Value::str(self.principal.describes(
                "the member record",
                &format!("`rivet {apply} {SUBJECT}`", apply = Verb::Apply),
                "anything running as uid 0 could have removed this file another way and left no \
                 record at all — and after a removal there is nothing left to compare against",
            )),
        ));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push((
            "next",
            Value::str(format!(
                "`{id}` IS GONE FROM THIS FLEET SERVER'S REGISTER AND THIS CANNOT BE UNDONE. \
                 rivet-fleetd stops listing it on its next read — NOTHING WAS RESTARTED and \
                 nothing needs to be. `rivet {get} file {devices}` lists what remains.",
                id = self.id,
                get = Verb::Get,
                devices = directory_of(&self.path),
            )),
        ));
        Value::object(fields)
    }
}

/// The command's shape, for every refusal to quote.
fn forms() -> String {
    format!("`rivet {apply} {SUBJECT} <id> [{CONFIRMATION}]`", apply = Verb::Apply)
}

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a malformed command; `Unsafe` for a machine that is not a
/// fleet server (no `register/devices/`), a member that is not there, or
/// something other than a file at the path; `ConfirmationRequired` carrying
/// the plan; `MachineRefused` when the machine could not do it.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Removed, OperationError> {
    let Some(id) = arguments_in.get(1).copied() else {
        return Err(arguments(
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the member's register IDENTIFIER, as the word after \
                 `{SUBJECT}`. {forms}.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };
    if !enroll_member::is_identifier(id) {
        return Err(arguments(
            id,
            format!(
                "`{id}` is not a register identifier. It is {ID_LEN} lowercase hexadecimal \
                 characters (ADR-049 §8.1), so it names ONE file inside register/devices/ and \
                 cannot contain a path separator. {forms}.",
                forms = forms(),
            ),
        ));
    }

    let tail = read_tail(SUBJECT, &forms(), arguments_in, 2)?;
    // No hazard beyond the confirmation itself, so an acknowledgement is
    // refused as superfluous — `enroll-member`'s rule, applied the same way.
    super::apply::accepted(tail.accept, None, tail.confirmed)?;

    // **Composed, never given.** `ServiceName::new` cannot fail for a constant
    // this build controls, but the closed set has no place to panic, so an
    // impossible `None` becomes a refusal naming the defect.
    let Some(service) = ServiceName::new(FLEET_SERVER_APP) else {
        return Err(unsafe_for(
            FLEET_SERVER_APP,
            format!(
                "internal defect: `{FLEET_SERVER_APP}` is not a service name, so this build \
                 cannot compose the fleet register path. NOTHING HAS BEEN REMOVED."
            ),
        ));
    };
    let base = data_dir(STATE_MOUNT_POINT, &service);
    let devices = format!("{base}/{REGISTER_DIR}/{DEVICES_DIR}");
    let path = format!("{devices}/{id}{RECORD_SUFFIX}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;

    resolve(Retraction { id, devices, path }, tail.confirmed, board)
}

/// **The change this subject is**, as `ADR-006` §6 defines one.
struct Retraction<'a> {
    id: &'a str,
    devices: String,
    path: String,
}

/// What planning worked out.
struct Prepared {
    principal: Principal,
}

impl Change for Retraction<'_> {
    type Prepared = Prepared;
    type Applied = Removed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path.as_str();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        self.check_fleet_server(&directory)?;

        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let was_bytes = self.check_removable(&stat)?;
        let principal = board.principal();

        board
            .can_change_path(&self.change(was_bytes))
            .map_err(|e| refused(e, path))?;

        let effect = Effect::remove(path).was(format!("{was_bytes} bytes")).noting(format!(
            "`{id}` is REMOVED from this fleet server's register — THIS CANNOT BE UNDONE, there \
             is no trash on this machine and no snapshot.",
            id = self.id,
        ));

        let plan = Plan::new(
            path,
            format!(
                "retracts `{id}` from this fleet server's register, owned by {app}",
                id = self.id,
                app = FLEET_SERVER_APP,
            ),
        )
        .doing(effect)
        .leaving(
            "NOTHING IS RESTARTED. rivet-fleetd re-reads register/devices/ on every request, so \
             the member is gone on its next read",
        )
        .leaving(
            "every other record in the register is untouched, and this subject checks nothing \
             about WHY the member is being removed: that is the workstation's decision, already \
             made",
        )
        .taking_effect(TakesEffect::NotAsserted {
            because: "the record is gone from disk as soon as this returns, and rivet-fleetd \
                      stops listing it on its next request",
        })
        .confirmed_by(&[SUBJECT, self.id]);

        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Removed, OperationError> {
        let path = self.path.clone();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        self.check_fleet_server(&directory)?;
        let now = board.stat_path(&path).map_err(|e| refused(e, &path))?;
        let was_bytes = self.check_removable(&now)?;

        let wrote = board
            .change_path(&self.change(was_bytes))
            .map_err(|e| refused(e, &path))?;

        Ok(Removed {
            id: self.id.to_string(),
            path,
            bytes: was_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Retraction<'_> {
    /// The act, built in one place so the plan's gate and the removal cannot
    /// describe two different things.
    fn change(&self, was_bytes: u64) -> PathChange<'_> {
        PathChange::RemoveFile { path: &self.path, was_bytes }
    }

    /// **`register/devices/` must already be a directory** — the machine is a
    /// fleet server, and its supervisor made this at provision time. See the
    /// module documentation and [`super::enroll_member`]'s, which this mirrors
    /// exactly.
    fn check_fleet_server(&self, stat: &PathStat) -> Result<(), OperationError> {
        match stat.kind {
            Some(PathKind::Directory) => Ok(()),
            other => Err(unsafe_for(
                &self.devices,
                format!(
                    "REFUSED {devices} is {what} and NOTHING HAS BEEN REMOVED. A fleet server's \
                     supervisor makes it at provision time ({app}'s own register); a machine \
                     without it is not a fleet server, and there is no member here to retract.",
                    devices = self.devices,
                    app = FLEET_SERVER_APP,
                    what = other.map_or_else(
                        || "not there".to_string(),
                        |kind| format!("a {} and not a directory", kind.as_str())
                    ),
                ),
            )),
        }
    }

    /// **The inverse of `enroll-member`'s `check_replaceable`.** A file at the
    /// path is removable; nothing there is refused as `NO SUCH MEMBER` rather
    /// than treated as a success — `super::remove`'s rule, and the same
    /// argument: a mistyped id must not look like a removal for ever. Anything
    /// else at the path is refused rather than touched.
    fn check_removable(&self, stat: &PathStat) -> Result<u64, OperationError> {
        match stat.kind {
            None => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is no member record at {path} — NO SUCH MEMBER, and NOTHING \
                     HAS BEEN REMOVED. `{id}` is not enrolled in this fleet server's register, \
                     or was removed already. This is a refusal rather than a success on purpose: \
                     'nothing was there' and 'I removed it' are different facts, and a verb that \
                     answered the same way to both would let a mistyped id look like a removal \
                     for ever. `rivet {get} file {devices}` lists what is enrolled.",
                    path = self.path,
                    id = self.id,
                    get = Verb::Get,
                    devices = self.devices,
                ),
            )),
            Some(PathKind::File) => Ok(stat.size_bytes),
            Some(other) => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN REMOVED. A \
                     member record is a FILE; removing a {other} here would either destroy \
                     something nothing enrolled or follow a link out of the register.",
                    other = other.as_str(),
                    path = self.path,
                ),
            )),
        }
    }
}

#[cfg(test)]
#[path = "remove_member_tests.rs"]
mod tests;
