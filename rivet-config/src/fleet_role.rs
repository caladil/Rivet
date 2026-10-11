//! `fleet-role` — **which of `SPEC-05` §5's two roles this machine is**,
//! as a real document key. Added 2026-09-13, `docs/PLAN-Feature-Set-2026-09-12.md`
//! §5.2 item 2 (`.rulings/` R-162, Wave 3a).
//!
//! # Why this is a second copy of `rivetos_register::FleetRole`, and why that
//! is accepted rather than fixed
//!
//! `cmd/rivet-fleet/src/compose.rs` already carries this exact three-word
//! vocabulary, and its own header says why a second copy is not `ADR-011` §3
//! broken by neglect: *"kept here as a second, small copy"* of
//! `cmd/rivet-provision`'s `listen_for`/`services_for`. That module's
//! `role_listen_default` and `role_services` are the identical mapping this
//! module publishes, for the register's `Device` rather than for
//! `rivet-config`'s `Document`.
//!
//! The alternative — one crate depending on the other so there is a single
//! `FleetRole` — is refused by `ADR-014` §3's isolation rule: `rivet-config`
//! is the schema crate for the machine document, `rivet-register` is the
//! fleet's own persistence, and neither is an adapter over the other. A third
//! crate holding one shared enum was considered and rejected for the same
//! reason `cmd/rivet-fleet/src/compose.rs` gives for its own copy: the cost of
//! a shared crate for one enum and three string constants is higher than the
//! cost of the two copies staying in step, which
//! `a_fleet_role_is_exactly_one_of_three_words`-shaped tests on both sides
//! make cheap to notice if they ever do not.
//!
//! # What this key composes, and what it does not
//!
//! **A pure default, never an assertion.** `SPEC-05` §5 already rules how this
//! resolves when it and an explicit key disagree, for the register's own
//! `Device::fleet_role`: *"only when the device's own listen is absent, since
//! the register's merge rule is the device wins."* This module extends that
//! same rule to the document itself rather than inventing a second one:
//! [`crate::document::Document::effective_control_plane`] consults
//! [`Document::fleet_role`](crate::document::Document::fleet_role) only when
//! `[control-plane]` is absent, and [`role_services`] is a list callers may
//! layer under `[services.<name>].enabled` the identical way
//! `cmd/rivet-fleet/src/compose.rs::services` already layers its own copy —
//! never a refusal, because a default that only fills a gap cannot disagree
//! with a key that fills it explicitly (`ADR-050` §5, `.rulings/` R-159 R21).
//!
//! **What is deliberately NOT built here:** `[applications.rivet-proxyd]
//! upstream-name`. `SPEC-05` §5 requires it wherever `rivet-proxyd` is
//! enabled, but deriving it needs the fleet authority's own certificate —
//! I/O this crate is forbidden from performing (`ADR-014` §3). Both
//! `cmd/rivet-provision` (from the CA on disk) and `cmd/rivet-fleet`'s
//! `Device::applications` (via `add document`'s `adopt`) already supply it
//! from outside this crate, and a document whose role enables the proxy with
//! no `upstream-name` is accepted here exactly as it always has been —
//! closing that gap is a caller's job, not a schema one.

/// The identical vocabulary `cmd/rivet-provision`'s `Fleet` question and
/// `rivetos_register::FleetRole` use, so that a value round-tripping through
/// the register, the interview and this document is one word rather than
/// several spellings of one fact (`ADR-011`).
///
/// **Two roles, not three.** A machine that is neither a server nor a member
/// is `Option::None` — there is no `StandAlone` variant, because one never
/// named a behaviour [`listen_default`] or [`role_services`] gave a role that
/// `None` did not already give. Removed as a ratchet increment,
/// `rivetos_register::FleetRole`'s own copy having gone first: a document may
/// still carry the retired `"stand-alone"` spelling, and `crate::parse`'s
/// `read_fleet_role` is what keeps such a document loading, as `None`, rather
/// than refusing it. **`ADR-059` §8, 2026-09-22: [`Self::ALL`] dropped the
/// word too** — the provision interview's Fleet menu now offers only
/// `{server, member}`, generated from `ALL` the same way it always was; a
/// legacy document is still read exactly as before, because that path never
/// went through `ALL` or `parse` to begin with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetRole {
    /// Runs `rivet-fleetd`, `rivet-proxyd` and `rivet-mdnsd` by default —
    /// `ADR-050` §5.
    Server,
    /// Runs `rivet-proxyd` and `rivet-mdnsd` by default, reached by name by
    /// the server that manages it (`.rulings/` R-159 R21).
    Member,
}

