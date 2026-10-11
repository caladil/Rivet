//! `cargo rivet recover` — **a host rebuilt from what has already been taken.**
//! `ADR-063` §7, `.rulings/` R-235.
//!
//! A recovery draws on three things and takes nothing new:
//!
//! | Part | From |
//! | :-- | :-- |
//! | The desired-state document, and the applications it installs, by digest | the fleet repository (`documents/`, `manifest.toml`, `objects/`) |
//! | The pin the host answers to | the fleet repository's register, or `--pin` |
//! | `/state/secrets` | a `cargo rivet backup-secrets` directory, passphrase-sealed |
//! | Each application's data | a folder of `cargo rivet backup` output, or Rivet Monitor's backup folder |
//!
//! # Four steps, and the first is never an agent's
//!
//! 1. **The card**, written by `cargo rivet provision --existing`'s own
//!    machinery: the recorded document, the recorded identity, the same pin.
//! 2. **The secrets**, after the first boot: `restore-secrets`, with the
//!    operator typing the passphrase.
//! 3. **The applications**, each installed by the digest the document
//!    resolves to — `cargo rivet install --artifact`'s own path — then one
//!    converge so the supervisor starts what the document enables.
//! 4. **The data**, each application's file sent to the `restore-path` it
//!    declares — `cargo rivet restore`'s own path.
//!
//! **The card step and the rest are separate invocations.** Without
//! `--after-boot`, a confirmed run writes the card and stops, naming the
//! command to run once the machine has booted; it refuses without a terminal,
//! because a card is written by a person. With `--after-boot` it runs steps 2
//! to 4 in order and stops at the first refusal, so an agent proves those
//! steps against a machine that is already running.
//!
//! # The plan reads everything and connects to nothing
//!
//! Without `confirm` nothing is written and no machine is reached. The plan
//! refuses, before anything is written:
//!
//! * a host with no recorded document in the fleet repository, or a document
//!   recorded under its name that names another host;
//! * a host with no pin — neither in the fleet repository's register nor
//!   given — and a `--pin` that is not the one the register records;
//! * an application the document installs that the repository cannot resolve
//!   to one artifact, or whose object is absent or does not hash to its digest;
//! * a secrets directory that holds no sealed backup;
//! * application data for an application the document does not install, or a
//!   directory of it holding no file;
//! * a `--replace` naming an application with no data in the backup.
//!
//! Which board a secrets backup belongs to is sealed inside it, so it is
//! checked once the passphrase is typed: before the card is written, and again
//! by `restore-secrets` before any secret is sent.
//!
//! # A recovery that writes over data asks first, every time
//!
//! Before any application's data is sent, each application is asked for what
//! it holds now, through the same backup route `cargo rivet backup` reads. An
//! application holding exactly the backup's bytes is skipped. One answering
//! with no data at all is restored. **Anything else — different bytes, an
//! error, no answer — is refused unless `--replace <application>` names it**,
//! and the refusal lists every such application before ANY of them is sent
//! anything. A freshly installed application that has written an empty
//! database therefore needs `--replace` too: nothing here can tell an empty
//! database from a real one, so the operator says which it is.
//!
//! # What a backup does not hold
//!
//! Per-machine material in an application's data directory — the TLS pair an
//! application is handed from the fleet's authority (`ADR-055` §3) — is in no
//! backup, and this verb does not stage it: the commands that do read the
//! fleet's private authority, which is the operator's to hand over. An
//! application whose manifest declares material, enabled by the recovered
//! document, is refused at step 3 by `cargo rivet install`'s own rule until
//! its material is staged, and the refusal names the commands. One that
//! needs material without declaring it installs, cannot start, and does not
//! answer its backup route, so step 4 refuses it. Either way the operator
//! stages the material and runs `--after-boot` again. Every step is safe to
//! repeat: a secret already on the machine is skipped, an artifact it already
//! runs is not installed again, and data it already holds is not sent.

use std::path::{Path, PathBuf};

use rivet_core::package::port::Digest as _;
use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};

use crate::backup::ReachesApplications as _;

/// How this verb names itself in a prompt and in the commands it prints.
const VERB: &str = "cargo rivet recover";

