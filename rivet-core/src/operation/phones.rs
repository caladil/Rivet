//! `rivet get phones` — **every phone this machine's fleet server has
//! enrolled**, as raw record bytes. `ADR-061` §5 and §6's board-side half of
//! `cargo rivet fleet phones`.
//!
//! # Why this hands back bytes and not a label, a key and a timestamp
//!
//! `ADR-014` §3 gives `rivet-core` an empty `[dependencies]` table, and
//! `rivetos-register` depends on this crate — so the edge cannot run the other
//! way, and `rivet-core` can hold no opinion about what a phone record's TOML
//! means. Duplicating `rivetos-register::phones`'s reader here, by hand, would
//! be a second parser of one format (`ADR-011`), and this build's own
//! `AGENTS.md` §8.4 already answers a version of this question for `cmd/rivet`
//! specifically: it is the LOGIN SHELL, and every dependency linked into it
//! is argued at its own entry in that crate's manifest — the place a decision
//! to add `rivetos-register` there would belong, and where it has not been
//! made.
//!
//! So this subject reads `register/phones/` through the SAME [`Editor`] port
//! `get file` already reads any path under `/state` with, and returns each
//! record's bytes untouched — exactly what [`super::enroll_member`]'s own
//! documentation already points a caller at: *"`rivet get file <path>` says
//! what is on disk."* `cargo rivet fleet phones list` is what decodes a
//! record, because `cmd/cargo-rivet` already depends on `rivetos-register` and
//! carries none of `cmd/rivet`'s image-size argument.
//!
//! # What it does NOT do
//!
//! * **It does not create the register.** A machine without
//!   `register/devices/` is not a fleet server, the same check
//!   [`super::enroll_member`] makes, and this refuses rather than inventing
//!   one.
//! * **A register with no `phones/` directory is a register with no
//!   phones.** `ADR-061` §5 — this is `Ok` with an empty list, not a
//!   refusal, on a fleet server that has enrolled none.
//! * **It verifies nothing about a record's contents.** Whether the bytes at
//!   a path are a phone record `rivetos-register` can read is a question for
//!   whoever decodes them.

