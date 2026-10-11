//! `rivet apply passkey-code [confirm]` — **mint a one-time code that enrolls
//! the first passkey in this machine's proxy.** `ADR-063` §4.
//!
//! The control plane is authenticated by pinned keys; a browser holding no
//! passkey yet has nothing else to prove it is the operator's. So the operator
//! asks the machine for a code over the control plane, types it into
//! `https://<this machine>/sign-in/enroll`, and the browser's new passkey is
//! enrolled. `crate::passkey` holds everything both ends must agree on.
//!
//! # What it does
//!
//! Draws [`crate::passkey::CODE_RANDOM_BYTES`] from the machine's random source,
//! renders them as a code, and writes it to
//! `/state/app/rivet-proxyd/data/passkey-enrolment`, owned by `rivet-proxyd`'s
//! uid with mode `0600` — the act `apply install-secret` performs, by the same
//! [`PathChange::InstallSecret`], so the ownership is set before the file
//! reaches its name. **The code is in the answer and nowhere else**: not in the
//! plan, which is shown before anything is drawn, and not in the journal, which
//! records the path and the length.
//!
//! # What it does not do
//!
//! * **It restarts nothing.** The proxy reads the file when a browser offers a
//!   code, so a code is live the moment this returns.
//! * **It does not keep two codes.** A second code replaces the first.
//! * **It does not create the proxy's data directory.** A machine without one
//!   does not run the proxy, and a code there would enroll nothing.
//! * **It does not withdraw a code.** `apply remove-file` on the path does, and
//!   the answer names the command.

use super::apply::CONFIRMATION;
use super::filesystem::{check_writable_path, read_tail, refused, unsafe_for};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::passkey::{
    CODE_RANDOM_BYTES, ENROLL_PAGE, ENROLLMENT_ATTEMPTS, ENROLLMENT_FILE,
    ENROLLMENT_LIFETIME_SECONDS, PROXY_SERVICE, code_from, display,
};
use crate::service::{ServiceName, app_identity, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "passkey-code";

/// The mode the code is written with — `install-secret`'s, for its reason.
const CODE_MODE: u32 = super::install_secret::SECRET_MODE;

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `passkey-code`.",
    },
    Field {
        name: "code",
        description: "THE ONE-TIME CODE, as a person types it. It appears here and nowhere else \
                      — not in the plan and not in the journal. It enrolls one passkey, for ten \
                      minutes from now.",
    },
    Field {
        name: "path",
        description: "Where the code was written: rivet-proxyd's data directory, owned by the \
                      proxy, mode 0600.",
    },
    Field {
        name: "lifetime-seconds",
        description: "How long the proxy honours the code, from the moment it was written.",
    },
    Field {
        name: "replaced",
        description: "Whether an earlier code was there and is now dead.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked (ADR-028 §4). ATTRIBUTION AND \
                      NOT AUTHORITY.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of the write reached the journal. The record names the \
                      path and the length, never the code.",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of it. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "Where to type the code, what it enrolls, and how to withdraw it.",
    },
];

/// A code that was issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    code: String,
    path: String,
    replaced: bool,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Issued {
    /// The canonical code, as stored.
    #[cfg(test)]
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The structured answer. `ADR-006` §4.
    #[must_use]
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("code", Value::str(display(&self.code))),
            ("path", Value::str(self.path.clone())),
            (
                "lifetime-seconds",
                Value::Integer(i64::try_from(ENROLLMENT_LIFETIME_SECONDS).unwrap_or(i64::MAX)),
            ),
            ("replaced", Value::Bool(self.replaced)),
            (
                "principal",
                Value::str(self.principal.describes(
                    "the enrollment code",
                    &format!("`rivet {apply} {SUBJECT}`", apply = Verb::Apply),
                    "anything running as uid 0 could have written this file another way and left \
                     no record at all",
                )),
            ),
            ("journalled", Value::Bool(self.unrecorded.is_none())),
        ];
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push((
            "next",
            Value::str(format!(
                "Open https://<this machine's name>{ENROLL_PAGE} in the browser that should hold \
                 the passkey and type {code} within {minutes} minutes. It enrolls ONE passkey and \
                 is then deleted; {attempts} wrong codes delete it too. A new `rivet {apply} \
                 {SUBJECT} {CONFIRMATION}` replaces it, and `rivet {apply} remove-file {path} \
                 {CONFIRMATION}` withdraws it. Nothing was restarted: the proxy reads the file \
                 when a browser offers a code.",
                code = display(&self.code),
                minutes = ENROLLMENT_LIFETIME_SECONDS / 60,
                attempts = ENROLLMENT_ATTEMPTS,
                apply = Verb::Apply,
                path = self.path,
            )),
        ));
        Value::object(fields)
    }
}

