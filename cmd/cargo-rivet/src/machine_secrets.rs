//! `cargo rivet backup-secrets` and `cargo rivet restore-secrets` — **one
//! board's `/state/secrets`, pulled to a workstation over the control plane,
//! sealed under a typed passphrase, and put back onto the same board and no
//! other.** `ADR-030` §4 and §5, `.rulings/` R-183 D5 and R-185.
//!
//! # What the store is, and so why the backup is encrypted
//!
//! `/state/secrets` holds the device identity (`device.seed`, `device.crt`),
//! the control plane's `peers`, and the values `rivet_core::secret` stores — a
//! wireless pre-shared key is `<interface>.psk`. **`ADR-020` §11 rules those
//! plain text on the board** (hex, for the values). A copy of them is enough to
//! present as that board until its key is removed from every pin list
//! (`ADR-025` §5), so R-185 D1 rules the copy on the workstation encrypted: the
//! same envelope and the same typed prompt as `cargo rivet key backup`
//! (`crate::envelope`), and a forgotten passphrase loses the backup.
//!
//! # The backup is one file, and nothing about the secrets is readable
//!
//! `<into>/secrets-backup.enc` is one envelope (magic `RVTSECR`). Inside it, in
//! memory only, is a manifest — the format line, the board's key, the host, the
//! time, and each file's BLAKE3, size and name — then the files' bytes in the
//! manifest's order. **Nothing else is written**: no file names, no sizes, no
//! digests, and not the board key. The board key is public, and a readable copy
//! would let a restore refuse another board before asking for the passphrase —
//! but every restore needs the passphrase anyway (even its plan compares the
//! backup's bytes with the board's), and a second, unauthenticated statement of
//! which board a backup belongs to is one more thing to reconcile. So the
//! binding lives inside the envelope, where the AEAD authenticates it.
//!
//! The digests inside are not what detects tampering — the AEAD is, and a
//! wrong passphrase and an altered file are refused with the same sentence. They
//! stay because a manifest that disagrees with its own bytes is a defect in
//! whatever wrote it, and saying which file is wrong is cheap.
//!
//! **No plaintext secret reaches the workstation's disk.** The archive is built
//! and sealed in memory, and the envelope is written through `<name>.partial`
//! and a rename. Nothing prints a digest either: a BLAKE3 of a short
//! pre-shared key is something to guess against.
//!
//! **A backup an earlier build wrote unencrypted** — `secrets-backup.txt` beside
//! a `files/` directory — is refused by name, with the instruction to take a new
//! one. That build was never released.
//!
//! # Which board — the device key, proven by the handshake
//!
//! The backup records the board's **Ed25519 device public key** — the key
//! `--pin` names and the TLS handshake proves the board holds (`ADR-025` §2).
//! R-185 D2 rules it: **a board is the same board only if that key matches.** A
//! Pi re-carded with a new identity is a different board and is refused; it gets
//! freshly provisioned secrets. The hardware serial plays no part: a board
//! reports it as plain text nothing authenticates, a virtual machine has none,
//! and an x86 machine's `/proc/cpuinfo` carries none.
//!
//! Before anything is written the key is checked against what the backup
//! itself carries: `device.crt` must carry it and `device.seed` must derive to
//! it. A backup whose own files name a different board is refused as not what
//! it says it is.
//!
//! # What restore writes, and what it leaves running
//!
//! Each file goes back through `apply restore-secret` — root-owned, mode 0600,
//! a temporary renamed into place, so nothing ever reads half a file. **Nothing
//! is stopped or restarted.** The readers of `/state/secrets` are `rivet-mcpd`
//! and `rivet-netd`: both are in the reachability set `apply service-stop`
//! refuses (`ADR-017` §6), both read their files when they start, and stopping
//! `rivet-mcpd` would end the session doing the restore. Applications never
//! open this directory (`ADR-020` §8). So a restored file is live at the
//! board's next boot — except `peers`, which `rivet-mcpd` reloads for the next
//! connection (`ADR-063` §3) — and the report says that rather than implying
//! otherwise.
//!
//! Every refusal below comes after the backup is opened and before anything is
//! sent:
//!
//! * a wrong passphrase, or an envelope altered since it was written;
//! * a manifest naming a different board from the one `--pin` names;
//! * a `peers` file whose grammar the board's own reader would refuse, or
//!   that does not name the key this workstation connects with — either way
//!   it would admit nobody who can undo it, and the board refuses the same
//!   file for the same reason;
//! * `device.seed` or `device.crt` that disagree with the recorded key;
//! * a file whose bytes do not match the size or digest the manifest records.
//!
//! A file the board already holds with the same bytes is skipped. A file whose
//! bytes differ is replaced only with `--replace`, the house rule `key restore`
//! and `fleet objects restore` follow. A file on the board that the backup does
//! not name is left alone and reported.
//!
//! # No admin server, no schedule
//!
//! R-183 D5. Each verb runs once, when asked, and keeps nothing.

use std::path::{Path, PathBuf};

use rivet_core::json::Json;
use rivet_core::package::port::Digest as _;
use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};
use rivet_remote::tls::CallsTools;

use crate::envelope::{self, Kind};

/// The one file a backup is, inside the backup directory.
pub const BACKUP_FILE: &str = "secrets-backup.enc";

/// The manifest an earlier build wrote in the clear. Recognised only to refuse
/// it.
pub const UNENCRYPTED_MANIFEST: &str = "secrets-backup.txt";

/// The directory that earlier build wrote each secret into, in the clear.
pub const UNENCRYPTED_FILES_DIR: &str = "files";

/// The envelope a secrets backup is. A different magic from a seed backup's,
/// so neither restore verb mistakes the other's file for its own.
const SECRETS_ENVELOPE: Kind = Kind {
    magic: b"RVTSECR",
    noun: "a Rivet machine-secrets backup file",
};

