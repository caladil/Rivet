//! **The register** — `ADR-034` §2's store, and the authority.
//!
//! ```text
//! <register>/register.toml            the fleet itself: what every machine shares
//! <register>/devices/<id>.toml        one file per device, filed by ADR-025 §4.1's
//!                                     opaque identifier
//! <register>/credentials.toml         §7's store. NOT here — see crate::credentials
//! ```
//!
//! # Filed by identifier, labelled by name
//!
//! `ADR-034` §10.2, and the reason is `ADR-025` §4.1's:
//!
//! > **One file per device, named by the opaque identifier**, with the human
//! > label as a field inside it. `ADR-025` §4.1 makes the identifier
//! > authoritative and the name a mutable label, so a register that filed
//! > devices by *name* would rewrite history on a rename and break every
//! > reference to the device. The directory listing is ugly and no human reads
//! > it — the interface shows names.
//!
//! So [`Device::id`] is the file name and [`Device::name`] is a field, and
//! [`Register::resolve`] accepts either — because an operator types the label
//! and an agent may hold the identifier.
//!
//! **An ambiguous label is refused, never guessed.** Two devices may carry one
//! label, because a label is mutable and nothing stops a mistake; picking the
//! first would silently provision the wrong machine, which is the one failure
//! in this tool that is expensive to undo.
//!
//! # What is here and what is deliberately not
//!
//! **No secret.** `ADR-034` §3 is the whole of it: the register holds no
//! signing key and no device private key. §3.4 is what that buys — *"the
//! public register stays committable, reviewable and agent-readable"* — and
//! `crate::credentials` is the separate file §7.4 requires for the one secret
//! that is allowed anywhere near this tool.
//!
//! # Good to commit, without the tool running `git`
//!
//! `ADR-034` §10.3 answered §8 item 4: version control is the operator's, and
//! *"the register's job is to write files that are good to commit — stable
//! ordering, no churning timestamps, one device per file."* All three are
//! properties of this module and all three are asserted below.
/// **The register's I/O.** Split out because `ADR-014` §3 puts I/O at the
/// composition root, so every property the files must have is asserted without
/// a filesystem and this is the thin half that remains.
pub mod store;

/// **The Armada — the PC-side registry of Fleets**, the top of the two-level
/// register (`ADR-059` §5). This crate's other half holds one Fleet's members;
/// this holds the operator's Fleets.
pub mod armada;

// **Phones** — `ADR-061` §5. One file per phone under `phones/`, beside the
// devices, reached through its own functions rather than through `Register`, so
// no caller of `Register` can write or drop one. A comment and not a doc
// comment: a `///` here would merge with the module's own header and resolve
// its links in this file's scope.
pub mod phones;


use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The fleet-wide file's name.
pub const REGISTER_FILE: &str = "register.toml";

/// The directory device records live in, under the register root.
pub const DEVICES_DIR: &str = "devices";

/// The suffix every device record carries.
pub const RECORD_SUFFIX: &str = ".toml";

/// How long an opaque identifier is, in characters — **`rivet_core`'s.**
///
/// The grammar moved out of this module on 2026-08-26, on `AccountName`'s terms
/// exactly: the day the second consumer appeared. `rivet-config` has to *refuse*
/// an identifier a document carries — `[repos]` gave a repository one, because
/// `ADR-025` §4.1's *"the name is a mutable label"* is not a fact about devices
/// — and it cannot depend on a binary crate.
///
/// **One grammar and two subjects, not two grammars.** Two spellings of the
/// rule would eventually differ, and the first sign of it would be a register
/// that accepted an id a document refused.
pub const ID_CHARS: usize = rivet_core::identifier::CHARS;

