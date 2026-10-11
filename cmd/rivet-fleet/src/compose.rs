//! **Composing a machine's document from the register.**
//!
//! `ADR-025` §3 has named this actor since 2026-08-22:
//!
//! > the **fleet tool** generates the pair, writes it into the provisioning
//! > file, and **retains only the public key.**
//!
//! `ADR-034` §1 records that the work was being done by hand — *"a provisioning
//! document is edited in a text editor outside the repository, a public key is
//! pasted into it, and the record of which machine holds which credential is in
//! the maintainer's memory."* This module is that job.
//!
//! # It renders text and then makes the machine's own parser agree
//!
//! The composed text is not trusted. It is handed straight to
//! [`rivet_config::Document::parse`] — **the same code the board runs** — and a
//! composition that does not parse is a refusal here rather than a refusal on
//! first boot.
//!
//! That is not belt and braces; it is the whole reason this is a program.
//! `tools/card/provisioning.example.toml` says what the alternative costs, in
//! its own words:
//!
//! > **`cmd/rivet-image` does not validate this.** It does not depend on
//! > `rivet-config`, so nothing at build time can say whether these bytes
//! > parse. **The refusal comes from the board, on first boot**, which is the
//! > wrong end.
//!
//! A document this module returns has already been through the board's parser.
//! The bytes it returns are `Document::to_toml`'s — canonical by `ADR-007`
//! §3.1 — so two registers in the same state compose byte-identical documents
//! and a diff between two of them is a diff of intent.
//!
//! # Two layers, merged here and nowhere else
//!
//! `ADR-007` §3.4's layer 3 is the fleet and layer 4 is the machine, and §3.4
//! records that nothing merges layers. **Nothing still does, on a machine.**
//! What happens here is narrower and is not that engine: the register composes
//! *one* document out of *its own two* records, before any machine has seen
//! either. `ADR-034` §6.1 draws exactly this line —
//!
//! > `ADR-007` §3.4's deferred precedence engine is a prerequisite for the
//! > fleet layer being *effective* and **not** for the register being
//! > *useful* — a register that only composes provisioning documents is worth
//! > building before any merging exists.
//!
//! The rule for the merge is one sentence: **the device wins, and the fleet
//! fills in.** It is stated once, here, and every field follows it.
//!
//! # A third, lower layer: the role — `SPEC-05` §5
//!
//! `cmd/rivet-provision`'s menu asks one question this register-side composer
//! did not until now — whether a machine is a `server`, a `member`, or
//! neither — and applies defaults from the answer: a control plane that
//! binds every interface for a server or a member, one of two non-empty
//! service sets (or none, for neither), and (through
//! [`provisioning_complete`]) a requirement that the proxy's `upstream-name`
//! and the document's `timezone` are both answered before a document leaves
//! this module for good. [`Device::fleet_role`] is that same question —
//! `Option<FleetRole>`, `None` standing for "neither" rather than a third
//! variant, because no default here ever distinguished the two (a ratchet
//! increment removed `FleetRole::StandAlone` for exactly that reason) — and
//! [`role_listen_default`]/[`role_services`] are the two defaults it drives —
//! the SAME defaults `cmd/rivet-provision`'s `listen_for`/`services_for`
//! apply, kept here as a **second, small copy** rather than a shared crate.
//! `rivet-provision` has no library target and extracting one the day before
//! a fleet is minted is the wrong moment; see this crate's `STATE.md`/the
//! commit this module changed in for what unifying the two would take.
//!
//! **The role is beneath the fleet, which is beneath the device.** A role
//! default only ever fills a gap neither of the other two layers spoke to —
//! [`services`] inserts it first and lets `fleet.services` and then
//! `device.services` overwrite it, so an operator who explicitly turns a
//! role's service off is still obeyed.
//!
//! # Two composers, and the difference is deliberate
//!
//! [`provisioning`] is **permissive**: it renders whatever the record
//! currently holds, refusing only when `rivet-config`'s own parser would
//! (`ComposeError::Refused`) or when there is nowhere to reach the machine at
//! all (`ComposeError::Unreachable`). `crate::main`'s `compare` — the engine
//! behind `add document`, `apply document`'s guard, and `set listen`'s
//! sanity check — calls this one, because those are exactly the callers that
//! have to keep working on a record `SPEC-05` §5 would refuse: **`add
//! document` is how a gap in `timezone` or `[applications.*]` gets filled at
//! all**, and a compose that hard-refused during adoption would make the one
//! tool that repairs the gap unable to run.
//!
//! [`provisioning_complete`] is **strict**: `provisioning`, plus `SPEC-05`
//! §5's own gates. It is the one export, a card build, and a live write are
//! required to pass through — anywhere a document leaves this register for
//! good, as opposed to being compared against one.

use rivet_config::Document;
use rivet_core::hostname::Hostname;

use crate::register::{Device, Fleet, FleetRole};

/// Why a document could not be composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// The composed text is not a document `rivet-config` accepts, and this is
    /// what it said.
    ///
    /// **The refusal is the machine's own**, quoted rather than paraphrased:
    /// an operator who fixes what this says has fixed what the board would have
    /// said on first boot.
    Refused { detail: String, text: String },
    /// The record has nothing to compose a reachable machine from.
    ///
    /// Not a parse failure — an empty document is valid TOML and a valid
    /// document. It is refused because of what it *means*: the example file
    /// says it plainly, *"without this document, the card boots and nothing can
    /// reach it"*.
    Unreachable { id: String, missing: &'static str },
    /// The record composes, but not into a FINAL document — `SPEC-05` §5
    /// names something the role requires and this record does not answer.
    ///
    /// **Only [`provisioning_complete`] returns this.** [`provisioning`]
    /// itself stays permissive, because `crate::main`'s `compare` — the
    /// engine behind `add document` — has to keep composing a record exactly
    /// this incomplete so that adoption can fill the gap in. A refusal here
    /// is the gate in front of an export, a card, or a live write, not in
    /// front of a diff.
    Incomplete { id: String, missing: Vec<String> },
}

impl core::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused { detail, text } => write!(
                f,
                "the composed document is not one rivet-config accepts, so nothing has been \
                 written. This is the SAME parser the board runs, and this is what it said:\n\
                 \x20 {detail}\n\
                 What was composed:\n{text}"
            ),
            Self::Unreachable { id, missing } => write!(
                f,
                "the record for {id} has no {missing}, and a card burned from a document \
                 without one boots and cannot be reached - there is no keyboard on a \
                 headless board and ADR-018 5.1's rollback needs somebody to cause a reset. \
                 Nothing has been composed. Add it to the device's record, or to the fleet's."
            ),
            Self::Incomplete { id, missing } => {
                writeln!(
                    f,
                    "the record for {id} composes, but not into a document SPEC-05 5 accepts as \
                     FINAL - nothing has been exported, built onto a card, or sent to a machine. \
                     `rivet-fleet add document --device {id} --confirm` fills a gap the machine \
                     itself already answers; a gap it does not is `set` at the command line, or a \
                     ruling for the fleet."
                )?;
                for one in missing {
                    writeln!(f, "\x20 missing  {one}")?;
                }
                Ok(())
            }
        }
    }
}

/// The hostname this device's document should carry, out of its record.
///
/// **The register's label IS the hostname**, when it can be one. `ADR-025`
/// §4.1 makes the human label a mutable alias and the identifier authoritative,
/// and a hostname is exactly a mutable alias — so a second `hostname` field on
/// the record would be two spellings of one fact (`ADR-011` §3), and a device
/// called `kitchen-pi` in the register that answered to something else on the
/// network is the confusion the register exists to remove.
///
/// **`None` when the label is not a valid hostname**, and it is omitted rather
/// than refused: a label may legitimately be `Someone's kitchen Pi`, and a
/// register that would not compose a document because of a space in a label
/// would be enforcing a network grammar on a human note. The machine then
/// derives its own name (`rivet_core::hostname::derived`), and
/// `rivet-fleet explain device` is where an operator is told which happened.
#[must_use]
pub fn hostname(device: &Device) -> Option<Hostname> {
    Hostname::new(&device.name)
}

