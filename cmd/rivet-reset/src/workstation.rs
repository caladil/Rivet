//! **The half of a reset that is files** — the authority, the register, and the
//! provisioning documents composed on this machine.
//!
//! # What is removed is named, and what is refused is named louder
//!
//! `SPEC-05` §2 makes the signing seeds a **refusal** rather than an
//! omission, and the difference is the whole of [`Hazard`]: an omission is
//! something a future edit quietly drops, and a refusal is something that stops
//! the program. `~/.rivet/keys/rivet-system.seed` and `rivet-app.seed` are not
//! fleet identity — every board's trust store carries their public halves, and
//! destroying them invalidates every payload this project has ever signed.
//! There is no flag for it and there is no argument that reaches past it.
//!
//! The same guard covers two more things it was never asked about, because the
//! cost of being wrong is the same shape: **an SSH key, ever** (a standing rule
//! of this maintainer's, and one an agent has broken before), and
//! `~/.rivet/identity`, which holds the control-plane operator identity that
//! reaches every running board. Neither is fleet-authority material, and a
//! reset that took one out would leave the operator locked out of machines that
//! are working perfectly.

use std::path::{Path, PathBuf};

/// One file a reset would remove.
pub struct Found {
    /// Where it is.
    pub path: PathBuf,
    /// How big, so *"is this the one I made an hour ago"* has an answer.
    pub bytes: u64,
    /// What it is, in the operator's terms rather than the filesystem's.
    pub note: &'static str,
}

/// One directory's worth of them.
pub struct Group {
    /// The heading this appears under.
    pub title: &'static str,
    /// The directory itself, which is removed once it is empty.
    pub root: PathBuf,
    /// What was found, in a stable order.
    pub files: Vec<Found>,
    /// **Traversal errors [`walk`] hit** — an entry it could not even stat,
    /// most often. Non-empty means the enumeration above is incomplete, so
    /// the plan a caller shows and confirms cannot be trusted: the caller
    /// refuses on this exactly as it refuses on a [`Hazard`], before
    /// anything is removed.
    pub errors: Vec<String>,
}

impl Group {
    /// Whether there is anything here at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Where `~/.rivet` is on this machine.
///
/// The same two variables `rivet-provision` reads, in the same order, so that
/// the tool that MADE the authority and the tool that removes it cannot
/// disagree about where it lives.
#[must_use]
pub fn rivet_home() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".rivet"))
}

/// The default authority directory — `rivet-provision`'s `authority_directory`.
#[must_use]
pub fn default_ca() -> Option<PathBuf> {
    Some(rivet_home()?.join("ca"))
}

/// The default provisioning directory — `rivet-provision`'s `save::directory`.
#[must_use]
pub fn default_provision() -> Option<PathBuf> {
    Some(rivet_home()?.join("provision"))
}

/// The default Armada Registry — `cargo rivet`'s `armada::default_dir`,
/// `~/.rivet/armada`.
///
/// Restated rather than called, because `cmd/cargo-rivet` has no library
/// target. It resolves the same way: the `~/.rivet` both tools derive from
/// `HOME`, then `USERPROFILE`, joined to `armada`.
#[must_use]
pub fn default_armada() -> Option<PathBuf> {
    Some(rivet_home()?.join("armada"))
}

/// The default fleet repository — `rivetos_fleet_repo::default_root`, the crate
/// that writes the records `--keep-authority` clears, asked directly.
#[must_use]
pub fn default_fleet() -> Option<PathBuf> {
    rivetos_fleet_repo::default_root()
}

/// The directory holding the signing seeds, which is never removed.
#[must_use]
pub fn keys_directory() -> Option<PathBuf> {
    Some(rivet_home()?.join("keys"))
}

/// The directory holding the control-plane operator identity, also never
/// removed.
#[must_use]
pub fn identity_directory() -> Option<PathBuf> {
    Some(rivet_home()?.join("identity"))
}

