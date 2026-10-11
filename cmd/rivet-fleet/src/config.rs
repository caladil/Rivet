//! **This tool's own configuration** — not a machine's.
//!
//! `ADR-034` §10.3 names the trap in as many words and it is worth repeating
//! here, where the mistake would be made:
//!
//! > **Two layer stacks now exist and must not be conflated.**
//! > `ignition::config` layers **the tool's own configuration** (defaults ←
//! > file ← env ← locked). `ADR-007` §3.4 layers **a machine's desired state**
//! > (schema ← image ← fleet ← machine). They are different stacks about
//! > different things and the register is the *source* of one while being
//! > *stored under* the other.
//!
//! Everything in this file is the first stack. Nothing here is a fact about any
//! Rivet.
//!
//! # Why this schema derives serde and `rivet-config`'s does not
//!
//! `rivet-config`'s manifest records that whether the document schema carries
//! serde derives is *"the maintainer's call, not a side effect of this crate"*,
//! and that is untouched: **nothing here derives anything on a `rivet-config`
//! type.** `ignition::config` loads a consumer's own schema through serde, so
//! this crate's own struct carries the derives and the document schema does
//! not. The two are different types in different crates for exactly this
//! reason.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The name Ignition files this tool's configuration and logs under.
///
/// The binary's name, deliberately: an operator looking for where `rivet-fleet`
/// keeps its settings should find them under `rivet-fleet`.
pub const APP_NAME: &str = "rivet-fleet";

/// The register directory's name, when the configuration does not name one.
///
/// **Relative to Ignition's configuration directory**, per `ADR-034` §10.3:
/// *"the register's default location is Ignition's, not this ADR's"*. §2.2 said
/// only *"a directory on the operator's own machine"* and left it open;
/// Ignition owns the platform-correct path.
pub const DEFAULT_REGISTER_DIR: &str = "register";

/// The prefix Ignition's **environment layer** reads under.
///
/// `RIVET_FLEET_REGISTER` names a register for one invocation. This is not a
/// second answer to `ADR-034` §2.3's *named rather than discovered* — it is
/// Ignition's own layer 3, so the register is still named, just named
/// somewhere with a shorter lifetime than a file.
///
/// It is opt-in per Ignition's design, and this is the opt-in. What it buys
/// concretely: a second register can be worked on, and this tool can be
/// exercised end to end, without editing the configuration that points at the
/// operator's real fleet.
pub const ENV_PREFIX: &str = "RIVET_FLEET";

/// Load this tool's configuration through all of Ignition's layers.
///
/// **The one place the loader is built**, so that the environment layer is not
/// something a caller can forget to opt into.
#[must_use]
pub fn load() -> Config {
    ignition::config::Loader::new(APP_NAME)
        .env_prefix(ENV_PREFIX)
        .load()
}

/// `rivet-fleet`'s configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Where the register lives.
    ///
    /// `None` is not "no register" — it is *the operator has not said*, and
    /// [`Config::register_dir`] applies the default. A tool that read silence
    /// as "nowhere" would refuse to run on a fresh install.
    #[serde(default)]
    pub register: Option<PathBuf>,

    /// Ignition's, embedded exactly as its documentation prescribes.
    #[serde(default)]
    pub logging: ignition::logging::LogConfig,

    /// Ignition's, embedded exactly as its documentation prescribes.
    ///
    /// **Self-update is a feature here and a hazard on a device**, which is the
    /// whole of `ADR-034` §10.3's argument for this binary taking Ignition when
    /// every other binary in this project refused it.
    #[serde(default)]
    pub update: ignition::update::UpdateConfig,
}

impl Config {
    /// Where the register is, applying the default when nothing said.
    ///
    /// **The one place that fallback is written.** `None` reaching a caller
    /// that invented its own default would be two answers to *where is the
    /// register*, which is what `ADR-034` §2.3 rules out by requiring it be
    /// named rather than discovered.
    ///
    /// `None` from here means Ignition could not determine a configuration
    /// directory at all — a platform question, not a configuration one — and
    /// the caller refuses rather than picking somewhere.
    #[must_use]
    pub fn register_dir(&self) -> Option<PathBuf> {
        if let Some(named) = &self.register {
            return Some(named.clone());
        }
        let beside = ignition::config::config_path(APP_NAME)?;
        Some(beside.parent()?.join(DEFAULT_REGISTER_DIR))
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A tool with no configuration file must still run. `ignition::config`
    /// degrades to `Default` rather than failing, so `Default` has to be a
    /// usable state rather than an empty one.
    #[test]
    fn the_default_configuration_is_a_usable_one() {
        let config = Config::default();
        assert_eq!(config.register, None);
        // Ignition's own default is logging ON, and this crate does not
        // override it — a fleet tool that lost its own history by default
        // would be one nobody could reconstruct a provisioning from.
        assert!(config.logging.enabled);
    }

    /// An operator who names a register gets that one, unconditionally. The
    /// failure this guards is a default silently winning over a stated value,
    /// which on this particular setting means writing records into a directory
    /// nobody is looking at.
    #[test]
    fn a_named_register_is_the_one_used() {
        let config = Config {
            register: Some(PathBuf::from("/somewhere/else")),
            ..Config::default()
        };
        assert_eq!(config.register_dir(), Some(PathBuf::from("/somewhere/else")));
    }

    /// The schema round-trips through TOML, which is what `ignition::config`
    /// does to it. A field that serialises and does not deserialise is one an
    /// operator's file cannot set.
    #[test]
    fn the_schema_survives_the_round_trip_ignition_puts_it_through() {
        let config = Config {
            register: Some(PathBuf::from("/registers/home")),
            ..Config::default()
        };
        let text = toml_edit::ser::to_string_pretty(&config).expect("a serialisable schema");
        let back: Config = toml_edit::de::from_str(&text).expect("a deserialisable schema");
        assert_eq!(back, config);
    }

    /// An empty file is a valid configuration, and every key is optional.
    /// `ignition::config` layers defaults under the file, so a document that
    /// sets one key must not blank the rest.
    #[test]
    fn an_empty_document_is_a_complete_configuration() {
        let back: Config = toml_edit::de::from_str("").expect("an empty document");
        assert_eq!(back, Config::default());

        let partial: Config =
            toml_edit::de::from_str("register = \"/here\"\n").expect("one key set");
        assert_eq!(partial.register, Some(PathBuf::from("/here")));
        assert!(partial.logging.enabled, "setting one key blanked another");
    }
}
