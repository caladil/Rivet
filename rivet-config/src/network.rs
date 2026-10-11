//! The network table: **how this machine is told its own address.**
//!
//! Ruled 2026-08-23. Three shapes were offered and the maintainer
//! took the second — *a `[network]` table plus the provisioning file* — adding
//! that many devices will still be addressed by DHCP (`.rulings/` R-267). That
//! second sentence is what folds option 3 in: **DHCP is a value of this table,
//! not a competing design**, and it is first-class beside static addressing
//! from the first line of schema rather than bolted on when `udhcpc` arrives.
//!
//! # Why the table exists at all
//!
//! `cmd/rivet-netd` took `--interface`, `--address` and `--gateway` on
//! `argv`. That works and it is the wrong authority: `ADR-007` §3.3 makes the
//! configuration document the statement of *what the machine should be*, so an
//! address living in a service's argument list is a second thing that also
//! looks authoritative — the defect `ADR-011` §3 is entirely about. The same
//! ruling was made the same day one directory over, when `/etc` went from two
//! generators to one (`ADR-007` §3.5, `ADR-008` §5.3), and it is the governing
//! precedent here.
//!
//! ```toml
//! [network.eth0]
//! method  = "static"
//! address = "192.168.1.50/24"
//! gateway = "192.168.1.1"
//!
//! [network.wlan0]
//! method = "dhcp"
//! ```
//!
//! # The surface, and the argument for it against `ADR-007` §3.6
//!
//! `method` is **required, and it is a key rather than an inference.** The
//! obvious cheaper design is to let the presence of `address` mean static and
//! its absence mean DHCP, and that design is refused. The rule that refuses it
//! is `ADR-007` §3.6.5:
//!
//! > **Rivet does not accept an input it intends to discard.**
//!
//! Read it against each candidate:
//!
//! | Surface | What happens to `method = "dhcp"` beside an `address` | Verdict |
//! | :-- | :-- | :-- |
//! | `address` present ⇒ static | there is no `method` key, so the operator's *intent* is unwritable; a stale `address` line silently wins | the intent is the discarded input |
//! | `dhcp = true` boolean | two spellings of one fact, and the boolean and the address can disagree with no vocabulary to name the disagreement | `ADR-011` §3, in miniature |
//! | **`method` required, contradictions refused** | the document names the mistake, at a line and column, and nothing is applied | chosen |
//!
//! The sharp edge is the middle row of the first column, so it is worth being
//! explicit: **a document that says `method = "dhcp"` and also carries a static
//! `address` is refused, not resolved.** It is not resolved by precedence, not
//! by "the address wins because it is more specific", and not by ignoring the
//! unused key. Two of those three silently discard something the operator
//! wrote, and the third — ignoring the key — is the same defect as an ignored
//! misspelling, which §3.6.5 names in as many words.
//!
//! `cmd/rivet-netd` already refused exactly this pair on `argv`
//! (`args::ArgError::Conflict`) and argued it from `ADR-011`. This table is
//! where that refusal now lives, one layer earlier, where it can carry a
//! position in a file.
//!
//! # Absence, and what it is allowed to mean
//!
//! [`crate::services`] states the rule: *absence is only allowed to mean
//! something when it means exactly one thing.* Applied here:
//!
//! * **The whole `[network]` table may be absent**, and means one thing: the
//!   operator has configured no interface. Unambiguous, so it is allowed — and
//!   it is the steady state of a machine whose networking has not been set up
//!   yet.
//! * **`method` is required in every interface that appears.** There is no
//!   counterpart anywhere for it to be inherited from — no manifest, no image
//!   default that the record specifies — so an absent `method` is *nobody
//!   said*, and `AGENTS.md` §3 refuses rather than guessing. A default of
//!   `dhcp` was considered and rejected on the record's own terms: `ADR-004`
//!   §8 says the DHCP client does not exist yet, so the default would name a
//!   mechanism that cannot run, and the rejected option 3 warned that "a
//!   machine that cannot be addressed without it is unreachable".
//! * **`address` is required under `method = "static"`** and refused under
//!   `dhcp`. Its absence under `static` is not a shorthand for anything.
//! * **`gateway` is optional under `static`** and refused under `dhcp`. Its
//!   absence means one thing, and `rivet-core`'s own type already says what:
//!   *"bring the interface up and address it without making it the way off this
//!   machine"* — a coherent request rather than an incomplete one. Under
//!   `dhcp` it is refused because the lease carries one.
//!
//! # The values are `rivet-core`'s, not this crate's
//!
//! [`InterfaceName`], [`Ipv4Cidr`] and [`StaticAddressing`] already exist,
//! already carry their own rules, and are already tested against the kernel's
//! own `dev_valid_name()`. `ADR-009` §4 forbids a second declaration of a type
//! Rivet already owns, and a second address parser is the plainest possible
//! example of the second authority `ADR-011` is about. So this module declares
//! no address type; it declares which *keys* carry them and what a valid
//! combination is.
//!
//! That is also why [`Addressing::Static`] holds an address and a gateway
//! rather than a whole [`StaticAddressing`]: the core type carries the
//! interface name too, and an [`Interface`] that held both would have two
//! places to read one name from. [`Interface::static_plan`] assembles the core
//! type on demand, from the one name there is.
//!
//! # What this module does not do: apply
//!
//! It says what the machine should be. Whether `eth0` exists, whether the
//! lease can be obtained, whether the link has a carrier — all observed state,
//! all `ADR-007` §3.3's other half, none of it here and none of it persistable
//! into a document. `rivet-adapter-net` reads the kernel for those at the
//! moment it needs them.
//!
//! **DHCP is validated here and obtained there.** A document asking for
//! `method = "dhcp"` is a well-formed statement of desired state and is
//! accepted as one, whatever the card carries: whether a lease can be taken is
//! apply-time state. The supervisor runs a client for the interface and
//! `rivet-netd` turns the lease it records into the same plan a static
//! document produces, so one convergence applies both (`ADR-004` §2.1: *"almost
//! nothing in the path is link-specific"*).
//!
//! Refusing `dhcp` at *parse* time would conflate the schema with a card's
//! capability, and the schema would have to change when the capability did;
//! accepting it here is what lets a document mean the same thing on every
//! card, which is what `ADR-007` §3.2 versions the schema for.

