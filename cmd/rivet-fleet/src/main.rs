//! `rivet-fleet` — **the fleet register.** `ADR-034`.
//!
//! ```text
//! rivet-fleet                                    the terminal interface
//! rivet-fleet list devices                       the same operations, typed
//! rivet-fleet add device --name kitchen-pi
//! rivet-fleet export provisioning --device kitchen-pi
//! ```
//!
//! # What this program is
//!
//! `ADR-034` §1: several things were hand-managed and all of them are one job —
//! the provisioning document, which machine trusts which signing key, which
//! keys may reach a machine's control plane, and which services a machine runs.
//! The record of which machine holds which credential was *"in the maintainer's
//! memory"*. This is the thing that owns all of it.
//!
//! **The register is the authority and this is a way of reaching it**
//! (`ADR-034` §2). Not a server, and not a daemon: one binary over one store,
//! `cargo install`ed onto a workstation.
//!
//! # The composition root, and what that means here
//!
//! `ADR-014` §3 puts the wiring in `cmd/*` and nothing else. So this file
//! decides, and every module below it describes or computes:
//!
//! | Module | Owns |
//! | :-- | :-- |
//! | [`operation`] | what this tool can do, as a table both surfaces read |
//! | [`cli`] | `clap`, generated from that table |
//! | [`register`] | the records, and rendering them — no I/O |
//! | [`store`] | the I/O, and only the I/O |
//! | [`compose`] | a machine's document, made and then parsed by the board's own parser |
//! | [`diff`] | what writing that document would take away, and how to record it instead |
//! | [`credentials`] | `ADR-034` §7's one secret, obfuscated and named honestly |
//! | [`config`] | this tool's own settings, which are not any machine's |
//!
//! # It holds no signing key, and that is the design rather than an omission
//!
//! `ADR-034` §3, and §3.3 is the decisive form: the register and the signing
//! seed **must not share a failure domain**. `cargo rivet` holds a signing key;
//! this does not, will not, and has nowhere to put one. `crate::register`'s own
//! tests assert that a fully-populated record renders no secret material at all.
//!
//! # Exit status
//!
//! | | |
//! | :-- | :-- |
//! | `0` | it worked, or the plan was printed |
//! | `1` | the operation refused, and said why on stderr |
//! | `2` | the command line was wrong — `clap`'s own status |
//!
//! Two rather than one for a malformed command, because `cmd/rivet` makes the
//! same distinction: *this is not a thing you can ask* and
//! *this is a thing you asked and it did not happen* are different answers, and
//! a script that cannot tell them apart retries the wrong one.

mod cli;
mod card;
mod compose;
mod config;
mod credentials;
// What writing the composed document would take away from the machine that
// already has one — `ADR-034` §6.2's refusal, and the adoption that clears it.
mod diff;
// Reading JSON off a machine — the monitor pane's answers arrive as bytes
// something else wrote. Its header carries why this is written rather than
// depended on, and why it is a KNOWN duplicate of `cmd/rivet/src/json.rs`.
mod json;
mod member;
// This operator's own identity — `--identity` and `--pin`, and the control-plane
// transport they open to a member. `ADR-034` §11.5.1 and §11.5.2.
mod operator;
// The machine facts the monitor pane shows, and the tiered poller that fetches
// them without destroying the machine's journal. `ADR-035` phase 4.
mod telemetry;
mod operation;
mod register;
mod store;
mod tui;

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rivet_core::operation::Verb;
// For `read_document` on the transport — `ADR-034` §6.1's drift check reads the
// machine's own document, and that method is the trait's.
use rivet_remote::deploy::Remote as _;

use crate::cli::Request;
use crate::member::compare;
use crate::register::{Device, Register, RegisterError};

// **`2` is not written here.** `clap` exits the process itself when it will
// not accept a command line, with that status; a constant beside it would be a
// second spelling of somebody else's number (`ADR-011` §3), and one this code
// could never be observed to use.

/// An operation that ran and refused.
const REFUSED: u8 = 1;

/// The fleet name offered when nobody types one — `PLAN.md` §146.3.
///
/// **The same string `cmd/rivet-provision` defaults to, and deliberately not
/// the same constant.** That one is slugified into a certificate authority's
/// name constraint and is frozen at mint time; this one is a mutable label on a
/// register. Sharing a constant would couple two facts that are allowed to
/// diverge the moment an operator runs `set fleet --name` — see
/// [`name_the_fleet`] for the whole of the distinction.
const DEFAULT_FLEET: &str = "my-fleet";

fn main() -> ExitCode {
    let configuration = config::load();

    // Logging is initialised before anything can fail interestingly, and its
    // failure is not fatal — Ignition's own guidance, and the right call for a
    // tool whose job is to be usable on a fresh machine.
    let _log = ignition::logging::init(config::APP_NAME, &configuration.logging);

    let surface = ignition::cli::Cli::new(config::APP_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .about("The fleet register: enroll machines, compose their documents, reach them.")
        .update(
            ignition::cli::Surface::Both,
            vec![ignition::update::Binary::new(config::APP_NAME)],
        );

    let matches = cli::extend(surface.command()).get_matches();

    match surface.dispatch(&matches, &configuration.update) {
        Ok(ignition::cli::Outcome::Handled) => return ExitCode::SUCCESS,
        Ok(ignition::cli::Outcome::Passthrough) => {}
        Err(e) => return refuse(&e.to_string()),
    }

    let Some(root) = configuration.register_dir() else {
        return refuse(
            "this platform has no configuration directory that Ignition can name, so there is \
             nowhere for the register to be. Name one explicitly: `register = \"...\"` in \
             rivet-fleet's own configuration file.",
        );
    };

    // **The operator's own identity, read once for the whole run.** `ADR-034`
    // §11.5.2, and §11.5.1's *"one identity for one operator"* — there is no
    // per-device operator key here and none is invented.
    let operator = operator::Operator::from_matches(&matches);

    // **`ADR-052` §9 step 2's fleet repository, resolved once, HERE** — the
    // one place in this program that is allowed to read `HOME`/`USERPROFILE`
    // for it. Every call downstream takes the answer as a parameter; see
    // [`project`] for what it cost to have that be true only sometimes.
    let project_root = rivetos_fleet_repo::default_root();

    let Some(request) = cli::request(&matches) else {
        // Bare `rivet-fleet` opens the full-screen interface — `ADR-034`
        // §10.1, with `ratatui` adopted by §11.3.
        return match tui::run(&root, operator, project_root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => refuse(&format!("the terminal would not do it: {e}")),
        };
    };

    match perform(&request, &operator, &root, project_root.as_deref()) {
        Ok(report) => {
            print!("{report}");
            let _ = std::io::stdout().flush();
            ExitCode::SUCCESS
        }
        Err(message) => refuse(&message),
    }
}

/// Everything on stderr, and the code that says which kind of answer it is.
fn refuse(message: &str) -> ExitCode {
    eprintln!("rivet-fleet: {message}");
    ExitCode::from(REFUSED)
}

/// Do the thing, or say why not.
///
/// **Every arm is reached through `crate::operation::find`'s table**, because
/// `request` cannot be constructed for an operation the table does not carry.
/// The `match` below is over operations that are known to exist, and the final
/// arm is the honest one: an operation catalogued and not yet implemented says
/// so, rather than doing nothing and reporting success.
///
/// That last point is a defect this project shipped once, on 2026-08-26, when
/// `apply reload` landed reachable and uncatalogued in `cmd/rivet`. The lesson
/// taken from it was a catalogue gate; this is the same gate, from the start.
fn perform(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let operation = request.operation;
    match (operation.verb, operation.subject) {
        (Verb::Set, "fleet") => set_fleet(request, root, project_root),
        (Verb::Set, "service") => set_service(request, root, project_root),
        (Verb::Set, "listen") => set_listen(request, operator, root, project_root),
        (Verb::Set, "pin") => set_pin(request, root, project_root),
        (Verb::Set, "fleet-role") => set_fleet_role(request, root, project_root),
        (Verb::Set, "timezone") => set_timezone(request, root, project_root),
        (Verb::Set, "kiosk-auto-run") => set_kiosk_auto_run(request, root, project_root),
        (Verb::Set, "fleet-service") => set_fleet_service(request, root, project_root),
        (Verb::Set, "carry") => set_carry(request, root, project_root),
        (Verb::Set, "fleet-carry") => set_fleet_carry(request, root, project_root),
        (Verb::List, "services") => list_services(root),
        (Verb::List, "carry") => list_carry(root),
        (Verb::List, "devices") => list_devices(root),
        (Verb::List, "credentials") => list_credentials(root),
        (Verb::Add, "credential") => add_credential(request, root),
        (Verb::Remove, "credential") => remove_credential(request, root),
        (Verb::Get, "device") => get_device(request, root),
        (Verb::Get, "status") => get_status(request, operator, root),
        (Verb::Apply, "document") => apply_document(request, operator, root),
        (Verb::Apply, "trial-restart") => apply_trial_restart(request, operator, root),
        (Verb::Apply, "repo-delete") => apply_repo_delete(request, operator, root),
        (Verb::Add, "device") => add_device(request, root, project_root),
        (Verb::Add, "document") => add_document(request, operator, root, project_root),
        (Verb::Add, "repo") => add_repo(request, root, project_root),
        (Verb::Add, "peer") => add_peer(request, root, project_root),
        (Verb::Remove, "peer") => remove_peer(request, root, project_root),
        (Verb::Add, "identity") => add_identity(request, root, project_root),
        (Verb::Set, "repo") => set_repo(request, root, project_root),
        (Verb::Remove, "repo") => remove_repo(request, root, project_root),
        (Verb::List, "repos") => list_repos(root),
        (Verb::Remove, "device") => remove_device(request, root),
        (Verb::Export, "provisioning") => export_provisioning(request, root),
        (Verb::Export, "card-plan") => export_card_plan(request, root),
        (Verb::Explain, "device") => explain_device(request, root),
        (verb, subject) => Err(format!(
            "`{} {subject}` is in this tool's operation table and has no implementation behind \
             it yet. Nothing has been read and nothing has been changed. This is said rather \
             than silently succeeding, because a fleet tool that reported success without \
             acting is one whose register stops describing the fleet.",
            verb.as_str()
        )),
    }
}

fn load(root: &PathBuf) -> Result<Register, String> {
    store::load(root).map_err(|e| e.to_string())
}

/// **Save the whole register, exactly as `store::save` always has, and —
/// additionally, best-effort — project the same state into `ADR-052`'s fleet
/// repository.** `ADR-052` §9 step 2, and every call site below that used to
/// read `store::save(&register)` reads this instead.
///
/// **The primary save is unchanged by this function existing.** It is the
/// first thing done and its error, if any, is returned before the projection
/// ever runs — a caller of this function gets exactly the same `Result` it
/// would have gotten from `store::save` alone. The projection cannot make a
/// successful save fail, because `rivetos_fleet_repo::record_register` never
/// returns an error (see its own `Outcome::Refused`); it can only fail to
/// record history nobody was promised yet.
///
/// **`project_root` is resolved exactly once, in `main`, and handed down
/// through every caller of this function** — see [`project`] for why it is a
/// parameter here rather than something this function resolves for itself.
///
/// The projection's own outcome is printed rather than returned, so an
/// operator sees it without every one of this file's many `Result<String,
/// String>` handlers having to splice one more line onto their own report.
fn save(register: &Register, project_root: Option<&Path>) -> Result<(), String> {
    store::save(register).map_err(|e| e.to_string())?;
    project(register, project_root);
    Ok(())
}

/// [`save`]'s counterpart for `store::save_fleet`, which writes only the
/// fleet-wide file. The projection still renders the WHOLE register —
/// `rivetos_fleet_repo::record_register` takes no narrower argument, because
/// a projection that only ever wrote the fleet-wide half would drift from a
/// device file changed by some other call in the same run.
fn save_fleet(register: &Register, project_root: Option<&Path>) -> Result<(), String> {
    store::save_fleet(&register.root, &register.fleet).map_err(|e| e.to_string())?;
    project(register, project_root);
    Ok(())
}

/// The projection itself: record `register`'s current state into
/// `project_root`, if the caller was handed one.
///
/// **Silently does nothing on `None`.** A workstation with neither `HOME` nor
/// `USERPROFILE` set already cannot run most of this tool; a fleet repository
/// being unreachable on top of that is not a new fact worth printing on every
/// single command — see [`main`], which is the one place that turns their
/// absence into `None` in the first place.
///
/// # `project_root` is a parameter, and that used to be the defect
///
/// **This crate's own test suite calls handler functions like `add_device`
/// and `write_both` directly, not through `main`** — the ordinary way to unit
/// test a composition root without driving `clap` and a terminal. Once those
/// handlers called [`save`]/[`save_fleet`], every one of those tests started
/// reaching this function too, and this function used to resolve
/// `rivetos_fleet_repo::default_root()` for itself — reading the REAL
/// `HOME`/`USERPROFILE`, with no test double the way [`load`]/[`store::save`]
/// take an explicit `root`. Running `cargo test -p rivetos-fleet` once, before
/// that was noticed, created a live `~/.rivet/fleet` on the machine that ran
/// it, with the test suite's own fixture devices committed into it — exactly
/// the write `AGENTS.md` §10 forbids ("do not touch the maintainer's live
/// instance").
///
/// **The first fix was a `#[cfg(not(test))]` gate that compiled this whole
/// function as a no-op under `cargo test`**, which stopped the live write but
/// took the projection out of every one of this crate's 306 tests along with
/// it — production behaviour with nothing over it, in a feature the next
/// stage of `ADR-052` builds on. The defect was never that `project` read the
/// environment; it was that nothing else in this file did that, and every
/// sibling — [`load`], `store::save`, `store::save_fleet` — took its root as
/// an argument instead. This function now does the same: the environment is
/// read once, in `main`, and handed down as `project_root` like every other
/// root in this file. A test passes a temporary directory here exactly as it
/// already does for [`load`]/`store::save`, and the projection runs for real
/// under test — see `mod tests`'s `project_root_*` cases.
fn project(register: &Register, project_root: Option<&Path>) {
    let Some(root) = project_root else {
        return;
    };
    println!("  {}", rivetos_fleet_repo::record_register(root, register));
}

/// The device an operation names, resolved by label or identifier.
fn named<'a>(request: &Request, register: &'a Register) -> Result<&'a Device, String> {
    let asked = request
        .value("device")
        .ok_or("no device was named, and clap should have refused this first")?;
    register.resolve(asked).map_err(|e: RegisterError| e.to_string())
}

fn list_devices(root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    if register.devices.is_empty() {
        // **An empty register is an answer, not an error**, and it says what to
        // do next — a first run has nothing in it by construction.
        return Ok(format!(
            "no devices are enrolled in the register at {}.\nEnroll one: rivet-fleet add device \
             --name <label>\n",
            root.display()
        ));
    }
    let mut out = String::new();
    for device in register.devices.values() {
        out.push_str(&format!(
            "{:<20} {:<18} {:<12} {}\n",
            device.name,
            device.role.as_deref().unwrap_or("-"),
            device.location.as_deref().unwrap_or("-"),
            device.id,
        ));
    }
    Ok(out)
}

fn get_device(request: &Request, root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    toml_edit::ser::to_string_pretty(device).map_err(|e| e.to_string())
}

fn add_device(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let name = request
        .value("name")
        .ok_or("no name was given, and clap should have refused this first")?;

    // **The fleet gets its name here, once, on the way past** — `PLAN.md`
    // §146.3. See [`name_the_fleet`] for why this moment and not another.
    let named_fleet = name_the_fleet(&mut register, request.value("fleet"), project_root)?;

    // Checked before anything is written, `set_fleet_role`'s reason: a typo
    // here must not silently enroll a device with no role default rather than
    // the one that was meant.
    let fleet_role = match request.value("fleet-role") {
        None => None,
        Some(text) => Some(register::FleetRole::parse(text).ok_or_else(|| {
            format!("{text:?} is not one of SPEC-05 5's roles - `server` or `member`. Nothing has been enrolled.")
        })?),
    };

    let device = Device {
        id: mint_identifier(),
        name: name.to_owned(),
        role: request.value("role").map(str::to_owned),
        location: request.value("location").map(str::to_owned),
        fleet_role,
        ..Device::default()
    };
    let id = device.id.clone();
    register.enroll(device).map_err(|e| e.to_string())?;
    save(&register, project_root)?;

    Ok(format!(
        "enrolled {name} as {id}\n\
         {named_fleet}\
         \x20 record  {}\n\
         \x20 next    this device has no network interface, so no document can be composed \
         for it yet.\n",
        register.record_path(&id).display(),
    ))
}

/// The fleet's own name, asked for **once**, the first time a register holds a
/// device — `PLAN.md` §146.3.
///
/// # Why here, and not at some `init` verb
///
/// **There is no `init`.** A register springs into existence on the first
/// `add device` (`ADR-034` §2.2.1: the register lives wherever the fleet server
/// keeps it, and nothing ceremonially creates it). So the first enrollment is the
/// only moment that is guaranteed to happen exactly once per fleet, and asking
/// at any later one would be asking a question whose answer is already recorded.
///
/// # It asks ONCE, and only when there is something to ask about
///
/// The three conditions are `cmd/cargo-rivet`'s `resolve_name`, for its reasons:
///
/// * **`--fleet` given** — the caller has said, and is not asked again.
/// * **The fleet already has a name** — nothing is being chosen, and a question
///   whose answer is discarded teaches a person to press Return without reading.
/// * **Not a terminal** — a script, or CI. It takes the default and says so in
///   the report rather than blocking on a prompt nobody can answer.
///
/// # This is NOT the certificate authority's fleet label
///
/// `cmd/rivet-provision` takes a fleet name too, and slugifies it into the
/// authority's name constraint (`<fleet>.home.arpa`). **They are different
/// facts with different lifetimes**: the authority's label is frozen into a
/// root certificate at mint time and `PLAN.md` §149 is the open study of whether
/// it can ever be changed, while this one is a mutable label on a register
/// (`ADR-034` §6) that `set fleet --name` rewrites whenever an operator likes.
/// Recording them separately is deliberate; a reader who assumes one is the
/// other will expect a rename here to reach a certificate, and it does not.
///
/// Returns the line to splice into the enrollment report, empty when nothing was
/// named.
fn name_the_fleet(
    register: &mut Register,
    given: Option<&str>,
    project_root: Option<&Path>,
) -> Result<String, String> {
    if register.fleet.name.is_some() {
        return Ok(String::new());
    }

    // **Free text, and a space in it must survive** — §146.3's requirement is
    // not that a name can be chosen but that the operator's own survives the
    // round trip, so nothing here splits on whitespace or refuses one.
    let chosen = match given {
        Some(typed) => typed.to_owned(),
        None if !std::io::stdin().is_terminal() => DEFAULT_FLEET.to_owned(),
        None => {
            println!("\n  This register has no fleet name yet.");
            println!("  It is a label for a human looking at two registers, and it is");
            println!("  what `list devices` prints above the list. You can change it");
            println!("  later with `rivet-fleet set fleet --name`.");
            print!("\n  fleet name [{DEFAULT_FLEET}]: ");
            std::io::stdout().flush().ok();

            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|e| format!("cannot read a fleet name: {e}"))?;
            let typed = line.trim();
            if typed.is_empty() {
                DEFAULT_FLEET.to_owned()
            } else {
                typed.to_owned()
            }
        }
    };

    register.fleet.name = Some(chosen.clone());
    save_fleet(register, project_root)?;
    Ok(format!("\x20 fleet   {chosen}\n"))
}