impl FleetRole {
    /// The two words, in the order `cmd/rivet-provision`'s `Fleet` question
    /// offers them. **`"stand-alone"` dropped 2026-09-22** (`ADR-059` §8) —
    /// see this type's own doc comment; a document already carrying that
    /// spelling still loads, as `None`, through `crate::parse::read_fleet_role`
    /// alone, which never consults `ALL`.
    pub const ALL: &'static [&'static str] = &["server", "member"];

    /// What a value must be — quoted by the parser's own refusal, so the
    /// schema and the error an operator gets cannot describe this value
    /// differently (`ADR-006` §7).
    pub const RULE: &'static str = "one of \"server\" or \"member\"";

    /// Parse the word an operator wrote. Exactly these two spellings —
    /// `.rulings/` R-159 R21's sibling test on the register side is
    /// `a_fleet_role_is_exactly_one_of_two_words`, and this is its document
    /// twin. The retired `"stand-alone"` spelling is refused here exactly
    /// like any other unrecognised word: it named no role of its own even
    /// when a `StandAlone` variant existed, so this is not where a document
    /// that already carries it is kept loading — see
    /// `crate::parse::read_fleet_role` for that, separate, case.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "server" => Some(Self::Server),
            "member" => Some(Self::Member),
            _ => None,
        }
    }

    /// The word this renders back.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Member => "member",
        }
    }
}

impl core::fmt::Display for FleetRole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Every interface this document's `[control-plane]` should bind **when the
/// document says nothing else** — `ADR-004` §6, mirrored from
/// `cmd/rivet-provision::listen_for` and `cmd/rivet-fleet::compose::role_listen_default`.
///
/// `None` leaves [`crate::control_plane::DEFAULT_LISTEN`] — loopback — as the
/// answer, which is the schema's own fallback and stays loopback for the
/// reason `ADR-004` §6 gives: a machine whose stored document cannot be
/// read must not start answering the network.
#[must_use]
pub const fn listen_default(role: Option<FleetRole>) -> Option<&'static str> {
    match role {
        Some(FleetRole::Server | FleetRole::Member) => {
            Some(crate::control_plane::REACHABLE_LISTEN)
        }
        None => None,
    }
}

/// Every service this document's role turns on **when the document says
/// nothing else about that service** — `ADR-050` §5's role table, mirrored
/// from `cmd/rivet-fleet/src/compose.rs::role_services`.
///
/// Sorted, so two documents naming the same role produce the same order
/// however a caller iterates it (`ADR-007` §3).
#[must_use]
pub const fn role_services(role: Option<FleetRole>) -> &'static [&'static str] {
    match role {
        Some(FleetRole::Server) => &["rivet-fleetd", "rivet-mdnsd", "rivet-proxyd"],
        Some(FleetRole::Member) => &["rivet-mdnsd", "rivet-proxyd"],
        None => &[],
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **`ADR-059` §8: the menu offers `{server, member}` and nothing else.**
    /// `"stand-alone"` is gone from both the offered words and the rule text
    /// an operator's refusal quotes — a document already carrying the retired
    /// spelling is the separate, still-honoured case `crate::parse`'s own
    /// fleet-role tests cover, which never reads `ALL` or `RULE` at all.
    #[test]
    fn all_and_rule_offer_exactly_the_two_live_roles() {
        assert_eq!(FleetRole::ALL, &["server", "member"]);
        assert_eq!(FleetRole::RULE, "one of \"server\" or \"member\"");
        assert!(!FleetRole::ALL.contains(&"stand-alone"), "{:?}", FleetRole::ALL);
        assert!(!FleetRole::RULE.contains("stand-alone"), "{}", FleetRole::RULE);
    }

    /// **Exactly two words, and nothing guessed for a third.** `"stand-alone"`
    /// is one such refused word now: it named no role of its own, so it is no
    /// longer offered as one going forward through this function — a document
    /// that already carries it on disk is the separate, still-honoured case
    /// `crate::parse`'s own fleet-role tests cover.
    #[test]
    fn a_fleet_role_is_exactly_one_of_two_words() {
        assert_eq!(FleetRole::parse("server"), Some(FleetRole::Server));
        assert_eq!(FleetRole::parse("member"), Some(FleetRole::Member));
        assert_eq!(FleetRole::parse("stand-alone"), None);
        assert_eq!(FleetRole::parse("standalone"), None);
        assert_eq!(FleetRole::parse("Server"), None);
        assert_eq!(FleetRole::parse(""), None);
    }

    #[test]
    fn every_role_round_trips_through_its_own_spelling() {
        for role in [FleetRole::Server, FleetRole::Member] {
            assert_eq!(FleetRole::parse(role.as_str()), Some(role), "{role} does not round-trip");
        }
    }

    /// `ADR-050` §5: a server carries `fleetd`; a server or a member carries
    /// `proxyd` and `mdnsd`; an absent role carries nothing.
    #[test]
    fn role_services_matches_adr_050() {
        assert_eq!(
            role_services(Some(FleetRole::Server)),
            &["rivet-fleetd", "rivet-mdnsd", "rivet-proxyd"]
        );
        assert_eq!(role_services(Some(FleetRole::Member)), &["rivet-mdnsd", "rivet-proxyd"]);
        assert!(role_services(None).is_empty());
    }

    /// `ADR-004` §6: a server or a member is administered from elsewhere by
    /// definition and binds every interface; a machine with no role stated
    /// gets no role default at all.
    #[test]
    fn listen_default_matches_adr_004_6() {
        assert_eq!(
            listen_default(Some(FleetRole::Server)),
            Some(crate::control_plane::REACHABLE_LISTEN)
        );
        assert_eq!(
            listen_default(Some(FleetRole::Member)),
            Some(crate::control_plane::REACHABLE_LISTEN)
        );
        assert_eq!(listen_default(None), None);
    }
}
