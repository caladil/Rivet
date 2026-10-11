//! `rivet apply rename-phone <id> --hex <hex> [confirm]` — **rewrite one
//! enrolled phone's record.** `ADR-061` §6, the control-plane half of
//! `cargo rivet fleet phones rename`.
//!
//! This IS [`super::enroll_member`]'s act — [`PathChange::InstallSecret`], a
//! temporary written, owned and renamed into place — composed under the fleet
//! server's `register/phones/` instead of `register/devices/`, with one
//! difference: **a rename requires the record already exist.** Enrolling a
//! member may create a first record; renaming a phone that was never
//! enrolled is a caller acting on a stale or wrong identity, and is refused
//! by name rather than silently creating one (`super::remove_member`'s
//! argument, applied to a write instead of a removal).
//!
//! # The bytes are the phone's WHOLE record, composed on the workstation
//!
//! This subject writes what it is given; it does not know the register's
//! format — see [`super::phones`] for why `rivet-core` cannot. `cargo rivet
//! fleet phones rename` reads the current record (`get phones`), decodes it
//! with `rivetos-register::phones::read_record`, changes only the label, and
//! renders the WHOLE record back with `rivetos-register::phones::render` —
//! the same shape `cargo rivet` already renders for `enroll-member`. So a
//! caller here cannot change the identity, the key or the enrollment time
//! without this subject seeing new bytes for all four fields; it never
//! edits one field in place.
//!
//! # The path is composed, never given
//!
//! `<id>` is a phone identity — 43 characters of the base64url alphabet
//! (`ADR-061` §5, `rivetos-register::phones::IDENTITY_CHARS`) — so it
//! cannot contain a path separator, cannot be `.` or `..`, and names exactly
//! one file inside `register/phones/`.
//!
//! # What it does NOT do
//!
//! * **It does not create the register or `register/phones/`.** A machine
//!   without `register/devices/` is not a fleet server, the same check
//!   [`super::enroll_member`] makes.
//! * **It restarts nothing.** `rivet-fleetd` re-reads the register on every
//!   request over its `gantry` socket (`ADR-061` §6), so the new record is
//!   live on the next read.
//! * **It checks nothing about the label** beyond what fits in this build's
//!   argument grammar. `rivetos-register::phones::check_label` is what the
//!   bytes it is handed must already satisfy; a caller that got that wrong
//!   learns from the record it reads back, not from this subject.

use super::apply::CONFIRMATION;
use super::enroll_member::{DEVICES_DIR, FLEET_SERVER_APP, REGISTER_DIR};
use super::filesystem::{arguments, check_writable_path, describe_mode, read_tail, refused, unsafe_for};
use super::install_secret::{self, HEX_FLAG, SECRET_MODE};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::phones::{PHONES_DIR, RECORD_SUFFIX, check_fleet_server};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::service::{ServiceName, app_identity, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "rename-phone";

/// How long a phone identity is: base64url, unpadded, of a SHA-256.
/// `rivetos-register::phones::IDENTITY_CHARS`'s spelling, duplicated here for
/// `AGENTS.md` §10.1's reason `enroll-member` already gives for its own
/// `ID_LEN`: `rivet-core` cannot depend on the crate that owns the constant.
/// `pub(super)` so [`super::revoke_phone`] shares it.
pub(super) const IDENTITY_CHARS: usize = 43;

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `rename-phone`.",
    },
    Field {
        name: "identity",
        description: "The phone's identity, as it was given — the name of the record's file \
                      inside register/phones/.",
    },
    Field {
        name: "path",
        description: "The ABSOLUTE path written. Composed from the identity and never given, \
                      so no caller can put a file outside this fleet server's register/phones/.",
    },
    Field {
        name: "bytes",
        description: "How many bytes of record were written, DECODED.",
    },
    Field {
        name: "replaced-bytes",
        description: "How many bytes were there before this rewrite.",
    },
    Field {
        name: "mode",
        description: "The mode it was written with. Always 0600, owner rivet-fleetd.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked (ADR-028 §4). ATTRIBUTION AND \
                      NOT AUTHORITY.",
    },
    Field {
        name: "journalled",
        description: "Whether a record reached the journal. The record names the path and the \
                      length, never the content.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of it. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "That the phone's record is rewritten and rivet-fleetd's gantry socket \
                      will answer with it on its next `phones` request — nothing was \
                      restarted, because it re-reads register/phones/ at every request.",
    },
];

