//! `cargo rivet fleet bookmark add | list | remove` — **how this workstation
//! reaches each fleet, and nothing the fleet could contradict.** `ADR-063` §2.
//!
//! A bookmark is a fleet's name, its server's address, that server's pin, and
//! which of this workstation's operator identities the fleet knows. Members,
//! artifacts and what each machine runs are read from the fleet server when
//! they are needed; a bookmark holds none of them, so it cannot go stale about
//! any of them. Losing every bookmark costs retyping one address and one pin
//! per fleet.
//!
//! # Where they are
//!
//! `~/.rivet/bookmarks.toml`, one `[fleet.<name>]` table each:
//!
//! ```toml
//! [fleet.dev-fleet]
//! server   = "dev-01"
//! host     = "dev-01.local:7777"
//! pin      = "<64 hex>"
//! identity = "<this workstation's operator identity directory>"
//! ```
//!
//! and a fleet's fetch-only clone is `~/.rivet/fleets/<name>/` beside it
//! (`clone_dir`). Both are this workstation's own files and neither is
//! tracked anywhere. They are apart from `~/.rivet/fleet`, the repository
//! `ADR-052` put on the workstation, and from `rivet-fleet`'s register: a
//! bookmark is read from the register once, when it is added, and neither is
//! written by anything here.
//!
//! # A DHCP server is bookmarked by name
//!
//! A register record that carries no address is a DHCP machine, and its
//! bookmark keeps `<name>.local:<port>` rather than the address it resolves to
//! today: the transport resolves `.local` over mDNS at every connection
//! (`rivet_remote::tls`), and the pin, not the address, is the trust.

use std::path::{Path, PathBuf};

use rivet_remote::endpoint::Endpoint;
use toml_edit::{DocumentMut, Item, Table, value};

/// The file, inside `~/.rivet`.
pub(super) const FILE: &str = "bookmarks.toml";

/// The directory each fleet's clone is in, inside `~/.rivet`.
pub(super) const CLONES: &str = "fleets";

/// The table every bookmark sits under.
const TABLE: &str = "fleet";

/// One fleet, as this workstation reaches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Bookmark {
    /// The fleet's name — one DNS label, the name its root and its
    /// `home.arpa` zone carry.
    pub fleet: String,
    /// The fleet server's host name, for a person reading the report.
    pub server: String,
    /// `HOST:PORT` of the server's control plane.
    pub host: String,
    /// The server's pinned key.
    pub pin: String,
    /// The operator identity directory this fleet knows.
    pub identity: PathBuf,
}

impl Bookmark {
    /// The endpoint every verb that reaches this fleet opens.
    pub(super) fn endpoint(&self) -> Endpoint {
        Endpoint {
            host: Some(self.host.clone()),
            identity: Some(self.identity.clone()),
            pin: Some(self.pin.clone()),
        }
    }
}

/// `~/.rivet/bookmarks.toml`.
///
/// # Errors
///
/// Neither `HOME` nor `USERPROFILE` is set.
pub(super) fn default_file() -> Result<PathBuf, String> {
    crate::credentials::root()
        .map(|root| root.join(FILE))
        .ok_or_else(|| "this host sets neither HOME nor USERPROFILE, so there is no ~/.rivet".to_owned())
}

/// `~/.rivet/fleets/<fleet>` — a fleet's fetch-only clone.
///
/// # Errors
///
/// Neither `HOME` nor `USERPROFILE` is set.
pub(super) fn clone_dir(fleet: &str) -> Result<PathBuf, String> {
    crate::credentials::root()
        .map(|root| root.join(CLONES).join(fleet))
        .ok_or_else(|| "this host sets neither HOME nor USERPROFILE, so there is no ~/.rivet".to_owned())
}

