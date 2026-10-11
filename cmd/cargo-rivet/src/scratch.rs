//! **One scratch root per test process, swept on the way in.**
//!
//! Every test helper in this crate used to mint its own top-level directory in
//! `std::env::temp_dir()`, named `<prefix>-<label>-<pid>`, and nothing ever
//! removed it. Each helper opened with a `remove_dir_all` that looks like
//! cleanup but is not: the name embeds the **process id**, which differs on
//! every run, so that call only ever matched a directory made by the *same*
//! process. A test run therefore left its entire fixture set in `%TEMP%`
//! permanently. On 2026-09-20 that was **11,490 orphaned directories** and 465
//! stray `.img` files on a machine whose system drive had filled.
//!
//! The fix is not a `Drop` guard. Call sites like
//! `fixture(&scratch("publish-seed-fixture"), ..)` pass the directory as a
//! temporary and keep using paths *inside* it afterwards; a guard would delete
//! the directory at the end of that statement and the test would fail on
//! something unrelated to what it is testing. Instead every scratch directory
//! becomes a child of a single per-process root, and each run deletes the roots
//! that earlier runs left behind. Two properties follow:
//!
//! * `%TEMP%` gains **one** entry per test process instead of hundreds.
//! * Nothing accumulates across runs, whether the previous run passed, failed,
//!   panicked, or was killed — a killed run cannot run cleanup code, so the
//!   sweep deliberately belongs to the *next* run rather than to this one.
//!
//! Helpers keep returning `PathBuf`, so this changed no call sites.

// A scratch directory that cannot be created is not a condition any test here
// can carry on past, and silently returning a path to nothing would fail later
// in some unrelated assertion. Same reasoning, and same allowance, as every
// `*_tests.rs` file in this crate.
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Everything this crate's tests write lives under `%TEMP%/<HOUSE>/<pid>`.
const HOUSE: &str = "cargo-rivet-tests";

/// How old a root must be before a later run will delete it.
///
/// A root is deleted by *other* processes, so this has to clear the longest a
/// live test process could plausibly still be running — the suite builds real
/// artifacts and signs them. Two hours is far beyond that and still bounds
/// `%TEMP%` to the runs of a single sitting.
const STALE_AFTER: Duration = Duration::from_secs(2 * 60 * 60);

/// This process's scratch root, created once and swept once.
fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let house = std::env::temp_dir().join(HOUSE);
        sweep(&house);
        let mine = house.join(std::process::id().to_string());
        std::fs::remove_dir_all(&mine).ok();
        std::fs::create_dir_all(&mine).expect("a scratch directory");
        mine
    })
}

/// Delete roots left by runs that have certainly finished.
///
/// Best-effort throughout: a root belonging to a live concurrent test binary is
/// younger than [`STALE_AFTER`] and is skipped, and one being deleted by
/// another process at the same moment simply fails and is left for next time.
/// A sweep that cannot do its job must never fail the test run it precedes.
fn sweep(house: &Path) {
    let Ok(entries) = std::fs::read_dir(house) else {
        return; // First run on this machine, or `%TEMP%` is unreadable.
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            std::fs::remove_dir_all(entry.path()).ok();
        }
    }
}

/// A fresh, empty directory nothing else is using, named for the test that
/// asked and scoped to this process.
///
/// `name` carries the caller's own prefix as well as its label, because two
/// modules may well both want a `"publish-ok"` and they now share a parent.
/// Re-asking for the same `name` within one process wipes and returns the same
/// path, which is what the helpers this replaced did.
pub(crate) fn dir(name: &str) -> PathBuf {
    let path = root().join(name);
    std::fs::remove_dir_all(&path).ok();
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path
}

/// A scratch path with **nothing at it** — the parent exists, the entry does
/// not, and whatever was there before is gone.
///
/// Several helpers hand back a path and leave creating it to the caller, or
/// name a file rather than a directory tree — the 465 stray `rivet-prov-*.img`
/// files were the latter. Those must not be created here, because a test that
/// asks whether the thing exists is entitled to be told no.
pub(crate) fn absent(name: &str) -> PathBuf {
    let path = root().join(name);
    std::fs::remove_file(&path).ok();
    std::fs::remove_dir_all(&path).ok();
    path
}