/// One phone whose record was rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    identity: String,
    path: String,
    bytes: usize,
    replaced_bytes: u64,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Renamed {
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
                "bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ),
            (
                "replaced-bytes",
                Value::Integer(i64::try_from(self.replaced_bytes).unwrap_or(i64::MAX)),
            ),
        ];
        fields.push(("mode", Value::str(describe_mode(SECRET_MODE))));
        fields.push((
            "principal",
            Value::str(self.principal.describes(
                "the phone record",
                &format!("`rivet {apply} {SUBJECT}`", apply = Verb::Apply),
                "anything running as uid 0 could have written this file another way and left no \
                 record at all",
            )),
        ));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push((
            "next",
            Value::str(format!(
                "`{identity}`'s record is rewritten in this fleet server's register, and \
                 rivet-fleetd's gantry socket answers with it on its next `phones` request — it \
                 re-reads register/phones/ at every request, so NOTHING WAS RESTARTED and \
                 nothing needs to be. `rivet {get} file {path}` says what is on disk.",
                identity = self.identity,
                get = Verb::Get,
                path = self.path,
            )),
        ));
        Value::object(fields)
    }
}

/// The command's shape, for every refusal to quote.
fn forms() -> String {
    format!(
        "`rivet {apply} {SUBJECT} <identity> {HEX_FLAG} <hex> [{CONFIRMATION}]`",
        apply = Verb::Apply,
    )
}

/// Whether `text` is a phone identity this subject will write a file for —
/// `IDENTITY_CHARS` characters of the base64url alphabet, so it names exactly
/// one file and reaches no other directory. `pub(super)` so
/// [`super::revoke_phone`] uses this SAME rule rather than a second
/// definition that could one day disagree.
#[must_use]
pub(super) fn is_identity(text: &str) -> bool {
    text.len() == IDENTITY_CHARS
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a malformed command; `Unsafe` for a machine that is not a
/// fleet server (no `register/devices/`), a phone not enrolled under that
/// identity, or something other than a file at the path; `ConfirmationRequired`
/// carrying the plan; `MachineRefused` when the machine could not do it.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Renamed, OperationError> {
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

    match arguments_in.get(2).copied() {
        Some(HEX_FLAG) => {}
        Some(other) => {
            return Err(arguments(
                other,
                format!(
                    "`{other}` is where `{HEX_FLAG}` should be. The record is always given by \
                     its encoding. {forms}.",
                    forms = forms(),
                ),
            ));
        }
        None => {
            return Err(arguments(
                SUBJECT,
                format!("`{HEX_FLAG} <hex>` is missing. {forms}.", forms = forms()),
            ));
        }
    }
    let Some(hex) = arguments_in.get(3).copied() else {
        return Err(arguments(
            HEX_FLAG,
            format!(
                "`{HEX_FLAG}` is the last word and has no value after it. {forms}.",
                forms = forms(),
            ),
        ));
    };
    if hex == CONFIRMATION || hex == super::apply::ACCEPT_FLAG {
        return Err(arguments(
            hex,
            format!(
                "`{HEX_FLAG}` is followed by `{hex}`, one of this command's own words rather \
                 than a hexadecimal value, so `{HEX_FLAG}` has no content and `{hex}` has been \
                 eaten."
            ),
        ));
    }
    let contents = install_secret::decode(hex)?;
    if contents.is_empty() {
        return Err(arguments(
            hex,
            format!(
                "`{HEX_FLAG}` decoded to zero bytes, and an empty file is not a phone record. \
                 {forms}.",
                forms = forms(),
            ),
        ));
    }

    let tail = read_tail(SUBJECT, &forms(), arguments_in, 4)?;
    super::apply::accepted(tail.accept, None, tail.confirmed)?;

    let Some(service) = ServiceName::new(FLEET_SERVER_APP) else {
        return Err(unsafe_for(
            FLEET_SERVER_APP,
            format!(
                "internal defect: `{FLEET_SERVER_APP}` is not a service name, so this build \
                 cannot compose the fleet register path. NOTHING HAS BEEN WRITTEN."
            ),
        ));
    };
    let base = data_dir(STATE_MOUNT_POINT, &service);
    let devices = format!("{base}/{REGISTER_DIR}/{DEVICES_DIR}");
    let phones_dir = format!("{base}/{REGISTER_DIR}/{PHONES_DIR}");
    let path = format!("{phones_dir}/{identity}{RECORD_SUFFIX}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;

    let identity_ids = app_identity(&service);
    resolve(
        Rename {
            identity,
            devices,
            path,
            contents,
            uid: identity_ids.uid,
            gid: identity_ids.gid,
        },
        tail.confirmed,
        board,
    )
}

/// **The change this subject is**, as `ADR-006` §6 defines one.
struct Rename<'a> {
    identity: &'a str,
    devices: String,
    path: String,
    contents: Vec<u8>,
    uid: u32,
    gid: u32,
}

