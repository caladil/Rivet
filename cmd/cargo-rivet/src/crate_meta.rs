//! Reading the *user's* `Cargo.toml` — `ADR-015` §3.2's cargo integration.
//!
//! > Declared in the standard metadata extension point, so it is inert to every
//! > other tool:
//! >
//! > ```toml
//! > [package.metadata.rivet]
//! > kind      = "daemon"
//! > ```
//!
//! The whole of this module is a translation: `[package.metadata.rivet]`'s
//! table into a [`Manifest`], and `[package]`'s `name` and `version` into the
//! artifact's identity. **Nothing here decides what a manifest means** — the
//! enumerated values, the defaults and the canonical text all belong to
//! `rivet_core::package::application`, which is also what reads them back on
//! a machine.
//!
//! # Unknown keys are refused
//!
//! `rivet-config`'s parser refuses an unknown key *"so that a misspelled
//! `verison` is reported as the typo it is rather than as a missing
//! `version`"*, and this file has a sharper reason: the manifest **is** the
//! permission request (`ADR-015` §3.3). A key this build silently dropped would
//! be an author asking for something and being told nothing, and the artifact
//! would ship declaring less than its author wrote.

use std::path::Path;

use rivet_core::package::application::{
    BackupFormat, BackupSurface, Cpu, Device, Endpoint, Manifest, Material, Memory, Network,
    ProgramKind, Ready, Restart, Socket, StateGrant,
};
use rivet_core::package::library::Reference;
use toml_edit::{DocumentMut, Item, Table};

/// What `Cargo.toml` says.
pub struct CrateMeta {
    pub name: String,
    /// The single `[[bin]] name`, when the manifest declares exactly one.
    ///
    /// **This, not [`Self::name`], is what an application is called on a
    /// machine** — see `product_name`. `None` when the manifest declares no
    /// `[[bin]]` (cargo then names the binary after the package) or more than
    /// one, because with several there is nothing to pick and `--bin` is how
    /// the author says which.
    pub bin_name: Option<String>,
    pub version: String,
    pub manifest: Manifest,
}

/// **What an application is CALLED on a machine.**
///
/// The binary's name if the manifest declares one, otherwise the package's.
///
/// # Why not `[package] name`, which is what this used to be
///
/// `ADR-033` §4 puts every library crate in the `rivetos-` namespace **because
/// `rivet-*` on crates.io belongs to rivet.gg** — `rivet-core`, `rivet-http`
/// and `rivet-logger` are all taken. The binary is `rivet-<thing>`; the package
/// is `rivetos-<thing>`; §4's table says the package name is seen by
/// *"crates.io, and nothing else"*.
///
/// That last part stopped being true when `cargo rivet package` started using
/// it as the artifact's `product`, which becomes **the directory under
/// `/state/app`, the `[services.<name>]` table an operator writes, and the
/// target of `rivet apply install-secret`.** So the crates.io identity was
/// leaking onto the machine, and the two names diverged in exactly the place a
/// person types one.
///
/// **The consequence was not cosmetic.** `rivetos-fleetd` builds the binary
/// `rivet-fleetd`, the board has `/state/app/rivet-fleetd`, and
/// `cargo rivet package` produced an artifact addressed to `rivetos-fleetd` —
/// so installing it created a second, unused directory and reported success
/// while the running daemon kept its old build. `PLAN.md` §129.5 records that
/// as why port 443 still spoke TLS 1.2 after the rest of the fleet had stopped.
///
/// Taking the binary name gives both properties at once: the package stays out
/// of rivet.gg's namespace and the machine sees the name an operator uses.
/// `cmd/rivet-proxyd` had been carrying `rivet-proxyd` as its PACKAGE name to
/// buy the second property, at the cost of the first; ruled 2026-09-04 and now
/// `rivetos-proxyd` like its siblings.
#[must_use]
pub fn product_name(meta: &CrateMeta) -> &str {
    meta.bin_name.as_deref().unwrap_or(&meta.name)
}

/// The keys `[package.metadata.rivet]` may hold — `ADR-015` §3.2's table.
///
/// **`exec` is deliberately not among them**, and its absence is a rule rather
/// than an omission: *"a service definition that could name an arbitrary
/// program would let a signed manifest launch something the signature never
/// covered."* An author who reaches for it is told so by name, below.
pub(crate) const DECLARED: [&str; 23] = [
    "kind", "restart", "ready", "network", "state", "memory", "listen", "provides", "connects",
    // A name for a person -- PLAN.md 104.42.
    "title",
    // **Shared Rivets, 2026-08-31.** The declared closure — every library
    // artifact this application needs, by the reference
    // `cargo rivet library` printed when it emitted one.
    "library",
    // **`ADR-015` §6.2's device capability, 2026-08-31** — kiosk mode. A CLASS
    // and never a path: see `rivet_core::package::application::Device`.
    "devices",
    // **`ADR-038` §3's declaration, 2026-09-02 — and MISSING from this list
    // until 2026-09-03.** `rivet_core`'s `KEYS` gained it the day it was
    // designed and this list did not, so the machine understood a key no
    // author could ask for. That is worse than an unbuilt feature: an
    // application declaring no settings **accepts none**, so the supervisor
    // delivered nothing, correctly, for ever — and `ADR-038` §8's *absent is
    // unset* made the silence indistinguishable from a machine nobody had
    // configured. Found by Reliquary's port, which declared two retention
    // settings and got the built-in defaults with no complaint from anything.
    "settings",
    // **`R-133`'s path route, 2026-09-03** — where `rivet-proxyd` mounts this
    // application under the machine's one name.
    "proxy-path",
    // **`R-152`'s per-route gate, 2026-09-05** — `ADR-046` §4.3. `operator`
    // (the default, and what an absent key means) or `none`.
    //
    // **This list is what makes the key declarable at all.** `Manifest::parse`
    // learned `proxy-auth` first, and every application still refused it here
    // with *"[package.metadata.rivet] has no key"* — because the tool that
    // reads a crate's table is a second gate, and a key the core understands
    // is not a key an author can write until BOTH know it.
    "proxy-auth",
    // **`ADR-048` §4's per-path rules, 2026-09-07** — which prefixes UNDER that
    // route ask something different of a caller, as `["none:pair"]`.
    //
    // Declared here in the same change as `rivet_core`'s `KEYS`, because
    // `proxy-auth` was not: the core understood a key for a day that no author
    // could write, and the refusal an author met named the wrong thing.
    "proxy-auth-paths",
    // **`ADR-051`'s backup surface, 2026-09-08** — how an application OFFERS
    // its data to whoever is backing this machine up. `ADR-030` rules
    // application data out of Rivet's own backup, so an application that wants
    // its data kept says so here.
    //
    // Declared in the same change as `rivet_core`'s `KEYS`, for the reason
    // `proxy-auth-paths` was: a key the core understands is not a key an author
    // can write until BOTH ends know it, and `proxy-auth` spent a day in that
    // state with the refusal an author met naming the wrong thing.
    "backup-path",
    "backup-format",
    "backup-feed",
    // **`ADR-015` §3's `cpu` request, added to `rivet_core`'s `KEYS`
    // 2026-09-09 (`PLAN.md` §172) and missing from THIS list — and from the
    // usage text below — until `.rulings/` R-162 caught it.** `memory`'s
    // footing exactly: optional, and its absence means no particular share
    // of the processor was asked for.
    "cpu",
    // **`PLAN-Feature-Set-2026-09-12` §6.2's addon primitive, `ADR-053`,
    // 2026-09-13 — a LIST since `.rulings/` R-162 R28** — the classes of
    // library this application accepts.
    "accepts-class",
    // **`ADR-051` §12's restore route, `.rulings/` R-183 D1, 2026-09-14** —
    // where this application accepts its own backup back. Declared in the same
    // change as `rivet_core`'s `KEYS`, for `proxy-auth-paths`' reason.
    "restore-path",
    // **`ADR-055` §3's per-machine material, `.rulings/` R-194, 2026-09-15**
    // — the KINDS of material this application needs placed on the machine
    // before it can work, as `["fleet-tls"]`. Declared in the same change as
    // `rivet_core`'s `KEYS`, for `proxy-auth-paths`' reason: the core
    // understood `proxy-auth` for a day that no author could write, and the
    // refusal an author met named the wrong thing.
    "material",
];