/// The effective control-plane pins for a device: its own, then the fleet's.
///
/// **The device's come first and duplicates are dropped**, keeping the first
/// occurrence — so a key present in both appears once, in the position the
/// device gave it. The device is the more specific statement, so it leads.
///
/// **Order carries no meaning here and is kept stable anyway.**
/// `rivetos_tls::parse_pins` builds a set and asks only whether a key is in it,
/// so nothing depends on position — but a composition that reordered itself
/// between runs would make every `peers` file a spurious diff, and the register
/// is meant to be diffable (`ADR-034` §3.4).
#[must_use]
pub fn control_plane_keys(device: &Device, fleet: &Fleet) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in device
        .control_plane_keys
        .iter()
        .chain(fleet.control_plane_keys.iter())
    {
        if !out.iter().any(|seen| seen == key) {
            out.push(key.clone());
        }
    }
    out
}

/// The `peers` file for a device, as `cmd/rivet-mcpd` reads it.
///
/// One hex key per line, `#` for comments. **The comments are the reason this
/// is composed rather than concatenated**: a bare column of hex is
/// unreviewable, and a `peers` file is the list of who may install payloads and
/// reboot the machine. Each key is labelled with where it came from, so an
/// operator reading a card can tell a fleet-wide key from one this device was
/// given on its own.
#[must_use]
pub fn peers(device: &Device, fleet: &Fleet) -> String {
    let mut out = String::new();
    out.push_str("# The Ed25519 public keys allowed to reach this machine's control plane.
");
    out.push_str("# Composed by rivet-fleet from the register - ADR-025 2, ADR-035 7.2.
");
    out.push_str(&format!("# Device: {} ({})
", device.name, device.id));

    if control_plane_keys(device, fleet).is_empty() {
        out.push_str(
            "#
             # NO KEYS. rivet-mcpd refuses to start with an empty pin file rather than
             # listening for a caller it could never accept - see rivetos_tls::Error::NoPins.
",
        );
        return out;
    }

    for key in &device.control_plane_keys {
        out.push_str(&format!("
# this device's own
{key}
"));
    }
    for key in &fleet.control_plane_keys {
        if device.control_plane_keys.iter().any(|k| k == key) {
            continue;
        }
        out.push_str(&format!("
# the fleet's
{key}
"));
    }
    out
}

/// The effective memory caps: the fleet's, then the device's own on top.
///
/// The same two-layer merge [`services`] does and for the same reason
/// (`ADR-007` §3.4's layer 3) — a fleet-wide cap is a default and a device's
/// own record wins, per service rather than per table.
#[must_use]
pub fn service_memory(
    device: &Device,
    fleet: &Fleet,
) -> std::collections::BTreeMap<String, String> {
    let mut out = fleet.service_memory.clone();
    for (name, cap) in &device.service_memory {
        out.insert(name.clone(), cap.clone());
    }
    out
}

/// The effective carried-version pins: the fleet's, then the device's own on
/// top — `.rulings/` R-160 R26.
///
/// **Two layers, not three.** [`services`] falls through role default, then
/// fleet, then device, because a role knows which PRODUCTS it carries. A role
/// does not know which VERSION (`ADR-050` §5 — *"no role knows a version"*),
/// so there is no role layer here to fall through first: a fleet-wide pin is
/// the least specific layer that exists, and a device's own pin wins over it,
/// per product rather than per table — [`service_memory`]'s shape exactly.
#[must_use]
pub fn carry(device: &Device, fleet: &Fleet) -> std::collections::BTreeMap<String, String> {
    let mut out = fleet.carry.clone();
    for (product, version) in &device.carry {
        out.insert(product.clone(), version.clone());
    }
    out
}

/// Where a device's effective carry pin for `product` came from —
/// [`service_source`]'s footing, narrowed to [`carry`]'s two layers.
///
/// **No role layer to report, and that is not an omission.** `carry`'s own
/// doc says why: no role knows a version. So unlike `service_source`, which
/// falls all the way back to `"its role"`, this has only two answers — a
/// caller asking about a product neither layer names is asking a question
/// `carry`'s own result already refused to answer (it would not be in the
/// map at all), so there is no third arm here for that case to reach.
#[must_use]
pub fn carry_source(device: &Device, fleet: &Fleet, product: &str) -> &'static str {
    if device.carry.contains_key(product) {
        "this device"
    } else if fleet.carry.contains_key(product) {
        "the fleet"
    } else {
        "neither - no pin is set"
    }
}

/// **The application that is the machine's front door** — `ADR-046`, and the
/// one service `SPEC-05` §5 requires an `upstream-name` for.
const PROXY_SERVICE: &str = "rivet-proxyd";

/// Every interface — what a server or member's control plane must answer on
/// to be administered from elsewhere. `cmd/rivet-provision`'s
/// `REACHABLE_LISTEN`, kept as its own small copy for this module's own
/// reason: see this module's header.
const REACHABLE_LISTEN: &str = "0.0.0.0:7777";

/// Where a role's control plane listens by default, or `None` for a role
/// that has no opinion — mirrors `cmd/rivet-provision`'s `listen_for`,
/// `ADR-004` §6.
///
/// **`None`, not the schema default, when there is nothing to say.** Writing
/// the schema's own loopback default explicitly here would change the bytes
/// [`render`] produces for every device that leaves the role question
/// unanswered, and that byte-stability is a claim `apply document` depends on —
/// see [`Device::listen`]'s own documentation for why absent and default are
/// the same state.
#[must_use]
fn role_listen_default(role: Option<FleetRole>) -> Option<&'static str> {
    match role {
        Some(FleetRole::Server | FleetRole::Member) => Some(REACHABLE_LISTEN),
        None => None,
    }
}

/// What a role turns on by default — mirrors `cmd/rivet-provision`'s
/// `services_for`, `ADR-050` §5's role table: a server runs `rivet-fleetd`,
/// `rivet-proxyd` (its front door) and `rivet-mdnsd` (so the name on its own
/// certificate resolves at all); a member announces itself with
/// `rivet-mdnsd` AND runs the front door, `rivet-proxyd`, exactly as a
/// server does — part of the image since `ADR-060`, so enabling it is all it
/// takes — so it is reached by name by the server that manages it and
/// authenticates a caller rather than serving nothing on 443; a machine with
/// no role stated gets nothing from its role.
///
/// **Fixed 2026-09-13** (`.rulings/` R-159 R21): a member answered
/// `["rivet-mdnsd"]` alone until this ruling, which left this function and
/// `ADR-050` §5 disagreeing about what a member's document enables —
/// `cmd/rivet-provision`'s `services_for` carried the identical defect and was
/// fixed in the same change, so the two composers agree again (this module's
/// own header names the duplication and why it exists).
#[must_use]
const fn role_services(role: Option<FleetRole>) -> &'static [&'static str] {
    match role {
        Some(FleetRole::Server) => &["rivet-fleetd", "rivet-mdnsd", PROXY_SERVICE],
        Some(FleetRole::Member) => &["rivet-mdnsd", PROXY_SERVICE],
        None => &[],
    }
}

/// The effective services: the role's defaults, then the fleet's, then the
/// device's own on top.
///
/// **The device wins per service, not per table**, and the fleet wins over
/// the role for the same reason — a fleet that enables `rivet-timed`
/// everywhere and a device that says `false` gives `false` for that one
/// service and leaves the rest standing, which is the same per-leaf rule
/// `ignition::config` applies to this tool's own configuration, arrived at
/// independently because a shared layer needs exactly this rule to stay
/// usable. The role is the least specific of the three layers and loses to
/// either of the other two, including an explicit `false` — `SPEC-05` §5's
/// defaults are a starting point for a fresh record, not a floor an operator
/// cannot go under.
#[must_use]
pub fn services(device: &Device, fleet: &Fleet) -> std::collections::BTreeMap<String, bool> {
    let mut out: std::collections::BTreeMap<String, bool> = role_services(device.fleet_role)
        .iter()
        .map(|name| ((*name).to_owned(), true))
        .collect();
    for (name, enabled) in &fleet.services {
        out.insert(name.clone(), *enabled);
    }
    for (name, enabled) in &device.services {
        out.insert(name.clone(), *enabled);
    }
    out
}

/// Where a given service's `enabled` came from, for `crate::main`'s
/// `explain device` — the role, the fleet, or this device's own record, in
/// the same order [`services`] resolves them and with the same winner.
#[must_use]
pub fn service_source(device: &Device, fleet: &Fleet, name: &str) -> &'static str {
    if device.services.contains_key(name) {
        "this device"
    } else if fleet.services.contains_key(name) {
        "the fleet"
    } else {
        "its role"
    }
}

/// The value of `[applications.rivet-proxyd] upstream-name`, if this
/// device's own record carries one.
///
/// **Device-only, never the fleet's.** Two machines' proxies answer to two
/// different names, so a fleet-wide default here would be a value every
/// machine would compose identically and none of them correctly.
#[must_use]
pub fn upstream_name(device: &Device) -> Option<&str> {
    device
        .applications
        .get(PROXY_SERVICE)
        .and_then(|settings| settings.get("upstream-name"))
        .map(String::as_str)
}

/// Compose the provisioning document for one device.
///
/// # Errors
///
/// [`ComposeError::Unreachable`] when the record would produce a machine
/// nothing can reach, and [`ComposeError::Refused`] when `rivet-config` will
/// not have the result — see the type for why both are refusals here rather
/// than surprises on a board.
pub fn provisioning(device: &Device, fleet: &Fleet) -> Result<Document, ComposeError> {
    if device.interfaces.is_empty() {
        return Err(ComposeError::Unreachable {
            id: device.id.clone(),
            missing: "network interface",
        });
    }
    let text = render(device, fleet);
    Document::parse(&text).map_err(|e| ComposeError::Refused {
        detail: e.to_string(),
        text,
    })
}

/// The composed document as the bytes a boot partition would carry.
///
/// `Document::to_toml`'s, never this module's own text: `ADR-007` §3.1 makes
/// that rendering canonical, so a register in one state composes one byte
/// string and a diff of two documents is a diff of intent rather than of
/// formatting.
///
/// # Errors
///
/// As [`provisioning`].
pub fn provisioning_toml(device: &Device, fleet: &Fleet) -> Result<String, ComposeError> {
    Ok(provisioning(device, fleet)?.to_toml())
}

/// The FINAL form of a device's document — [`provisioning`], plus `SPEC-05`
/// §5's own completeness gates.
///
/// **Everywhere a document leaves this register for good** — an export, a
/// card build, a live write — calls this and not [`provisioning`]. See this
/// module's header for why the two are different functions rather than one
/// stricter one.
///
/// # Errors
///
/// As [`provisioning`], plus [`ComposeError::Incomplete`] naming every
/// `SPEC-05` §5 requirement this record does not answer:
///
/// * **`timezone`** — always required, so a card never boots to a machine
///   that keeps UTC in silence.
/// * **`[applications.rivet-proxyd] upstream-name`** — required only when
///   [`services`] would enable the proxy, because a document that never
///   turns the proxy on has no dead front door to boot to.
#[must_use = "an Err here means nothing was composed"]
pub fn provisioning_complete(device: &Device, fleet: &Fleet) -> Result<Document, ComposeError> {
    let document = provisioning(device, fleet)?;

    let mut missing = Vec::new();
    if device.timezone.is_none() {
        missing.push(
            "timezone - SPEC-05 5: a machine provisioned without one keeps UTC silently, in \
             every log this record would otherwise be read from"
                .to_owned(),
        );
    }
    let proxy_enabled = services(device, fleet).get(PROXY_SERVICE).copied().unwrap_or(false);
    if proxy_enabled && upstream_name(device).is_none() {
        missing.push(format!(
            "[applications.{PROXY_SERVICE}] upstream-name - SPEC-05 5: {PROXY_SERVICE} is \
             enabled and starts with no default for this, so it exits, retries five times and \
             is marked failed"
        ));
    }
    if missing.is_empty() {
        Ok(document)
    } else {
        Err(ComposeError::Incomplete {
            id: device.id.clone(),
            missing,
        })
    }
}

/// [`provisioning_complete`], as the bytes a boot partition would carry.
///
/// # Errors
///
/// As [`provisioning_complete`].
pub fn provisioning_complete_toml(device: &Device, fleet: &Fleet) -> Result<String, ComposeError> {
    Ok(provisioning_complete(device, fleet)?.to_toml())
}

/// The text this module hands to the parser.
///
/// **Deliberately not canonical and deliberately not public.** Whatever this
/// produces is parsed and re-rendered by `rivet-config`, so its only job is to
/// be *readable by that parser*; making it pretty here would be a second
/// renderer for a format that already has one (`ADR-011`).
fn render(device: &Device, fleet: &Fleet) -> String {
    let mut out = String::new();
    out.push_str("version = 1\n");

    // Before any `[table]` header — a bare key after one belongs to that table.
    if let Some(name) = hostname(device) {
        out.push_str(&format!("hostname = {}\n", quoted(name.as_str())));
    }

    // `timezone` — `SPEC-05` §5, ruled 2026-09-07. Also a bare top-level key,
    // so it has to be written here and nowhere later: a document that ends in
    // a table has no top level left to append to, which is the trap
    // `examples/aura/aura-controller`'s General page walks an operator into.
    // **Absent writes nothing rather than a guess** — see
    // `crate::register::Device::timezone` for why a plausible default is the
    // wrong answer here specifically; [`provisioning_complete`] is where its
    // absence becomes a refusal.
    if let Some(zone) = &device.timezone {
        out.push_str(&format!("timezone = {}\n", quoted(zone)));
    }

    for (name, interface) in &device.interfaces {
        // **Never an interface named `resolver`.** `ADR-034` §6 retired the
        // shape where a resolver was recorded as one - `rivet-register`'s
        // `read_device` already drops a legacy `[interfaces.resolver]` entry
        // on the way in, so this `Device` should never carry one. This is
        // defence in depth rather than the fix itself: a `Device` built some
        // other way (a test, a future caller that skips `read_device`)
        // reaching this loop with the reserved name still present would
        // otherwise recompose exactly the refused table this ADR retired -
        // `[network.resolver] method = "..."`, which `rivet-config` rejects
        // because that table declares `nameservers` and `search`, never
        // `method`.
        if name == rivet_config::resolver::Resolver::TABLE {
            continue;
        }
        out.push_str(&format!("\n[network.{name}]\n"));
        out.push_str(&format!("method = {}\n", quoted(&interface.method)));
        if let Some(address) = &interface.address {
            out.push_str(&format!("address = {}\n", quoted(address)));
        }
        if let Some(gateway) = &interface.gateway {
            out.push_str(&format!("gateway = {}\n", quoted(gateway)));
        }
    }

    // `[network.resolver]` - `ADR-034` §6's field for a resolver, composed
    // here rather than in the loop above because it is not an interface: it
    // has its own home on `Device` precisely so it never has to pass through
    // that loop as one. Written only when the record says something,
    // matching `rivet_config::resolver::Resolver::is_silent` - an empty table
    // and an absent one must render identically, so a register with nothing
    // to say about DNS composes the same bytes it always did and leaves a
    // DHCP lease free to answer instead (`rivet-config/src/resolver.rs`).
    // `.filter` folds the "is there a resolver" check and the "does it say
    // anything" check into one `Option`, rather than one `if let` nested
    // inside another for the same reason `rivet_config::resolver::Resolver`
    // has `is_silent` at all: an empty table and an absent one must compose
    // identically.
    let resolver = device
        .resolver
        .as_ref()
        .filter(|r| !r.nameservers.is_empty() || !r.search.is_empty());
    if let Some(resolver) = resolver {
        out.push_str("\n[network.resolver]\n");
        if !resolver.nameservers.is_empty() {
            let listed: Vec<String> = resolver.nameservers.iter().map(|s| quoted(s)).collect();
            out.push_str(&format!("nameservers = [{}]\n", listed.join(", ")));
        }
        if !resolver.search.is_empty() {
            let listed: Vec<String> = resolver.search.iter().map(|s| quoted(s)).collect();
            out.push_str(&format!("search = [{}]\n", listed.join(", ")));
        }
    }

    // `[control-plane]` — `ADR-034` §6.1, and it is written from the record for
    // the reason that section gives: two places saying where a machine listens
    // is what `ADR-011` exists to forbid, and the disagreement would surface as
    // a fleet tool that cannot reach a board it can see.
    //
    // **The device's own value always wins.** Falling to `role_listen_default`
    // is `SPEC-05` §5's rule — a server or a member binds every interface,
    // because it is administered from elsewhere by definition, and a
    // stand-alone or roleless record keeps the schema's own loopback default
    // by writing nothing at all.
    //
    // **Absent AND roleless writes no table**, which is not a shortcut. A
    // machine with no `[control-plane]` reads `DEFAULT_LISTEN` for itself, so
    // the two states produce the same machine — and every register written
    // before either field existed goes on composing byte-identical documents,
    // which `apply document` compares.
    let listen = device
        .listen
        .as_deref()
        .or_else(|| role_listen_default(device.fleet_role));
    if let Some(listen) = listen {
        out.push_str(&format!("\n[control-plane]\nlisten = {}\n", quoted(listen)));
    }

    // `[kiosk]` — `SPEC-05` §5. Which application takes this machine's
    // display, when its record names one. **Absent writes no table at all**,
    // `[control-plane]`'s rule immediately above: a register written before
    // this field existed goes on composing exactly the bytes it composed
    // yesterday, and `SPEC-05` §5 leaves *nobody has said* deliberately
    // unguessed rather than composing `rivet-launcher` on the strength of it
    // usually being right.
    //
    // **Nothing here checks that the named application will be installed.**
    // `crate::register::Device::kiosk_auto_run` has the reason: that gap is
    // named rather than closed, because nothing at the moment a document is
    // composed can know what a later install will add.
    if let Some(app) = &device.kiosk_auto_run {
        out.push_str(&format!("\n[kiosk]\nauto-run = {}\n", quoted(app)));
    }

    // `[applications.<name>]` — `ADR-038`, `SPEC-05` §5. **Device-only**, for
    // `upstream_name`'s reason: two machines' own settings are two machines'
    // own facts, and a fleet layer here would make one machine's certificate
    // name compose onto every other machine's proxy.
    //
    // **An application with no keys writes no table at all.** An empty
    // `[applications.<name>]` is a real, if pointless, declaration to
    // `rivet-config` (`rivet_config::applications::Applications`), and this
    // register has no use for one: the only way a name reaches
    // `Device::applications` is `add document` copying a key that was there,
    // so an empty inner map cannot happen through this crate's own tools —
    // this guard is for a record edited some other way.
    for (name, settings) in &device.applications {
        if settings.is_empty() {
            continue;
        }
        out.push_str(&format!("\n[applications.{name}]\n"));
        for (key, value) in settings {
            out.push_str(&format!("{key} = {}\n", quoted(value)));
        }
    }

    let caps = service_memory(device, fleet);
    for (name, enabled) in services(device, fleet) {
        out.push_str(&format!("\n[services.{name}]\nenabled = {enabled}\n"));
        // After `enabled`, which is alphabetical and is also the order
        // `rivet-config` documents the two keys in. A cap for a service the
        // register does not name is unreachable — `set service` requires
        // `enabled` — so there is no orphan case to decide here.
        if let Some(cap) = caps.get(&name) {
            out.push_str(&format!("memory = \"{cap}\"\n"));
        }
    }

    // `[carry.<product>]` — `rivet_config::carry`, `.rulings/` R-159 R20 and
    // R-160 R26. The fleet-then-device merge above (`carry`), rendered the way
    // `cmd/rivet-image --fleet-repository` reads it back
    // (`Document::carried_version`). A product this device does not carry is
    // pinned anyway if the record says so — `rivet_config::carry`'s header:
    // *"a stray pin for a product this machine does not enable is inert
    // rather than an error"* — so this loop does not consult [`services`]
    // first.
    for (product, version) in carry(device, fleet) {
        out.push_str(&format!("\n[carry.{product}]\n"));
        out.push_str(&format!("version = {}\n", quoted(&version)));
    }

    // `[repos]` - PLAN.md §47. **Rendered from the device alone**, with no
    // fleet layer: a repository is data on one machine, and a fleet-wide
    // default would mean every machine in the fleet silently creating a
    // repository of the same name, which is four copies of a thing an operator
    // asked for once.
    for (name, id) in &device.repos {
        out.push_str(&format!("\n[repos.{name}]\n"));
        out.push_str(&format!("id = {}\n", quoted(id)));
    }

    out
}

/// A TOML basic string.
///
/// Written here rather than taken from a serialiser because this module emits
/// six values and importing a writer to quote them would be the second TOML
/// authority `ADR-009` §4 refuses. It escapes what TOML requires and nothing
/// more; anything it gets wrong is caught by the parse that follows, which is
/// the property this whole module is built on.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::register::Interface;

    fn device() -> Device {
        let mut device = Device {
            id: "0011223344556677".to_owned(),
            name: "kitchen-pi".to_owned(),
            ..Device::default()
        };
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.1.50/24".to_owned()),
                gateway: Some("192.168.1.1".to_owned()),
            },
        );
        device
    }

    // ---- the machine's own parser is the gate ------------------------------

    /// **The property this module is built on.** A composed document has
    /// already been through the parser the board runs, so a refusal arrives on
    /// a workstation rather than on first boot.
    #[test]
    fn a_composed_document_is_one_the_boards_own_parser_accepted() {
        let composed = provisioning(&device(), &Fleet::default()).expect("a valid composition");
        // And it survives its own rendering, which is what makes it storable.
        let text = composed.to_toml();
        assert_eq!(Document::parse(&text).expect("canonical text"), composed);
    }

    // ---- the resolver, `ADR-034` §6 ----------------------------------------

    /// **The regression this module exists to fix.** A device recording
    /// nameservers composes `[network.resolver] nameservers = [...]`, which is
    /// the shape `rivet-config`'s own `Resolver` accepts - not
    /// `[network.resolver] method = "dhcp"`, which it refuses by name. Checked
    /// by handing the composed text to the real parser, `Document::parse`,
    /// the same one the board runs.
    #[test]
    fn a_devices_nameservers_compose_a_resolver_table_the_real_parser_accepts() {
        let mut device = device();
        device.resolver = Some(crate::register::Resolver {
            nameservers: vec!["192.168.50.1".to_owned(), "1.1.1.1".to_owned()],
            search: vec!["lan".to_owned()],
        });
        let text = provisioning_toml(&device, &Fleet::default()).expect("a valid composition");
        let parsed = Document::parse(&text).expect("the same parser the board runs");
        let resolver = parsed.resolver().expect("a resolver was composed");
        assert_eq!(
            resolver.nameservers(),
            [
                core::net::Ipv4Addr::new(192, 168, 50, 1),
                core::net::Ipv4Addr::new(1, 1, 1, 1)
            ]
        );
        assert_eq!(resolver.search().len(), 1);
        assert_eq!(resolver.search()[0].as_str(), "lan");
    }

    /// A device with no resolver composes no `[network.resolver]` table at
    /// all - the same absent-means-silent rule `crate::register::Resolver`'s
    /// own doc comment states, so a register with no resolver recorded
    /// goes on composing the bytes it always did and a DHCP lease is
    /// left free to answer.
    #[test]
    fn a_device_with_no_resolver_composes_no_resolver_table() {
        let text = provisioning_toml(&device(), &Fleet::default()).expect("a valid composition");
        assert!(!text.contains("resolver"), "{text}");
        let parsed = Document::parse(&text).expect("valid");
        assert!(parsed.resolver().is_none(), "{text}");
    }

    /// **What each slot runs is a record and never composed** (`ADR-052` §7):
    /// a device naming both slots' system updates composes byte for byte the
    /// document it composes without them, so the key never reaches a
    /// machine's strict parser.
    #[test]
    fn a_devices_system_slots_compose_nothing() {
        let without = provisioning_toml(&device(), &Fleet::default()).expect("a valid composition");
        let mut with = device();
        with.system_slots.set("a", &"a1".repeat(32)).expect("a slot");
        with.system_slots.set("b", &"b2".repeat(32)).expect("a slot");
        let text = provisioning_toml(&with, &Fleet::default()).expect("a valid composition");
        assert_eq!(text, without);
        Document::parse(&text).expect("the composed document still parses");
    }

    /// **The defect itself, composed from a `Device` that still carries the
    /// legacy shape.** `rivet-register::read_device` is what actually keeps a
    /// `Device` from reaching this module with a `"resolver"` interface - this
    /// proves the renderer's own defence works even if that guarantee were
    /// ever bypassed, by constructing the bad shape directly rather than going
    /// through a register file. Before the guard in `render`'s interface loop
    /// existed, this composed `[network.resolver] method = "dhcp"` and
    /// `rivet-config` refused it - which is `AGENTS.md` §4's broken-
    /// deliberately check, done here in prose because the guard is one `if`.
    #[test]
    fn an_interface_literally_named_resolver_is_never_rendered() {
        let mut device = device();
        device.interfaces.insert(
            "resolver".to_owned(),
            Interface { method: "dhcp".to_owned(), address: None, gateway: None },
        );
        let text = provisioning_toml(&device, &Fleet::default()).expect("a valid composition");
        assert!(!text.contains("[network.resolver]"), "{text}");
        // And the real interface beside it is untouched.
        assert!(text.contains("[network.eth0]"), "{text}");
    }

    /// A record that would produce a document the parser refuses is a refusal
    /// **here**, carrying what the parser said.
    #[test]
    fn a_record_the_parser_will_not_have_is_refused_with_what_it_said() {
        let mut bad = device();
        bad.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                // `ADR-007` §7.4: an address beside `dhcp` is refused and never
                // resolved by precedence.
                method: "dhcp".to_owned(),
                address: Some("192.168.1.50/24".to_owned()),
                gateway: None,
            },
        );
        let error = provisioning(&bad, &Fleet::default()).expect_err("dhcp with an address");
        match &error {
            ComposeError::Refused { detail, .. } => {
                assert!(detail.contains("address"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
        // The message hands over both halves: what the parser said, and what
        // was composed — because one without the other is unactionable.
        let text = error.to_string();
        assert!(text.contains("SAME parser the board runs"), "{text}");
        assert!(text.contains("dhcp"), "{text}");
    }

    /// A device with no interface composes a card that boots and cannot be
    /// reached — refused before anything is written.
    #[test]
    fn a_device_with_no_interface_is_refused_rather_than_composed() {
        let mut stranded = device();
        stranded.interfaces.clear();
        let error = provisioning(&stranded, &Fleet::default()).expect_err("no interface");
        assert!(error.to_string().contains("network interface"), "{error}");
    }

    // ---- the merge: the device wins, the fleet fills in --------------------

    /// **Per service, not per table.** A device that disables one of the
    /// fleet's services keeps the rest.
    #[test]
    fn a_device_overrides_one_service_and_keeps_the_others() {
        let mut device = device();
        device.services.insert("rivet-mdnsd".to_owned(), false);
        let mut fleet = Fleet::default();
        fleet.services.insert("rivet-mdnsd".to_owned(), true);
        fleet.services.insert("rivet-timed".to_owned(), true);

        let effective = services(&device, &fleet);
        assert_eq!(effective.get("rivet-mdnsd"), Some(&false));
        assert_eq!(effective.get("rivet-timed"), Some(&true));
    }

    /// **`PLAN.md` §66.5, and the reason it was filed.** The register could
    /// describe `enabled` and nothing else, so a document it composed carried
    /// no cap — and `apply document` would have ERASED one an operator had set
    /// on the machine, which is §56.2's drift with the arrow reversed.
    #[test]
    fn a_devices_memory_cap_reaches_the_composed_document() {
        let mut device = device();
        let fleet = Fleet::default();
        device.services.insert("rivet-mdnsd".to_owned(), true);
        device
            .service_memory
            .insert("rivet-mdnsd".to_owned(), "32M".to_owned());

        // Parsed back rather than string-matched, so this asserts what a
        // MACHINE would read out of the document and not what this module
        // happened to print.
        let text = provisioning_toml(&device, &fleet).expect("a valid composition");
        let parsed = Document::parse(&text).expect("canonical text");
        let capped = parsed
            .services()
            .iter()
            .find(|s| s.name().as_str() == "rivet-mdnsd")
            .and_then(|s| s.memory())
            .map(|m| m.to_string());
        assert_eq!(capped.as_deref(), Some("32M"), "{text}");
    }

    /// A service with no cap composes exactly as it always did — the key is
    /// absent, not empty. Silence leaves a thing alone, and
    /// `memory = ""` would be a value rather than silence.
    #[test]
    fn a_service_with_no_cap_gains_no_memory_key_at_all() {
        let mut device = device();
        device.services.insert("rivet-timed".to_owned(), true);
        let text = provisioning_toml(&device, &Fleet::default()).expect("a valid composition");
        assert!(!text.contains("memory"), "{text}");
        let parsed = Document::parse(&text).expect("canonical text");
        let service = parsed
            .services()
            .iter()
            .find(|s| s.name().as_str() == "rivet-timed")
            .map(|s| s.memory().is_none());
        assert_eq!(service, Some(true), "{text}");
    }

    /// The same two layers `services` has, and the same winner: `ADR-007`
    /// §3.4's layer 3 is a default and the device's own record beats it, per
    /// service rather than per table.
    #[test]
    fn a_devices_cap_beats_the_fleets_and_the_fleets_applies_otherwise() {
        let mut device = device();
        let mut fleet = Fleet::default();
        device.services.insert("rivet-timed".to_owned(), true);
        device.services.insert("rivet-mdnsd".to_owned(), true);
        fleet
            .service_memory
            .insert("rivet-timed".to_owned(), "64M".to_owned());
        fleet
            .service_memory
            .insert("rivet-mdnsd".to_owned(), "64M".to_owned());
        device
            .service_memory
            .insert("rivet-timed".to_owned(), "16M".to_owned());

        let effective = service_memory(&device, &fleet);
        assert_eq!(effective.get("rivet-timed").map(String::as_str), Some("16M"));
        assert_eq!(effective.get("rivet-mdnsd").map(String::as_str), Some("64M"));
    }

    /// And the merge reaches the document rather than stopping at the map.
    #[test]
    fn the_merged_services_are_in_the_composed_document() {
        let mut device = device();
        device.services.insert("rivet-mdnsd".to_owned(), false);
        let mut fleet = Fleet::default();
        fleet.services.insert("rivet-timed".to_owned(), true);

        let text = provisioning_toml(&device, &fleet).expect("a valid composition");
        let parsed = Document::parse(&text).expect("canonical text");
        let named: Vec<(String, bool)> = parsed
            .services()
            .iter()
            .map(|s| (s.name().to_string(), s.enabled()))
            .collect();
        let named: Vec<(&str, bool)> = named.iter().map(|(n, e)| (n.as_str(), *e)).collect();
        assert!(named.contains(&("rivet-mdnsd", false)), "{named:?}");
        assert!(named.contains(&("rivet-timed", true)), "{named:?}");
    }

    // ---- carried-version pins, `.rulings/` R-160 R26 -----------------------

    /// The same two-layer merge `service_memory` has, and no role layer
    /// beneath it: `ADR-050` §5 says no role knows a version, so a
    /// fleet-wide pin is the least specific layer that exists here.
    #[test]
    fn a_devices_pin_beats_the_fleets_and_the_fleets_applies_otherwise() {
        let mut device = device();
        let mut fleet = Fleet::default();
        fleet.carry.insert("rivet-fleetd".to_owned(), "0.1.85".to_owned());
        fleet.carry.insert("hello-rivet".to_owned(), "0.4.2".to_owned());
        device.carry.insert("rivet-fleetd".to_owned(), "0.1.86".to_owned());

        let effective = carry(&device, &fleet);
        assert_eq!(effective.get("rivet-fleetd").map(String::as_str), Some("0.1.86"));
        assert_eq!(effective.get("hello-rivet").map(String::as_str), Some("0.4.2"));
    }

    /// And the merge reaches the composed document, which is what
    /// `cmd/rivet-image --fleet-repository` actually reads
    /// (`Document::carried_version`).
    #[test]
    fn the_merged_carry_pins_are_in_the_composed_document() {
        let mut device = device();
        device.carry.insert("rivet-fleetd".to_owned(), "0.1.86".to_owned());
        let mut fleet = Fleet::default();
        fleet.carry.insert("hello-rivet".to_owned(), "0.4.3".to_owned());

        let composed = provisioning(&device, &fleet).expect("a valid composition");
        assert_eq!(composed.carried_version("rivet-fleetd"), Some("0.1.86"));
        assert_eq!(composed.carried_version("hello-rivet"), Some("0.4.3"));
    }

    /// No pin anywhere composes no `[carry]` table at all — the same
    /// absent-means-silent rule every other optional table in this module
    /// follows.
    #[test]
    fn no_pin_composes_no_carry_entry() {
        let text = provisioning_toml(&device(), &Fleet::default()).expect("a valid composition");
        assert!(!text.contains("carry"), "{text}");
        let parsed = Document::parse(&text).expect("canonical text");
        assert!(parsed.carry().is_empty(), "{text}");
    }

    /// A pin for a product this device's role does not carry is inert rather
    /// than an error — `rivet_config::carry`'s own rule, reached here through
    /// a real composition rather than asserted only at the parser.
    #[test]
    fn a_pin_for_a_product_not_carried_still_composes_and_is_inert() {
        let mut fleet = Fleet::default();
        fleet.carry.insert("rivet-proxyd".to_owned(), "0.2.0".to_owned());
        let composed = provisioning(&device(), &fleet).expect("a valid composition");
        assert_eq!(composed.carried_version("rivet-proxyd"), Some("0.2.0"));
        // This fixture device carries no service at all, so the pin names a
        // product nothing here will ever read — exactly the inertness
        // `rivet_config::carry`'s header describes.
        assert!(composed.carried_services().is_empty());
    }

    // ---- the name a machine answers to -------------------------------------

    /// **The register's label IS the hostname.** A device called `kitchen-pi`
    /// in the register that answered to something else on the network is the
    /// confusion the register exists to remove.
    #[test]
    fn the_registers_label_becomes_the_machines_name() {
        let composed = provisioning(&device(), &Fleet::default()).expect("valid");
        assert_eq!(
            composed.hostname().map(rivet_core::hostname::Hostname::as_str),
            Some("kitchen-pi")
        );
    }

    /// **A label that is not a hostname is omitted, not refused.** A label may
    /// legitimately be a human note; a register that would not compose a
    /// document because of a space in one would be enforcing a network grammar
    /// on prose. The machine then derives its own name.
    #[test]
    fn a_label_that_is_not_a_hostname_is_omitted_and_the_document_still_composes() {
        let mut noted = device();
        noted.name = "Someone's kitchen Pi".to_owned();
        assert_eq!(hostname(&noted), None);
        let composed = provisioning(&noted, &Fleet::default())
            .expect("a document still composes for a device with a prose label");
        assert_eq!(composed.hostname(), None);
    }

    /// It survives the canonical rendering, which is what the board reads back
    /// — and it is written before any table header, or it would belong to one.
    #[test]
    fn the_name_survives_the_rendering_and_is_not_swallowed_by_a_table() {
        let text = provisioning_toml(&device(), &Fleet::default()).expect("valid");
        assert!(text.contains("hostname = \"kitchen-pi\""), "{text}");
        let before_any_table = text
            .split_once("[")
            .map_or(text.as_str(), |(head, _)| head);
        assert!(
            before_any_table.contains("hostname"),
            "the name is below a table header, so it belongs to that table: {text}"
        );
    }
    // ---- canonical, and therefore diffable ---------------------------------

    /// `ADR-007` §3.1 through this module: one register state, one byte string.
    #[test]
    fn one_register_state_composes_one_byte_string() {
        let fleet = Fleet::default();
        assert_eq!(
            provisioning_toml(&device(), &fleet).expect("valid"),
            provisioning_toml(&device(), &fleet).expect("valid")
        );
    }

    /// `ADR-034` §3, at the last place a secret could escape: whatever the
    /// register held, the composed document carries no private material. The
    /// register has nowhere to put one (`crate::register`'s own test); this is
    /// the other end of the same claim.
    #[test]
    fn a_composed_document_carries_nothing_secret() {
        let fleet = Fleet {
            control_plane_keys: vec![PIN_A.to_owned()],
            ..Fleet::default()
        };
        let text = provisioning_toml(&device(), &fleet).expect("valid");
        for forbidden in ["PRIVATE KEY", "passphrase", "seed", "psk", "pmk"] {
            assert!(
                !text.to_ascii_lowercase().contains(&forbidden.to_ascii_lowercase()),
                "the composed document carries {forbidden:?}: {text}"
            );
        }
    }

    // ---- control-plane pins, ADR-025 §2 ------------------------------------

    const PIN_A: &str = "aa00112233445566778899aabbccddeeff00112233445566778899aabbccddee";
    const PIN_B: &str = "bb00112233445566778899aabbccddeeff00112233445566778899aabbccddee";

    /// The device's own come first, then the fleet's, and a key in both appears
    /// once — the merge rule this module states, applied to the pins.
    #[test]
    fn a_devices_pins_come_before_the_fleets_and_duplicates_collapse() {
        let mut device = device();
        device.control_plane_keys.push(PIN_A.to_owned());
        let mut fleet = Fleet::default();
        fleet.control_plane_keys.push(PIN_B.to_owned());
        fleet.control_plane_keys.push(PIN_A.to_owned());

        assert_eq!(control_plane_keys(&device, &fleet), vec![PIN_A.to_owned(), PIN_B.to_owned()]);
    }

    /// **The file says where each key came from.** A `peers` file is the list of
    /// who may install payloads and reboot the machine; a bare column of hex is
    /// unreviewable, which is the whole reason this is composed rather than
    /// concatenated.
    #[test]
    fn the_peers_file_labels_each_key_with_its_source() {
        let mut device = device();
        device.control_plane_keys.push(PIN_A.to_owned());
        let mut fleet = Fleet::default();
        fleet.control_plane_keys.push(PIN_B.to_owned());

        let text = peers(&device, &fleet);
        let own = text.find(PIN_A).expect("the device's own key");
        let fleets = text.find(PIN_B).expect("the fleet's key");
        assert!(own < fleets, "the device's key must come first:\n{text}");
        assert!(text.contains("# this device's own"), "{text}");
        assert!(text.contains("# the fleet's"), "{text}");
        assert!(text.contains("kitchen-pi"), "the file does not name the device:\n{text}");
    }

    /// **What it composes is what the board parses.** The two live in different
    /// crates and a disagreement would be a card that boots and refuses
    /// everyone, so the board's own parser is the judge here.
    #[test]
    fn what_is_composed_is_what_rivetos_tls_reads_back() {
        let mut device = device();
        device.control_plane_keys.push(PIN_A.to_owned());
        let mut fleet = Fleet::default();
        fleet.control_plane_keys.push(PIN_B.to_owned());

        let parsed = rivetos_tls::parse_pins(&peers(&device, &fleet))
            .expect("the board must be able to read what the fleet composes");
        let rendered: Vec<String> = parsed.iter().map(|p| rivetos_tls::encode_hex(p)).collect();
        assert_eq!(rendered, vec![PIN_A.to_owned(), PIN_B.to_owned()]);
    }

    /// **An empty pin set produces a file that says so**, rather than an empty
    /// one an operator would read as "not written yet". `rivet-mcpd` refuses to
    /// start on it either way — `rivetos_tls::Error::NoPins` — and the comment
    /// is what tells a human why before they go looking at the board.
    #[test]
    fn a_device_nobody_may_reach_gets_a_file_that_explains_itself() {
        let text = peers(&device(), &Fleet::default());
        assert!(text.contains("NO KEYS"), "{text}");
        assert!(text.contains("refuses to start"), "{text}");
        assert_eq!(
            rivetos_tls::parse_pins(&text).expect("still parseable"),
            Vec::<rivetos_tls::Pin>::new(),
            "a comment-only file must parse to no keys rather than fail"
        );
    }

    // ---- SPEC-05 5's role defaults ------------------------------------------

    fn complete(device: &mut Device) {
        device.timezone = Some("America/Chicago".to_owned());
    }

    /// **A server binds every interface, and writes so even though nothing on
    /// the device record asked for it.** `SPEC-05` §5's first bullet, applied
    /// through the role rather than through an explicit `listen`.
    #[test]
    fn a_server_binds_every_interface_by_default() {
        let mut server = device();
        server.fleet_role = Some(FleetRole::Server);
        assert_eq!(server.listen, None, "the role, not an explicit value, must do this");
        let text = provisioning_toml(&server, &Fleet::default()).expect("valid");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(parsed.effective_control_plane().listen(), REACHABLE_LISTEN);
    }

    /// A member gets the same reachable listener a server does — `SPEC-05`
    /// §5: both are administered from elsewhere by definition.
    #[test]
    fn a_member_binds_every_interface_by_default_too() {
        let mut member = device();
        member.fleet_role = Some(FleetRole::Member);
        let text = provisioning_toml(&member, &Fleet::default()).expect("valid");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(parsed.effective_control_plane().listen(), REACHABLE_LISTEN);
    }

    /// **A machine with no role at all keeps loopback** — writing nothing,
    /// which is `Device::listen`'s own byte-stability claim: every register
    /// written before `fleet_role` existed must go on composing exactly what
    /// it composed yesterday.
    #[test]
    fn a_roleless_device_writes_no_control_plane_table() {
        let mut plain = device();
        plain.fleet_role = None;
        let text = provisioning_toml(&plain, &Fleet::default()).expect("valid");
        assert!(!text.contains("[control-plane]"), "{text}");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(
            parsed.effective_control_plane().listen(),
            rivet_config::control_plane::DEFAULT_LISTEN
        );
    }

    /// **The device's own `listen` always wins over the role's default** —
    /// the merge rule this module opens with, extended to the third layer.
    #[test]
    fn an_explicit_listen_beats_the_roles_default() {
        let mut member = device();
        member.fleet_role = Some(FleetRole::Member);
        member.listen = Some("192.168.1.9:7777".to_owned());
        let text = provisioning_toml(&member, &Fleet::default()).expect("valid");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(parsed.effective_control_plane().listen(), "192.168.1.9:7777");
    }

    /// **A server turns on `rivet-fleetd`, `rivet-proxyd` and `rivet-mdnsd`;
    /// a member turns on `rivet-proxyd` and `rivet-mdnsd`, but not
    /// `rivet-fleetd`** — `ADR-050` §5's role table, the same services
    /// `cmd/rivet-provision`'s `services_for` turns on for the same two
    /// answers. **Fixed 2026-09-13** (`.rulings/` R-159 R21): a member's
    /// `rivet-proxyd` was `None` here until this ruling, which `ADR-050` §5
    /// never asked for.
    #[test]
    fn a_server_and_a_member_get_their_own_default_service_sets() {
        let server = services(&Device { fleet_role: Some(FleetRole::Server), ..device() }, &Fleet::default());
        for name in ["rivet-fleetd", PROXY_SERVICE, "rivet-mdnsd"] {
            assert_eq!(server.get(name), Some(&true), "{name}: {server:?}");
        }

        let member = services(&Device { fleet_role: Some(FleetRole::Member), ..device() }, &Fleet::default());
        for name in [PROXY_SERVICE, "rivet-mdnsd"] {
            assert_eq!(member.get(name), Some(&true), "{name}: {member:?}");
        }
        assert_eq!(member.get("rivet-fleetd"), None, "a member does not run fleetd: {member:?}");
    }

    /// **A machine with no role at all gets nothing from its role.** The
    /// role is a starting point for a fresh record, not a service list every
    /// device is measured against.
    #[test]
    fn a_roleless_device_gets_no_default_services() {
        let plain = Device { fleet_role: None, ..device() };
        assert!(services(&plain, &Fleet::default()).is_empty());
    }

    /// **The device beats the role, even to turn a role default OFF** —
    /// `SPEC-05` §5's defaults are a starting point, not a floor an operator
    /// cannot go under.
    #[test]
    fn a_devices_own_false_beats_the_roles_default_true() {
        let mut server = device();
        server.fleet_role = Some(FleetRole::Server);
        server.services.insert(PROXY_SERVICE.to_owned(), false);
        let effective = services(&server, &Fleet::default());
        assert_eq!(effective.get(PROXY_SERVICE), Some(&false));
        assert_eq!(effective.get("rivet-fleetd"), Some(&true), "the rest of the role stands");
    }

    /// And the fleet, the middle layer, beats the role the same way.
    #[test]
    fn the_fleets_answer_beats_the_roles_default_too() {
        let mut fleet = Fleet::default();
        fleet.services.insert(PROXY_SERVICE.to_owned(), false);
        let server = Device { fleet_role: Some(FleetRole::Server), ..device() };
        assert_eq!(services(&server, &fleet).get(PROXY_SERVICE), Some(&false));
    }

    // ---- SPEC-05 5: application settings, and `upstream-name` --------------

    /// An application's settings compose as their own table, and round-trip
    /// through the board's own parser.
    #[test]
    fn a_devices_application_settings_compose_and_parse() {
        let mut d = device();
        d.applications
            .entry(PROXY_SERVICE.to_owned())
            .or_default()
            .insert("upstream-name".to_owned(), "rivet-01.example.arpa".to_owned());
        d.applications
            .entry("tempmon-rivet".to_owned())
            .or_default()
            .insert("sensor-labels".to_owned(), "GVH5075_1111=fridge".to_owned());
        let text = provisioning_toml(&d, &Fleet::default()).expect("valid");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(
            parsed.applications().get(PROXY_SERVICE).and_then(|a| a.get("upstream-name")),
            Some("rivet-01.example.arpa")
        );
        assert_eq!(
            parsed.applications().get("tempmon-rivet").and_then(|a| a.get("sensor-labels")),
            Some("GVH5075_1111=fridge")
        );
    }

    /// `upstream_name` reads it back out the same way the composed document
    /// carries it.
    #[test]
    fn upstream_name_reads_what_the_record_holds_for_the_proxy_and_nothing_else() {
        let mut d = device();
        assert_eq!(upstream_name(&d), None);
        d.applications
            .entry(PROXY_SERVICE.to_owned())
            .or_default()
            .insert("upstream-name".to_owned(), "arbor.example.arpa".to_owned());
        assert_eq!(upstream_name(&d), Some("arbor.example.arpa"));
    }

    /// An application named with an empty settings map — a shape only a
    /// record edited outside this crate's own tools could produce — writes no
    /// table at all, rather than the pointless declaration `rivet-config`
    /// would otherwise accept.
    #[test]
    fn an_application_with_no_settings_writes_no_table() {
        let mut d = device();
        d.applications.insert("nothing-here".to_owned(), std::collections::BTreeMap::new());
        let text = provisioning_toml(&d, &Fleet::default()).expect("valid");
        assert!(!text.contains("nothing-here"), "{text}");
    }

    // ---- SPEC-05 5: `timezone` -----------------------------------------------

    /// `timezone` composes as a bare top-level key, ahead of every table -
    /// the same placement `cmd/rivet-provision` uses and for the same reason:
    /// there is no appending to a document that ends in a table.
    #[test]
    fn timezone_composes_before_the_first_table() {
        let mut d = device();
        d.timezone = Some("America/Chicago".to_owned());
        let text = provisioning_toml(&d, &Fleet::default()).expect("valid");
        let before_any_table = text.split_once('[').map_or(text.as_str(), |(head, _)| head);
        assert!(before_any_table.contains("timezone"), "{text}");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(
            parsed.timezone().map(rivet_core::timezone::Timezone::as_str),
            Some("America/Chicago")
        );
    }

    // ---- SPEC-05 5: `[kiosk] auto-run` --------------------------------------

    /// A device naming an application composes `[kiosk] auto-run = "…"`, the
    /// shape `rivet-config`'s own `Kiosk` reads - the regression `rivet-01`
    /// could not be rebuilt from its record until `Device::kiosk_auto_run`
    /// existed to hold this at all.
    #[test]
    fn kiosk_auto_run_composes_the_application_the_record_names() {
        let mut d = device();
        d.kiosk_auto_run = Some("aura-renderer".to_owned());
        let text = provisioning_toml(&d, &Fleet::default()).expect("valid");
        assert!(text.contains("[kiosk]"), "{text}");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(
            parsed.kiosk().and_then(rivet_config::kiosk::Kiosk::auto_run),
            Some("aura-renderer")
        );
    }

    /// **Absent writes no table at all** - the same byte-stability every
    /// other optional field on `Device` keeps, so a register with nothing
    /// said about `SPEC-05` §5's kiosk question composes exactly what it
    /// always did.
    #[test]
    fn a_device_with_no_kiosk_auto_run_composes_no_kiosk_table() {
        let d = device();
        assert_eq!(d.kiosk_auto_run, None);
        let text = provisioning_toml(&d, &Fleet::default()).expect("valid");
        assert!(!text.contains("kiosk"), "{text}");
    }

    // ---- SPEC-05 5: `provisioning_complete`'s gates -------------------------

    /// **The property `provisioning` and `provisioning_complete` must NOT
    /// share.** A record missing `timezone` still composes for `compare` -
    /// `add document`'s engine - because that is exactly the gap adoption has
    /// to be able to fill. If this refused too, `add document` could never
    /// run against a record it exists to repair.
    #[test]
    fn the_permissive_composer_does_not_require_a_timezone() {
        let plain = device();
        assert_eq!(plain.timezone, None);
        provisioning(&plain, &Fleet::default()).expect("compare's engine must not refuse here");
    }

    /// **The strict composer refuses without a timezone**, and names it.
    #[test]
    fn the_strict_composer_refuses_without_a_timezone() {
        let plain = device();
        let error =
            provisioning_complete(&plain, &Fleet::default()).expect_err("no timezone recorded");
        match &error {
            ComposeError::Incomplete { missing, .. } => {
                assert!(missing.iter().any(|m| m.contains("timezone")), "{missing:?}");
            }
            other => panic!("{other:?}"),
        }
        assert!(error.to_string().contains("timezone"), "{error}");
    }

    /// **The strict composer refuses a proxy with no `upstream-name`** -
    /// `SPEC-05` §5's sharpest bullet: a document that enables the proxy and
    /// omits it is worse than one that never enables the proxy at all.
    #[test]
    fn the_strict_composer_refuses_an_enabled_proxy_with_no_upstream_name() {
        let mut d = device();
        complete(&mut d);
        d.services.insert(PROXY_SERVICE.to_owned(), true);
        let error =
            provisioning_complete(&d, &Fleet::default()).expect_err("no upstream-name recorded");
        match &error {
            ComposeError::Incomplete { missing, .. } => {
                assert!(missing.iter().any(|m| m.contains("upstream-name")), "{missing:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// The same refusal fires when the ROLE is what turns the proxy on,
    /// never just an explicit `[services.rivet-proxyd]`.
    #[test]
    fn the_upstream_name_gate_fires_from_a_roles_default_proxy_too() {
        let mut server = device();
        complete(&mut server);
        server.fleet_role = Some(FleetRole::Server);
        let error = provisioning_complete(&server, &Fleet::default())
            .expect_err("the role enabled the proxy and nothing named an upstream");
        match &error {
            ComposeError::Incomplete { missing, .. } => {
                assert!(missing.iter().any(|m| m.contains("upstream-name")), "{missing:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// **A record with no proxy enabled at all is never asked for
    /// `upstream-name`** - the gate is conditional on the effective service
    /// set, not a blanket requirement.
    #[test]
    fn a_device_with_no_proxy_enabled_is_not_asked_for_an_upstream_name() {
        let mut d = device();
        complete(&mut d);
        assert_eq!(services(&d, &Fleet::default()).get(PROXY_SERVICE), None);
        provisioning_complete(&d, &Fleet::default())
            .expect("nothing here should be asked for a proxy setting");
    }

    /// **Both gates satisfied, from a role default plus an application
    /// setting, compose a complete document** - `SPEC-05` §5's whole
    /// acceptance for the register-side composer, end to end.
    #[test]
    fn a_complete_server_record_composes_a_final_document() {
        let mut server = device();
        complete(&mut server);
        server.fleet_role = Some(FleetRole::Server);
        server
            .applications
            .entry(PROXY_SERVICE.to_owned())
            .or_default()
            .insert("upstream-name".to_owned(), "rivet-01.example.arpa".to_owned());

        let text = provisioning_complete_toml(&server, &Fleet::default())
            .expect("a complete record composes");
        let parsed = Document::parse(&text).expect("canonical text");
        assert_eq!(parsed.effective_control_plane().listen(), REACHABLE_LISTEN);
        assert_eq!(
            parsed.timezone().map(rivet_core::timezone::Timezone::as_str),
            Some("America/Chicago")
        );
        assert_eq!(
            parsed.applications().get(PROXY_SERVICE).and_then(|a| a.get("upstream-name")),
            Some("rivet-01.example.arpa")
        );
        let enabled: Vec<&str> = parsed
            .services()
            .iter()
            .filter(|s| s.enabled())
            .map(|s| s.name().as_str())
            .collect();
        for name in ["rivet-fleetd", PROXY_SERVICE, "rivet-mdnsd"] {
            assert!(enabled.contains(&name), "{name} missing from {enabled:?}");
        }
    }

    /// **Both reasons are named in one refusal** when both are missing -
    /// `ADR-034` §6.2's rule that an operator who discovers refusals one at a
    /// time stops before the last, applied to this gate too.
    #[test]
    fn both_missing_reasons_are_named_together() {
        let mut server = device();
        server.fleet_role = Some(FleetRole::Server);
        let error = provisioning_complete(&server, &Fleet::default())
            .expect_err("neither timezone nor upstream-name is recorded");
        let text = error.to_string();
        assert!(text.contains("timezone"), "{text}");
        assert!(text.contains("upstream-name"), "{text}");
    }

    // ---- `service_source`, for `explain device` -----------------------------

    /// A service on with no explicit answer anywhere but the role reports so.
    #[test]
    fn service_source_names_the_role_when_neither_layer_above_it_answered() {
        let server = Device { fleet_role: Some(FleetRole::Server), ..device() };
        assert_eq!(service_source(&server, &Fleet::default(), "rivet-fleetd"), "its role");
    }

    /// And the device and the fleet are still reported correctly once a role
    /// is in the mix, in the same precedence order `services` resolves.
    #[test]
    fn service_source_still_prefers_the_device_then_the_fleet_over_the_role() {
        let mut fleet = Fleet::default();
        fleet.services.insert("rivet-timed".to_owned(), true);
        let mut server = device();
        server.fleet_role = Some(FleetRole::Server);
        server.services.insert(PROXY_SERVICE.to_owned(), false);
        assert_eq!(service_source(&server, &fleet, PROXY_SERVICE), "this device");
        assert_eq!(service_source(&server, &fleet, "rivet-timed"), "the fleet");
        assert_eq!(service_source(&server, &fleet, "rivet-mdnsd"), "its role");
    }

    // ---- `carry_source`, for `explain device` and `list carry` -
    // `.rulings/` R-162 R30 -----------------------------------------------

    /// A product neither layer names has no pin, and that is said outright
    /// rather than guessed at — `carry`'s own map would not contain it.
    #[test]
    fn carry_source_names_neither_when_no_pin_is_set() {
        assert_eq!(carry_source(&device(), &Fleet::default(), "hello-rivet"), "neither - no pin is set");
    }

    /// A fleet-wide pin with no device override reports the fleet.
    #[test]
    fn carry_source_names_the_fleet_when_only_it_pins() {
        let mut fleet = Fleet::default();
        fleet.carry.insert("hello-rivet".to_owned(), "0.4.2".to_owned());
        assert_eq!(carry_source(&device(), &fleet, "hello-rivet"), "the fleet");
    }

    /// A device's own pin wins over a fleet-wide one, `carry`'s own
    /// precedence — `service_source_still_prefers_the_device_then_the_fleet`'s
    /// shape, minus the role layer `carry` never has.
    #[test]
    fn carry_source_prefers_the_device_over_the_fleet() {
        let mut fleet = Fleet::default();
        fleet.carry.insert("hello-rivet".to_owned(), "0.4.2".to_owned());
        let mut mine = device();
        mine.carry.insert("hello-rivet".to_owned(), "0.4.3".to_owned());
        assert_eq!(carry_source(&mine, &fleet, "hello-rivet"), "this device");
    }
}