use std::net::Ipv4Addr;

use rivet_core::net::{InterfaceName, Ipv4Cidr, StaticAddressing};
use rivet_core::ssid::Ssid;

/// How an interface gets its address. A closed vocabulary per `ADR-006` §6.4.
///
/// Two values, and both are first-class. `ADR-004` §8 makes DHCP a Tier B
/// mechanism — C now, Rust later, tracked — which is a statement about *how it
/// is implemented*, not about whether an operator may ask for it. The
/// maintainer's ruling on 2026-08-23 was explicit that many devices will use
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Method {
    /// A lease, obtained from a DHCP server on the link.
    Dhcp,
    /// An address the operator wrote down.
    Static,
}

impl Method {
    /// The whole rule, for a diagnostic to quote without quoting the input.
    pub const RULE: &'static str =
        "a method is `dhcp` — take a lease from the link — or `static` — use the address written \
         in this document";

    /// Every spelling, in schema order, for an error to list.
    pub const ALL: &'static [&'static str] = &["dhcp", "static"];

    /// The one spelling this method has.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dhcp => "dhcp",
            Self::Static => "static",
        }
    }

    /// Exactly the spellings above, and nothing else.
    ///
    /// `DHCP` and `Static` are refused for the same reason the
    /// now-removed `crate::services::Transport` refused `TCP`: a machine whose behaviour
    /// came from a spelling nobody wrote down is worse than a machine that
    /// refused to start.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "dhcp" => Some(Self::Dhcp),
            "static" => Some(Self::Static),
            _ => None,
        }
    }
}

impl core::fmt::Display for Method {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How one interface should be addressed.
///
/// The enum is the reason a contradictory document cannot exist as a value:
/// there is no variant that holds both a lease request and a static address, so
/// the parser has nowhere to put a document that asks for both even if it
/// wanted to. That is `AGENTS.md` §3's ladder — the strongest gate available is
/// the one where the type says no — applied to a schema rather than to a stub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Addressing {
    /// Take a lease. **Nothing here says the lease can be obtained** — see the
    /// module documentation on why that is an apply-time question.
    Dhcp,
    /// Use the address written in the document.
    Static {
        /// The host address and its subnet.
        address: Ipv4Cidr,
        /// The default route's next hop, or `None` for an interface that is
        /// addressed but is not the way off this machine.
        gateway: Option<Ipv4Addr>,
    },
}

impl Addressing {
    /// Which method this is, for a report that wants the operator's own word.
    pub const fn method(&self) -> Method {
        match self {
            Self::Dhcp => Method::Dhcp,
            Self::Static { .. } => Method::Static,
        }
    }
}