pub fn read(path: &Path) -> Result<CrateMeta, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{} is not valid TOML: {e}", path.display()))?;

    let package = doc.get("package").and_then(Item::as_table).ok_or_else(|| {
        format!(
            "{} has no [package] table; a virtual manifest is a workspace, not something to \
                 package",
            path.display()
        )
    })?;

    let name = string(package, "name")?.to_string();
    let version = version_of(package, path)?;

    let table = package
        .get("metadata")
        .and_then(Item::as_table)
        .and_then(|m| m.get("rivet"))
        .and_then(Item::as_table)
        .ok_or_else(|| {
            format!(
                "{} has no [package.metadata.rivet] table, so there is nothing saying what this \
                 application is (ADR-015 §3.2)",
                path.display()
            )
        })?;

    for (key, _) in table.iter() {
        if !DECLARED.contains(&key) {
            let hint = if key == "exec" {
                ". There is deliberately no `exec` field: the binary IS the crate's binary, and a \
                 manifest that could name an arbitrary program would launch something the \
                 signature never covered (ADR-015 §3.2)"
            } else {
                ""
            };
            return Err(format!(
                "[package.metadata.rivet] has no key {key:?}{hint}"
            ));
        }
    }

    let kind = match required(table, "kind")? {
        "daemon" => ProgramKind::Daemon,
        "oneshot" => ProgramKind::Oneshot,
        "socket-activated" => ProgramKind::SocketActivated,
        other => return Err(one_of("kind", other, "daemon, oneshot, socket-activated")),
    };

    let mut manifest = Manifest::new(kind);

    if let Some(text) = optional(table, "restart")? {
        manifest = manifest.with_restart(match text {
            "always" => Restart::Always,
            "on-failure" => Restart::OnFailure,
            "never" => Restart::Never,
            other => return Err(one_of("restart", other, "always, on-failure, never")),
        });
    }
    if let Some(text) = optional(table, "ready")? {
        manifest = manifest.with_ready(match text {
            "none" => Ready::None,
            "notify" => Ready::Notify,
            "listening" => Ready::Listening,
            other => return Err(one_of("ready", other, "none, notify, listening")),
        });
    }
    if let Some(text) = optional(table, "network")? {
        manifest = manifest.with_network(match text {
            "none" => Network::None,
            "egress" => Network::Egress,
            other => match other.split_once(':') {
                Some(("group", group)) if !group.is_empty() && !group.contains(':') => {
                    Network::Group(group.to_string())
                }
                _ => return Err(one_of("network", other, "none, egress, group:<name>")),
            },
        });
    }
    if let Some(text) = optional(table, "state")? {
        manifest = manifest.with_state(match text {
            "none" => StateGrant::None,
            "private" => StateGrant::Private,
            other => return Err(one_of("state", other, "none, private")),
        });
    }
    if let Some(text) = optional(table, "memory")? {
        let memory = Memory::parse(text)
            .map_err(|fault| format!("[package.metadata.rivet] memory: {fault}"))?;
        manifest = manifest.with_memory(Some(memory));
    }
    // **`ADR-015` §3's `cpu` request, added to the manifest 2026-09-09
    // (`PLAN.md` §172) — missing from this reader and from the usage text
    // until now, a defect `.rulings/` R-162 named alongside R28 and R30.**
    // `memory`'s footing exactly: absent means no particular share of the
    // processor was asked for.
    if let Some(text) = optional(table, "cpu")? {
        let cpu = Cpu::parse(text).map_err(|fault| format!("[package.metadata.rivet] cpu: {fault}"))?;
        // **Refused at authoring time — `.rulings/` R-210 D22.** A quota this
        // fine parses (the grammar is shared with the desired-state document,
        // which is allowed to tighten a manifest's request further still) but
        // cgroup v2 will not keep it, and a signed manifest carrying one is
        // refused every boot with no cause named. `CpuLimit::parse` itself is
        // not tightened: a manifest already installed with this cap must keep
        // reading back.
        if let Some(limit) = rivet_core::service::CpuLimit::of(&cpu)
            && limit.is_below_kernel_minimum()
        {
            return Err(format!(
                "[package.metadata.rivet] cpu: {text:?} — {}",
                limit.kernel_refusal()
            ));
        }
        manifest = manifest.with_cpu(Some(cpu));
    }

    let listen = list(table, "listen")?;
    let mut sockets = Vec::new();
    for entry in &listen {
        sockets.push(socket(entry)?);
    }
    manifest = manifest
        .with_listen(sockets)
        .map_err(|fault| format!("[package.metadata.rivet] listen: {fault}"))?;

    let mut provides = Vec::new();
    for entry in &list(table, "provides")? {
        provides.push(Endpoint::parse("provides", entry).map_err(|_| {
            format!(
                "[package.metadata.rivet] provides: {entry:?} is not `socket:<name>` \
                     (ADR-015 §5.3)"
            )
        })?);
    }
    manifest = manifest
        .with_provides(provides)
        .map_err(|fault| format!("[package.metadata.rivet] provides: {fault}"))?;

    let mut connects = Vec::new();
    for entry in &list(table, "connects")? {
        connects.push(Endpoint::parse("connects", entry).map_err(|_| {
            format!(
                "[package.metadata.rivet] connects: {entry:?} is not \
                     `<application>:<socket>` (ADR-015 §5.3)"
            )
        })?);
    }
    manifest = manifest
        .with_title(match table.get("title") {
            None => None,
            Some(item) => Some(
                item.as_str()
                    .ok_or_else(|| {
                        "[package.metadata.rivet] title: must be a string -- the name a person \
                         should see, such as \"Aura\" for a service called `aura-renderer`"
                            .to_string()
                    })?
                    .to_string(),
            ),
        })
        .map_err(|fault| format!("[package.metadata.rivet] title: {fault}"))?
        .with_connects(connects)
        .map_err(|fault| format!("[package.metadata.rivet] connects: {fault}"))?;

    // **Shared Rivets.** A reference is copied out of `cargo rivet library`'s
    // report and never composed by hand, so the refusal here says where one
    // comes from rather than describing a grammar.
    let mut closure = Vec::new();
    for entry in &list(table, "library")? {
        closure.push(Reference::parse(entry).map_err(|fault| {
            format!(
                "[package.metadata.rivet] library: {entry:?} is not a library reference: \
                 {fault}.\n\nA reference is the line `cargo rivet library` prints when it \
                 emits one — <name>-<version>-<64 hex>. It names exact bytes, which is why \
                 it is copied rather than written."
            )
        })?);
    }
    manifest = manifest
        .with_library(closure)
        .map_err(|fault| format!("[package.metadata.rivet] library: {fault}"))?;

    // `ADR-015` §6.2. The refusal names every class this build defines, because
    // an author who guessed `screen` or `keyboard` has no other way to find out
    // what the vocabulary is.
    let mut devices = Vec::new();
    for entry in &list(table, "devices")? {
        devices.push(Device::parse(entry).ok_or_else(|| {
            format!(
                "[package.metadata.rivet] devices: {entry:?} is not a device class. This build \
                 defines {} (ADR-015 §6.2). A class is deliberately not a path — a manifest \
                 that could name a device would be one that could ask for /dev/mem",
                Device::ALL
                    .iter()
                    .map(|d| format!("`{d}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            )
        })?);
    }
    manifest = manifest.with_devices(devices);

    // `ADR-038` §3. The refusal quotes the rule rather than restating it, and
    // it names the KEY that was wrong rather than saying the list is bad —
    // an author with six settings needs to know which one.
    let declared = list(table, "settings")?;
    for entry in &declared {
        if !rivet_core::package::application::is_setting_name(entry) {
            return Err(format!(
                "[package.metadata.rivet] settings: {entry:?} is not a setting name — {}",
                rivet_core::package::application::SETTING_RULE
            ));
        }
        // `ADR-038` §5, `.rulings/` R-169: named here, at packaging, so an
        // author sees it on a workstation rather than the supervisor
        // refusing the same manifest on a board — `with_settings` below
        // enforces the same rule and is what a manifest read off a card is
        // checked against, since it never runs through this file.
        if rivet_core::package::application::is_credential_like_name(entry) {
            return Err(format!(
                "[package.metadata.rivet] settings: {entry:?} looks like a credential — {}",
                rivet_core::package::application::CREDENTIAL_SETTING_RULE
            ));
        }
    }
    manifest = manifest
        .with_settings(declared)
        .map_err(|fault| format!("[package.metadata.rivet] settings: {fault}"))?;

    // **`R-133`.** Absent means NOT PROXIED, which is why this is `optional`
    // and has no default: a default of the crate's own name would mount every
    // application on the machine the moment a proxy was enabled.
    //
    // A reserved path (`ADR-046` §4) has a legal shape, so it is named with
    // its own reason rather than the shape rule.
    let asked = optional(table, "proxy-path")?.map(ToString::to_string);
    manifest = manifest.with_proxy_path(asked.clone()).map_err(|fault| match fault {
        rivet_core::package::application::ManifestFault::ReservedProxyPath => format!(
            "[package.metadata.rivet] proxy-path: {:?}: {fault}",
            asked.as_deref().unwrap_or_default()
        ),
        _ => format!(
            "[package.metadata.rivet] proxy-path: {fault} — {}",
            rivet_core::package::application::PROXY_PATH_RULE
        ),
    })?;

    // **The gate, and it only ever relaxes** — `ADR-046` §4.3. An absent key is
    // `operator`, so every application already built is gated the moment a
    // proxy starts checking, without being rebuilt. A key an author must
    // remember in order to TIGHTEN security is one an author forgets.
    if let Some(text) = optional(table, "proxy-auth")? {
        let auth = rivet_core::package::application::ProxyAuth::parse(text).ok_or_else(|| {
            format!(
                "[package.metadata.rivet] proxy-auth: `{text}` is not a value this understands. \
                 It is `operator` — require the caller's operator certificate, which is also what \
                 leaving the key out means — or `none`, which serves this application to anybody \
                 who reaches the proxy. `app` and `plain` (ADR-048 §4) answer for a PREFIX under \
                 proxy-auth-paths, never for the whole route."
            )
        })?;
        // **`ADR-046` §4's grammar for the bare key is `operator` and
        // `none` only.** `app` and `plain` exist to answer for a prefix
        // (`ADR-048` §4) — refused here in words, before `Manifest::parse`
        // would refuse it with a key name.
        if !auth.allowed_for_whole_route() {
            return Err(format!(
                "[package.metadata.rivet] proxy-auth: `{text}` answers for a PREFIX, not a whole \
                 route. Move it into proxy-auth-paths as `{text}:<prefix>`, or use `operator` or \
                 `none` here."
            ));
        }
        manifest = manifest.with_proxy_auth(auth);
    }

    // **`ADR-048` §4 — the same gate, per prefix.** The refusal names the entry
    // that was wrong and quotes the prefix rule, because an author with four
    // rules needs to know which one; and it says which half of `<auth>:<prefix>`
    // it could not read, because the two failures have different fixes.
    let declared_rules = list(table, "proxy-auth-paths")?;
    let mut rules = Vec::new();
    for entry in &declared_rules {
        let rule = rivet_core::package::application::ProxyRule::parse(entry).ok_or_else(|| {
            format!(
                "[package.metadata.rivet] proxy-auth-paths: {entry:?} is not a rule. Each is \
                 `<auth>:<prefix>` — `none` to serve that prefix to anybody who reaches the \
                 proxy, `operator` to require the caller's operator certificate there even where \
                 the application relaxed the rest, `app` to forward it like `none` while saying \
                 this application checks its own credential here (ADR-048 §4, ADR-061 §7), or \
                 `plain` to also serve it over plain HTTP on port 80 (ADR-061 §7). The prefix is \
                 relative to this application's proxy-path, and {}",
                rivet_core::package::application::PROXY_PREFIX_RULE
            )
        })?;
        rules.push(rule);
    }
    manifest = manifest
        .with_proxy_auth_paths(rules)
        .map_err(|fault| format!("[package.metadata.rivet] proxy-auth-paths: {fault}"))?;

    // **`ADR-051`'s backup surface, and the three keys are TWO shapes.**
    //
    // The refusals here are written out rather than delegated to
    // `Manifest::parse`, because this is where the author is: `parse` reads a
    // build product and answers with a key name, and an author who wrote
    // `backup-path` and forgot `backup-format` needs to be told which key and
    // why, in front of the Cargo.toml they just edited.
    let backup_path = optional(table, "backup-path")?.map(ToString::to_string);
    let backup_format = optional(table, "backup-format")?;
    let backup_feed = optional(table, "backup-feed")?.map(ToString::to_string);
    // **`ADR-051` §12.** Refused here in words, for the reason above, before
    // `Manifest::parse` would refuse it with a key name.
    let restore_path = optional(table, "restore-path")?.map(ToString::to_string);
    if restore_path.is_some() && (backup_path.is_none() || backup_format.is_none()) {
        return Err(if backup_feed.is_some() {
            "[package.metadata.rivet] declares restore-path beside backup-feed. A restore sends \
             back what ONE fetch of `backup-path` returned, and `cargo rivet restore` checks it \
             against `backup-format` before sending; a feed has no format and is not one fetch \
             (ADR-051 §12)."
                .to_string()
        } else {
            "[package.metadata.rivet] declares restore-path without backup-path and \
             backup-format. A restore accepts back what `backup-path` serves, and `cargo rivet \
             restore` checks the file against `backup-format` before sending it — with no \
             backup declared there is nothing to check it against (ADR-051 §12)."
                .to_string()
        });
    }
    let surface = match (backup_path, backup_format, backup_feed) {
        (None, None, None) => None,
        (Some(path), Some(format), None) => {
            let format = BackupFormat::parse(format).ok_or_else(|| {
                format!(
                    "[package.metadata.rivet] backup-format: `{format}` is not a format this \
                     build knows. It is one of {}. The format is declared HERE rather than \
                     known by whatever fetches the backup, so that an application migrating \
                     from one to another carries its own answer across the version boundary \
                     (ADR-051).",
                    BackupFormat::ALL
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            Some(BackupSurface::Artifact {
                path,
                format,
                restore: restore_path.clone(),
            })
        }
        (None, None, Some(path)) => Some(BackupSurface::Feed { path }),
        (Some(_), None, None) => {
            return Err("[package.metadata.rivet] backup-path is set and backup-format is not. \
                        They are one declaration: without a format the only thing a client can \
                        check about what it fetched is that as many bytes arrived as were \
                        promised (ADR-051)."
                .to_string());
        }
        (None, Some(_), None) => {
            return Err("[package.metadata.rivet] backup-format is set and backup-path is not, \
                        so it describes something nothing can reach. Add the route the backup \
                        is served at, relative to proxy-path (ADR-051)."
                .to_string());
        }
        _ => {
            return Err("[package.metadata.rivet] declares both backup shapes. `backup-path` \
                        with `backup-format` is ONE fetch returning the whole thing; \
                        `backup-feed` is a resumable collection with a cursor and a low-water \
                        mark. They are separate keys because that is a PROTOCOL difference \
                        rather than a format one, and a client written for one cannot read the \
                        other (ADR-051)."
                .to_string());
        }
    };
    // **A backup surface with no route is unreachable.** Refused here as well
    // as in `Manifest::parse`, because this is where the author can fix it —
    // and the same defect as enabling `rivet-proxyd` with no `upstream-name`
    // (`SPEC-05` §5).
    if surface.is_some() && manifest.proxy_path().is_none() {
        return Err("[package.metadata.rivet] declares a backup surface and no proxy-path. A \
                    backup path is RELATIVE to the route this application is mounted at, so \
                    with no route it names nothing that can be reached (ADR-051)."
            .to_string());
    }
    manifest = manifest.with_backup(surface).map_err(|fault| {
        format!(
            "[package.metadata.rivet] backup path: {fault} — {}",
            rivet_core::package::application::BACKUP_PATH_RULE
        )
    })?;
    // **An open restore route is refused before anything is signed** —
    // `ADR-051` §12. Asked of the manifest rather than recomputed here, because
    // the answer is the proxy's own longest-match over `proxy-auth` and
    // `proxy-auth-paths`, which are both set above.
    if let Some(path) = restore_path.as_deref()
        && !manifest.requires_operator_at(path)
    {
        return Err(format!(
            "[package.metadata.rivet] restore-path = {path:?} would be served WITHOUT an operator \
             certificate. {}. Add `operator:{path}` to proxy-auth-paths, or move the route \
             somewhere proxy-auth already gates.",
            rivet_core::package::application::RESTORE_AUTH_RULE
        ));
    }

    // **A `:direct` flag the machine would refuse is refused before anything is
    // signed** — `ADR-046` §7, `.rulings/` R-188. Asked of the manifest, after
    // `listen` and `proxy-path` are both set, for `restore-path`'s reason: the
    // answer depends on both keys, and the machine asks the same question.
    if manifest.refuses_direct() {
        let flagged: Vec<String> = manifest
            .listen()
            .iter()
            .filter(|s| s.is_direct())
            .map(|s| s.render())
            .collect();
        return Err(match manifest.proxy_path() {
            Some(path) => format!(
                "[package.metadata.rivet] listen: the first tcp port is `:direct`, and it is the \
                 port rivet-proxyd forwards `/{path}` to. {}. Remove `:direct` from it, or put the \
                 routed port first and the direct one after it.",
                rivet_core::package::application::DIRECT_RULE
            ),
            None => format!(
                "[package.metadata.rivet] listen: {} declared `:direct` and there is no \
                 proxy-path. {}. Remove `:direct`.",
                flagged.join(", "),
                rivet_core::package::application::DIRECT_RULE
            ),
        });
    }

    // **`PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053` — a LIST since
    // `.rulings/` R-162 R28.** An absent key is an empty list and means TAKES
    // NO ADDON, `devices`' and `settings`' footing: a default class would
    // give every application built from now on a capability it never asked
    // for.
    manifest = manifest
        .with_accepts_classes(list(table, "accepts-class")?)
        .map_err(|fault| {
            format!(
                "[package.metadata.rivet] accepts-class: {fault} — {}",
                rivet_core::package::application::CLASS_RULE
            )
        })?;

    // **`ADR-055` §3, `.rulings/` R-194.** `devices`' shape and its refusal:
    // the vocabulary is the platform's, so a token this build does not define
    // is named back with every kind it does — an author who guessed `tls` has
    // no other way to find out what the words are. A KIND and never a file: see
    // `rivet_core::package::application::Material`.
    let mut material = Vec::new();
    for entry in &list(table, "material")? {
        material.push(Material::parse(entry).ok_or_else(|| {
            format!(
                "[package.metadata.rivet] material: {entry:?} is not a kind of per-machine \
                 material. This build defines {} (ADR-055 §3). A kind is deliberately not a \
                 file list — the files of a kind travel together, and the day a fourth joins the \
                 set every manifest declaring the kind gets it",
                Material::ALL
                    .iter()
                    .map(|m| format!("`{m}` ({})", m.files().join(", ")))
                    .collect::<Vec<_>>()
                    .join(" and ")
            )
        })?);
    }
    manifest = manifest.with_material(material);

    // **The application's own name, checked at packaging** — `.rulings/`
    // R-210 D21. `not_a_reserved_name` is the same rule the install door
    // applies (`rivet_core::package::verify::not_a_reserved_name`), named
    // here directly rather than through a package header: this build has not
    // signed anything yet, only read a manifest.
    let bin_name = single_bin_name(&doc);
    let product = bin_name.as_deref().unwrap_or(&name);
    if rivet_core::service::is_reserved_application_name(product) {
        return Err(format!(
            "[package] name: {product:?} is a name this project's own machinery already means \
             something else by — an account or group this image defines, a word the uninstall \
             grammar reads as an instruction, or the deploy, supervisor, kernel-log or ingest \
             service name. One name cannot have two meanings (ADR-011). Rebuild the package \
             under a different name."
        ));
    }

    Ok(CrateMeta {
        name,
        bin_name,
        version,
        manifest,
    })
}

/// The one `[[bin]] name`, or `None`.
///
/// `None` for no `[[bin]]` at all — cargo then names the binary after the
/// package, so the two already agree — and `None` for more than one, because
/// with several there is nothing to pick and `--bin` is how the author says
/// which. Both cases fall back to the package name in [`product_name`], which
/// is what this did unconditionally before 2026-09-04.
fn single_bin_name(doc: &DocumentMut) -> Option<String> {
    let bins = doc.get("bin")?.as_array_of_tables()?;
    if bins.len() != 1 {
        return None;
    }
    Some(bins.get(0)?.get("name")?.as_str()?.to_string())
}

fn socket(entry: &str) -> Result<Socket, String> {
    let bad = || {
        format!(
            "[package.metadata.rivet] listen: {entry:?} is not `tcp:<port>`, `udp:<port>` or \
             `unix:<name>`, with `:direct` allowed after a port. Rivet opens the socket and \
             passes the descriptor, so a port here is a request for one rather than a permission \
             to bind (ADR-015 §5.2)"
        )
    };
    // The port and the `:direct` flag are read by `rivet_core`'s own function,
    // so this reader and the machine's cannot disagree about the grammar.
    use rivet_core::package::application::parse_port_and_reach;
    match entry.split_once(':') {
        Some(("tcp", rest)) => match parse_port_and_reach(rest).ok_or_else(bad)? {
            (port, false) => Ok(Socket::Tcp(port)),
            (port, true) => Ok(Socket::TcpDirect(port)),
        },
        // Same rule as `tcp`, and for the same reason — see `bad()` above.
        Some(("udp", rest)) => match parse_port_and_reach(rest).ok_or_else(bad)? {
            (port, false) => Ok(Socket::Udp(port)),
            (port, true) => Ok(Socket::UdpDirect(port)),
        },
        Some(("unix", name)) if !name.is_empty() && !name.contains(':') && !name.contains('/') => {
            Ok(Socket::Unix(name.to_string()))
        }
        _ => Err(bad()),
    }
}

/// The crate's version, resolving `version.workspace = true`.
///
/// # Why this is not just `string(package, "version")`
///
/// It was, until 2026-09-03, and the refusal it produced was
/// *"\[package\] has no string version"* — which is true and useless, because the
/// key is right there and says `version.workspace = true`.
///
/// **A workspace member inheriting its version is the ordinary case, not an
/// exotic one.** `cmd/rivet-smbd` carries a literal with a comment explaining
/// that this tool refuses an inherited one and that the literal is provisional;
/// it had only ever packaged `hello-app`, a detached fixture, so nothing had
/// found it. The first real workspace member to be packaged — Reliquary's Rivet
/// composition root — found it immediately.
///
/// **Telling the author to write a literal is the wrong fix**, and Reliquary is
/// the case that shows why: its `[workspace.package] version` carries a comment
/// about a version/tag drift that reached an install on 2026-08-07, and a
/// second place to write the number is a second place for it to be wrong.
/// `ADR-027` §3.2.1 makes an artifact's name a claim; a claim assembled from a
/// number that duplicates another number is the failure that ADR is about.
///
/// # What it does
///
/// A literal wins. Otherwise, if `version` is a table saying `workspace = true`,
/// walk **up** from this `Cargo.toml` for the first manifest carrying a
/// `[workspace]` table and take `[workspace.package] version` from it. That is
/// cargo's own rule, restated rather than shelled out to: this tool reads
/// `Cargo.toml` as plain TOML on purpose and running `cargo metadata` to learn
/// one string would make packaging depend on a resolver.
///
/// # Errors
///
/// Naming what was looked for and where, because every failure here is a
/// question about *which* file was expected to carry the number.
fn version_of(package: &Table, path: &Path) -> Result<String, String> {
    // A literal, which is what most crates have and what every packaged
    // artifact had before this function existed.
    if let Some(literal) = package.get("version").and_then(Item::as_str) {
        return Ok(literal.to_string());
    }

    let inherits = package
        .get("version")
        .and_then(Item::as_table_like)
        .and_then(|t| t.get("workspace"))
        .and_then(|w| w.as_value())
        .and_then(toml_edit::Value::as_bool)
        .unwrap_or(false);

    if !inherits {
        return Err(format!(
            "{} has no string `version` in [package], and its `version` is not \
             `version.workspace = true` either",
            path.display()
        ));
    }

    let mut dir = path.parent();
    while let Some(here) = dir {
        let candidate = here.join("Cargo.toml");
        // The crate's own manifest can itself be the workspace root — a
        // single-crate workspace is legal and `[workspace]` sits beside
        // `[package]`. So this loop does not skip the starting directory.
        if let Ok(text) = std::fs::read_to_string(&candidate)
            && let Ok(doc) = text.parse::<DocumentMut>()
            && doc.get("workspace").is_some()
        {
            return doc
                .get("workspace")
                .and_then(Item::as_table)
                .and_then(|w| w.get("package"))
                .and_then(Item::as_table)
                .and_then(|p| p.get("version"))
                .and_then(Item::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| {
                    format!(
                        "{} says `version.workspace = true`, and the workspace at {} has no \
                         string `version` in [workspace.package] to inherit",
                        path.display(),
                        candidate.display()
                    )
                });
        }
        dir = here.parent();
    }

    Err(format!(
        "{} says `version.workspace = true`, and no Cargo.toml with a [workspace] table was \
         found in any parent directory to inherit it from",
        path.display()
    ))
}

fn string<'a>(table: &'a Table, key: &str) -> Result<&'a str, String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .ok_or_else(|| format!("[package] has no string {key}"))
}

fn required<'a>(table: &'a Table, key: &'static str) -> Result<&'a str, String> {
    optional(table, key)?.ok_or_else(|| {
        format!("[package.metadata.rivet] must say {key}; there is no default for it")
    })
}

fn optional<'a>(table: &'a Table, key: &'static str) -> Result<Option<&'a str>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("[package.metadata.rivet] {key} must be a string")),
    }
}

