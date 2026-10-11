//! **Copying a backup out, and back in — one engine, three callers.**
//!
//! Hoisted 2026-09-14 out of `cmd/rivet-provision/src/backup.rs`, which
//! built it for `rivet-provision ca backup`/`restore` (`SPEC-05`, its
//! authority-backup step).
//! `cargo rivet key backup`/`restore` and `cargo rivet fleet objects
//! backup`/`restore` are the second and third callers — see this crate's
//! `Cargo.toml` for why a third caller in a *different* binary crate is
//! what forces a real crate here rather than a shared module.
//!
//! # One engine, because the failure mode is the same shape every time
//!
//! `copy_all`/`copy_named` copy OUT; the same functions, called with the
//! arguments reversed, copy back IN. Every direction shares the identical
//! refusal: **do not silently overwrite what is already at the
//! destination.** A backup that quietly replaced last week's good copy with
//! today's broken one, and a restore that landed on top of live material,
//! are the same mistake pointed in opposite directions.
//!
//! # What this crate does NOT do
//!
//! It copies bytes. It has no opinion about what those bytes mean, whether
//! they should be encrypted first, or which digest a caller checks them
//! against afterwards — `cargo rivet key backup` encrypts before handing
//! bytes here, and `cargo rivet fleet objects restore` checks a BLAKE3
//! digest before handing bytes here, but neither concern belongs in a crate
//! `rivet-core` (ADR-014 §3: *"NO I/O"*) cannot depend on and every
//! workstation binary that backs anything up can.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

/// One file that was copied, and how big it is.
pub struct Copied {
    /// The file's own name — no directory, because the report is already
    /// under a heading that names one.
    pub name: String,
    pub bytes: u64,
}

/// One file already at the destination that a copy would have silently
/// replaced.
#[derive(Debug)]
pub struct Existing {
    pub name: String,
    pub bytes: u64,
    /// How long ago it was written, or `None` if the filesystem would not
    /// say — printed as "age unknown" rather than guessed at.
    pub age: Option<Duration>,
}

/// What stopped a copy.
#[derive(Debug)]
pub enum Error {
    /// `source` holds nothing to copy — refused rather than silently writing
    /// zero files and calling that a backup.
    Empty(PathBuf),
    /// The destination already holds material this copy would replace.
    /// Nothing was copied: a half backup or a half restore is worse than
    /// none, because it looks complete until the day it is read.
    WouldOverwrite(Vec<Existing>),
    /// [`copy_named`] was asked for a file `source` does not have. Named
    /// rather than folded into [`Error::Io`], because "the source is
    /// inconsistent with what asked for it" is a different fact from "the
    /// filesystem refused" — a fleet repository whose manifest names a
    /// digest `objects/` does not hold, for instance.
    Missing(Vec<String>),
    /// A source file and its would-be destination are the same file —
    /// literally, an equivalent spelling of one path, or an alias through a
    /// symlink or hard link. `replace` authorizes overwriting a *different*
    /// file's old content; it is not a license to copy a file onto itself,
    /// which truncates the one copy that exists before there is anything
    /// left to read from it. Refused before any file in the set is copied.
    SameFile(Vec<PathBuf>),
    /// Whatever the filesystem said, with the path.
    Io(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty(dir) => write!(f, "{} holds nothing to copy", dir.display()),
            Self::WouldOverwrite(existing) => write!(
                f,
                "{} file(s) already at the destination would be replaced",
                existing.len()
            ),
            Self::Missing(names) => {
                write!(f, "{} named file(s) are not at the source: {}", names.len(), names.join(", "))
            }
            Self::SameFile(paths) => write!(
                f,
                "{} file(s) would be copied onto themselves: {}",
                paths.len(),
                paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            ),
            Self::Io(why) => write!(f, "{why}"),
        }
    }
}

/// **Copy every regular file `source` holds into `dest`.**
///
/// Creates `dest` when absent. Refuses — [`Error::WouldOverwrite`] —
/// if any file already at `dest` would be silently replaced, unless
/// `replace` is `true`. Nothing is copied on any refusal or any I/O failure
/// partway through the check: the existing-file survey runs to completion
/// before the first byte is written, so a caller never has to reason about a
/// partially-applied backup.
///
/// **Non-recursive, deliberately.** Every directory this crate's callers
/// point it at is flat by construction — `SPEC-05`'s authority directory,
/// `~/.rivet/keys`, and the fleet repository's `objects/` are each one
/// directory of sibling files, never a tree — so anything found in a
/// subdirectory here is not material this module has a way to reason about,
/// and it is left alone rather than guessed at.
///
/// # Errors
///
/// [`Error::Empty`] if `source` has nothing to copy, [`Error::WouldOverwrite`]
/// if `replace` is `false` and the destination already holds any of it, or
/// [`Error::Io`] for anything the filesystem said.
pub fn copy_all(source: &Path, dest: &Path, replace: bool) -> Result<Vec<Copied>, Error> {
    let files = flat_files(source)?;
    if files.is_empty() {
        return Err(Error::Empty(source.to_path_buf()));
    }
    copy_files(&files, dest, replace)
}