/// What the prompt calls this backup.
const WHAT: &str = "secrets backup";

/// The manifest's first line. Version 1 was the unencrypted layout.
const FORMAT: &str = "rivet-machine-secrets 2";

/// The line that ends the manifest; the files' bytes follow it.
const END: &str = "end";

/// The board's store. `rivet_core::state::SECRETS`, the one spelling.
const STORE: &str = rivet_core::state::SECRETS;

/// The operation that writes one back.
const RESTORE_SUBJECT: &str = rivet_core::operation::restore_secret::SUBJECT;

/// The file that names who may connect.
const PEERS: &str = rivet_core::operation::restore_secret::PEERS_NAME;

/// Where a passphrase comes from: the terminal, in the shipped verbs; a fixed
/// string, in a test. Called at most once per run, and only once every check
/// that needs no passphrase has passed.
pub(crate) type Ask<'a> = &'a dyn Fn() -> Result<String, String>;

// ---------------------------------------------------------------------------
// Usage and arguments
// ---------------------------------------------------------------------------

pub fn backup_usage() -> String {
    format!(
        "usage: cargo rivet backup-secrets --host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]\n\
         \n\
         Pulls every file in one board's {STORE} over ADR-035's control plane and\n\
         seals them, with a manifest recording WHICH BOARD they came from (its device\n\
         public key, the key --pin names), into ONE encrypted file: --into/{BACKUP_FILE}.\n\
         ADR-030, R-183 D5, R-185.\n\
         \n\
         WITHOUT `confirm` NO SECRET IS READ AND NO PASSPHRASE IS ASKED FOR. It lists\n\
         the store and prints what would be fetched; `confirm` asks for a passphrase,\n\
         fetches, encrypts and writes.\n\
         \n\
         backup-secrets options:\n\
         \x20 --host HOST:PORT   REQUIRED. Where rivet-mcpd is listening.\n\
         {IDENTITY_AND_PIN}\
         \x20 --into DIR         REQUIRED. There is no default. A directory that\n\
         \x20                    already holds a backup is refused, not overwritten.\n\
         \x20 confirm            fetch, encrypt and write. Without it, the plan.\n\
         \n\
         THE PASSPHRASE is typed twice, with no echo, at a real terminal: the same\n\
         envelope and prompt as `cargo rivet key backup`. There is no --passphrase\n\
         flag and no environment variable. Nothing about the secrets is written in\n\
         the clear — not their names, not their sizes.\n\
         \n\
         {}\n\
         \n\
         There is no schedule and no retention here: it runs once, when asked.\n\
         `cargo rivet restore-secrets` puts a backup back onto the SAME board.\n",
        envelope::FORGOTTEN
    )
}

pub fn restore_usage() -> String {
    format!(
        "usage: cargo rivet restore-secrets --host HOST:PORT --identity DIR --pin HEX --from DIR [--replace] [confirm]\n\
         \n\
         Opens a `cargo rivet backup-secrets` backup with its passphrase and puts it\n\
         back into {STORE} on THE BOARD IT CAME FROM, one `apply {RESTORE_SUBJECT}`\n\
         per file. ADR-030 §5, R-183 D5, R-185.\n\
         \n\
         The passphrase is asked for once, at a real terminal, before the board is\n\
         asked anything — the plan needs it too, to compare the backup with the board.\n\
         \n\
         REFUSED, AND NOTHING SENT, when:\n\
         \x20 * the passphrase is wrong, or {BACKUP_FILE} was altered since it was\n\
         \x20   written — the same refusal for both;\n\
         \x20 * the board key the backup recorded is not the key --pin names — a\n\
         \x20   different board. A board re-carded with a new identity is a different\n\
         \x20   board. Its device seed would make two boards one identity;\n\
         \x20 * a file's size or BLAKE3 digest is not what the manifest recorded;\n\
         \x20 * device.seed or device.crt disagree with the recorded key;\n\
         \x20 * {PEERS} does not name the key --identity connects with: after the next\n\
         \x20   boot the board would admit nobody who could undo it;\n\
         \x20 * a file on the board differs and --replace was not given;\n\
         \x20 * --from holds a backup an earlier build wrote UNENCRYPTED: take a new one.\n\
         \n\
         WITHOUT `confirm` NOTHING IS WRITTEN. It compares the backup with what the\n\
         board holds and prints, per file: write, replace, identical (skipped).\n\
         \n\
         restore-secrets options:\n\
         \x20 --host HOST:PORT   REQUIRED. Where rivet-mcpd is listening.\n\
         {IDENTITY_AND_PIN}\
         \x20 --from DIR         REQUIRED. The directory backup-secrets wrote.\n\
         \x20 --replace          allow replacing a file whose bytes differ.\n\
         \x20 confirm            write. Without it, the plan.\n\
         \n\
         {}\n\
         \n\
         NOTHING IS RESTARTED. rivet-mcpd and rivet-netd read these files when they\n\
         start and cannot be stopped over the control plane, so a restored file is\n\
         live at the board's NEXT BOOT — except peers, which rivet-mcpd reloads for\n\
         the next connection. A file on the board that the backup does not name is\n\
         left alone.\n",
        envelope::FORGOTTEN
    )
}

/// What `backup-secrets` was asked to do.
#[derive(Debug)]
pub struct BackupArgs {
    pub endpoint: Endpoint,
    pub into: PathBuf,
    pub confirm: bool,
}

/// What `restore-secrets` was asked to do.
#[derive(Debug)]
pub struct RestoreArgs {
    pub endpoint: Endpoint,
    pub from: PathBuf,
    pub replace: bool,
    pub confirm: bool,
}