/// One machine, as the register records it.
///
/// The fields are `ADR-034` §6's table, minus the two rows that are not the
/// register's to hold: the credential store (§7.4, a separate file) and history
/// (`ADR-028` §4.5), which is appended to rather than replaced and is therefore
/// not a field of a record this tool rewrites.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// `ADR-025` §4.1's opaque stable identifier. **The file name**, and the
    /// only field that never changes.
    pub id: String,
    /// The human label. Mutable, and not unique by construction.
    pub name: String,
    /// What this machine is for, in one word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Where it physically is, for a human reading the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// **This machine's own control-plane public key — the PIN** this tool
    /// presents to `--pin`. `ADR-025` §2, `ADR-034` §3.5 and §6.1.
    ///
    /// Sixty-four hex characters, and nothing else. The private half is blanked
    /// at first boot (`ADR-019` §3.1) and never reaches here; §3.6 is why the
    /// public half costs nothing to hold.
    ///
    /// # ~~`public_key`~~ — renamed 2026-08-29, and the old spelling still reads
    ///
    /// The field was called `public_key` and its own documentation said *"the
    /// device's public key, and its key id"*, which named two things and
    /// committed to neither. Beside [`Self::control_plane_keys`] — hex too, but
    /// the keys this machine ACCEPTS rather than the one it PRESENTS — a
    /// key-ish field with a generic name was read wrongly by this crate's own
    /// test, which put a whole line of base64 in it.
    ///
    /// `serde(alias)` keeps every register already on disk loading unchanged;
    /// the next save writes the new spelling, which is a one-line diff in a file
    /// that is committed and reviewed.
    ///
    /// # Absent is a REFUSAL, never a default
    ///
    /// [`Self::listen`] has a sensible default and this has none: connecting
    /// without a pin would mean trusting whatever answered at that address,
    /// which is the single failure `ADR-025` §2's trust model exists to
    /// prevent. `crate::operator::Operator::open` refuses by name and says how
    /// to record one — `add identity` when this tool is minting, `set pin` when
    /// the identity was minted elsewhere, which is what a card build does.
    #[serde(default, alias = "public_key", skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,

    /// Ed25519 public keys this device's CONTROL PLANE accepts, in addition to
    /// the fleet's — `ADR-025` §2, `ADR-035` §7.2.
    ///
    /// **Hex, sixty-four characters, and nothing else.** `ADR-025` §2.2 forbids
    /// coupling the device identity to any other key material the machine
    /// happens to carry, and the same separation applies to who may use each
    /// channel: what may reach the control plane is its own list, with its own
    /// lifetime, and that is the point.
    ///
    /// `crate::compose::peers` layers these under the fleet's and writes the
    /// `peers` file `cmd/rivet-mcpd` reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_plane_keys: Vec<String>,

    /// **If this device is a fleet SERVER, its own fleet-operator public key**,
    /// hex — the per-server self-signed credential `rivet-fleetd` PRESENTS when
    /// it dials this fleet's members (`cmd/rivet-provision/src/stage.rs`,
    /// `mint_self_signed("<fleet> fleet operator")`).
    ///
    /// **This is not [`Self::pin`], and not [`Self::control_plane_keys`].** The
    /// pin is the key this machine's *own* control plane is reached BY; this is
    /// the key this machine's `rivet-fleetd` reaches OTHER machines WITH. A
    /// member never carries one (it runs no `rivet-fleetd`), which is how a
    /// member record is told apart from a server's here.
    ///
    /// **Why the register holds it at all.** The armada records each fleet by
    /// bootstrapping the server's device record
    /// (`cmd/cargo-rivet/src/armada.rs::fleet_from_device`), and a member being
    /// provisioned into that fleet must be staged already trusting this key or
    /// the server can list it but never manage it (`HandshakeFailure`,
    /// `docs/PLAN.md` §183.2). Recording it on the server's own record is what
    /// carries it from the moment the server was provisioned to the moment a
    /// member joins.
    ///
    /// **Absent is the norm for every record written before this field, and for
    /// every member** — `Option`, defaulted, skipped when absent, so those
    /// records compose exactly the bytes they composed yesterday.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_key: Option<String>,

    /// **Where `rivet-mcpd` listens on this machine** — `ADR-034` §6.1.
    ///
    /// The document's `[control-plane]` `listen` value and nothing else: an
    /// address and a port, as `rivet_config::control_plane::LISTEN_RULE` states
    /// it. `crate::compose` writes it into the document from here, which is
    /// what makes this one value rather than two.
    ///
    /// # Why the register holds it at all
    ///
    /// `ADR-034` §9's payoff is that a member's address is *resolved from the
    /// record* rather than typed, so there is no flag that could tell this tool
    /// a machine had moved its listener — **a device whose document moved
    /// `rivet-mcpd` could not be reached by `rivet-fleet` at all**, which is
    /// what §11.5.2.1 found and §6.1 ruled.
    ///
    /// # Absent means the default, and the two are not distinguished
    ///
    /// §6.1 left this open — *"an absent field and a field equal to the default
    /// are different states and nobody has said whether they should be"* — and
    /// **this build takes the simple reading: they are the same state.** Absent
    /// composes no `[control-plane]` table at all, so a machine reads
    /// `rivet_config::control_plane::DEFAULT_LISTEN` for itself, and this tool
    /// dials `rivet_config::control_plane::DEFAULT_PORT`. Every register
    /// written before this field existed therefore composes the same bytes it
    /// composed yesterday, which matters more than the distinction would: an
    /// `apply document` compares the bytes it sent to the bytes read back.
    ///
    /// **What it costs to read it that way**, said out loud: an operator cannot
    /// record *"this machine is deliberately on the default"* as distinct from
    /// *"nobody has said"*. Nothing in this tool reads that difference today,
    /// and inventing a state for it before something does would be a key with
    /// nothing behind it.
    ///
    /// # The catch a reader should know about
    ///
    /// [`DEFAULT_LISTEN`] is **loopback**. A machine composed with no `listen`
    /// here listens on `127.0.0.1:7777` and this tool cannot reach it from a
    /// workstation at all — which is the state every register was in before
    /// this field existed. Recording a routable `listen` is what makes the
    /// control plane usable, and `set listen` is the operation that does it.
    ///
    /// [`DEFAULT_LISTEN`]: rivet_config::control_plane::DEFAULT_LISTEN
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,

    /// Which signing key ids this device trusts — `ADR-028` §5.5.
    ///
    /// A key *id*, never a key: an id names a trust relationship and confers
    /// nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusts: Vec<String>,

    /// Addressing per interface — `ADR-004` §6.1.
    ///
    /// A `BTreeMap` rather than a `HashMap`, and that is not a taste: a
    /// register that rendered its interfaces in hash order would produce a
    /// different file every run and be useless to commit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub interfaces: BTreeMap<String, Interface>,

    /// This device's resolver — `ADR-034` §6, added 2026-09-09.
    ///
    /// **A field of its own, never an entry of [`Self::interfaces`].** See
    /// [`Resolver`] for why: modelling a resolver as an interface is the
    /// defect this field exists to retire, and giving it nowhere to hide is
    /// the whole of the fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver: Option<Resolver>,

    /// The wireless network to join, by SSID. The credential itself is
    /// `crate::credentials`' and is not here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wireless: Option<String>,

    /// Which services this machine runs — `ADR-034` §5, *"the register's
    /// everyday operation"*.
    ///
    /// Ordered, for the same reason the interfaces are.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, bool>,

    /// A cap on how much memory a service on this machine may have — the
    /// operator's half of `ADR-007` §7.2, keyed by service name.
    ///
    /// **A second map rather than a richer value in [`Self::services`], and
    /// that is a decision.** `rivet_core::service`'s own module documentation
    /// draws the line this follows: *"the operator says whether a service
    /// runs"* and *"the operator limits"* are two different questions, one
    /// answered against `ADR-017` §6's control surface and the other against
    /// `ADR-015` §5.1's resource limits. `enabled` is also the whole of what
    /// the TUI edits and what every existing register on disk contains, so
    /// widening it would migrate a file for a fact that is not the same fact.
    ///
    /// **Stored as the operator's own text** — `32M`, not `33554432` — because
    /// that is what `ADR-007` §7.2's document takes and what
    /// `rivet_core::service::MemoryLimit` parses. It is validated on the way
    /// IN, by the `set` that records it, so nothing unparseable can reach a
    /// composed document.
    ///
    /// A name here that is absent from [`Self::services`] cannot happen: the
    /// only way to record one is `rivet-fleet set service`, which requires
    /// `enabled` in the same breath.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub service_memory: BTreeMap<String, String>,

    /// Which git repositories this machine serves — name to identifier.
    ///
    /// **Keyed by name, valued by identifier, and that is the wrong way round
    /// on purpose.** `ADR-025` §4.1 makes the identifier authoritative, so the
    /// obvious shape is id-to-name — but this map has to *render* as
    /// `[repos.<name>]`, and a rename has to be one edit an operator can make
    /// by hand in a file he can read. Keying by name makes the rename visible
    /// in a diff as a moved key with an unchanged value, which is exactly what
    /// happened; keying by id would render the same diff as two changed lines
    /// with the meaning buried in the values.
    ///
    /// The uniqueness the identifier needs is checked at compose time, by the
    /// parser, which is where every other document rule is enforced.
    ///
    /// Ordered, for the interfaces' reason: a register that rendered in hash
    /// order would produce a different file every run and be useless to commit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub repos: BTreeMap<String, String>,

    /// Whether the terminal interface keeps this device out of the way.
    ///
    /// **Hidden is not removed, and that is the point.** On 2026-08-27 the
    /// maintainer asked for a way to delete a device that would not really
    /// delete it but hide it thoroughly (`.rulings/` R-327), and that is right
    /// for a reason `ADR-025` §4.1 already gives: the identifier is
    /// authoritative, this record is the only place it exists, and deleting it
    /// loses the one fact that cannot be reconstructed. A later device of the
    /// same name would be a DIFFERENT device wearing an old label.
    ///
    /// So a hidden device keeps its identifier, its keys and its history, and
    /// stops appearing in a list. `remove device` still deletes and stays where
    /// it is — at the command line, typed on purpose (`ADR-012` §7.1).
    ///
    /// Absent means visible, which is why every record written before this
    /// field existed still reads.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,

    /// Which of `SPEC-05` §5's roles this machine's document should default
    /// from — `ADR-004` §6.2.1.
    ///
    /// **Not [`Self::role`].** That field is a free label for a human reading
    /// the list — `"photo server"`, `"camera"` — and was never the controlled
    /// vocabulary `cmd/rivet-provision`'s own `Fleet` question offers. This is
    /// that same question, asked of the register: `crate::compose` (in
    /// `cmd/rivet-fleet`) reads it to decide whether a control plane binds
    /// every interface or keeps loopback, and which services a role turns on
    /// by default, the same way `rivet-provision`'s menu already does. Two
    /// tools answering *"what does a server carry"* from two different fields
    /// would eventually disagree, which is `ADR-011` §3.
    ///
    /// **Absent applies no role default at all**, never a guess: every record
    /// written before this field existed still composes exactly the bytes it
    /// composed yesterday, and a machine whose role nobody has stated gets
    /// nothing invented on its behalf (`AGENTS.md` §5 — a plausible guess that
    /// is wrong does not fail, it succeeds and is acted on).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_fleet_role"
    )]
    pub fleet_role: Option<FleetRole>,

    /// This machine's timezone — `SPEC-05` §5, ruled 2026-09-07.
    ///
    /// The IANA name `rivet_config`'s `timezone` top-level key takes, exactly
    /// as `rivet-provision`'s menu records it.
    ///
    /// **Absent is never defaulted, here or anywhere downstream.** A machine
    /// provisioned without one keeps UTC **silently** — `ADR-021` §9 — so a
    /// plausible default would be exactly the wrong answer: cheap to compose
    /// and expensive to discover missing, because every timestamp on the
    /// machine is wrong including the ones that would be used to find the
    /// bug. `crate::compose::provisioning_complete` (in `cmd/rivet-fleet`)
    /// refuses to produce a FINAL document without it, naming the gap rather
    /// than composing a document that keeps it quiet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,

    /// **Per-application settings**, keyed by application name and then by
    /// key — `SPEC-05` §5, `ADR-038`.
    ///
    /// `[applications.<name>]` on the running machine, captured the way
    /// [`Self::interfaces`] already is: this crate holds whatever a device's
    /// own record says, `crate::compose` (in `cmd/rivet-fleet`) renders it
    /// back out, and `add document`'s `adopt` is what fills it in from a
    /// machine that already carries the value.
    ///
    /// **`rivet-proxyd`'s `upstream-name` is the one entry `SPEC-05` §5 gates
    /// on.** A document that enables the proxy and omits it is worse than one
    /// that does not enable the proxy at all — `cmd/rivet-proxyd` has no
    /// default and derives nothing, so the process exits, retries five times
    /// and is marked failed — and `crate::compose::provisioning_complete`
    /// refuses to produce a FINAL document without it rather than composing
    /// one that boots to a dead front door. Nothing else this field carries is
    /// gated the same way: TempMon's `sensor-labels` and `write-token` are ordinary
    /// examples of the same shape, ruled by `ADR-038` to be settings and not
    /// secrets — they live in the document already, in plain text, so the
    /// register holding them too is the same platform decision applied to a
    /// second place a document's bytes end up, not a new exposure.
    ///
    /// Ordered maps, for [`Self::interfaces`]' reason: a register that
    /// rendered in hash order would produce a different file every run and be
    /// useless to commit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applications: BTreeMap<String, BTreeMap<String, String>>,

    /// **Which application takes this device's display** — `SPEC-05` §5.
    ///
    /// `[kiosk] auto-run` on the running machine, captured the way
    /// [`Self::timezone`] already is: this crate holds whatever a device's
    /// own record says, `crate::compose` (in `cmd/rivet-fleet`) renders it
    /// back out as `[kiosk] auto-run = "…"`, and `add document`'s `adopt` is
    /// what fills it in from a machine that already answers it.
    ///
    /// **Absent writes no `[kiosk]` table at all**, [`Self::timezone`]'s rule
    /// exactly: every register written before this field existed goes on
    /// composing the same bytes it composed yesterday.
    ///
    /// **Not a completeness gate, and that is a decision this field does not
    /// get to make.** `SPEC-05` §5 names, on purpose, that nothing at the
    /// moment a document is composed can know whether the application this
    /// names will ever be installed — the manifest that would say so is
    /// written by an install that happens afterwards. So a value here is
    /// recorded and composed with nothing checked, and a name matching
    /// nothing takes the display and draws nothing on it. Closing that gap is
    /// a different piece of work than this field is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kiosk_auto_run: Option<String>,

    /// **Which published version of a carried product THIS device's card must
    /// carry** — `rivet_config::carry`, `.rulings/` R-160 R26.
    ///
    /// `[carry.<product>]` on the composed document, keyed by product name and
    /// valued by the version string `cargo rivet fleet publish` recorded. The
    /// same two-layer merge [`Self::service_memory`] gets and for the same
    /// reason (`ADR-007` §3): a device's own pin wins over the fleet's, per
    /// product rather than per table — `crate::compose::carry` (in
    /// `cmd/rivet-fleet`) is where the merge happens. **No role layer sits
    /// beneath it**, unlike [`Self::services`]: no role knows a version
    /// (`ADR-050` §5), so there is no role default for a fleet-wide pin to
    /// fall through to.
    ///
    /// Ordered, for [`Self::interfaces`]' reason: a register that rendered in
    /// hash order would produce a different file every run and be useless to
    /// commit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub carry: BTreeMap<String, String>,

    /// **Which system update each of this machine's A/B slots holds**, by the
    /// fleet repository's digest of it — `ADR-052` §7.
    ///
    /// The fleet repository's prune never removes a digest named here, so the
    /// build a machine runs and the build its other slot rolls back to both
    /// stay published. `cargo rivet fleet record` writes a slot's entry only
    /// while the machine reports that slot running and committed, so nothing
    /// here names a build that has not committed on this machine.
    ///
    /// **A record only, never composed.** `crate::compose` (in
    /// `cmd/rivet-fleet`) renders nothing from it, so it never reaches a
    /// machine's document or that document's strict parser.
    ///
    /// Absent writes no table, so every record without one renders the bytes
    /// it rendered before the field existed.
    #[serde(default, skip_serializing_if = "SystemSlots::is_empty")]
    pub system_slots: SystemSlots,
}

