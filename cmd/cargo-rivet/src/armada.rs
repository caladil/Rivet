//! `cargo rivet armada` — **the interactive CRUD tool over the Armada
//! Registry**, `ADR-059` §6.
//!
//! The Armada Registry (`rivetos_register::armada`) is the operator's list of
//! Fleets, kept on this workstation at [`default_dir`]. This module is the
//! tool that reads and writes it: a bare `cargo rivet armada` is an
//! interactive interview — the standing rule for this whole binary's
//! human-facing verbs (`crate::write_image`'s menus, and — since 2026-09-22 —
//! the bare `cargo rivet deploy` verb `crate::fleet::run_interview` answers) —
//! and `cargo rivet armada <verb>` is the scripted twin a script or an agent
//! drives directly.
//!
//! # Same shape as `cargo rivet deploy` / `install`
//!
//! `ADR-059` §5 splits this tool bare = interview, `<verb>` = scripted, one
//! `SubVerb` enum, one `parse`/`run`/`usage` triple, all under the ONE verb
//! `armada`. **`crate::fleet` used to be the same shape** — bare `fleet`
//! dropped into the deploy interview, `fleet <verb>` was the scripted
//! sub-verbs — but `ADR-052`'s fleet repository and `ADR-059`'s Armada are two
//! different things that happened to share a `SubVerb` skeleton, and
//! 2026-09-22 gave the deploy interview its own top-level verb instead
//! (`cargo rivet deploy`, bare only; `cargo rivet install` its scripted twin;
//! `cargo rivet fleet <verb>` unchanged and no longer bare-interactive at
//! all). `armada` keeps the older, one-verb shape on purpose (`ADR-011` — one
//! shape for "a CRUD tool with an interactive front end", not a second one
//! invented here) and reuses `crate::write_image::choose_with` for a
//! single-answer menu rather than writing a third copy of it;
//! `crate::fleet_deploy`'s own `multi_choose_with`/`refuse_unless_terminal`
//! are private to that module (that file's own header explains why:
//! `write_image::refuse_unless_terminal`'s text is *"copied rather than
//! reached for across a module boundary that does not expose it"*), so the
//! single-select menus this tool needs are written the same way — copied, not
//! reached for.
//!
//! # Where a Fleet's `Server` comes from — never invented here
//!
//! "Add a Fleet" bootstraps an existing Server-role device (`crate::device`'s
//! own register, filtered to [`rivetos_register::FleetRole::Server`]) into
//! the Armada. Its host and pin are **not re-derived** by this module: they
//! are read out of exactly what [`crate::device::expand`] would fill into
//! `--host`/`--pin` for `cargo rivet run <device>` or `install <device>` —
//! the same static-interface-first, DHCP-`.local`-fallback resolution that
//! function is already tested against (`cmd/cargo-rivet/src/device.rs`). See
//! [`fleet_from_device`].
//!
//! # What this module does not do
//!
//! It never reads or writes the maintainer's real Armada Registry or device
//! register from a test — every test below hands a scratch directory to the
//! functions that take one, mirroring `crate::device`'s own tests module
//! header. It mints no seed and reaches no machine: a Fleet's `Server.pin`
//! is the pin the device register already holds, not a key this tool asks
//! for over a wire.
//!
//! [`default_dir`]: crate::armada::default_dir
//! [`fleet_from_device`]: crate::armada::fleet_from_device

// The link targets above are spelled from the crate root because this module
// is declared with an outer `///` comment on its `mod` line in `main.rs`, and
// rustdoc then resolves every link in these `//!` lines from the crate root
// rather than from here. It is also why `fleet_from_device` is `pub(super)`
// rather than private — the crate root has to be able to see it.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use rivetos_register::armada::{store, Armada, Fleet, Server};
use rivetos_register::FleetRole;

// ---------------------------------------------------------------------------
// where the Armada Registry lives
// ---------------------------------------------------------------------------

