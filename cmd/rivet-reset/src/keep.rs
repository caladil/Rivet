//! **`--keep-authority`** — the fleet starts empty and the authority stays.
//! `SPEC-05` §2.
//!
//! # What it is for
//!
//! Rebuilding a fleet while keeping the certificate authority, so that the
//! authority's root and the operator's browser certificate stay installed in
//! the Windows stores and nothing about the browser has to be redone. Every
//! record of the old fleet's machines goes; everything that is not a record of
//! a machine stays.
//!
//! | removed | where | how |
//! | :-- | :-- | :-- |
//! | the operator register | `rivet-fleet`'s register directory | file by file, as the ordinary reset does |
//! | the fleet repository's machine records | `register/` and `documents/` in `~/.rivet/fleet` | `git rm`, then one commit naming this program |
//! | provisioning notebooks | `~/.rivet/provision` | file by file |
//! | the Armada's Fleets | `~/.rivet/armada/fleets` | file by file |
//!
//! **Kept, and named in the report:** the authority directory, the Windows
//! certificate stores (never opened by this mode), the fleet repository's
//! `manifest.toml` and `objects/` (the published software, which a fresh fleet
//! installs exactly as the old one did), the Armada's own settings file (where
//! backups go and how many to keep, which describe the operator and not a
//! fleet), the signing seeds, and the operator identities.
//!
//! # Why the fleet repository is cleared through git
//!
//! It is a git repository whose purpose is attribution (`ADR-052` §5). A
//! removal made behind git's back would leave a dirty tree the next
//! `cargo rivet provision` commits through, attributing the old fleet's
//! disappearance to a new machine's document. So the removal is its own
//! commit, and a tree that is already dirty is refused before anything
//! anywhere is removed: a reset has no business committing somebody else's
//! work.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::workstation::{self, Group};
use crate::{Args, asked, shortened};

/// The name this mode commits under, so `git log` says what emptied the
/// fleet repository.
const COMMITTED_BY: &str = "rivet-reset --keep-authority";

/// How a location's files are removed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Removal {
    /// `workstation::remove`: each file, then each directory it emptied.
    Files,
    /// `rivetos_fleet_repo::clear_records`: `git rm` and one commit.
    Git,
}

/// One location this mode empties.
struct Place {
    group: Group,
    removal: Removal,
}

/// One thing this mode keeps, for the report.
struct Kept {
    what: &'static str,
    path: Option<PathBuf>,
    /// What is there, or why it stays.
    note: String,
}

