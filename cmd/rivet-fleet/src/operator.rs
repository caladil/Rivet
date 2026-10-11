//! **This operator's own identity**, and the control-plane transport it opens
//! to a member.
//!
//! `ADR-034` §11.5.1 made every remote human-facing tool an MCP/TLS client, and
//! §11.5.2 named the two things in the way. This module is the second of them:
//!
//! > **`rivet-fleet` holds no operator identity.** `crate::main`'s deploy path
//! > passes `identity: None` and says so.
//!
//! It does now. (That deploy path is gone too: `PLAN-Feature-Set-2026-09-12`
//! §3, *One deployment surface*, removed `apply deploy` on 2026-09-12, and
//! `cargo rivet deploy` is how software reaches a machine.) `--identity <dir>` and `--pin <hex>` are the same pair
//! `cargo rivet run` and `cargo rivet deploy` take, read by the same code —
//! `rivet_remote::endpoint`, which was `cmd/cargo-rivet`'s own module until this
//! change moved it into the library both tools depend on. **Two spellings of
//! `--pin` is the failure `ADR-011` §3 names**, and it is avoided here by not
//! writing a second one rather than by keeping two in step.
//!
//! # One operator, one identity — and what that deliberately does not settle
//!
//! `ADR-034` §11.5.2 is explicit: *"Where the operator identity comes from for a
//! fleet of more than one machine … `ADR-025` §2.1 is open and `ADR-020`'s
//! secrets-at-rest is ruled-and-unbuilt beneath it. This section needs one
//! identity for one operator; it does not answer the fleet question."*
//!
//! So there is one `--identity` for the whole run and it is presented to every
//! machine this run reaches. A per-device operator key, a key store, and
//! anything that would keep a private key at rest are all out of scope and none
//! of them is started here.
//!
//! # The pin comes from the REGISTER, and `--pin` overrides it
//!
//! This is the one place `rivet-fleet` differs from `cargo rivet`, and the
//! difference is a fact about the register rather than a preference: `ADR-034`
//! §3.5 has the register hold the **public** half of every device key, and
//! `add identity` writes it there when it mints one. So the pin `cargo rivet`
//! makes an operator type is already recorded, and asking for it again would be
//! asking a person to copy a value out of one of this tool's own screens into
//! one of its own flags.
//!
//! `--pin` is still taken, because a machine whose identity has been replaced
//! has a record that is briefly wrong, and *the record is wrong* must be
//! survivable without editing it first.
//!
//! # ~~The PORT is the one fact that is neither typed nor recorded~~
//!
//! **Struck 2026-08-29**, rather than deleted, because a reader who finds the
//! old reasoning should see what happened to it. It said:
//!
//! > *the register has no field for a control-plane port … a device whose
//! > document moves `rivet-mcpd` to another port cannot be reached by this tool
//! > until the register can record where it listens.*
//!
//! It can now. `ADR-034` §6.1 gave §6's device record a `listen`, and
//! [`crate::register::Device::control_plane_port`] is where it is read. The
//! address is still the record's — `crate::member::address_of`, and `ADR-034`
//! §9's whole payoff — and the port is now the record's too, so **neither half
//! of `host:port` is a value this tool chose.**
//!
//! A record that says nothing still dials
//! `rivet_config::control_plane::DEFAULT_PORT`, read from the crate that owns
//! `[control-plane]` rather than written here, and **it is named on screen and
//! in every refusal** so that nobody has to guess which number was used.

use std::path::PathBuf;

use crate::register::Device;

/// What this run will present to a machine, and what it will accept back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Operator {
    /// `--identity <dir>`. **`None` is the ordinary state**, not a failure: a
    /// person listing devices or composing a document reaches no machine.
    pub identity: Option<PathBuf>,
    /// `--pin <hex>`, overriding what the register holds for the device.
    pub pin: Option<String>,
}

/// **Why a machine could not be reached**, in the words the screen shows.
///
/// A sentence rather than an enum because every one of them is read by a human
/// and none of them is branched on. The distinction that matters — *no identity
/// was given* against *the identity is unusable* — is in the text, and
/// [`Operator::refusal`] is where the first is composed.
pub type Refusal = String;

impl Operator {
    /// Read the two flags out of `clap`'s answer.
    #[must_use]
    pub fn from_matches(matches: &clap::ArgMatches) -> Self {
        Self {
            identity: crate::cli::global(matches, crate::cli::IDENTITY).map(PathBuf::from),
            pin: crate::cli::global(matches, crate::cli::PIN),
        }
    }