/// Where this workstation's Armada Registry lives — `~/.rivet/armada`, or
/// `%USERPROFILE%\.rivet\armada`.
///
/// **The same home resolution `crate::device::default_identity_dir` restates
/// and `crate::credentials::root` already centralises** — `$HOME` then
/// `$USERPROFILE`, joined to `.rivet` — reused here via [`crate::credentials::root`]
/// rather than a third copy of the same two `env::var_os` calls. Unlike
/// `crate::device::default_register_dir` (which resolves `rivet-fleet`'s OWN
/// configuration directory through Ignition, because that register belongs to
/// a *different* binary with its own app name), the Armada Registry is this
/// tool's own state under the `~/.rivet` tree every other `cargo-rivet`
/// artifact already lives under — `crate::fleet::default_root`'s
/// `~/.rivet/fleet` is the direct sibling this mirrors, ending in `armada`
/// rather than `fleet`.
///
/// **No environment-variable override.** `crate::fleet::default_root` (the
/// closest sibling — another `~/.rivet/<name>` root with no `--at` in the
/// interactive tool) has none either; an override would be a second way to
/// answer "which Armada", and `ADR-059` names exactly one per operator.
///
/// # Errors
///
/// This host sets neither `HOME` nor `USERPROFILE` — a container run with
/// neither, the same real state [`crate::credentials::root`] already names.
pub(crate) fn default_dir() -> Result<PathBuf, String> {
    default_dir_with(crate::credentials::root())
}

/// [`default_dir`], with the resolved home handed in — the seam a test uses
/// so it never depends on this process's real `HOME`/`USERPROFILE`
/// (`crate::device`'s module header: "never touched by a test").
fn default_dir_with(home: Option<PathBuf>) -> Result<PathBuf, String> {
    home.map(|home| home.join("armada")).ok_or_else(|| {
        "this host sets neither HOME nor USERPROFILE, so there is nowhere to default the Armada \
         Registry (ADR-059 §6.3). Set one of them; there is no --at override for this tool."
            .to_string()
    })
}

// ---------------------------------------------------------------------------
// a Fleet's Server, off a device register's own record — never invented here
// ---------------------------------------------------------------------------

/// One flag's value out of a word vector [`crate::device::expand`] built —
/// `--host X` back to `X`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|at| args.get(at + 1)).cloned()
}

/// **Build the `Fleet` `name`'s own device record bootstraps into the
/// Armada** — its host and pin read out of exactly what
/// [`crate::device::expand`] resolves for `run <name>`/`install <name>`,
/// never a second, hand-rolled reading of [`rivetos_register::Device`].
///
/// **The server's own fleet-operator public key comes off the record too**, as
/// [`rivetos_register::Device::operator_key`] — the key a member joining this
/// Fleet must be staged to trust or the server can list it but never manage it
/// (`docs/PLAN.md` §183.2). It is read straight off the record rather than
/// through `expand` (which only fills `--host`/`--identity`/`--pin`), and a
/// record that carries none — a server provisioned before the key was recorded
/// — yields `None`, so such a Fleet stages a member exactly as it did before
/// this existed.
///
/// # Errors
///
/// [`crate::device::expand`]'s: `name` answers to no device in the register
/// at `register_root` (naming what it does hold), its address is ambiguous
/// or unresolvable, or it carries no control-plane pin and none can be
/// supplied here.
pub(super) fn fleet_from_device(name: &str, register_root: &Path) -> Result<Fleet, String> {
    let expanded = crate::device::expand(name, &[], register_root)?;
    let host = flag_value(&expanded, "--host").ok_or_else(|| {
        format!(
            "cargo-rivet: internal error — crate::device::expand did not fill --host for \
             {name:?}; this is a defect in cargo-rivet, not in the register."
        )
    })?;
    let pin = flag_value(&expanded, "--pin").ok_or_else(|| {
        format!(
            "cargo-rivet: internal error — crate::device::expand did not fill --pin for \
             {name:?}; this is a defect in cargo-rivet, not in the register."
        )
    })?;

    // **The fleet-operator key, off the same record.** `expand` succeeded
    // above, so the register reads and `name` resolves; reading it again here
    // to reach a field `expand` does not expose is the same "one authority for
    // the record, read it rather than re-derive it" the host and pin already
    // follow. A record with no key is the ordinary case and yields `None`.
    let operator_key = rivetos_register::store::load(register_root)
        .ok()
        .and_then(|register| register.resolve(name).ok().and_then(|d| d.operator_key.clone()));

    Ok(Fleet { name: name.to_string(), server: Server { host, pin, operator_key } })
}

