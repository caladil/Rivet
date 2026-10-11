//! `cargo rivet bump` — increment an application's patch version and commit it,
//! alone. The deliberate pre-package ritual `tools/card/bump.sh` already is for
//! the system image, generalized to a target manifest.
//!
//! Ruled 2026-09-22 (option A): the patch number moves each time a standalone
//! application repository is packaged (.rulings/ R-323), and the honest place
//! for that is a **committed step before packaging**, not a side effect of
//! [`crate::package`], which is a pure read of the tree and must stay one so its
//! `-dirty` claim (`ADR-027` §3) stays true.
//!
//! # It mirrors `tools/card/bump.sh`, and does not reinvent it
//!
//! The sequence is `ADR-027` §10's, the one `bump.sh` runs for the operating
//! system: **refuse a dirty tree → increment the patch → commit that one change
//! alone.** Committing first is what makes version and commit a permanent 1:1
//! mapping, so no artifact is ever stamped `-dirty` for a number that was minted
//! and not recorded. This verb is that sequence pointed at whatever manifest it
//! is given, instead of hard-wired to this workspace's root.
//!
//! # PATCH ONLY — never the minor, never the major
//!
//! `vX.Y.Z` bumps `Z`. `Y` and `X` are left exactly as written. This is
//! absolute in this project.
//!
//! # What it will NOT bump — the guard
//!
//! A manifest whose version is `version.workspace = true` **inherits** its
//! number from a workspace root elsewhere. Bumping it would move the whole
//! workspace's version — and for a `cmd/*` binary of this repository that number
//! is the operating system's, which `tools/card/bump.sh` alone mints
//! (`ADR-027` §10). So this verb refuses an inherited version and points the
//! operator at the root / at `bump.sh`. It is for a **standalone application
//! repository that owns its own version** — a `[package] version = "…"` literal,
//! or its own `[workspace.package] version`.
//!
//! # Libraries are out of scope
//!
//! `cargo rivet library` takes its version from `--version`, a free string that
//! is never parsed and never compared — the digest is a library's identity. A
//! library has no versioned source file to read a number from or write one back
//! to, so there is nothing here for this verb to bump.

use std::path::{Path, PathBuf};
use std::process::Command;

use toml_edit::{DocumentMut, Item};

pub fn usage() -> String {
    "usage: cargo rivet bump [PATH]\n\
     \n\
     Increments the PATCH version (the Z of X.Y.Z, never the minor or the major)\n\
     of a manifest, then commits that one change alone — the deliberate,\n\
     committed pre-package step tools/card/bump.sh already is for the system\n\
     image, ADR-027 §10.2:\n\
     \n\
     \x20 refuse a dirty tree -> increment the patch -> commit \"Version X.Y.Z\"\n\
     \n\
     Committing first makes version and commit a 1:1 mapping, so no artifact is\n\
     ever stamped -dirty for a number that was minted and not recorded.\n\
     \n\
     PATH   the Cargo.toml to bump, or a directory holding one. Default: the\n\
     \x20      Cargo.toml in the current directory.\n\
     \n\
     WHAT IT BUMPS. A `[package] version = \"…\"` literal, or a\n\
     `[workspace.package] version` this file owns. A version written as\n\
     `version.workspace = true` INHERITS from a workspace root elsewhere, and\n\
     this refuses it: bumping there moves the whole workspace's number, and for a\n\
     cmd/* binary of this repository that number is the operating system's, which\n\
     tools/card/bump.sh alone mints (ADR-027 §10.2). Point this at a standalone\n\
     application repository that owns its own version, or run bump.sh for the OS.\n\
     \n\
     IT REFUSES A DIRTY TREE, the same as bump.sh: the number is minted on a tree\n\
     with nothing else on it so the mapping to a commit is exact. Commit or set\n\
     your other work aside first.\n\
     \n\
     LIBRARIES ARE OUT OF SCOPE. `cargo rivet library` versions are a free\n\
     --version string, never parsed and never compared — the digest is a\n\
     library's identity — so there is no versioned source for this to bump.\n"
        .to_string()
}

