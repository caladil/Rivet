//! `rivet apply revoke-phone <id> [confirm]` — **retract one enrolled phone's
//! record.** The exact inverse of [`super::rename_phone`], `ADR-061` §6 —
//! the control-plane tool for taking a phone OUT of a fleet, so an operator
//! never hand-deletes a `register/phones/<id>.toml` over a shell they were
//! not meant to need.
//!
//! # Why a subject of its own, and not `remove-file`
//!
//! `remove-file` (`ADR-006` §8) removes any regular file under `/state` and
//! could in principle reach this one — but it takes a PATH, and this crate's
//! whole file surface holds to one rule: a caller supplies `<id>` alone, and
//! the path is composed from it, never given (see
//! [`super::enroll_member`]'s own documentation for the argument in full). A
//! caller that could name `register/phones/<anything>.toml` directly could
//! also mistype it, and `remove-file`'s refusals are generic where this
//! subject's are specific: `NO SUCH PHONE` rather than *"nothing there"*, and
//! a plan that says `<id>` by name rather than a bare path.
//!
//! # The path is composed, never given
//!
//! `<id>` is a phone identity — 43 characters of the base64url alphabet
//! (`ADR-061` §5) — so it cannot contain a path separator, cannot be `.` or
//! `..`, and names exactly one file inside `register/phones/`. The SAME rule
//! [`super::rename_phone::is_identity`] applies, reused rather than
//! redefined, so the two subjects can never disagree about what an identity
//! is.
//!
//! # What it does NOT do
//!
//! * **It does not create the register**, for the same reason `rename-phone`
//!   does not: a machine without `register/devices/` is not a fleet server,
//!   and this refuses rather than inventing one.
//! * **It restarts nothing.** `rivet-fleetd`'s gantry socket re-reads the
//!   register on every request, so the phone is gone from the next `phones`.
//! * **It checks nothing about WHY the phone is being revoked.** Whether the
//!   phone is lost, replaced or simply no longer wanted is the workstation's
//!   call — this subject carries out a decision already made.
//! * **A phone that is not there is a refusal, not a success** —
//!   `super::remove_member`'s departure from `ADR-006` §6's idempotence, and
//!   the same argument: *"nothing was there"* and *"I revoked it"* are
//!   different facts, and an operator who got the identity wrong needs to
//!   know which one happened.

use super::apply::CONFIRMATION;
use super::enroll_member::{DEVICES_DIR, FLEET_SERVER_APP, REGISTER_DIR};
use super::filesystem::{arguments, check_writable_path, directory_of, read_tail, refused, unsafe_for};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::phones::{PHONES_DIR, RECORD_SUFFIX, check_fleet_server};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::rename_phone::{IDENTITY_CHARS, is_identity};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::service::{ServiceName, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "revoke-phone";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `revoke-phone`.",
    },
    Field {
        name: "identity",
        description: "The phone's identity, as it was given — the name of the record's file \
                      inside register/phones/.",
    },
    Field {
        name: "path",
        description: "The ABSOLUTE path that was removed. Composed from the identity and \
                      never given, so no caller can reach a file outside this fleet server's \
                      register/phones/.",
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
        description: "That the phone is gone from the register and rivet-fleetd's gantry \
                      socket will stop answering with it on its next `phones` request — \
                      nothing was restarted, because it re-reads every request.",
    },
];

/// One phone whose record was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revoked {
    identity: String,
    path: String,
    bytes: u64,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Revoked {
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
            ("identity", Value::str(self.identity.clone())),
            ("path", Value::str(self.path.clone())),
            (
                "before-bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ),
        ];
        fields.push((
            "principal",
            Value::str(self.principal.describes(
                "the phone record",
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
                "`{identity}` IS GONE FROM THIS FLEET SERVER'S REGISTER AND THIS CANNOT BE \
                 UNDONE. rivet-fleetd's gantry socket stops answering with it on its next \
                 `phones` request — NOTHING WAS RESTARTED and nothing needs to be. `rivet {get} \
                 file {phones}` lists what remains.",
                identity = self.identity,
                get = Verb::Get,
                phones = directory_of(&self.path),
            )),
        ));
        Value::object(fields)
    }
}