/// **Copy exactly the named files `source` holds into `dest`, and no
/// others.**
///
/// The extension `copy_all` does not offer: a caller with a *subset* of a
/// directory's files to move — `cargo rivet fleet objects backup`'s
/// referenced-digests default (`ADR-052`, R-183 D4), scoped narrower
/// than "everything `objects/` holds" (that whole-directory case is still
/// `copy_all`, unchanged, under `--all`).
///
/// **Empty `names` copies nothing and is not an error.** Unlike [`copy_all`],
/// where an empty *directory* means a backup command found nothing to
/// protect at all (a real refusal), an empty *selection* here means the
/// scope this caller asked for is legitimately empty — a freshly
/// initialised fleet repository with nothing published yet, for one. `Ok`
/// with an empty `Vec` is that state told straight, not a silent success
/// hiding a real refusal — the caller's own report says "0 objects" rather
/// than this function inventing a sentence for it.
///
/// # Errors
///
/// [`Error::Missing`] if any name in `names` is not a regular file directly
/// inside `source` (checked in full, and named in full, before anything is
/// copied — the same all-at-once ladder [`copy_all`] uses for
/// [`Error::WouldOverwrite`]), [`Error::WouldOverwrite`] if `replace` is
/// `false` and any named file already exists at `dest`, or [`Error::Io`] for
/// anything the filesystem said.
///
/// **A name is "directly inside `source`" only if it is exactly one plain
/// path component** — a single [`Component::Normal`] that reads back as the
/// whole string. A parent, a root, a prefix, or a nested path is refused as
/// [`Error::Missing`] the same as a name `source` simply does not have,
/// rather than resolved through the filesystem: `source.join(name)` walks
/// back out of `source` for `..`, and replaces `source` outright for an
/// absolute `name` (`PathBuf::join`'s documented behaviour for an absolute
/// argument), so this boundary is checked before that join is ever trusted.
pub fn copy_named(
    source: &Path,
    dest: &Path,
    names: &[String],
    replace: bool,
) -> Result<Vec<Copied>, Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }

    let mut missing = Vec::new();
    let mut files = Vec::new();
    for name in names {
        if is_direct_child_name(name) {
            let path = source.join(name);
            if path.is_file() {
                files.push(path);
                continue;
            }
        }
        missing.push(name.clone());
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Err(Error::Missing(missing));
    }
    files.sort();
    copy_files(&files, dest, replace)
}

/// Whether `name` names exactly one plain path component on this platform —
/// a single [`Component::Normal`] that reads back as the whole string.
///
/// This is what keeps [`copy_named`]'s `source.join(name)` from ever
/// resolving outside `source`: a `..`, a bare root or prefix, or a nested
/// path each contain more than one component (or, for a `Component::Normal`
/// compared against the original string, a separator [`Path`] parsed away
/// that `name` did not literally spell), and every one of those is refused
/// here rather than joined and trusted. Platform-native: a backslash is a
/// separator on Windows and an ordinary filename byte on Unix, exactly as
/// [`Path`] itself treats it.
fn is_direct_child_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(part)), None) if part == std::ffi::OsStr::new(name)
    )
}

