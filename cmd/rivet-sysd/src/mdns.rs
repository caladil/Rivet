//! **What `rivet-mdnsd` announces from, so the supervisor can tell when that
//! has changed** — `ADR-017` §3.
//!
//! The responder reads its inputs at its start and then answers: the
//! machine's name from the running kernel, and its one address from the
//! projection [`Inputs::projection`] renders — a static address as the
//! document wrote it, or for a DHCP interface the address in that interface's
//! lease. The supervisor makes that reading once, here, and both compares it
//! and hands it over; the responder runs confined and could read neither the
//! document nor a lease itself.
//!
//! **The supervisor restarts the responder when they change**, because nothing
//! else will: a machine renamed by a reload, or given a different address by a
//! DHCP renewal, would otherwise go on announcing the old answer until it
//! rebooted. Both reach the supervisor as a reload — `apply reload` for a
//! document change, and `rivet-dhcp-hook`'s nudge for every lease it writes —
//! so the reload is where the comparison is made. How `rivet-netd` applies a
//! renewed lease to the interface is a separate matter and is not touched
//! here.
//!
//! # A lease arriving is not always a change worth a restart
//!
//! A responder that started before its interface's first lease is still
//! running, waiting for that lease, and reads it from its projection once the
//! reload the lease's nudge causes has rewritten that. So when
//! the inputs at its start named no address at all, a lease arriving is left
//! to it. Everything else that changes — the name, or an address it was
//! already announcing or could have been — restarts it. A responder that gave
//! up and is `Failed` or backing off is started again on any change, because
//! the reason it gave up may be the thing that changed.

use std::net::Ipv4Addr;

/// The responder's inputs, as it would read them now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    /// The machine's name, as `crate::naming` gave it to the kernel.
    pub hostname: Option<String>,
    /// Every interface with an address, and that address, in document order.
    pub addressed: Vec<(String, Ipv4Addr)>,
    /// Every DHCP interface with no lease yet.
    pub leaseless: Vec<String>,
}

impl Inputs {
    /// Read them: `hostname` as `crate::naming` settled it, the document's
    /// interfaces, and each DHCP interface's lease under
    /// `crate::etc::LEASE_DIR`.
    pub fn read(
        k: &mut dyn crate::kernel::Kernel,
        document: &rivet_config::Document,
        hostname: Option<&str>,
    ) -> Self {
        let mut addressed = Vec::new();
        let mut leaseless = Vec::new();
        for interface in document.network() {
            let name = interface.name().to_string();
            match interface.static_plan() {
                Some(plan) => addressed.push((name, plan.address.address())),
                None => match leased(k, &name) {
                    Some(address) => addressed.push((name, address)),
                    None => leaseless.push(name),
                },
            }
        }
        Self {
            hostname: hostname.map(str::to_string),
            addressed,
            leaseless,
        }
    }

    /// **What the responder is handed**: these inputs as
    /// `rivet_core::mdns`'s projection, which
    /// `crate::runtime::deliver_mdns_projection` writes into its home. The
    /// responder runs confined and may read neither the document nor the
    /// leases, so this is the whole of what it announces from — the same
    /// reading the supervisor compares, never a second one.
    ///
    /// A name that is not a hostname projects as no name, which the responder
    /// refuses; `crate::naming` only ever settles a valid one.
    #[must_use]
    pub fn projection(&self) -> rivet_core::mdns::Projection {
        rivet_core::mdns::Projection {
            hostname: self.hostname.as_deref().and_then(rivet_core::hostname::Hostname::new),
            addressed: self.addressed.clone(),
            leaseless: self.leaseless.clone(),
        }
    }

    /// **Whether a responder still running on `self` must be restarted to
    /// announce `now`** — this module's header.
    #[must_use]
    pub fn restart_needed_for(&self, now: &Self) -> bool {
        self != now && (self.hostname != now.hostname || !self.addressed.is_empty())
    }
}

/// The address in `interface`'s lease, as `rivet-dhcp-hook` wrote it, or
/// nothing. A lease that will not read is no lease, as the responder treats it.
fn leased(k: &mut dyn crate::kernel::Kernel, interface: &str) -> Option<Ipv4Addr> {
    let text = k
        .read_file(&format!("{}/{interface}.lease", crate::etc::LEASE_DIR))
        .ok()?;
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "ip")
        .and_then(|(_, value)| value.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn inputs(hostname: &str, addressed: &[(&str, [u8; 4])], leaseless: &[&str]) -> Inputs {
        Inputs {
            hostname: Some(hostname.to_string()),
            addressed: addressed
                .iter()
                .map(|(n, a)| ((*n).to_string(), Ipv4Addr::from(*a)))
                .collect(),
            leaseless: leaseless.iter().map(|n| (*n).to_string()).collect(),
        }
    }

    #[test]
    fn a_new_name_restarts_the_responder() {
        let was = inputs("kitchen", &[("eth0", [10, 0, 0, 2])], &[]);
        let now = inputs("larder", &[("eth0", [10, 0, 0, 2])], &[]);
        assert!(was.restart_needed_for(&now));
    }

    #[test]
    fn a_new_address_restarts_the_responder() {
        let was = inputs("kitchen", &[("eth0", [10, 0, 0, 2])], &[]);
        let now = inputs("kitchen", &[("eth0", [10, 0, 0, 3])], &[]);
        assert!(was.restart_needed_for(&now));
    }

    /// **A first lease is the responder's own to read**: it started with no
    /// address, is waiting for exactly this, and announces it unprompted.
    #[test]
    fn a_first_lease_while_the_responder_waits_for_it_restarts_nothing() {
        let was = inputs("kitchen", &[], &["eth0"]);
        let now = inputs("kitchen", &[("eth0", [10, 0, 0, 2])], &[]);
        assert!(!was.restart_needed_for(&now));
    }

    #[test]
    fn nothing_changed_restarts_nothing() {
        let was = inputs("kitchen", &[("eth0", [10, 0, 0, 2])], &[]);
        assert!(!was.restart_needed_for(&was.clone()));
    }
}