    /// Whether this run can reach a machine at all.
    #[must_use]
    pub const fn holds_identity(&self) -> bool {
        self.identity.is_some()
    }

    /// **The pin for one device**: `--pin` if it was given, else the register's.
    ///
    /// See the module header — the register is the ordinary source and the flag
    /// is the override, which is the reverse of `cargo rivet`'s arrangement
    /// because the register is a thing `cargo rivet` does not have.
    #[must_use]
    pub fn pin_for<'a>(&'a self, device: &'a Device) -> Option<&'a str> {
        self.pin
            .as_deref()
            .or(device.pin.as_deref())
            .filter(|pin| !pin.is_empty())
    }

    /// **The three flags, filled from this run and this device's record.**
    ///
    /// Composed rather than opened, so that the values can be checked — the
    /// address the record resolves, the identity and the pin — without opening
    /// a connection; [`Self::open`] is the one caller that goes on to connect.
    ///
    /// # Errors
    ///
    /// The record's own refusal when it names no address, or more than one.
    pub fn endpoint_for(
        &self,
        device: &Device,
    ) -> Result<rivet_remote::endpoint::Endpoint, Refusal> {
        Ok(rivet_remote::endpoint::Endpoint {
            host: Some(control_plane_host(device)?),
            identity: self.identity.clone(),
            pin: self.pin_for(device).map(str::to_owned),
        })
    }

    /// **The transport, or the sentence saying why there is none.**
    ///
    /// `Err` in every case rather than `Ok(None)`: this tool's callers all want
    /// to say something to a person when a machine cannot be reached, and an
    /// `Option` would make *no identity was given* into a case each of them
    /// composed its own words for.
    ///
    /// # Errors
    ///
    /// No identity, no pin, an unreadable identity, an address the record
    /// cannot supply, or a pin that is not 64 hex characters — all before
    /// anything is connected to.
    pub fn open(&self, device: &Device) -> Result<rivet_remote::tls::Tls, Refusal> {
        if !self.holds_identity() {
            return Err(self.refusal());
        }
        let endpoint = self.endpoint_for(device)?;
        if endpoint.pin.is_none() {
            return Err(format!(
                "{name} has no control-plane key in the register and none was given, so there \
                 is nothing to pin and this end will not trust whatever answers at that \
                 address. ADR-025 §2 makes the pin the whole of the trust model, and ADR-034 \
                 §6.1 makes an absent one a refusal rather than a default - there is no \
                 sensible default for a pin.\n\
                 \x20 fix  `set pin --device {name} --pin <hex>` RECORDS a key this machine \
                 already has, which is the case when its identity was minted by a card build \
                 (`cargo rivet identity new`) rather than by this tool.\n\
                 \x20 or   `add identity --device {name} --out <dir>` MINTS a new one and \
                 records the public half - but that replaces the machine's identity, so it \
                 needs a new card and locks out every peer pinning the old key.\n\
                 \x20 or   --pin <hex> for one run, without recording it.",
                name = device.name
            ));
        }
        endpoint.paired()?;
        endpoint.open()?.ok_or_else(|| self.refusal())
    }

    /// **The words for a run that holds no identity.**
    ///
    /// One sentence, in one place, so that the monitor pane and a refused
    /// operation say the same thing — `AGENTS.md` §5's *report what the system
    /// said*, applied to this end's own state.
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        "rivet-fleet was started without --identity, so it holds no control-plane identity and \
         can reach no machine. ADR-035 leaves the image with no other route in, so \
         nothing has been sent.\n\
         \x20 fix  `cargo rivet identity new --directory <dir>` mints an operator identity, \
         and that machine must accept its public key — `add peer` records one, `apply \
         document` puts it on the board. Then start with `--identity <dir>`."
            .to_owned()
    }
}

/// **`<address>:<port>`, as the control plane takes it.**
///
/// Both halves are the record's — `ADR-034` §9's *"resolving address, access and
/// trust from the record"* for the address, and §6.1's `listen` for the port.
/// See the module header for what this replaced.
///
/// # Errors
///
/// `crate::member::address_of`'s refusal, unchanged.
pub fn control_plane_host(device: &Device) -> Result<String, Refusal> {
    Ok(format!(
        "{}:{}",
        crate::member::address_of(device)?,
        device.control_plane_port()
    ))
}

#[cfg(test)]
#[path = "operator_tests.rs"]
mod tests;