// ---------------------------------------------------------------------------
// reading and rendering
// ---------------------------------------------------------------------------

/// `List`'s report — name and server host, one Fleet per line. Pure, so it is
/// unit-tested directly against a fixture [`Armada`] rather than only through
/// a scratch-directory round trip.
///
/// **No monitor or backup column.** Whether a host is health-monitored or
/// backed up is Rivet Monitor's own per-host setting (`ADR-059` §6), and the
/// Armada holds none.
fn render_list(armada: &Armada) -> String {
    if armada.fleets.is_empty() {
        return format!(
            "{}\n\n(empty — no Fleets yet. \"Add a Fleet\" (or `cargo rivet armada add \
             <server-device-name>`) bootstraps one from a Server-role device.)\n",
            armada.root.display(),
        );
    }
    let mut out = format!("{}\n\n", armada.root.display());
    for fleet in armada.fleets.values() {
        out.push_str(&format!("{:<20} {}\n", fleet.name, fleet.server.host));
    }
    out
}

fn load(root: &Path) -> Result<Armada, String> {
    store::load(root).map_err(|e| format!("could not read the Armada Registry at {}: {e}", root.display()))
}

fn save(armada: &Armada) -> Result<(), String> {
    store::save(armada)
        .map(|_| ())
        .map_err(|e| format!("could not write the Armada Registry at {}: {e}", armada.root.display()))
}

// ---------------------------------------------------------------------------
// mutations — shared by the interview and the scripted verbs
// ---------------------------------------------------------------------------

/// Add or refresh `fleet` in the Armada at `root`.
///
/// **A Fleet that already exists has its record replaced** — re-bootstrapping
/// a Fleet (its device moved, its pin rotated) refreshes its server's address,
/// pin and operator key from the device register, which is where they come
/// from.
fn add_fleet_record(root: &Path, fresh: Fleet) -> Result<String, String> {
    let mut armada = load(root)?;
    let name = fresh.name.clone();
    let report = if armada.fleets.contains_key(&name) {
        armada.fleets.insert(name.clone(), fresh);
        format!(
            "\nRefreshed Fleet {name:?}'s server address/pin in the Armada Registry at {}.\n",
            root.display(),
        )
    } else {
        armada.fleets.insert(name.clone(), fresh);
        format!("\nAdded Fleet {name:?} to the Armada Registry at {}.\n", root.display())
    };
    save(&armada)?;
    Ok(report)
}

/// **A fleet server `cargo rivet provision` has just written and enrolled
/// becomes the Armada's Fleet** — answered as a line for its report, never an
/// error, because the card is already written and verified.
///
/// The Fleet is the server's own record, the one `enrollment_device` built and
/// the register now holds: its host from [`rivetos_register::Device::dial_host`]
/// (the static address, or `<hostname>.local`, with no lookup made), its pin,
/// and the `rivet-fleetd` operator key its staging bundle carries. The member
/// interview reads that key out of this Armada to stage the member trusting
/// the server that manages it.
///
/// **Added only where the answer is not in doubt**: an Armada holding no
/// Fleet, or one already holding a Fleet of this name, which is refreshed —
/// exactly `cargo rivet armada add`'s own refresh. An Armada holding OTHER
/// Fleets is left alone and the line names the command: a member provisioned
/// next could no longer tell which server to trust (the lookup is 0/1/many),
/// and a Fleet that worked yesterday must not stop working because a second
/// server was provisioned today.
pub(crate) fn bootstrap_provisioned_server(root: &Path, server: &rivetos_register::Device) -> String {
    let name = &server.name;
    let by_hand = format!("`cargo rivet armada add {name}` adds it by hand");
    let mut armada = match load(root) {
        Ok(armada) => armada,
        Err(why) => return format!("armada: {name} NOT added — {why}. {by_hand}.\n"),
    };
    let others: Vec<&str> = armada.fleets.keys().filter(|k| *k != name).map(String::as_str).collect();
    if !others.is_empty() {
        return format!(
            "armada: {name} NOT added — the Armada at {} already holds {}, and a member \
             provisioned next would no longer know which server to trust. {by_hand}.\n",
            root.display(),
            others.join(", "),
        );
    }
    let host = match server.dial_host() {
        Ok(host) => format!("{host}:{}", server.control_plane_port()),
        Err(why) => return format!("armada: {name} NOT added — {why} {by_hand}.\n"),
    };
    let Some(pin) = server.pin.clone().filter(|p| !p.is_empty()) else {
        return format!("armada: {name} NOT added — its record carries no pin. {by_hand}.\n");
    };
    let key_note = if server.operator_key.is_some() {
        "with its rivet-fleetd operator key, so the member provisioned next is staged to trust it"
    } else {
        "WITHOUT an operator key — its staging bundle named none — so a member provisioned next is \
         staged trusting the human operator alone"
    };
    let refreshed = armada.fleets.contains_key(name);
    armada.fleets.insert(
        name.clone(),
        Fleet { name: name.clone(), server: Server { host, pin, operator_key: server.operator_key.clone() } },
    );
    match save(&armada) {
        Ok(()) => format!(
            "armada: Fleet {name:?} {} in {}, {key_note}\n",
            if refreshed { "refreshed" } else { "added" },
            root.display(),
        ),
        Err(why) => format!("armada: {name} NOT added — {why}. {by_hand}.\n"),
    }
}