/// # Errors
///
/// A flag with no value, a word this verb does not take, a missing `--into`,
/// or half of `ADR-025` §2's trust.
pub fn parse_backup(args: &[String]) -> Result<BackupArgs, String> {
    let mut endpoint = Endpoint::default();
    let mut into = None;
    let mut confirm = false;
    let mut index = 0;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "--into" => {
                into = Some(PathBuf::from(value_after(args, index, "--into")?));
                index += 2;
            }
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option `backup-secrets` takes. There is no --passphrase \
                     flag and no environment variable: the passphrase is typed.\n\n{}",
                    backup_usage()
                ));
            }
        }
    }
    endpoint.paired()?;
    let into = into.ok_or_else(|| {
        format!(
            "--into is required: there is nowhere to put a board's secrets, and a default would \
             be this tool choosing where they live.\n\n{}",
            backup_usage()
        )
    })?;
    Ok(BackupArgs {
        endpoint,
        into,
        confirm,
    })
}

/// # Errors
///
/// As [`parse_backup`], for `--from`.
pub fn parse_restore(args: &[String]) -> Result<RestoreArgs, String> {
    let mut endpoint = Endpoint::default();
    let mut from = None;
    let mut replace = false;
    let mut confirm = false;
    let mut index = 0;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "--from" => {
                from = Some(PathBuf::from(value_after(args, index, "--from")?));
                index += 2;
            }
            "--replace" => {
                replace = true;
                index += 1;
            }
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option `restore-secrets` takes. There is no \
                     --passphrase flag and no environment variable: the passphrase is typed.\n\n{}",
                    restore_usage()
                ));
            }
        }
    }
    endpoint.paired()?;
    let from = from.ok_or_else(|| {
        format!("--from is required: the directory `backup-secrets` wrote.\n\n{}", restore_usage())
    })?;
    Ok(RestoreArgs {
        endpoint,
        from,
        replace,
        confirm,
    })
}

fn value_after(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

/// The one key `--pin` names, as lower-case hex.
///
/// # Errors
///
/// No pin, or a pin that is not exactly one 64-hex-character key.
fn pin_of(endpoint: &Endpoint) -> Result<String, String> {
    let text = endpoint
        .pin
        .as_deref()
        .ok_or("--pin is required: it is the board's identity, and this verb records it")?;
    let pins = rivetos_tls::parse_pins(text).map_err(|e| format!("--pin: {e}"))?;
    match pins.as_slice() {
        [one] => Ok(rivetos_tls::encode_hex(one)),
        _ => Err("--pin must name exactly one key".to_string()),
    }
}

/// Open the control plane, or say why not.
fn open(endpoint: &Endpoint, usage: &str) -> Result<rivet_remote::tls::Tls, String> {
    endpoint.open()?.ok_or_else(|| {
        format!(
            "--identity and --pin are required: a board's secrets are reached over ADR-035's \
             control plane and nothing else.\n\n{usage}"
        )
    })
}

// ---------------------------------------------------------------------------
// Reading the board
// ---------------------------------------------------------------------------

/// One entry of `/state/secrets` as `get file` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    name: String,
    bytes: u64,
}

/// `get file <path>`, parsed, and checked to be an answer about `path`.
fn get_file(caller: &dyn CallsTools, path: &str) -> Result<Json, String> {
    let answer = rivet_remote::command::perform(caller, &["get", "file", path])?;
    let json = rivet_core::json::parse(&answer).map_err(|why| {
        format!("the board's answer about {path} is not JSON this build reads ({why})")
    })?;
    match json.get("path").and_then(Json::as_str) {
        Some(seen) if seen == path => Ok(json),
        Some(seen) => Err(format!(
            "this asked about {path} and the board answered about {seen}; nothing is read from it"
        )),
        None => Err(format!("the board's answer about {path} names no path")),
    }
}

/// The regular files directly inside the store.
///
/// # Errors
///
/// No store; a listing the board cut short; an entry that is not a regular
/// file, or whose name `apply restore-secret` could not write back. **A backup
/// that silently left something out would be reported as whole**, so each of
/// those refuses the backup rather than skipping the entry.
fn list_store(caller: &dyn CallsTools) -> Result<Vec<Listed>, String> {
    let json = get_file(caller, STORE)?;
    if json.get("present") != Some(&Json::Bool(true)) {
        return Err(format!(
            "there is no {STORE} on that board, so there is nothing to back up. rivet-initd makes \
             it when it installs the board's identity."
        ));
    }
    if json.get("type").and_then(Json::as_str) != Some("directory") {
        return Err(format!("{STORE} on that board is not a directory"));
    }
    if json.get("entries_truncated") != Some(&Json::Bool(false)) {
        return Err(format!(
            "the board listed {STORE} and cut the listing short, or did not say whether it did; \
             a backup of part of it would be reported as the whole"
        ));
    }
    let entries = json
        .get("entries")
        .and_then(Json::as_array)
        .ok_or_else(|| format!("the board's listing of {STORE} carries no entries"))?;
    let mut out = Vec::new();
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(Json::as_str)
            .ok_or("an entry in the listing has no name")?;
        let kind = entry.get("type").and_then(Json::as_str).unwrap_or("unknown");
        if kind != "file" {
            return Err(format!(
                "{STORE}/{name} is a {kind} and not a regular file. This backs up files, and \
                 leaving it out would make a backup that is not the whole store — so nothing was \
                 backed up. Say what it is before going further: `cargo rivet run … -- get file \
                 {STORE}/{name}`."
            ));
        }
        if !rivet_core::operation::is_secret_name(name) {
            return Err(format!(
                "{STORE}/{name} has a name `apply {RESTORE_SUBJECT}` cannot write back, so a \
                 backup of it could never be restored — nothing was backed up."
            ));
        }
        let bytes = entry
            .get("size_bytes")
            .and_then(Json::as_integer)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| format!("{STORE}/{name} has no size in the listing"))?;
        out.push(Listed {
            name: name.to_string(),
            bytes,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// One file's bytes, exactly as the board holds them.
///
/// # Errors
///
/// Nothing there, not a file, content withheld, an encoding this build does
/// not read, or a length that disagrees with the board's own count.
fn read_secret(caller: &dyn CallsTools, name: &str) -> Result<Option<Vec<u8>>, String> {
    let path = format!("{STORE}/{name}");
    let json = get_file(caller, &path)?;
    if json.get("present") != Some(&Json::Bool(true)) {
        return Ok(None);
    }
    if json.get("type").and_then(Json::as_str) != Some("file") {
        return Err(format!("{path} on that board is not a regular file"));
    }
    let Some(content) = json.get("content").and_then(Json::as_str) else {
        let why = json
            .get("content_omitted")
            .and_then(Json::as_str)
            .unwrap_or("the board gave no reason");
        return Err(format!("the board would not give the contents of {path}: {why}"));
    };
    let bytes = match json.get("content_encoding").and_then(Json::as_str) {
        Some("text") => content.as_bytes().to_vec(),
        Some("base64") => rivet_core::base64::decode(content)
            .ok_or_else(|| format!("{path}: the board said base64 and it does not decode"))?,
        other => {
            return Err(format!(
                "{path}: the board says content_encoding is {other:?}, which this build does not \
                 read"
            ));
        }
    };
    let counted = json
        .get("size_bytes")
        .and_then(Json::as_integer)
        .and_then(|n| usize::try_from(n).ok());
    if counted != Some(bytes.len()) {
        return Err(format!(
            "{path}: the board counted {counted:?} bytes and {} arrived; nothing is kept from it",
            bytes.len()
        ));
    }
    Ok(Some(bytes))
}

// ---------------------------------------------------------------------------
// The manifest, and the archive it heads
// ---------------------------------------------------------------------------

/// One file a backup holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    bytes: u64,
    blake3: String,
}

/// What a backup says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Manifest {
    board_key: String,
    host: String,
    taken_unix_seconds: u64,
    files: Vec<Entry>,
}