/// [`Device::system_slots`]: the digest of the system update in slot `a` and
/// in slot `b`, each absent until something records it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSlots {
    /// Slot `a`'s system update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    /// Slot `b`'s system update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
}

impl SystemSlots {
    /// Whether neither slot is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.a.is_none() && self.b.is_none()
    }

    /// Every digest recorded, slot `a`'s first, each beside its slot.
    #[must_use]
    pub fn digests(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        if let Some(digest) = &self.a {
            out.push(("a", digest.as_str()));
        }
        if let Some(digest) = &self.b {
            out.push(("b", digest.as_str()));
        }
        out
    }

    /// The digest recorded for `slot`, if any.
    #[must_use]
    pub fn get(&self, slot: &str) -> Option<&str> {
        match slot {
            "a" => self.a.as_deref(),
            "b" => self.b.as_deref(),
            _ => None,
        }
    }

    /// Record `digest` against `slot` (`a` or `b`), leaving the other slot's
    /// entry as it was.
    ///
    /// # Errors
    ///
    /// `slot` names neither slot.
    pub fn set(&mut self, slot: &str, digest: &str) -> Result<(), String> {
        match slot {
            "a" => self.a = Some(digest.to_owned()),
            "b" => self.b = Some(digest.to_owned()),
            other => return Err(format!("{other:?} is not an A/B slot; a slot is `a` or `b`")),
        }
        Ok(())
    }
}

/// `SPEC-05` §5's roles, and `cmd/rivet-provision`'s own `Fleet` question
/// vocabulary — the controlled set [`Device::fleet_role`] is restricted to,
/// so that the two tools apply the same defaults for the same word rather
/// than two spellings of one question (`ADR-011` §3).
///
/// **Two roles, not three.** A machine that is neither a server nor a member
/// is `Option::None` — there is no `StandAlone` variant, because one never
/// named a behaviour distinct from the role being absent: every match on
/// this type (`cmd/rivet-fleet/src/compose.rs::role_listen_default` and
/// `::role_services`) paired the two identically. Removed as a ratchet
/// increment; a record on disk may still carry the retired `"stand-alone"`
/// spelling, and [`Device::fleet_role`]'s deserializer
/// (`deserialize_fleet_role`, private to this crate) is what keeps such a
/// record loading, as `None`, rather than refusing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FleetRole {
    /// A fleet server — `rivet-fleetd`, `rivet-proxyd` and `rivet-mdnsd` by
    /// default, reachable from elsewhere by definition.
    Server,
    /// A fleet member — `rivet-mdnsd` by default, reachable by name from the
    /// server that manages it.
    Member,
}

impl FleetRole {
    /// The word an operator types, and the word this renders back — the same
    /// two `cmd/rivet-provision`'s menu offers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Member => "member",
        }
    }

    /// Parse an operator's word, or `None` for anything else.
    ///
    /// **Exactly two answers, and nothing guessed for a third.** A word this
    /// tool does not recognise is refused by the caller rather than mapped
    /// to whichever role looks closest — `AGENTS.md` §5. The retired
    /// `"stand-alone"` spelling is one such word here: it is no longer a
    /// role to be typed going forward (a record already holding it is a
    /// different case — see [`Device::fleet_role`]'s deserializer).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "server" => Some(Self::Server),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

/// [`Device::fleet_role`]'s deserializer — the seam that keeps a record
/// written before this ratchet loading correctly.
///
/// A record may carry the retired `"stand-alone"` spelling, or simply omit
/// the key (`default` on the field handles that case before this function
/// ever runs). Both meant, and still mean, no role default at all, so both
/// come back as `None` rather than a refusal: the word never named a
/// behaviour [`FleetRole::Server`] or [`FleetRole::Member`] did not already
/// cover between them. Anything else unrecognised is still a refusal, the
/// same as it always was.
fn deserialize_fleet_role<'de, D>(deserializer: D) -> Result<Option<FleetRole>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(rename_all = "kebab-case")]
    enum Word {
        Server,
        Member,
        StandAlone,
    }
    Ok(match Option::<Word>::deserialize(deserializer)? {
        None | Some(Word::StandAlone) => None,
        Some(Word::Server) => Some(FleetRole::Server),
        Some(Word::Member) => Some(FleetRole::Member),
    })
}

impl core::fmt::Display for FleetRole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Device {
    /// **The port this tool dials to reach this machine's control plane.**
    ///
    /// The port half of [`Device::listen`], or
    /// `rivet_config::control_plane::DEFAULT_PORT` when the record does not say
    /// — see that field for why absent and default are the same state here.
    ///
    /// **The address half is deliberately not used.** What a machine BINDS and
    /// what a workstation DIALS are different facts: a board listening on
    /// `0.0.0.0:7777` is reached at its own interface address, which is
    /// [`Self::address`]'s answer and `ADR-034` §9's payoff. Only
    /// the port is shared between the two, and it is the only half taken.
    ///
    /// A `listen` the register holds but cannot be read as an address falls
    /// back to the default rather than refusing: `crate::compose` hands the
    /// value to `rivet-config`'s own parser, which is where a malformed one is
    /// refused (`ADR-011` — one authority for the grammar), and a probe that
    /// panicked on a record an operator had hand-edited would be worse than one
    /// that dialled the usual number and reported what happened.
    #[must_use]
    pub fn control_plane_port(&self) -> u16 {
        self.listen
            .as_deref()
            .and_then(|listen| listen.parse::<std::net::SocketAddr>().ok())
            .map_or(rivet_config::control_plane::DEFAULT_PORT, |address| {
                address.port()
            })
    }