/// What the operator asks of one interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    name: InterfaceName,
    addressing: Addressing,
    /// The wireless network this interface joins, if it is a wireless one.
    ///
    /// # An SSID is here and a passphrase is not
    ///
    /// `ADR-007` §7.3 keeps wireless CREDENTIALS out of this document and
    /// §7.6.3 records that the schema declares no key one could be written
    /// under. That rule is untouched: the PMK lives in `ADR-020`'s store at
    /// `rivet_core::secret::path_of`, and nothing here can hold it.
    ///
    /// **An SSID is not a credential.** Every access point broadcasts it in the
    /// clear; it is on the screen of every phone in the building. Keeping it
    /// out would protect nothing and would leave a machine unable to say which
    /// network it is FOR — which is desired state in `ADR-007` §3.3's sense,
    /// exactly like an address.
    ///
    /// **Its presence is what makes an interface wireless.** `ADR-007` §7
    /// forbids INFERRING `method` from the presence of `address` because a DHCP
    /// intent beside a stale `address` would have nowhere to be written; absence
    /// may mean something only when it means exactly one thing. An absent SSID
    /// does: no interface joins a network without naming one, so it means a
    /// wired interface and nothing else.
    ssid: Option<Ssid>,
}

impl Interface {
    /// Every key an interface entry may declare, in schema order.
    ///
    /// One list, read by the parser and by the error it produces — `ADR-011`:
    /// exactly one thing is authoritative, and it says so at the moment the
    /// answer is needed.
    pub const DECLARED_KEYS: &'static [&'static str] = &["method", "address", "gateway", "ssid"];

    /// The keys that belong to `method = "static"` alone, in the order a
    /// refusal should report them.
    ///
    /// Under `method = "dhcp"` each of these is refused rather than ignored,
    /// because the lease carries the fact it states and a key whose value has
    /// no effect is an input Rivet accepted and discarded (`ADR-007`
    /// §3.6.5).
    pub const STATIC_ONLY_KEYS: &'static [&'static str] = &["address", "gateway"];

    /// Build an interface entry. The parser is the only caller today.
    pub(crate) const fn new(
        name: InterfaceName,
        addressing: Addressing,
        ssid: Option<Ssid>,
    ) -> Self {
        Self {
            name,
            addressing,
            ssid,
        }
    }

    /// The wireless network this interface joins, or `None` for a wired one.
    #[must_use]
    pub const fn ssid(&self) -> Option<&Ssid> {
        self.ssid.as_ref()
    }

    /// Whether this interface is wireless.
    ///
    /// **Asked of the SSID rather than guessed from the name.** `wlan0` is a
    /// convention and not a rule: a machine can rename an interface, and a
    /// wired one called `wlan0` is a thing a person can produce by accident. The
    /// document says which networks a machine joins; the NAME says nothing.
    #[must_use]
    pub const fn is_wireless(&self) -> bool {
        self.ssid.is_some()
    }

    /// Which interface this configures.
    ///
    /// **A checked name, not a claim that the interface exists.** Whether
    /// `eth0` is present is the kernel's to answer and is never cached
    /// (`AGENTS.md` §5); this crate performs no I/O and could not know
    /// (`ADR-014` §3.1). A document written for a machine that has not been
    /// built yet is the normal case — `ADR-007` §3.6.3 makes the document an
    /// input artifact authored anywhere.
    pub const fn name(&self) -> &InterfaceName {
        &self.name
    }

    /// How the operator asked for it to be addressed.
    pub const fn addressing(&self) -> &Addressing {
        &self.addressing
    }

    /// `rivet-core`'s own plan, for an interface addressed by hand.
    ///
    /// `None` for a leased interface, because there is no plan until there is a
    /// lease and `rivet-core` deliberately has no lease type to return.
    ///
    /// Assembled here rather than stored, so that the interface name has one
    /// home — see the module documentation.
    pub fn static_plan(&self) -> Option<StaticAddressing> {
        match &self.addressing {
            Addressing::Dhcp => None,
            Addressing::Static { address, gateway } => Some(StaticAddressing {
                interface: self.name.clone(),
                address: *address,
                gateway: *gateway,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---------------------------------------------------------------------
    // Wireless - ADR-004, added 2026-08-30
    // ---------------------------------------------------------------------

    /// **The presence of an SSID is what makes an interface wireless**, and it
    /// is asked rather than guessed from the name. `wlan0` is a convention, not
    /// a rule.
    #[test]
    fn an_interface_is_wireless_because_it_names_a_network() {
        let doc = crate::parse::document(
            "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\nssid = \"a-network\"\n",
        )
        .expect("a valid document");
        let wlan = doc.network().first().expect("one interface");
        assert!(wlan.is_wireless());
        assert_eq!(wlan.ssid().map(|s| s.as_str()), Some("a-network"));
    }

    /// And an interface CALLED `wlan0` with no SSID is not wireless. The name
    /// says nothing; a machine can rename an interface, and a wired one called
    /// `wlan0` is a thing a person can produce by accident.
    #[test]
    fn a_name_that_looks_wireless_is_not_enough() {
        let doc =
            crate::parse::document("version = 1\n[network.wlan0]\nmethod = \"dhcp\"\n")
                .expect("valid");
        let wlan = doc.network().first().expect("one interface");
        assert!(!wlan.is_wireless(), "an interface was wireless by NAME");
        assert_eq!(wlan.ssid(), None);
    }

    /// An SSID is optional everywhere, so a wired interface omitting it is the
    /// ordinary document rather than an incomplete one.
    #[test]
    fn a_wired_interface_has_no_ssid() {
        let doc = crate::parse::document(
            "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n",
        )
        .expect("valid");
        assert!(!doc.network()[0].is_wireless());
    }

    /// **Wireless is orthogonal to addressing.** A wireless interface may take
    /// a lease or hold a static address, and both are ordinary - `ADR-004`
    /// §2.1: almost nothing in the path is link-specific.
    #[test]
    fn a_wireless_interface_may_be_static_or_dhcp() {
        for (method, extra) in [
            ("dhcp", ""),
            ("static", "address = \"10.0.0.2/24\"\n"),
        ] {
            let text = format!(
                "version = 1\n[network.wlan0]\nmethod = \"{method}\"\n{extra}ssid = \"n\"\n"
            );
            let doc = crate::parse::document(&text).unwrap_or_else(|e| panic!("{method}: {e}"));
            assert!(doc.network()[0].is_wireless(), "{method}");
        }
    }

    /// **There is no key a passphrase could be written under**, and that is
    /// `ADR-007` §7.3 and §7.6.3 rather than an omission: it is what makes a
    /// document safe to read, diff and paste into a bug report.
    #[test]
    fn there_is_no_key_for_a_credential() {
        for key in ["psk", "passphrase", "password", "pmk", "key"] {
            let text = format!(
                "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\nssid = \"n\"\n{key} = \"x\"\n"
            );
            assert!(
                crate::parse::document(&text).is_err(),
                "the schema accepted `{key}` - ADR-007 §7.3 keeps credentials out of the document"
            );
        }
    }

    /// A name a radio could not carry is refused, and the refusal does not echo
    /// it - `ADR-020` §4 applied to the document that holds it.
    #[test]
    fn a_name_too_long_for_a_radio_is_refused_without_being_quoted() {
        let long = "x".repeat(33);
        let text =
            format!("version = 1\n[network.wlan0]\nmethod = \"dhcp\"\nssid = \"{long}\"\n");
        let refused = crate::parse::document(&text).expect_err("33 bytes is too long");
        let said = refused.to_string();
        assert!(!said.contains("xxxxxxxx"), "the refusal echoed the name: {said}");
        assert!(said.contains("32"), "{said}");
    }

    /// **It survives a canonical rewrite.** `apply edit` rewrites the document,
    /// and a key the serialiser did not know would be DROPPED rather than
    /// refused - a board would silently stop knowing which network it is on.
    #[test]
    fn an_ssid_survives_being_written_and_read_back() {
        let text = "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\nssid = \"My WiFi 2.4\"\n";
        let doc = crate::parse::document(text).expect("valid");
        let again = crate::parse::document(&crate::serialise::document(&doc)).expect("re-parses");
        assert_eq!(
            again.network()[0].ssid().map(|s| s.as_str()),
            Some("My WiFi 2.4")
        );
    }


    fn name(text: &str) -> InterfaceName {
        InterfaceName::new(text).expect("a valid fixture name")
    }

    // ---- the method vocabulary -------------------------------------------

    #[test]
    fn both_methods_are_first_class_and_spell_themselves_back() {
        for text in Method::ALL {
            let m = Method::parse(text).expect(text);
            assert_eq!(m.as_str(), *text);
            assert_eq!(m.to_string(), *text);
        }
    }

    #[test]
    fn the_method_vocabulary_is_closed_and_case_sensitive() {
        for wrong in [
            "DHCP", "Dhcp", "STATIC", "Static", "", "auto", "manual", "none", "dhcpv4", "static ",
        ] {
            assert_eq!(Method::parse(wrong), None, "{wrong:?} was accepted");
        }
    }

    /// `ALL` is what an error lists, so a method the parser accepts and `ALL`
    /// omits would tell an operator a value does not exist.
    #[test]
    fn every_method_the_parser_accepts_is_listed_in_all() {
        for m in [Method::Dhcp, Method::Static] {
            assert!(Method::ALL.contains(&m.as_str()), "{m} is not listed");
        }
        assert_eq!(Method::ALL.len(), 2);
    }

    #[test]
    fn the_rule_names_both_methods_because_a_refusal_that_omits_one_is_a_lie() {
        for m in Method::ALL {
            assert!(Method::RULE.contains(m), "{} omits {m}", Method::RULE);
        }
    }

    // ---- addressing -------------------------------------------------------

    #[test]
    fn addressing_reports_the_method_the_operator_wrote() {
        assert_eq!(Addressing::Dhcp.method(), Method::Dhcp);
        assert_eq!(
            Addressing::Static {
                address: Ipv4Cidr::parse("192.168.1.50/24").expect("valid"),
                gateway: None,
            }
            .method(),
            Method::Static
        );
    }

    /// The structural claim the module documentation makes: there is no value
    /// that is both. If someone adds a `Both` variant or an `address` field to
    /// `Dhcp`, this stops compiling — which is the point of asserting it as a
    /// match rather than as a sentence.
    #[test]
    fn no_addressing_value_can_hold_a_lease_request_and_a_static_address() {
        let exhaustive = |a: &Addressing| match a {
            Addressing::Dhcp => None,
            Addressing::Static { address, gateway } => Some((*address, *gateway)),
        };
        assert_eq!(exhaustive(&Addressing::Dhcp), None);
    }

    // ---- the plan handed to the adapter -----------------------------------

    #[test]
    fn a_static_interface_assembles_the_cores_own_plan() {
        let iface = Interface::new(name("eth0"), Addressing::Static {
                address: Ipv4Cidr::parse("192.168.1.50/24").expect("valid"),
                gateway: Some("192.168.1.1".parse().expect("literal")),
            }, None);
        let plan = iface.static_plan().expect("a static interface has a plan");
        assert_eq!(plan.interface.as_str(), "eth0");
        assert_eq!(plan.address.to_string(), "192.168.1.50/24");
        assert_eq!(plan.gateway.map(|g| g.to_string()).as_deref(), Some("192.168.1.1"));
        // And it is a plan `rivet-core` itself accepts, which is the only
        // thing that makes assembling it here safe.
        assert_eq!(plan.validate(), Ok(()));
    }

    #[test]
    fn the_plans_interface_is_the_one_the_entry_names_and_there_is_only_one() {
        // The reason `Addressing::Static` does not hold a whole
        // `StaticAddressing`: two homes for one name is two answers.
        let iface = Interface::new(name("wlan0"), Addressing::Static {
                address: Ipv4Cidr::parse("10.0.0.2/24").expect("valid"),
                gateway: None,
            }, None);
        assert_eq!(iface.name().as_str(), "wlan0");
        assert_eq!(
            iface.static_plan().map(|p| p.interface),
            Some(name("wlan0"))
        );
    }

    #[test]
    fn a_leased_interface_has_no_static_plan_rather_than_an_empty_one() {
        let iface = Interface::new(name("eth0"), Addressing::Dhcp, None);
        assert_eq!(iface.static_plan(), None);
        assert_eq!(iface.addressing().method(), Method::Dhcp);
    }

    // ---- the key lists ----------------------------------------------------

    #[test]
    fn every_static_only_key_is_also_a_declared_key() {
        // Otherwise a key could be refused as static-only *and* as unknown,
        // and which message an operator saw would depend on check order.
        for key in Interface::STATIC_ONLY_KEYS {
            assert!(
                Interface::DECLARED_KEYS.contains(key),
                "`{key}` is static-only and not declared"
            );
        }
    }

    #[test]
    fn method_is_declared_and_is_not_static_only() {
        assert!(Interface::DECLARED_KEYS.contains(&"method"));
        assert!(!Interface::STATIC_ONLY_KEYS.contains(&"method"));
    }

    /// Observed state has nowhere to go — `ADR-007` §3.3. The rule is
    /// structural: the keys simply are not declared, so a document carrying one
    /// is refused like any other unknown key. This is the guard against someone
    /// adding a convenient place to record what the kernel said.
    #[test]
    fn no_declared_key_records_what_the_machine_is() {
        for observed in [
            "carrier", "state", "link", "mac", "index", "mtu", "leased_address", "lease_expires",
            "dns", "up",
        ] {
            assert!(
                !Interface::DECLARED_KEYS.contains(&observed),
                "`{observed}` is observed state, not desired state — ADR-007 §3.3"
            );
        }
    }
}