fn blake3_hex(bytes: &[u8]) -> String {
    rivet_core::package::key_material::to_hex(&rivet_crypto::Blake3::new().of(bytes))
}

impl Manifest {
    /// The manifest's lines, ending with [`END`].
    fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("format = {FORMAT}\n"));
        out.push_str(&format!("board-key = {}\n", self.board_key));
        out.push_str(&format!("host = {}\n", self.host));
        out.push_str(&format!("taken-unix-seconds = {}\n", self.taken_unix_seconds));
        for file in &self.files {
            out.push_str(&format!("file = {} {} {}\n", file.blake3, file.bytes, file.name));
        }
        out.push_str(END);
        out.push('\n');
        out
    }

    /// **Strict**: every line is known, every required key appears once, and a
    /// file is named once. A manifest this build half-understood would be a
    /// restore of something nobody described. `text` is the manifest without
    /// its [`END`] line.
    fn parse(text: &str) -> Result<Self, String> {
        let mut format = None;
        let mut board_key = None;
        let mut host = None;
        let mut taken = None;
        let mut files: Vec<Entry> = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = n + 1;
            let (key, value) = line
                .split_once(" = ")
                .ok_or_else(|| format!("line {at} is not `key = value`"))?;
            let once = |slot: &mut Option<String>| -> Result<(), String> {
                if slot.replace(value.to_string()).is_some() {
                    return Err(format!("line {at}: `{key}` is given twice"));
                }
                Ok(())
            };
            match key {
                "format" => once(&mut format)?,
                "board-key" => once(&mut board_key)?,
                "host" => once(&mut host)?,
                "taken-unix-seconds" => once(&mut taken)?,
                "file" => {
                    let mut parts = value.splitn(3, ' ');
                    let (Some(digest), Some(size), Some(name)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        return Err(format!("line {at}: a file line is `<blake3> <bytes> <name>`"));
                    };
                    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(format!("line {at}: `{digest}` is not a BLAKE3 digest"));
                    }
                    let bytes = size
                        .parse::<u64>()
                        .map_err(|_| format!("line {at}: `{size}` is not a byte count"))?;
                    if !rivet_core::operation::is_secret_name(name) {
                        return Err(format!("line {at}: `{name}` is not a secret's name"));
                    }
                    if files.iter().any(|f| f.name == name) {
                        return Err(format!("line {at}: `{name}` is named twice"));
                    }
                    files.push(Entry {
                        name: name.to_string(),
                        bytes,
                        blake3: digest.to_ascii_lowercase(),
                    });
                }
                other => return Err(format!("line {at}: `{other}` is not a key this format has")),
            }
        }
        match format.as_deref() {
            Some(FORMAT) => {}
            Some(other) => {
                return Err(format!(
                    "this backup is format `{other}` and this build reads `{FORMAT}`"
                ));
            }
            None => return Err("this manifest names no format".to_string()),
        }
        let board_key = board_key.ok_or("this manifest records no board-key")?;
        if board_key.len() != 64 || !board_key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("this manifest's board-key is not a 64-hex-character key".to_string());
        }
        let taken_unix_seconds = taken
            .ok_or("this manifest records no taken-unix-seconds")?
            .parse::<u64>()
            .map_err(|_| "this manifest's taken-unix-seconds is not a number".to_string())?;
        Ok(Self {
            board_key: board_key.to_ascii_lowercase(),
            host: host.ok_or("this manifest records no host")?,
            taken_unix_seconds,
            files,
        })
    }
}

/// A backup's files, by name, in the manifest's order.
type Files = Vec<(String, Vec<u8>)>;

/// **The plaintext an envelope seals**: the manifest, then every file's bytes
/// in the manifest's order. Built in memory and handed straight to
/// `envelope::seal`; it never touches a disk.
fn archive(manifest: &Manifest, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = manifest.render().into_bytes();
    for (_, bytes) in files {
        out.extend_from_slice(bytes);
    }
    out
}

