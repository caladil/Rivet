//! **`rivet-provision stage`** — the `ADR-049` staging bundle for a document
//! that already exists, without the interview.
//!
//! `w` composes a bundle beside every server or member document it writes. A
//! document that reaches `cargo rivet provision --document` or `cargo rivet
//! write-image --document` some other way — composed by an agent, edited by
//! hand, copied from another workstation — has none, and a server or member
//! card needs one whatever it carries: the front door is handed its
//! certificate from it, and a server's `rivet-fleetd` its register and
//! credentials (`ADR-050` §4). A missing bundle is built rather than refused
//! (`ADR-049` §12), and this verb is what builds it.
//!
//! **It is `crate::stage::compose` and nothing else.** The role, the hostname,
//! the listener and the addressing are read out of the document, where the
//! interview reads them out of its answers; the authority, the Armada and the
//! previous bundle come from the same credential root the interview uses. So a
//! bundle built here is the bundle `w` would have written for the same
//! document, including the two side effects `w` has: the machine's `peers`
//! gains the key its fleet server presents, and the machine's own leaf is
//! written into the authority directory for a later deploy to read.
//!
//! **It prints what it staged by name and size, never by content.** Half of a
//! bundle is private keys.

use std::path::{Path, PathBuf};

use rivet_core::staging::Staged;

/// `rivet-fleetd`, whose being enabled makes a document a fleet server's.
const FLEETD: &str = "rivet-fleetd";
/// `rivet-proxyd`, whose being enabled without `rivet-fleetd` makes a
/// document a fleet member's.
const PROXYD: &str = "rivet-proxyd";

/// What `stage --help` prints, and the usage a refusal names.
pub(crate) const USAGE: &str = "\
usage: rivet-provision stage --document PATH --identity DIR [--at DIR]
                             [--fleet-root FLEET [--suffixes LIST]]
                             [--passkeys FILE]

  Build the ADR-049 staging bundle for a server or member document that has
  none, without the interview. The role, hostname, listener and addressing are
  read from the document; the authority from <at>/ca; a member's fleet server
  key from <at>/armada. The bundle is written beside the document, as
  <stem>.staged.txt for a <stem>.provisioning.toml and <hostname>.staged.txt
  otherwise, and what it holds is listed by name and size.

  --document PATH  the provisioning document
  --identity DIR   the machine's own identity (device.crt, peers). Its peers
                   file gains the key its fleet server presents
  --at DIR         the credential root. Default: RIVET_AT, then ~/.rivet
  --fleet-root FLEET
                   fleet-root mode (ADR-063 section 4.2), for a SERVER: the
                   card carries a request and the machine mints FLEET's root
                   and its own certificate on first boot. No authority is
                   read and none is needed. Without the flag, a fleet-root
                   choice the first-provision guard recorded in <at>/ca is
                   used when there is one
  --suffixes LIST  the suffixes that root is constrained to, from local,
                   FLEET.home.arpa and FLEET.lan. Default: local and
                   FLEET.home.arpa
  --passkeys FILE  a passkeys file read off this machine's proxy, carried to
                   the proxy on the new card so enrolled passkeys keep working
                   at every name they were enrolled at

  cargo rivet provision and cargo rivet write-image run this themselves when
  a server or member document has no bundle beside it.";

/// What one run staged — returned rather than printed, so a test reads it.
/// `run` prints [`Self::lines`]; the other two are what a test asserts on.
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Report {
    /// Where the bundle was written; `None` for a document with no role, for
    /// which nothing is staged.
    pub path: Option<PathBuf>,
    /// `server` or `member`, or `None` for a document with neither.
    pub role: Option<&'static str>,
    /// Every line to print, in order: `stage::compose`'s own notes, then the
    /// bundle's files by name and size, then the workstation leaf.
    pub lines: Vec<String>,
}

