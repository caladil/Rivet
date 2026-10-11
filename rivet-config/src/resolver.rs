//! `[network.resolver]` — **the nameservers this machine asks, when it is told
//! them rather than leased them.**
//!
//! The maintainer's ruling, 2026-09-01 (`.rulings/` R-358): the resolver comes
//! from DHCP or is set statically. This module is the static half.
//!
//! # Why it is inside `[network]` and not a top-level `[resolver]`
//!
//! Ruled the same day, and the trade was stated before it was taken. `[network]`
//! is otherwise *strictly* one sub-table per interface name — `read_network`'s
//! own `EXPECTED` said so — and this makes it "interface names, plus one
//! reserved word". The cost is real and small: an interface literally named
//! `resolver` becomes unconfigurable. Rivet never renames interfaces and the
//! kernel hands out `eth0`, `wlan0`, `enp3s0`, so the collision is theoretical.
//!
//! What it buys is that networking is in one place, which is where an operator
//! looks when DNS is broken. `netplan` made the same choice — `version` and
//! `renderer` sit beside `ethernets` — so the shape is not novel.
//!
//! # This is DESIRED state, and `dns` on an interface is not
//!
//! `crate::network` has a test asserting that `dns` is **not** a declared
//! interface key, because a nameserver an interface *learned from a lease* is
//! observed state and `ADR-007` §3.3 keeps those out of the document. That rule
//! is untouched. What is written here is the opposite: the operator saying what
//! the resolver *shall* be, which is exactly what a document is for.
//!
//! The two meet at `/etc/resolv.conf`, which has one authority (`ADR-011`):
//! **what the document says wins; a DHCP lease fills it only when this table is
//! silent.**
//!
//! # Why the file has to exist at all
//!
//! It is not Rivet's choice. musl compiles nine `/etc` paths in as string
//! literals (`ADR-008` §5), `/etc/resolv.conf` is one of them, and there is no
//! resolver configuration API to use instead. `/etc` on a Rivet is a generated
//! tmpfs — a projection of this document, rebuilt every boot and never storage
//! (`ADR-007` §3.5) — so this table is the input and the file is the artefact.
//!
//! **The image ships no `/etc/resolv.conf`** and that stays true: a stale copy
//! baked into a read-only filesystem would answer confidently and wrongly, and
//! could not be corrected on the running machine. Absent, musl falls back to
//! `127.0.0.1`, where nothing listens, which fails cleanly and fast
//! (`rivet_core::etc::MUSL_FALLBACK_NAMESERVER`).

use core::net::Ipv4Addr;

/// `[network.resolver]` — the nameservers, in the order they will be asked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Resolver {
    nameservers: Vec<Ipv4Addr>,
    search: Vec<SearchDomain>,
}

impl Resolver {
    /// The reserved sub-table name inside `[network]`.
    ///
    /// **Named here rather than spelled in the parser**, because
    /// `crate::parse` must exclude exactly this word from interface names and
    /// two spellings of it would be a table that parses as an interface.
    pub const TABLE: &'static str = "resolver";

    /// Every key this table accepts. An unknown one is refused by name, on
    /// `crate::splash`'s rule: a misspelled setting that parses is a setting
    /// that silently does nothing.
    pub const DECLARED_KEYS: &'static [&'static str] = &["nameservers", "search"];

    /// What the value has to be, for a refusal that says so.
    pub const RULE: &'static str =
        "a list of IPv4 addresses, in the order they will be asked; at most three, \
         because musl reads no more";

    /// **musl asks at most three, and silently ignores the rest.**
    ///
    /// `resolv.conf(5)` caps `nameserver` lines at `MAXNS`, which is 3 in musl
    /// as in glibc. A fourth would be written and never used, so it is refused
    /// instead — `ADR-007` §3.6 forbids accepting an input the system discards.
    pub const MAX: usize = 3;

    /// **The search list musl will append to an unqualified name.**
    ///
    /// `resolv.conf(5)`'s `search`. `domain` is the older spelling of the same
    /// idea holding exactly one entry, and the two are mutually exclusive with
    /// last-one-wins — a rule nobody should have to know, so only `search` is
    /// accepted and one entry is the ordinary case.
    ///
    /// **musl bounds this by BYTES, not by count**: its resolver reads the
    /// search list into a 256-byte buffer and stops. So the limit here is the
    /// rendered length rather than a number of domains, which is why
    /// [`Resolver::SEARCH_BYTES`] is a byte count.
    pub const SEARCH_BYTES: usize = 256;

    /// Build one.
    #[must_use]
    pub fn new(nameservers: Vec<Ipv4Addr>, search: Vec<SearchDomain>) -> Self {
        Self { nameservers, search }
    }

    /// The search domains, in the order they will be tried.
    #[must_use]
    pub fn search(&self) -> &[SearchDomain] {
        &self.search
    }

    /// The nameservers, in document order. Order is preserved because it is
    /// meaningful: musl asks them in the order the file lists them.
    #[must_use]
    pub fn nameservers(&self) -> &[Ipv4Addr] {
        &self.nameservers
    }

    /// **Whether this table says anything at all.**
    ///
    /// `[network.resolver]` with no keys states nothing, and `crate::serialise`
    /// must not write one back — two documents meaning the same thing would
    /// otherwise produce different bytes (`ADR-007` §3.1). It is also the
    /// signal that a DHCP lease may supply the answer instead.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.nameservers.is_empty() && self.search.is_empty()
    }
}

