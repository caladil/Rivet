//! `rivet apply enroll-member <id> --hex <hex> [confirm]` — **write one fleet
//! member's register record into this fleet server's register.** `ADR-059`
//! §6.1, the never-built half of `ADR-034` §2 ("the first record is written
//! onto the card; every record after it arrives over the control plane").
//!
//! A fleet server (`rivet-fleetd`) keeps its register at
//! `/state/app/rivet-fleetd/data/register/`, and it only ever *reads* it —
//! `cmd/rivet-fleetd/src/fleet.rs` re-reads the whole directory on every
//! `GET /api/fleet`, discovering members by scanning `devices/<id>.toml`, with
//! no index in `register.toml`. So enrolling a member is exactly: put one
//! `devices/<id>.toml` where `rivet-fleetd` can read it, and it appears on the
//! next read. Nothing on the fleet server writes that file today; this subject
//! is the control-plane operation that does.
//!
//! # Why a subject of its own, and not `install-secret` or `restore-secret`
//!
//! This IS [`install_secret`]'s act — [`PathChange::InstallSecret`], a
//! temporary written, owned and renamed into place — but neither sibling
//! reaches the register:
//!
//! * **`install-secret`** composes `/state/app/<app>/data/<name>` and forbids a
//!   path separator in `<name>`, so it writes ONE flat file in an app's data
//!   directory and cannot reach the `register/devices/` subtree.
//! * **`restore-secret`** composes `/state/secrets/<name>`, owner fixed at root.
//!
//! So this composes `/state/app/rivet-fleetd/data/register/devices/<id>.toml`
//! and fixes the owner at `rivet-fleetd`'s own derived uid — the file is the
//! server's to read, exactly as `install-secret` hands an application its own
//! certificate ([`app_identity`]).
//!
//! # The path is composed, never given
//!
//! `<id>` is a register identifier — sixteen lowercase hexadecimal characters
//! (`ADR-049` §8) — so it cannot contain a path separator, cannot be `.` or
//! `..`, and names exactly one file inside `register/devices/`. The traversal
//! question is answered by the shape of the command, as it is for
//! `install-secret` and `restore-secret`.
//!
//! # The bytes are the member's record, composed on the workstation
//!
//! This subject writes what it is given; it does not know the register's
//! format. `cargo rivet` (or provisioning) renders the member's
//! `rivetos_register::Device` to the exact TOML `rivet-fleetd`'s `read_device`
//! parses back, and carries it as hex — the control plane is JSON (`ADR-035`
//! §3.2), and hex is what `install-secret` chose for bytes on it.
//!
//! # What it does NOT do
//!
//! * **It does not create the register.** `register/devices/` exists on any
//!   provisioned fleet server: the supervisor stages the server's own record
//!   there at provision time (`ADR-034` §3, `crate::staging`). A machine
//!   without that directory is not a fleet server, and this refuses rather than
//!   inventing one — a member cannot join a fleet that has no server.
//! * **It restarts nothing.** `rivet-fleetd` re-reads the register on every
//!   request, so the record is live on the next `GET /api/fleet`.
//! * **It checks nothing about the member itself.** Whether the record is
//!   well-formed, and whether its id or pin collides with one already enrolled,
//!   is the workstation's refusal (it holds the register it is adding to); a
//!   command that carries bytes cannot re-derive that context.

