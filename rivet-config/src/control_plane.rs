//! `[control-plane]` — where `rivet-mcpd` listens. `ADR-035` §7.2.
//!
//! # This key exists because TLS landed, and `MCPD_LISTEN` said it would
//!
//! `cmd/rivet-sysd`'s `MCPD_LISTEN` constant carried the argument for its own
//! deletion from the day it was written:
//!
//! > **It is a constant because phase 1 accepts nothing else.** `cmd/rivet-mcpd`
//! > refuses every non-loopback address by name, since `ADR-035` §7.2's TLS is
//! > ruled and not built … An operator key offering a choice between one legal
//! > value and a refusal would be a control surface over nothing.
//! >
//! > **When TLS lands this MOVES rather than gets copied.** A non-loopback bind
//! > becomes legal, the address becomes a real decision, and it becomes a
//! > document key — at which point this constant is deleted in the same change.
//! > Adding the key while leaving this here is the two-authorities bug itself.
//!
//! TLS landed (`PLAN.md` §75) and carried a deploy (§81). So the constant is
//! gone and this is where the address lives.
//!
//! # Why the program reads it rather than the supervisor passing it
//!
//! `ADR-017` §4.1 draws a line this must not cross: *what a service IS* comes
//! from the image or a signed manifest, and **only `enabled` and a memory cap
//! are the operator's**. A supervisor that rewrote a service's argument vector
//! from the document would be letting configuration decide what a program is.
//!
//! So `cmd/rivet-mcpd` takes `--document <path>` and reads this itself, which is
//! exactly what `cmd/rivet-netd` and `cmd/rivet-mdnsd` already do — *"a path
//! says where to read a statement from, not what the statement is"*.
//!
//! # Absent means the loopback default, and that is not the same as `off`
//!
//! A machine with no `[control-plane]` table listens on [`DEFAULT_LISTEN`] —
//! the value the constant had, so every card built before this key existed
//! behaves identically. **Whether the service runs at all is
//! `[services.rivet-mcpd]`'s `enabled`**, and keeping the two apart is
//! deliberate: *where* it listens and *whether* it listens are different
//! questions, and collapsing them would make an address change look like a way
//! to turn a service off.

/// Where `rivet-mcpd` listens when the document does not say.
///
/// **Loopback, and the port is still not ruled by anything.** `ADR-035` names
/// no port; 7777 was picked on 2026-08-28 and is recorded as a default with an
/// argument rather than a decision — see `PLAN.md` §74.3.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7777";

/// The port half of [`DEFAULT_LISTEN`], for the tools that must name a port
/// without naming an address.
///
/// **Added 2026-08-29, and the reason is a measurement.** A cold agent given
/// this repository and a board found **three different control-plane ports in
/// three places**: the README's examples used `7423`, `rivet-mcpd --help` said
/// `127.0.0.1:7777`, and the board listened on 7777. A workstation tool that
/// needs the number had nowhere to read it from — [`DEFAULT_LISTEN`] is an
/// address, and a caller wanting the port was left to slice a string or to
/// write `7777` again, which is how the third spelling appears (`ADR-011`).
///
/// `the_default_address_and_the_default_port_are_the_same_number` is what keeps
/// the two from drifting; there is no way to compose one from the other in a
/// `const`, so the agreement is asserted rather than derived.
pub const DEFAULT_PORT: u16 = 7777;

/// Every interface — the value a fleet `server` or `member` binds,
/// `ADR-004` §6. **A third small copy of `cmd/rivet-provision`'s
/// `REACHABLE_LISTEN` and `cmd/rivet-fleet/src/compose.rs`'s own**, on the
/// footing `crate::fleet_role`'s module documentation argues for: one shared
/// crate for a single string constant costs more than three copies a test on
/// each side keeps in step.
pub const REACHABLE_LISTEN: &str = "0.0.0.0:7777";