/// **Where `rivet-fleet` keeps its register**, resolved the way `rivet-fleet`
/// resolves it.
///
/// # Why this is eleven lines and not a call
///
/// `cmd/rivet-fleet` has no library target — it is `[[bin]]` and nothing else —
/// so `Config::register_dir` cannot be imported, and `ADR-011` §3's one-spelling
/// rule is met the only way that is left: by going through **Ignition's own
/// loader**, with `rivet-fleet`'s application name and environment prefix, so
/// that a configuration file naming a register and a `RIVET_FLEET_REGISTER` in
/// the environment are both honoured exactly as that tool honours them. What is
/// restated here is the default's *shape* — the register directory beside the
/// configuration file — and the test below is what holds the two together.
///
/// **A reset that removed the wrong register would be silent**, because both
/// answers are directories full of TOML.
#[must_use]
pub fn default_register() -> Option<PathBuf> {
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct AsFleetSeesIt {
        #[serde(default)]
        register: Option<PathBuf>,
    }
    let loaded: AsFleetSeesIt = ignition::config::Loader::new(FLEET_APP_NAME)
        .env_prefix(FLEET_ENV_PREFIX)
        .load();
    if let Some(named) = loaded.register {
        return Some(named);
    }
    let beside = ignition::config::config_path(FLEET_APP_NAME)?;
    Some(beside.parent()?.join(FLEET_REGISTER_DIR))
}

/// `cmd/rivet-fleet/src/config.rs`'s `APP_NAME`.
const FLEET_APP_NAME: &str = "rivet-fleet";
/// Its `ENV_PREFIX`, which is what makes `RIVET_FLEET_REGISTER` work.
const FLEET_ENV_PREFIX: &str = "RIVET_FLEET";
/// Its `DEFAULT_REGISTER_DIR`.
const FLEET_REGISTER_DIR: &str = "register";

/// Whether the entry described by `meta` is a link this program must never
/// follow: a Unix symlink, or — on Windows — ANY reparse point.
///
/// **`FileType::is_symlink` alone is not a safe test for a directory
/// junction** (`IO_REPARSE_TAG_MOUNT_POINT`, what `cmd /C mklink /J`
/// creates, as opposed to `IO_REPARSE_TAG_SYMLINK`). Checking the raw
/// `FILE_ATTRIBUTE_REPARSE_POINT` bit directly, rather than trusting the
/// higher-level classification, is what catches a junction regardless of
/// which reparse tag it carries — B14.
///
/// Takes an already-read [`std::fs::symlink_metadata`] — never `metadata`,
/// which follows the very thing this function exists to ask about.
#[must_use]
fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// **Whether the OS classifies a link ITSELF — never what it points to —
/// as a directory**, used only to choose the removal call for something
/// [`is_link`] already said must not be followed.
///
/// A Windows junction reports `FILE_ATTRIBUTE_DIRECTORY` on itself and
/// removes with `RemoveDirectory` (`std::fs::remove_dir`); `DeleteFile`
/// (`std::fs::remove_file`) refuses it. A Unix symlink never reports this,
/// whatever it points to, so `remove_file` is always correct there.
#[cfg(windows)]
#[must_use]
fn link_is_directory_typed(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// Everything under `dir`, deepest last, with a sentence for each — and
/// every traversal error the walk could not route around.
///
/// **`rm -r` semantics: a link is never followed.** A symlink (Unix) or a
/// reparse point (Windows — [`is_link`] covers a junction too) found inside
/// `dir` is listed as an entry to be removed **itself**, the same as a
/// file; what it points to is never read, sized, or walked. Before this,
/// `Path::is_dir` decided whether to recurse, and `Path::is_dir` follows
/// the very thing being tested — so a directory link inside a removal root
/// made the walk, and then the removal, reach outside it (B14). A link
/// back toward its own ancestor is therefore not a loop to guard against
/// separately: nothing here ever opens a link to find out what is behind
/// it, so there is nothing that could recurse through one.
///
/// **Depth-first with directories after their contents**, because that is the
/// order they can be removed in — and building the list in removal order means
/// the report and the removal cannot disagree about what was covered.
///
/// A directory this program could not even list, or an entry it could not
/// stat, is a traversal error, returned rather than silently dropped: an
/// enumeration that skipped something is a plan the confirmation lied
/// about. `dir` itself not existing is not one of these — an unprovisioned
/// workstation has no register or provisioning directory at all, and that
/// is "nothing here", at any depth.
///
/// **`dir` itself being a link is a traversal error too**, and nothing under
/// it is listed. The same `rm -r` rule applied one level up: a removal root
/// that is a symlink or a junction names somewhere else, and walking through
/// it would plan the removal of whatever that is under a spelling no
/// containment check recognises. Refused rather than reduced to removing the
/// link itself, because a root given on the command line is something the
/// operator meant as a directory to empty, and an alias of one is a mistake to
/// report.
#[must_use]
pub fn walk(dir: &Path, describe: fn(&Path) -> &'static str) -> (Vec<Found>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    if std::fs::symlink_metadata(dir).is_ok_and(|meta| is_link(&meta)) {
        errors.push(format!(
            "{}: is a symbolic link or junction, and a directory to be emptied is never \
             followed through one. Name the directory it points to, if that is what you meant",
            dir.display()
        ));
        return (out, errors);
    }
    walk_into(dir, describe, &mut out, &mut errors);
    (out, errors)
}