pub fn usage() -> String {
    format!(
        "usage: cargo rivet recover HOSTNAME --from DIR [--secrets DIR] [--at DIR]\n\
         \x20                          [--image PATH-OR-DIGEST] [--device PATH]\n\
         \x20                          [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
         \x20                          [--after-boot] [--replace-secrets] [--replace APP]... [confirm]\n\
         \n\
         Rebuilds HOSTNAME from what has already been taken (ADR-063 section 7, R-235):\n\
         its document and the applications it installs, by digest, from the fleet\n\
         repository; its secrets from a `cargo rivet backup-secrets` directory; each\n\
         application's data from a folder of `cargo rivet backup` output or Rivet\n\
         Monitor's backup folder.\n\
         \n\
         \x20 1  the card       `cargo rivet provision --existing HOSTNAME`'s own machinery:\n\
         \x20                   the recorded document, the recorded identity, the same pin\n\
         \x20 2  the secrets    after the first boot, `restore-secrets`, the passphrase typed\n\
         \x20 3  applications   each installed by its digest, then one converge\n\
         \x20 4  the data       each application's file sent to its own restore route\n\
         \n\
         WITHOUT `confirm` IT ONLY PLANS. It reads and names everything it would use,\n\
         connects to nothing, and refuses what is missing or mismatched: a host the\n\
         fleet repository does not record, an artifact its document names that the\n\
         repository lacks, a pin the register disagrees with, data for an application\n\
         the document does not install.\n\
         \n\
         WITH `confirm` AND NO --after-boot IT WRITES THE CARD (step 1) AND STOPS. A\n\
         card is written by a person: this refuses without a terminal. With --secrets,\n\
         the passphrase is asked for first, and a backup of another board is refused\n\
         before anything is written. Boot the machine, then run the command it prints.\n\
         \n\
         WITH `confirm` AND --after-boot IT RUNS STEPS 2 TO 4, in that order, against\n\
         the running machine, and stops at the first refusal. Every step is safe to\n\
         run again: what the machine already holds is skipped.\n\
         \n\
         A RECOVERY THAT WRITES OVER DATA ASKS FIRST, EVERY TIME. Each application is\n\
         asked what it holds before anything is sent. The same bytes as the backup:\n\
         skipped. No data: restored. Anything else, including no answer, is refused\n\
         unless --replace names that application, and nothing is sent to any\n\
         application until every one of them is settled.\n\
         \n\
         recover options:\n\
         \x20 HOSTNAME           the machine, as the fleet repository records it.\n\
         \x20 --from DIR         REQUIRED. Application data: DIR/<application>/<file>, as\n\
         \x20                    `cargo rivet backup --into DIR` writes it, or\n\
         \x20                    DIR/HOSTNAME/<application>/<files>, as Rivet Monitor keeps\n\
         \x20                    them. Where a directory holds several files, the newest\n\
         \x20                    is used, and the plan names it.\n\
         \x20 --secrets DIR      the directory `cargo rivet backup-secrets` wrote. Without\n\
         \x20                    it, step 2 is skipped and the machine keeps the secrets\n\
         \x20                    its card gave it.\n\
         \x20 --at DIR           the fleet repository. Default: ~/.rivet/fleet.\n\
         \x20 --image, --device  passed to `provision --existing` for the card step.\n\
         \x20 --host HOST:PORT   where the machine's control plane listens. Default: the\n\
         \x20                    register's record, or <hostname>.local.\n\
         {IDENTITY_AND_PIN}\
         \x20                    Default identity: ~/.rivet/identity/operator. Default\n\
         \x20                    pin: the register's record; a --pin that disagrees with\n\
         \x20                    it is refused, because a recovery keeps the pin.\n\
         \x20 --after-boot       steps 2 to 4, against the running machine.\n\
         \x20 --replace-secrets  let step 2 replace a secret whose bytes differ.\n\
         \x20 --replace APP      let step 4 replace APP's data, whatever it holds now.\n\
         \x20                    Repeat it for each application.\n\
         \x20 confirm            the LAST word. Without it, the plan.\n\
         \n\
         NOT IN ANY BACKUP: per-machine material an application is handed from the\n\
         fleet's authority (its TLS pair), and this does not stage it. An application\n\
         declaring material is refused at step 3, naming the commands that stage it;\n\
         one that needs it undeclared cannot answer, and is refused at step 4. Stage\n\
         it, then run --after-boot again.\n"
    )
}

/// What `recover` was asked to do.
#[derive(Debug, Default)]
pub struct RecoverArgs {
    pub host: String,
    pub from: PathBuf,
    pub secrets: Option<PathBuf>,
    pub at: Option<PathBuf>,
    pub endpoint: Endpoint,
    pub image: Option<String>,
    pub device: Option<String>,
    pub after_boot: bool,
    pub replace_secrets: bool,
    pub replace: Vec<String>,
    pub confirm: bool,
}

