//! **The DHCP leases `rivet-netd` applies, so the supervisor can tell when one
//! has changed** — `ADR-004` §8.
//!
//! A DHCP interface's client is a supervised service, and `cmd/rivet-dhcp-hook`
//! records each lease it takes, and each renewal, under
//! `rivet_adapter_net::lease::lease_path`, removes it when the lease is
//! withdrawn, and nudges this process. The nudge is a reload, and the reload
//! reads the leases here and hands them to `Supervisor::follow_leases`, which
//! runs `rivet-netd` again when one is new, different or gone. `rivet-netd` is
//! the one program that writes an interface's address, and it reads each lease
//! once, at its start.
//!
//! The comparison is of the recorded text, whole: a renewal that changes the
//! router or the mask, not only the address, is one `rivet-netd` must apply.

/// Each DHCP interface the document names, in document order, and the lease
/// text recorded for it, or `None` when there is none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Leases(pub Vec<(String, Option<String>)>);

impl Leases {
    /// Read the lease of every interface the document addresses by DHCP.
    pub fn read(k: &mut dyn crate::kernel::Kernel, document: &rivet_config::Document) -> Self {
        let mut out = Vec::new();
        for interface in document.network() {
            if !matches!(interface.addressing(), rivet_config::Addressing::Dhcp) {
                continue;
            }
            let name = interface.name().to_string();
            let text = k.read_file(&rivet_adapter_net::lease::lease_path(&name)).ok();
            out.push((name, text));
        }
        Self(out)
    }

    /// **The interfaces whose lease is new, different or gone since `was`**,
    /// which are the ones whose address `rivet-netd` must change. A lease that
    /// went away is among them: the interface carries exactly its lease's
    /// address, so one with no lease is left with none. An interface that had
    /// no lease before and has none now is not.
    #[must_use]
    pub fn changed_since(&self, was: &Self) -> Vec<String> {
        self.0
            .iter()
            .filter(|(name, text)| {
                let previous = was
                    .0
                    .iter()
                    .find(|(before, _)| before == name)
                    .and_then(|(_, previous)| previous.as_ref());
                previous != text.as_ref()
            })
            .map(|(name, _)| name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn leases(entries: &[(&str, Option<&str>)]) -> Leases {
        Leases(
            entries
                .iter()
                .map(|(n, t)| ((*n).to_string(), t.map(str::to_string)))
                .collect(),
        )
    }

    #[test]
    fn a_first_lease_is_a_change() {
        let was = leases(&[("eth0", None)]);
        let now = leases(&[("eth0", Some("ip=10.0.0.2\n"))]);
        assert_eq!(now.changed_since(&was), ["eth0"]);
    }

    /// **A renewal that changes anything is followed**, the router as much as
    /// the address.
    #[test]
    fn a_renewal_that_changes_the_lease_is_a_change() {
        let was = leases(&[("eth0", Some("ip=10.0.0.2\nrouter=10.0.0.1\n"))]);
        let now = leases(&[("eth0", Some("ip=10.0.0.2\nrouter=10.0.0.254\n"))]);
        assert_eq!(now.changed_since(&was), ["eth0"]);
    }

    /// **A renewal that rewrote the same lease is not**, which is the ordinary
    /// renewal and must not run `rivet-netd` every hour for nothing.
    #[test]
    fn the_same_lease_written_again_is_not_a_change() {
        let was = leases(&[("eth0", Some("ip=10.0.0.2\n"))]);
        assert!(was.changed_since(&was.clone()).is_empty());
    }

    /// **A lease that went away is a change**: `nak` or `leasefail` removed
    /// it, and the address it put on the interface must come off.
    #[test]
    fn a_lease_that_went_away_is_a_change() {
        let was = leases(&[("eth0", Some("ip=10.0.0.2\n"))]);
        let now = leases(&[("eth0", None)]);
        assert_eq!(now.changed_since(&was), ["eth0"]);
    }

    /// No lease before and none now is nothing to do, which is every
    /// `deconfig` before the first lease.
    #[test]
    fn no_lease_before_and_none_now_is_not_a_change() {
        let was = leases(&[("eth0", None)]);
        assert!(was.changed_since(&was.clone()).is_empty());
        let first = leases(&[]);
        assert!(was.changed_since(&first).is_empty());
    }

    /// Only the interface that changed is named, on a machine with two.
    #[test]
    fn only_the_interface_that_changed_is_named() {
        let was = leases(&[("eth0", Some("ip=10.0.0.2\n")), ("wlan0", None)]);
        let now = leases(&[("eth0", Some("ip=10.0.0.2\n")), ("wlan0", Some("ip=10.0.1.2\n"))]);
        assert_eq!(now.changed_since(&was), ["wlan0"]);
    }
}
