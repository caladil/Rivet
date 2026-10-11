//! What a wired interface should look like, as a plan rather than as an action.
//!
//! `PLAN.md` §2.2 is the stage this serves: `eth0` up, an address, a default
//! route. `ADR-004` §2.1 is why it is worth having as its own vocabulary —
//! *"almost nothing in the path is link-specific"*, so everything here is
//! equally true of `wlan0` at Stage 2 and nothing in it mentions Ethernet.
//!
//! Note what is *not* here, for the same reason [`crate::mount`] has no `MS_*`
//! constants: no netlink, no `AF_INET`, no `libc`, no socket. Those are
//! platform facts and `ADR-014` §3 keeps them out of core. The adapter that
//! performs a [`StaticAddressing`] translates it into whatever the kernel calls
//! it — which is also what makes every rule below testable with no network, no
//! root and no device (`ADR-014` §3.1).
//!
//! # The two refusals this module exists for
//!
//! Validation earns its place only if it catches something a caller would
//! otherwise get wrong, so there are exactly two rules and both are operator
//! mistakes rather than type errors:
//!
//! 1. **A gateway outside the interface's own subnet.** The kernel cannot route
//!    to a next hop it has no way to reach, and the failure arrives late — as a
//!    refused `RTM_NEWROUTE` after the address has already been assigned, on a
//!    headless machine, over the link that was being brought up. Catching it
//!    before the first syscall turns a half-applied configuration into a
//!    refusal that names the mistake.
//! 2. **A host address that is the subnet's network or broadcast address.**
//!    `192.168.1.0/24` and `192.168.1.255/24` are the two values an operator
//!    reaches for by miscounting, and neither is a host.
//!
//! `AGENTS.md` §5 — refuse rather than guess. Neither rule is stated as a claim
//! about what Linux does with such a configuration, because that has not been
//! read from a kernel; they are stated as what **Rivet** declines to ask for.

use std::net::Ipv4Addr;

use crate::CoreError;

/// The longest interface name Linux accepts, in bytes.
///
/// `IFNAMSIZ` is 16 and includes the terminating NUL, so the name itself is at
/// most 15 bytes. The number is here rather than in the adapter because it is a
/// property of the *name*, which is what this module owns; the adapter's job is
/// to put the bytes on a socket, not to re-derive the rule.
pub const INTERFACE_NAME_MAX: usize = 15;

/// Why an interface name was refused.
///
/// A typed reason rather than a message, per `ADR-007` §3.6.5 — a caller that
/// wants to say something different about a name that is too long than about
/// one containing a slash must be able to match on it rather than parse prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFault {
    Empty,
    /// Longer than [`INTERFACE_NAME_MAX`].
    TooLong {
        len: usize,
    },
    /// `.` and `..` are refused because an interface name is used as a path
    /// component under `/sys/class/net`, where both mean something else.
    DotOrDotDot,
    /// `/`, `:`, whitespace and NUL. The character is carried because "invalid
    /// character" without saying which one is the kind of message that makes a
    /// person retype a name that was already right.
    ForbiddenCharacter {
        ch: char,
    },
}

impl core::fmt::Display for NameFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "it is empty"),
            Self::TooLong { len } => write!(
                f,
                "it is {len} bytes and the kernel accepts at most {INTERFACE_NAME_MAX}"
            ),
            Self::DotOrDotDot => write!(f, "`.` and `..` are path components, not names"),
            Self::ForbiddenCharacter { ch } => {
                write!(f, "it contains {ch:?}, which the kernel does not accept")
            }
        }
    }
}

/// The name of a network interface, checked once so nothing downstream has to.
///
/// The rules are the kernel's own, from `dev_valid_name()` in
/// `net/core/dev.c`: not empty, shorter than `IFNAMSIZ`, not `.` or `..`, and
/// containing no `/`, no `:` and no whitespace. They are reproduced here rather
/// than discovered at the syscall because the syscall's answer is `EINVAL` with
/// no indication of which rule was broken.
///
/// **This is a check, not a claim that the interface exists.** Whether `eth0`
/// is present is the kernel's to answer and is never cached — `AGENTS.md` §5.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceName(String);

