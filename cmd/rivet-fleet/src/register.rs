//! **Moved 2026-09-01** — the register is `rivetos-register` now.
//!
//! `ADR-034` §2.2's Remote mode, plus the maintainer's ruling that Local mode
//! is dropped, put the register on a fleet server rather than on the
//! workstation. `cmd/rivet-fleetd` reads it and this crate is not its owner any
//! more, so the code lives in a library both can link — the same move
//! `rivet_core::identifier` made out of this crate on 2026-08-26, for the same
//! reason.
//!
//! This re-export exists so that every path in this crate keeps reading; it is
//! not a second definition.

pub use rivetos_register::*;
