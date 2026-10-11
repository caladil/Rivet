//! **A fleet server's repository objects, received over the control plane** —
//! `.rulings/` R-182, R-184 D1 and D2; `ADR-052` §7.
//!
//! Four routes, beside `crate::bulk`'s `PUT /payload/` and `crate::git`'s
//! smart HTTP on the same listener:
//!
//! ```text
//! PUT  /repository-objects/<ADR-027 name>   Rivet-Blake3, Content-Length   one object
//! GET  /repository-objects/                                                 the listing
//! GET  /repository-objects/<digest>                                         one object, read back
//! POST /repository-objects/prune                                            remove what the manifest stops naming
//! ```
//!
//! The fourth is how a workstation's fetch-only clone takes the objects
//! (`ADR-063` §6); [`object`] opens it and the composition root streams it.
//!
//! # Where the store is, and why this daemon writes it
//!
//! In `rivet-fleetd`'s own data directory, apart from `/state/artifacts`
//! (R-184 D1) — `rivet_core::repository_objects::store`. This daemon and not
//! `rivet-fleetd` receives into it, because a confined application can read
//! neither `/system/trust` nor `/state/git/fleet.git`, and the store's two
//! rules need both; `rivet_core::repository_objects`'s module documentation
//! carries the argument. Each object file is then handed to `rivet-fleetd`'s
//! uid, and the directories stay root's with that application's group as its
//! read grant ([`hand_over`]), so the application whose directory it is can
//! read what the store holds and cannot plant anything where root writes.
//!
//! # What a transfer passes
//!
//! `crate::bulk::accept_under` — a name of `ADR-027`'s shape, a digest, a
//! length, a size ceiling — and then `crate::bulk::receive_onto` with
//! [`crate::bulk::TrustStoreDoor::mirror_on_this_machine`]: the body's BLAKE3
//! against the header as it streams to disk, then `rivet_core::package::verify`
//! against this machine's trust store, then the kind. Nothing is held in memory
//! beyond one 64 KiB buffer and the 4 KiB header, whatever the object's size.
//! Only after all of it does `rename(2)` name the object.
//!
//! # What decides a prune
//!
//! The `fleet` repository's `main:manifest.toml`, read here with git's own
//! plumbing (`rev-parse`, then `show <commit>:manifest.toml`) out of
//! `/state/git/fleet.git` — the text `cargo rivet fleet push` last pushed, and
//! never the copy a card carried, which is frozen on the day the card was
//! written. A prune removes only what that manifest does not name, and is
//! refused while anything it names is missing (`rivet_core::repository_objects::
//! Listing::prune`).

use std::path::Path;
use std::process::{Command, Stdio};

use rivet_core::artifact;
use rivet_core::repository_objects::{self as objects, Access, Caller, Held, Listing, Manifest};

use crate::beneath::{self, Dir};
use crate::bulk::{self, Door, Request, Response, Shelf};

/// Whether a request target reaches this module.
#[must_use]
pub fn is_repository_objects(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target);
    path.starts_with(objects::PREFIX) || path == objects::PREFIX.trim_end_matches('/')
}

/// Where this machine keeps the store, the repository and the git that reads it.
///
/// **Every path a parameter**, for `rivet_core::artifact::Store`'s reason: this
/// code removes directories, and a test that reached a machine's real paths
/// would remove a fleet server's objects.
#[derive(Debug, Clone)]
pub struct Machine {
    /// `rivet-fleetd`'s data directory. Its absence is a refusal.
    data_dir: String,
    store: artifact::Store,
    /// The bare `fleet` repository.
    repository: String,
    git: String,
    /// `Some(PATH)`: git runs with a replaced environment carrying only this
    /// `PATH`. `None` inherits this process's environment, for a test on a
    /// host whose git is wherever that host put it.
    git_path: Option<String>,
    /// Who the store's entries are handed to, on a platform that has owners.
    owner: Option<(u32, u32)>,
}