fn walk_into(
    dir: &Path,
    describe: fn(&Path) -> &'static str,
    out: &mut Vec<Found>,
    errors: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return,
        Err(why) => {
            errors.push(format!("{}: {why}", dir.display()));
            return;
        }
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(why) => errors.push(format!("{}: {why}", dir.display())),
        }
    }
    paths.sort();
    for path in paths {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(why) => {
                errors.push(format!("{}: {why}", path.display()));
                continue;
            }
        };
        if is_link(&meta) {
            // Never dereferenced: not walked, and sized from the link's own
            // metadata rather than whatever it points to.
            let note = describe(&path);
            out.push(Found { path, bytes: meta.len(), note });
            continue;
        }
        if meta.is_dir() {
            walk_into(&path, describe, out, errors);
            continue;
        }
        let note = describe(&path);
        out.push(Found { path, bytes: meta.len(), note });
    }
}

/// What each file in an authority directory is.
#[must_use]
pub fn describe_authority(path: &Path) -> &'static str {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match name.as_str() {
        "root.crt" => "the authority a browser trusts",
        "root.key" => "THE FLEET'S TRUST — one copy, and this destroys it",
        "browser.crt" => "your operator certificate",
        "browser.key" => "your operator private key",
        "browser.pfx" => "the bundle Windows imported your identity from",
        "suffix" => "the note of what this authority may vouch for",
        _ if name.starts_with("server-") && name.ends_with(".key") => {
            "a fleet server's private key"
        }
        _ if name.starts_with("server-") => "a fleet server's certificate",
        _ => "in the authority directory",
    }
}

/// What each file in the register is.
#[must_use]
pub fn describe_register(path: &Path) -> &'static str {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    if name == "register.toml" {
        return "the fleet itself";
    }
    if name == "credentials.store" {
        return "rivet-fleet's shared network credential store";
    }
    if path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d == "devices")
    {
        return "a device record";
    }
    "in the register"
}

/// What each file in `~/.rivet/provision` is.
///
/// **Two kinds share that directory and they are not the same artefact.**
/// `rivet-provision`'s `s` key writes an answers notebook and its `w` key writes
/// a composed document; both are named after the same hostname, and the
/// `.provisioning.toml` suffix is what stops them destroying each other. Both go
/// — `SPEC-05` §2 wants the state a first-ever run would find, and a notebook
/// half-describing the fleet being replaced is exactly the August leftover §2
/// opens by describing — but the report says which is which, because one of them
/// may be a sitting somebody meant to come back to.
#[must_use]
pub fn describe_document(path: &Path) -> &'static str {
    let name = path.to_string_lossy();
    if name.ends_with(".provisioning.toml") {
        "a composed provisioning document"
    } else if name.ends_with(".staged.txt") {
        "a machine's staging bundle, composed beside its document (ADR-049)"
    } else {
        "an answers notebook (work in progress)"
    }
}

/// What each file in the Armada's `fleets/` directory is.
#[must_use]
pub fn describe_armada(_path: &Path) -> &'static str {
    "a Fleet this workstation knows: its server's host and pin"
}

/// What each machine record in the fleet repository is.
#[must_use]
pub fn describe_fleet_record(path: &Path) -> &'static str {
    if path.to_string_lossy().ends_with(rivetos_fleet_repo::DOCUMENT_SUFFIX) {
        return "a machine's recorded provisioning document";
    }
    if path.file_name().is_some_and(|n| n == "register.toml") {
        return "the register's projection: the fleet itself";
    }
    if path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d == "devices")
    {
        return "the register's projection: a device record";
    }
    "in the fleet repository's records"
}