fn forget_fleet_record(root: &Path, name: &str) -> Result<String, String> {
    store::forget(root, name)
        .map_err(|e| format!("could not forget Fleet {name:?} at {}: {e}", root.display()))?;
    Ok(format!(
        "\nForgot Fleet {name:?} — its record on this PC is gone. This says nothing about the \
         Fleet's server or its members.\n",
    ))
}

fn set_backup_dir_record(root: &Path, path: &Path) -> Result<String, String> {
    let mut armada = load(root)?;
    armada.settings.backup_dir = Some(path.to_path_buf());
    save(&armada)?;
    Ok(format!("\nbackup_dir set to {}\n", path.display()))
}

fn set_keep_record(root: &Path, keep: u32) -> Result<String, String> {
    let mut armada = load(root)?;
    armada.settings.keep = keep;
    save(&armada)?;
    Ok(format!("\nkeep set to {keep}\n"))
}

/// Resolve `name` to a Server-role device's record in the register at
/// `register_root`, or refuse naming every device it does hold — the same
/// shape [`crate::device::expand_with`]'s own refusal takes for an unknown
/// name, restated here because that refusal is folded into `expand`'s error
/// text rather than exposed as a lookup this module can call.
fn resolve_server_device(
    register: &rivetos_register::Register,
    register_root: &Path,
    name: &str,
) -> Result<String, String> {
    let Some(device) = register.devices.values().find(|d| d.name == name) else {
        let mut names: Vec<&str> = register.devices.values().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        return Err(if names.is_empty() {
            format!(
                "{name:?} answers to no device, and the register at {} holds none at all.",
                register_root.display()
            )
        } else {
            format!(
                "{name:?} answers to no device in the register at {}. It holds: {}.",
                register_root.display(),
                names.join(", "),
            )
        });
    };
    if device.fleet_role != Some(FleetRole::Server) {
        return Err(format!(
            "{name:?} is recorded as {}, not a Server — the Armada bootstraps only Server-role \
             devices (ADR-059 §6.3). `rivet-provision`'s Fleet question records a device's role. \
             Nothing was added.",
            device.fleet_role.map_or("no role at all".to_string(), |r| r.to_string()),
        ));
    }
    Ok(device.name.clone())
}

fn add_fleet_for(armada_root: &Path, device_name: &str) -> Result<String, String> {
    let register_root = crate::device::default_register_dir()?;
    let register = rivetos_register::store::load(&register_root).map_err(|e| {
        format!("could not read the device register at {}: {e}", register_root.display())
    })?;
    let resolved = resolve_server_device(&register, &register_root, device_name)?;
    let fleet = fleet_from_device(&resolved, &register_root)?;
    add_fleet_record(armada_root, fleet)
}

