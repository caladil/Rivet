//! Reading the desired-state document off this host — `ADR-035` phase 4.
//!
//! **The whole adapter, and it is `read_to_string` over one absolute path.**
//! `crate::journal`'s note carries the argument for why that belongs here
//! rather than behind `BootMedium`: it needs no `mount(2)`, no device tree and
//! no pid 1, so a build that refuses every other machine operation can still
//! answer this one honestly.

use rivet_core::operation::machine::MachineError;
use rivet_core::state::MACHINE_DOCUMENT;

/// The document, or `None` when there is not one.
///
/// # Errors
///
/// When the file is there and cannot be read. **`NotFound` is not that**: a
/// first boot legitimately has no `/state/config` document, and reporting an
/// ordinary state as a fault is the thing this distinction exists to stop.
pub fn read() -> Result<Option<String>, MachineError> {
    match std::fs::read_to_string(MACHINE_DOCUMENT) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(MachineError::Refused(format!(
            "{MACHINE_DOCUMENT} is there and could not be read: {e}"
        ))),
    }
}