impl InterfaceName {
    /// The whole rule, in one sentence, for a diagnostic that must not quote
    /// its input.
    ///
    /// `&'static str` on purpose. [`CoreError::InvalidInterfaceName`] carries
    /// the offending name and is the right error for a caller that has one; a
    /// caller that must **not** reproduce what it read — `rivet-config`
    /// refusing a key in a document that also holds a passphrase (`ADR-020`
    /// §4) — needs a description of the rule with no capacity to carry input,
    /// and this is it. The rule is stated here, beside the code that enforces
    /// it, rather than restated in every crate that reports it (`ADR-011`).
    pub const RULE: &'static str = "an interface name is 1 to 15 bytes, is not `.` or `..`, and contains no `/`, no `:`, \
         no whitespace and no NUL — the kernel's own rule, from `dev_valid_name()`";

    pub fn new(name: &str) -> Result<Self, CoreError> {
        let fault = if name.is_empty() {
            Some(NameFault::Empty)
        } else if name.len() > INTERFACE_NAME_MAX {
            Some(NameFault::TooLong { len: name.len() })
        } else if name == "." || name == ".." {
            Some(NameFault::DotOrDotDot)
        } else {
            name.chars()
                .find(|c| *c == '/' || *c == ':' || c.is_whitespace() || *c == '\0')
                .map(|ch| NameFault::ForbiddenCharacter { ch })
        };
        match fault {
            Some(why) => Err(CoreError::InvalidInterfaceName {
                name: name.to_string(),
                why,
            }),
            None => Ok(Self(name.to_string())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for InterfaceName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An IPv4 address together with the prefix length that gives it a subnet.
///
/// One type rather than two fields, because an address without its prefix
/// length cannot answer the only question anything here asks of it — *is this
/// other address on the same link?* — and a pair of fields that must travel
/// together is a type that has not been written yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Cidr {
    address: Ipv4Addr,
    prefix_len: u8,
}

impl Ipv4Cidr {
    /// The whole rule, for a diagnostic that must not quote its input.
    ///
    /// Same argument as [`InterfaceName::RULE`]: [`CoreError::MalformedAddress`]
    /// carries the text and is right for a caller that may echo it, and a caller
    /// that may not needs the rule instead. Note that "no leading zeros" is
    /// `std`'s refusal rather than Rivet's, and is stated because an operator
    /// who wrote `192.168.001.1` has met the one address mistake that reads as
    /// correct.
    pub const RULE: &'static str = "an address is written `A.B.C.D/N` — a host address and the prefix length that gives it \
         a subnet, for example `192.168.1.50/24`. The prefix is required and the octets carry no \
         leading zeros";

    /// Build from parts, refusing a prefix length that is not a prefix length.
    pub fn new(address: Ipv4Addr, prefix_len: u8) -> Result<Self, CoreError> {
        if prefix_len > 32 {
            return Err(CoreError::PrefixTooLong { prefix_len });
        }
        Ok(Self {
            address,
            prefix_len,
        })
    }

    /// Parse `A.B.C.D/N`.
    ///
    /// The address half is parsed by `std`, deliberately: `std`'s parser
    /// refuses leading zeros, so `192.168.001.1` is an error rather than
    /// silently becoming something else. That is the exact failure `inet_aton`
    /// has — it reads a leading zero as octal — and it is worth inheriting a
    /// refusal instead of writing a parser that would have to remember not to.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        let malformed = || CoreError::MalformedAddress {
            text: text.to_string(),
        };
        let (address, prefix) = text.split_once('/').ok_or_else(malformed)?;
        let address: Ipv4Addr = address.parse().map_err(|_| malformed())?;
        // Not `prefix.parse::<u8>()` alone: "256" would overflow to a parse
        // error indistinguishable from "abc", and "33" would be a value error
        // that PrefixTooLong names precisely. Splitting them keeps the two
        // mistakes distinguishable in the message the operator sees.
        let prefix_len: u16 = prefix.parse().map_err(|_| malformed())?;
        let prefix_len = u8::try_from(prefix_len).unwrap_or(u8::MAX);
        Self::new(address, prefix_len)
    }

    pub fn address(&self) -> Ipv4Addr {
        self.address
    }

    pub fn prefix_len(&self) -> u8 {
        self.prefix_len
    }

    /// The subnet mask as a 32-bit word.
    ///
    /// `u32::MAX << 32` is undefined in C and a panic in debug Rust, so the
    /// all-hosts case is handled rather than shifted.
    pub fn netmask(&self) -> u32 {
        if self.prefix_len == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(self.prefix_len))
        }
    }

    /// The subnet's own address — the host bits cleared.
    pub fn network(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.address) & self.netmask())
    }

    /// The subnet's broadcast address, where it has one.
    ///
    /// `None` for `/31` and `/32`. A `/31` is a point-to-point link with two
    /// hosts and no broadcast address at all (RFC 3021), and a `/32` is a
    /// single host. Returning `Some` of something for those would put a
    /// meaningless `IFA_BROADCAST` on the wire.
    pub fn broadcast(&self) -> Option<Ipv4Addr> {
        if self.prefix_len >= 31 {
            return None;
        }
        Some(Ipv4Addr::from(u32::from(self.address) | !self.netmask()))
    }

    /// Is `other` on this link — reachable without going through a router?
    pub fn contains(&self, other: Ipv4Addr) -> bool {
        u32::from(other) & self.netmask() == u32::from(self.address) & self.netmask()
    }
}