/// A fleet's name is one DNS label, which is also a safe directory name.
///
/// # Errors
///
/// Anything else, named.
pub(super) fn check_name(fleet: &str) -> Result<(), String> {
    if rivet_core::fleet_root::is_label(fleet) {
        Ok(())
    } else {
        Err(format!(
            "{fleet:?} is not a fleet name: one DNS label — lower-case letters, digits and hyphens, \
             at most 63, not beginning or ending with a hyphen"
        ))
    }
}

/// Every bookmark in `file`, in the file's order. An absent file holds none.
///
/// # Errors
///
/// A file that cannot be read, is not TOML, or holds a bookmark missing a
/// field — refused whole, naming the fleet, rather than read around.
pub(super) fn read(file: &Path) -> Result<Vec<Bookmark>, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let doc: DocumentMut = text.parse().map_err(|e| format!("{} is not TOML: {e}", file.display()))?;
    let Some(fleets) = doc.get(TABLE) else {
        return Ok(Vec::new());
    };
    let fleets = fleets
        .as_table()
        .ok_or_else(|| format!("{}: `{TABLE}` is not a table of bookmarks", file.display()))?;
    let mut out = Vec::new();
    for (fleet, item) in fleets {
        let table = item
            .as_table()
            .ok_or_else(|| format!("{}: [{TABLE}.{fleet}] is not a table", file.display()))?;
        let field = |key: &str| {
            table
                .get(key)
                .and_then(Item::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("{}: [{TABLE}.{fleet}] has no string `{key}`", file.display()))
        };
        check_name(fleet).map_err(|why| format!("{}: {why}", file.display()))?;
        out.push(Bookmark {
            fleet: fleet.to_owned(),
            server: field("server")?,
            host: field("host")?,
            pin: field("pin")?,
            identity: PathBuf::from(field("identity")?),
        });
    }
    Ok(out)
}

/// The file's text for `bookmarks`.
pub(super) fn render(bookmarks: &[Bookmark]) -> String {
    let mut doc = DocumentMut::new();
    let mut fleets = Table::new();
    fleets.set_implicit(true);
    for one in bookmarks {
        let mut table = Table::new();
        table.insert("server", value(one.server.as_str()));
        table.insert("host", value(one.host.as_str()));
        table.insert("pin", value(one.pin.as_str()));
        table.insert("identity", value(one.identity.display().to_string()));
        fleets.insert(&one.fleet, Item::Table(table));
    }
    doc.insert(TABLE, Item::Table(fleets));
    format!(
        "# The fleets this workstation reaches — ADR-063 §2. Written by `cargo rivet fleet \
         bookmark`;\n# each fleet's server is the authority for everything else about it.\n\n{doc}"
    )
}

/// The one bookmark named `fleet`.
///
/// # Errors
///
/// [`read`]'s, or no bookmark of that name — listing the ones there are.
pub(super) fn find(file: &Path, fleet: &str) -> Result<Bookmark, String> {
    let all = read(file)?;
    if let Some(found) = all.iter().find(|b| b.fleet == fleet) {
        return Ok(found.clone());
    }
    let names: Vec<&str> = all.iter().map(|b| b.fleet.as_str()).collect();
    Err(format!(
        "this workstation has no bookmark for a fleet named {fleet:?} ({}). `cargo rivet fleet \
         bookmark add {fleet} <server>` adds one.",
        if names.is_empty() { format!("{} holds none", file.display()) } else { format!("it has: {}", names.join(", ")) }
    ))
}