/// Declare a repository on one device.
///
/// **Mints the identifier here rather than on the machine**, for `add device`'s
/// reason exactly: the register is the thing that holds the mapping from a name
/// to an identity (`ADR-034`), and an identifier minted on the machine would
/// have to travel back before anything could refer to it.
///
/// **This creates nothing.** It records an intention; `apply document` carries
/// it to the machine and the machine's own projection runs `git init --bare`.
/// That separation is `ADR-031` §7.5 - *a schema says what may be asked for; an
/// apply says what can be done* - and it is why an operator can declare a
/// repository for a machine that is switched off.
fn add_repo(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let device = named(request, &register)?.clone();
    let name = request
        .value("name")
        .ok_or("no name was given, and clap should have refused this first")?;

    // Refused HERE rather than at compose time, because the operator is in
    // front of the tool now and will be reading a diff later.
    let name = rivet_config::repos::RepoName::new(name).ok_or_else(|| {
        format!(
            "`{name}` is not a repository name; it must be {}",
            rivet_config::repos::RepoName::RULE
        )
    })?;

    if let Some(existing) = device.repos.get(name.as_str()) {
        return Err(format!(
            "`{name}` is already declared on {} as `{existing}`. To RENAME it, `set repo`; to \
             add a different repository, choose another name. Nothing has been changed.",
            device.name
        ));
    }

    let id = mint_identifier();
    let host = device.name.clone();
    register
        .devices
        .get_mut(&device.id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?
        .repos
        .insert(name.to_string(), id.clone());
    save(&register, project_root)?;

    Ok(format!(
        "declared {name} on {host} as {id}\n\
         \x20 next    `apply document` creates it on the machine. NOTHING EXISTS YET: this \
         register now says the repository should exist, and the machine has not been told.\n",
    ))
}

/// Rename a repository. **The identifier does not change**, which is the whole
/// point: the machine moves the directory rather than creating a second one.
fn set_repo(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let device = named(request, &register)?.clone();
    let from = request
        .value("repo")
        .ok_or("no repository was named, and clap should have refused this first")?;
    let to = request
        .value("name")
        .ok_or("no new name was given, and clap should have refused this first")?;

    let id = device.repos.get(from).cloned().ok_or_else(|| {
        format!(
            "{} declares no repository called `{from}`. `list repos` says what it does declare.",
            device.name
        )
    })?;
    let to = rivet_config::repos::RepoName::new(to).ok_or_else(|| {
        format!(
            "`{to}` is not a repository name; it must be {}",
            rivet_config::repos::RepoName::RULE
        )
    })?;
    if let Some(blocker) = device.repos.get(to.as_str()) {
        if blocker != &id {
            return Err(format!(
                "{} already declares a repository called `{to}`, and it is a different one \
                 (`{blocker}`). Renaming onto it would ask the machine to serve two \
                 repositories from one directory, which it refuses. Nothing has been changed.",
                device.name
            ));
        }
    }

    let host = device.name.clone();
    let record = register
        .devices
        .get_mut(&device.id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    record.repos.remove(from);
    record.repos.insert(to.to_string(), id.clone());
    save(&register, project_root)?;

    Ok(format!(
        "renamed {from} to {to} on {host}\n\
         \x20 id      {id} - UNCHANGED, which is what makes this a rename\n\
         \x20 next    `apply document` moves the directory on the machine. Every commit in it \
         comes along; nothing is created and nothing is lost.\n\
         \x20 note    anyone with `{from}` as a remote has to update it. The machine will \
         refuse the old name, because there is no longer a repository there.\n"
    ))
}

/// Stop declaring a repository. **The data is not deleted.**
fn remove_repo(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let device = named(request, &register)?.clone();
    let name = request
        .value("repo")
        .ok_or("no repository was named, and clap should have refused this first")?;

    let id = device.repos.get(name).cloned().ok_or_else(|| {
        format!(
            "{} declares no repository called `{name}`. `list repos` says what it does declare.",
            device.name
        )
    })?;

    if !request.confirmed {
        return Ok(format!(
            "PLAN - nothing has been changed.\n\
             \n\
             would undeclare  {name} ({id}) on {}\n\
             \n\
             THE REPOSITORY WOULD NOT BE DELETED. The next `apply document` leaves the \
             directory exactly where it is and the machine reports it as one nobody declared. \
             What is lost is this register's record of it - including the identifier, which \
             means re-declaring the same name later mints a NEW one and the machine sees a \
             collision rather than the repository it already has.\n\
             \n\
             To destroy the data, undeclare it first (this command) and then \
             `apply repo-delete --device <d> --repo {name} --id {id} --confirm`. That order \
             is not a formality: a repository the document still declares is recreated by the \
             next `apply document`, so the delete would look as though it had failed.\n\
             \n\
             Add `--confirm` to do it.\n",
            device.name,
        ));
    }

    let host = device.name.clone();
    register
        .devices
        .get_mut(&device.id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?
        .repos
        .remove(name);
    save(&register, project_root)?;

    Ok(format!(
        "undeclared {name} on {host}\n\
         \x20 was     {id}\n\
         \x20 data    UNTOUCHED. The directory is still on the machine and still holds every \
         commit; the next `apply document` reports it as undeclared and leaves it alone.\n\
         \x20 keep    write that identifier down if you may declare this name again - a new \
         one would collide with the directory that is already there.\n"
    ))
}

/// Every repository the register declares, by device.
fn list_repos(root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    let mut out = String::new();
    let mut found = 0;

    for device in register.devices.values() {
        if device.repos.is_empty() {
            continue;
        }
        out.push_str(&format!("{}\n", device.name));
        for (name, id) in &device.repos {
            found += 1;
            out.push_str(&format!("\x20 {name}  {id}\n"));
        }
    }

    if found == 0 {
        // An empty answer is an answer, and it says what to do next.
        return Ok("no device in this register declares a repository.\n\
                   \x20 next  `add repo --device <name> --name <repo>`\n"
            .to_string());
    }
    Ok(out)
}

fn remove_device(request: &Request, root: &PathBuf) -> Result<String, String> {
    let mut register = load(root)?;
    let device = named(request, &register)?.clone();

    if !request.confirmed {
        return Ok(format!(
            "PLAN - nothing has been changed.\n\
             \n\
             would forget  {} ({})\n\
             would remove  {}\n\
             \n\
             It would NOT touch the machine. The device keeps its identity, its keys and \
             whatever it is running; what is lost is this register's record of it.\n\
             \n\
             Add `--confirm` to do it.\n",
            device.name,
            device.id,
            register.record_path(&device.id).display(),
        ));
    }

    store::forget(&register, &device.id).map_err(|e| e.to_string())?;
    register.devices.remove(&device.id);
    Ok(format!(
        "forgot {} ({})\nNothing on the machine was changed.\n",
        device.name, device.id
    ))
}

fn export_provisioning(request: &Request, root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    // **The strict composer.** An export is a document leaving the register
    // for good — `compose`'s own header names the distinction — so this is
    // one of the callers `SPEC-05` §5's completeness gates apply to, unlike
    // `compare` (`add document`'s engine), which stays permissive so adoption
    // can fill the gap this would refuse on.
    compose::provisioning_complete_toml(device, &register.fleet).map_err(|e| e.to_string())
}

/// `PLAN.md` 0.0's runbook, for one device, with its own values in it.
///
/// **The maintainer's test of it, 2026-08-27:** an agent can build a card
/// because it reads the build script; this is what proves a human can. See
/// `crate::card` for what it deliberately does not do.
fn export_card_plan(request: &Request, root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    // Read rather than defaulted: a size this tool invented would be the exact
    // failure PLAN.md 13.4 records, arriving from the tool meant to prevent it.
    let card_mib = match request.value("card-mib") {
        Some(text) => Some(text.parse::<u32>().map_err(|_| {
            format!(
                "`--card-mib {text}` is not a number of MiB. It is the size of the PHYSICAL \
                 card, and it is asked for rather than guessed because /state is sized from \
                 it at build time and nothing on the running machine grows it."
            )
        })?),
        None => None,
    };
    crate::card::plan(device, &register, card_mib, root)
}

fn explain_device(request: &Request, root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;

    let mut out = format!("{} ({})\n", device.name, device.id);
    out.push_str(&format!(
        "  control-plane    {} from this device, {} from the fleet\n",
        device.control_plane_keys.len(),
        register.fleet.control_plane_keys.len(),
    ));
    out.push_str(&format!(
        "  role             {}\n",
        device.fleet_role.map_or(
            "not set - no SPEC-05 5 role default applies (ADR-004 6)".to_owned(),
            |role| format!("{role} - SPEC-05 5's defaults for it apply below")
        )
    ));
    for (name, enabled) in compose::services(device, &register.fleet) {
        let source = compose::service_source(device, &register.fleet, &name);
        out.push_str(&format!(
            "  services.{name:<16} {enabled:<6} from {source}\n"
        ));
    }
    // **`.rulings/` R-162 R30**: a pin per product, with its source — the
    // `services` block above's footing, minus a role layer `carry` never has.
    let carried = compose::carry(device, &register.fleet);
    if carried.is_empty() {
        out.push_str("  carry  NO PIN on any product\n");
    } else {
        for (product, version) in carried {
            let source = compose::carry_source(device, &register.fleet, &product);
            out.push_str(&format!(
                "  carry.{product:<15} {version:<10} from {source}\n"
            ));
        }
    }
    out.push_str(&format!(
        "  hostname         {}\n",
        match compose::hostname(device) {
            Some(name) => format!("{name}, from this device's label in the register"),
            None => format!(
                "NOT SET: the label {:?} is not a valid hostname, so this machine derives one \
                 from its own serial instead. Rename it to set one.",
                device.name
            ),
        }
    ));
    out.push_str(&format!(
        "  timezone         {}\n",
        device.timezone.as_deref().unwrap_or(
            "NOT SET - SPEC-05 5 refuses to export, build or write a FINAL document without \
             one. `add document` fills it in from a machine that already answers."
        )
    ));
    if let Some(app) = &device.kiosk_auto_run {
        out.push_str(&format!(
            "  kiosk.auto-run   {app} - SPEC-05 5: nothing here checks it is installed\n"
        ));
    }
    if compose::services(device, &register.fleet)
        .get("rivet-proxyd")
        .copied()
        .unwrap_or(false)
    {
        out.push_str(&format!(
            "  applications.rivet-proxyd.upstream-name  {}\n",
            compose::upstream_name(device).unwrap_or(
                "NOT SET - rivet-proxyd is enabled and SPEC-05 5 refuses a FINAL document \
                 without this, rather than boot a dead front door."
            )
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Reaching a member — ADR-034 11.4
// ---------------------------------------------------------------------------

/// **The control plane, pointed at the device an operation named.**
///
/// **Everything comes out of the record and nothing is typed.** `ADR-034` §9's
/// payoff — *"resolving address, access and trust from the record instead of
/// `--host root@<address>`"* — is this function: the address is
/// `crate::member::address_of`'s, the port is `Device::listen`'s (§6.1), and the
/// pin is `Device::pin`'s. `crate::operator` carries every refusal.
///
/// # There is one transport, and that is the whole design
///
/// `ADR-034` §11.5.1 refused every shell-out — *"there is nothing left at the
/// far end"* — and `ADR-035` left the image with exactly one way in. So this
/// returns the control-plane client or a refusal, and there is no second branch
/// for a reader to wonder about.
fn reach(
    operator: &operator::Operator,
    device: &Device,
) -> Result<rivet_remote::tls::Tls, String> {
    operator.open(device)
}

/// **Say what the machine's own document thinks, when it disagrees with us.**
///
/// `ADR-034` §6.1's detection, at the point the tool is already connected and
/// has the document in its hand. Costs nothing: `put_document` reads the
/// document back anyway, and this reads the copy that was already fetched.
fn note_drift(device: &Device, seen: Option<&str>, dialled: &str, report: &mut String) {
    if let Some(text) = seen
        && let Some(said) = member::listen_drift(device, text, dialled)
    {
        report.push_str(&said);
    }
}

fn apply_document(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    let mut report = String::new();
    if !request.confirmed {
        // The plan reaches no machine at all, so no transport is opened for it
        // — an `--identity`-less run must still be able to see what would land.
        member::put_document(device, &register.fleet, &mut member::Nothing, false, &mut report)?;
        return Ok(report);
    }
    let dialled = operator::control_plane_host(device)?;
    let mut remote = reach(operator, device)?;
    apply_onto(device, &register.fleet, &mut remote, &dialled, &mut report)?;
    Ok(report)
}

/// **`apply document --confirm` once the machine is reached**, with the
/// transport as a parameter so the order below can be proved against a
/// machine that misbehaves on demand.
fn apply_onto(
    device: &Device,
    fleet: &crate::register::Fleet,
    remote: &mut dyn member::Member,
    dialled: &str,
    report: &mut String,
) -> Result<(), String> {
    // **Before it is overwritten.** `ADR-034` §6.1 owes a disagreement report,
    // and this is the last moment the machine's own answer exists: the next
    // thing this operation does is replace it with the register's.
    // A read that fails here costs only the drift note: the removal guard
    // reads the document again inside `member::put_document`, and refuses the
    // write if that read fails.
    let seen = remote.read_document().unwrap_or_default();
    note_drift(device, seen.as_deref(), dialled, report);

    // **`ADR-034` §6.2, and it is the reason this verb is safe to run** —
    // enforced inside `member::put_document`, so the TUI's apply and
    // `set listen` cannot write past it either.
    member::put_document(device, fleet, remote, true, report)?;
    Ok(())
}

/// **Record what the machine has and the register lacks** — `ADR-034` §6.2's
/// recommended adoption, built.
///
/// # It only ever fills an absence
///
/// Every value it writes is one the composed document does not carry, which is
/// to say one the register has no answer for. **A value the register holds and
/// disagrees about is never touched**, and that is what keeps the register the
/// authoritative side: a tool that copied a machine's answer over a recorded
/// one would have quietly made the machine the authority (`ADR-034` §2).
///
/// # It says what it is copying and where from
///
/// Each value is named with its path in the machine's document and the field
/// it lands in. `ADR-034` §6.2: *"a register that silently absorbed a
/// machine's state would invert which side is authoritative."*
///
/// # Errors
///
/// Whatever the transport, the machine or `compose` refused.
fn add_document(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let device = named(request, &register)?.clone();
    // **The plan reads the machine too**, unlike every other plan in this
    // tool. There is nothing to plan otherwise: what this would record is a
    // fact about the machine and cannot be composed from the register. Reading
    // is not changing, and `get status` already reaches a machine without a
    // confirmation.
    let mut remote = reach(operator, &device)?;
    let machine = remote.read_document()?.ok_or_else(|| {
        format!(
            "{} holds no document at {}, so there is nothing to adopt. That is the state a \
             freshly-imaged board is in, and `apply document --confirm` is what puts one \
             there.",
            device.name,
            rivet_remote::deploy::DOCUMENT_PATH,
        )
    })?;
    adopt_report(&mut register, &device, &machine, request.confirmed, project_root)
}

/// **Everything `add document` does once it already holds the machine's
/// text** — split out so the reporting is testable without a fake transport,
/// `set_listen`'s `write_both` reason exactly: [`add_document`] is the thin
/// wrapper that reaches the machine, and this is the pure comparison and
/// report.
///
/// # It only ever fills an absence
///
/// Every value it writes is one the composed document does not carry, which is
/// to say one the register has no answer for. **A value the register holds and
/// disagrees about is never touched**, and that is what keeps the register the
/// authoritative side: a tool that copied a machine's answer over a recorded
/// one would have quietly made the machine the authority (`ADR-034` §2).
///
/// # It says what it is copying and where from
///
/// Each value is named with its path in the machine's document and the field
/// it lands in. `ADR-034` §6: *"a register that silently absorbed a
/// machine's state would invert which side is authoritative."*
///
/// # It reports CHANGES too, never only losses
///
/// A **loss** — the machine has a value the register does not — is what this
/// verb adopts. A **change** — both sides have a value and disagree — is
/// something this verb never touches, but it is exactly as real and
/// actionable a finding: it is the register disagreeing with a machine it
/// describes, over a value `apply document` will move the moment it runs. A
/// report naming one kind of finding and silently dropping the other reads
/// as complete when it is not, so both are always reported, whichever kind —
/// or both kinds — the comparison finds; only losses are ever written here.
///
/// # Errors
///
/// Whatever `compose` refused, or `toml_edit`'s words for text that is not
/// TOML at all.
fn adopt_report(
    register: &mut Register,
    device: &Device,
    machine: &str,
    confirmed: bool,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let (found, note) = compare(device, &register.fleet, machine)?;

    let mut report = String::new();
    if let Some(said) = note {
        report.push_str(&said);
    }
    if found.losses.is_empty() {
        report.push_str(&format!(
            "the register already holds everything {}'s document says, so there is nothing \
             to adopt and `apply document` would remove nothing.\n",
            device.name,
        ));
        if let Some(said) = diff::changes(&found) {
            report.push_str(&format!("\nIt would still {said}"));
        }
        return Ok(report);
    }

    // **Adopted into a COPY**, so a value the register cannot take leaves the
    // record on disk untouched rather than half-written.
    let mut record = device.clone();
    let mut taken: Vec<(String, String)> = Vec::new();
    let mut left: Vec<(String, String)> = Vec::new();
    for loss in &found.losses {
        match adopt(&mut record, loss) {
            Ok(into) => taken.push((loss.where_it_is(), into)),
            Err(why) => left.push((loss.where_it_is(), why)),
        }
    }

    if !confirmed {
        report.push_str(&format!(
            "PLAN - the register has not been changed, and neither has the machine. This \
             READ {} on {} and wrote nothing at either end.\n\n",
            rivet_remote::deploy::DOCUMENT_PATH,
            device.name,
        ));
        for (path, into) in &taken {
            report.push_str(&format!("would record  {path} \
              -> {into}\n"));
        }
        for (path, why) in &left {
            report.push_str(&format!("CANNOT        {path} \
              {why}\n"));
        }
        report.push_str("\nAdd `--confirm` to write the first list into the register.\n");
        if !left.is_empty() {
            report.push_str(
                "The second list would still be there afterwards, so `apply document` would \
                 still refuse. Each entry says what it would take.\n",
            );
        }
        // **The `# It reports CHANGES too` half, made concrete.** A CHANGE is
        // never adopted - only a LOSS is - so it is neither in the "would
        // record" list above nor written by a later `--confirm` of this same
        // command. Left unreported it is invisible twice over, so it is named
        // here even though this verb will never be the one that moves it.
        if let Some(said) = diff::changes(&found) {
            report.push_str(&format!(
                "\nadd document does not adopt these - a differing value is a CHANGE and not a \
                 loss (ADR-034 6), and this verb only ever fills an absence. `apply document` \
                 would still {said}"
            ));
        }
        return Ok(report);
    }

    let id = record.id.clone();
    let name = record.name.clone();
    *register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")? = record;
    save(register, project_root)?;

    report.push_str(&format!("adopted {} value(s) into {name}'s record\n", taken.len()));
    for (path, into) in &taken {
        report.push_str(&format!("\x20 {path} \
     -> {into}\n"));
    }
    report.push_str(&format!(
        "\x20 source  {} on {name}, read just now\n\
         \x20 NOTE    the register now says what the machine says. That is a COPY of a fact, \
         not a judgement that the fact is right - ADR-034 2 keeps the register \
         authoritative, and this only ever fills in where it had no answer at all. Read \
         them: `rivet-fleet get device --device {name}`.\n\
         \x20 machine NOTHING was written to it.\n",
        rivet_remote::deploy::DOCUMENT_PATH,
    ));
    if left.is_empty() {
        report.push_str(&format!(
            "\x20 next    `rivet-fleet apply document --device {name} --confirm`\n"
        ));
    } else {
        report.push_str(&format!(
            "\n{} value(s) could NOT be recorded, so `apply document` will still refuse:\n",
            left.len()
        ));
        for (path, why) in &left {
            report.push_str(&format!("\x20 {path} \
     {why}\n"));
        }
    }
    if let Some(said) = diff::changes(&found) {
        report.push_str(&format!("\n`apply document` would still {said}"));
    }
    Ok(report)
}

/// Put one lost value into a device record, or say why the register cannot
/// hold it.
///
/// **The device record only, and never the fleet's.** A fleet value composes
/// into *every* machine's document, so adopting one from one machine would
/// change what the other machines get — which is a decision about the fleet
/// taken from a fact about one member.
///
/// # Errors
///
/// A sentence naming why this value has nowhere to go.
fn adopt(device: &mut Device, loss: &diff::Loss) -> Result<String, String> {
    let segments: Vec<&str> = loss.path.iter().map(String::as_str).collect();
    let value = loss.value.clone();
    match segments.as_slice() {
        ["control-plane", "listen"] => {
            device.listen = Some(value.clone());
            Ok(format!("listen = \"{value}\""))
        }
        // **A bare top-level key, exactly like `hostname` above it in the
        // document** - but unlike `hostname` there is no second field this
        // could collide with the meaning of, so unlike that arm this one
        // records rather than refuses. `SPEC-05` §5: a machine already
        // running with a zone set is the one case this tool can fill the gap
        // from without asking the operator to retype what the board already
        // answered.
        ["timezone"] => {
            device.timezone = Some(value.clone());
            Ok(format!("timezone = \"{value}\""))
        }
        // **`[kiosk] auto-run`**, a bare value under its own table rather than
        // a top-level key, but the same shape as `timezone` above it: one
        // name, adopted from a machine that already answers it, with nothing
        // here to refuse. `SPEC-05` §5 leaves whether the name will resolve
        // to an installed application deliberately unchecked - see
        // `crate::register::Device::kiosk_auto_run` for why this arm does
        // not try.
        ["kiosk", "auto-run"] => {
            device.kiosk_auto_run = Some(value.clone());
            Ok(format!("kiosk.auto-run = \"{value}\""))
        }
        // **Any key, under any application.** `ADR-038` §4 puts no schema on
        // this region at all - "the keys come from a signed artifact's
        // manifest, not from here" - so this arm has no field name to check
        // and nothing it can refuse: whatever the machine's own document
        // carries under `[applications.*]` is, by construction, a value
        // `rivet-config` already accepted. That is also why this arm is safe
        // to write with `.entry(...).or_default()` where the resolver arm
        // below is not - there is no second, more specific pattern for an
        // application key that this could shadow, so there is nothing for an
        // unconditional insert to pre-empt.
        ["applications", app, key] => {
            device
                .applications
                .entry((*app).to_owned())
                .or_default()
                .insert((*key).to_owned(), value.clone());
            Ok(format!("applications.{app}.{key} = \"{value}\""))
        }
        ["services", service, "enabled"] => {
            // **`true` and nothing else is true.** The document's own parser
            // has already read this as a boolean and rendered it back, so the
            // only two strings that can arrive here are `true` and `false`.
            let enabled = value == "true";
            device.services.insert((*service).to_owned(), enabled);
            Ok(format!("services.{service} = {enabled}"))
        }
        ["services", service, "memory"] => {
            device.service_memory.insert((*service).to_owned(), value.clone());
            Ok(format!("service_memory.{service} = \"{value}\""))
        }
        ["repos", repo, "id"] => {
            device.repos.insert((*repo).to_owned(), value.clone());
            Ok(format!("repos.{repo} = \"{value}\""))
        }
        // **`[network.resolver]` is not an interface, and this arm must come
        // BEFORE the generic one below or `interface` binds `"resolver"` and
        // walks straight into it.** That is the exact defect `ADR-034` §6
        // retired: the generic arm's `.entry(...).or_insert_with(...)`
        // creates its default interface as a SIDE EFFECT of looking one up,
        // before the field name is even checked - so a `nameservers` loss
        // used to insert `[interfaces.resolver] method = "dhcp"` and only
        // THEN discover it had nowhere to put `nameservers` and return `Err`.
        // The mutation had already happened; the `Err` did not undo it, and
        // `add_document` writes `record` regardless of which losses were
        // recorded, so a refusal here still left a bad interface on disk. See
        // `crate::register::Resolver` for the field that gives this
        // information an honest home instead.
        //
        // A `Leaf::Many` loses one element per call (`rivet_core::leaf`), so
        // `add document` against a machine with two nameservers calls this
        // arm twice, once per address - appended and de-duplicated rather
        // than replaced, so neither call clobbers the other.
        ["network", "resolver", field @ ("nameservers" | "search")] => {
            let resolver = device.resolver.get_or_insert_with(register::Resolver::default);
            let list = match *field {
                "nameservers" => &mut resolver.nameservers,
                "search" => &mut resolver.search,
                _ => unreachable!("the pattern above admits only these two"),
            };
            if !list.iter().any(|already| already == &value) {
                list.push(value.clone());
            }
            Ok(format!("resolver.{field} += \"{value}\""))
        }
        ["network", "resolver", other] => Err(format!(
            "[network.resolver] has no `{other}` - rivet-config's Resolver::DECLARED_KEYS is \
             nameservers and search and nothing else, so a document the board's own parser \
             accepted could not have produced this loss."
        )),
        ["network", interface, field] => {
            // **`dhcp` is a placeholder and not a guess.** A document the
            // machine's parser accepted always carries `method` for an
            // interface, so a `method` loss arrives for every interface an
            // `address` loss arrives for and overwrites this. It is written
            // rather than left empty because an empty method composes a
            // document `rivet-config` refuses, which would turn a missing
            // field into an unreadable register.
            let entry = device
                .interfaces
                .entry((*interface).to_owned())
                .or_insert_with(|| register::Interface {
                    method: "dhcp".to_owned(),
                    address: None,
                    gateway: None,
                });
            match *field {
                "method" => entry.method = value,
                "address" => entry.address = Some(value),
                "gateway" => entry.gateway = Some(value),
                other => {
                    return Err(format!(
                        "the register's interface record has no `{other}` - it holds method, \
                         address and gateway and nothing else."
                    ));
                }
            }
            Ok(format!("interfaces.{interface}.{field}"))
        }
        ["hostname"] => Err(format!(
            "the composed hostname IS the device's label, and adopting `{value}` would mean \
             renaming the record to match whatever the machine answers to - which lets the \
             machine decide what the register calls it. Rename it deliberately if that is \
             what you want: `name` in the device record."
        )),
        ["volumes", volume, ..] => Err(format!(
            "the register has NO field for volumes - ADR-034 6's table does not list them - \
             so `{volume}` cannot be recorded here at all. Growing the record is a ruling, \
             not a fix."
        )),
        _ => Err(format!(
            "the register has no field for `{}`.",
            loss.where_it_is()
        )),
    }
}

fn apply_trial_restart(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    let mut report = String::new();
    if !request.confirmed {
        // As `apply_document`: a plan touches nothing, so it opens nothing and
        // needs no identity to be shown.
        member::trial_restart(device, &mut member::Nothing, false, &mut report)?;
        return Ok(report);
    }
    let mut remote = reach(operator, device)?;
    member::trial_restart(device, &mut remote, true, &mut report)?;
    Ok(report)
}

/// **Destroy a repository on a device.** The only operation in this tool that
/// loses data.
///
/// The identifier is typed rather than looked up in the register, and that is
/// the point of it: if it came from the register it would agree with the
/// register by construction, and the thing being guarded against is exactly a
/// register and a disk that have drifted apart. The machine compares what was
/// typed to what is in the directory.
///
/// **It refuses a repository this register still declares**, before the machine
/// is reached at all - a declared repository is recreated by the next `apply
/// document`, so the delete would look as though it had failed. The machine
/// makes the same check for itself, against its own document, because a
/// register can be out of date.
fn apply_repo_delete(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    let name = request
        .value("repo")
        .ok_or("no repository was named, and clap should have refused this first")?;
    let id = request
        .value("id")
        .ok_or("no identifier was given, and clap should have refused this first")?;

    if let Some(declared) = device.repos.get(name) {
        return Err(format!(
            "{} STILL DECLARES `{name}` as `{declared}`, so destroying it would be undone: the \
             next `apply document` would create it again, empty. Undeclare it first - \
             `remove repo --device {} --repo {name} --confirm` - and then ask again. Nothing \
             has been touched.",
            device.name, device.name,
        ));
    }
    if !rivet_core::identifier::is_identifier(id) {
        return Err(format!(
            "`{id}` is not an identifier; {}. It is the one the repository was declared with - \
             if this register no longer has it, the machine has it in its own \
             /state/git/{name}.git/rivet-id.",
            rivet_core::identifier::RULE,
        ));
    }

    if !request.confirmed {
        return Ok(format!(
            "PLAN - {} has not been touched.\n\
             \n\
             would run     rivet apply repo-delete {name} {id} confirm\n\
             would destroy /state/git/{name}.git, AND EVERY COMMIT IN IT\n\
             \n\
             THIS IS NOT RECOVERABLE. Nothing on that machine keeps a copy and nothing here \
             does either. If the only clone is on the board, this is the end of it - check \
             that you have one somewhere else FIRST.\n\
             \n\
             The machine will refuse if the directory carries an identifier other than {id}, \
             and it will refuse if its own document still declares the repository. Neither of \
             those can be checked from here.\n\
             \n\
             Add `--confirm` to do it.\n",
            device.name,
        ));
    }

    let remote = reach(operator, device)?;
    // Through the CATALOGUE rather than as a word list — `crate::member::Member`
    // carries why, and this was the fifth site in this crate still speaking to a
    // transport as though it were a shell.
    let said = member::Member::perform(&remote, &["apply", "repo-delete", name, id, "confirm"])?;
    Ok(format!(
        "asked {} to destroy {name}\n{said}\n\
         The request was DELIVERED and that is all this can tell you. The supervisor performs \
         it and can still refuse - `get log --device {}` is where it says which.\n",
        device.name, device.name,
    ))
}

fn get_status(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
) -> Result<String, String> {
    let register = load(root)?;
    let device = named(request, &register)?;
    let mut remote = reach(operator, device)?;
    member::status(&mut remote)
}

// ---------------------------------------------------------------------------
// ADR-034 §5 — the register's everyday operation
// ---------------------------------------------------------------------------

/// Read `--enabled`, which is a boolean and is spelled as one.
///
/// **`true` or `false`, and nothing else is accepted** — not `yes`, not `on`,
/// not `1`. `ADR-006` §6.4's argument against aliases applied to a value: a
/// surface with two spellings of one word is one that must be memorised rather
/// than predicted, and this is the value an operator types most often.
/// The `memory` an operation names, if it named one.
///
/// **Three answers, not two**, and the middle one is why this is not a
/// `Option<String>`:
///
/// * `Ok(None)` — nothing was said, so leave whatever the record holds;
/// * `Ok(Some(None))` — the word `none`, so remove the cap;
/// * `Ok(Some(Some(text)))` — a cap, already parsed and re-rendered.
///
/// Collapsing the first two would make an omitted argument mean *remove*, and
/// an operator who set a cap yesterday and toggled `enabled` today would lose
/// it without being told.
///
/// **Parsed here and stored as its own canonical text.** `MemoryLimit`'s
/// `Display` is what `ADR-007` §7.2's document takes, so recording that rather
/// than the operator's keystrokes means `32m` and `32M` become one value and a
/// composed document can never carry a spelling the machine would refuse.
fn memory_in(request: &Request) -> Result<Option<Option<String>>, String> {
    let Some(text) = request.value("memory") else {
        return Ok(None);
    };
    if text == "none" {
        return Ok(Some(None));
    }
    match rivet_core::service::MemoryLimit::parse(text) {
        Some(limit) => Ok(Some(Some(limit.to_string()))),
        None => Err(format!(
            "`{text}` is not a memory size. ADR-007 §7.2 takes a number and one of `K`, `M` \
             or `G` - `32M`, `512K`, `1G` - or the word `none` to remove a cap. It is refused \
             here rather than on the machine, because a register that composed a document the \
             machine refuses would be describing a state no machine can be in."
        )),
    }
}

/// The `version` a carry operation named, or the clearing word —
/// `memory_in`'s two-answer shape (`.rulings/` R-162 R30), minus its
/// "nothing said" branch: `set carry`/`set fleet-carry` have no other effect
/// to perform with the flag omitted, so `version` stays a required flag and
/// this only ever returns one of the two answers below.
///
/// * `Ok(None)` — the word `none`, so the pin is cleared;
/// * `Ok(Some(text))` — a version to pin, exactly as `cargo rivet fleet
///   publish` recorded it. Not parsed further here: a carry pin is an
///   opaque version string, checked only by `ADR-050` §5's build-time
///   resolution against what the repository actually holds.
fn carry_version_in(request: &Request) -> Result<Option<&str>, String> {
    let text = request
        .value("version")
        .ok_or("no version was given, and clap should have refused this first")?;
    if text == "none" {
        return Ok(None);
    }
    Ok(Some(text))
}

fn enabled_in(request: &Request) -> Result<bool, String> {
    match request.value("enabled") {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(format!(
            "`{other}` is not `true` or `false`. Nothing else is accepted - not `yes`, not `on`, \
             not `1` - because a surface with two spellings of one word is one that has to be \
             memorised rather than predicted."
        )),
        None => Err("no value was given, and clap should have refused this first".to_owned()),
    }
}

/// The service an operation names, refused early if the machine could not have
/// it.
///
/// **Checked against `rivet_core::service::ServiceName` and no further.** The
/// register does not know which services a machine can run — that is the
/// image's definitions plus whatever is installed — so this refuses only a name
/// the *document* could not carry, and leaves the machine to refuse a name it
/// does not have. Two answers to *what can this run* is `ADR-011` §3.
fn service_in(request: &Request) -> Result<String, String> {
    let name = request
        .value("service")
        .ok_or("no service was named, and clap should have refused this first")?;
    if rivet_core::service::ServiceName::new(name).is_none() {
        return Err(format!(
            "{name:?} is not a name a configuration document can carry, so it would be refused \
             by the machine rather than written. Nothing has been changed."
        ));
    }
    Ok(name.to_owned())
}

fn set_service(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let id = named(request, &register)?.id.clone();
    let service = service_in(request)?;
    let enabled = enabled_in(request)?;
    // Parsed BEFORE the register is touched, so a bad size changes nothing.
    let memory = memory_in(request)?;

    let device = register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    let previous = device.services.insert(service.clone(), enabled);
    let cap = match &memory {
        None => device.service_memory.get(&service).cloned(),
        // `remove` RETURNS the old value, and reporting that would tell an
        // operator the cap they just cleared is still there. Found by reading
        // this command's own output on 2026-08-28 (PLAN.md §67.6).
        Some(None) => {
            device.service_memory.remove(&service);
            None
        }
        Some(Some(text)) => {
            device.service_memory.insert(service.clone(), text.clone());
            Some(text.clone())
        }
    };
    let name = device.name.clone();
    save(&register, project_root)?;

    Ok(format!(
        "{name} {}\n\
         \x20 {service} = {enabled}{}\n\
         {}\
         \x20 next    the register says so; the MACHINE does not yet. Send it:\n\
         \x20         rivet-fleet apply document --device {name} --confirm\n",
        if previous.is_none() {
            "now names a service it did not"
        } else {
            "changed"
        },
        match previous {
            Some(was) if was == enabled => " (it already said that)".to_owned(),
            Some(was) => format!(" (was {was})"),
            None => String::new(),
        },
        // A cap line only when there is one, so the common answer stays two
        // lines. `memory_in`'s three answers all land here as "what the record
        // holds NOW", which is the only thing an operator can act on.
        match &cap {
            Some(text) => format!("\x20 memory  = {text}\n"),
            None => String::new(),
        },
    ))
}

// ---------------------------------------------------------------------------
// ADR-034 §6.1 — the two facts that are HOW THE TOOL REACHES THE MACHINE
// ---------------------------------------------------------------------------

/// **Record this machine's control-plane public key.**
///
/// The half `add identity` cannot do: that verb MINTS an identity, which needs
/// a new card and locks out every peer pinning the old key. A board provisioned
/// by a card build already has an identity, minted by
/// `cargo rivet identity new`, and until 2026-08-29 there was **no way to tell
/// the register what its public half was** — so `--pin` had to be typed on
/// every command, while `--help` said the register already held one. A help
/// string describing a field nothing could populate is `ADR-012` §7.4's
/// confident sentence a reader cannot test.
///
/// # This one is NOT §6.1's both-or-neither, and here is why
///
/// The listener lives in the machine's document, so changing it is one
/// operation writing two places. A pin lives in the machine's **identity
/// files**, which reach a board on a card and not over a wire — `ADR-019` §3.1
/// blanks the private half at first boot. So there is nothing on the machine
/// for this to write, and this verb only ever catches the register up with a
/// fact that is already true of the board. Making it a concurrent write would
/// mean inventing a way to replace a device identity remotely, which is not
/// this ticket and would need its own ruling.
///
/// # Errors
///
/// A pin that is not 64 hex characters — refused here rather than at the first
/// connection, because a record that cannot be used is worse than no record.
fn set_pin(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let id = named(request, &register)?.id.clone();
    let pin = request
        .value("pin")
        .ok_or("no key was given, and clap should have refused this first")?;

    // **Checked through `rivetos-tls`'s own parser**, never a length test
    // written here: it is what `--pin` is read with at connection time, so a
    // value this accepts is a value that will connect (`ADR-011`).
    rivetos_tls::parse_pins(pin).map_err(|e| {
        format!(
            "that is not a control-plane key: {e}\n\
             \x20 note  this is ADR-025 §2's pin - 64 hex characters, what `cargo rivet \
             identity new` printed when this machine's card was built. NOT a whole line of \
             key material, and NOT a key id.\n\
             \x20 fix   read it back from the identity directory the card was built from - \
             `cargo rivet identity show --directory <dir>`.\n\
             Nothing has been changed."
        )
    })?;

    let device = register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    let previous = device.pin.replace(pin.to_owned());
    let name = device.name.clone();
    save(&register, project_root)?;

    Ok(format!(
        "{name} {}\n\
         \x20 pin     {pin}\n\
         {}\
         \x20 next    commands against this device no longer need --pin. Try it:\n\
         \x20         rivet-fleet get status --device {name} --identity <dir>\n",
        if previous.is_none() {
            "now has a control-plane key in the register"
        } else {
            "had its control-plane key REPLACED"
        },
        match &previous {
            Some(was) if was == pin => "\x20 note    it already said that\n".to_owned(),
            Some(was) => format!(
                "\x20 was     {was}\n\
                 \x20 WARNING if the machine still holds the OLD key, this record now points \
                 at something that will not answer. Nothing was sent to it; only this \
                 register changed.\n"
            ),
            None => String::new(),
        },
    ))
}

/// Record which `SPEC-05` §5 role a device's document should default from —
/// `ADR-004` §6.
///
/// **Register-only, like [`set_pin`] and for the same reason.** The role only
/// ever feeds `crate::compose`'s role-default logic the next time this
/// device's document is composed; nothing on the machine changes until an
/// operator runs `apply document` or builds a card, so there is nothing here
/// for `ADR-034` §6.1's both-or-neither rule to bind.
///
/// # Errors
///
/// A word that is not one of `server` or `member` — refused here by name
/// rather than silently applying no role at all, which is what
/// `Option::None` already means and would make a typo indistinguishable from
/// a deliberate refusal. **There is no longer a word this command accepts
/// for clearing a role back to none** — `"stand-alone"` was that word and is
/// retired along with `FleetRole::StandAlone` (a ratchet increment); a
/// device record's `fleet-role` key can still be removed by hand.
fn set_fleet_role(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let id = named(request, &register)?.id.clone();
    let text = request
        .value("role")
        .ok_or("no role was given, and clap should have refused this first")?;
    let role = register::FleetRole::parse(text).ok_or_else(|| {
        format!(
            "{text:?} is not one of SPEC-05 5's roles - `server` or `member`, and nothing else, \
             so that this tool and rivet-provision's own menu apply the same defaults for the \
             same word. Nothing has been changed."
        )
    })?;

    let device = register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    let previous = device.fleet_role.replace(role);
    let name = device.name.clone();
    save(&register, project_root)?;

    Ok(format!(
        "{name} {}\n\
         \x20 role    {role}\n\
         \x20 next    `rivet-fleet export provisioning --device {name}` to see what this role \
         adds.\n",
        if previous.is_none() {
            "now has a fleet role"
        } else {
            "had its fleet role CHANGED"
        },
    ))
}

/// Record this device's timezone directly — `SPEC-05` §5.
///
/// **Register-only, like [`set_pin`] and [`set_fleet_role`].** `timezone`
/// only ever feeds `crate::compose`'s document the next time this device's
/// record is exported, built onto a card, or written to a machine; nothing
/// on the machine changes until then, so there is nothing here for `ADR-034`
/// §6's both-or-neither rule to bind.
///
/// # Why this verb has to exist at all
///
/// `adopt` already fills [`Device::timezone`] from a machine that already
/// answers it, and that is `add document`'s job. It cannot help a
/// **brand-new** record — one minted before its machine has ever booted —
/// because there is nothing to adopt from, and `SPEC-05` §5 refuses to
/// export, build or write a FINAL document without one. Before this verb the
/// only way to answer that gap was hand-editing the register file, which
/// `SPEC-05` §6 rules out.
///
/// # Errors
///
/// A name that is not shaped like an IANA zone. **Checked here through
/// `rivet_core::timezone::Timezone`**, the exact grammar `rivet-config`'s own
/// parser applies, so a value this accepts is a value the machine would
/// accept too (`ADR-011`).
///
/// **What this cannot check, said rather than guessed past: whether the zone
/// exists.** `cmd/rivet-provision`'s own menu holds a typed name against a
/// real zoneinfo database, or a compiled copy of one when no database is
/// readable — and that check lives in a binary crate with no library target,
/// so nothing outside it can call the function that runs it. Duplicating a
/// six-hundred-name table here would be a second copy of a fact `ADR-011`
/// gives one authority, so this stops at grammar and says so in the refusal.
/// A well-formed name for a zone no image carries passes here and is
/// reported by the machine's own projection instead, at first boot.
fn set_timezone(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let id = named(request, &register)?.id.clone();
    let zone = request
        .value("timezone")
        .ok_or("no timezone was given, and clap should have refused this first")?;

    if rivet_core::timezone::Timezone::new(zone).is_none() {
        return Err(format!(
            "`{zone}` is not a time zone name; it must be {}\n\
             \x20 note  this checks GRAMMAR only - the same check rivet-config's own parser \
             runs. Whether `{zone}` is a real zone this tool cannot say: that needs a \
             zoneinfo database, and rivet-provision's copy of one lives in a binary crate \
             this tool cannot reach.\n\
             Nothing has been changed.",
            rivet_core::timezone::Timezone::RULE
        ));
    }

    let device = register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    let previous = device.timezone.replace(zone.to_owned());
    let name = device.name.clone();
    save(&register, project_root)?;

    Ok(format!(
        "{name} {}\n\
         \x20 timezone  {zone}\n\
         {}\
         \x20 next    the register says so; the MACHINE does not yet. Send it:\n\
         \x20         rivet-fleet apply document --device {name} --confirm\n",
        if previous.is_none() {
            "now has a timezone"
        } else {
            "had its timezone CHANGED"
        },
        match &previous {
            Some(was) if was == zone => "\x20 note    it already said that\n".to_owned(),
            Some(was) => format!("\x20 was     {was}\n"),
            None => String::new(),
        },
    ))
}

/// Record which application takes this device's display directly —
/// `SPEC-05` §5.
///
/// **Register-only, [`set_timezone`]'s reason exactly.** `kiosk_auto_run`
/// only ever feeds `crate::compose`'s `[kiosk]` table the next time this
/// device's record is exported, built onto a card, or written to a machine.
///
/// # This does not, and cannot, check installation
///
/// `SPEC-05` §5 names the gap on purpose: nothing at the moment a document is
/// composed can know whether the application this names will ever be
/// installed, because the manifest that would say so is written by an
/// install that happens afterwards. So this records the name and nothing
/// more — a name matching nothing takes the display and draws nothing on it,
/// with no message anywhere, and this tool cannot check for it.
///
/// # Errors
///
/// An empty or whitespace-only name. There is no way to spell *no
/// application* through this verb — an absent [`Device::kiosk_auto_run`]
/// already means that, silently, the way every other optional field on
/// [`Device`] does.
fn set_kiosk_auto_run(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let id = named(request, &register)?.id.clone();
    let application = request
        .value("application")
        .ok_or("no application was named, and clap should have refused this first")?;
    if application.trim().is_empty() {
        return Err(
            "an empty name was given, and there is no way to spell \"no application\" through \
             this verb - an absent kiosk_auto_run already means that. Nothing has been \
             changed."
                .to_owned(),
        );
    }

    let device = register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    let previous = device.kiosk_auto_run.replace(application.to_owned());
    let name = device.name.clone();
    save(&register, project_root)?;

    Ok(format!(
        "{name} {}\n\
         \x20 kiosk.auto-run  {application}\n\
         {}\
         \x20 note    NOTHING HERE CAN CHECK THAT {application} WILL BE INSTALLED (SPEC-05 5) \
         - a name matching nothing takes the display and draws nothing on it.\n\
         \x20 next    the register says so; the MACHINE does not yet. Send it:\n\
         \x20         rivet-fleet apply document --device {name} --confirm\n",
        if previous.is_none() {
            "now names an application for its display"
        } else {
            "had its display application CHANGED"
        },
        match &previous {
            Some(was) if was == application => "\x20 note    it already said that\n".to_owned(),
            Some(was) => format!("\x20 was     {was}\n"),
            None => String::new(),
        },
    ))
}

/// **Move a machine's control plane, writing the register and the document
/// together — `ADR-034` §6.1's ruled concurrency.**
///
/// > The tools that change the listener are ours. So the operation that changes
/// > it writes the register and the document together — both, or neither.
///
/// # The order, and why the register is written FIRST
///
/// 1. compose the document with the new value, so a record `rivet-config` would
///    refuse is refused **before anything is written anywhere**;
/// 2. write the register — proving the file is writable while nothing has been
///    sent, so a disk that will not take it is NEITHER;
/// 3. reach the machine **on the OLD value**, because that is the one currently
///    answering — a listener change takes effect when `rivet-mcpd` restarts;
/// 4. put the document, read it back, reload;
/// 5. **if step 4 failed, put the register back.**
///
/// The alternative order — machine first, register after — was rejected because
/// its failure has no local recovery: a board that took the new document while
/// the register still holds the old value has moved somewhere this tool cannot
/// dial, and the only thing that could correct the register is a file this tool
/// would then have to be told about by hand anyway. Writing the register first
/// makes the rollback a local file operation, which is the one kind that can be
/// relied on to work at the moment the network has just failed.
///
/// # What this still cannot promise, said rather than hidden
///
/// A crash **between** steps 2 and 4 leaves the register ahead of the machine.
/// That is recoverable — `set listen` back to the old value, or `set pin`-style
/// hand editing of a file the operator owns — and it is not preventable without
/// a transaction spanning two machines. §6.1's residue paragraph is the same
/// admission from the other direction.
///
/// # Errors
///
/// A value `rivet-config` would refuse, a register that will not take the
/// change, or whatever the machine said — and in the last case the register has
/// been put back.
fn set_listen(
    request: &Request,
    operator: &operator::Operator,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let before = named(request, &register)?.clone();
    let listen = request
        .value("listen")
        .ok_or("no address was given, and clap should have refused this first")?
        .to_owned();

    // Refused here, in `rivet-config`'s own words, so nothing is written for a
    // value the machine would not bind (`ADR-011` — one authority for the rule).
    if !rivet_config::control_plane::ControlPlane::is_address(&listen) {
        return Err(format!(
            "`{listen}` is not an address this machine could listen on.\n\
             \x20 rule  {rule}\n\
             Nothing has been changed and nothing has been sent.",
            rule = rivet_config::control_plane::LISTEN_RULE
        ));
    }

    let mut after = before.clone();
    after.listen = Some(listen.clone());
    // **Composed before anything is written**, so a record that would produce a
    // document `rivet-config` refuses fails while both sides are untouched.
    compose::provisioning(&after, &register.fleet).map_err(|e| {
        format!("{e}\nNothing has been changed and nothing has been sent.")
    })?;

    let was = before
        .listen
        .clone()
        .unwrap_or_else(|| rivet_config::control_plane::DEFAULT_LISTEN.to_owned());
    let dialled = operator::control_plane_host(&before)?;

    if !request.confirmed {
        return Ok(format!(
            "PLAN - neither the register nor {name} has been touched.\n\
             \n\
             would record  listen = {listen}   (was {was})\n\
             would write   the composed document onto {name}, reaching it at {dialled}\n\
             would run     rivet apply reload confirm\n\
             \n\
             BOTH OR NEITHER. ADR-034 §6.1 rules that the operation changing a listener \
             writes the register and the document together, because the value being changed \
             is how this tool reaches the machine to change it. If the machine refuses, the \
             register is put back.\n\
             \n\
             THE NEW LISTENER DOES NOT TAKE EFFECT UNTIL rivet-mcpd RESTARTS. A reload \
             re-reads the document; the socket is bound at start-up (ADR-032 §3.2). Until \
             then this tool still reaches it at {dialled}, and after a restart it will use \
             the new port.\n\
             \n\
             Add `--confirm` to do it.\n",
            name = before.name,
        ));
    }

    // The transport is opened from the record AS IT WAS, which is the whole
    // subtlety: `after` would dial a port nothing is listening on yet.
    let mut remote = reach(operator, &before)?;
    write_both(&mut register, &before, &after, &dialled, &mut remote, project_root)
}

/// **The both-or-neither itself**, with the transport as a parameter.
///
/// Split out of [`set_listen`] for one reason and it is a good one: the property
/// `ADR-034` §6.1 rules — *both, or neither* — is only worth having if it is
/// **proved**, and proving it needs a machine that refuses on demand. A body
/// that opened its own connection could only be tested against a board.
///
/// See [`set_listen`] for the order and why the register is written first.
///
/// # Errors
///
/// Whatever the register or the machine refused. On a machine's refusal the
/// register has been put back, and the refusal says so; on a rollback that
/// itself failed, it says that far more loudly.
fn write_both(
    register: &mut Register,
    before: &Device,
    after: &Device,
    dialled: &str,
    remote: &mut dyn member::Member,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let listen = after
        .listen
        .clone()
        .unwrap_or_else(|| rivet_config::control_plane::DEFAULT_LISTEN.to_owned());
    let was = before
        .listen
        .clone()
        .unwrap_or_else(|| rivet_config::control_plane::DEFAULT_LISTEN.to_owned());

    register.devices.insert(after.id.clone(), after.clone());
    save(register, project_root).map_err(|e| {
        format!(
            "the register would not take the change: {e}\n\
             NEITHER side was changed - nothing has been sent to {}.",
            before.name
        )
    })?;

    let mut report = String::new();
    let seen = remote.read_document().unwrap_or_default();
    note_drift(before, seen.as_deref(), dialled, &mut report);

    match member::put_document(after, &register.fleet, remote, true, &mut report) {
        Ok(_) => Ok(format!(
            "{name} listens on {listen}   (was {was})\n\
             \x20 register  written\n\
             \x20 machine   written, at {dialled}\n\
             {report}\n\
             \x20 NOT YET IN EFFECT. The socket is bound when rivet-mcpd starts, so this \
             machine is still answering at {dialled} until it restarts. After that, this tool \
             reaches it at the new address on its own - the register says so.\n",
            name = after.name,
        )),
        // **NEITHER.** The machine would not take it, so the register is put
        // back to what it said before this command ran.
        Err(machine) => {
            register.devices.insert(before.id.clone(), before.clone());
            match save(register, project_root) {
                Ok(_) => Err(format!(
                    "{name} did not take it, so NEITHER side changed: {machine}\n\
                     \x20 register  put back to {was}\n\
                     \x20 machine   untouched\n\
                     ADR-034 6.1's both-or-neither, in the direction that had to be undone.",
                    name = before.name,
                )),
                // The one outcome that cannot be tidied, and it is shouted
                // rather than folded into the sentence above.
                Err(rollback) => Err(format!(
                    "{name} did not take it: {machine}\n\
                     \n\
                     AND THE REGISTER COULD NOT BE PUT BACK: {rollback}\n\
                     \n\
                     So the register now says `listen = {listen}` and the machine is still \
                     on {was}. This tool will dial the wrong address for this device until \
                     that is corrected - `set listen --device {name} --listen {was} \
                     --confirm`, or edit the record. THE MACHINE ITSELF WAS NOT CHANGED.",
                    name = before.name,
                )),
            }
        }
    }
}

fn set_fleet_service(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let service = service_in(request)?;
    let enabled = enabled_in(request)?;
    let memory = memory_in(request)?;
    register.fleet.services.insert(service.clone(), enabled);
    // The fleet layer carries a cap for the same reason it carries `enabled`:
    // `ADR-007` §3.4 makes it layer 3 and a device's own record still wins,
    // per service rather than per table (`crate::compose::service_memory`).
    let cap = match &memory {
        None => register.fleet.service_memory.get(&service).cloned(),
        Some(None) => {
            register.fleet.service_memory.remove(&service);
            None
        }
        Some(Some(text)) => {
            register
                .fleet
                .service_memory
                .insert(service.clone(), text.clone());
            Some(text.clone())
        }
    };
    save_fleet(&register, project_root)?;

    // **Which devices this actually changes is worth saying**, because a
    // fleet-wide value that a device overrides is one an operator will
    // otherwise expect to have taken effect. `ADR-007` §3.4's layers are only
    // useful if the operator can see which one won.
    let overridden: Vec<&str> = register
        .devices
        .values()
        .filter(|d| d.services.contains_key(&service))
        .map(|d| d.name.as_str())
        .collect();

    let mut out = format!("the fleet now says {service} = {enabled}\n");
    // Said on its own line, and only when there is one, for the reason the
    // device-level message says it: a cap an operator set yesterday and did
    // not mention today is one they should still be able to see.
    if let Some(text) = &cap {
        out.push_str(&format!("\x20 memory  = {text}\n"));
    }
    if overridden.is_empty() {
        out.push_str(&format!(
            "\x20 applies to every device ({} of them)\n",
            register.devices.len()
        ));
    } else {
        out.push_str(&format!(
            "\x20 applies to every device EXCEPT {}, which say so themselves and win per \
             service (ADR-007 3.4)\n",
            overridden.join(", ")
        ));
    }
    out.push_str(
        "\x20 next    the register says so; no machine does yet. Send each one:\n\
         \x20         rivet-fleet apply document --device <name> --confirm\n",
    );
    Ok(out)
}

/// The product an operation names, refused early if a document could not
/// carry it.
///
/// **Checked against `rivet_core::service::ServiceName`, [`service_in`]'s
/// identical rule** — `rivet_config::carry`'s header makes `[carry.<product>]`
/// and `[services.<name>]` share one vocabulary for what a product name is
/// (`ADR-011`), so this is the same check under a different flag name.
fn product_in(request: &Request) -> Result<String, String> {
    let name = request
        .value("product")
        .ok_or("no product was named, and clap should have refused this first")?;
    if rivet_core::service::ServiceName::new(name).is_none() {
        return Err(format!(
            "{name:?} is not a name a configuration document can carry, so it would be refused \
             by the machine rather than written. Nothing has been changed."
        ));
    }
    Ok(name.to_owned())
}

/// **No card carries an image service, so none can be pinned.** The system
/// image provides it (`ADR-060` put the proxy there), and a pin on it would be
/// read by no build. Clearing one is still allowed: documents and records
/// written before the proxy joined the image hold such pins, and `none` is how
/// they are removed.
fn refuse_a_pin_on_an_image_service(product: &str, version: Option<&str>) -> Result<(), String> {
    match version {
        Some(_) if is_image_service(product) => Err(format!(
            "{product} is part of the system image, so no card carries it and a pin on it would \
             be read by nothing (ADR-060). Nothing has been changed. A pin recorded before the \
             image carried it can still be cleared with --version none."
        )),
        _ => Ok(()),
    }
}

/// Whether the system image provides `product`, rather than a card carrying it.
fn is_image_service(product: &str) -> bool {
    rivet_core::service::IMAGE_SERVICE_NAMES.contains(&product)
}

/// **R-160 R26: pin which published version of a product ONE device's card
/// must carry.**
///
/// `ADR-050` §5 put the pin in the document alone; this is the register's
/// half — `crate::compose::carry` composes it in, the fleet filling a gap and
/// the device winning over it, [`set_fleet_service`]'s two-layer shape with no
/// role layer beneath it — no role knows a version.
///
/// **`.rulings/` R-162 R30: `none` clears the pin**, falling back to the
/// fleet's own pin if it has one — `set_service`'s `memory` clearing, applied
/// to `carry`'s two-layer shape.
fn set_carry(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let id = named(request, &register)?.id.clone();
    let product = product_in(request)?;
    let version = carry_version_in(request)?;
    refuse_a_pin_on_an_image_service(&product, version)?;

    let device = register
        .devices
        .get_mut(&id)
        .ok_or("the device resolved and then could not be found, which cannot happen")?;
    let previous = match version {
        None => device.carry.remove(&product),
        Some(text) => device.carry.insert(product.clone(), text.to_owned()),
    };
    let name = device.name.clone();
    let fleet_pin = register.fleet.carry.get(&product).cloned();
    save(&register, project_root)?;

    Ok(match version {
        Some(text) => format!(
            "{name} now pins {product} to {text}{}\n\
             \x20 next    the register says so; a card built from it does too. A running \
             machine is unaffected - a pin decides what a NEW card carries (ADR-050 \u{a7}5).\n",
            match &previous {
                Some(was) if was == text => " (it already said that)".to_owned(),
                Some(was) => format!(" (was {was})"),
                None => String::new(),
            },
        ),
        None => format!(
            "{name} no longer pins its own version of {product}{}\n\
             {}\
             \x20 next    the register says so; a card built from it does too.\n",
            match &previous {
                Some(was) => format!(" (was {was})"),
                None => " (it already said nothing)".to_owned(),
            },
            match fleet_pin {
                _ if is_image_service(&product) => format!(
                    "\x20 note    {product} is part of the system image, so no card carries it \
                     and nothing falls back in its place (ADR-060).\n"
                ),
                Some(text) => format!(
                    "\x20 note    the fleet still pins {product} to {text}; this device now \
                     carries THAT version instead of none at all (ADR-050 \u{a7}5).\n"
                ),
                None => format!(
                    "\x20 note    the fleet has no pin for {product} either, so a card build \
                     now refuses unless the repository holds exactly one published version.\n"
                ),
            },
        ),
    })
}

/// **R-160 R26: pin which published version of a product EVERY device
/// carries, unless its own record says otherwise.**
///
/// [`set_fleet_service`]'s shape exactly, and the naming below follows it
/// deliberately, so the two operations read the same way at the command line.
///
/// **`.rulings/` R-162 R30: `none` clears the fleet-wide pin.** A device with
/// its own override is unaffected either way; a device with none now has no
/// pin at all until one is set again.
fn set_fleet_carry(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let product = product_in(request)?;
    let version = carry_version_in(request)?;
    refuse_a_pin_on_an_image_service(&product, version)?;
    let previous = match version {
        None => register.fleet.carry.remove(&product),
        Some(text) => register.fleet.carry.insert(product.clone(), text.to_owned()),
    };
    save_fleet(&register, project_root)?;

    // **Which devices this actually changes is worth saying**, for
    // `set_fleet_service`'s reason: a fleet-wide pin a device overrides is one
    // an operator will otherwise expect to have taken effect.
    let overridden: Vec<&str> = register
        .devices
        .values()
        .filter(|d| d.carry.contains_key(&product))
        .map(|d| d.name.as_str())
        .collect();

    let mut out = match version {
        Some(text) => format!(
            "the fleet now pins {product} to {text}{}\n",
            match &previous {
                Some(was) if was == text => " (it already said that)".to_owned(),
                Some(was) => format!(" (was {was})"),
                None => String::new(),
            },
        ),
        None => format!(
            "the fleet no longer pins {product}{}\n",
            match &previous {
                Some(was) => format!(" (was {was})"),
                None => " (it already said nothing)".to_owned(),
            },
        ),
    };
    if overridden.is_empty() {
        out.push_str(&format!(
            "\x20 applies to every device ({} of them)\n",
            register.devices.len()
        ));
    } else {
        out.push_str(&format!(
            "\x20 applies to every device EXCEPT {}, which pin their own version and win \
             (ADR-050 \u{a7}5)\n",
            overridden.join(", ")
        ));
    }
    out.push_str(match version {
        Some(_) => "\x20 next    a card built from an unpinned device now resolves to this \
                    version.\n",
        None if is_image_service(&product) => {
            "\x20 next    nothing: the system image provides it and no card carries it \
             (ADR-060).\n"
        }
        None => "\x20 next    a card built from a device with no pin of its own now refuses \
                 unless the repository holds exactly one published version.\n",
    });
    Ok(out)
}

fn list_services(root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    if register.devices.is_empty() {
        return Ok("no devices are enrolled, so nothing runs anything.\n".to_owned());
    }
    let mut out = String::new();
    for device in register.devices.values() {
        let effective = compose::services(device, &register.fleet);
        if effective.is_empty() {
            out.push_str(&format!(
                "{}: nothing said. The machine runs whatever its IMAGE defaults to - which is \
                 what ADR-034 5 exists to stop being the answer.\n",
                device.name
            ));
            continue;
        }
        out.push_str(&format!("{}\n", device.name));
        for (service, enabled) in effective {
            // **Where it came from, not just what it says.** A layered value an
            // operator cannot see the source of is one they will change in the
            // wrong place.
            let source = if device.services.contains_key(&service) {
                "this device"
            } else {
                "the fleet"
            };
            out.push_str(&format!("  {service:<20} {enabled:<6} from {source}\n"));
        }
    }
    Ok(out)
}

/// **`.rulings/` R-162 R30: pins appear in a listing.** [`list_services`]'s
/// shape exactly, over [`compose::carry`] and [`compose::carry_source`]
/// instead of [`compose::services`] and its own inline source check.
fn list_carry(root: &PathBuf) -> Result<String, String> {
    let register = load(root)?;
    if register.devices.is_empty() {
        return Ok("no devices are enrolled, so nothing carries anything.\n".to_owned());
    }
    let mut out = String::new();
    for device in register.devices.values() {
        let effective = compose::carry(device, &register.fleet);
        if effective.is_empty() {
            out.push_str(&format!(
                "{}: no carried-version pin, on any product. A card build for it refuses unless \
                 a repository holds exactly one published version of whatever it carries \
                 (ADR-050 \u{a7}5).\n",
                device.name
            ));
            continue;
        }
        out.push_str(&format!("{}\n", device.name));
        for (product, version) in effective {
            let source = compose::carry_source(device, &register.fleet, &product);
            out.push_str(&format!("  {product:<20} {version:<10} from {source}\n"));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The fleet layer
// ---------------------------------------------------------------------------

fn set_fleet(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let mut changed = Vec::new();

    if let Some(name) = request.value("name") {
        register.fleet.name = Some(name.to_owned());
        changed.push(format!("name      {name}"));
    }
    if changed.is_empty() {
        // **Nothing given is a refusal, not a no-op that reports success.** An
        // operator who mistyped a flag would otherwise be told the fleet was
        // changed and find it unchanged later.
        return Err("no value was given, so nothing would change. `set fleet` takes --name; \
                    run `rivet-fleet set fleet --help` for what it does."
            .to_owned());
    }

    let path = store::save_fleet(root, &register.fleet).map_err(|e| e.to_string())?;
    project(&register, project_root);
    Ok(format!(
        "the fleet now says:\n  {}\nwritten to {}\nEvery machine gets this the next time its \
         document is composed; nothing running has changed.\n",
        changed.join("\n  "),
        path.display()
    ))
}

// ---------------------------------------------------------------------------
// ADR-034 7's one secret
// ---------------------------------------------------------------------------

/// Where the credential store is. **Beside the register's records and not among
/// them**, which is `ADR-034` §7.4: the public register stays committable, and
/// this file is the one thing here that is not.
fn credentials_path(root: &PathBuf) -> PathBuf {
    root.join(credentials::CREDENTIALS_FILE)
}

// ---------------------------------------------------------------------------
// control-plane identity — ADR-025 §2 and §3
// ---------------------------------------------------------------------------

/// Check a control-plane key and hand back its canonical form.
///
/// **Refused here rather than carried to a card.** A key that will not parse is
/// a key nobody can connect with, and finding that out means finding it out on
/// a board whose control plane refuses every caller — with `rivet-mcpd`
/// reporting, correctly and unhelpfully, that the peer is not pinned.
fn control_plane_key(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    // **Caught by shape rather than by naming a format.** A public-key file's
    // line is `<type> <base64> [comment]` and the space between those fields is
    // what makes it recognisable; a pin is 64 hex characters and has none. A
    // reader who pasted the wrong half of a key pair gets this sentence instead
    // of a base64 decoder's complaint, which is the whole value of the branch.
    if text.contains(' ') {
        return Err(format!(
            "that is a whole line of key material, and it is not what this channel takes.\n\
             \x20 wanted  64 hex characters - ADR-025 §2's control-plane identity, which \
             `cargo rivet identity new` prints\n\
             \x20 got     {}…",
            text.chars().take(20).collect::<String>()
        ));
    }
    if text.starts_with("-----BEGIN") {
        return Err("that is a PRIVATE key, and it must never reach a register that gets \
                    committed (ADR-034 §3.4). What goes here is the public half - 64 hex \
                    characters."
            .to_string());
    }
    let pins = rivetos_tls::parse_pins(text)
        .map_err(|e| format!("that is not a control-plane public key: {e}"))?;
    match pins.as_slice() {
        [one] => Ok(rivetos_tls::encode_hex(one)),
        [] => Err("no key was given".to_string()),
        many => Err(format!("{} keys were given and this takes one", many.len())),
    }
}

/// Record a key that may reach a control plane — the fleet's, or one device's.
fn add_peer(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let key = control_plane_key(
        request
            .value("key")
            .ok_or("no key was given, and clap should have refused this first")?,
    )?;

    let where_to = match request.value("device") {
        Some(_) => {
            let id = named(request, &register)?.id.clone();
            let name = register.devices[&id].name.clone();
            let keys = &mut register
                .devices
                .get_mut(&id)
                .ok_or("the device resolved and then could not be found")?
                .control_plane_keys;
            if keys.iter().any(|k| *k == key) {
                return Ok(format!("{name} already accepts that key; nothing changed\n"));
            }
            keys.push(key.clone());
            format!("{name} only")
        }
        None => {
            if register.fleet.control_plane_keys.iter().any(|k| *k == key) {
                return Ok("the fleet already accepts that key; nothing changed\n".to_string());
            }
            register.fleet.control_plane_keys.push(key.clone());
            format!("EVERY machine ({} in this register)", register.devices.len())
        }
    };
    save(&register, project_root)?;

    Ok(format!(
        "added a control-plane key, accepted by {where_to}\n\
         \x20 key     {key}\n\
         \x20 next    `add identity` composes it into a card's `peers` file. A machine \
         already running reads that file at startup, so it needs the file replaced and \
         rivet-mcpd restarted - there is no live reload of pins.\n"
    ))
}

/// Withdraw one. `ADR-025` §5: revocation is removing a key from a list.
fn remove_peer(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let key = control_plane_key(
        request
            .value("key")
            .ok_or("no key was given, and clap should have refused this first")?,
    )?;

    let (scope, removed) = match request.value("device") {
        Some(_) => {
            let id = named(request, &register)?.id.clone();
            let name = register.devices[&id].name.clone();
            let keys = &mut register
                .devices
                .get_mut(&id)
                .ok_or("the device resolved and then could not be found")?
                .control_plane_keys;
            let before = keys.len();
            keys.retain(|k| *k != key);
            (name, before != keys.len())
        }
        None => {
            let before = register.fleet.control_plane_keys.len();
            register.fleet.control_plane_keys.retain(|k| *k != key);
            ("the fleet".to_string(), before != register.fleet.control_plane_keys.len())
        }
    };
    if !removed {
        return Err(format!("{scope} does not hold that key, so nothing was removed"));
    }
    save(&register, project_root)?;

    Ok(format!(
        "removed a control-plane key from {scope}\n\
         \x20 key     {key}\n\
         \x20 WARNING a machine already running still holds the OLD `peers` file and still \
         accepts this key. Revocation takes effect when the file is replaced and rivet-mcpd \
         restarts - `add identity` composes the new one.\n"
    ))
}

/// Mint a device's TLS identity — `ADR-025` §3.
///
/// # The sequence `ADR-034` §3.5 requires, in the order it requires it
///
/// > ```text
/// > the tool GENERATES a device key   -> the private half goes into the provisioning document
/// > the tool RETAINS the public half  -> the register records the public half and the key id
/// > the private half exists           -> in memory, then on the card, then blanked at first
/// >                                      boot by ADR-019 section 3.1
/// > ```
/// >
/// > **The register is never written with the private half at any point in that
/// > sequence.** An implementation that writes it "temporarily" and removes it
/// > later has failed this decision, because a deleted file is not an absent
/// > one.
///
/// So: the seed is written to the operator's `--out` directory and the register
/// is saved with the **public** half. The two writes are to different places and
/// the register save cannot carry the seed, because the seed is never put on a
/// value the register serialises.
///
/// # `--confirm` is honoured, and this is the worst flag in the toolset to get
/// wrong
///
/// This operation is `confirms: true`, and it once did not read
/// `request.confirmed`: the documented dry run **minted the seed and wrote
/// `device.seed`, `device.crt` and `peers`**, and the real run afterwards was
/// then refused, because `cargo rivet identity new` correctly declines to
/// overwrite a key that exists exactly once. **The careful habit — look first,
/// then commit — destroyed the artifact.** `SPEC-05` §9.
///
/// So the plan path is computed from the register alone and touches nothing.
fn add_identity(
    request: &Request,
    root: &PathBuf,
    project_root: Option<&Path>,
) -> Result<String, String> {
    let mut register = load(root)?;
    let device = named(request, &register)?.clone();
    let out = PathBuf::from(
        request
            .value("out")
            .ok_or("no output directory was given, and clap should have refused this first")?,
    );

    if !request.confirmed {
        // **Every file named, and whether it is already there.** A plan that
        // says "would write device.seed" while one exists is the plan for an
        // operation that will be refused, and saying so here costs a line and
        // saves finding out after the mint.
        let accepted = compose::control_plane_keys(&device, &register.fleet).len();
        let mut files = String::new();
        for name in ["device.seed", "device.crt", "peers"] {
            let path = out.join(name);
            let note = if path.exists() { "   EXISTS ALREADY" } else { "" };
            files.push_str(&format!("\x20 would write   {}{note}\n", path.display()));
        }
        let holds = device.pin.as_ref().map_or_else(
            || "the register records the public half; this device holds none yet".to_owned(),
            |had| format!("the register would REPLACE the public half it holds ({had}), so every peer pinning the old one must be updated"),
        );
        return Ok(format!(
            "PLAN - nothing has been minted and nothing has been written.\n\
             \n\
             \x20 device        {name}  ({id})\n\
             {files}\
             \x20 would record  {holds}\n\
             \x20 peers         {accepted} key(s) would be allowed to reach this machine\n\
             \n\
             A CONTROL-PLANE IDENTITY EXISTS ONCE. `cargo rivet identity new` refuses to \
             overwrite one, so a directory that already holds device.seed makes the \
             confirmed run a refusal rather than a re-mint - choose an empty --out, or \
             move what is there.\n\
             \n\
             Add `--confirm` to do it.\n",
            name = device.name,
            id = device.id,
        ));
    }

    // **Refused before anything is generated.** `cargo rivet identity new`
    // refuses to overwrite for the reason that matters here too: replacing an
    // identity silently locks out every peer that pinned the old one.
    let minted = cargo_rivet_identity(&out, &device.name)?;

    let peers = compose::peers(&device, &register.fleet);
    std::fs::write(out.join("peers"), &peers)
        .map_err(|e| format!("cannot write {}: {e}", out.join("peers").display()))?;

    let previous = register
        .devices
        .get_mut(&device.id)
        .ok_or("the device resolved and then could not be found")?
        .pin
        .replace(minted.clone());
    save(&register, project_root)?;

    let accepted = compose::control_plane_keys(&device, &register.fleet).len();
    Ok(format!(
        "minted a control-plane identity for {name}\n\
         \x20 public  {minted}\n\
         \x20 files   {dir} - device.seed, device.crt, peers\n\
         \x20 peers   {accepted} key(s) may reach this machine\n\
         \x20 record  the register holds the PUBLIC half only (ADR-034 §3.5){replaced}\n\
         \n\
         \x20 next    RIVET_IDENTITY={dir} tools/card/build.sh\n\
         \n\
         \x20 THE SEED IN THAT DIRECTORY IS A PRIVATE KEY. It belongs on a card and \
         nowhere else: not in the register, not in git, not in a backup. The board erases \
         it from the card on first boot (ADR-019 §3.1) and it exists nowhere afterwards, \
         so a card written from it is the only copy.\n",
        name = device.name,
        dir = out.display(),
        replaced = if previous.is_some() {
            " - and REPLACED the key it held, so every peer pinning the old one must be updated"
        } else {
            ""
        },
    ))
}

/// Run `cargo rivet identity new` and return the public key it printed.
///
/// **Shelled out rather than reimplemented**, which is `ADR-034` §11.5's
/// argument applied to this: `cmd/cargo-rivet` already refuses to
/// overwrite an identity, already sets `0600` in the open, and already knows
/// the file layout `rivetos-tls` reads. A second minting path would be a second
/// place all three could be got wrong (`ADR-011`).
fn cargo_rivet_identity(out: &std::path::Path, name: &str) -> Result<String, String> {
    let output = std::process::Command::new("cargo")
        .args(["rivet", "identity", "new", "--directory"])
        .arg(out)
        .args(["--name", name])
        .output()
        .map_err(|e| {
            format!(
                "cannot run `cargo rivet identity new`: {e}\n\
                 \x20 note  this tool mints through cmd/cargo-rivet rather than reimplementing \
                 it, so cargo-rivet must be installed - `cargo install --path cmd/cargo-rivet \
                 --force --locked`."
            )
        })?;
    let said = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        return Err(format!(
            "`cargo rivet identity new` refused:\n{}{}",
            said,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    said.lines()
        .map(str::trim)
        .find(|l| l.len() == 64 && l.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("`cargo rivet identity new` printed no public key:\n{said}")
        })
}

fn load_credentials(root: &PathBuf) -> Result<credentials::Store, String> {
    credentials::load(&credentials_path(root)).map_err(|e| e.to_string())
}

fn save_credentials(root: &PathBuf, store: &credentials::Store) -> Result<PathBuf, String> {
    let path = credentials_path(root);
    credentials::save(&path, store, &salt()).map_err(|e| e.to_string())?;
    Ok(path)
}

fn list_credentials(root: &PathBuf) -> Result<String, String> {
    let store = load_credentials(root)?;
    if store.is_empty() {
        return Ok("no wireless credentials are held.\nRecord one: rivet-fleet add credential \
                   --network <ssid> --key <64 hex characters>\n"
            .to_owned());
    }
    let mut out = String::new();
    for network in store.networks() {
        // **Networks, never keys.** A store that could be printed is one that
        // would end up in a terminal's scrollback, which is `ADR-020` §3's
        // "secrets leak into logs and errors" arriving by the front door.
        out.push_str(&format!("{network}\n"));
    }
    out.push_str(&format!(
        "\n{} network(s). The keys are not printed and there is no flag that prints them.\n",
        store.networks().len()
    ));
    Ok(out)
}

fn add_credential(request: &Request, root: &PathBuf) -> Result<String, String> {
    let network = request
        .value("network")
        .ok_or("no network was named, and clap should have refused this first")?;

    // **The natural thing an operator types reaches the honest refusal.**
    // `--passphrase` exists so that somebody who has a passphrase and not a key
    // is told why this build will not take it, rather than finding no flag and
    // concluding the tool is unfinished. `AGENTS.md` §3: a stub that refuses is
    // worth more than a feature nobody can find the edge of.
    if let Some(passphrase) = request.value("passphrase") {
        return Err(credentials::derive_pmk(network, passphrase)
            .err()
            .map_or_else(|| "unreachable".to_owned(), |e| e.to_string()));
    }

    let key = request
        .value("key")
        .ok_or("give either --key (64 hexadecimal characters) or --passphrase.")?;
    let pmk = credentials::pmk_from_hex(key).map_err(|e| e.to_string())?;

    let mut store = load_credentials(root)?;
    store.put(credentials::Credential {
        ssid: network.to_owned(),
        pmk,
    });
    let path = save_credentials(root, &store)?;
    Ok(format!(
        "recorded a key for {network}\n\
         \x20 store  {}\n\
         \x20 note   THAT FILE IS OBFUSCATED AND NOT ENCRYPTED (ADR-034 10.4). Anyone with it \
         and rivet-fleet's source can read it. Do not commit it, and do not copy it anywhere \
         you would not copy the key itself.\n",
        path.display()
    ))
}

fn remove_credential(request: &Request, root: &PathBuf) -> Result<String, String> {
    let network = request
        .value("network")
        .ok_or("no network was named, and clap should have refused this first")?;
    let mut store = load_credentials(root)?;
    if store.get(network).is_none() {
        return Err(format!("no key is held for {network:?}."));
    }
    if !request.confirmed {
        return Ok(format!(
            "PLAN - nothing has been changed.\n\
             \n\
             would forget  the key for {network}\n\
             \n\
             Machines already provisioned with it keep working: they hold their own copy, which \
             is ADR-034 7.3's second question and the reason this key may be here at all.\n\
             \n\
             Add `--confirm` to do it.\n"
        ));
    }
    store.forget(network);
    let path = save_credentials(root, &store)?;
    Ok(format!(
        "forgot the key for {network}\n  store  {}\n",
        path.display()
    ))
}

/// Thirty-two bytes for the credential store's salt.
///
/// The same source, and the same honest caveat, as [`mint_identifier`] — and
/// here it matters even less: the salt exists so that two stores with one
/// content do not have one file, which is a property about *recognisability*
/// and not about secrecy. Nothing in `crate::credentials` is a security
/// boundary and that module says so at length.
fn salt() -> [u8; 32] {
    use rivet_core::package::port::Digest as _;
    let mut hash = rivet_crypto::Blake3::new();
    hash.start();
    hash.write(b"rivet-fleet credential store salt");
    hash.write(&std::process::id().to_le_bytes());
    if let Ok(since) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hash.write(&since.as_nanos().to_le_bytes());
    }
    hash.finish()
}

/// Sixteen hexadecimal characters of entropy for a new device.
///
/// **The randomness is the operating system's**, taken through `getrandom` on
/// the platform Ignition already brought in — no, it is not: this workspace has
/// no random-number dependency and `rivet-crypto` exposes none. What is used
/// instead is stated rather than hidden: the process id, the address of a heap
/// allocation, and the system clock, hashed with BLAKE3.
///
/// **That is adequate here and would not be adequate for a key.** `ADR-025`
/// §4.1 asks the identifier to be *stable* and *opaque* — it is neither secret
/// nor unguessable, and nothing anywhere treats knowing one as authority. What
/// it must not do is collide, and 64 bits from three independent sources does
/// not, on a fleet of tens.
///
/// If this ever needs to be unguessable, it needs a real source and that is a
/// dependency decision (`AGENTS.md` §1), not a change to this function.
fn mint_identifier() -> String {
    use rivet_core::package::port::Digest as _;

    let mut hash = rivet_crypto::Blake3::new();
    hash.start();
    hash.write(&std::process::id().to_le_bytes());
    let heap = Box::new(0u8);
    hash.write(&(std::ptr::from_ref::<u8>(&*heap) as usize).to_le_bytes());
    if let Ok(since) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hash.write(&since.as_nanos().to_le_bytes());
    }
    register::identifier_from(&hash.finish())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::member::guard;

    // -----------------------------------------------------------------------
    // ADR-034 §6.1 — both, or neither
    // -----------------------------------------------------------------------

    /// A real directory under the OS temporary directory, removed at the end.
    ///
    /// `crate::store`'s own, for its reason: the workspace has no `tempfile`
    /// and taking one to write three tests would be a dependency decision
    /// (`ADR-009` §4) rather than a test.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("rivet-fleet-listen-{label}-{n}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A device the composer will accept: one address.
    fn a_device() -> Device {
        let mut device = Device {
            id: "0011223344556677".to_owned(),
            name: "kitchen-pi".to_owned(),
            // None of this fixture's callers are about SPEC-05 5's
            // completeness gates - `write_both` reaches `put_document`,
            // which composes through `provisioning_complete` because a live
            // write is a document leaving the register for good.
            timezone: Some("UTC".to_owned()),
            ..Device::default()
        };
        device.interfaces.insert(
            "eth0".to_owned(),
            register::Interface {
                method: "static".to_owned(),
                address: Some("192.168.1.50/24".to_owned()),
                gateway: Some("192.168.1.1".to_owned()),
            },
        );
        device
    }

    /// A machine that either takes a document or refuses one.
    struct Machine {
        takes_it: bool,
        held: std::cell::RefCell<Option<String>>,
    }

    impl rivet_remote::deploy::Remote for Machine {
        fn push(&mut self, _l: &std::path::Path, _r: &str) -> Result<String, String> {
            unreachable!("set listen pushes no payload")
        }
        fn blake3(&mut self, _r: &str) -> Result<[u8; 32], String> {
            unreachable!("set listen hashes nothing")
        }
        fn install(&mut self, _r: &str) -> Result<String, String> {
            unreachable!("set listen installs nothing")
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            Ok(self.held.borrow().clone())
        }
        fn edit_document(
            &mut self,
            _c: &rivet_remote::deploy::ConfigChange,
        ) -> Result<String, String> {
            unreachable!("set listen replaces rather than edits")
        }
        fn put_document(&mut self, text: &str) -> Result<(), String> {
            if !self.takes_it {
                return Err("the machine would not take it".to_owned());
            }
            *self.held.borrow_mut() = Some(text.to_owned());
            Ok(())
        }
        fn ask(&mut self, _w: &[&str]) -> Result<String, String> {
            unreachable!("nothing speaks a word list at a transport any more")
        }
    }

    impl member::Member for Machine {
        fn perform(&self, _w: &[&str]) -> Result<String, String> {
            Ok(String::new())
        }
    }

    /// Set up a register on disk holding one device, and give back what
    /// [`write_both`] needs.
    fn one_device_register(scratch: &Scratch, listen: Option<&str>) -> (Register, Device) {
        let mut register = Register::empty(&scratch.0);
        let mut device = a_device();
        device.listen = listen.map(str::to_owned);
        register.enroll(device.clone()).expect("a fresh register");
        store::save(&register).expect("a writable register");
        (register, device)
    }

    /// What the register on disk says this device listens on, read back from
    /// the files rather than from memory — which is the only reading that
    /// proves anything about *both, or neither*.
    fn listen_on_disk(root: &std::path::Path, id: &str) -> Option<String> {
        store::load(root)
            .expect("a readable register")
            .devices
            .get(id)
            .and_then(|d| d.listen.clone())
    }

    // -----------------------------------------------------------------------
    // `ADR-052` §9 step 2 — the projection, now that it takes a root instead
    // of a `cfg` gate. `rivet-fleet-repo`'s own test suite proves
    // `record_register` itself: renders in `rivetos_register`'s own layout,
    // reads back, commits nothing twice, and never writes to the register's
    // own root (`projecting_the_register_never_writes_to_its_own_root`). What
    // was never proved is that THIS crate's `save`/`save_fleet`/`project`
    // hand it the root a caller actually gave them — the exact thing
    // `#[cfg(test)]`-ing `project` into a no-op made unprovable.
    // -----------------------------------------------------------------------

    /// The commit subjects in a repository's log, oldest first — this crate's
    /// own copy of `rivet-fleet-repo`'s `tests::log`, for the same reason
    /// every other test helper here is copied rather than shared: nowhere for
    /// a `#[cfg(test)]` helper to live without a `dev-dependency` on a crate
    /// this one already depends on for real.
    fn project_log(root: &std::path::Path) -> String {
        std::process::Command::new("git")
            .args(["log", "--format=%s"])
            .current_dir(root)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// **Saving a register projects it into the root it was given.** `save`
    /// takes `project_root` as a parameter now instead of resolving
    /// `HOME`/`USERPROFILE` for itself, so a temporary directory here is a
    /// real projection target rather than something `#[cfg(test)]` compiled
    /// away — the read-back proves the parameter actually reaches
    /// `rivetos_fleet_repo::record_register`.
    #[test]
    fn saving_a_register_projects_it_into_the_root_it_was_given() {
        let scratch = Scratch::new("project-save");
        let project = Scratch::new("project-save-target");
        let (register, device) = one_device_register(&scratch, None);

        save(&register, Some(&project.0)).expect("a writable register");

        let projected = project.0.join(rivetos_fleet_repo::REGISTER_DIR);
        assert!(
            projected.join(register::REGISTER_FILE).is_file(),
            "the register was not projected into the root it was given"
        );
        let read_back = store::load(&projected).expect("the projection reads back");
        assert!(read_back.devices.contains_key(&device.id), "{:?}", read_back.devices);
    }

    /// **No empty commit, at this layer too.** `rivet-fleet-repo`'s own
    /// `projecting_the_same_register_twice_makes_no_second_commit` proves
    /// `record_register` is idempotent; this proves `save` calls it against
    /// the SAME root on a second, unchanged save, rather than losing track of
    /// `project_root` between calls.
    #[test]
    fn saving_the_same_register_twice_makes_no_second_projection_commit() {
        let scratch = Scratch::new("project-repeat");
        let project = Scratch::new("project-repeat-target");
        let (register, _device) = one_device_register(&scratch, None);

        save(&register, Some(&project.0)).expect("first save");
        let before = project_log(&project.0);
        save(&register, Some(&project.0)).expect("second save, unchanged");
        assert_eq!(
            project_log(&project.0),
            before,
            "an unchanged second save must not add a commit"
        );
    }

    /// **The projection lands only in the root it was handed.**
    /// `project_root` is a parameter precisely so the register's own root and
    /// the fleet repository it projects into cannot be confused with one
    /// another — this uses two distinct temporary directories and checks the
    /// projection's `.git` exists only in the one named as `project_root`,
    /// never in the register's own root that [`Scratch`] set up.
    #[test]
    fn the_projection_never_writes_outside_the_root_it_was_given() {
        let scratch = Scratch::new("project-confusion-register");
        let project = Scratch::new("project-confusion-target");
        let (register, _device) = one_device_register(&scratch, None);

        save(&register, Some(&project.0)).expect("a writable register");

        assert!(project.0.join(".git").is_dir(), "the named root was never written to");
        assert!(
            !scratch.0.join(".git").is_dir(),
            "the projection wrote into the register's own root instead of the root it was \
             handed"
        );
    }

    /// **`None` is the honest default, and it must stay silent rather than
    /// fall back to a real machine's `HOME`.** A caller that passes no
    /// `project_root` — every test above it in this file, and every run on a
    /// workstation with neither `HOME` nor `USERPROFILE` set — gets an
    /// ordinary save with nothing projected anywhere, not a projection this
    /// function quietly resolved for itself.
    #[test]
    fn saving_with_no_project_root_projects_nothing() {
        let scratch = Scratch::new("project-none");
        let (register, _device) = one_device_register(&scratch, None);
        save(&register, None).expect("a writable register");
        assert!(
            !scratch.0.join(".git").is_dir(),
            "a `None` project root must not turn into a git repository anywhere"
        );
    }

    /// **BOTH.** `ADR-034` §6.1: the operation that changes the listener writes
    /// the register and the document together. This is the succeeding half —
    /// the register holds the new value on disk, and the document the machine
    /// received carries the same one.
    #[test]
    fn a_listener_change_the_machine_accepts_lands_on_both_sides() {
        let scratch = Scratch::new("both");
        let (mut register, before) = one_device_register(&scratch, None);
        let mut after = before.clone();
        after.listen = Some("0.0.0.0:7778".to_owned());
        let mut machine = Machine { takes_it: true, held: std::cell::RefCell::new(None) };

        let said =
            write_both(&mut register, &before, &after, "192.168.1.50:7777", &mut machine, None)
                .expect("the machine took it");

        // The register, read back off the disk.
        assert_eq!(
            listen_on_disk(&scratch.0, &before.id).as_deref(),
            Some("0.0.0.0:7778"),
            "the register did not record it"
        );
        // And the document the machine was actually given.
        let document = machine.held.borrow().clone().expect("a document was written");
        assert!(document.contains("[control-plane]"), "{document}");
        assert!(document.contains("0.0.0.0:7778"), "{document}");
        // The report says the new listener is not yet in effect, which is the
        // fact an operator is most likely to assume away.
        assert!(said.contains("NOT YET IN EFFECT"), "{said}");
    }

    /// **NEITHER.** The machine refuses, so the register is put back — and *put
    /// back* is checked on the disk, not in memory. A build that wrote the file
    /// and only rolled back the in-memory copy would pass a weaker test and
    /// leave a register that cannot reach its own machine.
    #[test]
    fn a_listener_change_the_machine_refuses_leaves_the_register_as_it_was() {
        let scratch = Scratch::new("neither");
        let (mut register, before) = one_device_register(&scratch, Some("0.0.0.0:7777"));
        let mut after = before.clone();
        after.listen = Some("0.0.0.0:7778".to_owned());
        let mut machine = Machine { takes_it: false, held: std::cell::RefCell::new(None) };

        let refused = write_both(
            &mut register,
            &before,
            &after,
            "192.168.1.50:7777",
            &mut machine,
            None,
        )
        .expect_err("the machine refused");

        assert_eq!(
            listen_on_disk(&scratch.0, &before.id).as_deref(),
            Some("0.0.0.0:7777"),
            "the register kept a value the machine never took"
        );
        assert!(machine.held.borrow().is_none(), "the machine held a document");
        assert!(refused.contains("NEITHER side changed"), "{refused}");
        assert!(refused.contains("put back"), "{refused}");
    }

    /// **A record with no `listen` rolls back to having no `listen`**, rather
    /// than to the default written out. The two are the same state
    /// (`Device::listen`), and a rollback that wrote one where there had been
    /// none would change the composed document's bytes for a command that
    /// failed.
    #[test]
    fn a_rollback_restores_an_absent_field_as_absent() {
        let scratch = Scratch::new("absent");
        let (mut register, before) = one_device_register(&scratch, None);
        let mut after = before.clone();
        after.listen = Some("0.0.0.0:7778".to_owned());
        let mut machine = Machine { takes_it: false, held: std::cell::RefCell::new(None) };

        let _ =
            write_both(&mut register, &before, &after, "192.168.1.50:7777", &mut machine, None);

        assert_eq!(listen_on_disk(&scratch.0, &before.id), None);
        // And in memory too, so a caller reading the register it passed in sees
        // the same thing the disk does.
        assert_eq!(register.devices[&before.id].listen, None);
    }

    /// **`ADR-034` §6.1's DETECTION**: a machine whose own document names a
    /// different listener is reported, and the report names all three values —
    /// including the one the tool reached it on, which is the only one either
    /// of the other two can be checked against.
    #[test]
    fn a_machine_whose_document_disagrees_is_reported_with_the_address_that_worked() {
        let scratch = Scratch::new("drift");
        let (mut register, before) = one_device_register(&scratch, Some("0.0.0.0:7777"));
        let mut after = before.clone();
        after.listen = Some("0.0.0.0:7779".to_owned());

        // The machine already holds a document saying something else entirely.
        let theirs = compose::provisioning_toml(
            &Device { listen: Some("0.0.0.0:7778".to_owned()), ..before.clone() },
            &register.fleet,
        )
        .expect("composable");
        let mut machine = Machine {
            takes_it: true,
            held: std::cell::RefCell::new(Some(theirs)),
        };

        let said = write_both(
            &mut register,
            &before,
            &after,
            "192.168.1.50:7777",
            &mut machine,
            None,
        )
        .expect("the machine took it");

        assert!(said.contains("DRIFT"), "{said}");
        assert!(said.contains("0.0.0.0:7777"), "the register's value is missing:\n{said}");
        assert!(said.contains("0.0.0.0:7778"), "the machine's value is missing:\n{said}");
        assert!(
            said.contains("192.168.1.50:7777"),
            "the address it was REACHED on is missing:\n{said}"
        );
    }

    /// Two devices enrolled in one process get two identifiers. A minting
    /// function that returned the same string twice would file the second
    /// device over the first.
    #[test]
    fn two_identifiers_minted_in_a_row_are_different() {
        let a = mint_identifier();
        let b = mint_identifier();
        assert_ne!(a, b);
        assert!(register::is_identifier(&a), "{a}");
        assert!(register::is_identifier(&b), "{b}");
    }

    /// **The catalogue gate, asserted.** Every operation the table carries has
    /// an implementation, or `perform` says so — and a row added without one
    /// reaches the honest arm rather than a silent success.
    ///
    /// This is the defect `cmd/rivet` shipped on 2026-08-26 in its other
    /// direction: there, dispatch had an arm the catalogue did not. Here the
    /// catalogue is the only way in, so what has to be checked is the reverse.
    #[test]
    fn an_operation_with_no_implementation_says_so_rather_than_succeeding() {
        let unimplemented = operation::Operation {
            verb: Verb::Apply,
            subject: "nothing-yet",
            summary: "A row with no implementation behind it.",
            inputs: &[],
            confirms: false,
        };
        let request = Request {
            // Leaked so the `&'static` in `Request` is honest; this is a test
            // and the allocation lives as long as the process.
            operation: Box::leak(Box::new(unimplemented)),
            inputs: Vec::new(),
            confirmed: false,
        };
        let error = perform(
            &request,
            &operator::Operator::default(),
            &PathBuf::from("/nowhere"),
            None,
        )
        .expect_err("no implementation");
        assert!(error.contains("no implementation"), "{error}");
        assert!(error.contains("nothing has been changed"), "{error}");
    }

    // -----------------------------------------------------------------------
    // ADR-034 §6.2 — the guard, and the adoption that clears it
    // -----------------------------------------------------------------------

    /// The document the live board held on 2026-08-29, in shape: a
    /// `[control-plane]` and a `[services.rivet-mcpd]` the register knew
    /// nothing about.
    fn a_board_document(device: &Device, fleet: &register::Fleet) -> String {
        let mut running = device.clone();
        running.listen = Some("0.0.0.0:7777".to_owned());
        running.services.insert("rivet-mcpd".to_owned(), true);
        compose::provisioning_toml(&running, fleet).expect("composable")
    }

    /// **The measured case, through the code path `apply document` runs.**
    /// `guard` is what stands between the composed document and the write, and
    /// this is the state that would have cut the only Rivet in existence off
    /// the network.
    #[test]
    fn the_guard_refuses_the_write_that_would_have_cut_the_board_off() {
        let device = a_device();
        let fleet = register::Fleet::default();
        let held = a_board_document(&device, &fleet);
        let mut report = String::new();

        let refusal = guard(&device, &fleet, Some(&held), &mut report)
            .expect_err("both values would be removed");

        assert!(refusal.contains("control-plane.listen"), "{refusal}");
        assert!(refusal.contains("services.rivet-mcpd.enabled"), "{refusal}");
        assert!(refusal.contains("set listen --device kitchen-pi"), "{refusal}");
        assert!(refusal.contains("set service --device kitchen-pi"), "{refusal}");
        assert!(refusal.contains("NOTHING HAS BEEN WRITTEN"), "{refusal}");
    }

    /// **And the ordinary path is not made harder.** A register that accounts
    /// for what the machine holds passes, and says it checked — `ADR-034` §6.2
    /// is explicit that a refusal firing on every ordinary run is the failure
    /// mode, not the goal.
    #[test]
    fn the_guard_passes_a_register_that_accounts_for_everything() {
        let mut device = a_device();
        device.listen = Some("0.0.0.0:7777".to_owned());
        device.services.insert("rivet-mcpd".to_owned(), true);
        let fleet = register::Fleet::default();
        let held = compose::provisioning_toml(&device, &fleet).expect("composable");
        let mut report = String::new();

        guard(&device, &fleet, Some(&held), &mut report).expect("nothing is lost");
        assert!(report.contains("nothing this machine holds would be REMOVED"), "{report}");
    }

    /// A board with no document yet has nothing to lose, and the first apply
    /// onto a fresh card must not be blocked by a guard about erasure.
    #[test]
    fn the_guard_passes_a_machine_that_holds_no_document_at_all() {
        let mut report = String::new();
        guard(&a_device(), &register::Fleet::default(), None, &mut report)
            .expect("nothing to remove");
        assert!(report.contains("holds no document yet"), "{report}");
    }

    /// **A document this tool did not write.** `PLAN.md` §102's finding says
    /// the live board's `[control-plane]` was *"hand-written at some point,
    /// never re-composed"* — so the document the guard will meet in anger is
    /// **not** canonical, and a test that composes both sides proves the
    /// comparison agrees with itself and nothing about the format
    /// (`AGENTS.md` §4).
    ///
    /// This one is written the way a person writes one: tables in another
    /// order, a comment, a blank line in the middle of a table, and an inline
    /// table for the interface. The two losses are found all the same.
    #[test]
    fn a_hand_written_document_is_compared_by_meaning_and_not_by_layout() {
        let device = a_device();
        let fleet = register::Fleet::default();
        let by_hand = "\
# somebody edited this at the board's own prompt
version = 1
hostname = \"kitchen-pi\"

[control-plane]
listen   =   \"0.0.0.0:7777\"


[network]
eth0 = { method = \"static\", address = \"192.168.1.50/24\", gateway = \"192.168.1.1\" }

[services.rivet-mcpd]

enabled = true
";
        let (found, note) = compare(&device, &fleet, by_hand).expect("comparable");
        assert_eq!(note, None, "rivet-config takes this document");
        let named: Vec<String> = found.losses.iter().map(diff::Loss::where_it_is).collect();
        assert_eq!(
            named,
            vec!["control-plane.listen".to_owned(), "services.rivet-mcpd.enabled".to_owned()],
            "layout must not change what is found"
        );
        // And nothing about the LAYOUT is reported as a change.
        assert_eq!(found.changes, Vec::new(), "{:?}", found.changes);
    }

    /// A machine whose reads answer from a script, in order, and that keeps
    /// whatever it is sent. Once the script runs out it answers with what it
    /// holds.
    struct Scripted {
        reads: std::collections::VecDeque<Result<Option<String>, String>>,
        held: Option<String>,
        written: bool,
    }

    impl rivet_remote::deploy::Remote for Scripted {
        fn push(&mut self, _l: &std::path::Path, _r: &str) -> Result<String, String> {
            unreachable!("apply document pushes no payload")
        }
        fn blake3(&mut self, _r: &str) -> Result<[u8; 32], String> {
            unreachable!("apply document hashes nothing")
        }
        fn install(&mut self, _r: &str) -> Result<String, String> {
            unreachable!("apply document installs nothing")
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            self.reads.pop_front().unwrap_or_else(|| Ok(self.held.clone()))
        }
        fn edit_document(
            &mut self,
            _c: &rivet_remote::deploy::ConfigChange,
        ) -> Result<String, String> {
            unreachable!("apply document replaces rather than edits")
        }
        fn put_document(&mut self, text: &str) -> Result<(), String> {
            self.held = Some(text.to_owned());
            self.written = true;
            Ok(())
        }
        fn ask(&mut self, _w: &[&str]) -> Result<String, String> {
            unreachable!("nothing speaks a word list at a transport any more")
        }
    }

    impl member::Member for Scripted {
        fn perform(&self, _w: &[&str]) -> Result<String, String> {
            Ok(String::new())
        }
    }

    /// **A read that failed is not a machine with nothing to lose** — the
    /// external audit's F48. The first read fails, the next would succeed
    /// and shows what the register lacks, and the write must still be
    /// refused: no reading of the machine's document, no full write over it.
    #[test]
    fn a_failed_first_read_does_not_let_an_apply_remove_anything() {
        let device = a_device();
        let fleet = register::Fleet::default();
        let held = a_board_document(&device, &fleet);
        let mut machine = Scripted {
            reads: [Err("the connection dropped".to_owned()), Ok(Some(held.clone()))].into(),
            held: Some(held.clone()),
            written: false,
        };
        let mut report = String::new();

        let refusal = apply_onto(&device, &fleet, &mut machine, "kitchen-pi:7777", &mut report)
            .expect_err("nothing was read, so nothing may be overwritten");
        assert!(!machine.written, "the document was replaced: {report}{refusal}");
        assert_eq!(machine.held.as_deref(), Some(held.as_str()));
    }

    /// **The two halves, tied together.** Adopt every loss the guard found,
    /// put the record back, and the guard passes — which is the whole promise
    /// of `add document`, and the one thing neither module can prove alone.
    #[test]
    fn adopting_what_the_guard_found_makes_the_guard_pass() {
        let mut device = a_device();
        let fleet = register::Fleet::default();
        let held = a_board_document(&device, &fleet);

        let (found, note) = compare(&device, &fleet, &held).expect("comparable");
        assert_eq!(note, None, "a composed document parses");
        assert_eq!(found.losses.len(), 2, "{:?}", found.losses);

        for loss in &found.losses {
            adopt(&mut device, loss).unwrap_or_else(|e| panic!("{}: {e}", loss.where_it_is()));
        }
        assert_eq!(device.listen.as_deref(), Some("0.0.0.0:7777"));
        assert_eq!(device.services.get("rivet-mcpd"), Some(&true));

        let mut report = String::new();
        guard(&device, &fleet, Some(&held), &mut report).expect("the refusal is cleared");
    }

    /// **What adoption refuses, and why it refuses rather than doing it.** A
    /// volume has no field at all, and a hostname is derived rather than
    /// adopted; a tool that took either would have let one machine's state
    /// decide something it has no standing to decide (`ADR-034` §2 — the
    /// register is the authority).
    #[test]
    fn adoption_refuses_a_field_the_register_does_not_have() {
        let mut device = a_device();
        let cases = [
            (vec!["volumes", "data", "guid"], "0000", "NO field for volumes"),
            (vec!["hostname"], "elsewhere", "renaming the record"),
        ];
        for (path, value, expected) in cases {
            let loss = diff::Loss {
                path: path.iter().map(|s| (*s).to_owned()).collect(),
                value: value.to_owned(),
                whole: true,
            };
            match adopt(&mut device, &loss) {
                Ok(into) => panic!("{path:?} should not be adopted, and it went into {into}"),
                Err(why) => assert!(why.contains(expected), "{path:?}: {why}"),
            }
        }
        // And the record is untouched by any of them.
        assert_eq!(device.name, "kitchen-pi");
        assert!(device.services.is_empty(), "{:?}", device.services);
    }

    /// **The regression test for `ADR-034` §6's defect.** A `[network.resolver]
    /// nameservers = [...]` on the machine reaches `add document` as one
    /// `Loss` per nameserver (`rivet_core::leaf::between`'s `Leaf::Many`
    /// handling), so this calls `adopt` twice - exactly as `add_document`'s
    /// loop does - and checks the result lands in `Device::resolver`, never
    /// in `Device::interfaces`. Before the dedicated match arm existed, both
    /// calls fell into the generic `["network", interface, field]` arm,
    /// which inserted `interfaces["resolver"] = { method: "dhcp", .. }` as a
    /// SIDE EFFECT before discovering `nameservers` had nowhere to go — and
    /// that insertion survived the `Err` it returned, which is how the
    /// maintainer's real register ended up with `[interfaces.resolver]
    /// method = "dhcp"` and no nameserver at all.
    #[test]
    fn adopting_a_resolvers_nameservers_lands_in_the_resolver_field_not_an_interface() {
        let mut device = a_device();
        for value in ["192.168.50.1", "1.1.1.1"] {
            let loss = diff::Loss {
                path: vec!["network".to_owned(), "resolver".to_owned(), "nameservers".to_owned()],
                value: value.to_owned(),
                whole: false,
            };
            adopt(&mut device, &loss).unwrap_or_else(|e| panic!("{value}: {e}"));
        }
        assert!(
            !device.interfaces.contains_key("resolver"),
            "the defect this test guards: {:?}",
            device.interfaces
        );
        let resolver = device.resolver.as_ref().expect("adopted");
        assert_eq!(resolver.nameservers, ["192.168.50.1", "1.1.1.1"]);
        assert!(resolver.search.is_empty(), "{:?}", resolver.search);
    }

    /// The same arm, for `search` rather than `nameservers`, and adopting
    /// twice does not duplicate an entry already recorded.
    #[test]
    fn adopting_a_search_domain_is_appended_and_deduplicated() {
        let mut device = a_device();
        let loss = diff::Loss {
            path: vec!["network".to_owned(), "resolver".to_owned(), "search".to_owned()],
            value: "lan".to_owned(),
            whole: false,
        };
        adopt(&mut device, &loss).expect("adopted");
        adopt(&mut device, &loss).expect("adopted again");
        assert_eq!(device.resolver.expect("present").search, ["lan"]);
    }

    /// A key `[network.resolver]` does not declare is refused by name rather
    /// than silently adopted or silently dropped - `rivet-config`'s own
    /// `Resolver::DECLARED_KEYS` is `nameservers` and `search`, so a real
    /// machine document could never produce this loss; this proves the arm
    /// says so instead of guessing where an unrecognised field should land.
    #[test]
    fn an_unknown_resolver_field_is_refused_rather_than_guessed_at() {
        let mut device = a_device();
        let loss = diff::Loss {
            path: vec!["network".to_owned(), "resolver".to_owned(), "method".to_owned()],
            value: "dhcp".to_owned(),
            whole: true,
        };
        let error = adopt(&mut device, &loss).expect_err("resolver has no method");
        assert!(error.contains("nameservers"), "{error}");
        assert!(error.contains("search"), "{error}");
        assert!(device.resolver.is_none(), "nothing should have been recorded");
    }

    // ---- SPEC-05 5: timezone and application settings ----------------------

    /// **`timezone`** adopts as a bare top-level fact — the gap `compose`'s
    /// strict path refuses to leave silent, filled from a machine that
    /// already answers it.
    #[test]
    fn adopting_a_timezone_lands_on_the_device_record() {
        let mut device = a_device();
        let loss = diff::Loss {
            path: vec!["timezone".to_owned()],
            value: "America/Chicago".to_owned(),
            whole: true,
        };
        adopt(&mut device, &loss).expect("adopted");
        assert_eq!(device.timezone.as_deref(), Some("America/Chicago"));
    }

    /// **`[kiosk] auto-run`** adopts under its own table, the gap `ADR-034`
    /// §6 gave `Device` a field for: before it existed, `rivet-01` - a kiosk
    /// running `aura-renderer` - could not be fully rebuilt from its record.
    #[test]
    fn adopting_a_kiosk_auto_run_lands_on_the_device_record() {
        let mut device = a_device();
        let loss = diff::Loss {
            path: vec!["kiosk".to_owned(), "auto-run".to_owned()],
            value: "aura-renderer".to_owned(),
            whole: true,
        };
        adopt(&mut device, &loss).expect("adopted");
        assert_eq!(device.kiosk_auto_run.as_deref(), Some("aura-renderer"));
    }

    /// **`[applications.<name>].<key>`** adopts under any application and any
    /// key at all — `ADR-038` §4 puts no schema on this region, so this arm
    /// has nothing to refuse. `upstream-name` is `SPEC-05` §5's own example;
    /// a second application in the same loss set proves the map is keyed by
    /// application and not flattened into one.
    #[test]
    fn adopting_application_settings_lands_under_the_right_application() {
        let mut device = a_device();
        for (app, key, value) in [
            ("rivet-proxyd", "upstream-name", "rivet-01.example.arpa"),
            ("tempmon-rivet", "sensor-labels", "GVH5075_1111=fridge"),
            ("tempmon-rivet", "write-token", "abc123"),
        ] {
            let loss = diff::Loss {
                path: vec!["applications".to_owned(), app.to_owned(), key.to_owned()],
                value: value.to_owned(),
                whole: true,
            };
            adopt(&mut device, &loss).unwrap_or_else(|e| panic!("{app}.{key}: {e}"));
        }
        assert_eq!(
            device.applications.get("rivet-proxyd").and_then(|a| a.get("upstream-name")),
            Some(&"rivet-01.example.arpa".to_owned())
        );
        let tempmon = device.applications.get("tempmon-rivet").expect("present");
        assert_eq!(tempmon.get("sensor-labels"), Some(&"GVH5075_1111=fridge".to_owned()));
        assert_eq!(tempmon.get("write-token"), Some(&"abc123".to_owned()));
    }

    /// Re-adopting the same key updates it rather than refusing or
    /// duplicating — an application setting is a scalar, not a list.
    #[test]
    fn readopting_an_application_setting_replaces_it() {
        let mut device = a_device();
        for value in ["old.example.arpa", "new.example.arpa"] {
            let loss = diff::Loss {
                path: vec![
                    "applications".to_owned(),
                    "rivet-proxyd".to_owned(),
                    "upstream-name".to_owned(),
                ],
                value: value.to_owned(),
                whole: true,
            };
            adopt(&mut device, &loss).expect("adopted");
        }
        assert_eq!(
            device.applications.get("rivet-proxyd").and_then(|a| a.get("upstream-name")),
            Some(&"new.example.arpa".to_owned())
        );
    }

    /// **End to end: everything a machine's document carries that `SPEC-05`
    /// §5 requires is adoptable, and the record composes a FINAL document
    /// afterwards** — the acceptance this whole change is for, run against
    /// `compose::provisioning_complete` rather than assumed.
    #[test]
    fn a_fully_adopted_record_composes_a_final_document() {
        let mut device = a_device();
        device.fleet_role = Some(register::FleetRole::Server);
        let fleet = register::Fleet::default();
        let losses = [
            (vec!["timezone".to_owned()], "America/Chicago", true),
            (
                vec!["applications".to_owned(), "rivet-proxyd".to_owned(), "upstream-name".to_owned()],
                "rivet-01.example.arpa",
                true,
            ),
            (vec!["services".to_owned(), "rivet-fleetd".to_owned(), "enabled".to_owned()], "true", true),
            (vec!["services".to_owned(), "rivet-proxyd".to_owned(), "enabled".to_owned()], "true", true),
            (vec!["services".to_owned(), "rivet-mdnsd".to_owned(), "enabled".to_owned()], "true", true),
        ];
        for (path, value, whole) in losses {
            let loss = diff::Loss { path, value: value.to_owned(), whole };
            adopt(&mut device, &loss).unwrap_or_else(|e| panic!("{}: {e}", loss.where_it_is()));
        }
        compose::provisioning_complete(&device, &fleet)
            .expect("every SPEC-05 5 gate is answered by what was just adopted");
    }

    // -----------------------------------------------------------------------
    // `set timezone` and `set kiosk-auto-run` — the two register-only verbs
    // a brand-new record has no machine to `add document` from yet.
    // -----------------------------------------------------------------------

    /// **The gap `SPEC-05` §5 left open.** A record minted before its
    /// machine has ever booted has nothing for `add document` to adopt from,
    /// so this verb is what answers the gate before a machine exists to ask.
    #[test]
    fn set_timezone_records_a_well_formed_zone() {
        let scratch = Scratch::new("set-timezone");
        let (register, device) = one_device_register(&scratch, None);
        let request = Request {
            operation: operation::find(Verb::Set, "timezone").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.name.clone()]),
                ("timezone", vec!["America/Chicago".to_owned()]),
            ],
            confirmed: false,
        };
        let report = perform(&request, &operator::Operator::default(), &register.root, None)
            .expect("a well-formed zone is recorded");
        assert!(report.contains("America/Chicago"), "{report}");

        let reloaded = store::load(&scratch.0).expect("readable");
        assert_eq!(
            reloaded.devices.get(&device.id).and_then(|d| d.timezone.clone()),
            Some("America/Chicago".to_owned())
        );
    }

    /// **The refusal.** A value that is not shaped like an IANA zone at all
    /// must not reach the register, and the message must say this checks
    /// grammar only - `set_timezone`'s own doc has the reason a stronger
    /// check is not reachable from here.
    #[test]
    fn set_timezone_refuses_a_value_that_is_not_zone_shaped() {
        let scratch = Scratch::new("set-timezone-bad");
        let (register, device) = one_device_register(&scratch, None);
        let request = Request {
            operation: operation::find(Verb::Set, "timezone").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.name.clone()]),
                ("timezone", vec!["../../etc/passwd".to_owned()]),
            ],
            confirmed: false,
        };
        let error = perform(&request, &operator::Operator::default(), &register.root, None)
            .expect_err("a path is not a zone name");
        assert!(error.contains("GRAMMAR"), "{error}");

        // `a_device()` starts with `timezone = Some("UTC")` - the refusal
        // must leave it exactly there rather than clearing it.
        let reloaded = store::load(&scratch.0).expect("readable");
        assert_eq!(
            reloaded.devices.get(&device.id).and_then(|d| d.timezone.clone()),
            Some("UTC".to_owned()),
            "nothing must have been written"
        );
    }

    /// **`kiosk_auto_run`'s own register-only verb**, for the record
    /// `rivet-01` - a kiosk running `aura-renderer` - could not have been
    /// composed from before `add document` ever reached it.
    #[test]
    fn set_kiosk_auto_run_records_the_named_application() {
        let scratch = Scratch::new("set-kiosk-auto-run");
        let (register, device) = one_device_register(&scratch, None);
        let request = Request {
            operation: operation::find(Verb::Set, "kiosk-auto-run").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.name.clone()]),
                ("application", vec!["aura-renderer".to_owned()]),
            ],
            confirmed: false,
        };
        let report = perform(&request, &operator::Operator::default(), &register.root, None)
            .expect("a name is recorded");
        assert!(report.contains("aura-renderer"), "{report}");
        assert!(report.contains("NOTHING HERE CAN CHECK"), "{report}");

        let reloaded = store::load(&scratch.0).expect("readable");
        assert_eq!(
            reloaded.devices.get(&device.id).and_then(|d| d.kiosk_auto_run.clone()),
            Some("aura-renderer".to_owned())
        );
    }

    /// **The one bad value this verb can refuse.** Nothing else about the
    /// name is checked - `SPEC-05` §5 leaves whether it will be installed
    /// deliberately open - so a blank answer, which has no way to mean
    /// anything through this verb, is the one thing left to catch.
    #[test]
    fn set_kiosk_auto_run_refuses_an_empty_name() {
        let scratch = Scratch::new("set-kiosk-auto-run-empty");
        let (register, device) = one_device_register(&scratch, None);
        let request = Request {
            operation: operation::find(Verb::Set, "kiosk-auto-run").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.name.clone()]),
                ("application", vec!["   ".to_owned()]),
            ],
            confirmed: false,
        };
        let error = perform(&request, &operator::Operator::default(), &register.root, None)
            .expect_err("whitespace is not a name");
        assert!(error.contains("empty"), "{error}");

        let reloaded = store::load(&scratch.0).expect("readable");
        assert_eq!(
            reloaded.devices.get(&device.id).and_then(|d| d.kiosk_auto_run.clone()),
            None
        );
    }

    // -----------------------------------------------------------------------
    // `set carry` / `set fleet-carry` clearing with `none`, `list carry` and
    // `explain device`'s pins — `.rulings/` R-162 R30
    // -----------------------------------------------------------------------

    fn set_carry_request(device: &str, product: &str, version: &str) -> Request {
        Request {
            operation: operation::find(Verb::Set, "carry").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.to_owned()]),
                ("product", vec![product.to_owned()]),
                ("version", vec![version.to_owned()]),
            ],
            confirmed: false,
        }
    }

    fn set_fleet_carry_request(product: &str, version: &str) -> Request {
        Request {
            operation: operation::find(Verb::Set, "fleet-carry").expect("catalogued"),
            inputs: vec![
                ("product", vec![product.to_owned()]),
                ("version", vec![version.to_owned()]),
            ],
            confirmed: false,
        }
    }

    /// **Clearing a device's own pin falls back to the fleet's** — R30's
    /// first acceptance case.
    #[test]
    fn clearing_a_device_carry_pin_falls_back_to_the_fleet_pin() {
        let scratch = Scratch::new("set-carry-clear-device");
        let (register, device) = one_device_register(&scratch, None);
        let operator = operator::Operator::default();

        perform(&set_fleet_carry_request("hello-rivet", "0.4.2"), &operator, &register.root, None)
            .expect("the fleet pin is recorded");
        perform(
            &set_carry_request(&device.name, "hello-rivet", "0.4.3"),
            &operator,
            &register.root,
            None,
        )
        .expect("the device pin is recorded");

        let report = perform(
            &set_carry_request(&device.name, "hello-rivet", "none"),
            &operator,
            &register.root,
            None,
        )
        .expect("`none` clears a pin");
        assert!(report.contains("no longer pins"), "{report}");
        assert!(report.contains("0.4.2"), "the fallback fleet pin must be named: {report}");

        let reloaded = store::load(&scratch.0).expect("readable");
        let mine = reloaded.devices.get(&device.id).expect("the device");
        assert!(!mine.carry.contains_key("hello-rivet"), "the device's own pin must be gone");
        assert_eq!(
            compose::carry(mine, &reloaded.fleet).get("hello-rivet"),
            Some(&"0.4.2".to_owned()),
            "the effective pin now falls through to the fleet's"
        );
    }

    /// **Clearing the fleet pin leaves no pin**, when nothing overrides it.
    #[test]
    fn clearing_the_fleet_carry_pin_leaves_no_pin() {
        let scratch = Scratch::new("set-fleet-carry-clear");
        let (register, device) = one_device_register(&scratch, None);
        let operator = operator::Operator::default();

        perform(&set_fleet_carry_request("hello-rivet", "0.4.2"), &operator, &register.root, None)
            .expect("the fleet pin is recorded");

        let report =
            perform(&set_fleet_carry_request("hello-rivet", "none"), &operator, &register.root, None)
                .expect("`none` clears the fleet pin");
        assert!(report.contains("no longer pins"), "{report}");

        let reloaded = store::load(&scratch.0).expect("readable");
        assert!(!reloaded.fleet.carry.contains_key("hello-rivet"));
        let mine = reloaded.devices.get(&device.id).expect("the device");
        assert!(compose::carry(mine, &reloaded.fleet).is_empty(), "no pin anywhere now");
    }

    /// **A pin on an image service is refused, for one device and for the
    /// fleet, and nothing is written.** The proxy is part of the image
    /// (`ADR-060`), so no card carries it and no build would read the pin.
    #[test]
    fn a_carry_pin_on_an_image_service_is_refused_and_nothing_changes() {
        let scratch = Scratch::new("set-carry-image-service");
        let (register, device) = one_device_register(&scratch, None);
        let operator = operator::Operator::default();

        let error = perform(
            &set_carry_request(&device.name, "rivet-proxyd", "0.1.87-g767b86f"),
            &operator,
            &register.root,
            None,
        )
        .expect_err("an image service cannot be pinned for a device");
        assert!(error.contains("part of the system image"), "{error}");
        assert!(error.contains("Nothing has been changed"), "{error}");

        let error = perform(
            &set_fleet_carry_request("rivet-proxyd", "0.1.87-g767b86f"),
            &operator,
            &register.root,
            None,
        )
        .expect_err("an image service cannot be pinned for the fleet");
        assert!(error.contains("part of the system image"), "{error}");

        let reloaded = store::load(&scratch.0).expect("readable");
        assert!(reloaded.fleet.carry.is_empty(), "{:?}", reloaded.fleet.carry);
        let mine = reloaded.devices.get(&device.id).expect("the device");
        assert!(mine.carry.is_empty(), "{:?}", mine.carry);
    }

    /// **A legacy pin on an image service can still be cleared, and the note
    /// says nothing falls back.** dev-01's record held `rivet-proxyd` from
    /// before the proxy joined the image; clearing it used to say a card build
    /// would now refuse, which described a carry that no longer happens.
    #[test]
    fn a_legacy_image_service_pin_clears_and_says_nothing_carries_it() {
        let scratch = Scratch::new("set-carry-legacy-image-service");
        let (_, device) = one_device_register(&scratch, None);
        let mut register = store::load(&scratch.0).expect("readable");
        register
            .devices
            .get_mut(&device.id)
            .expect("the device")
            .carry
            .insert("rivet-proxyd".to_owned(), "0.1.87-g767b86f".to_owned());
        store::save(&register).expect("writable");

        let report = perform(
            &set_carry_request(&device.name, "rivet-proxyd", "none"),
            &operator::Operator::default(),
            &register.root,
            None,
        )
        .expect("`none` clears a legacy pin");
        assert!(report.contains("no longer pins its own version of rivet-proxyd"), "{report}");
        assert!(report.contains("was 0.1.87-g767b86f"), "{report}");
        assert!(report.contains("part of the system image"), "{report}");
        assert!(!report.contains("refuses unless"), "{report}");

        let reloaded = store::load(&scratch.0).expect("readable");
        let mine = reloaded.devices.get(&device.id).expect("the device");
        assert!(!mine.carry.contains_key("rivet-proxyd"), "{:?}", mine.carry);
    }

    /// **`list carry` and `explain device` both show the right source.**
    #[test]
    fn list_and_explain_show_carry_pins_with_their_source() {
        let scratch = Scratch::new("carry-list-explain");
        let (register, device) = one_device_register(&scratch, None);
        let operator = operator::Operator::default();

        perform(&set_fleet_carry_request("hello-rivet", "0.4.2"), &operator, &register.root, None)
            .expect("the fleet pin is recorded");
        perform(
            &set_carry_request(&device.name, "other-product", "1.0.0"),
            &operator,
            &register.root,
            None,
        )
        .expect("the device pin is recorded");

        let listed = list_carry(&scratch.0).expect("a readable register");
        assert!(listed.contains("hello-rivet") && listed.contains("from the fleet"), "{listed}");
        assert!(
            listed.contains("other-product") && listed.contains("from this device"),
            "{listed}"
        );

        let explain_request = Request {
            operation: operation::find(Verb::Explain, "device").expect("catalogued"),
            inputs: vec![("device", vec![device.name.clone()])],
            confirmed: false,
        };
        let explained = perform(&explain_request, &operator, &scratch.0, None).expect("explains");
        assert!(
            explained.contains("hello-rivet") && explained.contains("from the fleet"),
            "{explained}"
        );
        assert!(
            explained.contains("other-product") && explained.contains("from this device"),
            "{explained}"
        );
    }

    /// **A device with no pin at all says so, in both surfaces.**
    #[test]
    fn list_and_explain_say_so_when_nothing_is_pinned() {
        let scratch = Scratch::new("carry-list-explain-empty");
        let (register, device) = one_device_register(&scratch, None);

        let listed = list_carry(&scratch.0).expect("a readable register");
        assert!(listed.contains("no carried-version pin"), "{listed}");

        let explain_request = Request {
            operation: operation::find(Verb::Explain, "device").expect("catalogued"),
            inputs: vec![("device", vec![device.name.clone()])],
            confirmed: false,
        };
        let explained =
            perform(&explain_request, &operator::Operator::default(), &register.root, None)
                .expect("explains");
        assert!(explained.contains("NO PIN"), "{explained}");
    }

    // -----------------------------------------------------------------------
    // `add document`'s report — §3: losses and changes, both, not one
    // -----------------------------------------------------------------------

    /// **The regression test for the finding that named this gap.** The
    /// maintainer's own register held a `listen` that disagreed with a
    /// machine's, alongside a loss `add document` could adopt, and a dry run
    /// reported only the loss - `diff::changes` was never called from the
    /// plan branch. This constructs the same shape - one LOSS and one CHANGE
    /// in a single comparison - and asserts the plan names both.
    #[test]
    fn add_documents_plan_reports_losses_and_changes_together() {
        let scratch = Scratch::new("add-document-plan-both");
        let mut device = a_device();
        device.listen = Some("192.168.1.1:7777".to_owned());
        let mut register = Register::empty(&scratch.0);
        register.enroll(device.clone()).expect("fresh");
        store::save(&register).expect("writable");

        // The machine agrees on `listen`'s PRESENCE and disagrees on its
        // VALUE (a change), and carries `rivet-mcpd` enabled, which the
        // register does not (a loss).
        let mut running = device.clone();
        running.listen = Some("0.0.0.0:7777".to_owned());
        running.services.insert("rivet-mcpd".to_owned(), true);
        let machine = compose::provisioning_toml(&running, &register.fleet).expect("composable");

        let report =
            adopt_report(&mut register, &device, &machine, false, None).expect("plans");

        assert!(
            report.contains("services.rivet-mcpd.enabled"),
            "the LOSS must be reported:\n{report}"
        );
        assert!(
            report.contains("control-plane.listen"),
            "the CHANGE must be reported too - this is the defect: \n{report}"
        );
        assert!(report.contains("0.0.0.0:7777"), "the machine's value is missing:\n{report}");
        assert!(report.contains("192.168.1.1:7777"), "the register's value is missing:\n{report}");

        // And it is still a PLAN: neither the loss nor the change was written.
        let reloaded = store::load(&scratch.0).expect("readable");
        let saved = reloaded.devices.get(&device.id).expect("still enrolled");
        assert_eq!(saved.listen.as_deref(), Some("192.168.1.1:7777"));
        assert!(!saved.services.contains_key("rivet-mcpd"));
    }

    /// **The same shape, confirmed.** The loss is adopted; the change is
    /// reported and left exactly where it was - `add document` never writes
    /// a value the register already holds an opinion about.
    #[test]
    fn add_documents_confirmed_adoption_still_names_the_change_it_leaves_alone() {
        let scratch = Scratch::new("add-document-confirm-both");
        let mut device = a_device();
        device.listen = Some("192.168.1.1:7777".to_owned());
        let mut register = Register::empty(&scratch.0);
        register.enroll(device.clone()).expect("fresh");
        store::save(&register).expect("writable");

        let mut running = device.clone();
        running.listen = Some("0.0.0.0:7777".to_owned());
        running.services.insert("rivet-mcpd".to_owned(), true);
        let machine = compose::provisioning_toml(&running, &register.fleet).expect("composable");

        let report =
            adopt_report(&mut register, &device, &machine, true, None).expect("adopts");
        assert!(report.contains("services.rivet-mcpd"), "{report}");
        assert!(report.contains("control-plane.listen"), "{report}");

        let reloaded = store::load(&scratch.0).expect("readable");
        let saved = reloaded.devices.get(&device.id).expect("still enrolled");
        assert_eq!(saved.services.get("rivet-mcpd"), Some(&true), "the LOSS was adopted");
        assert_eq!(
            saved.listen.as_deref(),
            Some("192.168.1.1:7777"),
            "the CHANGE was left where the register already had an opinion"
        );
    }

    // -----------------------------------------------------------------------
    // The fleet's name — `PLAN.md` §146.3
    // -----------------------------------------------------------------------

    /// **The requirement is not that a name can be chosen; it is that HIS
    /// survives.** §146.3 states it in those terms because the value the
    /// maintainer intends to type has a space in it, and the failure being
    /// guarded against is a tokeniser splitting it rather than a missing
    /// feature. So the case with the space is the case that is asserted.
    #[test]
    fn a_fleet_name_with_a_space_in_it_survives_the_round_trip() {
        let scratch = Scratch::new("fleet-name-space");
        let mut register = Register::empty(&scratch.0);

        let line = name_the_fleet(&mut register, Some("Someone's fleet"), None).expect("names it");

        assert_eq!(register.fleet.name.as_deref(), Some("Someone's fleet"));
        assert!(line.contains("Someone's fleet"), "{line}");

        // And it is on disk, not only in memory — the next command loads it.
        let reloaded = store::load(&scratch.0).expect("loads");
        assert_eq!(reloaded.fleet.name.as_deref(), Some("Someone's fleet"));
    }

    /// **Asked once.** A register that already carries a name is not asked
    /// again and is not overwritten, which is the property that lets this sit
    /// on `add device` — a verb that runs many times — without becoming a
    /// question an operator learns to dismiss.
    #[test]
    fn a_fleet_that_already_has_a_name_is_not_asked_again_or_overwritten() {
        let scratch = Scratch::new("fleet-name-once");
        let mut register = Register::empty(&scratch.0);
        register.fleet.name = Some("home".to_owned());

        // A second value is offered and must be ignored rather than applied.
        let line = name_the_fleet(&mut register, Some("something-else"), None).expect("returns");

        assert_eq!(register.fleet.name.as_deref(), Some("home"));
        assert!(line.is_empty(), "an unasked question reported something: {line}");
    }

    /// **A script takes the default rather than blocking**, which is
    /// `cmd/cargo-rivet`'s `resolve_name` rule and its reason: a prompt nobody
    /// can answer is a hang, and a first run under CI is exactly where that
    /// would be discovered late. The test suite is not a terminal, so this is
    /// the branch it takes with nothing given.
    #[test]
    fn with_no_value_and_no_terminal_the_default_is_taken_silently() {
        let scratch = Scratch::new("fleet-name-default");
        let mut register = Register::empty(&scratch.0);

        let line = name_the_fleet(&mut register, None, None).expect("defaults");

        assert_eq!(register.fleet.name.as_deref(), Some(DEFAULT_FLEET));
        assert!(line.contains(DEFAULT_FLEET), "{line}");
    }

    /// **The two fleet names are separate facts and this pins it.**
    /// `cmd/rivet-provision` freezes its fleet label into a certificate
    /// authority's name constraint; this one is a mutable label on a register.
    /// They happen to share a default string today, and a reader who concluded
    /// they were one value would expect `set fleet --name` to reach a
    /// certificate. It does not.
    #[test]
    fn the_registers_fleet_name_is_mutable_where_the_authoritys_is_not() {
        let scratch = Scratch::new("fleet-name-mutable");
        let mut register = Register::empty(&scratch.0);

        name_the_fleet(&mut register, Some("my-fleet"), None).expect("names it");
        assert_eq!(register.fleet.name.as_deref(), Some("my-fleet"));

        // `set fleet --name` rewrites it, which is the whole difference.
        register.fleet.name = Some("renamed".to_owned());
        store::save_fleet(&register.root, &register.fleet).expect("saves");

        let reloaded = store::load(&scratch.0).expect("loads");
        assert_eq!(reloaded.fleet.name.as_deref(), Some("renamed"));
    }

    // -----------------------------------------------------------------------
    // `add identity` and `--confirm` — AGENTS.md's own warning, checked
    // against the current source rather than trusted
    // -----------------------------------------------------------------------

    /// **The dry run must mint nothing.** `add_identity`'s own doc names the
    /// defect this pins: `--confirm` once went unread and a documented plan
    /// minted `device.seed`, `device.crt` and `peers` anyway, so the careful
    /// habit of looking before committing destroyed the artifact it was
    /// trying to protect. Everything here is a temporary directory — the
    /// register root, `project_root` (`None`, so nothing is projected into a
    /// fleet repository either) and the identity's own `--out` — so a false
    /// result here can only mean the CODE minted unconfirmed, never that this
    /// test touched anything real.
    #[test]
    fn add_identity_without_confirm_writes_nothing_and_leaves_the_register_unchanged() {
        let scratch = Scratch::new("add-identity-plan");
        let out = Scratch::new("add-identity-plan-out");
        let (register, device) = one_device_register(&scratch, None);
        assert_eq!(device.pin, None, "the fixture must start with no recorded pin");

        let request = Request {
            operation: operation::find(Verb::Add, "identity").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.name.clone()]),
                ("out", vec![out.0.display().to_string()]),
            ],
            confirmed: false,
        };
        let report = perform(&request, &operator::Operator::default(), &register.root, None)
            .expect("an unconfirmed request is always answerable with a plan");
        assert!(report.contains("PLAN"), "{report}");
        assert!(report.contains("nothing has been written"), "{report}");

        assert!(!out.0.join("device.seed").exists(), "a dry run minted a seed");
        assert!(!out.0.join("device.crt").exists(), "a dry run minted a certificate");
        assert!(!out.0.join("peers").exists(), "a dry run wrote a peers file");

        let reloaded = store::load(&scratch.0).expect("the register is still readable");
        assert_eq!(
            reloaded.devices.get(&device.id).and_then(|d| d.pin.clone()),
            None,
            "a dry run recorded a pin the register did not have before"
        );
    }

    /// **The contrast.** `--confirm` still has to work — the fix for the bug
    /// above is reading the flag, not disabling minting — so this shows the
    /// same request, confirmed, actually writes the three files into `--out`
    /// and records the public half in the register. Runs the real `cargo
    /// rivet identity new` this operation shells out to (`cargo install
    /// --path cmd/cargo-rivet --force` puts it on `PATH` first); every path
    /// it touches is one of this test's own temporary directories.
    ///
    /// **Ignored by default** because it needs that installed `cargo rivet`,
    /// which the build container's `tools/test-all.sh` does not have. The dry
    /// run above is the half that guards the defect and needs nothing installed.
    #[test]
    #[ignore = "needs an installed cargo-rivet on PATH: cargo install --path cmd/cargo-rivet --force --locked"]
    fn add_identity_with_confirm_mints_and_records_the_public_half() {
        let scratch = Scratch::new("add-identity-confirm");
        let out = Scratch::new("add-identity-confirm-out");
        let (register, device) = one_device_register(&scratch, None);

        let request = Request {
            operation: operation::find(Verb::Add, "identity").expect("catalogued"),
            inputs: vec![
                ("device", vec![device.name.clone()]),
                ("out", vec![out.0.display().to_string()]),
            ],
            confirmed: true,
        };
        let report = perform(&request, &operator::Operator::default(), &register.root, None)
            .expect(
                "a confirmed mint should succeed — if this failed because `cargo rivet` is not \
                 a recognised subcommand, install it first: `cargo install --path \
                 cmd/cargo-rivet --force --locked`",
            );
        assert!(report.contains("minted a control-plane identity"), "{report}");

        assert!(out.0.join("device.seed").exists(), "the seed was not written");
        assert!(out.0.join("device.crt").exists(), "the certificate was not written");
        assert!(out.0.join("peers").exists(), "the peers file was not written");

        let reloaded = store::load(&scratch.0).expect("the register is still readable");
        let pin = reloaded.devices.get(&device.id).and_then(|d| d.pin.clone());
        assert!(pin.is_some(), "a confirmed mint did not record the public half");
    }
}