/// Run the mode. The caller has already refused the options that only make
/// sense when the authority is being removed.
#[allow(clippy::too_many_lines)]
pub fn run(args: &Args) -> ExitCode {
    println!();
    println!("rivet-reset --keep-authority — SPEC-05 §2. NOTHING HAS BEEN REMOVED YET.");
    println!();
    println!("  The fleet starts empty and the authority stays. The certificate authority,");
    println!("  the Windows certificate stores, the signing seeds and the operator identities");
    println!("  are not touched.");

    let Some(ca) = args.ca.clone().or_else(workstation::default_ca) else {
        eprintln!(
            "rivet-reset: this host sets neither HOME nor USERPROFILE, so there is nowhere for \
             a Rivet authority to be. Pass --ca DIR, and the other directories by name."
        );
        return ExitCode::FAILURE;
    };
    let register = args.register.clone().or_else(workstation::default_register);
    let provision = args.provision.clone().or_else(workstation::default_provision);
    let fleet = args.fleet.clone().or_else(workstation::default_fleet);
    let armada = args.armada.clone().or_else(workstation::default_armada);

    // **The breadth guard, on every directory named or defaulted**, before
    // anything is walked. `--fleet ~/.rivet` would otherwise walk the whole
    // tree the seeds live in.
    for dir in [&register, &provision, &fleet, &armada].into_iter().flatten() {
        if let Some(why) = workstation::too_broad(dir) {
            eprintln!("rivet-reset: {why}, which is too broad to reset.");
            eprintln!("  Name the directory itself: --register, --provision, --fleet or --armada.");
            return ExitCode::FAILURE;
        }
    }

    // ---------------------------------------------------------------- plan
    let mut places: Vec<Place> = Vec::new();
    if let Some(dir) = &register {
        let (files, errors) = workstation::walk(dir, workstation::describe_register);
        places.push(Place {
            group: Group { title: "THE OPERATOR REGISTER", files, root: dir.clone(), errors },
            removal: Removal::Files,
        });
    }
    if let Some(dir) = &fleet {
        let (mut files, mut errors) = workstation::walk(
            &dir.join(rivetos_fleet_repo::REGISTER_DIR),
            workstation::describe_fleet_record,
        );
        let (more_files, more_errors) = workstation::walk(
            &dir.join(rivetos_fleet_repo::DOCUMENTS_DIR),
            workstation::describe_fleet_record,
        );
        files.extend(more_files);
        errors.extend(more_errors);
        places.push(Place {
            group: Group {
                title: "THE FLEET REPOSITORY'S MACHINE RECORDS",
                files,
                root: dir.clone(),
                errors,
            },
            removal: Removal::Git,
        });
    }
    if let Some(dir) = &provision {
        let (files, errors) = workstation::walk(dir, workstation::describe_document);
        places.push(Place {
            group: Group {
                title: "PROVISIONING NOTEBOOKS AND DOCUMENTS",
                files,
                root: dir.clone(),
                errors,
            },
            removal: Removal::Files,
        });
    }
    if let Some(dir) = &armada {
        let fleets = dir.join(rivetos_register::armada::FLEETS_DIR);
        let (files, errors) = workstation::walk(&fleets, workstation::describe_armada);
        places.push(Place {
            group: Group { title: "THE ARMADA'S FLEETS", files, root: fleets, errors },
            removal: Removal::Files,
        });
    }

    // ------------------------------------------------------------ removed
    println!();
    println!("  WOULD BE REMOVED");
    for place in &places {
        let group = &place.group;
        println!();
        println!(
            "  {}  {}  — {} file(s)",
            group.title,
            group.root.display(),
            group.files.len()
        );
        if group.is_empty() && group.errors.is_empty() {
            println!("      nothing here — already clean");
            continue;
        }
        for file in &group.files {
            println!(
                "      {:<38} {:>8}  {}",
                shortened(&file.path, &group.root),
                file.bytes,
                file.note
            );
        }
        for error in &group.errors {
            println!("      NOT EXAMINED: {error}");
        }
        if place.removal == Removal::Git {
            println!("      removed with git rm, and committed as one commit whose subject begins");
            println!(
                "      \"{COMMITTED_BY} removed the fleet's {} machine record(s)\"",
                group.files.len()
            );
        }
    }

    // **An incomplete enumeration is refused before anything else is even
    // examined** — the same reasoning as the ordinary reset: a traversal
    // error (B14) means the plan above is not the whole truth.
    let walk_errors: usize = places.iter().map(|p| p.group.errors.len()).sum();
    if walk_errors > 0 {
        println!();
        println!("  REFUSED. Nothing was removed.");
        println!("      {walk_errors} traversal problem(s) above (marked NOT EXAMINED) — an");
        println!("      enumeration that skipped something is not a plan this can act on.");
        return ExitCode::FAILURE;
    }

    // --------------------------------------------------------------- kept
    let kept = kept(&ca, fleet.as_ref(), armada.as_ref());
    println!();
    println!("  KEPT — named, and not touched");
    for one in &kept {
        println!();
        match &one.path {
            Some(path) => println!("  {}  {}", one.what, path.display()),
            None => println!("  {}", one.what),
        }
        println!("      {}", one.note);
    }
    if args.backup.is_some() || args.proceed_without_backup {
        println!();
        println!("  --backup and --proceed-without-backup are not needed here: this mode does not");
        println!("  remove the authority, so there is nothing a backup would have to cover.");
    }

    // ------------------------------------------------------------ refusals
    let touching: Vec<PathBuf> = places
        .iter()
        .flat_map(|p| p.group.files.iter().map(|f| f.path.clone()))
        .collect();
    let mut hazards = workstation::hazards(&touching);
    hazards.extend(workstation::inside_kept(
        &touching,
        &kept_roots(&ca, fleet.as_ref(), armada.as_ref()),
    ));
    if !hazards.is_empty() {
        println!();
        println!("  REFUSED. Nothing was removed.");
        for hazard in &hazards {
            println!("      {}", hazard.path.display());
            println!("        {}", hazard.why);
        }
        return ExitCode::FAILURE;
    }

    // **The fleet repository must be clean before ANYTHING is removed**, not
    // only before its own commit. A refusal after the register had gone would
    // leave a workstation half-reset, in a state nobody described.
    let records = places
        .iter()
        .find(|p| p.removal == Removal::Git && !p.group.is_empty());
    if let Some(place) = records {
        let dir = &place.group.root;
        let dirty = match rivetos_fleet_repo::uncommitted(dir) {
            Ok(None) => None,
            Ok(Some(lines)) => Some(format!(
                "it has uncommitted changes, and a reset will not commit somebody else's work.\n\
                 Commit or remove them, then run this again:\n{lines}"
            )),
            Err(why) => Some(format!(
                "{why}. Its machine records are cleared through git, so that the removal is a \
                 commit rather than a dirty tree the next provisioning run commits through."
            )),
        };
        if let Some(why) = dirty {
            println!();
            println!("  REFUSED. Nothing was removed. The fleet repository {}:", dir.display());
            for line in why.lines() {
                println!("      {line}");
            }
            return ExitCode::FAILURE;
        }
    }

    // ------------------------------------------------------------- ending
    let files: usize = places.iter().map(|p| p.group.files.len()).sum();
    let locations = places.iter().filter(|p| !p.group.is_empty()).count();
    println!();
    if files == 0 {
        println!("  Nothing to remove. The fleet is already empty, and the authority is kept.");
        return ExitCode::SUCCESS;
    }
    println!("  {files} file(s) from {locations} location(s) would be removed.");
    if args.dry_run {
        println!("  --dry-run: nothing was removed.");
        return ExitCode::SUCCESS;
    }
    if !args.confirm && !asked() {
        println!("  Nothing was removed.");
        return ExitCode::SUCCESS;
    }

    // ------------------------------------------------------------ doing it
    //
    // **The git step first**, because it is the one that can still refuse —
    // a tree dirtied since the check above, a `.seed` somebody dropped in. A
    // refusal there leaves every location as it was.
    println!();
    let mut failed: Vec<String> = Vec::new();
    for place in places.iter().filter(|p| p.removal == Removal::Git && !p.group.is_empty()) {
        match rivetos_fleet_repo::clear_records(&place.group.root, COMMITTED_BY) {
            Ok(Some(subject)) => println!("  committed in {}: {subject}", place.group.root.display()),
            Ok(None) => println!("  {}: nothing to commit", place.group.root.display()),
            Err(why) => {
                println!();
                println!("  STOPPED. The fleet repository refused, and nothing else was removed:");
                for line in why.lines() {
                    println!("      {line}");
                }
                return ExitCode::FAILURE;
            }
        }
        report_gone(&place.group);
    }
    for place in places.iter().filter(|p| p.removal == Removal::Files && !p.group.is_empty()) {
        failed.extend(workstation::remove(&place.group));
        report_gone(&place.group);
    }

    // Anything the git step left on disk — a file a global ignore rule hid
    // from `git status`, say — is a record that did not go.
    for place in places.iter().filter(|p| p.removal == Removal::Git) {
        for file in place.group.files.iter().filter(|f| f.path.exists()) {
            failed.push(format!("{}: still there after the commit", file.path.display()));
        }
    }

    if !failed.is_empty() {
        println!();
        println!("  SOME OF IT DID NOT GO:");
        for why in &failed {
            println!("      {why}");
        }
        return ExitCode::FAILURE;
    }
    println!();
    println!("  Done. The fleet is empty and the authority is kept. No certificate store was");
    println!("  opened, so whatever the browser had installed is installed still. A new fleet");
    println!("  server is provisioned next, and it will find the authority already there.");
    ExitCode::SUCCESS
}