/// # Errors
///
/// No hostname, a flag with no value, a word this verb does not take, a
/// missing `--from`, `--image` or `--device` beside `--after-boot`, or a
/// `confirm` that is not the last word.
pub fn parse(args: &[String]) -> Result<RecoverArgs, String> {
    let mut parsed = RecoverArgs::default();
    let Some(host) = args.first().filter(|w| !w.starts_with('-') && *w != "confirm") else {
        return Err(format!(
            "the first word is the HOSTNAME to recover, as the fleet repository records it.\n\n{}",
            usage()
        ));
    };
    parsed.host.clone_from(host);
    let mut from = None;
    let mut index = 1;
    while index < args.len() {
        if let Some(next) = parsed.endpoint.read(args, index)? {
            index = next;
            continue;
        }
        let value = |flag: &str| -> Result<String, String> {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match args[index].as_str() {
            "--from" => {
                from = Some(PathBuf::from(value("--from")?));
                index += 2;
            }
            "--secrets" => {
                parsed.secrets = Some(PathBuf::from(value("--secrets")?));
                index += 2;
            }
            "--at" => {
                parsed.at = Some(PathBuf::from(value("--at")?));
                index += 2;
            }
            "--image" => {
                parsed.image = Some(value("--image")?);
                index += 2;
            }
            "--device" => {
                parsed.device = Some(value("--device")?);
                index += 2;
            }
            "--replace" => {
                let named = value("--replace")?;
                if !parsed.replace.contains(&named) {
                    parsed.replace.push(named);
                }
                index += 2;
            }
            "--after-boot" => {
                parsed.after_boot = true;
                index += 1;
            }
            "--replace-secrets" => {
                parsed.replace_secrets = true;
                index += 1;
            }
            "confirm" if index + 1 == args.len() => {
                parsed.confirm = true;
                index += 1;
            }
            "confirm" => {
                return Err("`confirm` must be the LAST word, so that it confirms the whole command \
                            rather than whatever happens to follow it (ADR-012 §7)"
                    .to_string());
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option `recover` takes. There is no --passphrase flag \
                     and no environment variable: a secrets backup's passphrase is typed.\n\n{}",
                    usage()
                ));
            }
        }
    }
    parsed.from = from.ok_or_else(|| {
        format!(
            "--from is required: the folder of application backups to recover from. A host \
             with no application data is recovered from an empty directory, named on purpose.\n\n{}",
            usage()
        )
    })?;
    if parsed.after_boot && (parsed.image.is_some() || parsed.device.is_some()) {
        return Err(
            "--image and --device choose the card, and --after-boot runs against a machine \
             whose card is already written. Pass one or the other."
                .to_string(),
        );
    }
    Ok(parsed)
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// One application the document installs, resolved to the artifact that
/// installs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Application {
    pub product: String,
    pub version: String,
    pub digest: String,
}

/// One application's data, as the backup folder holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Data {
    pub application: String,
    pub file: PathBuf,
    /// Other files beside it that were not chosen, for the plan to say so.
    pub passed_over: usize,
}

/// Everything a recovery will use, read and checked.
#[derive(Debug)]
pub(crate) struct Plan {
    pub host: String,
    pub root: PathBuf,
    pub document: PathBuf,
    /// Whether the fleet repository's register holds a record for the host.
    pub recorded: bool,
    /// The pin the recovery binds to, as lower-case hex.
    pub pin: String,
    pub applications: Vec<Application>,
    /// The directory holding the sealed secrets backup.
    pub secrets: Option<PathBuf>,
    /// Where application data was read from.
    pub data_dir: PathBuf,
    pub data: Vec<Data>,
}

/// The hostname as `rivetos_fleet_repo` files a document under it.
fn safe_name(name: &str) -> String {
    let safe: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    if safe.is_empty() { "unnamed".to_owned() } else { safe }
}

/// Every hostname a document is recorded for, for a refusal to list.
fn recorded_hosts(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join(rivetos_fleet_repo::DOCUMENTS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(rivetos_fleet_repo::DOCUMENT_SUFFIX))
                .map(str::to_owned)
        })
        .collect();
    out.sort();
    out
}

/// A pin as one lower-case 64-hex key.
fn one_pin(text: &str) -> Result<String, String> {
    let pins = rivetos_tls::parse_pins(text).map_err(|e| format!("--pin: {e}"))?;
    match pins.as_slice() {
        [one] => Ok(rivetos_tls::encode_hex(one)),
        _ => Err("--pin must name exactly one key: a recovery binds to one machine".to_string()),
    }
}

/// What every refusal of the plan ends with.
const NOTHING: &str = "NOTHING WAS WRITTEN, and no machine was reached.";

