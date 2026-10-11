//! **Removing a directory without deleting it in the supervisor's loop** —
//! `ADR-017` §3.
//!
//! Deleting a tree costs time in proportion to what is in it, and the loop is
//! single threaded. Renaming one costs a syscall. So every removal the
//! supervisor makes — an application's scratch on a spawn or an exit, an
//! uninstalled application's versions and data, a superseded library build, a
//! destroyed repository — renames its target aside **inside the directory that
//! held it** and hands the aside name to [`Kernel::reclaim_in_child`], whose
//! forked helper deletes it afterwards.
//!
//! **The port has no call that deletes a tree**, so the loop cannot do so by
//! accident: the deleting is all in the helper's own code, after `fork`.
//!
//! **The rename is the removal.** It is atomic, so from the moment it returns
//! the target's name is free and every reader that lists the directory passes
//! the aside over ([`rivet_core::service::is_set_aside`]); the operator is
//! answered then, and the space comes back shortly after.
//!
//! # The aside name
//!
//! `<stem>.reclaim-<ms>-<n>` beside the target, where `<ms>` is the monotonic
//! clock and `<n>` a count this process keeps, so two set aside in one
//! millisecond are two names. A removal's stem is empty — `.reclaim-<ms>-<n>`
//! — so that its name begins with `.`, which no live name in any directory a
//! removal renames inside can (`rivet_core::service::RECLAIM_MARK` says why),
//! and so that no removed name, however long, can push the aside past the
//! filesystem's limit on one entry. Scratch keeps its own stem, `tmp`, because
//! its helper runs at every spawn and reclaims only scratch.
//!
//! The monotonic clock starts again at every boot and the count at every
//! start, so a name left by a power cut can be the name a new removal wants.
//! A rename that finds its aside name taken moves to the next count.
//!
//! # After a power cut
//!
//! A helper deletes **every** entry carrying its prefix, not only the one it
//! was forked for, and [`sweep_leftovers`] forks one at start for each
//! directory still holding an aside. So nothing a power cut leaves set aside
//! outlives the next start.

use std::sync::atomic::{AtomicU64, Ordering};

use rivet_core::service::{self, RECLAIM_MARK, ServiceName};

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::SELF_NAME;

/// How many asides this process has named, which makes each name its own.
static NAMED: AtomicU64 = AtomicU64::new(0);

/// How many names a rename tries when each one it tries is already taken.
///
/// Each attempt is a different name, so the only way to exhaust this is a
/// directory holding that many leftovers named for this same millisecond in
/// a consecutive run — a run of power cuts at the same instant of boot.
const NAME_ATTEMPTS: usize = 16;

/// The prefix every aside of `stem` carries, and the one a helper is given.
#[must_use]
pub fn prefix(stem: &str) -> String {
    format!("{stem}{RECLAIM_MARK}")
}