/// The shared second half of [`copy_all`] and [`copy_named`]: check for a
/// same-file collision and an overwrite, then copy — the one place either
/// function actually touches `dest`.
fn copy_files(files: &[PathBuf], dest: &Path, replace: bool) -> Result<Vec<Copied>, Error> {
    // **Same-file preflight, before anything else — including before
    // `replace` is even consulted.** `replace=true` skips the overwrite
    // survey below entirely, and without this check a source copied onto
    // itself reached `std::fs::copy` unguarded: opening the destination
    // truncates it before there is anything left to read from the source,
    // because the two names are the one file. B05.
    let colliding = same_file_pairs(files, dest);
    if !colliding.is_empty() {
        return Err(Error::SameFile(colliding));
    }

    if !replace {
        let mut existing = Vec::new();
        for file in files {
            let Some(name) = file.file_name() else { continue };
            let target = dest.join(name);
            if let Ok(meta) = std::fs::metadata(&target) {
                existing.push(Existing {
                    name: name.to_string_lossy().into_owned(),
                    bytes: meta.len(),
                    age: meta
                        .modified()
                        .ok()
                        .and_then(|m| SystemTime::now().duration_since(m).ok()),
                });
            }
        }
        if !existing.is_empty() {
            existing.sort_by(|a, b| a.name.cmp(&b.name));
            return Err(Error::WouldOverwrite(existing));
        }
    }

    std::fs::create_dir_all(dest).map_err(|e| Error::Io(format!("{}: {e}", dest.display())))?;

    let mut copied = Vec::new();
    for file in files {
        let Some(name) = file.file_name() else { continue };
        let target = dest.join(name);
        std::fs::copy(file, &target)
            .map_err(|e| Error::Io(format!("{}: {e}", target.display())))?;
        let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        copied.push(Copied {
            name: name.to_string_lossy().into_owned(),
            bytes,
        });
    }
    Ok(copied)
}

/// Every `(source, would-be destination)` pair in `files` → `dest` that name
/// the SAME file, checked before `copy_files` writes anything — B05.
fn same_file_pairs(files: &[PathBuf], dest: &Path) -> Vec<PathBuf> {
    let mut colliding = Vec::new();
    for file in files {
        let Some(name) = file.file_name() else { continue };
        let target = dest.join(name);
        if is_same_file(file, &target) {
            colliding.push(target);
        }
    }
    colliding
}

/// Whether `a` and `b` name the same file, on disk rather than by spelling.
///
/// **Canonicalized-path comparison first**, which resolves `.`/`..`,
/// separator and case differences, and a symlink alias, on every platform —
/// `b` not existing yet (the ordinary case: nothing is at the destination)
/// makes canonicalization fail, which is correctly "not the same file" and
/// falls through rather than erroring.
///
/// **On Unix, additionally compare `(dev, ino)`.** A hard link is two
/// directory entries for one inode, and neither path is a symlink, so
/// canonicalizing either one just returns that same path back — the
/// comparison above cannot see the alias, and this one is what does.
///
/// Windows hard links are not compared. The by-handle identity a hard link
/// needs — `MetadataExt::file_index`/`volume_serial_number` — is gated
/// behind the unstable `windows_by_handle` feature (rust-lang/rust #63010)
/// on current stable Rust, so it is not reachable without a nightly
/// toolchain, which this workspace does not use. Canonicalizing either path
/// does not help either: a hard link is two directory entries for one file,
/// and canonicalizing one returns that same path unchanged, not the other
/// entry's. `SPEC-05`'s callers hand this crate flat directories of files an
/// operator would not ordinarily hard-link, so canonicalized-path detection
/// covers the cases those callers can actually produce; a Windows hard-link
/// alias is the gap, named here rather than silently assumed away.
fn is_same_file(a: &Path, b: &Path) -> bool {
    let canonicalized_match = matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(ca), Ok(cb)) if ca == cb
    );
    if canonicalized_match {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let same_inode = matches!(
            (std::fs::metadata(a), std::fs::metadata(b)),
            (Ok(ma), Ok(mb)) if ma.dev() == mb.dev() && ma.ino() == mb.ino()
        );
        if same_inode {
            return true;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
    }
    false
}

