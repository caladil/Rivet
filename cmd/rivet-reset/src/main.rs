//! **`rivet-reset`** — returning a workstation to the state a first-ever run
//! would find.
//!
//! `SPEC-05` §2, and it is step **one** of the twelve. A fleet is minted more
//! than once in a project's life, and the second minting must not inherit the
//! first's leftovers.
//!
//! # The afternoon this exists to prevent
//!
//! On 2026-09-07 a new authority was minted carrying the same subject name as
//! the one it replaced. Browsers select client certificates by matching the
//! issuer **name**, so every certificate either authority had ever issued
//! became eligible at once: the maintainer was offered **four**, none of which
//! could authenticate, and the only correct one was invisible because it had no
//! private key. Clearing that took an afternoon of hand-run `certutil` and
//! PowerShell. A device record written in August turned up in a fleet minted
//! that afternoon, because it was lying in a directory something else was
//! reading.
//!
//! **Both halves of that are this program.** The files, and the certificate
//! store — and `SPEC-05` §2 is explicit that the store *"is not optional and
//! not a follow-up chore"*.
//!
//! # What it will not do, and neither can be argued out of it
//!
//! **The signing seeds stay.** `~/.rivet/keys/*.seed` are not fleet identity:
//! every board's trust store carries their public halves, and destroying them
//! invalidates every payload ever signed. This refuses to remove them and says
//! why (`SPEC-05` §2). **No SSH key, ever**, and no board — a reset is a
//! workstation operation.
//!
//! # It lists before it removes, always
//!
//! Nothing is removed by a run that has not first printed every path and every
//! certificate, and confirmation is required (`SPEC-05` §2). Running it on an
//! already-clean workstation is a **report**, not an error.

mod authority;
mod keep;
mod store;
mod workstation;

use std::io::Write as _;
use std::path::PathBuf;

use authority::Verdict;
use workstation::Group;

/// What `--help` says.
const HELP: &str = "\
rivet-reset — return this workstation to the state a first-ever run would find.

SPEC-05 §2, and step 1 of the twelve. It removes the fleet's certificate
authority, the register, the provisioning documents composed here, and the
authority's certificates from the Windows stores — the root AND every leaf it
issued. --keep-authority is the other mode: it empties the fleet and keeps all
of the authority.

usage: rivet-reset [options]

  --ca DIR           the authority. Default ~/.rivet/ca
  --register DIR     the fleet register. Default: where rivet-fleet keeps it,
                     resolved through the same loader that tool uses, so a
                     configuration file and RIVET_FLEET_REGISTER both count
  --provision DIR    composed documents and answer notebooks. Default
                     ~/.rivet/provision
  --root FILE        the authority's root certificate, if it is not in --ca.
                     This is what the certificate store is matched against
  --verdict FILE     ask what this program would decide about ONE certificate
                     file — PEM or DER — against --root, and stop. No store is
                     opened and nothing is removed. May be given more than once
  --include-retired  also remove authorities in the store that carry this
                     authority's SUBJECT NAME and a different key, and the
                     leaves they issued. These are listed either way; without
                     the flag they are left alone
  --backup DIR       where to look for a backup of the authority — made with
                     `rivet-provision ca backup DIR`. root.crt is compared
                     byte for byte against DIR and one level of its
                     subdirectories; the report says what was found
  --proceed-without-backup
                     required to remove the authority when no backup was
                     found. Without --backup at all counts as none found.
                     Not required under --dry-run, which removes nothing
  --dry-run          print the report and stop. Removes nothing, asks nothing
  --confirm          proceed without the interactive question. The report is
                     printed first either way
  --keep-authority   start the FLEET again and KEEP the authority. See below
  --fleet DIR        the fleet repository. Default ~/.rivet/fleet.
                     --keep-authority only
  --armada DIR       the Armada Registry. Default ~/.rivet/armada.
                     --keep-authority only
  -h, --help         this text