/// Split an opened archive into its manifest and its files, checking every
/// file against the size and digest the manifest records.
///
/// # Errors
///
/// No [`END`] line; a manifest this build does not read; bytes the manifest
/// does not account for, or too few; a file whose digest differs.
fn unarchive(plaintext: &[u8]) -> Result<(Manifest, Files), String> {
    let terminator = format!("\n{END}\n");
    let split = plaintext
        .windows(terminator.len())
        .position(|w| w == terminator.as_bytes())
        .ok_or_else(|| format!("its manifest has no `{END}` line"))?;
    let header = std::str::from_utf8(&plaintext[..=split])
        .map_err(|_| "its manifest is not UTF-8".to_string())?;
    let manifest = Manifest::parse(header)?;
    let body = &plaintext[split + terminator.len()..];

    let accounted: u64 = manifest.files.iter().map(|f| f.bytes).sum();
    if accounted != body.len() as u64 {
        return Err(format!(
            "it holds {} byte(s) of files and its manifest accounts for {accounted}",
            body.len()
        ));
    }
    let mut faults = Vec::new();
    let mut files = Vec::new();
    let mut at = 0usize;
    for entry in &manifest.files {
        // `accounted == body.len()` above makes every slice in range.
        let len = usize::try_from(entry.bytes).unwrap_or(usize::MAX);
        let bytes = body.get(at..at.saturating_add(len)).unwrap_or_default().to_vec();
        at = at.saturating_add(len);
        let actual = blake3_hex(&bytes);
        if actual == entry.blake3 {
            files.push((entry.name.clone(), bytes));
        } else {
            faults.push(format!(
                "  {} — its BLAKE3 is not the digest the manifest recorded",
                entry.name
            ));
        }
    }
    if !faults.is_empty() {
        return Err(format!(
            "its manifest does not describe its own files:\n{}",
            faults.join("\n")
        ));
    }
    Ok((manifest, files))
}

// ---------------------------------------------------------------------------
// Identity checks, shared by both directions
// ---------------------------------------------------------------------------