/// **Rename `path` aside**, beside itself, under `stem`'s prefix, and answer
/// with the name it now has.
///
/// # Errors
///
/// Whatever `rename(2)` said — `ENOENT` for a path that is not there, which
/// each caller reads for itself. A name already taken is not an error: the
/// next one is tried.
pub fn rename_aside(k: &mut dyn Kernel, path: &str, stem: &str) -> Result<String, Errno> {
    let (parent, _) = path.rsplit_once('/').unwrap_or((".", path));
    let prefix = prefix(stem);
    let now = k.now_ms();
    let mut last = Errno::EEXIST;
    for _ in 0..NAME_ATTEMPTS {
        let n = NAMED.fetch_add(1, Ordering::Relaxed) + 1;
        let aside = format!("{parent}/{prefix}{now}-{n}");
        match k.rename(path, &aside) {
            Ok(()) => return Ok(aside),
            // A directory renamed onto a non-empty one, or onto a name that is
            // taken by something that is not a directory.
            Err(e) if e == Errno::ENOTEMPTY || e == Errno::EEXIST || e == Errno::ENOTDIR => {
                last = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// A removal that has happened: the name it was renamed to, and whether a
/// helper is deleting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetAside {
    /// Where the target now is, until the helper deletes it.
    pub aside: String,
    /// `Some` when the helper could not be forked. The target is still
    /// removed — its name is free — and the aside is deleted by the next
    /// helper in the same directory or at the next start.
    pub unforked: Option<Errno>,
}

/// **Remove `path`: rename it aside, then fork the helper that deletes it.**
/// `service` is who the helper reports for.
///
/// # The one call here that can destroy an operator's data
///
/// So every caller is narrow on purpose, and no path reaches here without a
/// type having refused everything that is not one component below a directory
/// this machine owns: an application's directory from a `ServiceName`, a
/// library's from a `Reference` that `Reference::parse` accepted off the card,
/// a repository's from a `RepoName`. The property is not that no path comes
/// from a disk — retention's does — but that each goes through a parser that
/// can refuse rather than through a format string.
///
/// # Errors
///
/// What the rename said. Nothing was removed.
pub fn set_aside(k: &mut dyn Kernel, service: &str, path: &str) -> Result<SetAside, Errno> {
    let aside = rename_aside(k, path, "")?;
    let (parent, _) = path.rsplit_once('/').unwrap_or((".", path));
    let unforked = k.reclaim_in_child(service, parent, &prefix("")).err();
    Ok(SetAside { aside, unforked })
}

/// **Fork a helper for every directory still holding something set aside** —
/// what a power cut left between a rename and its deletion. Called once, at
/// start, before anything is removed.
///
/// Looks in the four places a removal renames inside: the application root,
/// each application's own directory, the library root and the repository
/// root. Each is one listing in this loop; the deleting is the helpers'.
/// A directory that cannot be listed is passed over: its readers report it
/// in their own words, and nothing here is lost by waiting for the next start.
pub fn sweep_leftovers(k: &mut dyn Kernel, journal: &mut Journal, volume: &str) {
    let app_root = service::app_root(volume);
    let mut dirs: Vec<(String, String, String)> = Vec::new();
    let apps = k.list_dir(&app_root).unwrap_or_default();
    if apps.iter().any(|e| service::is_set_aside(e)) {
        dirs.push((SELF_NAME.to_string(), app_root.clone(), prefix("")));
    }
    for entry in &apps {
        let Some(name) = ServiceName::new(entry) else {
            continue;
        };
        let dir = service::app_dir(volume, &name);
        let inside = k.list_dir(&dir).unwrap_or_default();
        for stem in ["", service::TMP_DIR] {
            let wanted = prefix(stem);
            if inside.iter().any(|e| e.starts_with(&wanted)) {
                dirs.push((name.to_string(), dir.clone(), wanted));
            }
        }
    }
    for root in [
        rivet_core::package::install::lib_root(volume),
        rivet_core::git::REPOSITORY_ROOT.to_string(),
    ] {
        if k.list_dir(&root)
            .unwrap_or_default()
            .iter()
            .any(|e| service::is_set_aside(e))
        {
            dirs.push((SELF_NAME.to_string(), root, prefix("")));
        }
    }

    for (svc, dir, wanted) in dirs {
        let corr = journal.next_correlation();
        match k.reclaim_in_child(&svc, &dir, &wanted) {
            Ok(()) => journal.emit(
                k,
                Severity::Info,
                SELF_NAME,
                &corr,
                "something removed before the last shutdown was still on the card awaiting \
                 deletion, so a helper is deleting it now, off the supervisor's loop",
                vec![("path".to_string(), Value::Str(dir))],
            ),
            Err(e) => journal.emit(
                k,
                Severity::Warn,
                SELF_NAME,
                &corr,
                "something removed before the last shutdown is still on the card awaiting \
                 deletion, and the helper to delete it could not start. It is invisible to \
                 everything on the machine and costs only its space; the next start tries again",
                vec![
                    ("path".to_string(), Value::Str(dir)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
    }

    /// **The removal is the rename**: the target's name is free at once, the
    /// aside sits beside it under a name no live entry can have, and a helper
    /// is handed that directory and the removal's prefix.
    #[test]
    fn a_removal_renames_aside_and_hands_the_aside_to_a_helper() {
        let mut k = FakeKernel::new();
        k.dirs_mut()
            .insert("/state/lib".to_string(), vec!["libx".to_string()]);
        k.dirs_mut().insert("/state/lib/libx".to_string(), vec![]);

        let done = set_aside(&mut k, "rivet-sysd", "/state/lib/libx").expect("set aside");

        assert!(done.aside.starts_with("/state/lib/.reclaim-"), "{done:?}");
        assert_eq!(done.unforked, None);
        assert_eq!(
            k.reclaims,
            [(
                "rivet-sysd".to_string(),
                "/state/lib".to_string(),
                ".reclaim-".to_string()
            )]
        );
        assert_eq!(k.list_dir("/state/lib"), Ok(vec![]), "the name is still listed");
    }

    /// A path that is not there is the caller's to read, and forks nothing.
    #[test]
    fn a_removal_of_nothing_says_enoent_and_forks_nothing() {
        let mut k = FakeKernel::new();
        assert_eq!(
            set_aside(&mut k, "a", "/state/lib/absent"),
            Err(Errno::ENOENT)
        );
        assert!(k.reclaims.is_empty());
    }

    /// **A name a power cut left behind is not reused**: the rename that
    /// finds its aside name taken moves to the next one.
    #[test]
    fn a_taken_aside_name_moves_to_the_next() {
        let mut k = FakeKernel::new();
        k.dirs_mut().insert("/state/lib/libx".to_string(), vec![]);
        k.rename_results = Script::sequence(vec![Err(Errno::ENOTEMPTY)], Ok(()));

        let aside = rename_aside(&mut k, "/state/lib/libx", "").expect("renamed");

        assert_eq!(k.renames.len(), 2, "{:?}", k.renames);
        assert_ne!(k.renames[0].1, k.renames[1].1, "the same name was tried twice");
        assert_eq!(aside, k.renames[1].1);
    }

    /// **A start forks one helper per directory still holding an aside**, and
    /// none for a directory that holds none.
    #[test]
    fn a_start_reclaims_what_a_power_cut_left_set_aside() {
        let mut k = FakeKernel::new();
        k.dirs_mut().insert(
            "/state/app".to_string(),
            vec!["a".to_string(), "b".to_string(), ".reclaim-9-1".to_string()],
        );
        k.dirs_mut().insert(
            "/state/app/a".to_string(),
            vec![
                "data".to_string(),
                ".reclaim-4-2".to_string(),
                "tmp.reclaim-4-3".to_string(),
            ],
        );
        k.dirs_mut()
            .insert("/state/app/b".to_string(), vec!["data".to_string()]);
        k.dirs_mut()
            .insert("/state/lib".to_string(), vec!["libx".to_string()]);
        k.dirs_mut().insert(
            rivet_core::git::REPOSITORY_ROOT.to_string(),
            vec![".reclaim-2-1".to_string()],
        );

        sweep_leftovers(&mut k, &mut journal(), service::BUILT_IN_VOLUME);

        let mut asked: Vec<(String, String)> = k
            .reclaims
            .iter()
            .map(|(_, d, p)| (d.clone(), p.clone()))
            .collect();
        asked.sort();
        let mut want = vec![
            ("/state/app".to_string(), ".reclaim-".to_string()),
            ("/state/app/a".to_string(), ".reclaim-".to_string()),
            ("/state/app/a".to_string(), "tmp.reclaim-".to_string()),
            (
                rivet_core::git::REPOSITORY_ROOT.to_string(),
                ".reclaim-".to_string(),
            ),
        ];
        want.sort();
        assert_eq!(asked, want);
    }

    /// An ordinary start, with nothing set aside, forks nothing and says
    /// nothing.
    #[test]
    fn a_clean_start_forks_nothing() {
        let mut k = FakeKernel::new();
        k.dirs_mut()
            .insert("/state/app".to_string(), vec!["a".to_string()]);
        k.dirs_mut()
            .insert("/state/app/a".to_string(), vec!["data".to_string(), "tmp".to_string()]);
        sweep_leftovers(&mut k, &mut journal(), service::BUILT_IN_VOLUME);
        assert!(k.reclaims.is_empty(), "{:?}", k.reclaims);
        assert!(k.records.is_empty(), "{:?}", k.records);
    }
}