/// **Read and check everything the recovery will use.** Reads files only.
///
/// # Errors
///
/// Every refusal in this module's documentation that needs no passphrase and
/// no machine.
pub(crate) fn plan(args: &RecoverArgs, root: &Path) -> Result<Plan, String> {
    let host = &args.host;

    // --- the document ---------------------------------------------------------
    let document = root
        .join(rivetos_fleet_repo::DOCUMENTS_DIR)
        .join(format!("{}{}", safe_name(host), rivetos_fleet_repo::DOCUMENT_SUFFIX));
    if !document.is_file() {
        let known = recorded_hosts(root);
        return Err(format!(
            "{host} is NOT IN THE FLEET REPOSITORY at {}: there is no {}. {NOTHING}\n\
             \x20 recorded  {}\n\
             \x20 why       a recovery puts back the document a verified card write recorded, and \
             nothing else says what this machine was.",
            root.display(),
            document.display(),
            if known.is_empty() { "no host at all".to_string() } else { known.join(", ") },
        ));
    }
    let text = std::fs::read_to_string(&document)
        .map_err(|e| format!("cannot read {}: {e}", document.display()))?;
    let doc = rivet_config::Document::parse(&text)
        .map_err(|e| format!("{} does not parse as a machine document: {e:?}. {NOTHING}", document.display()))?;
    let named = doc.hostname().map(|h| h.as_str().to_owned());
    if named.as_deref() != Some(host.as_str()) {
        return Err(format!(
            "{} is filed under {host} and names {}. {NOTHING} A recovery of {host} must put back \
             {host}'s own document.",
            document.display(),
            named.map_or_else(|| "no hostname".to_string(), |n| format!("the hostname {n:?}")),
        ));
    }

    // --- the pin ----------------------------------------------------------------
    let register_root = root.join(rivetos_fleet_repo::REGISTER_DIR);
    let recorded_pin: Option<String> = if register_root.is_dir() {
        let register = rivetos_register::store::load(&register_root)
            .map_err(|e| format!("could not read the register at {}: {e}", register_root.display()))?;
        match register.resolve(host) {
            Ok(device) => Some(device.pin.clone().unwrap_or_default()),
            Err(_) => None,
        }
    } else {
        None
    };
    let recorded = recorded_pin.is_some();
    let recorded_pin = recorded_pin.filter(|p| !p.is_empty()).map(|p| one_pin(&p)).transpose()?;
    let given_pin = args.endpoint.pin.as_deref().map(one_pin).transpose()?;
    let pin = match (recorded_pin, given_pin) {
        (Some(recorded), Some(given)) if recorded != given => {
            return Err(format!(
                "--pin is not the pin the register records for {host}. {NOTHING}\n\
                 \x20 register  {recorded}\n\
                 \x20 --pin     {given}\n\
                 \x20 why       a recovery puts a machine back as the SAME machine: its identity, \
                 and so its pin, is what every peer and bookmark already trusts."
            ));
        }
        (Some(pin), _) | (None, Some(pin)) => pin,
        (None, None) => {
            return Err(format!(
                "{host} is NOT IN THE REGISTER{} and no --pin was given, so there is no pin to \
                 recover it to. {NOTHING} Give the pin the machine answered to with --pin.",
                if recorded { " with a pin" } else { "" },
            ));
        }
    };

    // --- the applications, by digest --------------------------------------------
    let mut applications = Vec::new();
    for service in doc.carried_services() {
        let product = service.name().as_str();
        let (entry, object) =
            crate::fleet::resolve_carried_object(root, product, doc.carried_version(product))
                .map_err(|e| format!("{host}'s document installs {product}, and {e}\n{NOTHING}"))?;
        let bytes = std::fs::read(&object).map_err(|e| format!("cannot read {}: {e}", object.display()))?;
        let actual = rivet_core::package::key_material::to_hex(&rivet_crypto::Blake3::new().of(&bytes));
        if actual != entry.digest {
            return Err(format!(
                "{} is named for {} and its bytes hash to {actual} — the fleet repository's object \
                 store is damaged, and {product} cannot be installed from it. {NOTHING}",
                object.display(),
                entry.digest
            ));
        }
        applications.push(Application { product: entry.product, version: entry.version, digest: entry.digest });
    }

    // --- the secrets --------------------------------------------------------------
    let secrets = match &args.secrets {
        None => None,
        Some(given) => {
            let dir = if given.file_name().is_some_and(|n| n == crate::machine_secrets::BACKUP_FILE) {
                given.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            } else {
                given.clone()
            };
            crate::machine_secrets::check_sealed(&dir).map_err(|e| format!("--secrets: {e}"))?;
            Some(dir)
        }
    };

    // --- the data -------------------------------------------------------------------
    if !args.from.is_dir() {
        return Err(format!("--from {} is not a directory. {NOTHING}", args.from.display()));
    }
    let installs = |name: &str| applications.iter().any(|a| a.product == name);
    let monitor = args.from.join(host);
    let data_dir = if monitor.is_dir() && !installs(host) { monitor } else { args.from.clone() };
    let mut data = Vec::new();
    let mut strays = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&data_dir)
        .map_err(|e| format!("cannot read {}: {e}", data_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with('.')))
        .collect();
    entries.sort();
    for dir in entries {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        if !installs(&name) {
            strays.push(name);
            continue;
        }
        let (file, passed_over) = newest_file(&dir)?;
        data.push(Data { application: name, file, passed_over });
    }
    if !strays.is_empty() {
        let installed: Vec<&str> = applications.iter().map(|a| a.product.as_str()).collect();
        return Err(format!(
            "{} holds data for {}, which {host}'s document does not install. {NOTHING}\n\
             \x20 installs  {}\n\
             \x20 why       restoring it would need an application this recovery never puts \
             back; a backup folder from another machine, or a document that changed since the \
             backup, both look like this.",
            data_dir.display(),
            strays.join(", "),
            if installed.is_empty() { "nothing".to_string() } else { installed.join(", ") },
        ));
    }
    for named in &args.replace {
        if !data.iter().any(|d| &d.application == named) {
            return Err(format!(
                "--replace {named}: there is no data for {named} in {}, so there is nothing to \
                 replace its data with. {NOTHING}",
                data_dir.display()
            ));
        }
    }

    Ok(Plan {
        host: host.clone(),
        root: root.to_path_buf(),
        document,
        recorded,
        pin,
        applications,
        secrets,
        data_dir,
        data,
    })
}