/// **A bookmark for `fleet`, from `server`'s register record** — its address,
/// or `<name>.local:<port>` for a record that carries none, and its pin —
/// with `given`'s `--host`/`--identity`/`--pin` winning, and `identity`
/// defaulting to `default_identity`. Pure over an already-loaded register.
///
/// # Errors
///
/// `server` answers to no record, the record's address is ambiguous, or there
/// is no pin and none was given.
pub(super) fn from_register(
    fleet: &str,
    server: &str,
    register: &rivetos_register::Register,
    given: &Endpoint,
    default_identity: Option<PathBuf>,
) -> Result<Bookmark, String> {
    check_name(fleet)?;
    let record = register.resolve(server).ok();
    let host = match (&given.host, record) {
        (Some(host), _) => host.clone(),
        // `dial_host` is the record's one address, or `<name>.local` for a
        // record carrying none, and refuses several.
        (None, Some(device)) => format!("{}:{}", device.dial_host()?, device.control_plane_port()),
        (None, None) => {
            return Err(format!(
                "{server:?} answers to no record in the register, and no --host was given, so \
                 there is no address to bookmark. Nothing was written."
            ));
        }
    };
    let pin = match given.pin.clone().or_else(|| record.and_then(|d| d.pin.clone()).filter(|p| !p.is_empty())) {
        Some(pin) => pin,
        None => {
            return Err(format!(
                "{server} has no control-plane key in the register and no --pin was given, so \
                 there is nothing to pin (ADR-025 §2). Nothing was written."
            ));
        }
    };
    let identity = given.identity.clone().or(default_identity).ok_or_else(|| {
        "no operator identity: pass --identity DIR (HOME and USERPROFILE are both unset)".to_owned()
    })?;
    Ok(Bookmark { fleet: fleet.to_owned(), server: server.to_owned(), host, pin, identity })
}

/// **Write `bookmark` into `file`.** Adding the same bookmark again changes
/// nothing; a different one for a fleet already bookmarked is refused unless
/// `replace`, because a bookmark's pin is a trust decision.
///
/// # Errors
///
/// [`read`]'s, the refusal above, or the file cannot be written.
pub(super) fn add(file: &Path, bookmark: &Bookmark, replace: bool) -> Result<String, String> {
    let mut all = read(file)?;
    let shown = describe(bookmark);
    if let Some(existing) = all.iter_mut().find(|b| b.fleet == bookmark.fleet) {
        if existing == bookmark {
            return Ok(format!("have  {shown}\nAlready bookmarked. Nothing was changed.\n"));
        }
        if !replace {
            return Err(format!(
                "{} is already bookmarked differently:\n  now   {}\n  new   {shown}\nNothing was \
                 changed. --replace writes the new one.",
                bookmark.fleet,
                describe(existing),
            ));
        }
        *existing = bookmark.clone();
    } else {
        all.push(bookmark.clone());
    }
    write(file, &all)?;
    Ok(format!("MADE  {shown}\nin    {}\n", file.display()))
}

/// Remove the bookmark for `fleet`. Its clone is left where it is.
///
/// # Errors
///
/// [`read`]'s, `fleet` is not bookmarked, or the file cannot be written.
pub(super) fn remove(file: &Path, fleet: &str) -> Result<String, String> {
    let removed = find(file, fleet)?;
    let rest: Vec<Bookmark> = read(file)?.into_iter().filter(|b| b.fleet != fleet).collect();
    write(file, &rest)?;
    Ok(format!(
        "REMOVED  {}\nIts clone, if it has one, is left where it is.\n",
        describe(&removed)
    ))
}

/// Every bookmark, one line each.
///
/// # Errors
///
/// [`read`]'s.
pub(super) fn list(file: &Path) -> Result<String, String> {
    let all = read(file)?;
    if all.is_empty() {
        return Ok(format!(
            "no bookmarks in {}. `cargo rivet fleet bookmark add <fleet> <server>` adds one.\n",
            file.display()
        ));
    }
    Ok(all.iter().map(|b| format!("{}\n", describe(b))).collect())
}

fn describe(b: &Bookmark) -> String {
    format!(
        "{}  server {} at {}, pin {}…, identity {}",
        b.fleet,
        b.server,
        b.host,
        b.pin.get(..16).unwrap_or(&b.pin),
        b.identity.display()
    )
}

fn write(file: &Path, all: &[Bookmark]) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let staged = file.with_extension("toml.new");
    std::fs::write(&staged, render(all)).map_err(|e| format!("cannot write {}: {e}", staged.display()))?;
    std::fs::rename(&staged, file).map_err(|e| format!("cannot replace {}: {e}", file.display()))
}