/// Parse the argument surface: an optional manifest path (or `-h`/`--help`).
///
/// `None` means the default, `./Cargo.toml`. A directory is resolved to the
/// `Cargo.toml` inside it, so `cargo rivet bump .` and `cargo rivet bump ../app`
/// both work.
enum Parsed {
    Help,
    Manifest(PathBuf),
}

fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut path: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            other if other.starts_with('-') => {
                return Err(format!("{other:?} is not an option `bump` takes.\n\n{}", usage()));
            }
            other => {
                if path.is_some() {
                    return Err(format!(
                        "bump takes at most one PATH and was given a second, {other:?}.\n\n{}",
                        usage()
                    ));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    Ok(Parsed::Manifest(resolve_manifest(path)))
}

/// A bare path, a directory, or the default, resolved to a `Cargo.toml`.
fn resolve_manifest(path: Option<PathBuf>) -> PathBuf {
    match path {
        None => PathBuf::from("Cargo.toml"),
        Some(given) if given.is_dir() => given.join("Cargo.toml"),
        Some(given) => given,
    }
}

pub fn run(args: &[String]) -> Result<String, String> {
    let manifest = match parse(args)? {
        Parsed::Help => return Ok(usage()),
        Parsed::Manifest(path) => path,
    };

    let manifest = manifest
        .canonicalize()
        .map_err(|e| format!("cannot find {}: {e}", manifest.display()))?;
    let dir = manifest
        .parent()
        .ok_or("the manifest path has no directory")?
        .to_path_buf();

    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{} is not valid TOML: {e}", manifest.display()))?;

    // Where the number lives, or the guard's refusal — computed before anything
    // touches git or the tree, so a manifest this verb will not bump costs
    // nothing and changes nothing.
    let located = locate(&doc).map_err(|why| format!("{}: {why}", manifest.display()))?;
    let next = bump_patch(&located.current)?;

    // **Refuse a dirty tree — the first step of ADR-027 §10's bump sequence,
    // bump.sh's own refusal.** The
    // version and the commit are a 1:1 mapping, and they can only be that if the
    // number is minted on a tree with nothing else on it.
    let status = git(&dir, &["status", "--porcelain"])?;
    if !status.trim().is_empty() {
        let listed: String = status
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect();
        return Err(format!(
            "cargo rivet bump: the tree is dirty and ADR-027 §10.2 refuses to mint a version \
             from it.\n\
             \n\
             The version and the commit are a 1:1 mapping, and they can only be that if the \
             number is minted on a tree with nothing else on it. Commit or set your other work \
             aside first:\n\
             \n\
             {listed}"
        ));
    }

    // **Increment the patch — step 2.** The located field is rewritten; the rest
    // of the document keeps its formatting because it is edited in place.
    set_version(&mut doc, &located, &next);

    // Read the edit back before committing to it, exactly as bump.sh does: an
    // edit that did not take must not become a commit.
    let check = locate(&doc).map_err(|why| {
        format!("the bump wrote a document {} can no longer read a version from: {why}", manifest.display())
    })?;
    if check.current != next {
        return Err(format!(
            "the bump did not take: {} still reads {:?} after the edit, not {next:?}. Nothing \
             was written.",
            manifest.display(),
            check.current
        ));
    }
    std::fs::write(&manifest, doc.to_string())
        .map_err(|e| format!("cannot write {}: {e}", manifest.display()))?;

    // **Refresh the lockfile — bump.sh:166.** A Cargo.lock carries every path
    // package's version too; left stale it dirties the tree the moment anyone
    // looks, and ADR-027 §10's refusal then fires on the NEXT bump, blaming a
    // tree nobody edited. `cargo metadata` rewrites it as a side effect. Only
    // run when there is a lock beside this manifest to refresh.
    let lock = dir.join("Cargo.lock");
    let have_lock = lock.is_file();
    if have_lock {
        refresh_lock(&dir, &manifest)?;
    }

    // **Commit that one change alone — step 3.** The manifest, and the lock when
    // it is tracked. `git add` of the lock is gated on it already being tracked
    // so an untracked lock in a scratch directory is never swept into the commit.
    stage(&dir, &manifest)?;
    if have_lock && is_tracked(&dir, &lock)? {
        stage(&dir, &lock)?;
    }
    commit(&dir, &next)?;

    // **The sequence must END clean — step 4.** If the commit left anything
    // behind, the next run blames a tree nobody edited; say so here, where the
    // cause is one screen away.
    let left = git(&dir, &["status", "--porcelain"])?;
    if !left.trim().is_empty() {
        let listed: String = left.lines().map(|line| format!("    {line}\n")).collect();
        return Err(format!(
            "cargo rivet bump: {} was committed, but the tree is NOT clean afterwards. Something \
             carries the version and was not refreshed:\n\n{listed}",
            next
        ));
    }

    let describe = git(&dir, &["describe", "--tags", "--always", "--dirty"])
        .unwrap_or_else(|_| "(git describe unavailable)".to_string());

    Ok(format!(
        "version   {} -> {}\n\
         source    {}\n\
         manifest  {}\n\
         committed Version {}  ({})\n",
        located.current,
        next,
        located.source.describe(),
        manifest.display(),
        next,
        describe.trim(),
    ))
}

/// Which field of a manifest carries the version this verb may bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// `[package] version = "…"` — a crate that states its own literal.
    PackageLiteral,
    /// `[workspace.package] version = "…"` — a workspace root that owns the
    /// number its members inherit.
    WorkspacePackageLiteral,
}