/// `rivet-provision stage ARGS`, from `main`.
pub(crate) fn run(args: &[String]) -> std::process::ExitCode {
    let mut document: Option<PathBuf> = None;
    let mut identity: Option<PathBuf> = None;
    let mut at: Option<PathBuf> = None;
    let mut fleet_root: Option<PathBuf> = None;
    let mut suffixes: Option<PathBuf> = None;
    let mut passkeys: Option<PathBuf> = None;
    let mut rest = args.iter();
    while let Some(one) = rest.next() {
        let slot = match one.as_str() {
            "--document" => &mut document,
            "--identity" => &mut identity,
            "--at" => &mut at,
            "--fleet-root" => &mut fleet_root,
            "--suffixes" => &mut suffixes,
            "--passkeys" => &mut passkeys,
            "-h" | "--help" => {
                println!("{USAGE}");
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                eprintln!("rivet-provision stage: {other:?} is not an argument this verb takes\n\n{USAGE}");
                return std::process::ExitCode::FAILURE;
            }
        };
        let Some(value) = rest.next() else {
            eprintln!("rivet-provision stage: {one} needs a value\n\n{USAGE}");
            return std::process::ExitCode::FAILURE;
        };
        *slot = Some(PathBuf::from(value));
    }
    let (Some(document), Some(identity)) = (document, identity) else {
        eprintln!("rivet-provision stage: --document and --identity are both required\n\n{USAGE}");
        return std::process::ExitCode::FAILURE;
    };
    let Some(root) = at.or_else(crate::rivet_root) else {
        eprintln!(
            "rivet-provision stage: neither RIVET_AT, HOME nor USERPROFILE is set, so there is no \
             credential root to read the authority from. Pass --at DIR."
        );
        return std::process::ExitCode::FAILURE;
    };

    if suffixes.is_some() && fleet_root.is_none() {
        eprintln!("rivet-provision stage: --suffixes belongs to --fleet-root\n\n{USAGE}");
        return std::process::ExitCode::FAILURE;
    }
    let extras = Extras {
        fleet_root: fleet_root.map(|f| f.to_string_lossy().into_owned()),
        suffixes: suffixes.map(|s| crate::split_suffixes(&s.to_string_lossy())),
        passkeys,
    };
    match stage_with(&document, &identity, &root, &extras) {
        Ok(report) => {
            for line in &report.lines {
                println!("{line}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("rivet-provision stage: NOTHING STAGED. {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Which role `doc` names, read off the services it enables —
/// `cmd/cargo-rivet`'s `role_of`, restated because neither crate is a library
/// the other can call.
fn role_of(doc: &rivet_config::Document) -> Option<&'static str> {
    let enabled = |name: &str| doc.service(name).is_some_and(rivet_config::services::Service::enabled);
    if enabled(FLEETD) {
        Some("server")
    } else if enabled(PROXYD) {
        Some("member")
    } else {
        None
    }
}

/// Compose and write the bundle for `document`, reading the authority and the
/// Armada under `root`.
///
/// # Errors
///
/// The document cannot be read or does not parse, names no hostname, the
/// authority or the identity is missing — each named with the path it was
/// looked for at — or `stage::compose` or the write refuses. **Nothing is
/// written before every check has passed.**
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn stage(document: &Path, identity: &Path, root: &Path) -> Result<Report, String> {
    stage_with(document, identity, root, &Extras::default())
}

/// What `stage` may be asked beyond the document, the identity and the root.
#[derive(Debug, Default)]
pub(crate) struct Extras {
    /// `--fleet-root FLEET`: the server's root is minted on the machine.
    pub fleet_root: Option<String>,
    /// `--suffixes LIST`, for `--fleet-root`.
    pub suffixes: Option<Vec<String>>,
    /// `--passkeys FILE`: carried to the proxy verbatim.
    pub passkeys: Option<PathBuf>,
}

/// [`stage`], with [`Extras`].
///
/// # Errors
///
/// [`stage`]'s, and a fleet-root request that does not compose, or a
/// passkeys file that cannot be read or is empty.
pub(crate) fn stage_with(
    document: &Path,
    identity: &Path,
    root: &Path,
    extras: &Extras,
) -> Result<Report, String> {
    let text = std::fs::read_to_string(document)
        .map_err(|e| format!("cannot read the document {}: {e}", document.display()))?;
    let doc = rivet_config::Document::parse(&text)
        .map_err(|e| format!("{} does not parse: {e:?}", document.display()))?;

    let Some(role) = role_of(&doc) else {
        return Ok(Report {
            path: None,
            role: None,
            lines: vec![format!(
                "  nothing staged: {} enables neither {FLEETD} nor {PROXYD}, so no service of its \
                 role reads per-machine material from the card",
                document.display()
            )],
        });
    };

    let hostname = doc.hostname().map(ToString::to_string).ok_or_else(|| {
        format!(
            "{} is a fleet {role}'s document and names no hostname. Its certificate, its register \
             record and its bundle's file name are all derived from one, so add `hostname = \
             \"...\"` at the top of the document.",
            document.display()
        )
    })?;

    // **Both named by path, before anything is minted.** `compose` would
    // refuse either later, in a sentence about a file rather than about which
    // of the two things a person has to go and make.
    let authority = root.join("ca");
    // **Fleet-root mode** (`ADR-063` §4): named on the command line, or
    // recorded by the first-provision guard. Either way the workstation's
    // authority is never read for this card.
    let fleet_root = match &extras.fleet_root {
        Some(fleet) => {
            let suffixes = extras
                .suffixes
                .clone()
                .unwrap_or_else(|| rivet_core::fleet_root::default_suffixes(fleet));
            Some(
                rivet_core::fleet_root::Request::new(fleet, &hostname, &suffixes)
                    .map_err(|why| format!("--fleet-root {fleet}: {why}"))?,
            )
        }
        None if role == "server" => crate::fleet_root_request(&authority, &hostname)?,
        None => None,
    };
    if fleet_root.is_some() && role != "server" {
        return Err(format!(
            "fleet-root mode is a FLEET SERVER's, and {} is a fleet {role}'s document",
            document.display()
        ));
    }
    let passkeys = match &extras.passkeys {
        None => None,
        Some(path) => {
            let bytes = std::fs::read(path)
                .map_err(|e| format!("--passkeys {}: {e}", path.display()))?;
            if bytes.is_empty() || core::str::from_utf8(&bytes).is_err() {
                return Err(format!(
                    "--passkeys {} is empty or not text, and a proxy's passkeys file is one \
                     line of text per passkey",
                    path.display()
                ));
            }
            Some(bytes)
        }
    };
    for file in ["root.crt", "root.key"] {
        if fleet_root.is_some() {
            break;
        }
        if !authority.join(file).is_file() {
            return Err(format!(
                "no certificate authority at {}: {file} is missing. A fleet {role}'s front door is \
                 issued its certificate from it. `rivet-provision ca <suffixes> <fleet-name> {}` \
                 mints one, and the interview offers to.",
                authority.display(),
                authority.display()
            ));
        }
    }
    if !identity.join(rivetos_tls::CERTIFICATE_FILE).is_file() {
        return Err(format!(
            "no identity at {}: {} is missing. It is this machine's own control-plane key — the \
             register record's pin, and the directory whose peers file gains the key its fleet \
             server presents. The interview mints one at `w`; `cargo rivet setup` mints the board \
             identity.",
            identity.display(),
            rivetos_tls::CERTIFICATE_FILE
        ));
    }

    // The first interface named `eth0`, the one the interview writes, or the
    // first there is. Only a server's register record carries it.
    let interface = doc
        .network()
        .iter()
        .find(|i| i.name().as_str() == "eth0")
        .or_else(|| doc.network().first());
    let (method, address, gateway) = match interface.map(rivet_config::Interface::addressing) {
        Some(rivet_config::Addressing::Static { address, gateway }) => (
            "static",
            address.to_string(),
            gateway.map(|g| g.to_string()).unwrap_or_default(),
        ),
        _ => ("dhcp", String::new(), String::new()),
    };
    let control_plane = doc.effective_control_plane();
    let listen = control_plane.listen().to_owned();

    let bundle = crate::save::staging_beside(document, &hostname);
    // **The operator credential a previous bundle for this machine carries is
    // reused**, as `w` reuses it: beside the document first, then where `w`
    // itself writes. A fresh one is minted only when neither holds one.
    let staged_before = std::fs::read_to_string(&bundle).ok().or_else(|| {
        let saved = root.join("provision").join(format!(
            "{}{}",
            crate::save::safe_name(&hostname),
            crate::save::STAGING_SUFFIX
        ));
        std::fs::read_to_string(saved).ok()
    });
    let operator_key = (role == "member").then(|| crate::fleet_operator_key_in(root)).flatten();

    let machine = crate::stage::Machine {
        fleet: role,
        hostname: &hostname,
        listen: &listen,
        method,
        address: &address,
        gateway: &gateway,
        authority: &authority,
        identity: Some(identity),
        staged_before: staged_before.as_deref(),
        operator_key: operator_key.as_deref(),
        fleet_root: fleet_root.as_ref(),
    };
    let mut plan = crate::stage::compose(&machine)?;
    if let Some(bytes) = passkeys {
        let count = bytes.iter().filter(|b| **b == b'\n').count();
        plan.items.push(
            Staged::new(PROXYD, "passkeys", bytes).map_err(|why| format!("internal defect: {why}"))?,
        );
        plan.notes.push(format!(
            "  staged for {PROXYD}: passkeys, carried from the machine's own proxy ({count} \
             line(s)). Each passkey keeps working at the name it was enrolled at, where the new \
             certificate still carries that name"
        ));
    }

    let mut lines = vec![format!(
        "rivet-provision stage: {hostname}, a fleet {role}, from {}",
        document.display()
    )];
    lines.extend(plan.notes.iter().cloned());

    let written = write_bundle(&bundle, &plan.items)?;
    lines.push(format!("  staged  {} ({written} file(s))", bundle.display()));
    for item in &plan.items {
        lines.push(format!("    {}/{}  {} bytes", item.application, item.path, item.bytes.len()));
    }

    // **The workstation's copy of this machine's leaf**, exactly as `w`
    // writes it, and on the same footing: reported, never fatal, because the
    // card material above is already written and reads back.
    if let Some((subject, leaf)) = &plan.workstation_leaf {
        match crate::write_server_leaf(&authority, subject, leaf) {
            Ok(paths) => {
                for path in paths {
                    lines.push(format!("  {}", path.display()));
                }
            }
            Err(why) => lines.push(format!("  WORKSTATION COPY NOT WRITTEN: {why}")),
        }
    }

    Ok(Report { path: Some(bundle), role: Some(role), lines })
}

/// Write `items` to `path` and read the file back through the parser
/// `cmd/rivet-image` will use — the check `w` makes on its own bundle.
/// Answers how many files the file holds.
///
/// Publishes through `crate::publish_bundle`, the same temp-and-rename
/// `write_staging` uses for the interview's own bundle, so a write this verb
/// cannot finish never truncates a bundle already at `path`.
fn write_bundle(path: &Path, items: &[Staged]) -> Result<usize, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::publish_bundle(path, &rivet_core::staging::render(items))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let back = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    rivet_core::staging::parse(&back)
        .map(|parsed| parsed.len())
        .map_err(|e| format!("{} WILL NOT READ BACK as a staging bundle: {e}", path.display()))
}

#[cfg(test)]
#[path = "stage_document_tests.rs"]
mod tests;
