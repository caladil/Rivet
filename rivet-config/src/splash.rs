//! `[splash]` — what the boot screen is allowed to say.
//!
//! The maintainer's requirement, 2026-08-30 (.rulings/ R-343): the splash gets a
//! third line of text giving the IP address and port of the admin port, on by
//! default, with an option to suppress that line or the version for security
//! reasons.
//!
//! # Why this is configuration and not a build-time choice
//!
//! Because the answer differs per machine and not per image. A board on a desk
//! should say where to reach it; the same image on a board in a lobby should
//! not. `ADR-007` §3.3 makes the document the statement of what a machine
//! should be, and "what my screen says about me" is exactly that.
//!
//! # Both default to ON, and the suppression is the deliberate act
//!
//! A screen that says nothing useful is the safe default only if you think the
//! screen is a liability. It is not: a headless board whose address came from
//! DHCP is otherwise a machine nobody can find without a router's lease table.
//! **So the useful thing happens unless somebody says otherwise**, and turning
//! it off is a decision that appears in the document where it can be reviewed.
//!
//! # What the security grounds are actually worth, stated honestly
//!
//! Neither line is a secret. The address is broadcast by `rivet-mdnsd` and
//! answered by ARP; the version is in every `get machine` reply. Suppressing
//! them stops a **passer-by with a camera**, which is a real threat for a
//! screen in a public place and is the only one this addresses. It is not a
//! control over anything reachable on the network, and nothing here should be
//! read as making the machine harder to attack.

/// The service whose `enabled` decides whether a name is announced at all.
///
/// Spelled once, here: a second spelling is a second thing that can disagree
/// about which service publishes `.local` (`ADR-011`).
const MDNS_SERVICE: &str = "rivet-mdnsd";

/// What the boot screen may draw beyond the shield and the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Splash {
    endpoint: bool,
    version: bool,
}

impl Default for Splash {
    /// **Both on.** See the module documentation: the useful thing happens
    /// unless somebody says otherwise.
    fn default() -> Self {
        Self {
            endpoint: true,
            version: true,
        }
    }
}

impl Splash {
    /// The keys this table accepts.
    pub const DECLARED_KEYS: &'static [&'static str] = &["endpoint", "version"];

    /// The rule both keys obey, for a diagnostic that must not quote its input.
    pub const RULE: &'static str =
        "`true` or `false` — whether the boot screen draws that line. Both default to true, and \
         setting one to false is how a machine whose screen is somewhere public is told to keep \
         quiet about itself";

    /// Build from parts.
    #[must_use]
    pub const fn new(endpoint: bool, version: bool) -> Self {
        Self { endpoint, version }
    }

    /// Whether the screen names the address and port to reach this machine on.
    #[must_use]
    pub const fn endpoint(self) -> bool {
        self.endpoint
    }

    /// Whether the screen names the version.
    #[must_use]
    pub const fn version(self) -> bool {
        self.version
    }

    /// The name this machine can be reached by, or `None` for its address.
    ///
    /// # Both halves are required, and neither is sufficient
    ///
    /// A hostname alone gives a name **nothing resolves**: `ADR-025` puts
    /// `<hostname>` in `/etc/hosts` for the machine's own use, and that does
    /// nothing for the laptop reading the screen. `rivet-mdnsd` alone announces
    /// a name the document has not set. **Only together do they mean a person
    /// standing in front of the television can type what they see and reach the
    /// machine** — which is the entire value of the line.
    ///
    /// `.local` is `rivet-mdnsd`'s zone and is composed here rather than by the
    /// splash, because the splash draws what it is told and this is a fact about
    /// what the machine ANNOUNCES (`ADR-011`).
    ///
    /// **A name that does not resolve is worse than an address**, which is why
    /// this is conservative: absent either half, the screen shows the address it
    /// discovered, and an address always works.
    #[must_use]
    pub fn endpoint_name(document: &crate::Document) -> Option<String> {
        let hostname = document.hostname()?;
        let announced = document
            .services()
            .iter()
            .any(|s| s.name().as_str() == MDNS_SERVICE && s.enabled());
        announced.then(|| format!("{hostname}.local"))
    }

    /// The flags `cmd/rivet-initd` passes to `cmd/rivet-splash`.
    ///
    /// **Composed here rather than at the call site**, `ADR-011`: the splash
    /// binary's argument spelling is a fact, and a second place that spells it
    /// is a second place that can spell it wrong. The splash ignores arguments
    /// it does not know, so an older binary meeting a newer flag draws a plainer
    /// screen rather than refusing to draw one.
    #[must_use]
    pub fn arguments(self, port: u16, name: Option<&str>) -> Vec<String> {
        let mut out = Vec::new();
        if !self.endpoint {
            out.push("--no-endpoint".to_owned());
        }
        if !self.version {
            out.push("--no-version".to_owned());
        }
        out.push("--port".to_owned());
        out.push(port.to_string());
        // **Passed even when the endpoint line is off**, like the port and for
        // the same reason: a conditional would make the two builds differ on a
        // machine where nobody noticed, and a flag costs nothing.
        if let Some(name) = name {
            out.push("--name".to_owned());
            out.push(name.to_owned());
        }
        out
    }
}

#[cfg(test)]
#[path = "splash_tests.rs"]
mod tests;