use super::enroll_member::{DEVICES_DIR, FLEET_SERVER_APP, REGISTER_DIR};
use super::filesystem::{refused, unsafe_for};
use super::machine::{Board, PathKind};
use super::{ErrorCode, Field, OperationError};
use crate::package::key_material::to_hex;
use crate::service::{ServiceName, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "phones";

/// The directory phone records live in, under the register root — the same
/// spelling `rivetos-register::phones::PHONES_DIR` uses. `pub(super)` so
/// [`super::rename_phone`] and [`super::revoke_phone`] compose the SAME path
/// rather than a second copy of it.
pub(super) const PHONES_DIR: &str = "phones";

/// The suffix every phone record carries — `rivetos-register::RECORD_SUFFIX`'s
/// spelling.
pub(super) const RECORD_SUFFIX: &str = ".toml";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `phones`.",
    },
    Field {
        name: "count",
        description: "How many phones this fleet server has enrolled, as a count rather than \
                      the length of a list a caller has to measure. ZERO IS A REAL ANSWER: it \
                      is what a fleet server with none enrolled looks like.",
    },
    Field {
        name: "phones",
        description: "Every phone, sorted by identity. Each carries `identity` (the file name \
                      inside register/phones/, without its .toml suffix) and `record` (the \
                      WHOLE FILE, as hexadecimal). THIS SUBJECT DOES NOT DECODE A RECORD: \
                      rivet-core holds no TOML reader for rivetos-register's format (ADR-014 \
                      §3), so the label, the key and the enrollment time are `cargo rivet fleet \
                      phones list`'s to extract, on the workstation. `rivet get file <path>` \
                      reads the SAME bytes for one phone, at register/phones/<identity>.toml \
                      under this fleet server's data directory.",
    },
];

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a second word; `Unsafe` for a machine that is not a fleet
/// server (no `register/devices/`) or a `register/phones/` that is not a
/// directory; `MachineRefused` when a record could not be read.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            (*extra).to_string(),
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. Every phone this fleet \
                 server has enrolled is the answer; select one from it with a JSON tool rather \
                 than with an argument this build does not have."
            ),
        ));
    }

    // **Composed, never given.** `ServiceName::new` cannot fail for a constant
    // this build controls, but the closed set has no place to panic, so an
    // impossible `None` becomes a refusal naming the defect — `enroll-member`'s
    // rule, applied the same way.
    let Some(service) = ServiceName::new(FLEET_SERVER_APP) else {
        return Err(unsafe_for(
            FLEET_SERVER_APP,
            format!(
                "internal defect: `{FLEET_SERVER_APP}` is not a service name, so this build \
                 cannot compose the fleet register path."
            ),
        ));
    };
    let base = data_dir(STATE_MOUNT_POINT, &service);
    let devices = format!("{base}/{REGISTER_DIR}/{DEVICES_DIR}");
    let phones_dir = format!("{base}/{REGISTER_DIR}/{PHONES_DIR}");

    let directory_stat = board.stat_path(&devices).map_err(|e| refused(e, &devices))?;
    check_fleet_server(&devices, &directory_stat)?;

    let phones_stat = board.stat_path(&phones_dir).map_err(|e| refused(e, &phones_dir))?;
    let entries = match phones_stat.kind {
        // A register with no `phones/` directory is a register with no
        // phones (ADR-061 §5) — every register written before phones
        // existed reads unchanged.
        None => Vec::new(),
        Some(PathKind::Directory) => phones_stat.entries,
        Some(other) => {
            return Err(unsafe_for(
                &phones_dir,
                format!(
                    "REFUSED {phones_dir} is a {} and not a directory. A fleet server's phones \
                     live in a directory `rivetos-register` creates on first enrollment; this is \
                     something else and nothing here has been read from it.",
                    other.as_str()
                ),
            ));
        }
    };

    let mut phones: Vec<(String, Value)> = Vec::new();
    for entry in &entries {
        if entry.kind != PathKind::File {
            continue;
        }
        let Some(identity) = entry.name.strip_suffix(RECORD_SUFFIX) else {
            continue;
        };
        let path = format!("{phones_dir}/{}", entry.name);
        let read = board.read_file(&path).map_err(|e| refused(e, &path))?;
        let Some(contents) = read.contents else {
            // Raced away between the listing and the read. `get log` is the
            // authority for what changed; this answer simply does not
            // include a phone that is no longer there.
            continue;
        };
        phones.push((
            identity.to_string(),
            Value::object(vec![
                ("identity", Value::str(identity)),
                ("record", Value::str(to_hex(&contents))),
            ]),
        ));
    }
    phones.sort_by(|a, b| a.0.cmp(&b.0));
    let phones: Vec<Value> = phones.into_iter().map(|(_, v)| v).collect();

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        (
            "count",
            Value::Integer(i64::try_from(phones.len()).unwrap_or(i64::MAX)),
        ),
        ("phones", Value::List(phones)),
    ];
    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get phones result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// **`register/devices/` must already be a directory** — the machine is a
/// fleet server, and its supervisor made this at provision time. See
/// [`super::enroll_member`]'s own documentation, which this mirrors exactly.
pub(super) fn check_fleet_server(
    devices: &str,
    stat: &super::machine::PathStat,
) -> Result<(), OperationError> {
    match stat.kind {
        Some(PathKind::Directory) => Ok(()),
        other => Err(unsafe_for(
            devices,
            format!(
                "REFUSED {devices} is {what} and NOTHING WAS READ. A fleet server's supervisor \
                 makes it at provision time ({app}'s own register); a machine without it is not \
                 a fleet server. Provision this machine as a Server first.",
                app = FLEET_SERVER_APP,
                what = other.map_or_else(
                    || "not there".to_string(),
                    |kind| format!("a {} and not a directory", kind.as_str())
                ),
            ),
        )),
    }
}

#[cfg(test)]
#[path = "phones_tests.rs"]
mod tests;