impl core::fmt::Display for Ipv4Cidr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix_len)
    }
}

/// The desired state of one interface addressed by hand.
///
/// Static rather than leased, because `ADR-004` §8's DHCP client is BusyBox
/// `udhcpc` — a C mechanism this repository cannot build yet — and Stage 1 has
/// to be reachable before it exists. `crate::net` deliberately has no lease
/// type: see `rivet_adapter_net::dhcp`, which refuses rather than pretending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticAddressing {
    pub interface: InterfaceName,
    pub address: Ipv4Cidr,
    /// The default route's next hop, or `None` to bring the interface up and
    /// address it without making it the way off this machine.
    pub gateway: Option<Ipv4Addr>,
}

impl StaticAddressing {
    /// The rule a gateway obeys, for a diagnostic that must not quote its
    /// input.
    ///
    /// It is separate from [`Ipv4Cidr::RULE`] because the two shapes differ by
    /// exactly the thing operators get wrong: a gateway is a *host*, so it
    /// carries no prefix, and `--gateway 192.168.1.1/24` is a mistake that
    /// should be named rather than have its prefix quietly dropped.
    pub const GATEWAY_RULE: &'static str = "a gateway is a bare host address, `A.B.C.D`, with no prefix length — it is one machine \
         on this interface's own subnet, not a subnet";

    /// Check what the kernel would otherwise refuse late, or accept and regret.
    ///
    /// See the module docs for why these two and only these two.
    pub fn validate(&self) -> Result<(), CoreError> {
        // Rule 2 first: a network or broadcast address as the *host* address
        // makes the subnet arithmetic in rule 1 meaningless, so reporting the
        // gateway would be reporting a consequence rather than the cause.
        if self.address.prefix_len() < 31 {
            let addr = self.address.address();
            if addr == self.address.network() {
                return Err(CoreError::NotAHostAddress {
                    address: self.address,
                    role: AddressRole::Network,
                });
            }
            if self.address.broadcast() == Some(addr) {
                return Err(CoreError::NotAHostAddress {
                    address: self.address,
                    role: AddressRole::Broadcast,
                });
            }
        }

        if let Some(gateway) = self.gateway
            && !self.address.contains(gateway)
        {
            return Err(CoreError::GatewayOffLink {
                gateway,
                subnet: self.address,
            });
        }

        Ok(())
    }
}

/// Which reserved role an address turned out to have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressRole {
    Network,
    Broadcast,
}

