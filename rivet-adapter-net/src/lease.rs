//! **What `udhcpc` hands back, and the one parser that reads it.**
//!
//! `ADR-004` §8's amendment dropped `udhcpc` as a stopgap on 2026-08-26,
//! because Stage 1 had ended and nothing was being unblocked by it.
//! **Reversed 2026-08-30**, on the maintainer's ruling, after the Rust
//! alternatives were surveyed and measured rather than assumed: the only
//! maintained crate, `dhcproto`, depends unconditionally on `hickory-proto` and
//! lands 53 crates including an ICU/IDNA stack — Unicode normalisation,
//! reaching a DHCP client — and it is a **codec**, so the state machine and the
//! T1/T2 timers get written either way. Everything else is abandoned
//! (`dhcparse`, last released 2022, 39 recent downloads), server-shaped, or a
//! stated learning project.
//!
//! The maintainer's position (`.rulings/` R-367) was that bringing in a C DHCP
//! solution was acceptable, and that reverse engineering DHCP was not wanted.
//!
//! # Why `udhcpc` and not `dhcpcd`
//!
//! **`udhcpc` configures nothing.** It speaks the protocol, execs a program
//! with the lease in its environment, and leaves every decision to that
//! program. So `cmd/rivet-netd` keeps every netlink write and `ADR-011`'s one
//! authority per fact survives intact. `dhcpcd` wants to own interface
//! configuration, which would have made two things able to set an address —
//! and an address that disagrees with itself is a machine nobody can find.
//!
//! # One parser, two sources, because they are the same bytes
//!
//! `udhcpc` delivers a lease as **environment variables** to the program it
//! execs. That program — `cmd/rivet-dhcp-hook` — writes them to a file, and
//! `rivet-netd` reads the file. Those are two readings of one vocabulary, so
//! there is one [`Lease::from_pairs`], and the file format is *deliberately the
//! same `key=value` shape the environment used*. Two parsers for one lease is
//! `ADR-011` §3's failure with an address in it.
//!
//! # It refuses rather than guessing
//!
//! `AGENTS.md` §5. A lease missing its address is not a lease with a default
//! address, and a netmask that is not a netmask is not 24. Each of those is a
//! machine that comes up looking reachable and is not — which on a headless
//! board is the most expensive failure available.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use rivet_core::net::{InterfaceName, Ipv4Cidr, StaticAddressing};

/// What `udhcpc` is saying happened.
///
/// The words are `udhcpc`'s own, passed as `argv[1]` to the program it execs.
/// [`Event::Other`] exists so that a version which grows a state does not make
/// the hook fail to parse its own arguments: an unknown event is reported and
/// ignored, which is what an unknown event deserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Bring the interface up with no address. Said before anything else, and
    /// again on giving up.
    Deconfig,
    /// A lease was obtained.
    Bound,
    /// The same lease, extended.
    Renew,
    /// The server refused. The address is gone and `udhcpc` starts over.
    Nak,
    /// No server answered.
    LeaseFail,
    /// Something this build does not know about.
    Other(String),
}

impl Event {
    /// Read `udhcpc`'s `argv[1]`.
    #[must_use]
    pub fn parse(word: &str) -> Self {
        match word {
            "deconfig" => Self::Deconfig,
            "bound" => Self::Bound,
            "renew" => Self::Renew,
            "nak" => Self::Nak,
            "leasefail" => Self::LeaseFail,
            other => Self::Other(other.to_string()),
        }
    }

    /// Whether a lease is expected to accompany this event.
    ///
    /// **Asked here rather than inferred from whether parsing succeeded.** A
    /// `deconfig` carrying no address is correct; a `bound` carrying no address
    /// is a fault. Those two must not produce the same answer, and they would
    /// if the only question asked were *did the environment parse*.
    #[must_use]
    pub fn carries_a_lease(&self) -> bool {
        matches!(self, Self::Bound | Self::Renew)
    }

    /// The word `udhcpc` used, for the journal.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Deconfig => "deconfig",
            Self::Bound => "bound",
            Self::Renew => "renew",
            Self::Nak => "nak",
            Self::LeaseFail => "leasefail",
            Self::Other(word) => word,
        }
    }
}