// ---------------------------------------------------------------------------
// menus — copied from write_image.rs/fleet_deploy.rs's own shapes rather
// than reached for across a module boundary that does not expose them
// (fleet_deploy.rs's own `refuse_unless_terminal` says why)
// ---------------------------------------------------------------------------

/// `write_image::refuse_unless_terminal`'s own text, copied for its own
/// reason: no interactive prompt without a terminal.
fn refuse_unless_terminal(is_terminal: bool, what: &str, instead: &str) -> Result<(), String> {
    if is_terminal {
        return Ok(());
    }
    Err(format!(
        "this run has no terminal to ask about {what} at, so nothing was written and nothing \
         was read from stdin. `{instead}` is the flag-driven, non-interactive way in."
    ))
}

/// A free-text answer, or a refusal with no terminal. Only `armada`'s
/// Settings step needs this — every other question here is a menu.
fn read_text_with(is_terminal: bool, prompt: &str, instead: &str) -> Result<String, String> {
    refuse_unless_terminal(is_terminal, prompt, instead)?;
    print!("\n  {prompt}: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read an answer: {e}"))?;
    Ok(line.trim().to_string())
}

/// Refuse to ask a question with nowhere to type an answer, naming every
/// scripted verb this interview composes — `crate::fleet_deploy`'s own
/// `refusal_for_a_scripted_caller` is the precedent for naming the whole set
/// rather than one verb.
fn refusal_for_a_scripted_caller() -> String {
    "this run has no terminal, and `cargo rivet armada` with no sub-verb is the interactive \
     Armada Registry tool — a human tool. It composes verbs that each have a flag-driven form \
     for a script or an agent to call directly:\n\
     \x20 list             cargo rivet armada list\n\
     \x20 add a Fleet      cargo rivet armada add <server-device-name>\n\
     \x20 forget a Fleet   cargo rivet armada forget <fleet-name>\n\
     \x20 set backup dir   cargo rivet armada set-backup-dir <path>\n\
     \x20 set keep count   cargo rivet armada set-keep <n>\n"
        .to_string()
}

// ---------------------------------------------------------------------------
// the interview
// ---------------------------------------------------------------------------

/// `cargo rivet armada` with no sub-verb — the thin seam `main.rs`'s dispatch
/// calls, matching `crate::fleet::run_interview`'s own shape.
///
/// # Errors
///
/// [`run_interview_with`]'s.
pub(crate) fn run_interview() -> Result<String, String> {
    run_interview_with(std::io::stdin().is_terminal())
}

/// [`run_interview`], with whether this run has a terminal taken as a
/// parameter — `crate::fleet_deploy::run_with`'s own reason: a test drives
/// the refusal without a real stdin. The menu loop itself is not driven by a
/// test beyond that refusal — see this crate's brief: the live menu answers
/// are unproven without a real terminal.
fn run_interview_with(is_terminal: bool) -> Result<String, String> {
    if !is_terminal {
        return Err(refusal_for_a_scripted_caller());
    }
    let root = default_dir()?;
    let mut summary = String::new();

    loop {
        // **No numbered `Exit` option.** `x` is the one way to leave this
        // menu, and every other numbered menu `choose_with`/`multi_choose_with`
        // back, on the removable-device menu's own convention.
        let options: Vec<String> = vec![
            "List Fleets".to_string(),
            "Add a Fleet".to_string(),
            "Forget a Fleet".to_string(),
            "Settings (backup directory, keep count)".to_string(),
        ];
        let chosen = match crate::write_image::choose_with(
            is_terminal,
            "action",
            &options,
            "cargo rivet armada <verb>",
        ) {
            Ok(crate::write_image::Choice::Selected(n)) => n,
            Ok(crate::write_image::Choice::Exit) => {
                summary.push_str("\nExited.\n");
                return Ok(summary);
            }
            Err(e) => return Err(e),
        };

        let outcome = match chosen {
            0 => load(&root).map(|armada| render_list(&armada)),
            1 => add_fleet_interview(is_terminal, &root),
            2 => forget_interview(is_terminal, &root),
            3 => settings_interview(is_terminal, &root),
            _ => Err("internal error: choose_with returned an index outside the menu it was \
                      given"
                .to_string()),
        };
        match outcome {
            Ok(report) => summary.push_str(&report),
            Err(e) => summary.push_str(&format!("\n{e}\n")),
        }
    }
}