/// **The newest file in one application's backup directory**, and how many
/// others it passed over. Newest by modification time, then by name; a
/// `.partial` file, which `cargo rivet backup` writes before it renames, is
/// never one.
fn newest_file(dir: &Path) -> Result<(PathBuf, usize), String> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.extension().is_none_or(|x| x != "partial"))
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .collect();
    files.sort();
    let count = files.len();
    let Some((_, newest)) = files.pop() else {
        return Err(format!(
            "{} holds no backup file. {NOTHING} Remove the empty directory, or put that \
             application's backup in it.",
            dir.display()
        ));
    };
    Ok((newest, count - 1))
}

/// The plan, as a person reads it.
pub(crate) fn render(plan: &Plan, args: &RecoverArgs) -> String {
    let mut out = format!(
        "recover {}\n\
         \x20 document  {}\n\
         \x20 pin       {} ({})\n\
         \x20 from      {}\n",
        plan.host,
        plan.document.display(),
        plan.pin,
        if plan.recorded && args.endpoint.pin.is_none() {
            "the register's record"
        } else if plan.recorded {
            "given, and the register's record agrees"
        } else {
            "given; the register holds no record"
        },
        plan.data_dir.display(),
    );
    out.push_str(&format!(
        "\n1  the card{}\n\x20  cargo rivet provision {}\n",
        if args.after_boot { " — taken as done: --after-boot" } else { "" },
        card_words(plan, args).join(" ")
    ));
    out.push_str("\n2  the secrets\n");
    match &plan.secrets {
        Some(dir) => out.push_str(&format!(
            "\x20  {}{}, opened with the passphrase typed at the terminal; refused if it is \
             another board's\n",
            dir.join(crate::machine_secrets::BACKUP_FILE).display(),
            if args.replace_secrets { ", replacing secrets whose bytes differ" } else { "" },
        )),
        None => out.push_str("\x20  no --secrets: skipped, and the machine keeps what its card gave it\n"),
    }
    out.push_str("\n3  the applications, by digest\n");
    if plan.applications.is_empty() {
        out.push_str("\x20  the document installs none\n");
    }
    for app in &plan.applications {
        out.push_str(&format!("\x20  {:<24} {:<20} {}\n", app.product, app.version, app.digest));
    }
    if !plan.applications.is_empty() {
        out.push_str("\x20  then `apply reload confirm`, so the supervisor starts what the document enables\n");
    }
    out.push_str("\n4  the data\n");
    if plan.data.is_empty() {
        out.push_str("\x20  no application has data in the backup\n");
    }
    for data in &plan.data {
        out.push_str(&format!(
            "\x20  {:<24} {}{}{}\n",
            data.application,
            data.file.display(),
            if data.passed_over > 0 { format!(" (newest; {} older passed over)", data.passed_over) } else { String::new() },
            if args.replace.contains(&data.application) { " — --replace: whatever it holds now" } else { "" },
        ));
    }
    if !plan.data.is_empty() {
        out.push_str(
            "\x20  each application is asked what it holds first: the same bytes are skipped, no \
             data is restored, anything else needs --replace <application>\n",
        );
    }
    out.push_str(&format!(
        "\nNothing was written and no machine was reached. {}\n",
        if args.after_boot {
            "Add `confirm` to run steps 2 to 4."
        } else {
            "Add `confirm`, at a terminal, to write the card; then boot the machine and run the \
             same command with --after-boot."
        }
    ));
    out
}