--keep-authority — a new fleet under the same authority (SPEC-05 §2)

  The authority's root and your browser certificate stay installed, so nothing
  about the browser is redone. Every record of the old fleet's machines goes:

    the register          every device record, the fleet file, and the
                          credential store beside them (--register)
    the fleet repository  register/ and documents/, removed with git and
                          committed as one commit naming this program. A
                          repository with uncommitted changes is REFUSED
                          before anything anywhere is removed (--fleet)
    provisioning          composed documents, staging bundles and answer
                          notebooks (--provision)
    the Armada            every Fleet it lists, fleets/*.toml (--armada)

  KEPT, and named in the report: the authority directory (--ca), the Windows
  certificate stores (never opened in this mode), the fleet repository's
  manifest.toml and objects/ (the published software), the Armada's own
  armada.toml (where backups go), the signing seeds and the identities.
  --backup is not needed, because the authority is not removed.
  --include-retired, --root and --verdict are refused with it.

    rivet-reset --keep-authority --dry-run     the plan; removes nothing
    rivet-reset --keep-authority --confirm     do it without asking

WHAT IT WILL NOT REMOVE, and there is no flag for any of it:

  ~/.rivet/keys/*.seed       the signing seeds. Every board's trust store holds
                             their public halves; removing them invalidates
                             every payload this project has ever signed
  ~/.rivet/identity          the control-plane operator identity, which reaches
                             every running board
  anything under .ssh        no tool of this maintainer's touches an SSH key

CERTIFICATES ARE MATCHED ON THE AUTHORITY'S KEY, NOT ITS NAME. A re-minted
authority may carry the same subject as the one it replaced — on 2026-09-07 one
did — so a matcher that compared names would delete an authority you meant to
keep. Every certificate here is one whose signature this authority's public key
verifies.
";

/// What a run was asked to do.
struct Args {
    ca: Option<PathBuf>,
    register: Option<PathBuf>,
    provision: Option<PathBuf>,
    root: Option<PathBuf>,
    /// Certificate files to rule on and nothing else — `--verdict`.
    verdict: Vec<PathBuf>,
    include_retired: bool,
    /// Where to look for a backup of the authority — `SPEC-05` §2.
    backup: Option<PathBuf>,
    /// The one word required to remove the authority when no backup was
    /// found. `SPEC-05` §2.
    proceed_without_backup: bool,
    dry_run: bool,
    confirm: bool,
    /// Clear the fleet and keep the authority — `SPEC-05` §2, `keep.rs`.
    keep_authority: bool,
    /// The fleet repository, whose machine records `--keep-authority` clears.
    fleet: Option<PathBuf>,
    /// The Armada Registry, whose Fleets `--keep-authority` clears.
    armada: Option<PathBuf>,
}

fn main() -> std::process::ExitCode {
    let typed: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&typed) {
        Ok(Some(args)) => args,
        // `--help`, which is not a failure.
        Ok(None) => {
            print!("{HELP}");
            return std::process::ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("rivet-reset: {why}");
            eprintln!("  rivet-reset --help");
            return std::process::ExitCode::FAILURE;
        }
    };
    if args.keep_authority {
        keep::run(&args)
    } else {
        run(&args)
    }
}

/// Read the command line. `Ok(None)` is `--help`.
fn parse(typed: &[String]) -> Result<Option<Args>, String> {
    let mut args = Args {
        ca: None,
        register: None,
        provision: None,
        root: None,
        verdict: Vec::new(),
        include_retired: false,
        backup: None,
        proceed_without_backup: false,
        dry_run: false,
        confirm: false,
        keep_authority: false,
        fleet: None,
        armada: None,
    };
    let mut rest = typed.iter();
    while let Some(one) = rest.next() {
        let mut path = |what: &str| -> Result<PathBuf, String> {
            rest.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{what} needs a directory after it"))
        };
        match one.as_str() {
            "-h" | "--help" => return Ok(None),
            "--ca" => args.ca = Some(path("--ca")?),
            "--register" => args.register = Some(path("--register")?),
            "--provision" => args.provision = Some(path("--provision")?),
            "--root" => args.root = Some(path("--root")?),
            "--verdict" => args.verdict.push(path("--verdict")?),
            "--include-retired" => args.include_retired = true,
            "--backup" => args.backup = Some(path("--backup")?),
            "--proceed-without-backup" => args.proceed_without_backup = true,
            "--dry-run" => args.dry_run = true,
            "--confirm" => args.confirm = true,
            "--keep-authority" => args.keep_authority = true,
            "--fleet" => args.fleet = Some(path("--fleet")?),
            "--armada" => args.armada = Some(path("--armada")?),
            other => return Err(format!("{other:?} is not an option this takes")),
        }
    }
    // **An option that would be silently ignored is refused instead.** Each of
    // these is about only one of the two modes, and a run that dropped one
    // would do something other than what was typed.
    if args.keep_authority {
        if args.include_retired || args.root.is_some() || !args.verdict.is_empty() {
            return Err("--include-retired, --root and --verdict are about removing the \
                        authority's certificates, and --keep-authority removes none"
                .to_owned());
        }
    } else if args.fleet.is_some() || args.armada.is_some() {
        return Err("--fleet and --armada name what --keep-authority clears, and the ordinary \
                    reset does not clear either"
            .to_owned());
    }
    Ok(Some(args))
}

/// One certificate, and what this run intends to do about it.
struct Line {
    held: store::Held,
    /// Why it is going, or why it is staying.
    why: String,
    going: bool,
    /// It carries the authority's subject name and a different key — so
    /// `--include-retired` is what decides its fate, and the report says so.
    retired: bool,
}

#[allow(clippy::too_many_lines)]
fn run(args: &Args) -> std::process::ExitCode {
    println!();
    println!("rivet-reset — SPEC-05 §2. NOTHING HAS BEEN REMOVED YET.");

    // ---------------------------------------------------------------- files
    let ca = match args.ca.clone().or_else(workstation::default_ca) {
        Some(dir) => dir,
        None => {
            eprintln!(
                "rivet-reset: this host sets neither HOME nor USERPROFILE, so there is nowhere \
                 for a Rivet authority to be. Pass --ca DIR."
            );
            return std::process::ExitCode::FAILURE;
        }
    };
    let root_path = args.root.clone().unwrap_or_else(|| ca.join("root.crt"));

    // **`--verdict` answers about FILES and opens no store**, so it comes
    // before anything is surveyed. It exists because the matcher is the part of
    // this program that has to be right, and *"would this certificate go?"* is a
    // question an operator should be able to ask without running a reset to find
    // out. `tools/provision/rehearse-reset.sh` asks it of two authorities that
    // share a subject name, which is the 2026-09-07 state exactly.
    if !args.verdict.is_empty() {
        return verdicts(&args.verdict, &root_path);
    }

    let provision = args
        .provision
        .clone()
        .or_else(workstation::default_provision);
    let register = args.register.clone().or_else(workstation::default_register);

    let (authority_files, authority_errors) =
        workstation::walk(&ca, workstation::describe_authority);
    let mut groups = vec![Group {
        title: "THE AUTHORITY",
        files: authority_files,
        root: ca.clone(),
        errors: authority_errors,
    }];
    if let Some(dir) = &register {
        let (files, errors) = workstation::walk(dir, workstation::describe_register);
        groups.push(Group { title: "THE REGISTER", files, root: dir.clone(), errors });
    }
    if let Some(dir) = &provision {
        let (files, errors) = workstation::walk(dir, workstation::describe_document);
        groups.push(Group { title: "PROVISIONING DOCUMENTS", files, root: dir.clone(), errors });
    }

    // **The breadth guard runs before the report is printed**, because a plan
    // over a home directory is a page nobody reads to the end of and a
    // confirmation nobody reads is not one.
    for group in &groups {
        if let Some(why) = workstation::too_broad(&group.root) {
            eprintln!("rivet-reset: {why}, which is too broad to reset.");
            eprintln!("  Name the directory itself: --ca, --register or --provision.");
            return std::process::ExitCode::FAILURE;
        }
    }

    for group in &groups {
        println!();
        println!("  {}  {}", group.title, group.root.display());
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
    }

    // **An incomplete enumeration is refused before anything else is even
    // examined.** A traversal error (B14) means some entry was skipped
    // rather than listed, and removing against a plan that silently left
    // something out is the same shape of mistake [`workstation::hazards`]
    // already refuses for.
    let walk_errors: usize = groups.iter().map(|g| g.errors.len()).sum();
    if walk_errors > 0 {
        println!();
        println!("  REFUSED. Nothing was removed.");
        println!("      {walk_errors} traversal problem(s) above (marked NOT EXAMINED) — an");
        println!("      enumeration that skipped something is not a plan this can act on.");
        return std::process::ExitCode::FAILURE;
    }

    // --------------------------------------------------------- certificates
    let root_der = match std::fs::read_to_string(&root_path) {
        Ok(pem) => match authority::der_of_pem(&pem) {
            Ok(der) => Some(der),
            Err(why) => {
                eprintln!("rivet-reset: {}: {why}", root_path.display());
                return std::process::ExitCode::FAILURE;
            }
        },
        Err(_) => None,
    };

    let mut lines: Vec<Line> = Vec::new();
    let mut store_failed = false;
    println!();
    match &root_der {
        None => {
            println!("  THE CERTIFICATE STORE");
            println!("      {} IS NOT THERE, so no certificate can be", root_path.display());
            println!("      matched against this authority's KEY — and matching on the");
            println!("      NAME is what put four unusable certificates in front of a");
            println!("      person on 2026-09-07 (findings §3.1). Nothing in any store");
            println!("      was examined.");
            println!();
            println!("      If the authority you are clearing out is gone from disk, point");
            println!("      at a copy of its root: rivet-reset --root <root.crt>");
        }
        Some(root) => match store::survey() {
            Err(why) => {
                store_failed = true;
                println!("  THE CERTIFICATE STORE — NOT READ: {why}");
                println!("      This is reported rather than treated as an empty store: a");
                println!("      reset that said `nothing to remove` after failing to open a");
                println!("      store would be the worst available answer.");
            }
            Ok(held) => {
                lines = classify(held, root, args.include_retired);
                report_certificates(&lines, &root_path);
            }
        },
    }

    // ------------------------------------------------------------- refusals
    println!();
    println!("  LEFT ALONE — a refusal, not an omission (SPEC-05 §2)");
    let seeds = workstation::seeds();
    if seeds.is_empty() {
        println!("      the signing seeds — there are none in {}",
            workstation::keys_directory().map_or_else(
                || "~/.rivet/keys".to_owned(),
                |d| d.display().to_string()
            ));
    } else {
        for seed in &seeds {
            println!("      {}", seed.display());
        }
    }
    println!("      Every board's trust store carries their public halves. Removing one");
    println!("      invalidates every payload this project has ever signed, so there is");
    println!("      no flag for it and this program will not do it.");
    if let Some(identity) = workstation::identity_directory() {
        println!("      {}", identity.display());
        println!("      The control-plane operator identity, which reaches every running");
        println!("      board. A new fleet does not replace it.");
    }
    println!("      Any SSH key, and any board. Reset is a workstation operation.");

    // **The guard, over everything this run would actually touch.**
    let touching: Vec<PathBuf> = groups
        .iter()
        .flat_map(|g| g.files.iter().map(|f| f.path.clone()))
        .collect();
    let hazards = workstation::hazards(&touching);
    if !hazards.is_empty() {
        println!();
        println!("  REFUSED. Nothing was removed.");
        for hazard in &hazards {
            println!("      {}", hazard.path.display());
            println!("        {}", hazard.why);
        }
        return std::process::ExitCode::FAILURE;
    }

    // --------------------------------------------------------- the backup
    //
    // **Nothing before this checked.** `root.key` is about to be removed
    // `remove_file` by `remove_file`, and `root.key` is the fleet's only
    // copy of its whole trust — SPEC-05 §2. Byte-for-byte against
    // `--backup`'s directory (and one level of its subdirectories) is the
    // honest version of "is there a backup": it asks whether the exact file
    // this authority's `root.crt` already is shows up somewhere the operator
    // named, and it opens no key to do it.
    println!();
    println!("  THE BACKUP");
    let backup_found = args.backup.as_deref().and_then(|dir| {
        std::fs::read(&root_path)
            .ok()
            .and_then(|current| workstation::backup_covers(dir, &current))
    });

    // **B13: a "backup" this run is about to delete is not a backup.**
    // Checked before the match below prints it as found — `--backup`
    // naming the authority directory itself, a location nested inside any
    // removal root, or a lexical/symlink/junction alias of one all end with
    // `backup_covers` finding a byte-for-byte match, because the "backup"
    // is the very file being removed. Refused outright, the same as a
    // hazard: unlike "no backup was found", this is not something
    // `--proceed-without-backup` should be able to wave through, because
    // the operator did not choose to proceed without a backup — they were
    // about to be told, wrongly, that one existed.
    let removal_roots: Vec<PathBuf> = groups.iter().map(|g| g.root.clone()).collect();
    let unsafe_backup = backup_found.as_deref().and_then(|found| {
        let inside = workstation::backup_is_a_removal_target(found, &removal_roots)?;
        Some((found, inside))
    });
    if let Some((found, inside)) = unsafe_backup {
        println!("      {} matches, byte for byte — but it is inside {},", found.display(), inside.display());
        println!("      which this run is about to remove. A backup this reset would itself");
        println!("      delete is not a backup. Point --backup at a location this run does not");
        println!("      touch.");
        println!();
        println!("  REFUSED. Nothing was removed.");
        return std::process::ExitCode::FAILURE;
    }

    match (&args.backup, &backup_found) {
        (Some(_), Some(found)) => {
            println!("      found: {}", found.display());
        }
        (Some(dir), None) => {
            println!(
                "      NOT FOUND: {} (and one level of its subdirectories) holds no root.crt",
                dir.display()
            );
            println!("      matching this authority's, byte for byte");
        }
        (None, _) => {
            println!("      NO --backup WAS GIVEN, so none could be checked");
        }
    }
    if backup_found.is_none() {
        println!(
            "      root.key is the fleet's whole trust, and this is about to remove its only"
        );
        println!(
            "      copy. rivet-provision ca backup <dir> writes one; --backup <dir> here says"
        );
        println!("      where to look. --proceed-without-backup removes it anyway.");
    }

    // ----------------------------------------------------------- the ending
    let files: usize = groups.iter().map(|g| g.files.len()).sum();
    let certificates = lines.iter().filter(|l| l.going).count();
    println!();
    if files == 0 && certificates == 0 {
        println!(
            "  Nothing to remove. This workstation is already in the state a first-ever run \
             would find."
        );
        // **A clean workstation is a report and not an error** — `SPEC-05` §2
        // — even when a store could not be read, because the sentence above is
        // then about the files and the store said its own piece already.
        return if store_failed {
            std::process::ExitCode::FAILURE
        } else {
            std::process::ExitCode::SUCCESS
        };
    }
    println!("  {files} file(s) and {certificates} certificate(s) would be removed.");

    if args.dry_run {
        println!("  --dry-run: nothing was removed.");
        return std::process::ExitCode::SUCCESS;
    }

    // **The one extra word, and it is required before the interactive
    // question — not after.** A run that is going to be refused for want of
    // a backup should say so before asking "remove all of it?", not after.
    // Not a wall: the maintainer's view of people who skip the instructions and
    // lose their keys is a shrug (.rulings/ R-316) — the tool says it once, and
    // `--proceed-without-backup` is what lets him mean it.
    if backup_found.is_none() && !args.proceed_without_backup {
        println!();
        println!("  REFUSED. No backup of this authority was found, and root.key is about to");
        println!("  become unrecoverable. Pass --proceed-without-backup to remove it anyway.");
        return std::process::ExitCode::FAILURE;
    }

    if !args.confirm && !asked() {
        println!("  Nothing was removed.");
        return std::process::ExitCode::SUCCESS;
    }

    // ------------------------------------------------------------ doing it
    println!();
    let mut failed: Vec<String> = Vec::new();
    for group in &groups {
        if group.is_empty() {
            continue;
        }
        failed.extend(workstation::remove(group));
        // **Counted off the filesystem, not off the plan.** What was listed and
        // what went are two different numbers whenever anything is locked, and
        // reporting the first as if it were the second is how a reset comes to
        // claim a directory is clear when it is not.
        let gone = group.files.iter().filter(|f| !f.path.exists()).count();
        println!(
            "  removed {gone} of {} from {}",
            group.files.len(),
            group.root.display()
        );
    }
    let mut removed = 0_usize;
    for line in lines.iter().filter(|l| l.going) {
        match store::remove(&line.held) {
            Ok(()) => {
                removed += 1;
                println!(
                    "  removed {} {} {}",
                    line.held.store,
                    &line.held.thumbprint,
                    authority::subject_of(&line.held.der)
                );
            }
            Err(why) => failed.push(why),
        }
    }
    if certificates > 0 {
        println!("  removed {removed} of {certificates} certificate(s)");
    }

    if !failed.is_empty() {
        println!();
        println!("  SOME OF IT DID NOT GO:");
        for why in &failed {
            println!("      {why}");
        }
        return std::process::ExitCode::FAILURE;
    }
    // **A store that could not be read is a failure even when the files went.**
    // `SPEC-05` §2: the browser half *"is not optional and not a follow-up
    // chore"*, and a run that removed the authority from disk while never
    // opening a store has left the operator in the exact state §2 describes —
    // with the only file that could have identified those certificates now gone.
    if store_failed {
        println!();
        println!("  THE FILES WENT AND THE CERTIFICATE STORE WAS NOT EXAMINED.");
        println!("  SPEC-05 §2 makes the store half of a reset, not a follow-up. Keep a copy");
        println!("  of the authority's root.crt and finish with: rivet-reset --root <that file>");
        return std::process::ExitCode::FAILURE;
    }
    println!();
    println!("  Done. Restart the browser: a certificate it has already offered stays in");
    println!("  its picker until it does.");
    std::process::ExitCode::SUCCESS
}

/// **`--verdict FILE …`** — rule on certificate files and remove nothing.
///
/// # Why a flag for this
///
/// The one thing this program must not get wrong is *which certificates belong
/// to the authority being reset*, and on 2026-09-07 the thing that made that
/// hard was two authorities wearing one name. A matcher that can only be
/// exercised by running a reset against a live certificate store is one nobody
/// can check before trusting it, and *"trust it"* here means letting it delete
/// certificates.
///
/// So the same [`authority::verdict`] the store path uses is reachable from
/// files: `--root A/root.crt --verdict A/leaf.crt --verdict B/leaf.crt` answers
/// for both, and the two answers differ even though every name in them agrees.
fn verdicts(files: &[PathBuf], root_path: &std::path::Path) -> std::process::ExitCode {
    let root = match std::fs::read(root_path)
        .map_err(|e| format!("{e}"))
        .and_then(|bytes| authority::der_of_file(&bytes))
    {
        Ok(der) => der,
        Err(why) => {
            eprintln!("rivet-reset: {}: {why}", root_path.display());
            eprintln!(
                "  --verdict compares against an authority's KEY, so it needs that \
                 authority's root certificate. Name it with --root."
            );
            return std::process::ExitCode::FAILURE;
        }
    };

    println!();
    println!(
        "  matched against the KEY in {} ({})",
        root_path.display(),
        authority::subject_of(&root)
    );
    println!("  Nothing is removed by --verdict and no certificate store was opened.");
    let mut refused = false;
    for file in files {
        println!();
        println!("  {}", file.display());
        let der = match std::fs::read(file)
            .map_err(|e| format!("{e}"))
            .and_then(|bytes| authority::der_of_file(&bytes))
        {
            Ok(der) => der,
            Err(why) => {
                println!("      NOT EXAMINED: {why}");
                refused = true;
                continue;
            }
        };
        let (mark, why) = match authority::verdict(&der, &root) {
            Verdict::TheRoot => ("GOING ", "THE AUTHORITY'S OWN ROOT".to_owned()),
            Verdict::Issued => (
                "GOING ",
                "issued by this authority — its signature verifies".to_owned(),
            ),
            Verdict::SameNameOtherKey => (
                "stays ",
                "SAME NAME, DIFFERENT KEY — another authority's, and only \
                 --include-retired reaches it"
                    .to_owned(),
            ),
            Verdict::Unrelated => ("stays ", "nothing to do with this authority".to_owned()),
            Verdict::Unreadable(said) => {
                refused = true;
                ("      ", format!("NOT EXAMINED: {said}"))
            }
        };
        println!("      {mark} {why}");
        println!("      subject   {}", authority::subject_of(&der));
        println!("      issued by {}", authority::issuer_of(&der));
    }
    if refused {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

/// Decide what each certificate in the store is, and what to do about it.
///
/// **Every decision is [`authority::verdict`]'s**, which is the signature test.
/// This function only turns verdicts into sentences and applies
/// `--include-retired`.
fn classify(held: Vec<store::Held>, root: &[u8], include_retired: bool) -> Vec<Line> {
    // The retired authorities present in the store, found before anything else
    // so that their leaves can be recognised in the same pass.
    let retired_roots: Vec<Vec<u8>> = held
        .iter()
        .filter(|h| {
            authority::verdict(&h.der, root) == Verdict::SameNameOtherKey
                && authority::is_self_signed_authority(&h.der)
        })
        .map(|h| h.der.clone())
        .collect();

    let mut out = Vec::new();
    for one in held {
        let mut retired = false;
        let (why, going) = match authority::verdict(&one.der, root) {
            Verdict::TheRoot => ("THE AUTHORITY'S OWN ROOT".to_owned(), true),
            Verdict::Issued => ("issued by this authority — its signature verifies".to_owned(), true),
            Verdict::SameNameOtherKey => {
                retired = true;
                let what = if authority::is_self_signed_authority(&one.der) {
                    "a RETIRED authority: this subject name, a different key"
                } else if retired_roots
                    .iter()
                    .any(|r| authority::signed_by(&one.der, r))
                {
                    "issued by a retired authority of the same name, whose root is in the store"
                } else {
                    "carries this authority's subject name; its own authority's key is not in \
                     any store, so only the NAME says so"
                };
                (what.to_owned(), include_retired)
            }
            Verdict::Unrelated => (String::new(), false),
            Verdict::Unreadable(why) => (format!("NOT EXAMINED: {why}"), false),
        };
        out.push(Line {
            held: one,
            why,
            going,
            retired,
        });
    }
    out
}

/// Print the certificate half of the report.
fn report_certificates(lines: &[Line], root_path: &std::path::Path) {
    println!("  THE CERTIFICATE STORE — matched on the KEY in {}", root_path.display());
    let interesting: Vec<&Line> = lines.iter().filter(|l| !l.why.is_empty()).collect();
    let unrelated = lines.len() - interesting.len();

    if interesting.is_empty() {
        println!("      nothing in My, Root or CA belongs to this authority");
    }
    for line in &interesting {
        let key = if line.held.has_private_key { "  [has a private key]" } else { "" };
        println!(
            "      {} {:<8} {:.16}…{key}",
            if line.going { "GOING " } else { "stays " },
            line.held.store.to_string(),
            line.held.thumbprint
        );
        println!(
            "               {} — {}",
            authority::subject_of(&line.held.der),
            line.why
        );
        // **The issuer, printed for every line.** It is the field a browser
        // matches on, and it is therefore the field that makes two certificates
        // look identical to the person choosing between them — so a report that
        // is about telling them apart has to show it beside the verdict that
        // did.
        println!("               issued by {}", authority::issuer_of(&line.held.der));
        if line.retired && !line.going {
            println!("               pass --include-retired to remove it too");
        }
    }
    println!(
        "      {unrelated} other certificate(s) in the three stores are not this authority's \
         and are NOT touched"
    );
}

/// The question, asked once.
///
/// **A closed stdin is not consent.** `Ok(0)` is EOF, and the default is no.
fn asked() -> bool {
    print!("  remove all of it? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let read = std::io::stdin().read_line(&mut answer);
    matches!(read, Ok(n) if n > 0) && answer.trim().eq_ignore_ascii_case("y")
}

/// A path with its group's directory taken off the front, for the report.
///
/// The heading already carries the directory, and a column of forty repeated
/// characters is a column nobody reads across.
fn shortened(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_options_are_read_the_way_they_are_typed() {
        let args = parse(&[
            "--ca".to_owned(),
            "/x/ca".to_owned(),
            "--dry-run".to_owned(),
            "--include-retired".to_owned(),
        ])
        .expect("parses")
        .expect("not --help");
        assert_eq!(args.ca, Some(PathBuf::from("/x/ca")));
        assert!(args.dry_run);
        assert!(args.include_retired);
        assert!(!args.confirm);
    }

    /// **`--backup` and `--proceed-without-backup` read the way they are
    /// typed, and neither is on by default.** A default of `true` for the
    /// override would be the one word required to proceed no longer costing
    /// anything to skip — `SPEC-05` §2.
    #[test]
    fn the_backup_options_are_read_the_way_they_are_typed() {
        let neither = parse(&["--dry-run".to_owned()])
            .expect("parses")
            .expect("not --help");
        assert_eq!(neither.backup, None);
        assert!(!neither.proceed_without_backup);

        let both = parse(&[
            "--backup".to_owned(),
            "/x/backup".to_owned(),
            "--proceed-without-backup".to_owned(),
        ])
        .expect("parses")
        .expect("not --help");
        assert_eq!(both.backup, Some(PathBuf::from("/x/backup")));
        assert!(both.proceed_without_backup);
    }

    /// **An unknown option is refused by name**, and a flag that needs a value
    /// and did not get one is refused rather than silently taking the next
    /// flag as a directory. `--ca --dry-run` would otherwise compose a plan
    /// over a directory called `--dry-run`, which is a thing this program
    /// would then offer to remove.
    #[test]
    fn a_malformed_command_line_is_refused() {
        assert!(parse(&["--wat".to_owned()]).is_err());
        assert!(parse(&["--ca".to_owned()]).is_err());
        assert!(parse(&["--backup".to_owned()]).is_err(), "--backup needs a directory too");
    }

    /// **`--keep-authority` reads its two directories, and refuses the options
    /// that only mean something when the authority is being removed**, rather
    /// than ignoring them — a run that dropped `--include-retired` would do
    /// something other than what was typed.
    #[test]
    fn keep_authority_takes_its_directories_and_refuses_the_authority_options() {
        let args = parse(&[
            "--keep-authority".to_owned(),
            "--fleet".to_owned(),
            "/x/fleet".to_owned(),
            "--armada".to_owned(),
            "/x/armada".to_owned(),
            "--dry-run".to_owned(),
        ])
        .expect("parses")
        .expect("not --help");
        assert!(args.keep_authority);
        assert_eq!(args.fleet, Some(PathBuf::from("/x/fleet")));
        assert_eq!(args.armada, Some(PathBuf::from("/x/armada")));

        for authority_only in [
            vec!["--include-retired"],
            vec!["--root", "/x/root.crt"],
            vec!["--verdict", "/x/leaf.crt"],
        ] {
            let mut typed = vec!["--keep-authority".to_owned()];
            typed.extend(authority_only.iter().map(|s| (*s).to_owned()));
            assert!(parse(&typed).is_err(), "{typed:?} was accepted");
        }
        assert!(
            parse(&["--keep-authority".to_owned(), "--backup".to_owned(), "/b".to_owned()]).is_ok(),
            "--backup is unneeded in this mode, not wrong"
        );
    }

    /// **`--fleet` and `--armada` without `--keep-authority` are refused**:
    /// the ordinary reset clears neither, so accepting them would promise a
    /// removal that never happens.
    #[test]
    fn the_keep_authority_directories_are_refused_without_it() {
        assert!(parse(&["--fleet".to_owned(), "/x/fleet".to_owned()]).is_err());
        assert!(parse(&["--armada".to_owned(), "/x/armada".to_owned()]).is_err());
        assert!(parse(&["--keep-authority".to_owned(), "--fleet".to_owned()]).is_err());
    }

    #[test]
    fn help_is_not_a_failure() {
        assert!(parse(&["--help".to_owned()]).expect("parses").is_none());
    }

    /// **`--verdict` accumulates**, because the question it answers is about
    /// TWO certificates: the one that must go and the identically-named one
    /// that must not.
    #[test]
    fn verdict_can_be_asked_about_more_than_one_certificate() {
        let args = parse(&[
            "--root".to_owned(),
            "/a/root.crt".to_owned(),
            "--verdict".to_owned(),
            "/a/leaf.crt".to_owned(),
            "--verdict".to_owned(),
            "/b/leaf.crt".to_owned(),
        ])
        .expect("parses")
        .expect("not --help");
        assert_eq!(
            args.verdict,
            vec![PathBuf::from("/a/leaf.crt"), PathBuf::from("/b/leaf.crt")]
        );
        assert!(!args.confirm, "asking a question is not consent to remove");
    }

    /// **The help text names the refusal**, because a person looking for a way
    /// to remove the seeds should find the reason rather than nothing.
    #[test]
    fn the_help_says_what_will_not_be_removed() {
        assert!(HELP.contains("seed"), "{HELP}");
        assert!(HELP.contains("SSH"), "{HELP}");
        assert!(HELP.contains("KEY, NOT ITS NAME"), "{HELP}");
    }

    /// The report's file lines are relative to the heading, which is what makes
    /// the columns readable.
    #[test]
    fn a_path_is_shown_under_its_own_directory() {
        let root = PathBuf::from("/r");
        assert_eq!(shortened(&root.join("devices").join("a.toml"), &root),
                   PathBuf::from("devices").join("a.toml").display().to_string());
    }
}
