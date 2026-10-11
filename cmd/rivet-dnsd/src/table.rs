//! **What the zone holds**, in Rivet's own types — the document's `[dns]`
//! table turned into fully qualified names and the TTLs each is served with.
//!
//! Pure, so every rule is tested without a socket; `crate::serve` turns a
//! [`Zone`] into the dependency's types and nothing here names one.

use core::net::Ipv4Addr;

use rivet_config::dns::{Dns, Label};

/// The upstream every name outside the zone is forwarded to: Quad9, by
/// address (`ADR-047` §1). By address because a resolver that had to resolve
/// its own upstream would depend on itself.
pub const UPSTREAM: &[Ipv4Addr] = &[Ipv4Addr::new(9, 9, 9, 9), Ipv4Addr::new(149, 112, 112, 112)];

/// One A-record, fully qualified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    /// `<label>.<zone>.`, with the trailing dot.
    pub name: String,
    pub address: Ipv4Addr,
    pub ttl: u32,
}

/// The zone as served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// `<zone>.`, with the trailing dot.
    pub origin: String,
    /// The name the SOA and NS records point at: this machine's own name under
    /// the zone. There is no substitute; see [`zone`].
    pub server: String,
    /// `hostmaster.<zone>.` — the SOA's mailbox field, which nothing reads.
    pub mailbox: String,
    /// The positive TTL, for the NS record and any record without its own.
    pub ttl: u32,
    /// The negative TTL. It is both the SOA record's own TTL and its `minimum`
    /// field, because a resolver caches an absence for the lower of the two
    /// (RFC 2308 §5) and both must say what `ADR-047` §8 set.
    pub negative_ttl: u32,
    pub records: Vec<Address>,
}

/// Why there is no zone to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoServerName(pub String);

impl core::fmt::Display for NoServerName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Build the zone from the document's table and this machine's name.
///
/// `hostname` is the name the machine carries, which the SOA and NS records
/// name under the zone. It is compared case-insensitively, as DNS compares
/// every name (RFC 4343), so it is folded to lowercase before it becomes a
/// label.
///
/// # Errors
///
/// **No name, or one that is not a single label, is refused.** A substitute
/// would rename the zone's SOA and NS records with nothing saying so, so the
/// service refuses to start and names the reason instead.
pub fn zone(dns: &Dns, hostname: Option<&str>) -> Result<Zone, NoServerName> {
    let origin = format!("{}.", dns.zone());
    let ttl = dns.effective_ttl().seconds();
    let Some(hostname) = hostname else {
        return Err(NoServerName(
            "the document handed to this service names no hostname, so the zone's SOA and NS \
             records would name no server. The supervisor writes the machine's name into it; a \
             machine it could not name has none to write (set `hostname` in the document)"
                .to_owned(),
        ));
    };
    let Some(server_label) = Label::new(&hostname.to_ascii_lowercase()) else {
        return Err(NoServerName(format!(
            "the hostname `{hostname}` is not a single DNS label ({}), so the zone's SOA and NS \
             records cannot name it",
            Label::RULE
        )));
    };
    Ok(Zone {
        server: format!("{server_label}.{origin}"),
        mailbox: format!("hostmaster.{origin}"),
        ttl,
        negative_ttl: dns.effective_negative_ttl().seconds(),
        records: dns
            .records()
            .iter()
            .map(|r| Address {
                name: format!("{}.{origin}", r.label()),
                address: r.address(),
                ttl: r.ttl().map_or(ttl, |t| t.seconds()),
            })
            .collect(),
        origin,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn dns(src: &str) -> Dns {
        let doc = rivet_config::Document::parse(&format!("version = 1\n{src}")).unwrap();
        doc.dns().cloned().unwrap()
    }

    #[test]
    fn every_record_is_placed_under_the_zone_with_a_trailing_dot() {
        let z = zone(
            &dns("[dns]\nzone = \"home-fleet.home.arpa\"\n[dns.records.dev-01]\naddress = \"192.168.50.66\"\n"),
            Some("dev-01"),
        )
        .unwrap();
        assert_eq!(z.origin, "home-fleet.home.arpa.");
        assert_eq!(
            z.records,
            [Address {
                name: "dev-01.home-fleet.home.arpa.".to_owned(),
                address: Ipv4Addr::new(192, 168, 50, 66),
                ttl: 300,
            }]
        );
        assert_eq!(z.server, "dev-01.home-fleet.home.arpa.");
        assert_eq!(z.mailbox, "hostmaster.home-fleet.home.arpa.");
    }

    /// `ADR-047` §8's defaults when the table says nothing, and a record's own
    /// TTL beating the table's when it says something.
    #[test]
    fn the_ttls_are_the_tables_and_a_record_may_override_its_own() {
        let silent =
            zone(&dns("[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\n"), Some("m")).unwrap();
        assert_eq!((silent.ttl, silent.negative_ttl), (300, 60));
        assert_eq!(silent.records[0].ttl, 300);

        let set = zone(
            &dns(
                "[dns]\nzone = \"f.home.arpa\"\nttl = 120\nnegative-ttl = 10\n\
                 [dns.records.a]\naddress = \"10.0.0.1\"\n\
                 [dns.records.b]\naddress = \"10.0.0.2\"\nttl = 5\n",
            ),
            Some("m"),
        )
        .unwrap();
        assert_eq!((set.ttl, set.negative_ttl), (120, 10));
        let ttls: Vec<(&str, u32)> = set.records.iter().map(|r| (r.name.as_str(), r.ttl)).collect();
        assert_eq!(ttls, [("a.f.home.arpa.", 120), ("b.f.home.arpa.", 5)]);
    }

    /// **A missing or unusable name is a refusal, never a substitute** —
    /// `ADR-047` §8. A server named `ns.<zone>` in place of the machine's own
    /// name would rename the zone's SOA and NS records with nothing saying so.
    #[test]
    fn a_machine_without_a_name_is_refused_rather_than_renamed() {
        for hostname in [None, Some(""), Some("Not A Label"), Some("a.b")] {
            let refused = zone(&dns("[dns]\nzone = \"f.home.arpa\"\n"), hostname);
            assert!(refused.is_err(), "{hostname:?} was served as {refused:?}");
        }
        let none = zone(&dns("[dns]\nzone = \"f.home.arpa\"\n"), None).unwrap_err();
        assert!(none.0.contains("names no hostname"), "{none}");
    }

    /// A name that differs from a label only by case is the same DNS name
    /// (RFC 4343), so it is served under its lowercase spelling.
    #[test]
    fn a_hostname_in_mixed_case_names_the_server_in_lowercase() {
        let z = zone(&dns("[dns]\nzone = \"f.home.arpa\"\n"), Some("Kitchen-Pi")).unwrap();
        assert_eq!(z.server, "kitchen-pi.f.home.arpa.");
    }

    #[test]
    fn the_upstream_is_quad9_by_address() {
        assert_eq!(UPSTREAM, [Ipv4Addr::new(9, 9, 9, 9), Ipv4Addr::new(149, 112, 112, 112)]);
    }
}