/// The words `cargo rivet provision` is given for the card step. `confirm` is
/// never among them, so provision asks its own question before it writes.
pub(crate) fn card_words(plan: &Plan, args: &RecoverArgs) -> Vec<String> {
    let mut words = vec![
        "--existing".to_string(),
        plan.host.clone(),
        "--fleet-repository".to_string(),
        plan.root.display().to_string(),
    ];
    if let Some(image) = &args.image {
        words.push("--image".to_string());
        words.push(image.clone());
    }
    if let Some(device) = &args.device {
        words.push("--device".to_string());
        words.push(device.clone());
    }
    words
}

/// The command a person runs once the recovered machine has booted.
fn after_boot_command(args: &RecoverArgs) -> String {
    let mut words = vec![VERB.to_string(), args.host.clone(), "--from".to_string(), args.from.display().to_string()];
    if let Some(secrets) = &args.secrets {
        words.push("--secrets".to_string());
        words.push(secrets.display().to_string());
    }
    if let Some(at) = &args.at {
        words.push("--at".to_string());
        words.push(at.display().to_string());
    }
    words.push("--after-boot".to_string());
    words.push("confirm".to_string());
    words.join(" ")
}

// ---------------------------------------------------------------------------
// Step 1 — the card
// ---------------------------------------------------------------------------

/// **Whether a secrets backup belongs to the machine `pin` names**, asked
/// with the passphrase `ask` answers with.
///
/// # Errors
///
/// The backup cannot be opened, or it is another board's.
pub(crate) fn secrets_belong_to(
    dir: &Path,
    pin: &str,
    ask: crate::machine_secrets::Ask<'_>,
) -> Result<(), String> {
    let recorded = crate::machine_secrets::recorded_board_key(dir, ask)?;
    if recorded != pin {
        return Err(format!(
            "REFUSED — the secrets backup is from a DIFFERENT BOARD, and NOTHING WAS WRITTEN.\n\
             \x20 backup taken from  {recorded}\n\
             \x20 recovering         {pin}\n\
             \x20 why  a recovery keeps the machine's pin, and a backup of /state/secrets goes back \
             onto the board it came from and no other (ADR-030 §5)."
        ));
    }
    Ok(())
}

/// Step 1: check the secrets, then hand the card to `provision --existing`.
fn write_card(plan: &Plan, args: &RecoverArgs) -> Result<String, String> {
    use std::io::IsTerminal as _;
    if !std::io::stdin().is_terminal() {
        return Err(format!(
            "the card is written by a person at a terminal, and this has none. NOTHING WAS \
             WRITTEN.\n\
             \x20 an agent  proves steps 2 to 4 against a machine that is already running: {}",
            after_boot_command(args)
        ));
    }
    if let Some(dir) = &plan.secrets {
        secrets_belong_to(dir, &plan.pin, &|| crate::machine_secrets::ask_at_the_terminal(VERB))?;
    }
    let written = crate::provision::run(&card_words(plan, args))?;
    Ok(format!(
        "{written}\n\
         Step 1 is done. Boot {host}, wait until its control plane answers, then run steps 2 to 4:\n\
         \x20 {}\n",
        after_boot_command(args),
        host = plan.host,
    ))
}

// ---------------------------------------------------------------------------
// Steps 2 to 4 — against the running machine
// ---------------------------------------------------------------------------

/// What an application holds now, as its own backup route answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Holds {
    /// A 2xx answer with no body.
    Nothing,
    /// A 2xx answer, and its bytes.
    Bytes(Vec<u8>),
    /// No answer, an error status, or the machine's own refusal — why.
    Unknown(String),
}

