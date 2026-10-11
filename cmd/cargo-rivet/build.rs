//! **Stamp this build with the commit it came from.**
//!
//! # The measurement that asked for this
//!
//! A cold agent given this repository and a board on 2026-08-29 lost more time
//! to this than to anything else. The `cargo-rivet.exe` on the workstation's
//! `PATH` was a stale build: five verbs, no `run`, no `identity`, and a `deploy`
//! that named a transport this project no longer has.
//! `target/debug/cargo-rivet.exe` had seven and spoke
//! the control plane. **Neither `--help` carried a version**, so the two builds
//! were indistinguishable — and an agent that trusts `PATH`, which is the normal
//! thing to do, concludes that `ADR-035`'s control plane has no client.
//!
//! A version alone would not have been enough either. `ADR-027` §10.2 makes
//! version and commit a permanent 1:1 mapping **for image builds**, because the
//! patch is incremented and committed alone before every one. Between those
//! bumps a workstation builds this tool many times from many commits, all of
//! them `0.1.9`. So the commit is what distinguishes two builds of a host tool,
//! and the version is what ties one to a release.
//!
//! # Where the string comes from, in order
//!
//! 1. **`RIVET_DESCRIBE`**, if it is set. This is the variable `tools/card/`
//!    already uses to hand a build its describe string, and `AGENTS.md` §10.5
//!    says why it exists: WSL's `git` and Windows' `git` disagree about whether
//!    this tree is dirty, so anything that goes into a NAME or a RECORD is asked
//!    of Windows' `git` and passed in. Honouring it here means one spelling.
//! 2. **`git describe --tags --always --dirty`**, run here.
//! 3. **`an unknown commit`** — said in as many words. A build outside a
//!    checkout is a real thing (a vendored crate, a source tarball) and
//!    inventing a plausible commit for one would be worse than admitting it.
//!
//! # What it does not promise
//!
//! `cargo` re-runs this when `build.rs` changes, when `RIVET_DESCRIBE` changes,
//! and when the two git files below change — `HEAD` moves on a checkout or a
//! commit, and `index` moves on a `git add`, which is what makes `-dirty` go
//! stale. That covers the ordinary cases and not every one: editing a tracked
//! file without staging it leaves a stamp that says clean until the next
//! rebuild. **So `-dirty` here is weaker than the claim `ADR-027` §3.2.1 makes
//! about an artifact's name**, which is composed at build time by a script that
//! refuses a dirty tree outright. This stamp identifies a binary; it does not
//! name an artifact, and nothing verifies against it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RIVET_DESCRIBE");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    if let Some(git_dir) = git_dir(&manifest) {
        // `HEAD` moves on a checkout or a commit; `index` moves on a `git add`.
        for file in ["HEAD", "index"] {
            let path = git_dir.join(file);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    let describe = std::env::var("RIVET_DESCRIBE")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| git_describe(&manifest))
        .unwrap_or_else(|| "an unknown commit".to_owned());

    println!("cargo:rustc-env=RIVET_BUILD_DESCRIBE={describe}");
}

/// This crate's `.git` directory, following a worktree's `gitdir:` pointer.
///
/// **A worktree is the normal case here**, not an edge one: agents work in
/// `.claude/worktrees/…`, where `.git` is a FILE naming the real directory.
fn git_dir(from: &Path) -> Option<PathBuf> {
    for directory in from.ancestors() {
        let candidate = directory.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.is_file() {
            let text = std::fs::read_to_string(&candidate).ok()?;
            let named = text.trim().strip_prefix("gitdir:")?.trim();
            let path = PathBuf::from(named);
            return Some(if path.is_absolute() { path } else { directory.join(path) });
        }
    }
    None
}

/// `git describe`, or nothing at all — never a guess.
fn git_describe(from: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .current_dir(from)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if text.is_empty() { None } else { Some(text) }
}
