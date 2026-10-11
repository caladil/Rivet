//! **Reading and writing the Armada Registry on disk.** `crate::store`'s shape,
//! one level up: where `crate::store` holds one Fleet's members, this holds the
//! operator's Fleets. `ADR-059` §5.
//!
//! The layout under the Armada Registry root:
//!
//! ```text
//! armada.toml          the Armada-wide Settings
//! fleets/<name>.toml   one record per Fleet, filed under its name
//! ```
//!
//! Every file is rewritten from state (`crate::store`'s rule), so the output is
//! a function of the [`Armada`] and not of the history of edits. The root is
//! taken as an argument, exactly as `crate::store::load` takes one — resolving
//! `~/.rivet/armada` from the environment is the caller's job (`cmd/cargo-rivet`
//! resolves the register root the same way), so a test hands a scratch dir.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{Armada, Fleet, FLEETS_DIR, RECORD_SUFFIX, SETTINGS_FILE};
use crate::RegisterError;

fn io(path: &Path, e: &std::io::Error) -> RegisterError {
    RegisterError::Io { path: path.to_path_buf(), detail: e.to_string() }
}

fn malformed(path: &Path, detail: impl core::fmt::Display) -> RegisterError {
    RegisterError::Malformed { path: path.to_path_buf(), detail: detail.to_string() }
}

/// The Armada settings file, at the registry root.
#[must_use]
pub fn settings_path(root: &Path) -> PathBuf {
    root.join(SETTINGS_FILE)
}

/// The directory of per-Fleet records.
#[must_use]
pub fn fleets_dir(root: &Path) -> PathBuf {
    root.join(FLEETS_DIR)
}

/// One Fleet's record file, filed under its name.
///
/// **Does not check `name`.** This is a pure "what path would this be"
/// helper; [`forget`] and [`render`] (and so [`save`], which renders before
/// it writes) are the entry points that turn a caller-supplied name into a
/// side effect, and they refuse an unsafe one before ever calling this.
#[must_use]
pub fn fleet_path(root: &Path, name: &str) -> PathBuf {
    fleets_dir(root).join(format!("{name}{RECORD_SUFFIX}"))
}

/// Whether `name` is exactly one plain path component — a single
/// [`std::path::Component::Normal`] that reads back as the whole string.
///
/// [`fleet_path`] joins `name` into `fleets/` unchecked, so every entry point
/// that turns a caller-supplied name into a path checks this first: a `..`
/// walks back out of `fleets/`, a nested path reaches a different directory
/// entirely, and an absolute name replaces `fleets/` outright
/// (`PathBuf::join`'s documented behaviour for an absolute argument).
fn is_record_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(part)), None) if part == std::ffi::OsStr::new(name)
    )
}

/// **Every file the Armada would write, as `(path, text)` pairs** — the
/// settings file first, then one per Fleet in name order (`BTreeMap` iterates
/// sorted, so the set is the same on every run).
///
/// # Errors
///
/// [`RegisterError::InvalidName`] if any Fleet's name is not exactly one
/// plain path component — checked for every Fleet before any is rendered, so
/// [`save`] (which calls this first) never writes the valid Fleets in an
/// Armada that also holds one unsafe name. [`RegisterError::Malformed`] when
/// a record will not serialise.
pub fn render(armada: &Armada) -> Result<Vec<(PathBuf, String)>, RegisterError> {
    for name in armada.fleets.keys() {
        if !is_record_name(name) {
            return Err(RegisterError::InvalidName { name: name.clone() });
        }
    }
    let mut out = Vec::with_capacity(armada.fleets.len() + 1);
    let settings = settings_path(&armada.root);
    out.push((settings.clone(), render_one(&armada.settings, &settings)?));
    for (name, fleet) in &armada.fleets {
        let path = fleet_path(&armada.root, name);
        out.push((path.clone(), render_one(fleet, &path)?));
    }
    Ok(out)
}

fn render_one<T: Serialize>(value: &T, path: &Path) -> Result<String, RegisterError> {
    toml_edit::ser::to_string_pretty(value).map_err(|e| malformed(path, e))
}

/// **Read the whole Armada Registry at `root`.**
///
/// An absent `armada.toml` is default settings, and an absent `fleets/` is no
/// Fleets — absence is what a registry nothing has written to looks like,
/// not an error.
///
/// # Errors
///
/// [`RegisterError::Io`] for a path that will not read, and
/// [`RegisterError::Malformed`] for a record that is not valid TOML or whose
/// `name` disagrees with its file name (a file renamed by hand, which the
/// references elsewhere can no longer be trusted against).
pub fn load(root: &Path) -> Result<Armada, RegisterError> {
    let mut armada = Armada::empty(root.to_path_buf());

    let settings = settings_path(root);
    match fs::read_to_string(&settings) {
        Ok(text) => {
            armada.settings =
                toml_edit::de::from_str(&text).map_err(|e| malformed(&settings, e))?;
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(io(&settings, &e)),
    }

    let dir = fleets_dir(root);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(armada),
        Err(e) => return Err(io(&dir, &e)),
    };

    // Collected and sorted rather than taken in `read_dir` order, which makes
    // no promise: the registry's value depends on two runs producing the same
    // thing (`crate::store::load`'s own reasoning).
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io(&dir, &e))?;
        let path = entry.path();
        let is_record = path.extension().is_some_and(|ext| {
            RECORD_SUFFIX.strip_prefix('.').is_some_and(|want| ext.eq_ignore_ascii_case(want))
        });
        if is_record {
            paths.push(path);
        }
    }
    paths.sort();

    for path in paths {
        let text = fs::read_to_string(&path).map_err(|e| io(&path, &e))?;
        let fleet: Fleet = toml_edit::de::from_str(&text).map_err(|e| malformed(&path, e))?;
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        if fleet.name != stem {
            return Err(malformed(
                &path,
                format!(
                    "this Fleet record's name is {:?} but it is filed as {stem:?}; a Fleet is \
                     filed under its own name, and the two disagreeing means the file was \
                     renamed by hand",
                    fleet.name,
                ),
            ));
        }
        armada.fleets.insert(fleet.name.clone(), fleet);
    }
    Ok(armada)
}

/// **Write the whole Armada Registry to its root**, creating directories as
/// needed. Every file is rewritten (see the module header).
///
/// # Errors
///
/// [`RegisterError::Io`] for anything the filesystem refuses, and
/// [`RegisterError::Malformed`] if a record will not render.
pub fn save(armada: &Armada) -> Result<Vec<PathBuf>, RegisterError> {
    let files = render(armada)?;
    let mut written = Vec::with_capacity(files.len());
    for (path, text) in files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, &e))?;
        }
        fs::write(&path, text.as_bytes()).map_err(|e| io(&path, &e))?;
        written.push(path);
    }
    Ok(written)
}

/// **Remove one Fleet's record from the registry.** The file is deleted and
/// nothing else — forgetting a Fleet on this PC says nothing about the fleet
/// server or its members. A record already gone is not an error: the requested
/// state is the state (`crate::store::forget`'s rule).
///
/// # Errors
///
/// [`RegisterError::InvalidName`] if `name` is not exactly one plain path
/// component, checked before anything is removed. [`RegisterError::Io`] for
/// a file that exists and will not be removed.
pub fn forget(root: &Path, name: &str) -> Result<(), RegisterError> {
    if !is_record_name(name) {
        return Err(RegisterError::InvalidName { name: name.to_owned() });
    }
    let path = fleet_path(root, name);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(&path, &e)),
    }
}