/// **What steps 2 to 4 ask of the machine.** A trait so the order, and the
/// refusal to write over data, are tested against a fake machine; the real
/// one composes the verbs that already do each step.
pub(crate) trait Machine {
    /// Step 2: `restore-secrets` from `dir`, the passphrase typed.
    fn restore_secrets(&mut self, dir: &Path, replace: bool) -> Result<String, String>;
    /// Step 3: install one application by its digest.
    fn install(&mut self, application: &Application) -> Result<String, String>;
    /// Step 3: one converge, after every install.
    fn converge(&mut self) -> Result<String, String>;
    /// Step 4: what `application` holds now.
    fn holds(&mut self, application: &str) -> Holds;
    /// Step 4: send `file` to `application`'s restore route.
    fn restore_data(&mut self, application: &str, file: &Path) -> Result<String, String>;
}

/// The report so far, and the refusal that stopped it.
fn stopped(report: &str, step: u8, why: &str) -> String {
    format!("{report}\nSTOPPED AT STEP {step}. What is listed above is done; nothing after it was attempted.\n{why}")
}

/// **Steps 2 to 4, in order, stopping at the first refusal.**
///
/// # Errors
///
/// The first step that refuses, with everything done before it.
pub(crate) fn after_boot(plan: &Plan, args: &RecoverArgs, machine: &mut dyn Machine) -> Result<String, String> {
    let mut report = String::new();

    report.push_str("--- 2  the secrets\n");
    match &plan.secrets {
        Some(dir) => {
            let said = machine
                .restore_secrets(dir, args.replace_secrets)
                .map_err(|e| stopped(&report, 2, &e))?;
            report.push_str(&said);
        }
        None => report.push_str("no --secrets: skipped\n"),
    }

    report.push_str("\n--- 3  the applications\n");
    for app in &plan.applications {
        report.push_str(&format!("{} {} {}\n", app.product, app.version, app.digest));
        let said = machine.install(app).map_err(|e| stopped(&report, 3, &e))?;
        report.push_str(&said);
    }
    if plan.applications.is_empty() {
        report.push_str("the document installs none\n");
    } else {
        let said = machine.converge().map_err(|e| stopped(&report, 3, &e))?;
        report.push_str(&format!("converged: {}\n", said.trim()));
    }

    report.push_str("\n--- 4  the data\n");
    // **Every application is settled before any is sent anything.**
    let mut sends: Vec<(&Data, &'static str)> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for data in &plan.data {
        let bytes = std::fs::read(&data.file)
            .map_err(|e| stopped(&report, 4, &format!("cannot read {}: {e}", data.file.display())))?;
        let replace = args.replace.contains(&data.application);
        match machine.holds(&data.application) {
            Holds::Bytes(now) if now == bytes => {
                report.push_str(&format!(
                    "{:<24} already holds these {} bytes — not sent\n",
                    data.application,
                    bytes.len()
                ));
            }
            Holds::Nothing => sends.push((data, "holds no data")),
            Holds::Bytes(_) if replace => sends.push((data, "REPLACING what it holds, as --replace asked")),
            Holds::Unknown(_) if replace => sends.push((data, "REPLACING whatever it holds, as --replace asked")),
            Holds::Bytes(now) => refused.push(format!(
                "\x20 {:<24} holds {} bytes that are not the backup's {}",
                data.application,
                now.len(),
                bytes.len()
            )),
            Holds::Unknown(why) => refused.push(format!(
                "\x20 {:<24} could not say what it holds: {}",
                data.application,
                why.lines().next().unwrap_or_default()
            )),
        }
    }
    if !refused.is_empty() {
        let names: Vec<&str> = refused
            .iter()
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        return Err(stopped(
            &report,
            4,
            &format!(
                "A RECOVERY THAT WRITES OVER DATA ASKS FIRST, and NO APPLICATION WAS SENT ANY DATA:\n\
                 {}\n\
                 \x20 why  restoring replaces what the application holds now, and nothing here can \
                 tell an empty database from a real one, or a stale backup from a current one.\n\
                 \x20 fix  take a fresh `cargo rivet backup` first if what it holds matters, then \
                 run this again with {}. An application that did not answer may need its \
                 material staged first (`cargo rivet secrets`), which no backup holds.",
                refused.join("\n"),
                names.iter().map(|n| format!("--replace {n}")).collect::<Vec<_>>().join(" "),
            ),
        ));
    }
    for (data, why) in sends {
        report.push_str(&format!("{:<24} {why}\n", data.application));
        let said = machine
            .restore_data(&data.application, &data.file)
            .map_err(|e| stopped(&report, 4, &e))?;
        report.push_str(&said);
    }
    if plan.data.is_empty() {
        report.push_str("no application has data in the backup\n");
    }
    report.push_str(&format!(
        "\nRecovered {}: every step finished. The machine's own secrets are live at its next \
         boot, except peers; restart it when the operator chooses to prove it boots as itself.\n",
        plan.host
    ));
    Ok(report)
}