    /// **Where to reach this machine**, from its own record.
    ///
    /// It was `cmd/rivet-fleet`'s `member::address_of` until 2026-09-01 and
    /// moved here with the register, because a second program now needs the
    /// answer: `cmd/rivet-fleetd` reaches every device in the register, and
    /// two implementations of *"which of these addresses is the machine"*
    /// would eventually disagree about a device with two interfaces.
    ///
    /// **Ambiguity is refused, never resolved by picking.** Reaching the wrong
    /// machine is the mistake in this tool that is expensive to undo.
    ///
    /// # Errors
    ///
    /// No address, or more than one — both as a sentence naming the device,
    /// because both are fixed by editing the record rather than by retrying.
    pub fn address(&self) -> Result<String, String> {
        let addressed: Vec<(&String, &String)> = self
            .interfaces
            .iter()
            .filter_map(|(name, interface)| interface.address.as_ref().map(|a| (name, a)))
            .collect();

        match addressed.as_slice() {
            [] => Err(format!(
                "{}'s record carries no address, so there is nowhere to reach it. Either it is a \
                 DHCP machine - in which case its address is not knowable from the register, and \
                 rivet-mdnsd (PLAN.md 23) is the thing that would answer it - or its record has \
                 no [interfaces] at all. Nothing has been sent.",
                self.name
            )),
            [(_, address)] => Ok(host_of(address)),
            several => Err(format!(
                "{}'s record carries {} addresses ({}), and this tool will not choose one: \
                 reaching the wrong machine is the mistake here that is expensive to undo. \
                 Nothing has been sent.",
                self.name,
                several.len(),
                several
                    .iter()
                    .map(|(name, address)| format!("{name} {address}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// **Which host to dial to reach this machine.**
    ///
    /// [`Self::address`]'s answer when the record carries exactly one address.
    /// **When it carries none**, this does not refuse the way [`Self::address`]
    /// does: a DHCP machine's record has no address to give, but it is still
    /// reachable at `<name>.local` once `rivet-mdnsd` is answering for it, so
    /// that is the host handed back instead.
    ///
    /// This is the **one place** that derivation is spelled. It used to live
    /// only in `cmd/cargo-rivet/src/device.rs`'s `dial_address`, which is why
    /// `cargo rivet run <dhcp-member>` already worked while `rivet-fleetd`'s
    /// `/api/fleet` still reported the same machine `"unreachable"` — two
    /// programs need this answer, same as [`Self::address`] itself.
    ///
    /// **Several addresses is still refused, never picked.** [`Self::address`]'s
    /// own reasoning holds here unchanged: reaching the wrong machine is the
    /// mistake this tool cannot afford to make by guessing.
    ///
    /// # Errors
    ///
    /// [`Self::address`]'s ambiguity refusal when the record carries more than
    /// one address; otherwise, when it carries none, a sentence naming why
    /// [`Self::name`] is not usable as a hostname to derive `<name>.local`
    /// from. **Deliberately states only the fact, not a next step** — this
    /// method has two callers now (`cmd/cargo-rivet` and `cmd/rivet-fleetd`)
    /// and only one of them can offer `--host HOST:PORT`, so each caller adds
    /// its own advice onto this error rather than this method guessing which
    /// one is reading it. Both are fixed by editing the record, not by
    /// retrying.
    ///
    /// [`Self::name`]: Device::name
    pub fn dial_host(&self) -> Result<String, String> {
        let addressed: Vec<(&String, &String)> = self
            .interfaces
            .iter()
            .filter_map(|(name, interface)| interface.address.as_ref().map(|a| (name, a)))
            .collect();

        match addressed.as_slice() {
            [] => {
                let Some(hostname) = rivet_core::hostname::Hostname::new(&self.name) else {
                    return Err(format!(
                        "{}'s record carries no address, and {:?} is not a hostname, so there is \
                         no <hostname>.local to derive for it.",
                        self.name, self.name
                    ));
                };
                Ok(format!("{}.local", hostname.as_str()))
            }
            [(_, address)] => Ok(host_of(address)),
            // Several addresses: the same ambiguity `address()` refuses,
            // worded once and not repeated here.
            _ => self.address(),
        }
    }
}

/// An address with any `/prefix` taken off, because a record holds CIDR and a
/// socket wants a host.
fn host_of(address: &str) -> String {
    address
        .split_once('/')
        .map_or_else(|| address.to_owned(), |(host, _)| host.to_owned())
}

/// One interface's addressing, as the register records it.
///
/// **Deliberately not `rivet_config::Interface`.** That type is a *parsed
/// document's* interface and its constructor refuses what the document refuses;
/// this is a record an operator edits, and the two must be allowed to disagree
/// long enough for `crate::compose` to say *why*. Composing turns one into the
/// other and that is where the refusals happen — at the moment a document is
/// made, not at the moment a record is read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    /// `"dhcp"` or `"static"`, as `rivet-config` spells them.
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
}

/// One device's resolver, as the register records it — `ADR-034` §6.
///
/// # It is not `rivet_config::resolver::Resolver`, for [`Interface`]'s own
/// reason
///
/// This is a record an operator edits, not a parsed document's table, and the
/// two must be allowed to disagree long enough for `crate::compose` (in
/// `cmd/rivet-fleet`) to say why: `nameservers` and `search` are held as raw
/// strings here and validated where every other document rule is, at compose
/// time, by the parser the composed document is actually handed to. A record
/// with four nameservers typed into it is this crate's business to store, not
/// to truncate at `rivet_config::resolver::Resolver::MAX` on its way onto
/// disk.
///
/// # Why a resolver has its own field at all
///
/// Before this field existed, a resolver had nowhere honest to live: the only
/// per-network storage `Device` offered was [`Device::interfaces`], keyed by
/// interface name, and `[network.resolver]` is not an interface — it has no
/// `method`, no `address`, no `gateway`. One build of adoption
/// (`cmd/rivet-fleet/src/main.rs`'s `adopt`) reached for the map it had and
/// wrote a `"resolver"` entry with `method = "dhcp"` — a value that means
/// nothing for a resolver — while the nameservers it was supposed to be
/// carrying had no field to land in and were dropped. `crate::compose`
/// composed that record straight back out as `[network.resolver] method =
/// "dhcp"`, which `rivet-config` correctly refuses: that table declares
/// `nameservers` and `search`, and nothing else.
///
/// Giving a resolver a field of its own, distinct from the interface map,
/// removes the place the defect had to land — an interface named `resolver`
/// stops being representable in the register at all, matching
/// `rivet_config::resolver::Resolver::TABLE`'s status as a reserved word
/// rather than a real interface name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolver {
    /// The nameservers, in the order they will be asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nameservers: Vec<String>,
    /// The search domains, in the order they will be tried.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search: Vec<String>,
}

/// What every machine in this fleet shares — `ADR-007` §3.4's **layer 3**,
/// given the home `ADR-034` §6.1 says it always implied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fleet {
    /// A label for this fleet, for a human looking at two registers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Ed25519 public keys every machine's control plane accepts.
    ///
    /// **`ADR-025` §2's other half.** The fleet holds a map of device to public
    /// key — [`Device::pin`] — and *"the device holds the fleet's public
    /// key"*. This is that: the operator's or fleet server's key, recorded once
    /// and composed into every device's `peers` file.
    ///
    /// It is the field that makes control-plane access a FLEET fact. Before it
    /// existed, every `peers` file was written by hand and no two machines could
    /// be reasoned about together.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_plane_keys: Vec<String>,

    /// Signing key ids every machine trusts, in addition to its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusts: Vec<String>,
    /// Services every machine runs, unless its own record says otherwise.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub services: BTreeMap<String, bool>,
    /// Memory caps every machine applies, unless its own record says otherwise.
    /// `Device::service_memory` carries the argument for the shape.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub service_memory: BTreeMap<String, String>,
    /// Which published version of a carried product every device carries,
    /// unless its own record pins a different one —
    /// [`Device::carry`]'s fleet-wide default. `.rulings/` R-160 R26.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub carry: BTreeMap<String, String>,
}

/// Everything the register holds, in memory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Register {
    pub root: PathBuf,
    pub fleet: Fleet,
    /// Ordered by identifier, which is the order the directory is listed in and
    /// the order every rendering uses.
    pub devices: BTreeMap<String, Device>,
}

/// What went wrong, in the operator's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// The path could not be read or written, and what the system said.
    Io { path: PathBuf, detail: String },
    /// A file in the register is not a record this tool can read.
    Malformed { path: PathBuf, detail: String },
    /// A record's `id` field and its file name disagree.
    ///
    /// **A refusal rather than a repair**, because the two disagreeing means
    /// somebody renamed a file by hand and the register can no longer say which
    /// of the two the references elsewhere meant.
    Misfiled {
        path: PathBuf,
        says: String,
        filed_as: String,
    },
    /// No device answers to that label or identifier.
    NoSuchDevice { asked: String },
    /// More than one device answers to that label.
    Ambiguous { asked: String, ids: Vec<String> },
    /// A device with that label already exists.
    AlreadyEnrolled { name: String, id: String },
    /// A record name that is not exactly one plain path component — a
    /// parent, an absolute path, a root, a prefix, or anything containing a
    /// separator.
    ///
    /// Refused before any path is composed from it, rather than normalised
    /// into a different name: joining an unchecked name into a records
    /// directory can walk back out of it or replace it outright, so this is
    /// caught before that join is ever trusted.
    InvalidName { name: String },
}

impl core::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io { path, detail } => {
                write!(f, "{}: {detail}", path.display())
            }
            Self::Malformed { path, detail } => write!(
                f,
                "{} is not a register record this tool can read: {detail}",
                path.display()
            ),
            Self::Misfiled {
                path,
                says,
                filed_as,
            } => write!(
                f,
                "{} says its id is {says:?} but it is filed as {filed_as:?}. A record is filed \
                 under ADR-025 4.1's identifier and nothing may rename it, because every \
                 reference to this device is by that identifier. Nothing has been changed - \
                 move the file back, or correct the id field to match the name it is filed under.",
                path.display()
            ),
            Self::NoSuchDevice { asked } => write!(
                f,
                "no device in this register answers to {asked:?}, by label or by identifier"
            ),
            Self::Ambiguous { asked, ids } => write!(
                f,
                "{asked:?} is the label of {} devices: {}. A label is mutable and is not unique \
                 (ADR-025 4.1), so this is refused rather than guessed at - name one by its \
                 identifier.",
                ids.len(),
                ids.join(", ")
            ),
            Self::AlreadyEnrolled { name, id } => write!(
                f,
                "this register already has a device labelled {name:?} ({id}). Two devices may \
                 carry one label and the store permits it, but enrolling a second by the same \
                 name is more often a repeated command than an intention - say it again with a \
                 different label, or remove the first."
            ),
            Self::InvalidName { name } => write!(
                f,
                "{name:?} is not a name this register can file a record under - a record's name \
                 is exactly one plain path segment, never a parent, an absolute path, or a path \
                 containing a separator. Nothing was changed."
            ),
        }
    }
}