use super::apply::CONFIRMATION;
use super::filesystem::{
    arguments, check_writable_path, describe_mode, read_tail, refused, unsafe_for,
};
use super::install_secret::{self, HEX_FLAG, SECRET_MODE};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::service::{ServiceName, app_identity, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "enroll-member";

/// The application whose data directory holds the fleet register.
///
/// Spelled here because `rivet-core` depends on nothing that owns it:
/// `cmd/rivet-fleetd`'s `Cargo.toml` `[package] name` is the spelling the
/// server side reads, and `cmd/cargo-rivet`'s tests hold the two equal.
pub const FLEET_SERVER_APP: &str = "rivet-fleetd";

/// The register directory inside the fleet server's data directory, and the
/// `devices/` subtree of per-member records within it — the layout
/// `rivetos_register::store` writes and `cmd/rivet-fleetd/src/fleet.rs` reads.
/// Spelled here for [`FLEET_SERVER_APP`]'s reason; held equal to
/// `rivetos_register`'s constants by a `cmd/cargo-rivet` test.
///
/// `pub(super)`: [`super::remove_member`] composes the SAME path to retract a
/// record it does not enroll, and a second spelling of this layout is exactly
/// the drift `ADR-011` exists to close.
pub(super) const REGISTER_DIR: &str = "register";
pub(super) const DEVICES_DIR: &str = "devices";
pub(super) const RECORD_SUFFIX: &str = ".toml";

/// A register identifier: sixteen lowercase hexadecimal characters
/// (`ADR-049` §8). This is the whole of what confines `<id>` to one filename
/// inside `register/devices/`.
pub(super) const ID_LEN: usize = 16;

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `enroll-member`.",
    },
    Field {
        name: "id",
        description: "The member's register identifier, as it was given — the name of the \
                      record's file inside register/devices/.",
    },
    Field {
        name: "path",
        description: "The ABSOLUTE path written. Composed from the id and never given, so no \
                      caller can put a file outside this fleet server's register/devices/.",
    },
    Field {
        name: "bytes",
        description: "How many bytes of record were written, DECODED. ",
    },
    Field {
        name: "replaced-bytes",
        description: "How many bytes were there before. ABSENT when nothing was there (a first \
                      enrollment); present when a record for this id was replaced (a re-enroll).",
    },
    Field {
        name: "mode",
        description: "The mode it was written with. Always 0600, owner rivet-fleetd, so the \
                      fleet server can read its own register and nothing else can.",
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
        description: "That the member is in the register and rivet-fleetd will list it on its \
                      next read — nothing was restarted, because it re-reads every request.",
    },
];

