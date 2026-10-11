//! `[dns]` — **the names this machine answers for, when it runs the name
//! service.** `ADR-047` §8.
//!
//! ```toml
//! [dns]
//! zone = "my-fleet.home.arpa"
//! ttl = 300
//! negative-ttl = 60
//!
//! [dns.records.arbor]
//! address = "192.168.50.10"
//!
//! [dns.records.printer]
//! address = "192.168.50.40"
//! ttl = 60
//! ```
//!
//! # The table is read only by the name service
//!
//! Nothing else on a machine reads this table. It is the desired state of
//! `cmd/rivet-dnsd`, which is an image service shipped disabled: a document can
//! carry `[dns]` on a machine where the service is off, and the table then
//! states a zone nobody serves. That is the same footing as `[kiosk]` naming an
//! application that is not installed (`ADR-007` §3: a document is authored
//! anywhere, and describing a service before it runs is the normal case).
//!
//! # `zone` is required, and it is under `home.arpa`
//!
//! A record is a label, and a label means nothing until it is placed under a
//! zone, so a `[dns]` table without `zone` has stated no name at all and is
//! refused. The zone is `home.arpa` or a name beneath it: RFC 8375 reserves
//! that suffix for exactly this use, so no public resolver answers for it and a
//! zone here cannot shadow a public name. A zone outside it would make every
//! client that uses this resolver answer `NXDOMAIN` for the real names in that
//! part of the tree, which is a failure invisible from the machine that causes
//! it. Whether the fleet's root certificate permits the zone is a question for
//! the authority that issues certificates (`ca::permitted_suffixes` in
//! `cmd/rivet-provision`), not for this crate, which reads no certificate.
//!
//! # The two TTLs have defaults, and the defaults are the ruling
//!
//! `ttl` is 300 seconds and `negative-ttl` is 60 (`ADR-047` §8). A
//! `[dns]` table that omits either is served with the default; the document
//! keeps the omission, so a later change to a default reaches it. Both are
//! bounded to a day: `ADR-047` §8's asymmetry is that a record too long in a
//! client's cache leaves the network wrong with no way to force it, and a day
//! is already far past any use a hand-edited table has for a longer one.

use core::net::Ipv4Addr;

/// `[dns]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dns {
    zone: Zone,
    ttl: Option<Ttl>,
    negative_ttl: Option<Ttl>,
    records: Vec<Record>,
}

impl Dns {
    /// Every key the table accepts, in the order the schema lists them.
    pub const DECLARED_KEYS: &'static [&'static str] = &["zone", "ttl", "negative-ttl", "records"];

    /// `ADR-047` §8's positive TTL: five minutes.
    pub const DEFAULT_TTL: Ttl = Ttl(300);

    /// `ADR-047` §8's negative TTL: one minute, because adding a name is the
    /// common operation and a client that asked before it existed caches the
    /// absence for this long.
    pub const DEFAULT_NEGATIVE_TTL: Ttl = Ttl(60);

    /// Build one. Records are sorted by label, so a document read and written
    /// back is byte-identical however the text ordered them (`ADR-007` §3).
    #[must_use]
    pub fn new(zone: Zone, ttl: Option<Ttl>, negative_ttl: Option<Ttl>, mut records: Vec<Record>) -> Self {
        records.sort_by(|a, b| a.label.cmp(&b.label));
        Self {
            zone,
            ttl,
            negative_ttl,
            records,
        }
    }

    /// The zone every record is placed under.
    #[must_use]
    pub const fn zone(&self) -> &Zone {
        &self.zone
    }

    /// `ttl` as written, or `None` when the document omitted it.
    #[must_use]
    pub const fn ttl(&self) -> Option<Ttl> {
        self.ttl
    }

    /// `negative-ttl` as written, or `None` when the document omitted it.
    #[must_use]
    pub const fn negative_ttl(&self) -> Option<Ttl> {
        self.negative_ttl
    }

    /// The positive TTL the service serves: the document's, or
    /// [`Self::DEFAULT_TTL`]. **The one place that default is applied.**
    #[must_use]
    pub fn effective_ttl(&self) -> Ttl {
        self.ttl.unwrap_or(Self::DEFAULT_TTL)
    }

    /// The negative TTL the service serves: the document's, or
    /// [`Self::DEFAULT_NEGATIVE_TTL`].
    #[must_use]
    pub fn effective_negative_ttl(&self) -> Ttl {
        self.negative_ttl.unwrap_or(Self::DEFAULT_NEGATIVE_TTL)
    }

    /// Every record, ordered by label.
    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }
}