// ---------------------------------------------------------------------------
// the command line
// ---------------------------------------------------------------------------

/// What `fleet bookmark` was asked to do.
#[derive(Debug)]
pub(super) enum BookmarkArgs {
    Add { fleet: String, server: String, endpoint: Endpoint, replace: bool },
    List,
    Remove { fleet: String },
}

pub(super) fn usage() -> String {
    "usage: cargo rivet fleet bookmark add <fleet> <server> [--host HOST:PORT] [--identity DIR]\n\
     \x20                                   [--pin HEX] [--replace]\n\
     \x20      cargo rivet fleet bookmark list\n\
     \x20      cargo rivet fleet bookmark remove <fleet>\n\
     \n\
     ADR-063 §2 — how this workstation reaches a fleet: its name, its server's\n\
     address and pin, and which operator identity it knows, in ~/.rivet/bookmarks.toml.\n\
     `add` reads <server>'s address and pin from rivet-fleet's register (a record with\n\
     no address is kept as <server>.local) and writes nothing to the register; the\n\
     flags win over it. A different bookmark for a fleet already bookmarked is\n\
     refused without --replace. The fleet's clone is ~/.rivet/fleets/<fleet>/.\n"
        .to_owned()
}

/// `args[0]` is `"bookmark"`.
///
/// # Errors
///
/// A missing or unknown action, a missing name, an unknown flag, or half of
/// `--identity`/`--pin`.
pub(super) fn parse(args: &[String]) -> Result<BookmarkArgs, String> {
    let Some(action) = args.get(1).map(String::as_str) else {
        return Err(format!("`fleet bookmark` needs an action: add, list or remove.\n\n{}", usage()));
    };
    let mut endpoint = Endpoint::default();
    let mut replace = false;
    let mut words: Vec<String> = Vec::new();
    let mut index = 2;
    while index < args.len() {
        if action == "add"
            && let Some(next) = endpoint.read(args, index)?
        {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "-h" | "--help" => return Err(usage()),
            "--replace" if action == "add" => {
                replace = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!("{other:?} is not an option of `fleet bookmark {action}`\n\n{}", usage()));
            }
            other => {
                words.push(other.to_owned());
                index += 1;
            }
        }
    }
    endpoint.paired()?;
    match (action, words.as_slice()) {
        ("add", [fleet, server]) => {
            check_name(fleet)?;
            Ok(BookmarkArgs::Add { fleet: fleet.clone(), server: server.clone(), endpoint, replace })
        }
        ("list", []) => Ok(BookmarkArgs::List),
        ("remove", [fleet]) => Ok(BookmarkArgs::Remove { fleet: fleet.clone() }),
        ("-h" | "--help", _) => Err(usage()),
        ("add" | "list" | "remove", _) => Err(format!("`fleet bookmark {action}` was given {words:?}.\n\n{}", usage())),
        (other, _) => Err(format!("`fleet bookmark {other}` is not an action: add, list or remove.\n\n{}", usage())),
    }
}

/// # Errors
///
/// Each action's own.
pub(super) fn run(args: &BookmarkArgs) -> Result<String, String> {
    let file = default_file()?;
    match args {
        BookmarkArgs::Add { fleet, server, endpoint, replace } => {
            let register_root = crate::device::default_register_dir()?;
            let register = rivetos_register::store::load(&register_root).map_err(|e| {
                format!("could not read the register at {}: {e}. Nothing was written.", register_root.display())
            })?;
            let bookmark =
                from_register(fleet, server, &register, endpoint, crate::device::default_identity_dir())?;
            add(&file, &bookmark, *replace)
        }
        BookmarkArgs::List => list(&file),
        BookmarkArgs::Remove { fleet } => remove(&file, fleet),
    }
}

#[cfg(test)]
#[path = "fleet_bookmark_tests.rs"]
mod tests;