/// **The real machine**: each step is the verb that already does it, run
/// with the endpoint this recovery resolved.
struct Board {
    words: Vec<String>,
    root: PathBuf,
    tls: rivet_remote::tls::Tls,
}

impl Board {
    fn with(&self, rest: &[&str]) -> Vec<String> {
        let mut words = self.words.clone();
        words.extend(rest.iter().map(|w| (*w).to_string()));
        words
    }
}

impl Machine for Board {
    fn restore_secrets(&mut self, dir: &Path, replace: bool) -> Result<String, String> {
        let dir = dir.display().to_string();
        let mut rest = vec!["--from", dir.as_str()];
        if replace {
            rest.push("--replace");
        }
        rest.push("confirm");
        crate::machine_secrets::restore_execute(&self.with(&rest))
    }

    fn install(&mut self, application: &Application) -> Result<String, String> {
        let root = self.root.display().to_string();
        crate::deploy_cmd(&self.with(&["--artifact", &application.digest, "--at", &root, "confirm"]))
    }

    fn converge(&mut self) -> Result<String, String> {
        rivet_remote::command::perform(&self.tls, &["apply", "reload", "confirm"])
    }

    fn holds(&mut self, application: &str) -> Holds {
        match self.tls.fetch(application) {
            Err(why) => Holds::Unknown(why),
            Ok(answer) if answer.answered_by_the_machine() => {
                Holds::Unknown(format!("the machine answered {}: {}", answer.status, answer.text()))
            }
            Ok(answer) if !(200..300).contains(&answer.status) => {
                Holds::Unknown(format!("it answered {}: {}", answer.status, answer.text()))
            }
            Ok(answer) if answer.body.is_empty() => Holds::Nothing,
            Ok(answer) => Holds::Bytes(answer.body),
        }
    }

    fn restore_data(&mut self, application: &str, file: &Path) -> Result<String, String> {
        let file = file.display().to_string();
        crate::restore::execute(&self.with(&["--service", application, "--from", &file, "confirm"]))
    }
}

/// `--host`, `--identity` and `--pin` for steps 2 to 4: what was given, then
/// the register's record, then the default identity. The pin is the plan's.
fn endpoint_words(plan: &Plan, args: &RecoverArgs) -> Result<Vec<String>, String> {
    let mut given = Vec::new();
    if let Some(host) = &args.endpoint.host {
        given.extend(["--host".to_string(), host.clone()]);
    }
    if let Some(identity) = &args.endpoint.identity {
        given.extend(["--identity".to_string(), identity.display().to_string()]);
    }
    given.extend(["--pin".to_string(), plan.pin.clone()]);
    let mut words = if plan.recorded {
        crate::device::expand(&plan.host, &given, &plan.root.join(rivetos_fleet_repo::REGISTER_DIR))?
    } else {
        given
    };
    if !words.iter().any(|w| w == "--host") {
        return Err(format!(
            "{} has no record in the register to say where it listens, so --host HOST:PORT is \
             needed. Nothing was sent.",
            plan.host
        ));
    }
    if !words.iter().any(|w| w == "--identity") {
        let dir = crate::device::default_identity_dir()
            .ok_or("no --identity, and neither HOME nor USERPROFILE is set to find the default one")?;
        words.extend(["--identity".to_string(), dir.display().to_string()]);
    }
    Ok(words)
}

/// `cargo rivet recover`.
///
/// # Errors
///
/// The plan's refusals; then the card step's, or the first of steps 2 to 4
/// that refuses.
pub fn execute(args: &[String]) -> Result<String, String> {
    let args = parse(args)?;
    let root = crate::fleet::deploy_root(args.at.as_deref())?;
    let plan = plan(&args, &root)?;
    if !args.confirm {
        return Ok(render(&plan, &args));
    }
    if !args.after_boot {
        return write_card(&plan, &args);
    }
    let words = endpoint_words(&plan, &args)?;
    let mut endpoint = Endpoint::default();
    let mut index = 0;
    while index < words.len() {
        index = endpoint.read(&words, index)?.unwrap_or(index + 1);
    }
    let tls = endpoint.open()?.ok_or("internal error: an endpoint with no identity")?;
    let mut board = Board { words, root, tls };
    let report = after_boot(&plan, &args, &mut board)?;
    Ok(format!("recover {} — steps 2 to 4\n\n{report}", plan.host))
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;