/// **Something that must not be removed, found where something would be.**
///
/// Carries the path and the reason, because a refusal that does not say why is
/// one somebody works around.
pub struct Hazard {
    /// What was about to be touched.
    pub path: PathBuf,
    /// Why it must not be.
    pub why: &'static str,
}

/// Check every path this run would remove, and refuse the whole run if any of
/// them is something that must survive it.
///
/// **Whole-run, not per-file.** A reset that skipped the dangerous file and
/// removed the rest would leave a workstation in a state nobody described, and
/// the operator would find out from what still worked rather than from a
/// message.
#[must_use]
pub fn hazards(paths: &[PathBuf]) -> Vec<Hazard> {
    let keys = keys_directory();
    let identity = identity_directory();
    let mut out = Vec::new();
    for path in paths {
        let mut why = None;
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("seed")) {
            why = Some(
                "a SIGNING SEED. Every board's trust store carries its public half, so removing \
                 it invalidates every payload this project has ever signed. SPEC-05 §2",
            );
        } else if path
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case(".ssh"))
        {
            why = Some("inside .ssh. No tool of this maintainer's touches an SSH key, ever");
        } else if keys.as_ref().is_some_and(|k| path.starts_with(k)) {
            why = Some("inside ~/.rivet/keys, which holds the signing seeds. SPEC-05 §2");
        } else if identity.as_ref().is_some_and(|i| path.starts_with(i)) {
            why = Some(
                "inside ~/.rivet/identity — the control-plane operator identity, which reaches \
                 every running board. It is not fleet-authority material and a new fleet does \
                 not replace it",
            );
        }
        if let Some(why) = why {
            out.push(Hazard {
                path: path.clone(),
                why,
            });
        }
    }
    out
}

/// **Every path this run would remove that lies inside something it was told
/// to keep**, each with the reason it is kept.
///
/// `--keep-authority`'s second guard. The first is that its plan never lists
/// the authority, the manifest or the objects at all; this one refuses the
/// whole run if a directory given on the command line nevertheless puts one
/// of them in the plan — `--provision ~/.rivet/ca`, say. Whole-run for
/// [`hazards`]'s reason.
///
/// **Compared through [`same_or_inside`]**, so a path that reaches a kept root
/// through a linked parent, a junction, or another spelling of the same place
/// is caught as surely as one spelled the same way.
#[must_use]
pub fn inside_kept(paths: &[PathBuf], kept: &[(PathBuf, &'static str)]) -> Vec<Hazard> {
    paths
        .iter()
        .filter_map(|path| {
            kept.iter()
                .find(|(root, _)| same_or_inside(path, root))
                .map(|(_, why)| Hazard {
                    path: path.clone(),
                    why,
                })
        })
        .collect()
}

/// **A directory too broad to be a reset target**, or `None` if it is fine.
///
/// `--ca ~` would compose a plan naming every file in a home directory, and the
/// confirmation would be a page nobody reads to the end of. The guard is depth:
/// a target has to be somewhere specific.
#[must_use]
pub fn too_broad(dir: &Path) -> Option<String> {
    let depth = dir.components().count();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if home.is_some_and(|h| h == dir) {
        return Some(format!("{} is the home directory itself", dir.display()));
    }
    if rivet_home().is_some_and(|r| r == dir) {
        return Some(format!(
            "{} is ~/.rivet itself, which holds the signing seeds and the operator identity as \
             well as the authority",
            dir.display()
        ));
    }
    // A drive root is two components on Windows (`C:` and `\`) and one on
    // anything else, so three is the first depth that names something.
    if depth < 3 {
        return Some(format!("{} is a filesystem root", dir.display()));
    }
    None
}

/// The seeds this refuses to remove, so the report can name them.
///
/// **Reported whether or not they are there.** A person reading the report is
/// being told what a reset does *not* do, and that sentence is worth as much on
/// a machine where the seeds are missing as on one where they are present.
#[must_use]
pub fn seeds() -> Vec<PathBuf> {
    let Some(dir) = keys_directory() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "seed"))
        .collect();
    out.sort();
    out
}