fn add_fleet_interview(is_terminal: bool, armada_root: &Path) -> Result<String, String> {
    let register_root = crate::device::default_register_dir()?;
    let register = rivetos_register::store::load(&register_root).map_err(|e| {
        format!("could not read the device register at {}: {e}", register_root.display())
    })?;
    let mut servers: Vec<&rivetos_register::Device> =
        register.devices.values().filter(|d| d.fleet_role == Some(FleetRole::Server)).collect();
    if servers.is_empty() {
        return Ok(format!(
            "\nThe device register at {} holds no Server-role device to bootstrap into the \
             Armada. `rivet-provision`'s Fleet question records a device's role; nothing was \
             added.\n",
            register_root.display(),
        ));
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    let options: Vec<String> = servers.iter().map(|d| d.name.clone()).collect();
    let chosen = match crate::write_image::choose_with(
        is_terminal,
        "Server to bootstrap into the Armada",
        &options,
        "cargo rivet armada add <server-device-name>",
    ) {
        Ok(crate::write_image::Choice::Selected(n)) => n,
        Ok(crate::write_image::Choice::Exit) => {
            return Ok("\nExited — no Server was chosen, and nothing was added.\n".to_string());
        }
        Err(e) => return Err(e),
    };
    let fleet = fleet_from_device(&servers[chosen].name, &register_root)?;
    add_fleet_record(armada_root, fleet)
}

fn forget_interview(is_terminal: bool, root: &Path) -> Result<String, String> {
    let armada = load(root)?;
    if armada.fleets.is_empty() {
        return Ok("\nNo Fleets to forget.\n".to_string());
    }
    let names: Vec<String> = armada.fleets.keys().cloned().collect();
    let chosen = match crate::write_image::choose_with(
        is_terminal,
        "Fleet to forget",
        &names,
        "cargo rivet armada forget <fleet-name>",
    ) {
        Ok(crate::write_image::Choice::Selected(n)) => n,
        Ok(crate::write_image::Choice::Exit) => {
            return Ok("\nExited — no Fleet was chosen, and nothing was forgotten.\n".to_string());
        }
        Err(e) => return Err(e),
    };
    forget_fleet_record(root, &names[chosen])
}

fn settings_interview(is_terminal: bool, root: &Path) -> Result<String, String> {
    let mut armada = load(root)?;
    let current_dir = armada
        .settings
        .backup_dir
        .as_ref()
        .map_or_else(|| "(none)".to_string(), |p| p.display().to_string());
    let typed_dir = read_text_with(
        is_terminal,
        &format!("backup directory [{current_dir}, blank keeps it]"),
        "cargo rivet armada set-backup-dir <path>",
    )?;
    let current_keep = armada.settings.keep;
    let typed_keep = read_text_with(
        is_terminal,
        &format!("keep count [{current_keep}, blank keeps it]"),
        "cargo rivet armada set-keep <n>",
    )?;

    if !typed_dir.is_empty() {
        armada.settings.backup_dir = Some(PathBuf::from(typed_dir));
    }
    if !typed_keep.is_empty() {
        let keep: u32 = typed_keep
            .parse()
            .map_err(|_| format!("{typed_keep:?} is not a non-negative whole number"))?;
        armada.settings.keep = keep;
    }
    save(&armada)?;
    Ok(format!(
        "\nSettings saved: backup_dir = {}, keep = {}\n",
        armada.settings.backup_dir.as_ref().map_or_else(|| "(none)".to_string(), |p| p.display().to_string()),
        armada.settings.keep,
    ))
}

// ---------------------------------------------------------------------------
// the scripted/agent twin
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum SubVerb {
    List,
    Add { device: String },
    Forget { name: String },
    SetBackupDir { path: PathBuf },
    SetKeep { keep: u32 },
}

#[derive(Debug)]
pub struct ArmadaArgs {
    verb: SubVerb,
}

pub fn usage() -> String {
    "usage: cargo rivet armada                                 (interactive)\n\
     \x20             armada list\n\
     \x20             armada add            <server-device-name>\n\
     \x20             armada forget         <fleet-name>\n\
     \x20             armada set-backup-dir <path>\n\
     \x20             armada set-keep       <n>\n\
     \n\
     ADR-059 §6.3's Armada Registry — THIS workstation's list of Fleets: each\n\
     one's name and its server's host and pin. Lives at ~/.rivet/armada.\n\
     Whether a host is health-monitored or backed up is set in Rivet Monitor,\n\
     per host, and is not held here.\n\
     \n\
     Bare `cargo rivet armada` is the interactive tool — list, add, forget and\n\
     settings, menu-driven. `armada <verb>` is the same operations,\n\
     word-driven, for a script or an agent; neither reaches a Fleet's members\n\
     (those are read live from its server, never cached here) or the CA\n\
     (`ADR-059` §4.1: one root, shared, at ~/.rivet/ca).\n\
     \n\
     list              every Fleet: name and server host.\n\
     \n\
     add               bootstrap an existing Server-role device (the device\n\
     \x20                 register's own Fleet-role question) into the Armada.\n\
     \x20                 The Fleet's name is the device's name; its server\n\
     \x20                 host and pin are read exactly as `cargo rivet run\n\
     \x20                 <device>` would resolve them. Re-adding a Fleet that\n\
     \x20                 already exists refreshes its server.\n\
     \n\
     forget            remove one Fleet's record from the Armada. Says nothing\n\
     \x20                 about the Fleet's server or its members.\n\
     \n\
     set-backup-dir    where Armada backups are written.\n\
     set-keep          how many backups to keep.\n"
        .to_string()
}

/// # Errors
///
/// An unrecognised sub-verb, a missing device/Fleet name or path/count, an
/// unrecognised option, or a `set-keep` value that is not a whole number.
pub fn parse(args: &[String]) -> Result<ArmadaArgs, String> {
    let Some(sub) = args.first().map(String::as_str) else {
        return Err(format!(
            "`armada` needs a verb: list, add, forget, set-backup-dir or set-keep.\n\n{}",
            usage()
        ));
    };

    let one_arg = |label: &str| -> Result<String, String> {
        match args.get(1) {
            None => Err(format!("`armada {sub}` needs {label}.\n\n{}", usage())),
            Some(_) if args.len() > 2 => Err(format!(
                "`armada {sub}` takes one {label} and was given a second, {:?}",
                args[2]
            )),
            Some(value) => Ok(value.clone()),
        }
    };

    let verb = match sub {
        "list" => {
            if args.len() > 1 {
                return Err(format!("`armada list` takes no arguments; got {:?}", args[1]));
            }
            SubVerb::List
        }
        "add" => SubVerb::Add { device: one_arg("a server device name")? },
        "forget" => SubVerb::Forget { name: one_arg("a Fleet name")? },
        "set-backup-dir" => SubVerb::SetBackupDir { path: PathBuf::from(one_arg("a directory")?) },
        "set-keep" => {
            let text = one_arg("a whole number")?;
            let keep: u32 = text
                .parse()
                .map_err(|_| format!("{text:?} is not a non-negative whole number"))?;
            SubVerb::SetKeep { keep }
        }
        "-h" | "--help" => return Err(usage()),
        other => {
            return Err(format!(
                "`armada {other}` is not a verb. `armada` has list, add, forget, \
                 set-backup-dir and set-keep.\n\n{}",
                usage()
            ));
        }
    };

    Ok(ArmadaArgs { verb })
}

/// # Errors
///
/// Whichever sub-verb's own errors — see [`add_fleet_for`],
/// [`forget_fleet_record`].
pub fn run(args: &ArmadaArgs) -> Result<String, String> {
    let root = default_dir()?;
    match &args.verb {
        SubVerb::List => load(&root).map(|armada| render_list(&armada)),
        SubVerb::Add { device } => add_fleet_for(&root, device),
        SubVerb::Forget { name } => forget_fleet_record(&root, name),
        SubVerb::SetBackupDir { path } => set_backup_dir_record(&root, path),
        SubVerb::SetKeep { keep } => set_keep_record(&root, *keep),
    }
}

#[cfg(test)]
#[path = "armada_tests.rs"]
mod armada_tests;