/// **Whether `device.seed` and `device.crt`, where present, name `key`.**
///
/// # Errors
///
/// A seed that is not 32 bytes or derives to another key; a certificate that
/// carries no Ed25519 key or carries another one.
fn check_identity_files(files: &[(String, Vec<u8>)], key: &str, whose: &str) -> Result<(), String> {
    for (name, bytes) in files {
        if name == rivetos_tls::SEED_FILE {
            let seed: [u8; rivet_crypto::SEED_BYTES] = bytes.as_slice().try_into().map_err(|_| {
                format!("{whose} {name} is {} bytes and a device seed is 32", bytes.len())
            })?;
            let derived = rivetos_tls::encode_hex(&rivet_crypto::public_key_for(&seed).0);
            if derived != key {
                return Err(format!(
                    "{whose} {name} is the seed of {derived}, not of the board key {key}"
                ));
            }
        }
        if name == rivetos_tls::CERTIFICATE_FILE {
            let carried = rivetos_tls::public_key_in_certificate(bytes)
                .map(|pin| rivetos_tls::encode_hex(&pin))
                .ok_or_else(|| format!("{whose} {name} carries no Ed25519 public key"))?;
            if carried != key {
                return Err(format!(
                    "{whose} {name} carries {carried}, not the board key {key}"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// backup-secrets
// ---------------------------------------------------------------------------

/// `backup-secrets`, with a real connection and a real terminal behind it.
///
/// # Errors
///
/// See [`backup_via`].
pub fn backup_execute(args: &[String]) -> Result<String, String> {
    let parsed = parse_backup(args)?;
    let transport = open(&parsed.endpoint, &backup_usage())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    backup_via(&parsed, &transport, now, &|| {
        envelope::read_new(WHAT, "cargo rivet backup-secrets")
    })
}

/// A directory holding any part of a backup, this build's or the unencrypted
/// layout an earlier one wrote.
fn holds_a_backup(dir: &Path) -> bool {
    [BACKUP_FILE, UNENCRYPTED_MANIFEST, UNENCRYPTED_FILES_DIR]
        .iter()
        .any(|name| dir.join(name).exists())
        || dir.join(format!("{BACKUP_FILE}.partial")).exists()
}

/// **The whole backup, against any [`CallsTools`] and any passphrase source.**
///
/// # Errors
///
/// A directory that already holds a backup; any refusal of [`list_store`] or
/// [`read_secret`]; a file too large to put back; identity files that
/// disagree with `--pin`; the passphrase prompt's own refusal; or a file that
/// cannot be written.
pub(crate) fn backup_via(
    args: &BackupArgs,
    caller: &dyn CallsTools,
    now_unix_seconds: u64,
    ask: Ask<'_>,
) -> Result<String, String> {
    let board_key = pin_of(&args.endpoint)?;
    let target = args.into.join(BACKUP_FILE);
    if holds_a_backup(&args.into) {
        return Err(format!(
            "{} already holds a backup (or part of one), so NOTHING WAS READ OR WRITTEN. \
             Replacing one backup with another is a retention decision and this verb makes none: \
             move it, or name a different --into.",
            args.into.display()
        ));
    }
    let host = args.endpoint.host.clone().unwrap_or_default();
    if host.chars().any(char::is_control) {
        return Err("--host carries a control character, which the manifest cannot record".to_string());
    }

    let listed = list_store(caller)?;
    if listed.is_empty() {
        return Err(format!("{STORE} on that board is empty; there is nothing to back up"));
    }
    let cap = rivet_core::operation::SECRET_MAX_BYTES as u64;
    if let Some(big) = listed.iter().find(|l| l.bytes > cap) {
        return Err(format!(
            "{STORE}/{} is {} bytes and `apply {RESTORE_SUBJECT}` puts back at most {cap}, so a \
             backup of it could never be restored — nothing was backed up.",
            big.name, big.bytes
        ));
    }

    // **The certificate is public** (`ADR-025` §2), so it is read on the plan
    // path too: a board whose certificate names another key is refused before
    // `confirm`, not after.
    if listed.iter().any(|l| l.name == rivetos_tls::CERTIFICATE_FILE) {
        let certificate = read_secret(caller, rivetos_tls::CERTIFICATE_FILE)?
            .ok_or_else(|| format!("{} was listed and then was not there", rivetos_tls::CERTIFICATE_FILE))?;
        check_identity_files(
            &[(rivetos_tls::CERTIFICATE_FILE.to_string(), certificate)],
            &board_key,
            "the board's",
        )?;
    }

    if !args.confirm {
        let mut out = format!(
            "{} file(s) in {STORE} on the board whose key is\n  {board_key}\n\n",
            listed.len()
        );
        for one in &listed {
            out.push_str(&format!("  {:<24} {:>6} bytes\n", one.name, one.bytes));
        }
        out.push_str(&format!(
            "\nWould fetch them and seal them, with a manifest recording that key, into\n\
             {} under a passphrase you type twice.\n\
             No secret was read and no passphrase was asked for. Add `confirm` to fetch and write.\n",
            target.display()
        ));
        return Ok(out);
    }

    // **The passphrase before the first secret crosses the wire**: a run that
    // cannot get one — no terminal, two entries that differ — has fetched
    // nothing it would then have to throw away.
    let passphrase = ask()?;

    // Everything is fetched and checked before the first byte is written, so a
    // refusal part way leaves no file that looks like a backup.
    let mut fetched: Vec<(String, Vec<u8>)> = Vec::new();
    for one in &listed {
        let bytes = read_secret(caller, &one.name)?.ok_or_else(|| {
            format!("{STORE}/{} was listed and then was not there; nothing was written", one.name)
        })?;
        fetched.push((one.name.clone(), bytes));
    }
    check_identity_files(&fetched, &board_key, "the board's")?;

    let manifest = Manifest {
        board_key: board_key.clone(),
        host,
        taken_unix_seconds: now_unix_seconds,
        files: fetched
            .iter()
            .map(|(name, bytes)| Entry {
                name: name.clone(),
                bytes: bytes.len() as u64,
                blake3: blake3_hex(bytes),
            })
            .collect(),
    };
    let sealed = envelope::seal(SECRETS_ENVELOPE, &archive(&manifest, &fetched), &passphrase)?;

    std::fs::create_dir_all(&args.into)
        .map_err(|e| format!("cannot create {}: {e}", args.into.display()))?;
    write_private(&target, &sealed)?;

    let mut out = format!(
        "backed up {} file(s) from the board whose key is\n  {board_key}\ninto {} ({} bytes)\n\n",
        manifest.files.len(),
        target.display(),
        sealed.len()
    );
    for file in &manifest.files {
        out.push_str(&format!("  {:<24} {:>6} bytes\n", file.name, file.bytes));
    }
    out.push_str(&format!(
        "\nENCRYPTED under the passphrase you just typed: the file names, sizes and the\n\
         board's key are inside it too. {}\n",
        envelope::FORGOTTEN
    ));
    Ok(out)
}

/// Write a file through `<name>.partial` and a rename, mode 0600 on Unix.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let partial = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.partial"),
        None => "partial".to_string(),
    });
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&partial)
        .map_err(|e| format!("cannot create {}: {e}", partial.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    file.sync_all()
        .map_err(|e| format!("cannot flush {}: {e}", partial.display()))?;
    drop(file);
    std::fs::rename(&partial, path).map_err(|e| {
        format!("cannot rename {} to {}: {e}", partial.display(), path.display())
    })
}

// ---------------------------------------------------------------------------
// restore-secrets
// ---------------------------------------------------------------------------

/// A backup read off disk, opened, and checked against its own manifest.
#[derive(Debug)]
struct Backup {
    manifest: Manifest,
    files: Vec<(String, Vec<u8>)>,
}

/// **Read a backup, open it, and refuse it unless every file is what the
/// manifest says.**
///
/// The passphrase is asked for only once the file is known to be a secrets
/// backup at all.
///
/// # Errors
///
/// No backup file, or the unencrypted layout; a file that is not this
/// envelope; the passphrase prompt's refusal; a wrong passphrase or an altered
/// file; an archive that does not describe its own files; or identity files
/// that disagree with the recorded board key.
fn read_backup(from: &Path, ask: Ask<'_>) -> Result<Backup, String> {
    let (at, bytes) = sealed(from)?;
    let parsed = envelope::parse(SECRETS_ENVELOPE, &bytes)
        .map_err(|e| format!("{}: {e} — refused, and NOTHING WAS SENT.", at.display()))?;
    let passphrase = ask()?;
    let plaintext = parsed
        .open(&passphrase)
        .map_err(|e| format!("{}: {e}\nNOTHING WAS SENT.", at.display()))?;
    let (manifest, files) = unarchive(&plaintext).map_err(|e| {
        format!(
            "{} is NOT the backup its manifest describes: {e}\nRefused, and NOTHING WAS SENT: \
             putting back part of a board's secrets is a board that is wrong in a way nothing \
             announces (ADR-030 §5).",
            at.display()
        )
    })?;
    check_identity_files(&files, &manifest.board_key, "the backup's").map_err(|e| {
        format!(
            "{e}. The backup's own files name a different board from the one its manifest \
             records, so it is not what it says it is — refused, and NOTHING WAS SENT."
        )
    })?;
    Ok(Backup { manifest, files })
}

/// **Everything about a backup directory that can be known without its
/// passphrase**: that it holds the one sealed file, that the file is this
/// envelope, and not the unencrypted layout an earlier build wrote. `cargo
/// rivet recover`'s plan reads this much, and asks for no passphrase.
///
/// # Errors
///
/// [`read_backup`]'s, up to the passphrase prompt.
pub(crate) fn check_sealed(from: &Path) -> Result<PathBuf, String> {
    let (at, bytes) = sealed(from)?;
    envelope::parse(SECRETS_ENVELOPE, &bytes)
        .map_err(|e| format!("{}: {e} — refused, and NOTHING WAS SENT.", at.display()))?;
    Ok(at)
}

/// **Which board a backup belongs to**, opened under the passphrase `ask`
/// answers with and checked against its own files exactly as a restore checks
/// it. `cargo rivet recover` asks this before a card is written, so a backup
/// of another board is refused before anything is written anywhere.
///
/// # Errors
///
/// [`read_backup`]'s.
pub(crate) fn recorded_board_key(from: &Path, ask: Ask<'_>) -> Result<String, String> {
    Ok(read_backup(from, ask)?.manifest.board_key)
}

/// The passphrase for an existing secrets backup, typed once at a real
/// terminal — the prompt `restore-secrets` shows, naming `verb`.
///
/// # Errors
///
/// No terminal, or the prompt's own refusal.
pub(crate) fn ask_at_the_terminal(verb: &str) -> Result<String, String> {
    envelope::read_existing(WHAT, verb)
}

/// The sealed file's path and bytes, refusing the unencrypted layout by name.
fn sealed(from: &Path) -> Result<(PathBuf, Vec<u8>), String> {
    let at = from.join(BACKUP_FILE);
    if !at.is_file() {
        if from.join(UNENCRYPTED_MANIFEST).exists() || from.join(UNENCRYPTED_FILES_DIR).exists() {
            return Err(format!(
                "{} holds a backup an earlier build wrote UNENCRYPTED ({UNENCRYPTED_MANIFEST} \
                 beside {UNENCRYPTED_FILES_DIR}/) — refused, and NOTHING WAS SENT. A board's \
                 secrets backup is passphrase-encrypted (R-185): take a new one with `cargo rivet \
                 backup-secrets`, then remove the old directory, which holds that board's secrets \
                 in the clear.",
                from.display()
            ));
        }
        return Err(format!(
            "{} holds no {BACKUP_FILE}, so it is not a finished `cargo rivet backup-secrets` \
             backup — refused, and NOTHING WAS SENT.",
            from.display()
        ));
    }
    let bytes = std::fs::read(&at).map_err(|e| format!("cannot read {}: {e}", at.display()))?;
    Ok((at, bytes))
}

/// What restore will do with one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Doing {
    /// The board has none.
    Write,
    /// The board has one and its bytes differ.
    Replace,
    /// The board has these exact bytes already.
    Identical,
}

impl Doing {
    const fn word(self) -> &'static str {
        match self {
            Self::Write => "write",
            Self::Replace => "REPLACE",
            Self::Identical => "identical, skipped",
        }
    }
}

/// `restore-secrets`, with a real connection and a real terminal behind it.
///
/// # Errors
///
/// See [`restore_via`].
pub fn restore_execute(args: &[String]) -> Result<String, String> {
    let parsed = parse_restore(args)?;
    let identity = parsed
        .endpoint
        .identity
        .as_ref()
        .ok_or_else(|| format!("--identity is required.\n\n{}", restore_usage()))?;
    let operator_key = rivetos_tls::read_identity(identity)?
        .public_key()
        .map(|pin| rivetos_tls::encode_hex(&pin))
        .ok_or("the --identity certificate carries no Ed25519 public key")?;
    let transport = open(&parsed.endpoint, &restore_usage())?;
    restore_via(&parsed, &transport, &operator_key, &|| {
        envelope::read_existing(WHAT, "cargo rivet restore-secrets")
    })
}

/// **The whole restore, against any [`CallsTools`] and any passphrase
/// source.** `operator_key` is the public key `--identity` connects with, as
/// lower-case hex.
///
/// # Errors
///
/// Every refusal in this module's documentation, then the board's own.
pub(crate) fn restore_via(
    args: &RestoreArgs,
    caller: &dyn CallsTools,
    operator_key: &str,
    ask: Ask<'_>,
) -> Result<String, String> {
    let target_key = pin_of(&args.endpoint)?;
    // Everything about the backup itself is settled before the board is asked
    // anything.
    let backup = read_backup(&args.from, ask)?;
    let recorded = &backup.manifest.board_key;

    if *recorded != target_key {
        return Err(format!(
            "REFUSED — THIS BACKUP IS FROM A DIFFERENT BOARD, and NOTHING WAS SENT.\n\
             \x20 backup taken from  {recorded}\n\
             \x20 --pin names        {target_key}\n\
             \x20 why  a backup of /state/secrets goes back onto the board it came from and no \
             other (ADR-030 §5, R-183 D5, R-185 D2). It carries that board's device seed: put onto \
             another board it makes two boards one identity to every peer that pins it. A board \
             is the same board only if its device identity key matches — one re-carded with a new \
             identity is a different board; provision its secrets anew rather than restoring \
             another's."
        ));
    }

    if let Some((_, peers)) = backup.files.iter().find(|(name, _)| name == PEERS) {
        let keys = rivet_core::operation::restore_secret::parse_peers_grammar(peers).map_err(|why| {
            format!(
                "REFUSED — the backup's {PEERS} {why}, and NOTHING WAS SENT. rivet-mcpd reads \
                 {STORE}/{PEERS} when it starts and refuses the WHOLE file on a line like this, \
                 which would exit before its listener binds and leave the board answering nobody \
                 after the next boot."
            )
        })?;
        let admits = keys.iter().any(|line| line.eq_ignore_ascii_case(operator_key));
        if !admits {
            return Err(format!(
                "REFUSED — the backup's {PEERS} does not name the key this workstation connects \
                 with ({operator_key}), and NOTHING WAS SENT. rivet-mcpd reads {STORE}/{PEERS} \
                 when it starts, so after the board's next boot it would admit nobody who could \
                 undo this. Connect with an identity the backup's {PEERS} names."
            ));
        }
    }

    // What the board holds now.
    let listed = list_store_for_restore(caller)?;
    let mut plan: Vec<(&str, &[u8], Doing)> = Vec::new();
    for (name, bytes) in &backup.files {
        let doing = if listed.iter().any(|l| l == name) {
            match read_secret(caller, name)? {
                None => Doing::Write,
                Some(now) if now == *bytes => Doing::Identical,
                Some(_) => Doing::Replace,
            }
        } else {
            Doing::Write
        };
        plan.push((name.as_str(), bytes.as_slice(), doing));
    }
    let untouched: Vec<&String> = listed
        .iter()
        .filter(|name| !backup.files.iter().any(|(n, _)| n == *name))
        .collect();

    let mut table = String::new();
    for (name, bytes, doing) in &plan {
        table.push_str(&format!("  {name:<24} {:>6} bytes  {}\n", bytes.len(), doing.word()));
    }
    for name in &untouched {
        table.push_str(&format!("  {name:<24}               on the board, not in this backup: left alone\n"));
    }

    let replacing: Vec<&str> = plan
        .iter()
        .filter(|(_, _, d)| *d == Doing::Replace)
        .map(|(n, _, _)| *n)
        .collect();
    if !replacing.is_empty() && !args.replace {
        return Err(format!(
            "the board holds different bytes for {} — NOTHING WAS WRITTEN.\n{table}\n\
             Replacing a secret destroys the one there and nothing on the board can return it. \
             Pass --replace to put the backup's bytes over them.",
            replacing.join(", ")
        ));
    }

    let header = format!(
        "backup taken from the board whose key is {recorded}\n\
         (this board: the key --pin names and the handshake proves — the same)\n\n"
    );
    if !args.confirm {
        return Ok(format!(
            "{header}{table}\n\
             Each write is `apply {RESTORE_SUBJECT}`: root-owned, mode 0600, renamed into place.\n\
             NOTHING WILL BE RESTARTED — rivet-mcpd and rivet-netd read these when they start, so\n\
             what is written is live at the board's next boot, except peers, which rivet-mcpd\n\
             reloads for the next connection.\n\
             Nothing was written. Add `confirm`.\n"
        ));
    }

    let mut report = header;
    for (name, bytes, doing) in &plan {
        if *doing == Doing::Identical {
            report.push_str(&format!("  {name:<24} identical on the board — not written\n"));
            continue;
        }
        let hex = rivet_core::package::key_material::to_hex(bytes);
        let words = ["apply", RESTORE_SUBJECT, name, "--hex", &hex, "confirm"];
        let answer = rivet_remote::command::perform(caller, &words).map_err(|e| {
            format!(
                "{report}\n{name}: the board refused: {e}\n\
                 What is listed above as written is written; nothing rolls back."
            )
        })?;
        check_answer(&answer, name, bytes.len())?;
        // **Read back**, so the report states what is on the board rather than
        // what was sent.
        let back = read_secret(caller, name)?;
        if back.as_deref() != Some(*bytes) {
            return Err(format!(
                "{report}\n{name}: written, and reading it back does not give the bytes sent. \
                 Do not reboot the board until this is understood."
            ));
        }
        report.push_str(&format!(
            "  {name:<24} {:>6} bytes  {} and read back identical\n",
            bytes.len(),
            if *doing == Doing::Replace { "REPLACED" } else { "written" }
        ));
    }
    for name in &untouched {
        report.push_str(&format!("  {name:<24} not in this backup — left alone\n"));
    }
    report.push_str(
        "\nNOTHING WAS RESTARTED. rivet-mcpd and rivet-netd hold what they read when they\n\
         started; what was written is live at the board's next boot, except peers, which\n\
         rivet-mcpd has already reloaded for the next connection unless its journal says\n\
         it refused. That the board's programs accept these files is proven by that boot,\n\
         not by this report.\n",
    );
    Ok(report)
}

/// The names in the store, for comparison. Unlike [`list_store`], an entry
/// that is not a file is reported by name only when the backup would write
/// over it — the board's own subject refuses that too.
fn list_store_for_restore(caller: &dyn CallsTools) -> Result<Vec<String>, String> {
    let json = get_file(caller, STORE)?;
    if json.get("present") != Some(&Json::Bool(true)) {
        return Err(format!(
            "there is no {STORE} on that board, and `apply {RESTORE_SUBJECT}` does not make one — \
             a board without it was never given an identity"
        ));
    }
    let entries = json
        .get("entries")
        .and_then(Json::as_array)
        .ok_or_else(|| format!("the board's listing of {STORE} carries no entries"))?;
    if json.get("entries_truncated") != Some(&Json::Bool(false)) {
        return Err(format!("the board cut its listing of {STORE} short"));
    }
    Ok(entries
        .iter()
        .filter_map(|e| e.get("name").and_then(Json::as_str).map(str::to_string))
        .collect())
}

/// The board's answer to one `apply restore-secret`, checked to be about the
/// file that was sent and to have decoded every byte.
fn check_answer(answer: &str, name: &str, sent: usize) -> Result<(), String> {
    let json = rivet_core::json::parse(answer)
        .map_err(|why| format!("{name}: the board's answer is not JSON ({why})"))?;
    if json.get("name").and_then(Json::as_str) != Some(name) {
        return Err(format!("{name}: the board's answer is about another file: {answer}"));
    }
    let wrote = json.get("bytes").and_then(Json::as_integer);
    if wrote != i64::try_from(sent).ok() {
        return Err(format!(
            "{name}: {sent} bytes were sent and the board reports {wrote:?}; the file on the board \
             is not the backup's"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "machine_secrets_tests.rs"]
mod tests;