/// **Whether a backup of the authority about to be removed can be found at
/// `backup_dir`.**
///
/// `SPEC-05` §2: nothing before this checked, and `remove` below deletes
/// `root.key` — the fleet's only copy of its whole trust — file by file with
/// no way back. Comparing `root.crt` bytes against candidates in a named
/// directory is the honest version of "is there a backup": it does not ask
/// whether a directory merely *exists*, and it does not try to parse or
/// verify a key it has no business opening — it asks whether the exact file
/// this authority's `root.crt` already is shows up somewhere the operator
/// named.
///
/// **Two places are checked**: `backup_dir` itself, and one level of its
/// immediate subdirectories — `rivet-provision ca backup <dir>` writes
/// straight into `<dir>`, and a directory holding several fleets' backups
/// side by side (one subdirectory per fleet) is the other shape an operator
/// reasonably keeps. Comparison is byte-for-byte: `ca backup` copies the
/// file verbatim, so an exact match is what a real backup produces and
/// nothing else does.
///
/// Returns the matching file's path, for the report to name.
#[must_use]
pub fn backup_covers(backup_dir: &Path, current: &[u8]) -> Option<PathBuf> {
    let mut candidates = vec![backup_dir.join("root.crt")];
    if let Ok(entries) = std::fs::read_dir(backup_dir) {
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        subdirs.sort();
        candidates.extend(subdirs.into_iter().map(|d| d.join("root.crt")));
    }
    candidates
        .into_iter()
        .find(|c| std::fs::read(c).is_ok_and(|bytes| bytes == current))
}

/// Whether `candidate` is the same location as, or lies inside, `root`.
///
/// Checked by canonicalized path first, so a symlink, a Windows junction, or
/// a merely different spelling of the same place (`.`, `..`, or separator
/// differences) cannot hide the relationship from a lexical compare alone.
/// Falls back to the lexical compare when either side fails to
/// canonicalize: canonicalizing requires resolving the path against the
/// real filesystem, and that requirement must not be read as proof the two
/// paths are unrelated — the lexical relationship between them holds
/// whichever one currently has something backing it on disk.
/// [`too_broad`] makes the identical choice for the identical reason.
#[must_use]
fn same_or_inside(candidate: &Path, root: &Path) -> bool {
    if candidate == root || candidate.starts_with(root) {
        return true;
    }
    matches!(
        (std::fs::canonicalize(candidate), std::fs::canonicalize(root)),
        (Ok(c), Ok(r)) if c == r || c.starts_with(&r)
    )
}

/// **The safety half of `--backup` — `SPEC-05` §2, B13.** Whether the file
/// [`backup_covers`] matched is itself something this run is about to
/// remove.
///
/// A copy the reset destroys is not a backup: it reads as "found" right up
/// until it is gone, which is worse than reporting no backup at all,
/// because the operator was told a copy existed. Checked against every
/// removal root, because `--backup` naming the authority directory itself,
/// a location nested inside any removal root, or an alias of one — lexical
/// or through a symlink/junction — all end in the same file being deleted
/// alongside what it was meant to protect.
///
/// Returns the removal root the match falls inside, for the refusal to
/// name.
#[must_use]
pub fn backup_is_a_removal_target<'a>(
    found: &Path,
    removal_roots: &'a [PathBuf],
) -> Option<&'a PathBuf> {
    removal_roots.iter().find(|root| same_or_inside(found, root))
}

