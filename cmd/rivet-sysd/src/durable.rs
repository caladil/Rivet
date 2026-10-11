//! The crash-safe write, for a file in this crate that is written by path
//! rather than through `rivet-store`.
//!
//! **Added 2026-09-04**, for the audit of 2026-09-03's durability finding: a
//! repository's identifier (`crate::repos`) was written by truncate-in-place —
//! `open(O_TRUNC)`, `write`, done. That is two failures and neither is exotic
//! on the medium `ADR-018` §2 accepts:
//!
//! * **A power cut between the truncate and the write leaves an empty file.**
//!   For a repository's identifier that is a repository this machine no longer
//!   recognises — reported as undeclared, with a second empty one created
//!   beside it, which `crate::repos::create` already says in as many words.
//! * **Nothing was flushed**, so a file that *looks* written can still be
//!   absent, or present and full of zeroes, after a power cut — the `ext4`
//!   failure `rivet_store::store::ConfigStore::save` documents at length.
//!
//! # The sequence is `rivet-store`'s, reached through this crate's port
//!
//! `rivet_store::atomic` carries the four calls and the argument for their
//! order, and its own module documentation is explicit that a **second copy is
//! a second place for a step to be dropped in a refactor with nothing failing
//! until somebody pulls the power out.** So there is no second copy: this
//! module is an adapter, not an implementation. What it adds is the one thing
//! the `Filesystem` port has no vocabulary for — a **mode** — which both
//! callers need and neither can leave to the umask.
//!
//! `crate::clock` already reaches the same sequence the same way, and the
//! difference between its adapter and this one is only that its mode is a
//! constant.
//!
//! # What this does not do
//!
//! * **It does not create the directory.** Both callers already do, and
//!   `rivet_store::atomic`'s own documentation says why that is not part of the
//!   crash-safety argument.
//! * **It is not `write_file_nofollow`.** A symlink at the target is *replaced*
//!   by the rename rather than followed, which is a real difference — but the
//!   temp is still opened by path, so this is not the guarded write
//!   `crate::runtime` needs for a path below a directory an application owns.
//!   Neither caller here writes below such a directory: both write into
//!   root-owned trees.
//! * **It is not atomic across two files.** One name is replaced in one step.

use crate::kernel::{Errno, Kernel};

/// How far a committed write got.
///
/// Two outcomes rather than one, because the second is **not a failed write**
/// and must not be reported as one: past the rename the new bytes are in place
/// under the target name and every reader sees them. What was not established
/// is that the *name* survives a power cut, which is a durability fact and a
/// different sentence. `rivet_store::atomic` draws the same line and returns
/// `StoreError::NotDurable` for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Durability {
    /// The bytes and the directory entry both reached the medium.
    Flushed,
    /// The bytes are in place under the target name; the directory entry was
    /// not flushed, and the platform said this.
    NameNotFlushed(Errno),
}

/// Replace `path` with `bytes` at `mode`, so that a power cut leaves either the
/// old contents or the new ones and never a mixture.
///
/// The temp is a sibling — `rename(2)` is atomic only within one filesystem —
/// and it is a **fixed** name rather than a random one, so that one left behind
/// by a power cut is overwritten by the next attempt instead of accumulating on
/// the one writable filesystem (`ADR-018` §7). Both facts are
/// `rivet_store::atomic::in_progress`'s, spelled once there.
///
/// `Err` means **nothing was committed**: the old contents, or the old absence,
/// are what a reader still sees. A failure before the rename removes the temp
/// on a best-effort basis and returns the original errno, which is again
/// `rivet_store::atomic`'s rule and its reasoning.
pub fn commit(
    k: &mut dyn Kernel,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<Durability, Errno> {
    let temp = rivet_store::atomic::in_progress(path);
    let dir = parent_of(path).to_owned();
    let mut fs = KernelFs { k, mode };

    match rivet_store::atomic::commit(&mut fs, &dir, path, &temp, bytes) {
        Ok(()) => Ok(Durability::Flushed),
        // The file IS in place. See [`Durability`].
        Err(rivet_store::StoreError::NotDurable { errno, .. }) => {
            Ok(Durability::NameNotFlushed(from_store_errno(errno)))
        }
        Err(rivet_store::StoreError::Io { errno, .. }) => Err(from_store_errno(errno)),
        // `commit` returns no other variant. `EIO` rather than a panic: this is
        // a crate where `clippy::panic` is `deny`, and a supervisor that aborts
        // on an error it did not expect is worse than one that reports it.
        Err(_) => Err(Errno::EIO),
    }
}

/// The directory holding `path`.
///
/// An absolute path is the contract — the caller derives its own from a
/// constant root. A path with no `/` at all answers `.`, which is the honest
/// answer for a relative name and is never reached from this crate.
fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        // `/state` is a child of `/`, and `""` is not a directory anything can
        // open.
        Some(0) => "/",
        Some(cut) => &path[..cut],
        None => ".",
    }
}

/// `rivet-store`'s `Filesystem` port over this crate's [`Kernel`], carrying the
/// mode the port has no parameter for.
///
/// The writing twin of `crate::clock`'s `StateFs`, and the only difference is
/// that the mode is a field rather than a constant.
struct KernelFs<'k> {
    k: &'k mut dyn Kernel,
    mode: u32,
}