/// `[dns.records.<label>]` — one name and its address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    label: Label,
    address: Ipv4Addr,
    ttl: Option<Ttl>,
}

impl Record {
    /// Every key one record accepts.
    pub const DECLARED_KEYS: &'static [&'static str] = &["address", "ttl"];

    /// What `address` has to be, for a refusal that says so.
    pub const ADDRESS_RULE: &'static str = "an IPv4 address in dotted-quad form, such as \
                                            \"192.168.50.10\"";

    /// Build one.
    #[must_use]
    pub const fn new(label: Label, address: Ipv4Addr, ttl: Option<Ttl>) -> Self {
        Self { label, address, ttl }
    }

    /// The label, which is the table's own key.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// The address the name answers with.
    #[must_use]
    pub const fn address(&self) -> Ipv4Addr {
        self.address
    }

    /// This record's own TTL, if the document set one. `ADR-047` §8: the
    /// override exists to lower one record's TTL the day before it moves.
    #[must_use]
    pub const fn ttl(&self) -> Option<Ttl> {
        self.ttl
    }
}

/// A TTL in seconds, from 1 to [`Ttl::MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ttl(u32);

impl Ttl {
    /// One day. See the module documentation for why a longer one is refused.
    pub const MAX: u32 = 86_400;

    /// What a TTL has to be, for a refusal that says so.
    pub const RULE: &'static str = "a whole number of seconds from 1 to 86400 (one day)";

    /// Parse one from a document integer, or say nothing.
    #[must_use]
    pub fn new(seconds: i64) -> Option<Self> {
        let seconds = u32::try_from(seconds).ok()?;
        (1..=Self::MAX).contains(&seconds).then_some(Self(seconds))
    }

    /// The number of seconds.
    #[must_use]
    pub const fn seconds(self) -> u32 {
        self.0
    }
}

/// The zone the records are placed under: `home.arpa` or a name beneath it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Zone(String);

impl Zone {
    /// RFC 8375's special-use suffix, and the only part of the tree a zone
    /// here may occupy.
    pub const SUFFIX: &'static str = "home.arpa";

    /// What a zone has to be, for a refusal that says so.
    pub const RULE: &'static str = "`home.arpa` or a domain name ending in `.home.arpa` \
                                    (RFC 8375): dot-separated labels of a-z, 0-9 and hyphen, \
                                    each 1-63 bytes and not starting or ending with a hyphen, \
                                    253 bytes in total, lowercase, with no trailing dot";

    /// Parse one, or say nothing.
    ///
    /// **Lowercase only, and not lowercased for you**, on
    /// `crate::resolver::SearchDomain`'s argument: accepting `Home.Arpa` and
    /// writing `home.arpa` would make a document that does not round-trip to
    /// its own bytes.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        let under = text == Self::SUFFIX || text.ends_with(".home.arpa");
        if !under || text.len() > 253 {
            return None;
        }
        text.split('.').all(is_label).then(|| Self(text.to_owned()))
    }

    /// The zone, as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Zone {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One record's label: a single DNS label, the part before the zone.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label(String);

impl Label {
    /// What a label has to be, for a refusal that says so.
    pub const RULE: &'static str = "a single DNS label: a-z, 0-9 and hyphen, 1-63 bytes, not \
                                    starting or ending with a hyphen, lowercase, with no dot \
                                    (the zone supplies the rest of the name)";

    /// Parse one, or say nothing.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        is_label(text).then(|| Self(text.to_owned()))
    }

    /// The label, as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Label {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// RFC 1123's host label, lowercase.
