//! **The Armada — the operator's estate, on this PC.** `ADR-059` §2, §5.
//!
//! An Armada is a collection of Fleets. This module is the PC-side registry of
//! those Fleets: their names and how to reach each Fleet's central server —
//! plus Armada-wide settings (where backups go, how many to keep). It is the
//! top of the two-level register split (`ADR-034` §2, `ADR-059` §5):
//!
//! * **This** — the Armada Registry — lists *Fleets*, and answers "which
//!   deployment am I acting on". It lives on the operator's PC.
//! * A **Fleet register** (`crate::Register`, this crate's other half) lists a
//!   Fleet's *Members*, and lives on that Fleet's server. The workstation reads
//!   it as a client; it is not a second writer.
//!
//! **Members are not here.** A `Fleet` names its server and how to reach it;
//! the members enrolled in it are read live from that server
//! (`rivet-fleetd`'s `GET /api/fleet`), never cached authoritatively on the PC.
//!
//! **The CA is not here either.** `ADR-059` §4: one root, shared across the
//! whole Armada, at `~/.rivet/ca`. A Fleet's trust is that one root, so it is
//! not stored per Fleet.
//!
//! **This is added alongside the existing device register, not in place of it**
//! (`ADR-059` §4, the rename ratchet): the code that resolves a device name
//! through `crate::store` today keeps working until it is migrated Fleet by
//! Fleet. This module is the new home the migration moves toward.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How many backups to keep when the maintainer sets no other number —
/// `ADR-059` §3, the same "keep three" the fleet repository already prunes to.
pub const DEFAULT_KEEP: u32 = 3;

/// The file name of the Armada's own settings, at the Armada Registry root.
pub const SETTINGS_FILE: &str = "armada.toml";

/// The directory of per-Fleet records, under the Armada Registry root.
pub const FLEETS_DIR: &str = "fleets";

/// The suffix of one Fleet's record file.
pub const RECORD_SUFFIX: &str = ".toml";

/// **The operator's whole estate, in memory.** `ADR-059` §2.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Armada {
    /// Where this Armada Registry lives on disk — `~/.rivet/armada` by default
    /// (`crate::armada::store::default_dir`), or an override for a test.
    pub root: PathBuf,
    /// Armada-wide settings.
    pub settings: Settings,
    /// The Fleets this operator manages, keyed and ordered by Fleet name — the
    /// order the directory lists and every rendering uses, so two runs produce
    /// the same file.
    pub fleets: BTreeMap<String, Fleet>,
}

/// **One Fleet the Armada manages.** `ADR-059` §2. Its central server and how
/// the operator reaches it. A Stand-alone is a Fleet whose server has no other
/// members (`ADR-059` §2); it is not a separate kind here.
///
/// **Whether a host is health-monitored or backed up is not here.** Rivet
/// Monitor's own per-host switches are the only authority for that (`ADR-059`
/// §6), so the Armada holds no Rivet Monitor setting.
///
/// **Unknown keys are ignored, deliberately — no `deny_unknown_fields`.** A
/// record written while the Armada still carried `monitor`/`backup` must keep
/// loading; the keys are dropped on read and are gone from the file the next
/// time it is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fleet {
    /// The Fleet's name — the handle the Armada Registry addresses it by, and
    /// the stem of its record file. Chosen at provision time (`ADR-059` §3).
    pub name: String,
    /// How to reach this Fleet's central server's control plane.
    pub server: Server,
}

/// **How to reach a Fleet's server**, over the control plane the operator
/// reaches every machine by (`ADR-025`, `ADR-035`). The operator presents its
/// own identity and the server presents the pin recorded here; the trust root
/// is the Armada's one CA (`ADR-059` §4), so it is not stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// `HOST:PORT`, or a bare `<name>.local` the workstation's own resolver
    /// answers — the same address a device record carries (`crate::Device`).
    pub host: String,
    /// The server's pinned public key, hex, as the handshake proves it
    /// (`ADR-025` §2) — the pin is the trust, exactly as the device register
    /// holds one per device.
    pub pin: String,

    /// **This fleet server's own fleet-operator public key**, hex — the
    /// per-server self-signed credential `rivet-fleetd` PRESENTS when it dials
    /// this fleet's members over the control plane
    /// (`cmd/rivet-provision/src/stage.rs`,
    /// `mint_self_signed("<fleet> fleet operator")`).
    ///
    /// **Why it belongs in the Armada.** A member authorises control-plane
    /// callers only by the keys in its own `peers` file, and a member's `peers`
    /// today names the human operator alone — never this server's fleetd key.
    /// So the fleet server can list a member but every management call is
    /// rejected at the TLS handshake (`HandshakeFailure`, `docs/PLAN.md`
    /// §183.2). Carrying the key here is what lets a member be provisioned
    /// already trusting the server that will manage it.
    ///
    /// **`Option`, defaulted, skipped when absent.** Every Armada file written
    /// before this field existed still parses, and a Fleet whose server was
    /// provisioned before the key was recorded simply has none to stage — a
    /// member joining it then behaves exactly as it did before this field
    /// existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_key: Option<String>,
}

/// **Armada-wide settings.** `ADR-059` §3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where Armada backups are written, or `None` until the maintainer names
    /// one — a directory the Armada tracks, not a path typed each time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<PathBuf>,
    /// How many backups to keep. [`DEFAULT_KEEP`] when unset.
    pub keep: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { backup_dir: None, keep: DEFAULT_KEEP }
    }
}

impl Armada {
    /// An empty Armada rooted at `root` — no Fleets, default settings.
    #[must_use]
    pub fn empty(root: PathBuf) -> Self {
        Self { root, settings: Settings::default(), fleets: BTreeMap::new() }
    }
}

pub mod store;

#[cfg(test)]
#[path = "armada_tests.rs"]
mod armada_tests;