impl Register {
    /// An empty register rooted at `root`. Nothing is read and nothing is
    /// written.
    #[must_use]
    pub fn empty(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            fleet: Fleet::default(),
            devices: BTreeMap::new(),
        }
    }

    /// The device that answers to `asked`, by identifier or by label.
    ///
    /// **The identifier is tried first**, because it is the authoritative one
    /// and because a label that happens to look like an identifier must not
    /// shadow the real record.
    ///
    /// # Errors
    ///
    /// [`RegisterError::NoSuchDevice`] when nothing answers, and
    /// [`RegisterError::Ambiguous`] when more than one label does — never a
    /// guess. See this module's header for why that one matters.
    pub fn resolve(&self, asked: &str) -> Result<&Device, RegisterError> {
        if let Some(device) = self.devices.get(asked) {
            return Ok(device);
        }
        let matches: Vec<&Device> = self
            .devices
            .values()
            .filter(|device| device.name == asked)
            .collect();
        match matches.as_slice() {
            [] => Err(RegisterError::NoSuchDevice {
                asked: asked.to_owned(),
            }),
            [only] => Ok(only),
            several => Err(RegisterError::Ambiguous {
                asked: asked.to_owned(),
                ids: several.iter().map(|d| d.id.clone()).collect(),
            }),
        }
    }

    /// Add a device the register does not have.
    ///
    /// # Errors
    ///
    /// [`RegisterError::AlreadyEnrolled`] when something already carries that
    /// label. The store would permit it — a label is not a key — and this
    /// refuses anyway, because the far more likely cause is a command run
    /// twice.
    pub fn enroll(&mut self, device: Device) -> Result<(), RegisterError> {
        if let Some(existing) = self.devices.values().find(|d| d.name == device.name) {
            return Err(RegisterError::AlreadyEnrolled {
                name: existing.name.clone(),
                id: existing.id.clone(),
            });
        }
        self.devices.insert(device.id.clone(), device);
        Ok(())
    }

    /// The record that already carries `pin` under a label other than `name`.
    ///
    /// **One control-plane key per machine** (`.rulings/` R-189 D2). Two records
    /// with one pin are two machines holding one private key, and a caller
    /// dialling either one reaches whichever board answers. The label is the
    /// exception because a record under the SAME label is the same machine
    /// again, and the caller refuses that case separately, naming its repair.
    ///
    /// `pin` is hex. The comparison ignores case and surrounding whitespace,
    /// because a record written by hand may carry either.
    #[must_use]
    pub fn pin_held_elsewhere(&self, pin: &str, name: &str) -> Option<&Device> {
        let pin = pin.trim();
        self.devices.values().find(|device| {
            device.name != name
                && device
                    .pin
                    .as_deref()
                    .is_some_and(|held| held.trim().eq_ignore_ascii_case(pin))
        })
    }

    /// Where a device's record is, or would be.
    #[must_use]
    pub fn record_path(&self, id: &str) -> PathBuf {
        self.root
            .join(DEVICES_DIR)
            .join(format!("{id}{RECORD_SUFFIX}"))
    }

    /// Where the fleet-wide file is, or would be.
    #[must_use]
    pub fn fleet_path(&self) -> PathBuf {
        self.root.join(REGISTER_FILE)
    }
}

/// Whether `text` is an identifier this tool would mint.
///
/// Used to refuse a hand-written one before it is filed, rather than after
/// something references it.
#[must_use]
pub fn is_identifier(text: &str) -> bool {
    rivet_core::identifier::is_identifier(text)
}

/// Mint an identifier from 32 bytes of entropy the caller supplies.
///
/// **The randomness is the caller's**, which is what makes this testable at
/// all: `ADR-025` §4.1 requires the identifier be stable and opaque, and both
/// properties are about the *string*, not about where the bytes came from.
#[must_use]
pub fn identifier_from(entropy: &[u8]) -> String {
    rivet_core::identifier::from_entropy(entropy)
}

/// The register's whole state, as the bytes that would be written.
///
/// **Rendering is separate from writing**, so that every property the files
/// must have — stable ordering, no timestamps, one device per file — is
/// asserted without touching a disk.
///
/// # Errors
///
/// [`RegisterError::Malformed`] if a record cannot be rendered, which can only
/// happen if a field's type stops being representable in TOML.
pub fn render(register: &Register) -> Result<Vec<(PathBuf, String)>, RegisterError> {
    let mut out = Vec::with_capacity(register.devices.len() + 1);
    out.push((
        register.fleet_path(),
        render_one(&register.fleet, &register.fleet_path())?,
    ));
    // `BTreeMap` iteration is sorted by identifier, so this order is the same
    // on every run and on every machine.
    for (id, device) in &register.devices {
        let path = register.record_path(id);
        out.push((path.clone(), render_one(device, &path)?));
    }
    Ok(out)
}