fn is_label(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && !text.starts_with('-')
        && !text.ends_with('-')
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_zone_is_home_arpa_or_beneath_it() {
        for good in ["home.arpa", "my-fleet.home.arpa", "a.b.home.arpa"] {
            assert!(Zone::new(good).is_some(), "{good} was refused");
        }
        for bad in [
            "",
            "arpa",
            "local",
            "example.com",
            "evilhome.arpa",
            "my-fleet.home.arpa.",
            "My-Fleet.home.arpa",
            ".home.arpa",
            "-x.home.arpa",
            "two words.home.arpa",
        ] {
            assert!(Zone::new(bad).is_none(), "{bad:?} was accepted as a zone");
        }
    }

    #[test]
    fn a_label_is_one_lowercase_host_label() {
        for good in ["dev-01", "arbor", "a", "x1"] {
            assert!(Label::new(good).is_some(), "{good} was refused");
        }
        let long = "a".repeat(64);
        for bad in ["", "dev-01.home", "DEV", "-a", "a-", "a_b", long.as_str()] {
            assert!(Label::new(bad).is_none(), "{bad:?} was accepted as a label");
        }
    }

    #[test]
    fn a_ttl_is_from_one_second_to_one_day() {
        assert_eq!(Ttl::new(1).map(Ttl::seconds), Some(1));
        assert_eq!(Ttl::new(86_400).map(Ttl::seconds), Some(86_400));
        for bad in [0, -1, 86_401, i64::MAX] {
            assert!(Ttl::new(bad).is_none(), "{bad} was accepted as a TTL");
        }
    }

    #[test]
    fn the_defaults_are_adr_047_s_and_a_written_value_wins() {
        let zone = Zone::new("f.home.arpa").unwrap();
        let silent = Dns::new(zone.clone(), None, None, Vec::new());
        assert_eq!(silent.effective_ttl().seconds(), 300);
        assert_eq!(silent.effective_negative_ttl().seconds(), 60);
        let set = Dns::new(zone, Ttl::new(30), Ttl::new(5), Vec::new());
        assert_eq!(set.effective_ttl().seconds(), 30);
        assert_eq!(set.effective_negative_ttl().seconds(), 5);
    }

    /// The refusals the schema cannot express, driven through the parser: a
    /// zone outside `home.arpa`, a dotted label, a TTL out of range, an address
    /// that is not one, and a stray key. Each names the path it refused.
    #[test]
    fn the_parser_refuses_what_the_types_refuse_and_names_where() {
        for (src, names) in [
            ("[dns]\nzone = \"example.com\"\n", "dns.zone"),
            ("[dns]\nzone = \"f.home.arpa\"\n[dns.records.\"a.b\"]\naddress = \"10.0.0.1\"\n", "dns.records"),
            ("[dns]\nzone = \"f.home.arpa\"\nttl = 0\n", "dns.ttl"),
            ("[dns]\nzone = \"f.home.arpa\"\nnegative-ttl = 86401\n", "dns.negative-ttl"),
            ("[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0\"\n", "dns.records.a.address"),
            ("[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\nttl = -5\n", "dns.records.a.ttl"),
            ("[dns]\nzone = \"f.home.arpa\"\nforward = \"9.9.9.9\"\n", "dns.forward"),
            ("[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\ncname = \"b\"\n", "dns.records.a.cname"),
        ] {
            let text = format!("version = 1\n{src}");
            let err = crate::Document::parse(&text).expect_err(&text);
            assert!(err.to_string().contains(names), "{text}: {err}");
        }
    }

    /// The table as `ADR-047` §8 writes it, with a zone, read back whole.
    #[test]
    fn the_adr_047_example_reads_as_written() {
        let doc = crate::Document::parse(
            "version = 1\n[dns]\nzone = \"my-fleet.home.arpa\"\nttl = 300\nnegative-ttl = 60\n\
             [dns.records.arbor]\naddress = \"192.168.50.10\"\n\
             [dns.records.printer]\naddress = \"192.168.50.40\"\nttl = 60\n",
        )
        .unwrap();
        let dns = doc.dns().unwrap();
        assert_eq!(dns.zone().as_str(), "my-fleet.home.arpa");
        let got: Vec<(&str, Ipv4Addr, Option<u32>)> = dns
            .records()
            .iter()
            .map(|r| (r.label().as_str(), r.address(), r.ttl().map(Ttl::seconds)))
            .collect();
        assert_eq!(
            got,
            [
                ("arbor", Ipv4Addr::new(192, 168, 50, 10), None),
                ("printer", Ipv4Addr::new(192, 168, 50, 40), Some(60)),
            ]
        );
    }

    #[test]
    fn records_are_held_in_label_order() {
        let zone = Zone::new("f.home.arpa").unwrap();
        let r = |l: &str| Record::new(Label::new(l).unwrap(), Ipv4Addr::LOCALHOST, None);
        let dns = Dns::new(zone, None, None, vec![r("zed"), r("alpha"), r("mid")]);
        let labels: Vec<&str> = dns.records().iter().map(|r| r.label().as_str()).collect();
        assert_eq!(labels, ["alpha", "mid", "zed"]);
    }
}