impl Machine {
    /// The store on this machine: `rivet-fleetd` on the built-in volume.
    ///
    /// **`BUILT_IN_VOLUME`, and that marks a place `ADR-031` will reach** — the
    /// same gap `rivet_core::service::app_root` names for every caller passing
    /// it. A fleet server's carried `rivet-fleetd` is installed there.
    #[must_use]
    pub fn on_this_machine() -> Self {
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        let owner = rivet_core::service::ServiceName::new(objects::APPLICATION)
            .map(|name| rivet_core::service::app_identity(&name))
            .map(|id| (id.uid, id.gid));
        Self {
            data_dir: objects::data_dir(volume),
            store: objects::store(volume),
            repository: rivet_core::git::repository_dir(objects::REPOSITORY),
            git: rivet_core::git::BINARY.to_owned(),
            git_path: Some(
                rivet_core::git::BINARY
                    .rsplit_once('/')
                    .map_or(rivet_core::git::BINARY, |(dir, _)| dir)
                    .to_owned(),
            ),
            owner,
        }
    }

    /// A store in `data_dir`, a repository at `repository`, and whichever `git`
    /// this host runs. For a test.
    #[cfg(test)]
    #[must_use]
    pub fn at(data_dir: &Path, repository: &Path) -> Self {
        let data = data_dir.to_string_lossy().into_owned();
        Self {
            store: artifact::Store::at(format!("{data}/{}", objects::DIR)),
            data_dir: data,
            repository: repository.to_string_lossy().into_owned(),
            git: "git".to_owned(),
            git_path: None,
            owner: None,
        }
    }

    /// The store this machine writes. For a test.
    #[cfg(test)]
    #[must_use]
    pub const fn store(&self) -> &artifact::Store {
        &self.store
    }

    /// Who [`hand_over`] gives the store to. For a test: [`Machine::on_this_machine`]
    /// derives this from [`objects::APPLICATION`]'s real identity, which a
    /// scratch directory does not have.
    #[cfg(test)]
    #[must_use]
    pub fn owned_by(mut self, uid: u32, gid: u32) -> Self {
        self.owner = Some((uid, gid));
        self
    }
}

/// **One request to the store**, from its head to its answer.
///
/// `caller` is `None` when the connection presented no pinned key — the
/// plaintext loopback transport — and such a caller is refused: R-184 D2 names
/// the operator, and a connection that named nobody is not the operator.
pub fn serve(
    machine: &Machine,
    door: &dyn Door,
    request: &Request,
    already: &[u8],
    more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>,
    caller: Option<Caller>,
) -> Response {
    let path = request.target.split('?').next().unwrap_or(&request.target);
    let listing_path = objects::PREFIX.trim_end_matches('/');
    let (access, route) = match request.method.as_str() {
        "GET" if path == objects::PREFIX || path == listing_path => (Access::Read, Route::List),
        "POST" if path == objects::PRUNE => (Access::Write, Route::Prune),
        "PUT" if path.starts_with(objects::PREFIX) && path != objects::PRUNE => {
            (Access::Write, Route::Put)
        }
        _ => {
            return refuse(
                405,
                "Method Not Allowed",
                format!(
                    "{} {} is not a route of a fleet server's repository store. It serves PUT \
                     {prefix}<name>.rvt, GET {prefix}, GET {prefix}<digest> and POST {prune} \
                     (.rulings/ R-184 D1, ADR-063 §6).\n",
                    request.method,
                    request.target,
                    prefix = objects::PREFIX,
                    prune = objects::PRUNE,
                ),
            );
        }
    };

    let Some(caller) = caller else {
        return refuse(
            403,
            "Forbidden",
            "this connection presented no pinned key, and a fleet server's repository store \
             answers only the operator (.rulings/ R-184 D2). Connect over the control plane's \
             TLS with an identity this machine's peers file names.\n"
                .to_owned(),
        );
    };
    if !objects::permits(caller, access) {
        return refuse(
            403,
            "Forbidden",
            format!("{caller:?} may not {access:?} a fleet server's repository store (.rulings/ R-184 D2).\n"),
        );
    }

    if !Path::new(&machine.data_dir).is_dir() {
        return refuse(
            404,
            "Not Found",
            format!(
                "{} does not exist, so this machine keeps no repository objects. They live in \
                 {}'s own data directory (.rulings/ R-184 D1), and this machine creates nothing \
                 for an application it does not hold: a fleet server installs {} from its card \
                 at first boot.\n",
                machine.data_dir,
                objects::APPLICATION,
                objects::APPLICATION,
            ),
        );
    }

    match route {
        Route::Put => put(machine, door, request, already, more),
        Route::List => {
            let listing = listing(machine);
            Response { status: 200, reason: "OK", body: listing.render() }
        }
        Route::Prune => prune(machine),
    }
}