impl Source {
    fn describe(self) -> &'static str {
        match self {
            Self::PackageLiteral => "[package] version",
            Self::WorkspacePackageLiteral => "[workspace.package] version",
        }
    }
}

/// The version and where it lives, or a refusal.
#[derive(Debug)]
struct Located {
    source: Source,
    current: String,
}

/// **Find the number to bump, or refuse.** Pure over a parsed document, so the
/// whole decision — including the inherited-version guard — is testable without
/// a filesystem or a git tree.
///
/// The order matters. A file that owns a `[workspace.package] version` is a
/// workspace root, and its `[package]` (if any) usually inherits from that same
/// table — so that literal is checked first and is the one bumped. Only a
/// manifest with **no** owned version and a `version.workspace = true` is the
/// inheriting member the guard exists for.
fn locate(doc: &DocumentMut) -> Result<Located, String> {
    // A workspace root this file owns — bump.sh's own target, and the case an
    // adopter's single-file workspace root is in.
    if let Some(literal) = doc
        .get("workspace")
        .and_then(Item::as_table)
        .and_then(|w| w.get("package"))
        .and_then(Item::as_table)
        .and_then(|p| p.get("version"))
        .and_then(Item::as_str)
    {
        return Ok(Located {
            source: Source::WorkspacePackageLiteral,
            current: literal.to_string(),
        });
    }

    let package = doc.get("package").and_then(Item::as_table);

    // **The guard.** `version.workspace = true` with no `[workspace.package]`
    // in this file is a member inheriting from a root elsewhere.
    let inherits = package
        .and_then(|p| p.get("version"))
        .and_then(Item::as_table_like)
        .and_then(|t| t.get("workspace"))
        .and_then(|w| w.as_value())
        .and_then(toml_edit::Value::as_bool)
        .unwrap_or(false);
    if inherits {
        return Err(
            "its version is `version.workspace = true`, inherited from a workspace root \
             elsewhere. Bumping it would move the WHOLE workspace's number — and for a cmd/* \
             binary of this repository that number is the operating system's, which \
             tools/card/bump.sh alone mints (ADR-027 §10.2). Run bump.sh for the OS, or point \
             `cargo rivet bump` at the workspace root that owns the [workspace.package] version. \
             This verb is for a standalone application repository that owns its own version"
                .to_string(),
        );
    }

    // A crate that states its own literal.
    if let Some(literal) = package.and_then(|p| p.get("version")).and_then(Item::as_str) {
        return Ok(Located {
            source: Source::PackageLiteral,
            current: literal.to_string(),
        });
    }

    Err(
        "no version to bump. A manifest this verb bumps has either a `[package] version = \"…\"` \
         literal or a `[workspace.package] version` it owns; this has neither"
            .to_string(),
    )
}

