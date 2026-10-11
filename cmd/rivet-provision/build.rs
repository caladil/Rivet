//! **Stamp this build with the commit it came from**, in the same spelling
//! `cmd/cargo-rivet/build.rs` uses — that file carries the full argument for
//! why a host tool needs a commit as well as a version, and this one exists
//! because a SECOND binary is now compared against it.
//!
//! `cargo rivet provision` launches this crate's binary as the interview and
//! refuses the pair when the two builds disagree (`.rulings/` R-191 D3): they
//! share the interview's files, the breadcrumb and the notebook format, and a
//! mismatch otherwise fails in odd ways. A comparison needs both sides to say
//! the same kind of thing, so `rivet-provision --version` prints
//! `<version> (<describe>)` exactly as `cargo-rivet --version` does.
//!
//! **The two scripts must compute the same string for the same tree**, and
//! they do so by construction rather than by agreement: each asks the same
//! `git describe --tags --always --dirty` in its own manifest directory,
//! inside the same repository, and each honours `RIVET_DESCRIBE` first for
//! `AGENTS.md` §10.5's reason (WSL's `git` and Windows' `git` disagree about
//! whether this tree is dirty, so anything that goes into a name or a record
//! is asked of one of them and passed in). A build that does drift is not
//! silent: the refusal prints both strings and names the reinstall.

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

/// This crate's `.git` directory, following a worktree's `gitdir:` pointer —
/// a worktree is the normal case here, where `.git` is a FILE naming the real
/// directory.
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