/// **One held object, opened to be read back** — `GET
/// /repository-objects/<digest>` (`ADR-063` §6), the route a workstation's
/// fetch-only clone reads through.
///
/// Answered apart from [`serve`] because an object is binary and as large as a
/// system update: `crate::bulk::Response` carries a `String` body, so the
/// composition root writes [`object_head`] and then the file in pieces, as it
/// writes git's answer.
#[derive(Debug)]
pub struct Object {
    pub file: std::fs::File,
    /// The `ADR-027` name it arrived under.
    pub name: String,
    pub length: u64,
    pub digest: artifact::Digest,
}

/// **The object a request asks for, opened**, or the refusal to send instead —
/// `None` when the request is not `GET /repository-objects/<digest>` at all.
///
/// The same caller rule as every other route (a pinned key, and
/// [`objects::permits`] for [`Access::Read`]), then the object's directory and
/// its one file, each read with `symlink_metadata`: the store sits inside
/// `rivet-fleetd`'s own data directory, which that application can write, and
/// this daemon runs as root, so a link planted there would otherwise hand the
/// caller whatever file it pointed at. The digest is the name the door admitted
/// the object under, after checking the body against it; the reader checks the
/// bytes again.
#[must_use]
pub fn object(machine: &Machine, request: &Request, caller: Option<Caller>) -> Option<Result<Object, Response>> {
    if request.method != "GET" {
        return None;
    }
    let path = request.target.split('?').next().unwrap_or(&request.target);
    let digest = objects::requested_object(path)?;
    Some(open_object(machine, digest, caller))
}

fn open_object(machine: &Machine, digest: artifact::Digest, caller: Option<Caller>) -> Result<Object, Response> {
    let Some(caller) = caller else {
        return Err(refuse(
            403,
            "Forbidden",
            "this connection presented no pinned key, and a fleet server's repository store \
             answers only the operator (.rulings/ R-184 D2).\n"
                .to_owned(),
        ));
    };
    if !objects::permits(caller, Access::Read) {
        return Err(refuse(
            403,
            "Forbidden",
            format!("{caller:?} may not read a fleet server's repository store (.rulings/ R-184 D2).\n"),
        ));
    }
    let not_held = || {
        refuse(
            404,
            "Not Found",
            format!("this store holds no object {digest}. `GET {}` lists what it holds.\n", objects::PREFIX),
        )
    };
    let dir = machine.store.object_dir(&digest);
    let dir_meta = std::fs::symlink_metadata(&dir).map_err(|_| not_held())?;
    if !dir_meta.is_dir() {
        return Err(refuse(
            500,
            "Internal Server Error",
            format!("{dir} is not a real directory, so nothing is read from it\n"),
        ));
    }
    let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)
        .map_err(|e| refuse(500, "Internal Server Error", format!("cannot read {dir}: {e}\n")))?
        .flatten()
        .collect();
    let [entry] = entries.as_slice() else {
        return Err(refuse(
            500,
            "Internal Server Error",
            format!("{dir} holds {} entries where the store keeps one object, so none is sent\n", entries.len()),
        ));
    };
    let name = entry.file_name().to_string_lossy().into_owned();
    let path = entry.path();
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|e| refuse(500, "Internal Server Error", format!("cannot read {}: {e}\n", path.display())))?;
    if !meta.file_type().is_file() {
        return Err(refuse(
            500,
            "Internal Server Error",
            format!("{} is not a regular file (a link or a directory), so it is not sent\n", path.display()),
        ));
    }
    let file = std::fs::File::open(&path)
        .map_err(|e| refuse(500, "Internal Server Error", format!("cannot open {}: {e}\n", path.display())))?;
    Ok(Object { file, name, length: meta.len(), digest })
}