/// Every regular file directly inside `dir`, sorted — the flat, non-recursive
/// listing both [`copy_all`] and [`copy_named`]'s missing-file check start
/// from.
///
/// **Every entry is accounted for, one way or the other.** A directory-entry
/// read failure or a metadata read failure is propagated as [`Error::Io`]
/// rather than dropped: silently treating an entry this could not even stat
/// as "not a file" is how a backup would omit it while still reporting
/// success — I01. A subdirectory this CAN stat is still left alone, exactly
/// as before: this crate's callers only ever point it at flat directories,
/// and a subdirectory found there is not material it has a way to reason
/// about.
fn flat_files(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        let path = entry.path();
        let is_file = std::fs::metadata(&path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?
            .is_file();
        if is_file {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// An [`Existing::age`] rendered for a person, never a raw duration.
#[must_use]
pub fn age_text(age: Option<Duration>) -> String {
    match age {
        None => "age unknown".to_owned(),
        Some(d) if d.as_secs() < 86_400 => "less than a day old".to_owned(),
        Some(d) if d.as_secs() < 2 * 86_400 => "1 day old".to_owned(),
        Some(d) => format!("{} days old", d.as_secs() / 86_400),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// One scratch directory per test, so parallel tests do not collide —
    /// `AGENTS.md` §10.5's shared-cache warning is about the container, and
    /// the same discipline applies to any test that touches real paths.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rivet-filecopy-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// **A missing source directory is refused, not read as empty.** `Empty`
    /// and an absent directory are different facts, and the first silently
    /// including the second would tell an operator a backup was legitimately
    /// made of nothing.
    #[test]
    fn a_missing_source_is_an_io_error_not_an_empty_backup() {
        let base = scratch("missing-source");
        let source = base.join("does-not-exist");
        let dest = base.join("dest");
        match copy_all(&source, &dest, false) {
            Err(Error::Io(why)) => assert!(why.contains("does-not-exist"), "{why}"),
            other => panic!("expected Io, got a different outcome: {}", other.is_ok()),
        }
    }

    /// **An empty source directory is refused by name.** Writing zero files
    /// and calling that success is the shape of failure this exists to
    /// prevent — a backup command that ran and reported nothing wrong.
    #[test]
    fn an_empty_source_directory_is_refused() {
        let base = scratch("empty-source");
        let source = base.join("empty");
        std::fs::create_dir_all(&source).expect("an empty directory");
        let dest = base.join("dest");
        assert!(matches!(copy_all(&source, &dest, false), Err(Error::Empty(_))));
    }

    /// **The report names every file copied, with its size.** This is the
    /// requirement the maintainer set (`.rulings/` R-337): the report lists
    /// every file that was copied, each with its size — the evidence, not
    /// somebody's memory of having run the command.
    #[test]
    fn every_file_is_named_and_sized() {
        let base = scratch("names-and-sizes");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"the root certificate").expect("a file");
        std::fs::write(source.join("root.key"), b"the key").expect("a file");
        let dest = base.join("dest");

        let copied = copy_all(&source, &dest, false).expect("copies");
        assert_eq!(copied.len(), 2);
        let root_crt = copied.iter().find(|c| c.name == "root.crt").expect("root.crt");
        assert_eq!(root_crt.bytes, "the root certificate".len() as u64);
        let root_key = copied.iter().find(|c| c.name == "root.key").expect("root.key");
        assert_eq!(root_key.bytes, "the key".len() as u64);
        assert_eq!(
            std::fs::read(dest.join("root.crt")).expect("readable"),
            b"the root certificate"
        );
    }

    /// **A subdirectory in the source is left alone.** The authority
    /// directory is flat by construction; this asserts the module does not
    /// guess at what a subdirectory would mean.
    #[test]
    fn a_subdirectory_in_the_source_is_not_copied() {
        let base = scratch("subdirectory");
        let source = base.join("source");
        std::fs::create_dir_all(source.join("nested")).expect("a nested directory");
        std::fs::write(source.join("root.crt"), b"x").expect("a file");
        std::fs::write(source.join("nested").join("leftover"), b"y").expect("a nested file");
        let dest = base.join("dest");

        let copied = copy_all(&source, &dest, false).expect("copies");
        assert_eq!(copied.len(), 1, "only the flat file should be reported");
        assert!(!dest.join("nested").exists(), "the subdirectory was not copied");
    }

    /// **The refusal to overwrite an existing backup, and what it names.**
    /// This is the maintainer's requirement: a backup that
    /// quietly replaced last week's good copy with today's broken one is the
    /// failure the verb exists to prevent, so it must refuse and say what is
    /// already there.
    #[test]
    fn an_existing_file_at_the_destination_refuses_the_whole_copy() {
        let base = scratch("existing-backup");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"new").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join("root.crt"), b"old, and already here").expect("a file");

        match copy_all(&source, &dest, false) {
            Err(Error::WouldOverwrite(existing)) => {
                assert_eq!(existing.len(), 1);
                assert_eq!(existing[0].name, "root.crt");
                assert_eq!(existing[0].bytes, "old, and already here".len() as u64);
            }
            other => panic!("expected WouldOverwrite, got a different outcome: {}", other.is_ok()),
        }
        // **Nothing was copied.** A refused backup must not have partially
        // replaced what was already at the destination.
        assert_eq!(
            std::fs::read(dest.join("root.crt")).expect("still readable"),
            b"old, and already here"
        );
    }

    /// **`replace` is the explicit word that overrides the refusal**, and
    /// only that flag does — there is no silent path around it.
    #[test]
    fn replace_overwrites_what_was_there() {
        let base = scratch("replace");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"new").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join("root.crt"), b"old").expect("a file");

        let copied = copy_all(&source, &dest, true).expect("replace overrides the refusal");
        assert_eq!(copied.len(), 1);
        assert_eq!(std::fs::read(dest.join("root.crt")).expect("readable"), b"new");
    }

    /// **The round trip.** Mint what a throwaway authority would leave on
    /// disk, back it up, delete the "live" copy the way `rivet-reset` would,
    /// restore it, and assert every byte survived. This is `copy_all` used
    /// in both directions — `rivet-provision`'s `ca_backup` and `ca_restore`
    /// are thin wrappers around exactly this function, so proving it here
    /// proves the file half of both verbs without going near a certificate
    /// store.
    #[test]
    fn a_backup_and_a_restore_round_trip_every_byte() {
        let base = scratch("round-trip");
        let authority = base.join("authority");
        std::fs::create_dir_all(&authority).expect("an authority directory");
        let files: &[(&str, &[u8])] = &[
            ("root.crt", b"root certificate bytes"),
            ("root.key", b"root key bytes -- the fleet's whole trust"),
            ("browser.crt", b"browser certificate bytes"),
            ("browser.key", b"browser key bytes"),
            ("suffix", b"local\nhome-fleet.home.arpa"),
            ("server-arbor.local.crt", b"a fleet server's certificate"),
            ("server-arbor.local.key", b"a fleet server's key"),
        ];
        for (name, body) in files {
            std::fs::write(authority.join(name), body).expect("a file");
        }

        // Back it up.
        let backup = base.join("backup");
        let copied = copy_all(&authority, &backup, false).expect("backs up");
        assert_eq!(copied.len(), files.len());

        // Destroy the "live" authority — what rivet-reset does.
        for (name, _) in files {
            std::fs::remove_file(authority.join(name)).expect("remove");
        }
        std::fs::remove_dir(&authority).expect("remove the now-empty directory");
        assert!(!authority.exists());

        // Restore it.
        let restored = copy_all(&backup, &authority, false).expect("restores");
        assert_eq!(restored.len(), files.len());

        // Every byte survived the round trip.
        for (name, body) in files {
            assert_eq!(
                std::fs::read(authority.join(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
                *body,
                "{name} did not survive the round trip"
            );
        }
    }

    #[test]
    fn age_is_worded_for_a_person() {
        assert_eq!(age_text(None), "age unknown");
        assert_eq!(age_text(Some(Duration::from_secs(3600))), "less than a day old");
        assert_eq!(age_text(Some(Duration::from_secs(90_000))), "1 day old");
        assert_eq!(age_text(Some(Duration::from_secs(5 * 86_400))), "5 days old");
    }

    // -- copy_named ---------------------------------------------------------

    /// **An empty selection copies nothing and is `Ok`** — the case
    /// `copy_all` cannot express (it would refuse on `Error::Empty`), and
    /// the one `cargo rivet fleet objects backup` needs for a fresh
    /// repository with nothing published yet.
    #[test]
    fn copy_named_with_no_names_copies_nothing_and_is_not_an_error() {
        let base = scratch("named-empty");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        let dest = base.join("dest");
        let copied = copy_named(&source, &dest, &[], false).expect("an empty selection is Ok");
        assert!(copied.is_empty());
    }

    /// **Only the named files are copied, never the rest of the directory.**
    #[test]
    fn copy_named_copies_only_what_was_named() {
        let base = scratch("named-subset");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("aaaa"), b"a").expect("a file");
        std::fs::write(source.join("bbbb"), b"b").expect("a file");
        std::fs::write(source.join("cccc"), b"c").expect("a file");
        let dest = base.join("dest");

        let copied = copy_named(&source, &dest, &["aaaa".to_owned(), "cccc".to_owned()], false)
            .expect("copies the two named files");
        assert_eq!(copied.len(), 2);
        assert!(dest.join("aaaa").is_file());
        assert!(dest.join("cccc").is_file());
        assert!(!dest.join("bbbb").exists(), "an unnamed file must not be copied");
    }

    /// **A name the source does not have is refused, in full, before
    /// anything is copied** — the fleet-repository-inconsistency case: a
    /// manifest naming a digest `objects/` does not hold.
    #[test]
    fn copy_named_refuses_a_missing_name_and_copies_nothing() {
        let base = scratch("named-missing");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("aaaa"), b"a").expect("a file");
        let dest = base.join("dest");

        match copy_named(&source, &dest, &["aaaa".to_owned(), "zzzz".to_owned()], false) {
            Err(Error::Missing(names)) => assert_eq!(names, vec!["zzzz".to_owned()]),
            other => panic!("expected Missing, got a different outcome: {}", other.is_ok()),
        }
        assert!(!dest.exists(), "nothing should have been written on a Missing refusal");
    }

    /// **A name that does not name a file directly inside `source` is
    /// refused as missing, never resolved through the filesystem.** A parent
    /// component, a nested path, and an absolute path each reach outside
    /// `source` if handed straight to `source.join(name)`; this proves none
    /// of the three do, and that nothing is ever created at `dest`.
    #[test]
    fn copy_named_refuses_a_name_reaching_outside_the_source_directory() {
        let base = scratch("named-outside");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(base.join("outside.txt"), b"not part of source").expect("a file");
        let subdir = source.join("subdir");
        std::fs::create_dir_all(&subdir).expect("a nested directory");
        std::fs::write(subdir.join("file.txt"), b"nested").expect("a file");
        let dest = base.join("dest");

        let absolute = base.join("outside.txt");
        assert!(absolute.is_absolute(), "fixture must actually be absolute");

        for bad in [
            "../outside.txt".to_owned(),
            "subdir/file.txt".to_owned(),
            absolute.to_string_lossy().into_owned(),
        ] {
            match copy_named(&source, &dest, std::slice::from_ref(&bad), false) {
                Err(Error::Missing(names)) => assert_eq!(names, vec![bad.clone()], "{bad}"),
                other => panic!(
                    "expected Missing for {bad:?}, got a different outcome: {}",
                    other.is_ok()
                ),
            }
            assert!(!dest.exists(), "{bad} must not have created a destination");
        }
    }

    /// **A valid name mixed with one that escapes `source` refuses the whole
    /// request, in either order** — the same all-or-nothing ladder already
    /// proven for a name that is simply absent. The escaping name's target
    /// genuinely exists outside `source`, so a boundary check that merely
    /// happened to find nothing there cannot pass this by accident.
    #[test]
    fn copy_named_refuses_the_whole_request_when_one_name_escapes_source() {
        let base = scratch("named-outside-mixed");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("aaaa"), b"a").expect("a file");
        std::fs::write(base.join("escapes.txt"), b"outside source, and real").expect("a file");
        let dest = base.join("dest");

        let good = "aaaa".to_owned();
        let bad = "../escapes.txt".to_owned();

        assert!(matches!(
            copy_named(&source, &dest, &[good.clone(), bad.clone()], false),
            Err(Error::Missing(_))
        ));
        assert!(!dest.exists(), "good-then-bad order must copy nothing");

        assert!(matches!(
            copy_named(&source, &dest, &[bad, good], false),
            Err(Error::Missing(_))
        ));
        assert!(!dest.exists(), "bad-then-good order must copy nothing");
    }

    /// **A backslash is a separator on Windows and an ordinary filename byte
    /// on Unix — the boundary check must agree with the platform, not with a
    /// single hard-coded rule.** On Unix, a file genuinely named `a\b` is one
    /// plain component and copies normally; the Windows sibling of this test
    /// is below.
    #[cfg(unix)]
    #[test]
    fn copy_named_treats_a_literal_backslash_as_an_ordinary_unix_filename_byte() {
        let base = scratch("named-backslash-unix");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("a\\b"), b"a file whose name contains a backslash")
            .expect("a file");
        let dest = base.join("dest");

        let copied = copy_named(&source, &dest, &["a\\b".to_owned()], false)
            .expect("a literal backslash is one ordinary Unix path component");
        assert_eq!(copied.len(), 1);
        assert!(dest.join("a\\b").is_file());
    }

    /// **On Windows, `a\b` is two path components — a directory and a file —
    /// so it is refused exactly as `subdir/file.txt` is.** `source/a/b`
    /// genuinely exists here, so a check that only happens not to find a
    /// nested path cannot pass this by accident.
    #[cfg(windows)]
    #[test]
    fn copy_named_refuses_a_backslash_as_a_windows_path_separator() {
        let base = scratch("named-backslash-windows");
        let source = base.join("source");
        let nested = source.join("a");
        std::fs::create_dir_all(&nested).expect("a nested directory");
        std::fs::write(nested.join("b"), b"reachable only through a\\b").expect("a file");
        let dest = base.join("dest");

        match copy_named(&source, &dest, &["a\\b".to_owned()], false) {
            Err(Error::Missing(names)) => assert_eq!(names, vec!["a\\b".to_owned()]),
            other => panic!("expected Missing, got a different outcome: {}", other.is_ok()),
        }
        assert!(!dest.exists());
    }

    /// **`copy_named` refuses an overwrite exactly as `copy_all` does**, and
    /// `--replace`'s equivalent overrides it the same way.
    #[test]
    fn copy_named_refuses_to_overwrite_unless_told_to() {
        let base = scratch("named-overwrite");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("aaaa"), b"new").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join("aaaa"), b"old").expect("a file");

        assert!(matches!(
            copy_named(&source, &dest, &["aaaa".to_owned()], false),
            Err(Error::WouldOverwrite(_))
        ));
        assert_eq!(std::fs::read(dest.join("aaaa")).expect("still old"), b"old");

        let copied =
            copy_named(&source, &dest, &["aaaa".to_owned()], true).expect("replace overrides");
        assert_eq!(copied.len(), 1);
        assert_eq!(std::fs::read(dest.join("aaaa")).expect("now new"), b"new");
    }

    // -- B05: a file copied onto itself must not be truncated ---------------

    /// **`source == dest`, with `replace: true` — the exact shape `replace`
    /// was never meant to authorize.** `replace` skips the overwrite survey
    /// entirely, so before B05's fix this reached `std::fs::copy(file,
    /// &target)` with `file == target` and truncated the one copy that
    /// exists. Refused instead, and the file must still hold its original
    /// bytes afterward — the assertion that actually catches truncation,
    /// which a bare `is_err()` would not.
    #[test]
    fn copying_a_directory_onto_itself_is_refused_and_nothing_is_truncated() {
        let base = scratch("self-copy");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"the only copy of this authority's root")
            .expect("a file");

        match copy_all(&source, &source, true) {
            Err(Error::SameFile(paths)) => {
                assert_eq!(paths, vec![source.join("root.crt")]);
            }
            other => panic!("expected SameFile, got a different outcome: {}", other.is_ok()),
        }
        assert_eq!(
            std::fs::read(source.join("root.crt")).expect("still readable"),
            b"the only copy of this authority's root",
            "the source must not have been truncated by its own refused copy"
        );
    }

    /// **A lexically different spelling of the same directory** — reached
    /// through `..` rather than a symlink — is still the same file once
    /// resolved, and canonicalization is what a bare string compare would
    /// miss.
    #[test]
    fn an_equivalent_spelling_of_the_source_path_is_also_refused() {
        let base = scratch("self-copy-equivalent");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"original bytes").expect("a file");

        // Lexically `base/source/../source`, not `base/source` — a
        // different `PathBuf`, the same directory once resolved.
        let equivalent = base.join("source").join("..").join("source");
        assert_ne!(equivalent, source, "the point of this test is a DIFFERENT spelling");

        assert!(matches!(copy_all(&source, &equivalent, true), Err(Error::SameFile(_))));
        assert_eq!(
            std::fs::read(source.join("root.crt")).expect("still readable"),
            b"original bytes"
        );
    }

    /// **The refusal covers the WHOLE set, before the first write** — a
    /// multi-file `copy_named` where only the alphabetically LATER name
    /// collides must still refuse before the earlier one is copied. Proves
    /// the preflight in `copy_files` runs to completion rather than copying
    /// as it goes and stopping partway.
    #[cfg(unix)]
    #[test]
    fn a_collision_later_in_the_set_stops_an_earlier_file_from_being_copied() {
        let base = scratch("self-copy-partial");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("aaaa"), b"a, and it must stay only in source").expect("a");
        std::fs::write(source.join("bbbb"), b"b").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        // dest/bbbb is an alias of source/bbbb — "aaaa" sorts first, so a
        // copy loop that wrote as it went would have already placed it at
        // dest/aaaa by the time it reached the "bbbb" collision.
        std::os::unix::fs::symlink(source.join("bbbb"), dest.join("bbbb")).expect("symlink");

        let names = vec!["aaaa".to_owned(), "bbbb".to_owned()];
        assert!(matches!(copy_named(&source, &dest, &names, true), Err(Error::SameFile(_))));
        assert!(!dest.join("aaaa").exists(), "the earlier file must not have been copied");
    }

    /// **A symlink standing in for the whole destination directory** — the
    /// alias case the canonicalized-path compare exists for.
    #[cfg(unix)]
    #[test]
    fn a_symlink_alias_of_the_source_directory_is_refused() {
        let base = scratch("self-copy-symlink");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"authority bytes").expect("a file");
        let alias = base.join("alias");
        std::os::unix::fs::symlink(&source, &alias).expect("symlink");

        assert!(matches!(copy_all(&source, &alias, true), Err(Error::SameFile(_))));
        assert_eq!(std::fs::read(source.join("root.crt")).expect("still readable"), b"authority bytes");
    }

    /// **A hard link — two directory entries, one inode, neither a
    /// symlink** — is exactly what canonicalized-path comparison alone
    /// cannot see, and exactly what the `(dev, ino)` compare is for.
    #[cfg(unix)]
    #[test]
    fn a_hard_link_alias_is_refused() {
        use std::os::unix::fs::MetadataExt as _;

        let base = scratch("self-copy-hardlink");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"authority bytes").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::hard_link(source.join("root.crt"), dest.join("root.crt")).expect("hard_link");
        // Confirm the fixture is actually a hard link (one inode, two
        // names) rather than assuming the platform granted it.
        let ma = std::fs::metadata(source.join("root.crt")).expect("metadata");
        let mb = std::fs::metadata(dest.join("root.crt")).expect("metadata");
        assert_eq!((ma.dev(), ma.ino()), (mb.dev(), mb.ino()), "fixture is not actually hard-linked");

        assert!(matches!(copy_all(&source, &dest, true), Err(Error::SameFile(_))));
        assert_eq!(std::fs::read(source.join("root.crt")).expect("still readable"), b"authority bytes");
    }

    /// **A Windows junction standing in for the whole destination
    /// directory** — the same alias case as the Unix symlink test above,
    /// created the way `cmd /C mklink /J` does it, which needs no admin
    /// rights.
    #[cfg(windows)]
    #[test]
    fn a_junction_alias_of_the_source_directory_is_refused() {
        let base = scratch("self-copy-junction");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"authority bytes").expect("a file");
        let alias = base.join("alias");

        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&alias)
            .arg(&source)
            .status()
            .expect("mklink runs");
        assert!(status.success(), "mklink /J failed: {status}");

        assert!(matches!(copy_all(&source, &alias, true), Err(Error::SameFile(_))));
        assert_eq!(std::fs::read(source.join("root.crt")).expect("still readable"), b"authority bytes");
    }

    /// **Replacement between genuinely different files is unaffected** — the
    /// non-regression control for B05: two distinct files with the same
    /// name in unrelated directories must still replace exactly as before.
    #[test]
    fn replacement_between_different_files_still_succeeds_after_the_same_file_guard() {
        let base = scratch("self-copy-control");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"new bytes").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join("root.crt"), b"old bytes").expect("a file");

        let copied = copy_all(&source, &dest, true).expect("genuinely different files still replace");
        assert_eq!(copied.len(), 1);
        assert_eq!(std::fs::read(dest.join("root.crt")).expect("readable"), b"new bytes");
    }

    // -- I01: an entry this crate cannot stat must be reported, not skipped -

    /// **A dangling symlink inside the source directory.** `read_dir`
    /// enumerates it without touching its target, but `std::fs::metadata`
    /// (which follows it, matching the ORIGINAL `Path::is_file` this
    /// replaced) fails with `NotFound`, since a dangling symlink's target is
    /// never there to resolve. Before I01's fix, `entries.flatten()` kept
    /// the entry and `.filter(|p| p.is_file())` silently read that failure
    /// as "not a file", so the backup would have run to completion short
    /// one entry and called it done. Now it is refused instead.
    #[cfg(unix)]
    #[test]
    fn an_entry_this_crate_cannot_stat_is_reported_not_silently_dropped() {
        let base = scratch("unreadable-entry");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"a real file").expect("a file");
        std::os::unix::fs::symlink(base.join("nowhere-xyz"), source.join("dangling"))
            .expect("a dangling symlink");
        let dest = base.join("dest");

        match copy_all(&source, &dest, false) {
            Err(Error::Io(why)) => assert!(why.contains("dangling"), "{why}"),
            other => panic!(
                "expected Io naming the unreadable entry, got a different outcome: {}",
                other.is_ok()
            ),
        }
        assert!(!dest.exists(), "nothing should have been written when an entry could not be examined");
    }
}