/// Remove one entry the way its own type requires on THIS platform:
/// `remove_dir` for a real directory or a directory-typed link (a Windows
/// junction, which `remove_file`/`DeleteFile` refuses), `remove_file` for
/// everything else — including every Unix symlink, regardless of what it
/// points to (`link_is_directory_typed` is Windows-only for exactly that
/// reason).
///
/// Re-checked here via a fresh [`std::fs::symlink_metadata`] rather than
/// trusting a flag recorded when [`walk`] found the entry: like the walk
/// itself, this never follows the entry to decide, and re-stating the
/// decision from the entry's current metadata is no less correct than
/// caching it would have been.
fn remove_entry(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let as_directory = if is_link(&meta) {
        #[cfg(windows)]
        {
            link_is_directory_typed(&meta)
        }
        #[cfg(not(windows))]
        {
            false
        }
    } else {
        meta.is_dir()
    };
    if as_directory {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Remove a group's files, then the directories they were in.
///
/// Returns what could not be removed, each with its own sentence. **A failure
/// does not stop the rest**: a locked file is a thing to report, and stopping
/// would leave a half-reset workstation whose state depends on directory order.
#[must_use]
pub fn remove(group: &Group) -> Vec<String> {
    let mut failed = Vec::new();
    for file in &group.files {
        if let Err(why) = remove_entry(&file.path) {
            failed.push(format!("{}: {why}", file.path.display()));
        }
    }
    // Deepest first, so a directory is empty by the time it is reached.
    let mut dirs: Vec<PathBuf> = group
        .files
        .iter()
        .filter_map(|f| f.path.parent().map(Path::to_path_buf))
        .collect();
    dirs.push(group.root.clone());
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for dir in dirs {
        // **`remove_dir`, never `remove_dir_all`.** Anything left in there is
        // something this program did not list, and removing a file the operator
        // was never shown is the one thing a confirmation cannot cover.
        if dir.exists() && std::fs::remove_dir(&dir).is_err() {
            failed.push(format!(
                "{}: not removed — something is in it that was not on this list",
                dir.display()
            ));
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The refusal `SPEC-05` §2 requires**, and it is a test rather than a
    /// comment because the failure it prevents is unrecoverable.
    #[test]
    fn a_signing_seed_is_refused_wherever_it_is_found() {
        let found = hazards(&[
            PathBuf::from("/anywhere/rivet-system.seed"),
            PathBuf::from("/anywhere/rivet-app.seed"),
        ]);
        assert_eq!(found.len(), 2);
        for one in &found {
            assert!(one.why.contains("SIGNING SEED"), "{}", one.why);
        }
    }

    /// An SSH key is refused by the same guard, and by name.
    #[test]
    fn an_ssh_key_is_refused() {
        let found = hazards(&[PathBuf::from("/home/j/.ssh/id_ed25519")]);
        assert_eq!(found.len(), 1);
        assert!(found[0].why.contains("SSH"), "{}", found[0].why);
    }

    /// **The authority's own files are NOT hazards.** A guard that refused
    /// everything would be a program that does nothing, and this is what says
    /// the guard is narrow enough to leave the job possible.
    #[test]
    fn the_things_a_reset_is_for_are_not_refused() {
        let clear = hazards(&[
            PathBuf::from("/home/j/.rivet/ca/root.key"),
            PathBuf::from("/home/j/.rivet/ca/browser.pfx"),
            PathBuf::from("/home/j/.rivet/provision/arbor.provisioning.toml"),
        ]);
        assert!(clear.is_empty(), "{}", clear[0].path.display());
    }

    /// **A path inside something kept is refused, and a sibling of it is
    /// not.** `objects-old/` beside `objects/` shares a prefix as a string and
    /// not as a path, and must not be mistaken for it.
    #[test]
    fn a_path_inside_something_kept_is_refused() {
        let kept = [
            (PathBuf::from("/h/.rivet/ca"), "the authority"),
            (PathBuf::from("/h/.rivet/fleet/objects"), "the objects"),
        ];
        let found = inside_kept(
            &[
                PathBuf::from("/h/.rivet/ca/root.key"),
                PathBuf::from("/h/.rivet/fleet/objects/abc"),
                PathBuf::from("/h/.rivet/fleet/objects-old/abc"),
                PathBuf::from("/h/.rivet/fleet/documents/a.provisioning.toml"),
            ],
            &kept,
        );
        let refused: Vec<&str> = found.iter().map(|h| h.why).collect();
        assert_eq!(refused, ["the authority", "the objects"]);
    }

    /// A target has to name somewhere. `--ca /` would otherwise compose a plan
    /// over a whole filesystem.
    #[test]
    fn a_target_that_is_a_root_is_refused() {
        assert!(too_broad(Path::new("/")).is_some());
        assert!(too_broad(Path::new("C:\\")).is_some());
        assert!(too_broad(Path::new("/home/j/.rivet/ca")).is_none());
    }

    /// Files are described in the operator's terms, and the two things sharing
    /// `~/.rivet/provision` are told apart by the suffix that exists to tell
    /// them apart.
    #[test]
    fn each_file_says_what_it_is() {
        assert!(describe_authority(Path::new("/x/root.key")).contains("FLEET'S TRUST"));
        assert_eq!(
            describe_authority(Path::new("/x/server-arbor.local.crt")),
            "a fleet server's certificate"
        );
        assert_eq!(
            describe_document(Path::new("/x/arbor.provisioning.toml")),
            "a composed provisioning document"
        );
        assert_eq!(
            describe_document(Path::new("/x/rivet-0c3a8.toml")),
            "an answers notebook (work in progress)"
        );
        assert!(describe_document(Path::new("/x/arbor.staged.txt")).contains("staging bundle"));
        assert_eq!(
            describe_register(&PathBuf::from("/r").join("credentials.store")),
            "rivet-fleet's shared network credential store"
        );
        assert_eq!(
            describe_register(&PathBuf::from("/r").join("devices").join("abc.toml")),
            "a device record"
        );
    }

    /// **A backup is found by its bytes, not by its presence.** A directory
    /// that exists but holds a DIFFERENT authority's `root.crt` must not read
    /// as "a backup was found" — that would let a reset destroy the one
    /// authority it was about to remove while believing a copy of it exists
    /// somewhere.
    #[test]
    fn a_backup_is_matched_on_the_exact_bytes_of_root_crt() {
        let dir = std::env::temp_dir()
            .join(format!("rivet-reset-backup-match-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let backup = dir.join("backup");
        std::fs::create_dir_all(&backup).expect("a backup directory");
        std::fs::write(backup.join("root.crt"), b"the current authority's root").expect("write");

        assert_eq!(
            backup_covers(&backup, b"the current authority's root"),
            Some(backup.join("root.crt")),
            "an exact byte match must be found"
        );
        assert_eq!(
            backup_covers(&backup, b"a DIFFERENT authority's root"),
            None,
            "a different authority's backup must not read as covering this one"
        );
        assert_eq!(
            backup_covers(&dir.join("does-not-exist"), b"anything"),
            None,
            "a backup directory that is not there is not a backup that was found"
        );
    }

    /// **One level of subdirectories is checked too** — an operator keeping
    /// several fleets' backups under one parent directory, one subdirectory
    /// per fleet, is the other shape `ca backup` produces.
    #[test]
    fn a_backup_one_level_down_is_also_found() {
        let dir = std::env::temp_dir()
            .join(format!("rivet-reset-backup-nested-{}", std::process::id()));
        let nested = dir.join("home-fleet");
        std::fs::create_dir_all(&nested).expect("a nested backup directory");
        std::fs::write(nested.join("root.crt"), b"this fleet's root").expect("write");

        assert_eq!(
            backup_covers(&dir, b"this fleet's root"),
            Some(nested.join("root.crt"))
        );
    }

    /// **The walk finds everything and reports its size**, including inside the
    /// register's `devices/` directory — a device record written in August
    /// surviving into a fleet minted in September is the failure `SPEC-05` §2
    /// opens with.
    #[test]
    fn a_nested_record_is_found() {
        let dir = std::env::temp_dir().join(format!("rivet-reset-walk-{}", std::process::id()));
        let devices = dir.join("devices");
        std::fs::create_dir_all(&devices).expect("a scratch directory");
        std::fs::write(dir.join("register.toml"), "fleet\n").expect("a fleet file");
        std::fs::write(devices.join("abc.toml"), "a record\n").expect("a record");

        let (found, errors) = walk(&dir, describe_register);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(found.len(), 2, "the nested record was not found");
        assert!(found.iter().any(|f| f.note == "a device record"));
        assert!(found.iter().all(|f| f.bytes > 0));

        let group = Group {
            title: "test",
            root: dir.clone(),
            files: found,
            errors: Vec::new(),
        };
        assert!(remove(&group).is_empty(), "the scratch group did not remove");
        assert!(!dir.exists(), "the directory itself was left behind");
    }

    /// **A missing directory is not a traversal error.** An unprovisioned
    /// workstation has no register or provisioning directory at all, and
    /// that reads as "nothing here", not something to refuse on.
    #[test]
    fn a_directory_that_does_not_exist_is_not_a_traversal_error() {
        let dir = std::env::temp_dir()
            .join(format!("rivet-reset-walk-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (found, errors) = walk(&dir, describe_register);
        assert!(found.is_empty());
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// **B14: a directory link inside a removal root is never followed.**
    /// `root/link` points at an entirely unrelated scratch tree; the walk
    /// must list the link itself and must not read, size, or return
    /// anything from what it points to — proven by asserting the unrelated
    /// file is simply absent from the result, not merely unmarked for
    /// removal.
    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_a_removal_root_is_not_followed() {
        let base =
            std::env::temp_dir().join(format!("rivet-reset-walk-symlink-{}", std::process::id()));
        let root = base.join("root");
        let unrelated = base.join("unrelated");
        std::fs::create_dir_all(&root).expect("a removal root");
        std::fs::create_dir_all(&unrelated).expect("an unrelated tree");
        std::fs::write(unrelated.join("secret.txt"), b"must survive").expect("a file");
        std::os::unix::fs::symlink(&unrelated, root.join("link")).expect("a symlink");

        let (found, errors) = walk(&root, describe_document);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            found_paths(&found),
            vec![root.join("link").display().to_string()],
            "only the link itself, never its target"
        );
        assert!(
            found.iter().all(|f| f.path != unrelated.join("secret.txt")),
            "the walk must never return a path from inside the link's target"
        );

        // Removing the group must remove the link entry without touching
        // what it points to.
        let group = Group {
            title: "test",
            root: root.clone(),
            files: found,
            errors: Vec::new(),
        };
        assert!(remove(&group).is_empty());
        assert!(!root.exists(), "the removal root should be gone");
        assert!(unrelated.join("secret.txt").exists(), "the unrelated file must survive");
        assert_eq!(std::fs::read(unrelated.join("secret.txt")).expect("readable"), b"must survive");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// **B14: a symlink loop terminates**, because nothing here ever opens a
    /// link to see what is behind it. `root/self` points back at `root`
    /// itself; if the walk followed it even once it would recurse without
    /// bound.
    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_does_not_recurse() {
        let root =
            std::env::temp_dir().join(format!("rivet-reset-walk-loop-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a removal root");
        std::os::unix::fs::symlink(&root, root.join("self")).expect("a self-referential symlink");

        let (found, errors) = walk(&root, describe_document);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(found.len(), 1, "the loop link itself, once, never recursed into");
        assert_eq!(found[0].path, root.join("self"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Paths, for a readable assertion failure above.
    #[cfg(unix)]
    fn found_paths(found: &[Found]) -> Vec<String> {
        found.iter().map(|f| f.path.display().to_string()).collect()
    }

    /// **B13: a backup that is the removal root itself is unsafe.**
    #[test]
    fn a_backup_matching_the_removal_root_itself_is_unsafe() {
        let root = PathBuf::from("/h/.rivet/ca");
        let found = root.join("root.crt");
        assert_eq!(
            backup_is_a_removal_target(&found, std::slice::from_ref(&root)),
            Some(&root)
        );
    }

    /// **B13: a backup nested inside a DIFFERENT removal root is unsafe
    /// too** — not only the authority's own directory.
    #[test]
    fn a_backup_nested_inside_any_removal_root_is_unsafe() {
        let roots = vec![PathBuf::from("/h/.rivet/ca"), PathBuf::from("/h/.rivet/provision")];
        let found = PathBuf::from("/h/.rivet/provision/manual-backup/root.crt");
        assert_eq!(backup_is_a_removal_target(&found, &roots), Some(&roots[1]));
    }

    /// **B13: an independent backup, outside every removal root, is safe**
    /// — the control case a hard refusal must not overreach into.
    #[test]
    fn an_independent_backup_is_not_a_removal_target() {
        let roots = vec![PathBuf::from("/h/.rivet/ca"), PathBuf::from("/h/.rivet/provision")];
        let found = PathBuf::from("/d/backups/rivet-ca/root.crt");
        assert_eq!(backup_is_a_removal_target(&found, &roots), None);
    }

    /// **A sibling that merely shares a string prefix is not "inside".**
    /// `/h/.rivet/ca-old` is not inside `/h/.rivet/ca`, the same distinction
    /// `a_path_inside_something_kept_is_refused` makes for [`inside_kept`].
    #[test]
    fn a_lexical_sibling_is_not_treated_as_inside() {
        let roots = vec![PathBuf::from("/h/.rivet/ca")];
        let found = PathBuf::from("/h/.rivet/ca-old/root.crt");
        assert_eq!(backup_is_a_removal_target(&found, &roots), None);
    }
}