/// The head an [`Object`] is sent under: its length, its digest as
/// `Rivet-Blake3` and its name as [`objects::NAME_HEADER`].
#[must_use]
pub fn object_head(object: &Object) -> String {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {}\r\n\
         Rivet-Blake3: {}\r\n\
         {}: {}\r\n\
         Connection: close\r\n\
         \r\n",
        object.length,
        object.digest,
        objects::NAME_HEADER,
        object.name,
    )
}

/// Which of the three a request is.
enum Route {
    Put,
    List,
    Prune,
}

fn refuse(status: u16, reason: &'static str, body: String) -> Response {
    Response { status, reason, body }
}

/// `PUT /repository-objects/<name>`.
fn put(
    machine: &Machine,
    door: &dyn Door,
    request: &Request,
    already: &[u8],
    more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>,
) -> Response {
    let (name, expected, length) = match bulk::accept_under(request, objects::PREFIX) {
        Ok(accepted) => accepted,
        Err(response) => return response,
    };
    // `receive_onto` creates the store's root itself, below the data directory
    // and without following a link planted in its place (`crate::beneath`).
    let answer = bulk::receive_onto(
        &machine.store,
        door,
        Shelf::Repository,
        &name,
        expected,
        length,
        already,
        more,
    );
    let handed = hand_over(machine);
    match answer {
        Ok(mut response) => {
            response.body.push_str(&handed);
            response
        }
        Err(response) => response,
    }
}

/// The mode of every directory in the store: root's, readable and traversable
/// by `rivet-fleetd`'s group, nothing for anybody else.
const DIRECTORY_MODE: u32 = 0o750;

/// The owner of every directory in the store.
const ROOT_UID: u32 = 0;

/// **Hand the store's object files to `rivet-fleetd`, and keep its directories
/// root's**, so the application whose directory it is can read what it holds
/// and can write nothing root later writes into. Answers a line for the
/// response when anything could not be set, and nothing otherwise.
///
/// The root, `objects/`, `incoming/` and every `<digest>/` below them are uid 0,
/// the application's group, mode [`DIRECTORY_MODE`]: that group is the
/// application's read grant. Each file in a digest directory is the
/// application's. A directory the application owned could hold a link it
/// planted, and root writes there; one it can only read cannot. A store an
/// earlier build handed over whole is put back to this shape by the same walk,
/// because it sets every entry whether or not it was already right — a pass
/// that failed partway leaves nothing else to repair it.
///
/// Walks exactly the shape `rivet_core::artifact::Store` lays out and no
/// deeper, every step through a held descriptor (`crate::beneath`): a link is
/// never descended into or followed, and an entry that is not the store's shape
/// is reported and left as it is.
fn hand_over(machine: &Machine) -> String {
    let Some((uid, gid)) = machine.owner else {
        return String::new();
    };
    let mut failed = Vec::new();
    let root = match Dir::existing(machine.store.root()) {
        Ok(root) => root,
        Err(e) => return handed_over(uid, &[format!("{}: {}", machine.store.root(), why(&e))]),
    };
    keep_roots(&root, gid, &mut failed);
    for part in ["objects", "incoming"] {
        let held = match root.open(part) {
            Ok(held) => held,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                failed.push(format!("{}/{part}: {}", root.shown(), why(&e)));
                continue;
            }
        };
        keep_roots(&held, gid, &mut failed);
        for digest in held.names().unwrap_or_default() {
            let object = match held.open(&digest) {
                Ok(object) => object,
                // Gone under a concurrent prune: nothing to hand over.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    failed.push(format!("{}/{digest}: {}", held.shown(), why(&e)));
                    continue;
                }
            };
            keep_roots(&object, gid, &mut failed);
            for file in object.names().unwrap_or_default() {
                match object.kind(&file) {
                    Ok(beneath::Kind::File) => {
                        if let Err(e) = object.set_entry_owner(&file, uid, gid) {
                            failed.push(format!("{}/{file}: {e}", object.shown()));
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(beneath::Kind::Link) => failed.push(format!(
                        "{}/{file}: a symbolic link, so it was left in place and not handed over",
                        object.shown()
                    )),
                    Ok(_) => failed.push(format!(
                        "{}/{file}: not a file this store keeps, so it was left as it is",
                        object.shown()
                    )),
                    Err(e) => failed.push(format!("{}/{file}: {e}", object.shown())),
                }
            }
        }
    }
    handed_over(uid, &failed)
}