/// What planning worked out.
struct Prepared {
    principal: Principal,
}

impl Change for Rename<'_> {
    type Prepared = Prepared;
    type Applied = Renamed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path.as_str();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        check_fleet_server(&self.devices, &directory)?;

        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let was_bytes = self.check_replaceable(&stat)?;
        let principal = board.principal();

        board
            .can_change_path(&self.change(was_bytes))
            .map_err(|e| refused(e, path))?;

        let effect = Effect::set(
            path,
            format!("{was_bytes} bytes"),
            format!("{} bytes", self.contents.len()),
        )
        .noting(format!(
            "`{identity}`'s record IS REPLACED. The old bytes are gone.",
            identity = self.identity,
        ));

        let plan = Plan::new(
            path,
            format!(
                "rewrites `{identity}`'s record in this fleet server's register, owned by {app}",
                identity = self.identity,
                app = FLEET_SERVER_APP,
            ),
        )
        .doing(effect)
        .doing(
            Effect::set(
                path,
                "whatever owner and mode it has now",
                format!("uid {}, gid {}, mode {}", self.uid, self.gid, describe_mode(SECRET_MODE)),
            )
            .noting(
                "Written to a temporary, owned and renamed into place, so rivet-fleetd sees the \
                 old record or the new one and never half of either.",
            ),
        )
        .leaving(
            "NOTHING IS RESTARTED. rivet-fleetd re-reads register/phones/ at every gantry \
             socket request, so the new record is answered on its next read",
        )
        .leaving(
            "every other record in the register is untouched, and this subject checks nothing \
             about the label beyond this build's argument grammar: the bytes it was given are \
             what rivetos-register wrote them from, on the workstation",
        )
        .taking_effect(TakesEffect::NotAsserted {
            because: "the record is on disk as soon as this returns, and rivet-fleetd reads it \
                      on its next request",
        })
        .confirmed_by(&[SUBJECT, self.identity]);

        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Renamed, OperationError> {
        let path = self.path.clone();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        check_fleet_server(&self.devices, &directory)?;
        let now = board.stat_path(&path).map_err(|e| refused(e, &path))?;
        let was_bytes = self.check_replaceable(&now)?;

        let wrote = board
            .change_path(&self.change(was_bytes))
            .map_err(|e| refused(e, &path))?;

        Ok(Renamed {
            identity: self.identity.to_string(),
            path,
            bytes: self.contents.len(),
            replaced_bytes: was_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Rename<'_> {
    /// The act, built in one place so the plan's gate and the write cannot
    /// describe two different things.
    fn change(&self, was_bytes: u64) -> PathChange<'_> {
        PathChange::InstallSecret {
            path: &self.path,
            contents: &self.contents,
            mode: SECRET_MODE,
            uid: self.uid,
            gid: self.gid,
            was_bytes,
        }
    }

    /// **A file must already be at the path — a rename never creates.** The
    /// inverse of `enroll-member`'s `check_replaceable`, which allows a first
    /// write; renaming a phone that was never enrolled is a caller acting on
    /// a stale or wrong identity, and is refused by name
    /// (`super::remove_member`'s `check_removable`, applied to a write).
    fn check_replaceable(&self, stat: &PathStat) -> Result<u64, OperationError> {
        match stat.kind {
            None => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is no phone record at {path} — NO SUCH PHONE, and NOTHING \
                     HAS BEEN WRITTEN. `{identity}` is not enrolled in this fleet server's \
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
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN WRITTEN. A phone \
                     record is a FILE; writing over a {other} would either destroy something \
                     nothing enrolled or follow a link out of the register.",
                    other = other.as_str(),
                    path = self.path,
                ),
            )),
        }
    }
}

#[cfg(test)]
#[path = "rename_phone_tests.rs"]
mod tests;
