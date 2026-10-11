//! The crash-safe write, in one place, because two files on `/state` need it.
//!
//! It was inside [`crate::store::ConfigStore::save`] until 2026-08-23, when
//! [`crate::clock`] arrived needing the identical sequence for a different file.
//! Copying it would have been the worse outcome by some distance: the sequence
//! is four calls whose *order* is the entire guarantee, and a second copy is a
//! second place for a step to be dropped in a refactor with nothing failing
//! until somebody pulls the power out.
//!
//! # The sequence, and what each step buys
//!
//! ```text
//! write(target.new)            the new bytes, nowhere anything reads
//! sync_file(target.new)        they reach the card
//! rename(target.new -> target) THE COMMIT — atomic, by rename(2)
//! sync_dir(dir)                the directory entry reaches the card
//! ```
//!
//! The long form of what this guarantees, what it does not, and why an SD card
//! can defeat all of it is on [`crate::store::ConfigStore::save`] and is not
//! repeated here — `ADR-011`, one place per fact.
//!
//! # What is deliberately **not** here
//!
//! **`create_dir_all`.** It was the first call of the old sequence and it is not
//! part of the crash-safety argument; it is a fact about one caller.
//! `ConfigStore` creates `/state/config` because `ADR-017` §3.1.7 says the
//! directory is created by whatever writes into it. [`crate::clock`] writes
//! directly into `/state`, whose mount point **ships in the image and cannot be
//! created at runtime** (`rivet_core::state`'s module documentation — the root
//! it would be created on is read-only). So the caller decides, and the store's
//! recorded call order is unchanged.

use crate::port::Filesystem;
use crate::store::{Operation, StoreError};

/// Write `bytes` to `target` so that a power cut leaves either the old contents
/// or the new ones and never a mixture.
///
/// `temp` must be a sibling of `target` — `rename(2)` is atomic only within one
/// filesystem — and `dir` must be the directory holding both. Neither is checked
/// here: the two callers derive all three paths from one root, and a check would
/// be re-deriving what they already know.
///
/// A failure before the rename removes the temp on a best-effort basis, and the
/// *original* failure is what is returned even when the removal also fails: on a
/// full `/state` the partial temp is the thing occupying the space, so leaving
/// it there is what makes the next attempt fail too, and losing the failure that
/// matters behind a second one would be worse than either.
pub fn commit(
    fs: &mut dyn Filesystem,
    dir: &str,
    target: &str,
    temp: &str,
    bytes: &[u8],
) -> Result<(), StoreError> {
    if let Err(errno) = fs.write(temp, bytes) {
        let _ = fs.remove_file(temp);
        return Err(StoreError::Io {
            path: temp.to_owned(),
            op: Operation::Write,
            errno,
        });
    }

    // Separate from the write, and this is where `ENOSPC` actually arrives on
    // `ext4`: delayed allocation defers the block allocation past `write(2)`, so
    // a full filesystem is reported here.
    if let Err(errno) = fs.sync_file(temp) {
        let _ = fs.remove_file(temp);
        return Err(StoreError::Io {
            path: temp.to_owned(),
            op: Operation::SyncFile,
            errno,
        });
    }

    if let Err(errno) = fs.rename(temp, target) {
        let _ = fs.remove_file(temp);
        return Err(StoreError::Io {
            path: target.to_owned(),
            op: Operation::Rename,
            errno,
        });
    }

    // Past the commit. A failure from here on is a durability failure and not a
    // save failure, and the two must not be reported as one thing — the file
    // *is* in place.
    if let Err(errno) = fs.sync_dir(dir) {
        return Err(StoreError::NotDurable {
            path: target.to_owned(),
            errno,
        });
    }

    Ok(())
}