/// What a `listen` value must be.
pub const LISTEN_RULE: &str = "an address and port, as `127.0.0.1:7777` or `0.0.0.0:7777` or \
                               `[::]:7777`. A NON-LOOPBACK address is accepted here and \
                               refused by rivet-mcpd unless it was also given a TLS identity \
                               (ADR-035 §7.2) - this document says where to listen and the \
                               program says what it will do without an identity.";

/// The `[control-plane]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlane {
    listen: String,
}

impl Default for ControlPlane {
    fn default() -> Self {
        Self { listen: DEFAULT_LISTEN.to_owned() }
    }
}

impl ControlPlane {
    /// Every key this table declares.
    pub const DECLARED_KEYS: &'static [&'static str] = &["listen"];

    /// Build one from a checked address.
    #[must_use]
    pub fn new(listen: String) -> Self {
        Self { listen }
    }

    /// Where to bind.
    #[must_use]
    pub fn listen(&self) -> &str {
        &self.listen
    }

    /// The port half of [`ControlPlane::listen`].
    ///
    /// **Read here rather than by whoever needs it**, `ADR-011`: the address is
    /// one string and splitting it is one operation, and a second splitter is a
    /// second thing that can disagree about where the colon is.
    ///
    /// Returns `None` only for a value that is not an address, which
    /// [`ControlPlane::is_address`] refuses at parse time — so a document that
    /// got this far has one.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.listen.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
    }

    /// Whether `text` is an address this build would bind.
    ///
    /// **Checked here rather than on the board**, which is the rule every other
    /// value in this schema follows: a document that cannot be acted on should
    /// be refused where it is written, not where it is read. A machine that
    /// accepted `listen = "yes please"` would have a control plane that failed
    /// to start with a parse error, on a board, after a reboot.
    #[must_use]
    pub fn is_address(text: &str) -> bool {
        text.parse::<std::net::SocketAddr>().is_ok()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The default is the value the deleted constant had, so a card built
    /// before this key existed behaves identically.
    #[test]
    fn the_default_is_the_constant_that_was_deleted() {
        assert_eq!(ControlPlane::default().listen(), "127.0.0.1:7777");
        assert_eq!(DEFAULT_LISTEN, "127.0.0.1:7777");
    }

    /// **One number, two constants, and no way to derive one from the other in
    /// a `const`** — so the agreement is asserted here instead.
    ///
    /// A workstation tool naming the control plane's port reads
    /// [`DEFAULT_PORT`]; a machine binding an address reads [`DEFAULT_LISTEN`].
    /// Two spellings of one fact is `ADR-011` §3, and the third spelling is
    /// what a cold agent hit on 2026-08-29 when the README said `7423`.
    #[test]
    fn the_default_address_and_the_default_port_are_the_same_number() {
        let port = DEFAULT_LISTEN
            .rsplit_once(':')
            .map(|(_, p)| p)
            .expect("the default listen address names a port");
        assert_eq!(
            port,
            DEFAULT_PORT.to_string(),
            "DEFAULT_LISTEN and DEFAULT_PORT disagree about the control plane's port"
        );
    }

    /// Every shape an operator would reasonably write, and the ones that are
    /// not addresses at all.
    #[test]
    fn an_address_is_recognised_and_anything_else_is_not() {
        for good in ["127.0.0.1:7777", "0.0.0.0:7777", "[::]:7777", "10.0.0.2:7778"] {
            assert!(ControlPlane::is_address(good), "{good} should be an address");
        }
        for bad in ["127.0.0.1", "yes please", "", ":7777", "localhost:7777", "127.0.0.1:99999"] {
            assert!(!ControlPlane::is_address(bad), "{bad} should not be an address");
        }
    }

    /// **A host NAME is refused**, and it is worth its own test because it is
    /// the mistake an operator makes: `localhost:7777` looks obviously right
    /// and this build resolves nothing — `ADR-005` §3 has no resolver on the
    /// card beyond `/etc/hosts`, and binding is not resolving anyway.
    #[test]
    fn a_host_name_is_not_an_address() {
        assert!(!ControlPlane::is_address("localhost:7777"));
        assert!(!ControlPlane::is_address("board.local:7777"));
    }
}