/// One directory of the store back to root, with the application's group as
/// its read grant.
fn keep_roots(dir: &Dir, gid: u32, failed: &mut Vec<String>) {
    if let Err(e) = dir.set_owner(ROOT_UID, gid) {
        failed.push(format!("{}: {e}", dir.shown()));
    }
    if let Err(e) = dir.set_mode(DIRECTORY_MODE) {
        failed.push(format!("{}: {e}", dir.shown()));
    }
}

/// An error opening a store directory, said plainly when it was a link.
fn why(e: &std::io::Error) -> String {
    if beneath::is_not_real(e) {
        "a symbolic link or not a directory, so it was left in place and not handed over".to_owned()
    } else {
        e.to_string()
    }
}

/// [`hand_over`]'s line for the response.
fn handed_over(uid: u32, failed: &[String]) -> String {
    if failed.is_empty() {
        String::new()
    } else {
        format!(
            "\x20 owner   NOT every entry could be handed to {}'s uid {uid}, so that application \
             may not be able to read them: {}\n",
            objects::APPLICATION,
            failed.join("; ")
        )
    }
}

/// What the store holds and what the manifest names.
#[must_use]
pub fn listing(machine: &Machine) -> Listing {
    Listing { manifest: manifest(machine), held: held(&machine.store) }
}

/// Every object in `store`, by digest and the name it arrived under.
fn held(store: &artifact::Store) -> Vec<Held> {
    let Ok(entries) = std::fs::read_dir(store.objects_dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        // Not a digest: nothing this store put there, and nothing it lists or
        // removes — `crate::bulk`'s retention leaves such a directory alone too.
        let Some(digest) = artifact::Digest::new(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let Some(Ok(file)) = std::fs::read_dir(entry.path()).ok().and_then(|mut e| e.next()) else {
            continue;
        };
        out.push(Held { digest, name: file.file_name().to_string_lossy().into_owned() });
    }
    out.sort();
    out
}

/// **The `fleet` repository's `main:manifest.toml`, read with git's plumbing.**
fn manifest(machine: &Machine) -> Manifest {
    if !Path::new(&machine.repository).is_dir() {
        return Manifest::Unavailable(format!(
            "{} does not exist: this machine's document declares no [repos.{}], or rivet-sysd \
             has not converged it yet",
            machine.repository,
            objects::REPOSITORY
        ));
    }
    let commit = match git(machine, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}^{{commit}}", objects::BRANCH)]) {
        Ok(text) if !text.trim().is_empty() => text.trim().to_owned(),
        _ => {
            return Manifest::Unavailable(format!(
                "the {} repository has no {} yet; `cargo rivet fleet push <host> confirm` puts \
                 one there",
                objects::REPOSITORY,
                objects::BRANCH
            ));
        }
    };
    let text = match git(machine, &["show", &format!("{commit}:{}", objects::MANIFEST_FILE)]) {
        Ok(text) => text,
        Err(why) => {
            return Manifest::Unavailable(format!(
                "{} at {commit} carries no readable {}: {why}",
                objects::BRANCH,
                objects::MANIFEST_FILE
            ));
        }
    };
    match named(&text) {
        Ok(named) => Manifest::Read { commit, named },
        Err(why) => Manifest::Unavailable(format!("{}: {why}", objects::MANIFEST_FILE)),
    }
}