/// Where a save in progress is written before it is committed.
///
/// Derived from the target rather than being a second constant, so the two
/// cannot drift apart, and a **fixed** name rather than a random one so that a
/// temp left behind by a power cut is overwritten by the next save instead of
/// accumulating on the one writable filesystem (`ADR-018` §7).
///
/// **`pub` since 2026-09-04**, when `cmd/rivet-sysd`'s `durable` module started
/// committing a repository's identifier through
/// [`commit`]. The alternative was for that crate to append
/// [`crate::store::IN_PROGRESS_SUFFIX`] itself, which is the same fact spelled
/// twice (`ADR-011`) — and the fact that matters is not the suffix but that the
/// temp is a **sibling**, which a second spelling is free to get wrong.
pub fn in_progress(target: &str) -> String {
    format!("{target}{}", crate::store::IN_PROGRESS_SUFFIX)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::Errno;
    use crate::testfs::{Call, FakeFs, Script};

    /// The crash-safety argument, as an assertion on the call order. This is the
    /// one place it is checked now that two stores share the sequence.
    #[test]
    fn the_four_calls_happen_in_the_order_the_guarantee_depends_on() {
        let mut fs = FakeFs::mounted();
        assert_eq!(commit(&mut fs, "/state", "/state/a", "/state/a.new", b"xy"), Ok(()));
        assert_eq!(
            fs.calls,
            vec![
                Call::Write("/state/a.new".to_owned(), 2),
                Call::SyncFile("/state/a.new".to_owned()),
                Call::Rename("/state/a.new".to_owned(), "/state/a".to_owned()),
                Call::SyncDir("/state".to_owned()),
            ]
        );
        assert_eq!(fs.contents("/state/a"), Some(b"xy".as_slice()));
    }

    /// **Nothing is created.** The caller decides whether a directory needs
    /// making, because `/state` itself cannot be created at runtime.
    #[test]
    fn commit_creates_no_directory_of_its_own() {
        let mut fs = FakeFs::mounted();
        let _ = commit(&mut fs, "/state", "/state/a", "/state/a.new", b"x");
        assert!(
            !fs.calls.iter().any(|c| matches!(c, Call::CreateDirAll(_))),
            "{:?}",
            fs.calls
        );
    }

    #[test]
    fn a_failed_write_clears_the_temp_and_commits_nothing() {
        let mut fs = FakeFs::mounted();
        fs.write_results = Some(Script::always(Err(Errno::ENOSPC)));
        let err = commit(&mut fs, "/state", "/state/a", "/state/a.new", b"x")
            .expect_err("no space");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::Write, errno: Errno::ENOSPC, .. }),
            "{err:?}"
        );
        assert!(!fs.exists("/state/a"));
        assert!(fs.calls.contains(&Call::RemoveFile("/state/a.new".to_owned())));
    }

    /// The `ext4` case: `ENOSPC` arrives at `fsync`, with the write having
    /// already succeeded.
    #[test]
    fn a_failure_at_fsync_is_a_failure_and_not_a_success() {
        let mut fs = FakeFs::mounted();
        fs.sync_file_results = Some(Script::always(Err(Errno::ENOSPC)));
        let err = commit(&mut fs, "/state", "/state/a", "/state/a.new", b"x")
            .expect_err("no space, discovered late");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::SyncFile, .. }),
            "{err:?}"
        );
        assert!(!fs.exists("/state/a"));
        assert!(!fs.exists("/state/a.new"));
    }

    #[test]
    fn a_refused_commit_leaves_what_was_already_there() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/a", b"old");
        fs.rename_results = Some(Script::always(Err(Errno::EIO)));
        let err =
            commit(&mut fs, "/state", "/state/a", "/state/a.new", b"new").expect_err("refused");
        assert!(matches!(&err, StoreError::Io { op: Operation::Rename, .. }), "{err:?}");
        assert_eq!(fs.contents("/state/a"), Some(b"old".as_slice()));
    }

    /// Past the commit the file *is* in place, so a directory flush that fails
    /// must not be reported as a failed save.
    #[test]
    fn a_directory_flush_that_fails_still_leaves_the_new_bytes_in_place() {
        let mut fs = FakeFs::mounted();
        fs.sync_dir_results = Some(Script::always(Err(Errno::EIO)));
        let err = commit(&mut fs, "/state", "/state/a", "/state/a.new", b"new")
            .expect_err("the flush failed");
        assert!(matches!(&err, StoreError::NotDurable { .. }), "{err:?}");
        assert_eq!(fs.contents("/state/a"), Some(b"new".as_slice()));
    }

    #[test]
    fn the_in_progress_path_is_the_target_plus_the_one_suffix() {
        assert_eq!(in_progress("/state/clock"), "/state/clock.new");
        assert_eq!(
            in_progress("/state/config/machine.toml"),
            "/state/config/machine.toml.new"
        );
    }
}