/// Rewrite the located field to `next`, in place, leaving the rest untouched.
fn set_version(doc: &mut DocumentMut, located: &Located, next: &str) {
    let item = toml_edit::value(next);
    match located.source {
        Source::PackageLiteral => doc["package"]["version"] = item,
        Source::WorkspacePackageLiteral => doc["workspace"]["package"]["version"] = item,
    }
}

/// **Increment the patch of an `X.Y.Z` version, and only the patch.**
///
/// `tools/card/bump.sh`'s arithmetic exactly (`MAJOR.$MINOR.$((PATCH + 1))`):
/// major and minor are carried through as written, and the patch is the one
/// number that moves. `0.1.9` becomes `0.1.10`, not `0.2.0` — a decimal carry,
/// never a minor bump. `ADR-027` §10.
///
/// # Errors
///
/// A version that is not three integers separated by dots, refused rather than
/// guessed — the same shape `bump.sh` requires with its `[0-9]*.[0-9]*.[0-9]*`
/// check.
fn bump_patch(version: &str) -> Result<String, String> {
    let bad = || {
        format!(
            "{version:?} is not an X.Y.Z version this can bump: the major, minor and patch must \
             each be a whole number (tools/card/bump.sh's arithmetic, ADR-027 §10). PATCH is the \
             only component that ever changes"
        )
    };
    let mut parts = version.splitn(3, '.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(major), Some(minor), Some(patch))
            if is_number(major) && is_number(minor) && is_number(patch) =>
        {
            let next = patch
                .parse::<u64>()
                .map_err(|_| bad())?
                .checked_add(1)
                .ok_or_else(|| format!("the patch of {version:?} cannot be incremented: it would overflow"))?;
            // Major and minor are re-emitted as written — this bumps Z and
            // touches nothing else.
            Ok(format!("{major}.{minor}.{next}"))
        }
        _ => Err(bad()),
    }
}

/// A non-empty run of ASCII digits — a version component `bump_patch` accepts.
fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// git and cargo, run the way the rest of this crate runs them: never able to
// prompt, and with every failure named.
// ---------------------------------------------------------------------------

/// A git command in `dir` whose success is required; returns its trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // git must never be able to prompt during a bump.
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed in {} ({}): {}",
            args.join(" "),
            dir.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether a path is already tracked, so an untracked lock is never swept in.
fn is_tracked(dir: &Path, path: &Path) -> Result<bool, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--error-unmatch"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git ls-files: {e}"))?;
    Ok(out.status.success())
}