fn forms() -> String {
    format!("`rivet {apply} {SUBJECT} [{CONFIRMATION}]`", apply = Verb::Apply)
}

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a word this subject does not take; `Unsafe` for a machine
/// whose proxy has no data directory or whose code path holds something other
/// than a file; `ConfirmationRequired` carrying the plan; `MachineRefused` when
/// the machine could not draw random bytes or write the file.
pub(super) fn perform(arguments: &[&str], board: &mut dyn Board) -> Result<Issued, OperationError> {
    let tail = read_tail(SUBJECT, &forms(), arguments, 1)?;
    super::apply::accepted(tail.accept, None, tail.confirmed)?;

    let Some(service) = ServiceName::new(PROXY_SERVICE) else {
        return Err(unsafe_for(
            PROXY_SERVICE,
            format!(
                "internal defect: `{PROXY_SERVICE}` is not a service name, so this build cannot \
                 compose the proxy's data directory. NOTHING HAS BEEN WRITTEN."
            ),
        ));
    };
    let directory = data_dir(STATE_MOUNT_POINT, &service);
    let path = format!("{directory}/{ENROLLMENT_FILE}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;
    let identity = app_identity(&service);
    resolve(
        Issue {
            directory,
            path,
            uid: identity.uid,
            gid: identity.gid,
        },
        tail.confirmed,
        board,
    )
}

struct Issue {
    directory: String,
    path: String,
    uid: u32,
    gid: u32,
}

struct Prepared {
    principal: Principal,
}

impl Change for Issue {
    type Prepared = Prepared;
    type Applied = Issued;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let directory = board
            .stat_path(&self.directory)
            .map_err(|e| refused(e, &self.directory))?;
        self.check_directory(&directory)?;
        let stat = board.stat_path(&self.path).map_err(|e| refused(e, &self.path))?;
        let was = self.check_replaceable(&stat)?;
        let principal = board.principal();

        // The gate is asked of the act that will happen, with stand-in bytes
        // of the right length: the code is not drawn until it is confirmed.
        let stand_in = vec![b'0'; CODE_RANDOM_BYTES];
        board
            .can_change_path(&self.change(&stand_in, was.unwrap_or(0)))
            .map_err(|e| refused(e, &self.path))?;

        let effect = match was {
            None => Effect::create(&self.path),
            Some(bytes) => Effect::set(&self.path, format!("an earlier code ({bytes} bytes)"), "a new one")
                .noting("THE EARLIER CODE STOPS WORKING the moment this is written"),
        }
        .becomes(format!(
            "a new one-time code, owned by `{PROXY_SERVICE}` (uid {uid}), mode 0600, honoured for \
             {minutes} minutes",
            uid = self.uid,
            minutes = ENROLLMENT_LIFETIME_SECONDS / 60,
        ));

        let plan = Plan::new(
            &self.path,
            "issues a one-time code that enrolls the first passkey in this machine's proxy",
        )
        .doing(effect)
        .leaving(
            "the code is DRAWN ONLY WHEN THIS IS CONFIRMED and appears only in that answer — not \
             in this plan and not in the journal",
        )
        .leaving("nothing is restarted: the proxy reads the file when a browser offers a code")
        .leaving("every passkey already enrolled, and every client certificate, keeps working")
        .taking_effect(TakesEffect::Immediately)
        .confirmed_by(&[SUBJECT]);
        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Issued, OperationError> {
        let directory = board
            .stat_path(&self.directory)
            .map_err(|e| refused(e, &self.directory))?;
        self.check_directory(&directory)?;
        let now = board.stat_path(&self.path).map_err(|e| refused(e, &self.path))?;
        let was = self.check_replaceable(&now)?;

        let random = board
            .random_bytes(CODE_RANDOM_BYTES)
            .map_err(|e| refused(e, &self.path))?;
        let Some(code) = code_from(&random) else {
            return Err(unsafe_for(
                &self.path,
                format!(
                    "the machine returned {} random bytes where {CODE_RANDOM_BYTES} were asked \
                     for, so NO CODE WAS WRITTEN",
                    random.len()
                ),
            ));
        };
        let wrote = board
            .change_path(&self.change(code.as_bytes(), was.unwrap_or(0)))
            .map_err(|e| refused(e, &self.path))?;
        Ok(Issued {
            code,
            path: self.path,
            replaced: was.is_some(),
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Issue {
    fn change<'a>(&'a self, contents: &'a [u8], was_bytes: u64) -> PathChange<'a> {
        PathChange::InstallSecret {
            path: &self.path,
            contents,
            mode: CODE_MODE,
            uid: self.uid,
            gid: self.gid,
            was_bytes,
        }
    }

    fn check_directory(&self, stat: &PathStat) -> Result<(), OperationError> {
        match stat.kind {
            Some(PathKind::Directory) => Ok(()),
            _ => Err(unsafe_for(
                &self.directory,
                format!(
                    "REFUSED `{directory}` is not a directory, so this machine does not run \
                     `{PROXY_SERVICE}` and a code here would enroll nothing. NOTHING HAS BEEN \
                     WRITTEN. The proxy runs where the document enables \
                     `[services.{PROXY_SERVICE}]` (ADR-060).",
                    directory = self.directory,
                ),
            )),
        }
    }

    fn check_replaceable(&self, stat: &PathStat) -> Result<Option<u64>, OperationError> {
        match stat.kind {
            None => Ok(None),
            Some(PathKind::File) => Ok(Some(stat.size_bytes)),
            Some(other) => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN WRITTEN. The code \
                     is a FILE; writing through a {other} could put it somewhere the proxy does \
                     not look.",
                    other = other.as_str(),
                    path = self.path,
                ),
            )),
        }
    }
}

#[cfg(test)]
#[path = "passkey_code_tests.rs"]
mod tests;