/// One member that was enrolled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrolled {
    id: String,
    path: String,
    bytes: usize,
    replaced_bytes: Option<u64>,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Enrolled {
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
                "bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ),
        ];
        if let Some(replaced) = self.replaced_bytes {
            fields.push((
                "replaced-bytes",
                Value::Integer(i64::try_from(replaced).unwrap_or(i64::MAX)),
            ));
        }
        fields.push(("mode", Value::str(describe_mode(SECRET_MODE))));
        fields.push((
            "principal",
            Value::str(self.principal.describes(
                "the member record",
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
                "`{id}` is in this fleet server's register, and rivet-fleetd lists it on its \
                 next read — it re-reads register/devices/ on every request, so NOTHING WAS \
                 RESTARTED and nothing needs to be. `rivet {get} file {path}` says what is on \
                 disk.",
                id = self.id,
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
        "`rivet {apply} {SUBJECT} <id> {HEX_FLAG} <hex> [{CONFIRMATION}]`",
        apply = Verb::Apply,
    )
}

/// Whether `text` is a register identifier this subject will write a file for —
/// sixteen lowercase hexadecimal characters, so it names exactly one file and
/// reaches no other directory.
///
/// `pub(super)`: [`super::remove_member`] uses this SAME rule to decide what it
/// will remove a file for, rather than a second definition of the shape that
/// could one day disagree.
#[must_use]
pub(super) fn is_identifier(text: &str) -> bool {
    text.len() == ID_LEN && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a malformed command; `Unsafe` for a machine that is not a
/// fleet server (no `register/devices/`) or something other than a file at the
/// path; `ConfirmationRequired` carrying the plan; `MachineRefused` when the
/// machine could not do it.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Enrolled, OperationError> {
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
    if !is_identifier(id) {
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

    match arguments_in.get(2).copied() {
        Some(HEX_FLAG) => {}
        Some(other) => {
            return Err(arguments(
                other,
                format!(
                    "`{other}` is where `{HEX_FLAG}` should be. The record is always named by \
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
                "`{HEX_FLAG}` decoded to zero bytes, and an empty file is not a member record. \
                 {forms}.",
                forms = forms(),
            ),
        ));
    }

    let tail = read_tail(SUBJECT, &forms(), arguments_in, 4)?;
    // No hazard beyond the confirmation itself, so an acknowledgement is
    // refused as superfluous — `super::apply::accepted`'s rule, as
    // `restore-secret` applies it.
    super::apply::accepted(tail.accept, None, tail.confirmed)?;

    // **Composed, never given.** `ServiceName::new` cannot fail for a constant
    // this build controls, but the closed set has no place to panic, so an
    // impossible `None` becomes a refusal naming the defect.
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
    let path = format!("{devices}/{id}{RECORD_SUFFIX}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;

    let identity = app_identity(&service);
    resolve(
        Enrollment {
            id,
            devices,
            path,
            contents,
            uid: identity.uid,
            gid: identity.gid,
        },
        tail.confirmed,
        board,
    )
}

/// **The change this subject is**, as `ADR-006` §6 defines one.
struct Enrollment<'a> {
    id: &'a str,
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

impl Change for Enrollment<'_> {
    type Prepared = Prepared;
    type Applied = Enrolled;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path.as_str();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        self.check_fleet_server(&directory)?;

        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let was_bytes = self.check_replaceable(&stat)?;
        let principal = board.principal();

        board
            .can_change_path(&self.change(was_bytes.unwrap_or(0)))
            .map_err(|e| refused(e, path))?;

        let effect = match was_bytes {
            None => Effect::create(path)
                .becomes(format!("{} bytes", self.contents.len()))
                .noting(format!(
                    "NOTHING IS THERE NOW, so `{id}` is a member this fleet server has not held \
                     before.",
                    id = self.id,
                )),
            Some(was) => Effect::set(
                path,
                format!("{was} bytes"),
                format!("{} bytes", self.contents.len()),
            )
            .noting(format!(
                "A record for `{id}` was already here and IS REPLACED — a re-enrollment. The old \
                 bytes are gone.",
                id = self.id,
            )),
        };

        let plan = Plan::new(
            path,
            format!(
                "enrolls `{id}` into this fleet server's register, owned by {app}",
                id = self.id,
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
            "NOTHING IS RESTARTED. rivet-fleetd re-reads register/devices/ on every request, so \
             the member is listed on its next read",
        )
        .leaving(
            "every other record in the register is untouched, and this subject checks nothing \
             about the record's contents or whether its id or pin collides: that is the \
             workstation's refusal, which holds the register it is adding to",
        )
        .taking_effect(TakesEffect::NotAsserted {
            because: "the record is on disk as soon as this returns, and rivet-fleetd reads it on \
                      its next request",
        });

        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Enrolled, OperationError> {
        let path = self.path.clone();

        let directory = board.stat_path(&self.devices).map_err(|e| refused(e, &self.devices))?;
        self.check_fleet_server(&directory)?;
        let now = board.stat_path(&path).map_err(|e| refused(e, &path))?;
        let was_bytes = self.check_replaceable(&now)?;

        let wrote = board
            .change_path(&self.change(was_bytes.unwrap_or(0)))
            .map_err(|e| refused(e, &path))?;

        Ok(Enrolled {
            id: self.id.to_string(),
            path,
            bytes: self.contents.len(),
            replaced_bytes: was_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Enrollment<'_> {
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

    /// **`register/devices/` must already be a directory** — the machine is a
    /// fleet server, and its supervisor made this at provision time. See the
    /// module documentation.
    fn check_fleet_server(&self, stat: &PathStat) -> Result<(), OperationError> {
        match stat.kind {
            Some(PathKind::Directory) => Ok(()),
            other => Err(unsafe_for(
                &self.devices,
                format!(
                    "REFUSED {devices} is {what} and NOTHING HAS BEEN WRITTEN. A fleet server's \
                     supervisor makes it at provision time ({app}'s own register); a machine \
                     without it is not a fleet server, and a member cannot join a fleet that has \
                     no server. Provision this machine as a Server first.",
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

    /// A file or nothing at the path; anything else is refused.
    fn check_replaceable(&self, stat: &PathStat) -> Result<Option<u64>, OperationError> {
        match stat.kind {
            None => Ok(None),
            Some(PathKind::File) => Ok(Some(stat.size_bytes)),
            Some(other) => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN WRITTEN. A member \
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
#[path = "enroll_member_tests.rs"]
mod tests;