fn list(table: &Table, key: &'static str) -> Result<Vec<String>, String> {
    let Some(item) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| format!("[package.metadata.rivet] {key} must be an array of strings"))?;
    let mut out = Vec::new();
    for value in array {
        let text = value.as_str().ok_or_else(|| {
            format!("[package.metadata.rivet] {key} must be an array of strings")
        })?;
        out.push(text.to_string());
    }
    Ok(out)
}

fn one_of(key: &str, found: &str, admitted: &str) -> String {
    format!("[package.metadata.rivet] {key} = {found:?}; it must be one of {admitted}")
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Build a `[package]` table from TOML text, for `version_of`.
    fn package_of(text: &str) -> Table {
        text.parse::<DocumentMut>()
            .expect("the fixture is valid TOML")
            .get("package")
            .and_then(Item::as_table)
            .expect("the fixture has a [package] table")
            .clone()
    }

    fn doc_of(text: &str) -> DocumentMut {
        text.parse::<DocumentMut>().expect("the fixture is valid TOML")
    }

    fn meta_named(name: &str, bin: Option<&str>) -> CrateMeta {
        CrateMeta {
            name: name.to_string(),
            bin_name: bin.map(str::to_string),
            version: "0.1.0".to_string(),
            manifest: Manifest::new(rivet_core::package::application::ProgramKind::Daemon),
        }
    }

    /// **An application is called on a machine what its BINARY is called.**
    ///
    /// `ADR-033` §4 keeps packages in the `rivetos-` namespace because
    /// `rivet-*` on crates.io is rivet.gg's. The binary is `rivet-<thing>`. The
    /// product taking the package name put the crates.io identity into
    /// `/state/app`, into `[services.<name>]` and into
    /// `rivet apply install-secret` — the three places an operator types it.
    ///
    /// The consequence was not cosmetic: `rivetos-fleetd` produced an artifact
    /// addressed to `rivetos-fleetd` while the board had
    /// `/state/app/rivet-fleetd`, so an install created a second unused
    /// directory and reported success. `PLAN.md` §129.5.
    #[test]
    fn the_product_is_the_binary_name_when_the_two_differ() {
        assert_eq!(
            product_name(&meta_named("rivetos-fleetd", Some("rivet-fleetd"))),
            "rivet-fleetd",
            "the crates.io identity reached the machine, which is the defect"
        );
        assert_eq!(
            product_name(&meta_named("rivetos-proxyd", Some("rivet-proxyd"))),
            "rivet-proxyd"
        );
        // The ordinary case, where a crate names its binary after itself.
        assert_eq!(product_name(&meta_named("hello-rivet", None)), "hello-rivet");
        assert_eq!(
            product_name(&meta_named("hello-rivet", Some("hello-rivet"))),
            "hello-rivet"
        );
    }

    /// **Against the real manifest, not a fixture.** A fixture proves the
    /// parser; this proves it against the file that actually ships, which is
    /// where the first attempt at this went wrong.
    #[test]
    fn fleetds_own_manifest_yields_its_binary_name() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = here.join("../rivet-fleetd/Cargo.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|why| panic!("{} could not be read: {why}", path.display()));
        assert_eq!(
            single_bin_name(&doc_of(&text)),
            Some("rivet-fleetd".to_string()),
            "the package is rivetos-fleetd and the board's directory is rivet-fleetd; if this \
             is None the artifact is addressed to the crates.io identity again"
        );
    }

    /// **Only an unambiguous `[[bin]]` is taken.** With none, cargo names the
    /// binary after the package and the two already agree; with several there
    /// is nothing to pick, and `--bin` is how the author says which.
    #[test]
    fn a_single_bin_is_read_and_zero_or_several_are_not() {
        assert_eq!(
            single_bin_name(&doc_of("[package]\nname = \"p\"\n\n[[bin]]\nname = \"b\"\n")),
            Some("b".to_string())
        );
        assert_eq!(single_bin_name(&doc_of("[package]\nname = \"p\"\n")), None);
        assert_eq!(
            single_bin_name(&doc_of(
                "[package]\nname = \"p\"\n\n[[bin]]\nname = \"a\"\n\n[[bin]]\nname = \"b\"\n"
            )),
            None,
            "two binaries and no --bin is a question this cannot answer, so it must not guess"
        );
    }

    #[test]
    fn a_literal_version_is_taken_as_written() {
        let package = package_of("[package]\nname = \"x\"\nversion = \"0.4.2\"\n");
        let got = version_of(&package, Path::new("/nowhere/Cargo.toml")).expect("a literal");
        assert_eq!(got, "0.4.2");
    }

    /// **The refusal that started this**, kept as a test so its wording cannot
    /// silently go back to being useless: an author who wrote
    /// `version.workspace = true` and is told *"[package] has no string
    /// version"* is looking at a key that is right there.
    #[test]
    fn an_inherited_version_with_no_workspace_above_it_says_so() {
        let package = package_of("[package]\nname = \"x\"\nversion.workspace = true\n");
        let why = version_of(&package, Path::new("/nowhere/deep/Cargo.toml"))
            .expect_err("there is no workspace above /nowhere");
        assert!(
            why.contains("version.workspace = true"),
            "the refusal must quote what the author actually wrote: {why}"
        );
        assert!(
            why.contains("[workspace]"),
            "and name what was looked for: {why}"
        );
    }

    #[test]
    fn a_version_that_is_neither_a_string_nor_an_inheritance_is_refused() {
        let package = package_of("[package]\nname = \"x\"\nversion = 42\n");
        let why = version_of(&package, Path::new("/nowhere/Cargo.toml"))
            .expect_err("42 is not a version");
        assert!(why.contains("version"), "{why}");
    }

    /// The real shape: a member two directories below a workspace root.
    #[test]
    fn an_inherited_version_is_resolved_from_the_workspace_root() {
        let root = crate::scratch::dir("ver");
        let member = root.join("cmd").join("thing");
        std::fs::create_dir_all(&member).expect("make the member directory");
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"cmd/thing\"]\n\n[workspace.package]\nversion = \"9.9.9\"\n",
        )
        .expect("write the workspace root");

        let manifest = member.join("Cargo.toml");
        let package = package_of("[package]\nname = \"thing\"\nversion.workspace = true\n");
        let got = version_of(&package, &manifest).expect("inherit from the root");

        std::fs::remove_dir_all(&root).ok();
        assert_eq!(got, "9.9.9");
    }

    /// A single-crate workspace puts `[workspace]` beside `[package]` in the
    /// SAME file, so the walk must not skip the starting directory.
    #[test]
    fn a_crate_that_is_its_own_workspace_root_resolves_from_itself() {
        let root = crate::scratch::dir("own");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest = root.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[workspace]\n\n[workspace.package]\nversion = \"1.2.3\"\n\n[package]\nname = \
             \"solo\"\nversion.workspace = true\n",
        )
        .expect("write the manifest");

        let package = package_of(&std::fs::read_to_string(&manifest).expect("read it back"));
        let got = version_of(&package, &manifest).expect("inherit from itself");

        std::fs::remove_dir_all(&root).ok();
        assert_eq!(got, "1.2.3");
    }

    /// A workspace that inherits nothing is a different failure from no
    /// workspace at all, and the refusal has to tell them apart — otherwise an
    /// author adds a `[workspace.package]` that was already there.
    #[test]
    fn a_workspace_without_a_version_to_inherit_names_the_file_it_looked_in() {
        let root = crate::scratch::dir("nov");
        let member = root.join("member");
        std::fs::create_dir_all(&member).expect("make the member directory");
        std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"member\"]\n")
            .expect("write a workspace with no [workspace.package]");

        let package = package_of("[package]\nname = \"m\"\nversion.workspace = true\n");
        let why = version_of(&package, &member.join("Cargo.toml"))
            .expect_err("there is nothing to inherit");

        std::fs::remove_dir_all(&root).ok();
        assert!(
            why.contains("[workspace.package]"),
            "the refusal must name the table that is missing, not just say no: {why}"
        );
    }

    // ---- `accepts-class` — PLAN-Feature-Set-2026-09-12 §6.2, ADR-053, a
    // list since `.rulings/` R-162 R28 -------------------------------------

    /// **This is the write half of `ADR-053`**: `cargo rivet package` must be
    /// able to emit the new key from `Cargo.toml` metadata, the way every
    /// other manifest key already does — as a LIST, since R28.
    #[test]
    fn accepts_class_flows_from_cargo_toml_into_the_manifest() {
        let root =
            crate::scratch::dir("accepts-class");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"aura-controller\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\n\
             accepts-class = [\"temperature-sensor\", \"humidity-sensor\"]\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(
            meta.expect("a valid crate manifest").manifest.accepts_classes().to_vec(),
            vec!["temperature-sensor".to_string(), "humidity-sensor".to_string()]
        );
    }

    /// Absence is TAKES NO ADDON, which is what a crate that predates
    /// `ADR-053` means by writing nothing here — `read()` must not require
    /// the key. An empty list is the same state.
    #[test]
    fn a_crate_naming_no_class_accepts_no_addon() {
        let root =
            crate::scratch::dir("no-class");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"photo-index\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert!(meta.expect("a valid crate manifest").manifest.accepts_classes().is_empty());
    }

    /// The refusal names the key an author actually wrote, on
    /// `[package.metadata.rivet] proxy-path`'s footing.
    #[test]
    fn a_bad_accepts_class_is_refused_and_names_the_key() {
        let root =
            crate::scratch::dir("bad-class");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"aura-controller\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\naccepts-class = [\"Bad Class\"]\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        // `CrateMeta` derives no `Debug` (it holds a `Manifest`, which does
        // not either), so this is a `match` rather than `expect_err`.
        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("a class with a space must be refused"),
        };
        assert!(why.contains("accepts-class"), "{why}");
    }

    // ---- `material` — ADR-055 §3, `.rulings/` R-194 ----------------------

    /// **The write half of `ADR-055`**: an author can declare the key, and it
    /// reaches the manifest that gets signed. A key the core understands and
    /// this file does not is a key no author can write — which `proxy-auth`
    /// spent a day being.
    #[test]
    fn material_flows_from_cargo_toml_into_the_manifest() {
        let root =
            crate::scratch::dir("material");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"aura-controller\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\nmaterial = [\"fleet-tls\"]\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(
            meta.expect("a valid crate manifest").manifest.material(),
            [Material::FleetTls]
        );
    }

    /// Absence is NEEDS NOTHING PLACED, which is what every crate that
    /// predates `ADR-055` means by writing nothing here.
    #[test]
    fn a_crate_naming_no_material_needs_none() {
        let root =
            crate::scratch::dir("no-mat");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"photo-index\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert!(meta.expect("a valid crate manifest").manifest.material().is_empty());
    }

    /// **An unknown kind is refused, naming what the author wrote and every
    /// kind this build defines** — `devices`' refusal and its reason: an
    /// author who guessed `tls` has no other way to find out the vocabulary.
    #[test]
    fn a_material_kind_this_build_does_not_define_is_refused_by_name() {
        let root =
            crate::scratch::dir("bad-mat");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"tempmon-rivet\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\nmaterial = [\"tls\"]\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("a kind this build does not define must be refused"),
        };
        assert!(why.contains("material"), "{why}");
        assert!(why.contains("\"tls\""), "the typo itself must be named: {why}");
        assert!(why.contains("fleet-tls"), "the vocabulary must be named: {why}");
        for file in Material::FleetTls.files() {
            assert!(why.contains(file), "{file} is not named: {why}");
        }
    }

    // ---- `settings` refuses a credential-shaped name — `ADR-038` §5,
    // `.rulings/` R-169 and R-210 D16. The fixture is TempMon's real name for
    // its token, found in `rivet-01`'s own document, not a name invented for
    // this test.

    /// **The defect this closes.** A TempMon package built against this SDK
    /// declares `settings = ["write-token"]` today, and it must be refused at
    /// packaging — on a workstation, naming the offending key — rather than
    /// only on a board when the supervisor reads the installed manifest.
    #[test]
    fn a_credential_shaped_setting_is_refused_at_packaging_and_names_the_key() {
        let root =
            crate::scratch::dir("credential-setting");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"tempmon-rivet\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\nsettings = [\"write-token\"]\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("a credential-shaped setting must be refused"),
        };
        assert!(why.contains("write-token"), "{why}");
        assert!(
            why.contains("install-secret"),
            "the refusal must point at where a credential belongs instead: {why}"
        );
    }

    /// **D16 (`.rulings/` R-210): `token` now matches ANYWHERE in a name**,
    /// not only as its own whole hyphen-separated word — `apitoken` carries
    /// no hyphen at all, and packaged before this ruling.
    #[test]
    fn a_credential_shaped_name_with_no_hyphen_is_refused_at_packaging() {
        let root = crate::scratch::dir("credential-setting-no-hyphen");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"tempmon-rivet\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\nsettings = [\"apitoken\"]\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("a credential-shaped setting must be refused"),
        };
        assert!(why.contains("apitoken"), "{why}");
        assert!(
            why.contains("install-secret"),
            "the refusal must point at where a credential belongs instead: {why}"
        );
    }

    // ---- the application's own name refuses a reserved word — `.rulings/`
    // R-210 D21 ---------------------------------------------------------

    /// **The defect this closes.** A crate packaged under `root` — a system
    /// identity, not a service — installed today and was indistinguishable
    /// from the account `getpwnam("root")` already answers. Refused here, on
    /// a workstation, naming the crate's own name.
    #[test]
    fn an_application_named_for_a_reserved_word_is_refused_at_packaging() {
        let root = crate::scratch::dir("reserved-name");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"root\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("an application named root must be refused"),
        };
        assert!(why.contains("root"), "{why}");
    }

    /// **The `[[bin]]` name is what is checked, not the package name** —
    /// `product_name`'s own rule, applied to this refusal too. A crate whose
    /// crates.io-safe package name is ordinary but whose binary is named
    /// `ingest` must still be refused, because `ingest` is what lands under
    /// `/state/app` and in `rivet get log | where svc == ingest`.
    #[test]
    fn the_binary_name_is_checked_rather_than_the_package_name() {
        let root = crate::scratch::dir("reserved-name-bin");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"rivetos-ingest-relay\"\nversion = \"0.1.0\"\n\n\
             [[bin]]\nname = \"ingest\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("an application whose binary is named ingest must be refused"),
        };
        assert!(why.contains("ingest"), "{why}");
    }

    /// A setting that merely shares a word's substring, or is an ordinary
    /// name from a real manifest, is unaffected.
    #[test]
    fn an_ordinary_setting_still_packages() {
        let root =
            crate::scratch::dir("ordinary-setting");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"rivet-proxyd\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\nsettings = [\"upstream-name\"]\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(
            meta.expect("a valid crate manifest").manifest.settings(),
            &["upstream-name".to_string()]
        );
    }

    // ---- `cpu` — ADR-015 §3, `PLAN.md` §172; missing from this reader and
    // the usage text until `.rulings/` R-162 named it as a defect ----------

    /// **The defect itself, proven and fixed**: `cargo rivet package` reads
    /// `cpu` from `Cargo.toml` metadata the way it already reads `memory`.
    #[test]
    fn a_declared_cpu_share_flows_from_cargo_toml_into_the_manifest() {
        let root = crate::scratch::dir("cpu");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"aura-controller\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\ncpu = \"0.5\"\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(
            meta.expect("a valid crate manifest").manifest.cpu().map(Cpu::as_str),
            Some("0.5")
        );
    }

    /// **D22 (`.rulings/` R-210): a cpu cap the kernel cannot keep is refused
    /// here, not merely noted.** `0.005` is a quota of 500 microseconds
    /// against `CpuLimit::PERIOD_US`'s 100000, half of
    /// `CpuLimit::KERNEL_MIN_QUOTA_US`.
    #[test]
    fn a_cpu_share_below_the_kernel_minimum_is_refused_and_names_the_key() {
        let root = crate::scratch::dir("cpu-below-minimum");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"aura-controller\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\ncpu = \"0.005\"\n",
        )
        .expect("write the manifest");

        let result = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        let why = match result {
            Err(why) => why,
            Ok(_) => panic!("a cpu cap below the kernel minimum must be refused"),
        };
        assert!(why.contains("cpu"), "{why}");
        assert!(why.contains("0.005"), "{why}");
        assert!(why.contains("kernel"), "{why}");
    }

    /// The boundary itself still packages — the kernel's own minimum is a
    /// cap it keeps, not one it refuses.
    #[test]
    fn a_cpu_share_exactly_at_the_kernel_minimum_still_packages() {
        let root = crate::scratch::dir("cpu-at-minimum");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"aura-controller\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\ncpu = \"0.01\"\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(
            meta.expect("exactly the kernel's minimum must still package").manifest.cpu().map(Cpu::as_str),
            Some("0.01")
        );
    }

    /// Absence means no particular share was requested — `memory`'s footing.
    #[test]
    fn a_crate_naming_no_cpu_share_asks_for_none() {
        let root = crate::scratch::dir("no-cpu");
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            "[package]\nname = \"photo-index\"\nversion = \"0.1.0\"\n\n\
             [package.metadata.rivet]\nkind = \"daemon\"\n",
        )
        .expect("write the manifest");

        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(meta.expect("a valid crate manifest").manifest.cpu(), None);
    }

    // ---- `restore-path` — ADR-051 §12, `.rulings/` R-183 D1 ----------------

    /// `[package.metadata.rivet]` with `extra` appended, read through [`read`].
    fn read_rivet_table(tag: &str, extra: &str) -> Result<CrateMeta, String> {
        let root = crate::scratch::dir(&format!("meta-restore-{tag}"));
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            format!(
                "[package]\nname = \"tempmon-rivet\"\nversion = \"0.1.0\"\n\n\
                 [package.metadata.rivet]\nkind = \"daemon\"\nlisten = [\"tcp:8080\"]\n{extra}"
            ),
        )
        .expect("write the manifest");
        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();
        meta
    }

    /// **The key reaches the manifest `cargo rivet package` signs**, and the
    /// signed text parses back on a machine carrying it.
    #[test]
    fn a_declared_restore_path_flows_from_cargo_toml_into_the_manifest() {
        let meta = read_rivet_table(
            "ok",
            "proxy-path = \"tempmon\"\nbackup-path = \"api/v1/backup\"\n\
             backup-format = \"sqlite\"\nrestore-path = \"api/v1/restore\"\n",
        )
        .expect("a valid crate manifest");
        let surface = meta.manifest.backup().expect("a backup surface");
        assert_eq!(surface.restore_path(), Some("api/v1/restore"));
        let text = meta.manifest.render();
        assert!(text.contains("restore-path = \"api/v1/restore\"\n"), "{text}");
        Manifest::parse(text.as_bytes()).expect("what package emits, a machine reads");
    }

    /// **Refused in words, in front of the Cargo.toml the author edited**, for
    /// every case `Manifest::parse` would refuse with only a key name.
    #[test]
    fn a_restore_path_the_manifest_would_refuse_is_refused_here_first() {
        for (tag, extra, expect) in [
            (
                "alone",
                "proxy-path = \"tempmon\"\nrestore-path = \"api/v1/restore\"\n",
                "without backup-path and backup-format",
            ),
            (
                "feed",
                "proxy-path = \"tempmon\"\nbackup-feed = \"api/v1/blobs\"\n\
                 restore-path = \"api/v1/restore\"\n",
                "beside backup-feed",
            ),
            (
                "open",
                "proxy-path = \"tempmon\"\nproxy-auth = \"none\"\nbackup-path = \"api/v1/backup\"\n\
                 backup-format = \"sqlite\"\nrestore-path = \"api/v1/restore\"\n",
                "WITHOUT an operator",
            ),
            (
                "open-rule",
                "proxy-path = \"tempmon\"\nproxy-auth-paths = [\"none:api\"]\n\
                 backup-path = \"api/v1/backup\"\nbackup-format = \"sqlite\"\n\
                 restore-path = \"api/v1/restore\"\n",
                "WITHOUT an operator",
            ),
            (
                "escape",
                "proxy-path = \"tempmon\"\nbackup-path = \"api/v1/backup\"\n\
                 backup-format = \"sqlite\"\nrestore-path = \"../state\"\n",
                "a backup or restore path is",
            ),
        ] {
            let why = read_rivet_table(tag, extra)
                .err()
                .unwrap_or_else(|| panic!("{tag}: must be refused"));
            assert!(why.contains(expect), "{tag}: {why}");
        }
    }

    // ---- reserved paths — ADR-046 §4 -----------------------------------------

    /// **Packaging refuses a `proxy-path` the machine keeps for its own sign-in
    /// pages**, and says so in those words rather than as a bad character.
    #[test]
    fn a_reserved_proxy_path_is_refused_at_packaging() {
        for reserved in rivet_core::package::application::RESERVED_PROXY_PATHS {
            let why = read_rivet_table(
                &format!("reserved-{reserved}"),
                &format!("proxy-path = \"{reserved}\"\n"),
            )
            .err()
            .unwrap_or_else(|| panic!("{reserved}: must be refused at packaging"));
            assert!(why.contains("proxy-path"), "{reserved}: {why}");
            assert!(why.contains(reserved), "{reserved}: {why}");
            assert!(why.contains("sign-in pages"), "{reserved}: {why}");
            assert!(!why.contains("ONE segment"), "{reserved}: the shape is not the fault: {why}");
        }
    }

    // ---- `app` and `plain` — ADR-048 §4, ADR-061 §7 -------------------------

    /// **`app` and `plain` answer for a PREFIX, never for the whole route** —
    /// refused here in words, before `Manifest::parse` would refuse it with
    /// only a key name.
    #[test]
    fn app_and_plain_are_refused_as_the_whole_routes_gate() {
        for verdict in ["app", "plain"] {
            let why = read_rivet_table(
                &format!("whole-route-{verdict}"),
                &format!("proxy-path = \"gantry\"\nproxy-auth = \"{verdict}\"\n"),
            )
            .err()
            .unwrap_or_else(|| panic!("{verdict}: must be refused as a whole-route gate"));
            assert!(why.contains("proxy-auth-paths"), "{verdict}: {why}");
        }
    }

    /// The two new verdicts are declarable as per-path rules, and reach the
    /// signed manifest exactly as `none` and `operator` do.
    #[test]
    fn app_and_plain_are_declarable_as_per_path_rules() {
        let meta = read_rivet_table(
            "app-plain-rules",
            "proxy-path = \"gantry\"\nproxy-auth-paths = [\"app:api\", \"plain:install\"]\n",
        )
        .expect("a valid crate manifest");
        let rules = meta.manifest.proxy_auth_paths();
        assert_eq!(rules[0].prefix(), "api");
        assert_eq!(
            rules[0].auth(),
            rivet_core::package::application::ProxyAuth::App
        );
        assert_eq!(rules[1].prefix(), "install");
        assert_eq!(
            rules[1].auth(),
            rivet_core::package::application::ProxyAuth::Plain
        );
        let text = meta.manifest.render();
        assert!(
            text.contains("proxy-auth-paths = [\"app:api\", \"plain:install\"]\n"),
            "{text}"
        );
        Manifest::parse(text.as_bytes()).expect("what package emits, a machine reads");
    }

    /// A restore route under `app:` or `plain:` is refused here first, exactly
    /// as under `none:`.
    #[test]
    fn a_restore_path_under_app_or_plain_is_refused_here_first() {
        for verdict in ["app", "plain"] {
            let why = read_rivet_table(
                &format!("restore-{verdict}"),
                &format!(
                    "proxy-path = \"gantry\"\nproxy-auth-paths = [\"{verdict}:api\"]\n\
                     backup-path = \"api/backup\"\nbackup-format = \"sqlite\"\n\
                     restore-path = \"api/backup\"\n"
                ),
            )
            .err()
            .unwrap_or_else(|| panic!("{verdict}: a restore under it must be refused"));
            assert!(why.contains("WITHOUT an operator"), "{verdict}: {why}");
        }
    }

    // ---- `:direct` — ADR-046 §7, `.rulings/` R-188 ---------------------------

    /// `listen` as given, then `extra`, read through [`read`].
    fn read_listen(tag: &str, listen: &str, extra: &str) -> Result<CrateMeta, String> {
        let root = crate::scratch::dir(&format!("direct-{tag}"));
        std::fs::create_dir_all(&root).expect("make the directory");
        let manifest_path = root.join("Cargo.toml");
        std::fs::write(
            &manifest_path,
            format!(
                "[package]\nname = \"reliquary-rivet\"\nversion = \"0.1.0\"\n\n\
                 [package.metadata.rivet]\nkind = \"daemon\"\nlisten = {listen}\n{extra}"
            ),
        )
        .expect("write the manifest");
        let meta = read(&manifest_path);
        std::fs::remove_dir_all(&root).ok();
        meta
    }

    /// **Reliquary's declaration reaches the manifest `cargo rivet package`
    /// signs**, and the signed text parses back on a machine carrying it with
    /// the routed port on loopback and the phone's on every address.
    #[test]
    fn a_direct_port_flows_from_cargo_toml_into_the_signed_manifest() {
        let meta = read_listen(
            "ok",
            "[\"tcp:8444\", \"tcp:8443:direct\"]",
            "proxy-path = \"reliquary\"\n",
        )
        .expect("a valid crate manifest");
        let text = meta.manifest.render();
        assert!(
            text.contains("listen = [\"tcp:8444\", \"tcp:8443:direct\"]\n"),
            "{text}"
        );
        let back = Manifest::parse(text.as_bytes()).expect("what package emits, a machine reads");
        let bound: Vec<_> = back
            .listen()
            .iter()
            .map(|s| s.bind_address(back.proxied()))
            .collect();
        assert_eq!(
            bound,
            vec![
                Some(std::net::Ipv4Addr::LOCALHOST),
                Some(std::net::Ipv4Addr::UNSPECIFIED)
            ]
        );
    }

    /// **Refused in words, in front of the Cargo.toml the author edited**, for
    /// every case `Manifest::parse` would refuse with only a key name, and for
    /// a flag this grammar cannot read.
    #[test]
    fn a_direct_flag_the_machine_would_refuse_is_refused_here_first() {
        for (tag, listen, extra, expect) in [
            (
                "routed",
                "[\"tcp:8444:direct\", \"tcp:8443\"]",
                "proxy-path = \"reliquary\"\n",
                "rivet-proxyd forwards `/reliquary` to",
            ),
            (
                "no-route",
                "[\"tcp:443:direct\"]",
                "",
                "tcp:443:direct declared `:direct` and there is no proxy-path",
            ),
            (
                "misspelt",
                "[\"tcp:8443:drect\"]",
                "proxy-path = \"reliquary\"\n",
                "with `:direct` allowed after a port",
            ),
        ] {
            let why = read_listen(tag, listen, extra)
                .err()
                .unwrap_or_else(|| panic!("{tag}: must be refused"));
            assert!(why.contains(expect), "{tag}: {why}");
        }
    }
}