/// An address, and everything that arrived with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The interface it is for — `udhcpc`'s `interface`.
    pub interface: String,
    /// The address and its prefix as ONE value. `rivet_core::net::Ipv4Cidr`
    /// exists for this and says why: an address without its prefix is not an
    /// address.
    pub address: Ipv4Cidr,
    /// The default gateway. A server may offer several; the first is taken and
    /// the rest are dropped, because `ADR-023` has one default route.
    pub router: Option<Ipv4Addr>,
    /// Resolvers, in the order offered.
    pub nameservers: Vec<Ipv4Addr>,
    /// Which server answered. Kept for the journal, so a machine that got an
    /// address nobody expected can say who gave it one.
    pub server: Option<Ipv4Addr>,
    /// How long the lease is good for, in seconds.
    pub lease_seconds: u32,
    /// The search domain, when one was offered.
    pub domain: Option<String>,
}

/// Why a lease could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// A variable a lease cannot do without was absent or empty.
    Missing { key: &'static str },
    /// It was present and is not what it claims to be.
    Malformed {
        key: &'static str,
        value: String,
        expected: &'static str,
    },
}

impl core::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing { key } => write!(
                f,
                "udhcpc reported a lease with no `{key}`, which a lease cannot do without. \
                 NOTHING WAS CONFIGURED: an address completed from a partial lease is a machine \
                 that comes up looking reachable and is not"
            ),
            Self::Malformed {
                key,
                value,
                expected,
            } => write!(
                f,
                "udhcpc reported `{key}={value}`, which is not {expected}. NOTHING WAS CONFIGURED"
            ),
        }
    }
}

impl Lease {
    /// Read a lease from `udhcpc`'s vocabulary.
    ///
    /// **The lookup is a parameter rather than `std::env::var`**, which is what
    /// makes this a pure function a test can drive — and what lets the hook
    /// pass the environment while `rivet-netd` passes a file, with one
    /// implementation serving both.
    ///
    /// # Errors
    ///
    /// [`LeaseError`] when a required variable is absent or is not what it
    /// claims to be. It never substitutes a default for either.
    pub fn from_pairs(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, LeaseError> {
        let need = |key: &'static str| -> Result<String, LeaseError> {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or(LeaseError::Missing { key })
        };
        let address_of = |key: &'static str, text: &str| -> Result<Ipv4Addr, LeaseError> {
            text.parse().map_err(|_| LeaseError::Malformed {
                key,
                value: text.to_string(),
                expected: "an IPv4 address",
            })
        };

        let interface = need("interface")?;
        let ip_text = need("ip")?;
        let ip = address_of("ip", &ip_text)?;

        // **`mask` rather than `subnet`, and `udhcpc` sets both.** `subnet` is a
        // dotted netmask and `mask` is the prefix length. Taking the prefix
        // directly avoids converting a mask that may not be contiguous — which
        // a broken or hostile server can send, and converting it would invent a
        // prefix that was never offered.
        let mask_text = need("mask")?;
        let bad_mask = || LeaseError::Malformed {
            key: "mask",
            value: mask_text.clone(),
            expected: "a prefix length from 0 to 32",
        };
        let prefix_len: u8 = mask_text
            .parse::<u16>()
            .ok()
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(bad_mask)?;
        let address = Ipv4Cidr::new(ip, prefix_len).map_err(|_| bad_mask())?;

        // `router` and `dns` are space-separated lists. Only the first router is
        // taken; see the field.
        let router = match list_of("router", lookup).first() {
            Some(first) => Some(address_of("router", first)?),
            None => None,
        };
        let mut nameservers = Vec::new();
        for one in list_of("dns", lookup) {
            nameservers.push(address_of("dns", &one)?);
        }
        let server = match lookup("serverid").map(|v| v.trim().to_string()) {
            Some(text) if !text.is_empty() => Some(address_of("serverid", &text)?),
            _ => None,
        };

        let lease_text = need("lease")?;
        let lease_seconds: u32 = lease_text.parse().map_err(|_| LeaseError::Malformed {
            key: "lease",
            value: lease_text.clone(),
            expected: "a whole number of seconds",
        })?;