fn render_one<T: Serialize>(value: &T, path: &Path) -> Result<String, RegisterError> {
    toml_edit::ser::to_string_pretty(value).map_err(|e| RegisterError::Malformed {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

/// Read a device record, and refuse one that is not filed under its own id.
///
/// # A legacy `[interfaces.resolver]` is dropped, never carried forward
///
/// `ADR-034` §6 names the defect this guards: before [`Resolver`] existed,
/// one build of adoption wrote a resolver as an interface named `"resolver"`,
/// with `method = "dhcp"` and no nameserver — the field that would have held
/// one did not exist yet. A record still carrying that entry is read, not
/// refused: refusing here would turn a stale key nobody chose into a register
/// this tool cannot open at all, and there is nothing to migrate the entry
/// *into* — `method = "dhcp"` is not a nameserver, so [`Device::resolver`]
/// stays exactly what the record actually says about it, which is nothing.
/// The entry is simply dropped, so a `Device` this function returns can never
/// carry an interface named `"resolver"` for `crate::compose::render` (in
/// `cmd/rivet-fleet`) to render back out as the same refused table.
///
/// # Errors
///
/// [`RegisterError::Malformed`] for text that is not a record, and
/// [`RegisterError::Misfiled`] when the `id` field and the file name disagree.
pub fn read_device(path: &Path, text: &str) -> Result<Device, RegisterError> {
    let mut device: Device =
        toml_edit::de::from_str(text).map_err(|e| RegisterError::Malformed {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;
    device.interfaces.remove(rivet_config::resolver::Resolver::TABLE);
    let filed_as = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if device.id != filed_as {
        return Err(RegisterError::Misfiled {
            path: path.to_path_buf(),
            says: device.id,
            filed_as,
        });
    }
    if !is_identifier(&device.id) {
        // An id that is not one this tool would mint is refused here rather
        // than filed: `ADR-025` §4.1 makes it authoritative, and an
        // authoritative value with no format is one nothing can check a
        // reference against.
        return Err(RegisterError::Malformed {
            path: path.to_path_buf(),
            detail: format!(
                "{:?} is not an identifier: ADR-025 4.1's is {ID_CHARS} lowercase hexadecimal \
                 characters, and this tool mints them. A record filed under anything else \
                 cannot be told apart from a human label.",
                device.id
            ),
        });
    }
    Ok(device)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn device(id: &str, name: &str) -> Device {
        Device {
            id: id.to_owned(),
            name: name.to_owned(),
            ..Device::default()
        }
    }

    fn two() -> Register {
        let mut register = Register::empty("/reg");
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        register.enroll(device("8899aabbccddeeff", "garage-pi")).expect("fresh");
        register
    }

    // ---- filed by identifier, resolved by either ---------------------------

    /// The label is what an operator types and the identifier is what an agent
    /// holds. Both reach the same record.
    #[test]
    fn a_device_answers_to_its_label_and_to_its_identifier() {
        let register = two();
        assert_eq!(register.resolve("kitchen-pi").expect("by label").id, "0011223344556677");
        assert_eq!(
            register.resolve("0011223344556677").expect("by id").name,
            "kitchen-pi"
        );
    }

    /// **The refusal this module exists for.** A label is mutable and not
    /// unique; picking one silently would provision the wrong machine, and that
    /// is the one mistake here that is expensive to undo.
    #[test]
    fn a_label_two_devices_carry_is_refused_rather_than_guessed_at() {
        let mut register = Register::empty("/reg");
        register.devices.insert(
            "0011223344556677".to_owned(),
            device("0011223344556677", "pi"),
        );
        register.devices.insert(
            "8899aabbccddeeff".to_owned(),
            device("8899aabbccddeeff", "pi"),
        );
        let error = register.resolve("pi").expect_err("two devices carry that label");
        match &error {
            RegisterError::Ambiguous { ids, .. } => assert_eq!(ids.len(), 2),
            other => panic!("{other:?}"),
        }
        // And the message names both, so the operator can pick one.
        let text = error.to_string();
        assert!(text.contains("0011223344556677"), "{text}");
        assert!(text.contains("8899aabbccddeeff"), "{text}");
    }

    /// The identifier wins, so a label that looks like one cannot shadow the
    /// record actually filed under it.
    #[test]
    fn a_label_that_looks_like_an_identifier_does_not_shadow_the_real_one() {
        let mut register = Register::empty("/reg");
        register.devices.insert(
            "0011223344556677".to_owned(),
            device("0011223344556677", "real"),
        );
        register.devices.insert(
            "8899aabbccddeeff".to_owned(),
            device("8899aabbccddeeff", "0011223344556677"),
        );
        assert_eq!(
            register.resolve("0011223344556677").expect("the record").name,
            "real"
        );
    }

    #[test]
    fn nothing_answering_is_a_refusal_that_says_what_was_asked() {
        let register = two();
        let error = register.resolve("bathroom-pi").expect_err("no such device");
        assert!(error.to_string().contains("bathroom-pi"), "{error}");
    }

    /// Enrolling twice is far more often a repeated command than an intention.
    #[test]
    fn a_label_already_in_the_register_is_refused_at_enrollment() {
        let mut register = two();
        let error = register
            .enroll(device("ffffffffffffffff", "kitchen-pi"))
            .expect_err("already enrolled");
        assert!(error.to_string().contains("kitchen-pi"), "{error}");
        assert_eq!(register.devices.len(), 2, "the refused device was filed anyway");
    }

    /// A pin already on a record under another label is found, whatever its
    /// case; the same label is not reported, because that is the same machine.
    #[test]
    fn a_pin_is_found_only_under_another_label() {
        let mut register = two();
        let pin = "5f557a58".repeat(8);
        register.devices.get_mut("0011223344556677").expect("kitchen-pi").pin =
            Some(pin.to_uppercase());
        let held = register.pin_held_elsewhere(&pin, "rivet-new").expect("held");
        assert_eq!(held.name, "kitchen-pi");
        assert!(register.pin_held_elsewhere(&pin, "kitchen-pi").is_none());
        assert!(register.pin_held_elsewhere(&"ab".repeat(32), "rivet-new").is_none());
    }

    // ---- files that are good to commit -------------------------------------

    /// `ADR-034` §10.3: *"stable ordering, no churning timestamps, one device
    /// per file"*. This is the first of the three.
    #[test]
    fn the_same_register_renders_the_same_bytes_every_time() {
        let register = two();
        assert_eq!(render(&register).expect("renderable"), render(&register).expect("renderable"));
    }

    /// The second: a register built in a different order renders identically,
    /// which is the property a `HashMap` would have taken away.
    #[test]
    fn the_order_devices_were_added_in_does_not_reach_the_files() {
        let mut backwards = Register::empty("/reg");
        backwards.enroll(device("8899aabbccddeeff", "garage-pi")).expect("fresh");
        backwards.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        assert_eq!(render(&backwards).expect("renderable"), render(&two()).expect("renderable"));
    }

    /// The third: one device per file, filed under its identifier, plus the one
    /// fleet-wide file.
    #[test]
    fn every_device_is_its_own_file_named_by_its_identifier() {
        let rendered = render(&two()).expect("renderable");
        let paths: Vec<String> = rendered
            .iter()
            .map(|(p, _)| p.display().to_string().replace('\\', "/"))
            .collect();
        assert_eq!(
            paths,
            [
                "/reg/register.toml",
                "/reg/devices/0011223344556677.toml",
                "/reg/devices/8899aabbccddeeff.toml",
            ]
        );
    }

    /// Nothing rendered carries a clock reading. A register that changed every
    /// time it was written would be one nobody could review a diff of.
    #[test]
    fn nothing_rendered_carries_a_timestamp() {
        for (path, text) in render(&two()).expect("renderable") {
            for suspicious in ["written", "updated", "timestamp", "20"] {
                if suspicious == "20" {
                    // A bare year would be a false positive on an address, so
                    // this looks for the shape a date takes.
                    assert!(
                        !text.contains("20") || !text.contains('-') || !text.contains(':'),
                        "{} looks like it carries a date: {text}",
                        path.display()
                    );
                    continue;
                }
                assert!(!text.contains(suspicious), "{}: {text}", path.display());
            }
        }
    }

    /// A record round-trips, which is what makes the store readable at all.
    #[test]
    fn a_record_read_back_is_the_record_written() {
        let mut written = device("0011223344556677", "kitchen-pi");
        written.role = Some("proxy".to_owned());
        written.trusts.push("k1".to_owned());
        written.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("10.0.0.2/24".to_owned()),
                gateway: Some("10.0.0.1".to_owned()),
            },
        );
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        assert_eq!(read_device(&path, &text).expect("readable"), written);
    }

    /// **A server's `operator_key` round-trips, and an absent one stays
    /// absent.** `docs/PLAN.md` §183.2: this is the fleet-operator public key a
    /// member joining this server must be staged to trust, so the register must
    /// carry it through a save/load unchanged — and a record without one (every
    /// record written before this field, and every member) must read back as
    /// `None` and re-render without the key.
    #[test]
    fn a_devices_operator_key_round_trips_and_absent_stays_absent() {
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");

        let mut server = device("0011223344556677", "rivet-01");
        server.operator_key = Some("cd".repeat(32));
        let text = render_one(&server, &path).expect("renderable");
        assert_eq!(read_device(&path, &text).expect("readable"), server);
        assert_eq!(
            read_device(&path, &text).expect("readable").operator_key.as_deref(),
            Some("cd".repeat(32).as_str()),
        );

        // A record with none — the common case — reads back None and renders no
        // key at all, so a member's record and a pre-field server's record are
        // untouched.
        let member = device("8899aabbccddeeff", "kitchen-pi");
        assert_eq!(member.operator_key, None);
        let member_path = PathBuf::from("/reg/devices/8899aabbccddeeff.toml");
        let member_text = render_one(&member, &member_path).expect("renderable");
        assert!(!member_text.contains("operator_key"), "{member_text}");
        assert_eq!(
            read_device(&member_path, &member_text).expect("readable").operator_key,
            None,
        );
    }

    // ---- SPEC-05 5: the role, the timezone, and application settings -------

    /// The role, the timezone and an application's settings all round-trip,
    /// which is what makes `add document` able to fill them in at all.
    #[test]
    fn the_role_the_timezone_and_application_settings_round_trip() {
        let mut written = device("0011223344556677", "kitchen-pi");
        written.fleet_role = Some(FleetRole::Server);
        written.timezone = Some("America/Chicago".to_owned());
        written
            .applications
            .entry("rivet-proxyd".to_owned())
            .or_default()
            .insert("upstream-name".to_owned(), "rivet-01.example.arpa".to_owned());
        written
            .applications
            .entry("tempmon-rivet".to_owned())
            .or_default()
            .insert("sensor-labels".to_owned(), "GVH5075_1111=fridge".to_owned());
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        assert_eq!(read_device(&path, &text).expect("readable"), written);
    }

    /// **`kiosk_auto_run` round-trips like every other field.** `ADR-034` §6's
    /// gap: a kiosk machine's record could not carry the one thing `SPEC-05`
    /// §5 says it additionally declares, so `add document` had nowhere to put
    /// it and `rivet-01` - a kiosk running `aura-renderer` - could not be
    /// fully rebuilt from its own register record.
    #[test]
    fn a_devices_kiosk_auto_run_round_trips() {
        let mut written = device("0011223344556677", "kitchen-pi");
        written.kiosk_auto_run = Some("aura-renderer".to_owned());
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        assert_eq!(read_device(&path, &text).expect("readable"), written);
    }

    /// **`carry` round-trips like every other field** — `.rulings/` R-160
    /// R26.
    #[test]
    fn a_devices_carry_pin_round_trips() {
        let mut written = device("0011223344556677", "kitchen-pi");
        written.carry.insert("rivet-fleetd".to_owned(), "0.1.86".to_owned());
        written.carry.insert("hello-rivet".to_owned(), "0.4.3".to_owned());
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        assert_eq!(read_device(&path, &text).expect("readable"), written);
    }

    /// **`system_slots` round-trips, and an empty one renders nothing** —
    /// `ADR-052` §7. Absent is the state of every record written before it.
    #[test]
    fn a_devices_system_slots_round_trip_and_an_empty_one_renders_nothing() {
        let mut written = device("0011223344556677", "kitchen-pi");
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let bare = render_one(&written, &path).expect("renderable");
        assert!(!bare.contains("system_slots"), "{bare}");

        written.system_slots.set("b", &"b2".repeat(32)).expect("a slot");
        let text = render_one(&written, &path).expect("renderable");
        assert!(text.contains("[system_slots]"), "{text}");
        assert_eq!(read_device(&path, &text).expect("readable"), written);
        assert_eq!(written.system_slots.digests(), vec![("b", "b2".repeat(32).as_str())]);
        assert!(written.system_slots.set("c", "x").is_err(), "only a and b are slots");
    }

    /// **An existing record still loads.** A device written before `carry`
    /// existed carries no `[carry]` key at all — the same
    /// backward-compatibility every other field on this struct already has —
    /// and reading it back gets an empty map, not a refusal.
    #[test]
    fn a_device_record_with_no_carry_key_still_loads() {
        let text = "id = \"0011223344556677\"\nname = \"kitchen-pi\"\n";
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let read = read_device(&path, text).expect("an old record still loads");
        assert!(read.carry.is_empty());
    }

    /// **Absent is absent, for every one of the four.** A record written
    /// before these fields existed still renders none of them — the same
    /// property `Self::listen` and `Self::resolver` already have, and the
    /// one `crate::compose`'s byte-stability claim depends on.
    #[test]
    fn a_device_with_none_of_the_four_renders_none_of_them() {
        let written = device("0011223344556677", "kitchen-pi");
        assert_eq!(written.fleet_role, None);
        assert_eq!(written.timezone, None);
        assert!(written.applications.is_empty());
        assert_eq!(written.kiosk_auto_run, None);
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        for absent in ["fleet_role", "fleet-role", "timezone", "applications", "kiosk"] {
            assert!(!text.contains(absent), "{absent} in {text}");
        }
    }

    /// **Exactly two words, and nothing guessed for a third** — the same
    /// refusal-over-guessing rule `AGENTS.md` §5 states generally, applied to
    /// the one field a typo in would silently apply the wrong role's
    /// defaults. `"stand-alone"` is one such refused word now: it named no
    /// role of its own, so it is no longer offered as one going forward —
    /// see `a_record_with_the_retired_stand_alone_role_still_loads_as_none`
    /// below for the separate, still-honoured case of a record that already
    /// has it on disk.
    #[test]
    fn a_fleet_role_is_exactly_one_of_two_words() {
        assert_eq!(FleetRole::parse("server"), Some(FleetRole::Server));
        assert_eq!(FleetRole::parse("member"), Some(FleetRole::Member));
        assert_eq!(FleetRole::parse("stand-alone"), None);
        assert_eq!(FleetRole::parse("standalone"), None);
        assert_eq!(FleetRole::parse("Server"), None);
        assert_eq!(FleetRole::parse(""), None);
        for role in [FleetRole::Server, FleetRole::Member] {
            assert_eq!(FleetRole::parse(role.as_str()), Some(role), "{role} does not round-trip");
        }
    }

    /// **The ratchet's regression guard.** A record written before
    /// `FleetRole::StandAlone` was removed may still carry
    /// `fleet-role = "stand-alone"` on disk. It named no behaviour of its
    /// own even when the variant existed — every caller that matched on the
    /// role paired it with `None` (`cmd/rivet-fleet/src/compose.rs`) — so the
    /// record must still load, as `fleet_role: None`, rather than refuse.
    #[test]
    fn a_record_with_the_retired_stand_alone_role_still_loads_as_none() {
        let text = "id = \"0011223344556677\"\nname = \"kitchen-pi\"\nfleet-role = \"stand-alone\"\n";
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let read = read_device(&path, text).expect("a record with the retired role still loads");
        assert_eq!(read.fleet_role, None);
    }

    // ---- the resolver, `ADR-034` §6 ----------------------------------------

    /// A resolver round-trips exactly like every other field, as its own
    /// table rather than an entry of `[interfaces]`.
    #[test]
    fn a_devices_resolver_round_trips_as_its_own_table() {
        let mut written = device("0011223344556677", "kitchen-pi");
        written.resolver = Some(Resolver {
            nameservers: vec!["192.168.50.1".to_owned(), "1.1.1.1".to_owned()],
            search: vec!["lan".to_owned()],
        });
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        assert!(text.contains("[resolver]"), "{text}");
        assert!(!text.contains("[interfaces.resolver]"), "{text}");
        assert_eq!(read_device(&path, &text).expect("readable"), written);
    }

    /// **Absent is absent.** A record that says nothing about a resolver
    /// renders no `[resolver]` table at all - the same rule every other
    /// optional field on `Device` already follows, so a register written
    /// before this field existed goes on composing identical bytes.
    #[test]
    fn a_device_with_no_resolver_renders_no_resolver_table() {
        let written = device("0011223344556677", "kitchen-pi");
        assert_eq!(written.resolver, None);
        let path = PathBuf::from("/reg/devices/0011223344556677.toml");
        let text = render_one(&written, &path).expect("renderable");
        assert!(!text.contains("resolver"), "{text}");
    }

    /// **The defect this whole field exists to retire, read back.** A record
    /// still carrying the shape one build of adoption wrote - a resolver
    /// modelled as an interface, `method = "dhcp"` and no nameserver - is
    /// read without error, the entry is gone from `interfaces`, and
    /// `resolver` stays `None` rather than inventing a value the record never
    /// actually stated. Broken deliberately first (`AGENTS.md` §4): before
    /// `read_device` stripped this entry, this assertion failed with
    /// `interfaces` carrying a `"resolver"` key.
    #[test]
    fn a_legacy_resolver_interface_is_dropped_on_read_and_invents_nothing() {
        let text = "id = \"0011223344556677\"
name = \"kitchen-pi\"

[interfaces.eth0]
method = \"static\"
address = \"192.168.50.10/24\"
gateway = \"192.168.50.1\"

[interfaces.resolver]
method = \"dhcp\"
";
        let path = Path::new("/reg/devices/0011223344556677.toml");
        let device = read_device(path, text).expect("a legacy record is read, not refused");
        assert!(
            !device.interfaces.contains_key("resolver"),
            "{:?}",
            device.interfaces
        );
        assert!(device.interfaces.contains_key("eth0"), "the real interface must survive");
        assert_eq!(
            device.resolver, None,
            "there is nothing in `method = \"dhcp\"` to migrate into a nameserver"
        );
    }

    /// **A renamed file is a refusal, not a repair.** The two disagreeing means
    /// somebody moved a record by hand, and the register can no longer say
    /// which of the two names every reference elsewhere meant.
    #[test]
    fn a_record_filed_under_the_wrong_name_is_refused() {
        let written = device("0011223344556677", "kitchen-pi");
        let text = render_one(&written, Path::new("x")).expect("renderable");
        let error = read_device(Path::new("/reg/devices/somethingelse.toml"), &text)
            .expect_err("misfiled");
        match &error {
            RegisterError::Misfiled { says, filed_as, .. } => {
                assert_eq!(says, "0011223344556677");
                assert_eq!(filed_as, "somethingelse");
            }
            other => panic!("{other:?}"),
        }
        assert!(error.to_string().contains("Nothing has been changed"), "{error}");
    }

    /// Text that is not a record is refused with what the parser said, rather
    /// than with a default record.
    #[test]
    fn text_that_is_not_a_record_is_refused_rather_than_defaulted() {
        let error = read_device(Path::new("/reg/devices/x.toml"), "this is not toml {{{")
            .expect_err("not a record");
        assert!(matches!(error, RegisterError::Malformed { .. }), "{error:?}");
    }

    // ---- no secret is representable ----------------------------------------

    /// **`ADR-034` §3, as a test rather than as a promise.** A field for a
    /// private key or a signing seed would make the register a place one could
    /// be put, and §3.3's argument is that the two must not share a failure
    /// domain at all.
    #[test]
    fn the_record_has_nowhere_to_put_a_secret() {
        let mut full = device("0011223344556677", "kitchen-pi");
        // **Hex, because that is what a pin IS.** This line carried a whole
        // line of base64 until 2026-08-29 and was the evidence that the field's
        // old name misled a reader — see `Device::pin`.
        full.pin = Some("c0ffee".repeat(11)[..64].to_owned());
        full.listen = Some("0.0.0.0:7777".to_owned());
        full.wireless = Some("home".to_owned());
        let text = render_one(&full, Path::new("x")).expect("renderable");
        for forbidden in ["private", "secret", "seed", "passphrase", "psk", "pmk"] {
            assert!(
                !text.to_ascii_lowercase().contains(forbidden),
                "a fully-populated record renders {forbidden:?}: {text}"
            );
        }
    }

    // ---- identifiers -------------------------------------------------------

    /// **The shape a fleet server is seeded with**, parsed by the code that
    /// will parse it on the machine.
    ///
    /// `ADR-034` §3.5 makes the first record the fleet server's own, written by
    /// the provisioning tool onto the card because nothing is listening yet to
    /// be told. This is that record, taken from a real board on 2026-09-01
    /// (`rivet-01`) with its pin replaced — the point is the SHAPE, and a shape
    /// that only ever existed in a heredoc is a shape nothing verified.
    #[test]
    fn a_seeded_fleet_server_record_parses() {
        let text = "id = \"62bb0f53bd08efdb\"
name = \"rivet-01\"
role = \"fleet-server\"
location = \"bench\"
pin = \"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff\"
listen = \"0.0.0.0:7777\"

[interfaces.eth0]
method = \"static\"
address = \"192.168.50.66/24\"
gateway = \"192.168.50.1\"
";
        let path = std::path::Path::new("devices/62bb0f53bd08efdb.toml");
        let device = read_device(path, text).expect("the seeded shape parses");

        assert_eq!(device.id, "62bb0f53bd08efdb");
        assert_eq!(device.name, "rivet-01");
        assert_eq!(device.role.as_deref(), Some("fleet-server"));
        // The port comes from `listen`, not from the default, and that is the
        // whole of ADR-034 section 6.1.
        assert_eq!(device.control_plane_port(), 7777);
        // One interface with an address is the unambiguous case.
        assert_eq!(device.address().expect("one address"), "192.168.50.66");
    }

    // ---- dial_host -----------------------------------------------------------

    /// A record with exactly one address dials that address, exactly as
    /// [`Device::address`] would — `dial_host` is not a second rule.
    #[test]
    fn dial_host_with_one_address_is_that_address() {
        let mut device = device("0011223344556677", "kitchen-pi");
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.50.66/24".to_owned()),
                gateway: None,
            },
        );
        assert_eq!(device.dial_host().expect("one address"), "192.168.50.66");
    }

    /// **The gap this method closes.** A DHCP machine's record carries no
    /// address at all, and `dial_host` derives `<name>.local` rather than
    /// refusing — the same rule `cmd/cargo-rivet`'s `dial_address` already
    /// applied on its own.
    #[test]
    fn dial_host_with_no_address_and_a_valid_name_derives_dot_local() {
        let device = device("0011223344556677", "kitchen-pi");
        assert_eq!(
            device.dial_host().expect("valid hostname"),
            "kitchen-pi.local"
        );
    }

    /// A name that is not a valid hostname (RFC 1123 label — see
    /// `rivet_core::hostname`) cannot become `<name>.local`, and that is
    /// refused rather than guessed at.
    #[test]
    fn dial_host_with_no_address_and_an_invalid_name_is_refused() {
        let device = device("0011223344556677", "kitchen pi");
        let error = device.dial_host().expect_err("not a hostname");
        assert!(error.contains("kitchen pi"), "{error}");
    }

    /// Several addresses is still refused rather than picked — `dial_host`
    /// carries [`Device::address`]'s own ambiguity rule forward unchanged.
    #[test]
    fn dial_host_with_several_addresses_is_refused_like_address() {
        let mut device = device("0011223344556677", "kitchen-pi");
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.50.66/24".to_owned()),
                gateway: None,
            },
        );
        device.interfaces.insert(
            "wlan0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.50.67/24".to_owned()),
                gateway: None,
            },
        );
        let error = device.dial_host().expect_err("two addresses");
        assert!(error.contains("192.168.50.66"), "{error}");
        assert!(error.contains("192.168.50.67"), "{error}");
    }

    #[test]
    fn an_identifier_is_sixteen_lowercase_hex_characters() {
        let id = identifier_from(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
        assert_eq!(id, "0011223344556677");
        assert!(is_identifier(&id));
    }

    /// Too little entropy is padded rather than shortened: a short identifier
    /// would be filed and referenced before anybody noticed it was short.
    #[test]
    fn too_little_entropy_still_produces_a_full_length_identifier() {
        let id = identifier_from(&[0xab]);
        assert_eq!(id.len(), ID_CHARS);
        assert!(is_identifier(&id), "{id}");
    }

    #[test]
    fn something_that_is_not_an_identifier_is_not_mistaken_for_one() {
        for bad in [
            "kitchen-pi",
            "",
            "0011223344556677a",
            "0011223344556G77",
            // Uppercase hex. One identifier with two spellings would be two
            // file names for one device, which is the whole point of filing
            // by it — so the case is part of the format rather than beside it.
            "8899AABBCCDDEEFF",
        ] {
            assert!(!is_identifier(bad), "{bad:?}");
        }
    }
}