fn stage(dir: &Path, path: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("add")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git add: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git add {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// The commit, with `bump.sh`'s message shape: a subject and a one-paragraph
/// body, both passed as arguments — no shell, no here-document.
fn commit(dir: &Path, next: &str) -> Result<(), String> {
    let subject = format!("Version {next}");
    let body = "ADR-027 §10.2: the bump is committed before the package is built, so version and \
                commit are a 1:1 mapping and no artifact is ever stamped -dirty for a number that \
                was minted and not recorded.";
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "--quiet", "-m"])
        .arg(&subject)
        .arg("-m")
        .arg(body)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git commit: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// `cargo metadata` to rewrite `Cargo.lock` after the version moved — bump.sh's
/// own refresh, run in the manifest's directory so it resolves that workspace.
fn refresh_lock(dir: &Path, manifest: &Path) -> Result<(), String> {
    let out = Command::new("cargo")
        .current_dir(dir)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run cargo metadata to refresh Cargo.lock: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the version was bumped but `cargo metadata` could not refresh Cargo.lock ({}): {}. \
             Cargo.toml carries the new version; refresh the lock and commit, or discard the edit.",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---- the patch arithmetic — the whole point of the verb -----------------

    /// **The ordinary bump**, and the number this repository is actually at.
    #[test]
    fn a_patch_is_incremented() {
        assert_eq!(bump_patch("0.1.111").expect("a version"), "0.1.112");
    }

    /// **The carry** — a two-digit patch rolls without touching the minor.
    /// `0.1.9` is `0.1.10`, never `0.2.0`: this is the mistake the whole rule
    /// exists to forbid.
    #[test]
    fn a_patch_carries_into_two_digits_without_bumping_the_minor() {
        assert_eq!(bump_patch("0.1.9").expect("a version"), "0.1.10");
    }

    /// **The minor and the major are never touched.** Absolute in this project.
    #[test]
    fn the_minor_and_major_are_left_exactly_as_written() {
        assert_eq!(bump_patch("2.7.3").expect("a version"), "2.7.4");
        // The minor stays 7 and the major stays 2 across the carry too.
        assert_eq!(bump_patch("2.7.99").expect("a version"), "2.7.100");
        assert_eq!(bump_patch("13.0.0").expect("a version"), "13.0.1");
    }

    /// A version that is not three whole numbers is refused, not guessed —
    /// bump.sh's `[0-9]*.[0-9]*.[0-9]*` check in Rust.
    #[test]
    fn a_version_that_is_not_three_integers_is_refused() {
        for bad in ["0.1", "0.1.2.3", "1.2.x", "0.1.9-rc1", "", "v0.1.2", "0..1"] {
            assert!(
                bump_patch(bad).is_err(),
                "{bad:?} was accepted and should not have been"
            );
        }
    }

    // ---- locating the version, and the guard --------------------------------

    fn doc_of(text: &str) -> DocumentMut {
        text.parse::<DocumentMut>().expect("the fixture is valid TOML")
    }

    /// A crate that states its own literal is bumped in `[package]`.
    #[test]
    fn a_package_literal_is_located() {
        let located =
            locate(&doc_of("[package]\nname = \"app\"\nversion = \"0.3.4\"\n")).expect("a version");
        assert_eq!(located.source, Source::PackageLiteral);
        assert_eq!(located.current, "0.3.4");
    }

    /// A workspace root that owns the number is bumped in `[workspace.package]`,
    /// even when its own `[package]` inherits from that same table.
    #[test]
    fn a_workspace_package_literal_is_located_and_wins_over_an_inheriting_package() {
        let located = locate(&doc_of(
            "[workspace.package]\nversion = \"1.4.0\"\n\n[package]\nname = \"root\"\n\
             version.workspace = true\n",
        ))
        .expect("a version");
        assert_eq!(located.source, Source::WorkspacePackageLiteral);
        assert_eq!(located.current, "1.4.0");
    }

    /// **The guard.** A member inheriting `version.workspace = true` with no
    /// owned version is refused, and the refusal points at the root and bump.sh.
    #[test]
    fn an_inherited_version_is_refused_and_names_the_root() {
        let why = locate(&doc_of(
            "[package]\nname = \"rivet-proxyd\"\nversion.workspace = true\n",
        ))
        .expect_err("an inherited version must be refused");
        assert!(why.contains("version.workspace = true"), "{why}");
        assert!(why.contains("bump.sh"), "the refusal must name bump.sh: {why}");
    }

    /// A manifest with no version at all is refused, naming what one needs.
    #[test]
    fn a_manifest_with_no_version_is_refused() {
        let why = locate(&doc_of("[package]\nname = \"app\"\n"))
            .expect_err("no version to bump");
        assert!(why.contains("no version to bump"), "{why}");
    }

    /// **Locate, set, locate again** — the round trip `run` relies on to verify
    /// its own edit took, done here without touching git or the filesystem.
    #[test]
    fn setting_the_version_is_read_back_by_locate() {
        let mut doc = doc_of("[package]\nname = \"app\"\nversion = \"0.1.111\"\n");
        let located = locate(&doc).expect("a version");
        let next = bump_patch(&located.current).expect("a bump");
        set_version(&mut doc, &located, &next);
        assert_eq!(locate(&doc).expect("a version").current, "0.1.112");
        // And the surrounding manifest is intact.
        assert!(doc.to_string().contains("name = \"app\""));
    }
}