        Ok(Self {
            interface,
            address,
            router,
            nameservers,
            server,
            lease_seconds,
            domain: lookup("domain")
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
        })
    }

    /// The lease, as the thing `rivet-netd` already knows how to apply.
    ///
    /// # Why this is a conversion and not a second code path
    ///
    /// `ADR-004` §2.1: *"almost nothing in the path is link-specific"*. A
    /// lease is an interface, an address with a prefix, and a gateway — which
    /// is precisely [`StaticAddressing`], the value `rivet-netd` builds from a
    /// statically-configured document and hands to netlink. So DHCP does not
    /// get its own convergence; it produces a plan and joins the one that
    /// exists.
    ///
    /// **The name says "static" and the lease is not static**, which is worth a
    /// sentence rather than a rename: the type describes what is being ASKED
    /// of the kernel — this address, on this interface, with this route — and
    /// that request is identical however the address was learned. Where it came
    /// from is this module's business and stops here.
    ///
    /// # Errors
    ///
    /// When the interface name the server's reply carried is not one this
    /// system can use. `InterfaceName` refuses rather than truncating, and a
    /// truncated interface name applies an address to the wrong device.
    pub fn to_addressing(&self) -> Result<StaticAddressing, String> {
        let interface = InterfaceName::new(&self.interface).map_err(|why| {
            format!(
                "udhcpc reported a lease for `{}`, which is not a usable interface name: {why}",
                self.interface
            )
        })?;
        let plan = StaticAddressing {
            interface,
            address: self.address,
            gateway: self.router,
        };
        // **Checked here rather than trusted.** The plan came off the wire from
        // a server nobody on this machine controls, and `validate` is what
        // catches the combinations the kernel would refuse late or accept and
        // regret.
        plan.validate()
            .map_err(|why| format!("the lease does not describe a usable address: {why}"))?;
        Ok(plan)
    }

    /// Render the lease in the same vocabulary it was read from.
    ///
    /// **The file format IS the environment**, which is what lets one parser
    /// serve both sources. Keys are written in a fixed order, so two identical
    /// leases produce identical bytes and a file can be compared rather than
    /// re-parsed to be told nothing changed.
    #[must_use]
    pub fn to_pairs_text(&self) -> String {
        let mut out = String::new();
        let mut put = |key: &str, value: &str| {
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push('\n');
        };
        put("interface", &self.interface);
        put("ip", &self.address.address().to_string());
        put("mask", &self.address.prefix_len().to_string());
        if let Some(router) = self.router {
            put("router", &router.to_string());
        }
        if !self.nameservers.is_empty() {
            let each: Vec<String> = self.nameservers.iter().map(ToString::to_string).collect();
            put("dns", &each.join(" "));
        }
        if let Some(server) = self.server {
            put("serverid", &server.to_string());
        }
        put("lease", &self.lease_seconds.to_string());
        if let Some(domain) = &self.domain {
            put("domain", domain);
        }
        out
    }

    /// Read back what [`Lease::to_pairs_text`] wrote.
    ///
    /// # Errors
    ///
    /// The same [`LeaseError`]s [`Lease::from_pairs`] raises. A line without
    /// `=` is skipped rather than refused: this file is written by this build
    /// and read by it, and a stricter rule would turn a trailing newline into a
    /// machine with no address.
    pub fn parse_pairs_text(text: &str) -> Result<Self, LeaseError> {
        let mut pairs: BTreeMap<String, String> = BTreeMap::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                pairs.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        Self::from_pairs(&|key| pairs.get(key).cloned())
    }
}

/// Where a lease is left for `rivet-netd` to find.
///
/// **`/run` and not `/state`**: a lease is true only while this boot is, and
/// `rivet_core::mount` already makes `/run` a tmpfs. Writing it to `/state`
/// would leave a stale address on the card to be believed after a reboot, which
/// is the same class of failure as a stale `rivet.toml`.
///
/// Composed here and nowhere else, `ADR-011`: `cmd/rivet-dhcp-hook` writes it
/// and `cmd/rivet-netd` reads it, and those two disagreeing is a machine with
/// an address nothing knows about.
pub const LEASE_DIR: &str = "/run/rivet/dhcp";

/// The file one interface's lease lives in.
///
/// **Per interface, because there is one per interface.** `eth0` on a static
/// address while `wlan0` holds a lease is the configuration the maintainer has
/// asked for by name, and a single file would make the second writer erase the
/// first.
#[must_use]
pub fn lease_path(interface: &str) -> String {
    format!("{LEASE_DIR}/{interface}.lease")
}

/// A space-separated list from one variable, empties dropped.
fn list_of(key: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    lookup(key)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;