/// The command's shape, for every refusal to quote.
fn forms() -> String {
    format!("`rivet {apply} {SUBJECT} <identity> [{CONFIRMATION}]`", apply = Verb::Apply)
}

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a malformed command; `Unsafe` for a machine that is not a
/// fleet server (no `register/devices/`), a phone that is not there, or
/// something other than a file at the path; `ConfirmationRequired` carrying
/// the plan; `MachineRefused` when the machine could not do it.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Revoked, OperationError> {
    let Some(identity) = arguments_in.get(1).copied() else {
        return Err(arguments(
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the phone's IDENTITY, as the word after `{SUBJECT}`. \
                 {forms}.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };
    if !is_identity(identity) {
        return Err(arguments(
            identity,
            format!(
                "`{identity}` is not a phone identity. It is {IDENTITY_CHARS} characters of the \
                 base64url alphabet (A-Z, a-z, 0-9, - and _) (ADR-061 §5), so it names ONE \
                 file inside register/phones/ and cannot contain a path separator. {forms}.",
                forms = forms(),
            ),
        ));
    }

    let tail = read_tail(SUBJECT, &forms(), arguments_in, 2)?;
    super::apply::accepted(tail.accept, None, tail.confirmed)?;

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
    let phones_dir = format!("{base}/{REGISTER_DIR}/{PHONES_DIR}");
    let path = format!("{phones_dir}/{identity}{RECORD_SUFFIX}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;

    resolve(Revocation { identity, devices, path }, tail.confirmed, board)
}

/// **The change this subject is**, as `ADR-006` §6 defines one.
struct Revocation<'a> {
    identity: &'a str,
    devices: String,
    path: String,
}

/// What planning worked out.
struct Prepared {
    principal: Principal,
}

impl Change for Revocation<'_> {
    type Prepared = Prepared;
    type Applied = Revoked;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path.as_str();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        check_fleet_server(&self.devices, &directory)?;

        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let was_bytes = self.check_removable(&stat)?;
        let principal = board.principal();

        board
            .can_change_path(&self.change(was_bytes))
            .map_err(|e| refused(e, path))?;

        let effect = Effect::remove(path).was(format!("{was_bytes} bytes")).noting(format!(
            "`{identity}` is REMOVED from this fleet server's register — THIS CANNOT BE UNDONE, \
             there is no trash on this machine and no snapshot.",
            identity = self.identity,
        ));

        let plan = Plan::new(
            path,
            format!(
                "retracts `{identity}`'s record from this fleet server's register",
                identity = self.identity,
            ),
        )
        .doing(effect)
        .leaving(
            "NOTHING IS RESTARTED. rivet-fleetd's gantry socket re-reads register/phones/ at \
             every request, so the phone is gone on its next read",
        )
        .leaving(
            "every other record in the register is untouched, and this subject checks nothing \
             about WHY the phone is being revoked: that is the workstation's decision, already \
             made",
        )
        .taking_effect(TakesEffect::NotAsserted {
            because: "the record is gone from disk as soon as this returns, and rivet-fleetd's \
                      gantry socket stops answering with it on its next request",
        })
        .confirmed_by(&[SUBJECT, self.identity]);

        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Revoked, OperationError> {
        let path = self.path.clone();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        check_fleet_server(&self.devices, &directory)?;
        let now = board.stat_path(&path).map_err(|e| refused(e, &path))?;
        let was_bytes = self.check_removable(&now)?;

        let wrote = board
            .change_path(&self.change(was_bytes))
            .map_err(|e| refused(e, &path))?;

        Ok(Revoked {
            identity: self.identity.to_string(),
            path,
            bytes: was_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Revocation<'_> {
    /// The act, built in one place so the plan's gate and the removal cannot
    /// describe two different things.
    fn change(&self, was_bytes: u64) -> PathChange<'_> {
        PathChange::RemoveFile { path: &self.path, was_bytes }
    }

    /// **The inverse of `rename-phone`'s `check_replaceable`.** A file at the
    /// path is removable; nothing there is refused as `NO SUCH PHONE` rather
    /// than treated as a success — `super::remove_member`'s rule, and the
    /// same argument: a mistyped identity must not look like a revocation
    /// for ever. Anything else at the path is refused rather than touched.
    fn check_removable(&self, stat: &PathStat) -> Result<u64, OperationError> {
        match stat.kind {
            None => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is no phone record at {path} — NO SUCH PHONE, and NOTHING \
                     HAS BEEN REMOVED. `{identity}` is not enrolled in this fleet server's \
                     register, or was revoked already. `rivet {get} phones` lists what is \
                     enrolled.",
                    path = self.path,
                    identity = self.identity,
                    get = Verb::Get,
                ),
            )),
            Some(PathKind::File) => Ok(stat.size_bytes),
            Some(other) => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN REMOVED. A phone \
                     record is a FILE; removing a {other} here would either destroy something \
                     nothing enrolled or follow a link out of the register.",
                    other = other.as_str(),
                    path = self.path,
                ),
            )),
        }
    }
}

#[cfg(test)]
#[path = "revoke_phone_tests.rs"]
mod tests;