/// One entry of `search` — a domain appended to an unqualified name.
///
/// # Validated here, because the file it lands in has no escaping
///
/// `/etc/resolv.conf` is whitespace-separated with no quoting, so a value
/// carrying a space or a newline would not be a bad domain — it would be a
/// *different directive*. `search evil\nnameserver 6.6.6.6` is one string in a
/// document and two lines in the artefact, and the second is a nameserver
/// nobody wrote. The document is signed and operator-authored so this is not a
/// hostile-input path, but `ADR-011` is that a value is validated where it is
/// parsed, and a rule that holds only while nobody makes a mistake is not a
/// rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SearchDomain(String);

impl SearchDomain {
    /// What a domain has to be, for a refusal that says so.
    pub const RULE: &'static str = "a domain name: dot-separated labels of a-z, 0-9 and \
                                    hyphen, each 1-63 bytes and not starting or ending with \
                                    a hyphen, 253 bytes in total";

    /// RFC 1035, section 2.3.4's ceiling on a presentation-format name.
    pub const MAX_BYTES: usize = 253;

    /// The same section's ceiling on one label.
    pub const MAX_LABEL: usize = 63;

    /// Parse one, or say why not.
    ///
    /// **Lowercase only, and not lowercased for you.** DNS is
    /// case-insensitive, so accepting `LAN` and writing `lan` would make a
    /// document that does not round-trip to its own bytes — which `ADR-007`
    /// §3.1's canonical serialisation forbids. Refusing it sends the operator
    /// to fix the one character.
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > Self::MAX_BYTES {
            return None;
        }
        // A trailing dot is the root and is legal in a name; it is not legal
        // here, because it would make `lan` and `lan.` two spellings of one
        // search list.
        for label in text.split('.') {
            if label.is_empty() || label.len() > Self::MAX_LABEL {
                return None;
            }
            if label.starts_with('-') || label.ends_with('-') {
                return None;
            }
            if !label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return None;
            }
        }
        Some(Self(text.to_owned()))
    }

    /// The domain, as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for SearchDomain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_order_written_is_the_order_asked() {
        let r = Resolver::new(
            vec![Ipv4Addr::new(192, 168, 50, 1), Ipv4Addr::new(1, 1, 1, 1)],
            Vec::new(),
        );
        assert_eq!(
            r.nameservers(),
            [Ipv4Addr::new(192, 168, 50, 1), Ipv4Addr::new(1, 1, 1, 1)]
        );
    }

    /// An empty table states nothing, and that is what lets a DHCP lease answer
    /// instead of contending with it.
    #[test]
    fn a_table_with_no_nameservers_is_silent() {
        assert!(Resolver::default().is_silent());
        assert!(!Resolver::new(vec![Ipv4Addr::LOCALHOST], Vec::new()).is_silent());
        // A search list with no nameservers still says something: it changes
        // what an unqualified name resolves to through whatever the lease
        // supplies.
        let only_search = Resolver::new(
            Vec::new(),
            vec![SearchDomain::new("lan").unwrap_or_else(|| unreachable!("a domain"))],
        );
        assert!(!only_search.is_silent());
    }

    /// The reserved word is a constant so the parser and this module cannot
    /// disagree about which sub-table is not an interface.
    #[test]
    fn the_reserved_name_is_a_valid_interface_name_which_is_why_it_is_reserved() {
        assert_eq!(Resolver::TABLE, "resolver");
        // If the kernel would refuse this as an interface name there would be
        // no collision and no need to reserve it. It would not, so there is.
        assert!(
            rivet_core::net::InterfaceName::new(Resolver::TABLE).is_ok(),
            "`resolver` stopped being a name an interface could have; the \
             exception in read_network is then unnecessary"
        );
    }

    /// The reason the type exists: `resolv.conf` has no quoting, so a value
    /// with whitespace in it is not a bad domain, it is a second directive.
    #[test]
    fn a_domain_may_not_smuggle_a_second_directive() {
        for bad in [
            "evil\nnameserver 6.6.6.6",
            "two words",
            "trailing.",
            ".leading",
            "-hyphen.start",
            "hyphen.end-",
            "UPPER",
            "",
        ] {
            assert!(
                SearchDomain::new(bad).is_none(),
                "{bad:?} was accepted as a search domain"
            );
        }
        assert!(SearchDomain::new("lan").is_some());
        assert!(SearchDomain::new("home.arpa").is_some());
        assert!(SearchDomain::new("my-net.example.com").is_some());
    }
}
