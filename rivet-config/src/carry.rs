//! The `[carry]` table: **which published version of a carried product this
//! machine's card must carry**, when the fleet repository holds more than one.
//!
//! `ADR-050` §5 already decides *which* products a role's card carries — its
//! role table, read back by `crate::document::Document::carried_services` off
//! `[services.<name>] enabled`. What that leaves open is *which version*, the
//! moment the fleet repository a card is built from holds more than one
//! published `application` artifact for the same product name.
//! `cmd/rivet-image/src/fleet_payloads.rs::resolve` refuses rather than
//! guessing between them (`AGENTS.md` §5) — this table is where the machine
//! document, or the register it is composed from, states the answer instead
//! of being refused every time (`.rulings/` R-159 R20).
//!
//! # Why a table of its own, and not a fourth key on `[services.<name>]`
//!
//! `crate::services`' own header sets a three-clause test for what belongs in
//! that table, and its whole point is the supervisor's merge — `ADR-017`
//! §4's *"definitions ∪ desired state"* — which happens on an already
//! **running** machine. A version pin is read by a wholly different program at
//! a wholly different moment: `cmd/rivet-image --fleet-repository`, **before
//! the machine it will run on exists.** Mixing a build-time artifact-selection
//! key into the table the supervisor reads at runtime would make one table
//! answer to two authorities at two different times, which is the split
//! `ADR-017` §4 drew for exactly this reason, read a second way. So the pin
//! gets its own table, named for what it does rather than for where
//! `[services]` happens to sit.
//!
//! # One key, and it is required inside a table that appears
//!
//! **`[carry.<product>]` names a product, and `version` is the one thing it
//! may say about it.** A sub-table with no `version` would be a pin to
//! nothing, indistinguishable from not writing the table at all — `ADR-007`
//! §3's rule against accepting an input it would have to guess about, so
//! `version` is required exactly the way `[repos.<name>].id` is
//! (`crate::repos`).
//!
//! **The product name is checked against `ServiceName`**, the identical
//! grammar `[services.<name>]` uses — a pin names the same product a service
//! entry enables, so the two tables share one vocabulary for what a product
//! name is (`ADR-011`).
//!
//! # Absence is only allowed to mean one thing
//!
//! * **The whole `[carry]` table may be absent**, and means exactly one thing:
//!   every carried product resolves unpinned, so `resolve` succeeds only when
//!   the repository holds exactly one published version and refuses otherwise
//!   (`ADR-050` §5, R-159 R20).
//! * **A product named here that the role does not carry pins nothing** —
//!   `Document::carried_version` is consulted only for a name
//!   `Document::carried_services` already lists, so a stray pin for a product
//!   this machine does not enable is inert rather than an error. Refusing it
//!   would make writing a fleet-wide pin ahead of a role change a two-step
//!   operation for no reader's benefit.

use crate::services::ServiceName;

/// One product's version pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carry {
    product: ServiceName,
    version: String,
}

impl Carry {
    /// Every key a `[carry.<product>]` entry may declare, in schema order.
    ///
    /// **One key**, on [`crate::repos::Repo::DECLARED_KEYS`]'s footing exactly
    /// — see this module's header for why `version` is all a pin says.
    pub const DECLARED_KEYS: &'static [&'static str] = &["version"];

    /// Build a carry entry. Only the parser calls this today.
    pub(crate) const fn new(product: ServiceName, version: String) -> Self {
        Self { product, version }
    }

    /// Which product this pins.
    #[must_use]
    pub const fn product(&self) -> &ServiceName {
        &self.product
    }

    /// The version pinned, as the operator wrote it — the same string
    /// `cargo rivet fleet publish` recorded, and not otherwise validated:
    /// this crate performs no I/O and cannot ask a fleet repository whether a
    /// version it names has ever been published (`ADR-014` §3). That
    /// refusal is `cmd/rivet-image/src/fleet_payloads.rs::resolve`'s, at the
    /// moment a repository is actually in hand.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_table_declares_exactly_one_key() {
        assert_eq!(Carry::DECLARED_KEYS, ["version"]);
    }

    #[test]
    fn an_entry_carries_a_product_and_a_version() {
        let product = ServiceName::new("rivet-fleetd").expect("a valid fixture name");
        let carry = Carry::new(product.clone(), "0.1.0".to_owned());
        assert_eq!(carry.product(), &product);
        assert_eq!(carry.version(), "0.1.0");
    }
}