impl core::fmt::Display for AddressRole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Network => write!(f, "the subnet's network address"),
            Self::Broadcast => write!(f, "the subnet's broadcast address"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // -----------------------------------------------------------------------
    // Interface names
    // -----------------------------------------------------------------------

    #[test]
    fn the_names_this_project_actually_uses_are_accepted() {
        // eth0 and wlan0 were both read from a Raspberry Pi 4 Model B on
        // 2026-08-22 — `ADR-004` §8.2 for eth0, §8.1 for wlan0. Provenance
        // matters here (`AGENTS.md` §4): these are the two names the board
        // reported, not two names chosen to make a test pass.
        for name in ["eth0", "wlan0", "lo"] {
            assert!(InterfaceName::new(name).is_ok(), "{name} was refused");
        }
    }

    #[test]
    fn an_empty_name_is_refused() {
        assert_eq!(
            InterfaceName::new(""),
            Err(CoreError::InvalidInterfaceName {
                name: String::new(),
                why: NameFault::Empty
            })
        );
    }

    /// The boundary, from both sides. `IFNAMSIZ` is 16 *including* the NUL, so
    /// 15 is the last accepted length — an off-by-one here produces a name the
    /// kernel truncates rather than refuses, which is worse than either.
    #[test]
    fn a_name_is_accepted_at_fifteen_bytes_and_refused_at_sixteen() {
        let fifteen = "a".repeat(INTERFACE_NAME_MAX);
        let sixteen = "a".repeat(INTERFACE_NAME_MAX + 1);
        assert!(InterfaceName::new(&fifteen).is_ok());
        assert_eq!(
            InterfaceName::new(&sixteen),
            Err(CoreError::InvalidInterfaceName {
                name: sixteen,
                why: NameFault::TooLong { len: 16 }
            })
        );
    }

    #[test]
    fn dot_and_dot_dot_are_refused_as_path_components() {
        for name in [".", ".."] {
            assert_eq!(
                InterfaceName::new(name),
                Err(CoreError::InvalidInterfaceName {
                    name: name.to_string(),
                    why: NameFault::DotOrDotDot
                })
            );
        }
    }

    #[test]
    fn the_refused_character_is_named_rather_than_merely_reported() {
        for (name, ch) in [
            ("eth/0", '/'),
            ("eth:0", ':'),
            ("eth 0", ' '),
            ("eth\t0", '\t'),
            ("eth\n0", '\n'),
            // "eth", NUL, "0" — written with an explicit hex escape because
            // `\0` followed by a digit reads as an octal escape to anyone who
            // learned C first, and Rust has no octal escapes.
            ("eth\x000", '\0'),
        ] {
            assert_eq!(
                InterfaceName::new(name),
                Err(CoreError::InvalidInterfaceName {
                    name: name.to_string(),
                    why: NameFault::ForbiddenCharacter { ch }
                }),
                "{name:?}"
            );
        }
    }

    /// A dot inside a name is legal — VLAN interfaces are `eth0.100` — and the
    /// `.`/`..` rule must not grow into a ban on the character.
    #[test]
    fn a_dot_inside_a_name_is_not_the_dot_rule() {
        assert!(InterfaceName::new("eth0.100").is_ok());
    }

    // -----------------------------------------------------------------------
    // The rule strings
    // -----------------------------------------------------------------------

    /// The rules exist so a caller that may not echo its input can still say
    /// what was wrong. A rule that has drifted from the code it describes is
    /// worse than none, and prose cannot be asserted — so what is asserted is
    /// the part that *can* drift silently: the number.
    ///
    /// `AGENTS.md` §4 — a cheap guard that fails on every cycle. Change
    /// `INTERFACE_NAME_MAX` without changing the sentence and this fails.
    #[test]
    fn each_rule_string_still_names_the_constant_it_describes() {
        assert!(
            InterfaceName::RULE.contains(&INTERFACE_NAME_MAX.to_string()),
            "{}",
            InterfaceName::RULE
        );
        // And the shapes each rule is about, so `Ipv4Cidr::RULE` cannot end up
        // describing a gateway or vice versa.
        assert!(Ipv4Cidr::RULE.contains("A.B.C.D/N"), "{}", Ipv4Cidr::RULE);
        assert!(
            StaticAddressing::GATEWAY_RULE.contains("no prefix length"),
            "{}",
            StaticAddressing::GATEWAY_RULE
        );
    }

    /// A rule that quotes an example must quote one the parser accepts, or it
    /// teaches an operator a spelling that will be refused.
    #[test]
    fn the_address_rules_example_is_one_the_parser_accepts() {
        assert!(Ipv4Cidr::parse("192.168.1.50/24").is_ok());
        assert!(Ipv4Cidr::RULE.contains("192.168.1.50/24"));
    }

    // -----------------------------------------------------------------------
    // Addresses
    // -----------------------------------------------------------------------

    #[test]
    fn a_cidr_round_trips_through_its_own_display() {
        let cidr = Ipv4Cidr::parse("192.168.1.50/24").expect("valid");
        assert_eq!(cidr.to_string(), "192.168.1.50/24");
        assert_eq!(Ipv4Cidr::parse(&cidr.to_string()), Ok(cidr));
    }

    #[test]
    fn text_with_no_prefix_is_refused_rather_than_assumed_to_be_a_slash_32() {
        // Defaulting a missing prefix is exactly the "refuse rather than guess"
        // failure: /32 and /24 differ in whether the gateway is reachable, and
        // the wrong guess succeeds.
        assert_eq!(
            Ipv4Cidr::parse("192.168.1.50"),
            Err(CoreError::MalformedAddress {
                text: "192.168.1.50".to_string()
            })
        );
    }

    #[test]
    fn a_leading_zero_is_refused_rather_than_read_as_octal() {
        assert!(Ipv4Cidr::parse("192.168.001.1/24").is_err());
    }

    #[test]
    fn a_prefix_longer_than_thirty_two_is_named_as_such() {
        assert_eq!(
            Ipv4Cidr::parse("10.0.0.1/33"),
            Err(CoreError::PrefixTooLong { prefix_len: 33 })
        );
        assert_eq!(
            Ipv4Cidr::new("10.0.0.1".parse().expect("literal"), 255),
            Err(CoreError::PrefixTooLong { prefix_len: 255 })
        );
    }

    /// A prefix that overflows `u8` must not wrap into a *valid* prefix. `256`
    /// truncates to `0`, which is `0.0.0.0/0` — the default route's own prefix,
    /// and the most dangerous possible silent substitution here.
    #[test]
    fn a_prefix_that_overflows_does_not_wrap_into_a_valid_one() {
        let refused = Ipv4Cidr::parse("10.0.0.1/256");
        assert!(refused.is_err(), "got {refused:?}");
        assert_ne!(refused.map(|c| c.prefix_len()), Ok(0));
    }

    #[test]
    fn garbage_is_refused_and_the_text_is_carried_back() {
        for text in [
            "",
            "/",
            "10.0.0.1/",
            "/24",
            "not-an-address/24",
            "10.0.0.1/x",
        ] {
            assert_eq!(
                Ipv4Cidr::parse(text),
                Err(CoreError::MalformedAddress {
                    text: text.to_string()
                }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_mask_network_and_broadcast_agree_with_the_prefix() {
        let cidr = Ipv4Cidr::parse("192.168.1.50/24").expect("valid");
        assert_eq!(cidr.netmask(), 0xFFFF_FF00);
        assert_eq!(cidr.network(), Ipv4Addr::new(192, 168, 1, 0));
        assert_eq!(cidr.broadcast(), Some(Ipv4Addr::new(192, 168, 1, 255)));
    }

    /// `/0` is the shift that would be undefined in C and a panic in debug
    /// Rust. It is a legal prefix, so it is handled rather than avoided.
    #[test]
    fn a_slash_zero_masks_nothing_and_does_not_panic() {
        let cidr = Ipv4Cidr::parse("10.1.2.3/0").expect("valid");
        assert_eq!(cidr.netmask(), 0);
        assert_eq!(cidr.network(), Ipv4Addr::UNSPECIFIED);
        assert_eq!(cidr.broadcast(), Some(Ipv4Addr::new(255, 255, 255, 255)));
        assert!(cidr.contains(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn a_slash_thirty_one_and_a_slash_thirty_two_have_no_broadcast_address() {
        assert_eq!(
            Ipv4Cidr::parse("10.0.0.1/31").map(|c| c.broadcast()),
            Ok(None)
        );
        assert_eq!(
            Ipv4Cidr::parse("10.0.0.1/32").map(|c| c.broadcast()),
            Ok(None)
        );
        // And /30 still does, so the boundary is where it is meant to be.
        assert_eq!(
            Ipv4Cidr::parse("10.0.0.1/30").map(|c| c.broadcast()),
            Ok(Some(Ipv4Addr::new(10, 0, 0, 3)))
        );
    }

    #[test]
    fn containment_is_decided_by_the_mask_and_not_by_the_first_three_octets() {
        // The lazy implementation — compare the leading octets — gets /22
        // wrong, and /22 is an ordinary home-lab subnet rather than an exotic
        // one. 10.0.4.9 and 10.0.5.9 are in the same /22 and different /24s.
        let cidr = Ipv4Cidr::parse("10.0.4.9/22").expect("valid");
        assert!(cidr.contains(Ipv4Addr::new(10, 0, 5, 9)));
        assert!(cidr.contains(Ipv4Addr::new(10, 0, 7, 255)));
        assert!(!cidr.contains(Ipv4Addr::new(10, 0, 8, 1)));
    }

    // -----------------------------------------------------------------------
    // The plan
    // -----------------------------------------------------------------------

    fn plan(address: &str, gateway: Option<&str>) -> StaticAddressing {
        StaticAddressing {
            interface: InterfaceName::new("eth0").expect("literal"),
            address: Ipv4Cidr::parse(address).expect("literal"),
            gateway: gateway.map(|g| g.parse().expect("literal")),
        }
    }

    #[test]
    fn an_ordinary_home_network_validates() {
        assert_eq!(
            plan("192.168.1.50/24", Some("192.168.1.1")).validate(),
            Ok(())
        );
    }

    #[test]
    fn no_gateway_at_all_is_a_valid_plan() {
        assert_eq!(plan("192.168.1.50/24", None).validate(), Ok(()));
    }

    /// The rule the module exists for. `ADR-004` §2.1 puts the whole layer-3
    /// stack on the wired link first precisely so a failure has one candidate
    /// cause; a gateway typed with the wrong third octet would give it two.
    #[test]
    fn a_gateway_outside_the_subnet_is_refused_before_anything_is_applied() {
        let refused = plan("192.168.1.50/24", Some("192.168.2.1"));
        assert_eq!(
            refused.validate(),
            Err(CoreError::GatewayOffLink {
                gateway: Ipv4Addr::new(192, 168, 2, 1),
                subnet: refused.address,
            })
        );
    }

    /// A `/32` can never reach a gateway on-link, so the same rule catches the
    /// point-to-point configuration that would need a scope-link route first.
    /// Rivet does not build that route, so it declines rather than half-doing
    /// it.
    #[test]
    fn a_slash_thirty_two_with_a_gateway_is_refused() {
        assert!(plan("10.0.0.5/32", Some("10.0.0.1")).validate().is_err());
    }

    #[test]
    fn the_subnets_own_network_address_is_not_a_host_address() {
        let refused = plan("192.168.1.0/24", Some("192.168.1.1"));
        assert_eq!(
            refused.validate(),
            Err(CoreError::NotAHostAddress {
                address: refused.address,
                role: AddressRole::Network,
            })
        );
    }

    #[test]
    fn the_subnets_broadcast_address_is_not_a_host_address() {
        let refused = plan("192.168.1.255/24", Some("192.168.1.1"));
        assert_eq!(
            refused.validate(),
            Err(CoreError::NotAHostAddress {
                address: refused.address,
                role: AddressRole::Broadcast,
            })
        );
    }

    /// Both halves of a `/31` are hosts (RFC 3021), so the rule above must not
    /// fire on them. This is the case that a naive "clear the host bits and
    /// compare" implementation gets wrong.
    #[test]
    fn both_addresses_in_a_slash_thirty_one_are_hosts() {
        assert_eq!(plan("10.0.0.0/31", Some("10.0.0.1")).validate(), Ok(()));
        assert_eq!(plan("10.0.0.1/31", Some("10.0.0.0")).validate(), Ok(()));
    }

    /// The order the two rules are checked in is deliberate and is stated in
    /// `validate`'s comment, so it is asserted rather than left to be re-derived
    /// by whoever next reorders them: a plan that breaks both must report the
    /// address, because the gateway verdict is computed *from* the address.
    #[test]
    fn a_plan_that_breaks_both_rules_reports_the_address_not_the_gateway() {
        let refused = plan("192.168.1.0/24", Some("10.9.9.9"));
        assert!(matches!(
            refused.validate(),
            Err(CoreError::NotAHostAddress { .. })
        ));
    }

    /// Every refusal must say enough for an operator with no console to act on
    /// it. An error whose `Display` omits the offending value is a support
    /// request rather than a diagnostic.
    #[test]
    fn every_refusal_names_the_value_that_caused_it() {
        let cases = [
            (InterfaceName::new("eth/0").unwrap_err(), "eth/0"),
            (Ipv4Cidr::parse("nonsense/24").unwrap_err(), "nonsense/24"),
            (Ipv4Cidr::parse("10.0.0.1/33").unwrap_err(), "33"),
            (
                plan("192.168.1.50/24", Some("192.168.2.1"))
                    .validate()
                    .unwrap_err(),
                "192.168.2.1",
            ),
            (
                plan("192.168.1.0/24", None).validate().unwrap_err(),
                "192.168.1.0/24",
            ),
        ];
        for (error, expected) in cases {
            let rendered = error.to_string();
            assert!(
                rendered.contains(expected),
                "{rendered:?} does not name {expected:?}"
            );
        }
    }
}