/// **Counted off the filesystem, not off the plan** — the ordinary reset's
/// rule, for its reason.
fn report_gone(group: &Group) {
    let gone = group.files.iter().filter(|f| !f.path.exists()).count();
    println!(
        "  removed {gone} of {} from {}",
        group.files.len(),
        group.root.display()
    );
}

/// What the report names as kept.
fn kept(ca: &std::path::Path, fleet: Option<&PathBuf>, armada: Option<&PathBuf>) -> Vec<Kept> {
    let count = |dir: &std::path::Path| workstation::walk(dir, |_| "").0.len();
    let mut out = vec![
        Kept {
            what: "THE CERTIFICATE AUTHORITY",
            path: Some(ca.to_path_buf()),
            note: if ca.is_dir() {
                format!("{} file(s), none of them touched", count(ca))
            } else {
                "not there — nothing to keep, and nothing here creates one".to_owned()
            },
        },
        Kept {
            what: "THE WINDOWS CERTIFICATE STORES",
            path: None,
            note: "not opened by this mode, so nothing installed in them changes".to_owned(),
        },
    ];
    if let Some(fleet) = fleet {
        let manifest = fleet.join("manifest.toml");
        out.push(Kept {
            what: "THE FLEET REPOSITORY'S MANIFEST",
            path: Some(manifest.clone()),
            note: if manifest.is_file() {
                "the published artifacts, which a fresh fleet installs as the old one did"
                    .to_owned()
            } else {
                "not there".to_owned()
            },
        });
        let objects = fleet.join("objects");
        out.push(Kept {
            what: "THE FLEET REPOSITORY'S OBJECTS",
            path: Some(objects.clone()),
            note: if objects.is_dir() {
                format!("{} file(s): the signed artifacts themselves", count(&objects))
            } else {
                "not there".to_owned()
            },
        });
    }
    if let Some(armada) = armada {
        let settings = rivetos_register::armada::store::settings_path(armada);
        out.push(Kept {
            what: "THE ARMADA'S SETTINGS",
            path: Some(settings.clone()),
            note: if settings.is_file() {
                "where backups go and how many to keep: the operator's, not a fleet's".to_owned()
            } else {
                "not there".to_owned()
            },
        });
    }
    let seeds = workstation::seeds();
    out.push(Kept {
        what: "THE SIGNING SEEDS",
        path: workstation::keys_directory(),
        note: if seeds.is_empty() {
            "none here".to_owned()
        } else {
            seeds
                .iter()
                .filter_map(|s| s.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect::<Vec<_>>()
                .join(", ")
        },
    });
    out.push(Kept {
        what: "THE OPERATOR IDENTITIES",
        path: workstation::identity_directory(),
        note: "your control-plane identity and every machine identity minted here".to_owned(),
    });
    out.push(Kept {
        what: "ANY SSH KEY, AND ANY BOARD",
        path: None,
        note: "reset is a workstation operation".to_owned(),
    });
    out
}

/// The roots [`workstation::inside_kept`] refuses to reach into.
fn kept_roots(
    ca: &std::path::Path,
    fleet: Option<&PathBuf>,
    armada: Option<&PathBuf>,
) -> Vec<(PathBuf, &'static str)> {
    let mut out = vec![(
        ca.to_path_buf(),
        "inside the certificate authority, which --keep-authority keeps",
    )];
    if let Some(fleet) = fleet {
        out.push((
            fleet.join("manifest.toml"),
            "the fleet repository's manifest: published software, which is kept",
        ));
        out.push((
            fleet.join("objects"),
            "the fleet repository's objects: published software, which is kept",
        ));
        out.push((fleet.join(".git"), "inside the fleet repository's own .git"));
    }
    if let Some(armada) = armada {
        out.push((
            rivetos_register::armada::store::settings_path(armada),
            "the Armada's settings, which are kept",
        ));
    }
    out
}
