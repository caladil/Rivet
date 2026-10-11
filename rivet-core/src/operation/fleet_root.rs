//! `rivet get fleet-root` — **the public certificate of the fleet root this
//! machine minted**, and its SHA-256 fingerprint. `ADR-063` §4.
//!
//! # Why a subject, when `get file` can read the same bytes
//!
//! A workstation fetches the root over the pinned control plane, and the pin
//! is what vouches for it: nothing else stands between a device's trust store
//! and whatever answered. So the subject composes the one path the root's
//! certificate rests at (`crate::fleet_root::root_certificate_path`) rather
//! than trusting a caller to type it, and it answers the fingerprint a trust
//! store will show beside the certificate, so the person installing it can
//! compare the two by eye.
//!
//! # What it never answers
//!
//! **The root's private key.** It rests beside the certificate and is not a
//! path this subject can be made to read: the subject takes no argument.
//!
//! # A machine with no root is an answer
//!
//! `present` is false and `means` says why that is ordinary: the machine is
//! not the server of a fleet-root mode fleet, or its first boot has not minted
//! one. A certificate that is there and will not decode is a refusal.

use super::filesystem::refused;
use super::machine::Board;
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "fleet-root";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `fleet-root`.",
    },
    Field {
        name: "path",
        description: "Where the root's certificate rests on this machine.",
    },
    Field {
        name: "present",
        description: "Whether this machine holds a fleet root. FALSE IS A REAL ANSWER: every \
                      machine but a fleet-root mode fleet server's, and one whose first boot has \
                      not minted it, holds none.",
    },
    Field {
        name: "certificate",
        description: "The root's certificate, PEM, exactly as it rests — what a device's trust \
                      store installs. Present only when `present` is true. The private key is \
                      never answered.",
    },
    Field {
        name: "sha256",
        description: "SHA-256 of the certificate's DER, lower-case hex: the fingerprint a trust \
                      store shows beside it. Present only when `present` is true.",
    },
    Field {
        name: "means",
        description: "One sentence on what the answer is for.",
    },
];

/// Perform it.
///
/// # Errors
///
/// `Arguments` for any word after the subject; `MachineRefused` when the
/// certificate could not be read, will not decode, or could not be
/// fingerprinted.
pub(super) fn perform(arguments: &[&str], board: &mut dyn Board) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            (*extra).to_string(),
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The root's certificate \
                 rests at one path, which this subject composes itself."
            ),
        ));
    }
    let path = crate::fleet_root::root_certificate_path();
    let read = board.read_file(&path).map_err(|e| refused(e, &path))?;
    let Some(bytes) = read.contents else {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("path", Value::str(&path)),
            ("present", Value::Bool(false)),
            (
                "means",
                Value::str(
                    "this machine holds no fleet root: it is not the server of a fleet-root mode \
                     fleet, or its first boot has not minted one (ADR-063 §4)",
                ),
            ),
        ];
        debug_assert!(super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)));
        return Ok(Value::object(fields));
    };
    let unreadable = |why: &str| {
        OperationError::about(
            ErrorCode::MachineRefused,
            path.clone(),
            format!("{path} rests but {why}, so no certificate was answered"),
        )
    };
    let text = core::str::from_utf8(&bytes).map_err(|_| unreadable("is not text"))?;
    let der = der_of_pem(text).ok_or_else(|| unreadable("is not one PEM certificate"))?;
    let digest = board.sha256(&der).map_err(|e| refused(e, &path))?;
    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("path", Value::str(&path)),
        ("present", Value::Bool(true)),
        ("certificate", Value::str(text)),
        ("sha256", Value::str(crate::package::key_material::to_hex(&digest))),
        (
            "means",
            Value::str(
                "this fleet's root, minted on this machine and name-constrained to the fleet's \
                 names. Install it on a device to trust the fleet's HTTPS pages; the pin you \
                 reached this machine with is what vouches for it",
            ),
        ),
    ];
    debug_assert!(super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)));
    Ok(Value::object(fields))
}

/// The DER inside a PEM text holding exactly one certificate.
fn der_of_pem(text: &str) -> Option<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let after = text.split_once(BEGIN)?.1;
    let (body, tail) = after.split_once(END)?;
    if tail.contains(BEGIN) {
        return None;
    }
    let joined: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    crate::base64::decode(&joined).filter(|der| !der.is_empty())
}

#[cfg(test)]
#[path = "fleet_root_tests.rs"]
mod tests;