/// The digests a manifest's text names, through `rivet_core::repository_objects::named`.
///
/// **`toml_edit`, the parser `rivet-config` already links into this binary**, and
/// only the fields the store's rule reads: each `[[artifact]]`'s `digest`,
/// `product` and `kind`, and each `[[hidden]]`'s `product` and `kind`
/// (`ADR-052` §7). The manifest is written by `cmd/cargo-rivet`'s
/// `fleet::render`; a manifest that is not TOML, or an entry without one of
/// those as a string, is refused whole.
fn named(text: &str) -> Result<std::collections::BTreeSet<artifact::Digest>, String> {
    let doc: toml_edit::DocumentMut =
        text.parse().map_err(|e| format!("not valid TOML: {e}"))?;
    let mut entries = Vec::new();
    if let Some(item) = doc.get("artifact") {
        let array = item
            .as_array_of_tables()
            .ok_or("`artifact` is not an array of tables ([[artifact]])")?;
        for (at, table) in array.iter().enumerate() {
            let field = |key: &str| {
                table
                    .get(key)
                    .and_then(toml_edit::Item::as_str)
                    .ok_or_else(|| format!("[[artifact]] {} has no string {key}", at + 1))
            };
            entries.push((field("digest")?, field("product")?, field("kind")?));
        }
    }
    let mut hidden = Vec::new();
    if let Some(item) = doc.get("hidden") {
        let array = item
            .as_array_of_tables()
            .ok_or("`hidden` is not an array of tables ([[hidden]])")?;
        for (at, table) in array.iter().enumerate() {
            let field = |key: &str| {
                table
                    .get(key)
                    .and_then(toml_edit::Item::as_str)
                    .ok_or_else(|| format!("[[hidden]] {} has no string {key}", at + 1))
            };
            hidden.push((field("product")?, field("kind")?));
        }
    }
    objects::named(entries, &hidden)
}

/// `git` against the repository, read-only, with nothing a home directory or
/// a daemon's environment could add.
fn git(machine: &Machine, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(&machine.git);
    if let Some(path) = &machine.git_path {
        // **Replaced, not extended**, for `crate::git::serve`'s reason: an
        // inherited `GIT_DIR` would redirect this read to another repository.
        command.env_clear().env("PATH", path).env("HOME", "/nonexistent");
    }
    let output = command
        .arg("-c")
        .arg(format!("safe.directory={}", machine.repository))
        .arg("--git-dir")
        .arg(&machine.repository)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", machine.git))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// `POST /repository-objects/prune`.
fn prune(machine: &Machine) -> Response {
    let listing = listing(machine);
    let remove = match listing.prune() {
        Ok(remove) => remove,
        Err(why) => return refuse(409, "Conflict", format!("REFUSED: {why}\n")),
    };
    let Manifest::Read { commit, .. } = &listing.manifest else {
        return refuse(500, "Internal Server Error", "a prune was planned with no manifest\n".to_owned());
    };
    let mut body = format!(
        "prune against {}:{} at {commit}: {} object(s) held, {} not named\n",
        objects::BRANCH,
        objects::MANIFEST_FILE,
        listing.held.len(),
        remove.len()
    );
    // Removed through the held store (`crate::beneath`), so a store root an
    // application swapped for a link sends no removal anywhere else.
    let held = Dir::existing(machine.store.root()).and_then(|root| root.open("objects"));
    let mut failed = 0usize;
    for one in &remove {
        let removed = match &held {
            Ok(objects) => objects.remove(one.digest.as_str()),
            Err(e) => Err(std::io::Error::new(e.kind(), why(e))),
        };
        match removed {
            Ok(()) => body.push_str(&format!("removed {} {}\n", one.digest, one.name)),
            Err(e) => {
                failed += 1;
                body.push_str(&format!("not-removed {} {}: {e}\n", one.digest, one.name));
            }
        }
    }
    if failed > 0 {
        return refuse(500, "Internal Server Error", body);
    }
    Response { status: 200, reason: "OK", body }
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