/// The two error vocabularies are separate types on purpose; on the target they
/// are the same numbers, so each conversion is the number and nothing else.
const fn to_store_errno(e: Errno) -> rivet_store::Errno {
    rivet_store::Errno(e.0)
}

const fn from_store_errno(e: rivet_store::Errno) -> Errno {
    Errno(e.0)
}

impl rivet_store::Filesystem for KernelFs<'_> {
    fn create_dir_all(&mut self, path: &str) -> Result<(), rivet_store::Errno> {
        // Not reached: `rivet_store::atomic::commit` creates nothing, and both
        // callers make their own directories. Implemented rather than refused
        // because a port implementation that lies about one method is a trap
        // for the next caller — `crate::clock` says the same thing about the
        // same method.
        self.k.mkdir_p(path).map_err(to_store_errno)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, rivet_store::Errno> {
        self.k
            .read_file(path)
            .map(String::into_bytes)
            .map_err(to_store_errno)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), rivet_store::Errno> {
        self.k
            .write_file(path, bytes, self.mode)
            .map_err(to_store_errno)
    }

    fn sync_file(&mut self, path: &str) -> Result<(), rivet_store::Errno> {
        self.k.sync_file(path).map_err(to_store_errno)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), rivet_store::Errno> {
        self.k.rename(from, to).map_err(to_store_errno)
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), rivet_store::Errno> {
        self.k.sync_dir(path).map_err(to_store_errno)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), rivet_store::Errno> {
        self.k.remove_file(path).map_err(to_store_errno)
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    const TARGET: &str = "/state/git/photos.git/rivet-repo-id";
    const TEMP: &str = "/state/git/photos.git/rivet-repo-id.new";
    const DIR: &str = "/state/git/photos.git";

    /// The crash-safety argument, as an assertion on what reached the kernel.
    /// **Nothing is written at the target path**, which is the whole of the
    /// finding: a truncate-in-place is a window in which the file is empty.
    #[test]
    fn the_target_is_never_opened_for_writing_and_the_four_calls_are_in_order() {
        let mut k = FakeKernel::new();
        assert_eq!(commit(&mut k, TARGET, b"key\n", 0o600), Ok(Durability::Flushed));

        assert_eq!(k.writes.len(), 1);
        let write = k.writes.first().expect("just asserted");
        assert_eq!(write.path, TEMP, "the target was truncated in place");
        assert_eq!(write.contents, b"key\n");
        assert_eq!(write.mode, 0o600);

        assert_eq!(k.syncs, vec![TEMP.to_string()], "the bytes were not flushed");
        assert_eq!(k.renames, vec![(TEMP.to_string(), TARGET.to_string())]);
        assert_eq!(k.dir_syncs, vec![DIR.to_string()], "the name was not flushed");
        assert!(
            !k.truncations.iter().any(|p| p == TARGET),
            "the target was truncated: {:?}",
            k.truncations
        );
    }

    /// A failure before the rename commits nothing and returns the platform's
    /// own verdict rather than a paraphrase (`AGENTS.md` §5).
    #[test]
    fn a_write_that_fails_commits_nothing_and_carries_the_errno() {
        for errno in [Errno::ENOSPC, Errno::EROFS, Errno::EACCES] {
            let mut k = FakeKernel::new();
            k.write_results = Script::always(Err(errno));
            assert_eq!(commit(&mut k, TARGET, b"key\n", 0o600), Err(errno));
            assert!(k.renames.is_empty(), "{errno} still committed");
        }
    }

    /// `ENOSPC` on `ext4` arrives at the flush, not at the write — delayed
    /// allocation defers the block allocation past `write(2)`. A flush that
    /// fails must not become a successful save.
    #[test]
    fn a_flush_that_fails_is_not_a_save() {
        let mut k = FakeKernel::new();
        k.sync_file_results = Script::always(Err(Errno::ENOSPC));
        assert_eq!(commit(&mut k, TARGET, b"key\n", 0o600), Err(Errno::ENOSPC));
        assert!(k.renames.is_empty());
    }

    /// Past the rename the file IS in place, so a directory flush that fails is
    /// a durability fact and not a failed write. Reporting it as a failure
    /// would make a caller refuse a file it had just written.
    #[test]
    fn a_directory_flush_that_fails_still_leaves_the_bytes_in_place() {
        let mut k = FakeKernel::new();
        k.sync_dir_results = Script::always(Err(Errno::EIO));
        assert_eq!(
            commit(&mut k, TARGET, b"key\n", 0o600),
            Ok(Durability::NameNotFlushed(Errno::EIO))
        );
        assert_eq!(k.renames, vec![(TEMP.to_string(), TARGET.to_string())]);
    }

    #[test]
    fn the_temp_is_a_sibling_of_the_target_because_rename_crosses_no_filesystem() {
        let mut k = FakeKernel::new();
        let _ = commit(&mut k, TARGET, b"x", 0o600);
        let write = k.writes.first().expect("one write");
        assert_eq!(parent_of(&write.path), parent_of(TARGET));
    }

    #[test]
    fn the_parent_of_a_file_in_the_root_is_the_root() {
        assert_eq!(parent_of("/state/clock"), "/state");
        assert_eq!(parent_of("/state"), "/");
        assert_eq!(parent_of("relative"), ".");
    }
}
