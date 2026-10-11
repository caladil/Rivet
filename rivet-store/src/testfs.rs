//! A [`Filesystem`] that answers however a test needs it to.
//!
//! Compiled only under `cfg(test)`. It exists so this crate's decisions can be
//! driven through states a developer's machine does not reach and a board
//! reaches only by being damaged: a `/state` that never mounted, a card that is
//! full, a card returning `EIO`, a document half-written by a power cut.
//!
//! **It is a real little filesystem, not a stack of canned answers**, and that
//! is deliberate. `store::save`'s whole subject is the *ordering* of write,
//! sync, rename and sync — a fake that only returned scripted results could not
//! answer the question the tests actually ask, which is *what is on the card
//! after step N failed*. So [`FakeFs`] holds contents and directories, and the
//! [`Script`]s sit on top to make any single call fail at any point in the
//! sequence.
//!
//! `Script` is a near-copy of the ones in `cmd/rivet-initd/src/testkernel.rs`
//! and `rivet-adapter-net/src/testnetlink.rs`, deliberately and for the reason
//! the latter records: sharing it would mean a crate-to-crate test dependency
//! `ADR-014` §3 has no room for, and twenty lines duplicated is cheaper than
//! either alternative.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::port::{Errno, Filesystem};

/// A queue of answers ending in one that repeats for ever.
///
/// `Script::sequence(vec![a, b], z)` answers `a`, then `b`, then `z` on every
/// call after. `Script::always(z)` answers `z` from the start.
#[derive(Debug, Clone)]
pub struct Script<T: Clone> {
    queue: VecDeque<T>,
    forever: T,
}

impl<T: Clone> Script<T> {
    pub fn always(forever: T) -> Self {
        Self {
            queue: VecDeque::new(),
            forever,
        }
    }

    pub fn sequence(steps: Vec<T>, forever: T) -> Self {
        Self {
            queue: steps.into(),
            forever,
        }
    }

    pub fn next(&mut self) -> T {
        self.queue.pop_front().unwrap_or_else(|| self.forever.clone())
    }
}

/// One call, recorded in order, so a test can assert on the *sequence* rather
/// than on the end state alone. The ordering is the crash-safety argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    CreateDirAll(String),
    Read(String),
    Write(String, usize),
    SyncFile(String),
    Rename(String, String),
    SyncDir(String),
    RemoveFile(String),
}

/// A filesystem in a `BTreeMap`.
///
/// The default is an **empty** one with no directories at all, which is the
/// state of a machine whose `/state` did not mount — not a happy path. A test
/// that wants a working `/state` says so, which keeps the interesting case from
/// being the one nobody wrote down.
#[derive(Debug, Default)]
pub struct FakeFs {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    /// Every call, in order.
    pub calls: Vec<Call>,
    pub create_dir_results: Option<Script<Result<(), Errno>>>,
    pub read_results: Option<Script<Result<Vec<u8>, Errno>>>,
    pub write_results: Option<Script<Result<(), Errno>>>,
    pub sync_file_results: Option<Script<Result<(), Errno>>>,
    pub rename_results: Option<Script<Result<(), Errno>>>,
    pub sync_dir_results: Option<Script<Result<(), Errno>>>,
    pub remove_file_results: Option<Script<Result<(), Errno>>>,
}

impl FakeFs {
    /// A machine whose `/state` did not mount: nothing exists and nothing can
    /// be created, because the mount point is on the read-only root.
    pub fn unmounted() -> Self {
        Self {
            create_dir_results: Some(Script::always(Err(Errno::EROFS))),
            write_results: Some(Script::always(Err(Errno::EROFS))),
            rename_results: Some(Script::always(Err(Errno::EROFS))),
            ..Self::default()
        }
    }

    /// A machine with a working, empty `/state`.
    pub fn mounted() -> Self {
        let mut fs = Self::default();
        fs.dirs.insert("/state".to_owned());
        fs
    }

    /// Put a file there, creating its parent directories, without recording a
    /// call — a test's *arrangement* is not part of the sequence it asserts on.
    pub fn given(&mut self, path: &str, bytes: &[u8]) -> &mut Self {
        if let Some(parent) = path.rfind('/').map(|i| &path[..i]) {
            self.make_dirs(parent);
        }
        self.files.insert(path.to_owned(), bytes.to_vec());
        self
    }

    /// What is on the card now.
    pub fn contents(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Every path that holds a file, for a test asserting that nothing was left
    /// behind.
    pub fn paths(&self) -> Vec<&str> {
        self.files.keys().map(String::as_str).collect()
    }

    fn make_dirs(&mut self, path: &str) {
        let mut so_far = String::new();
        for part in path.split('/').filter(|p| !p.is_empty()) {
            so_far.push('/');
            so_far.push_str(part);
            self.dirs.insert(so_far.clone());
        }
    }

    fn parent_exists(&self, path: &str) -> bool {
        match path.rfind('/') {
            // A path directly under the root; the root always exists.
            Some(0) | None => true,
            Some(i) => self.dirs.contains(&path[..i]),
        }
    }

    /// A scripted answer if one is queued, or `None` to fall through to the
    /// fake's own behaviour.
    fn scripted<T: Clone>(script: &mut Option<Script<Result<T, Errno>>>) -> Option<Result<T, Errno>> {
        script.as_mut().map(Script::next)
    }
}

impl Filesystem for FakeFs {
    fn create_dir_all(&mut self, path: &str) -> Result<(), Errno> {
        self.calls.push(Call::CreateDirAll(path.to_owned()));
        if let Some(answer) = Self::scripted(&mut self.create_dir_results) {
            return answer;
        }
        if self.files.contains_key(path) {
            return Err(Errno::EEXIST);
        }
        self.make_dirs(path);
        Ok(())
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        self.calls.push(Call::Read(path.to_owned()));
        if let Some(answer) = Self::scripted(&mut self.read_results) {
            return answer;
        }
        self.files.get(path).cloned().ok_or(Errno::ENOENT)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), Errno> {
        self.calls.push(Call::Write(path.to_owned(), bytes.len()));
        if let Some(answer) = Self::scripted(&mut self.write_results) {
            return answer;
        }
        if !self.parent_exists(path) {
            return Err(Errno::ENOENT);
        }
        self.files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn sync_file(&mut self, path: &str) -> Result<(), Errno> {
        self.calls.push(Call::SyncFile(path.to_owned()));
        if let Some(answer) = Self::scripted(&mut self.sync_file_results) {
            return answer;
        }
        if self.files.contains_key(path) { Ok(()) } else { Err(Errno::ENOENT) }
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        self.calls.push(Call::Rename(from.to_owned(), to.to_owned()));
        if let Some(answer) = Self::scripted(&mut self.rename_results) {
            return answer;
        }
        let Some(bytes) = self.files.remove(from) else {
            return Err(Errno::ENOENT);
        };
        if !self.parent_exists(to) {
            self.files.insert(from.to_owned(), bytes);
            return Err(Errno::ENOENT);
        }
        self.files.insert(to.to_owned(), bytes);
        Ok(())
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), Errno> {
        self.calls.push(Call::SyncDir(path.to_owned()));
        if let Some(answer) = Self::scripted(&mut self.sync_dir_results) {
            return answer;
        }
        if self.dirs.contains(path) { Ok(()) } else { Err(Errno::ENOENT) }
    }

    fn remove_file(&mut self, path: &str) -> Result<(), Errno> {
        self.calls.push(Call::RemoveFile(path.to_owned()));
        if let Some(answer) = Self::scripted(&mut self.remove_file_results) {
            return answer;
        }
        if self.files.remove(path).is_some() { Ok(()) } else { Err(Errno::ENOENT) }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The fake has to behave like a filesystem or the tests above it prove
    /// nothing about one. This is the guard on the fake itself.
    #[test]
    fn the_fake_behaves_like_a_filesystem_for_the_operations_it_offers() {
        let mut fs = FakeFs::mounted();
        assert_eq!(fs.read("/state/a"), Err(Errno::ENOENT));
        assert_eq!(fs.write("/state/a", b"x"), Ok(()));
        assert_eq!(fs.read("/state/a"), Ok(b"x".to_vec()));
        assert_eq!(fs.rename("/state/a", "/state/b"), Ok(()));
        assert_eq!(fs.read("/state/a"), Err(Errno::ENOENT));
        assert_eq!(fs.read("/state/b"), Ok(b"x".to_vec()));
        assert_eq!(fs.remove_file("/state/b"), Ok(()));
        assert!(fs.paths().is_empty());
    }

    #[test]
    fn a_write_into_a_directory_that_does_not_exist_is_enoent() {
        let mut fs = FakeFs::mounted();
        assert_eq!(fs.write("/state/config/machine.toml", b"x"), Err(Errno::ENOENT));
        assert_eq!(fs.create_dir_all("/state/config"), Ok(()));
        assert_eq!(fs.write("/state/config/machine.toml", b"x"), Ok(()));
    }

    #[test]
    fn a_script_overrides_the_fakes_own_behaviour_for_exactly_as_long_as_it_says() {
        let mut fs = FakeFs::mounted();
        fs.write_results = Some(Script::sequence(vec![Err(Errno::ENOSPC)], Ok(())));
        assert_eq!(fs.write("/state/a", b"x"), Err(Errno::ENOSPC));
        assert!(!fs.exists("/state/a"), "a refused write must leave nothing");
        // ...and the second call falls through to the scripted `forever`, which
        // is `Ok` — but the fake's own bookkeeping is then bypassed, which is
        // exactly why the tests that care about contents do not script.
        assert_eq!(fs.write("/state/a", b"x"), Ok(()));
    }

    #[test]
    fn an_unmounted_state_refuses_every_write_the_way_a_read_only_root_does() {
        let mut fs = FakeFs::unmounted();
        assert_eq!(fs.create_dir_all("/state/config"), Err(Errno::EROFS));
        assert_eq!(fs.read("/state/config/machine.toml"), Err(Errno::ENOENT));
    }

    #[test]
    fn every_call_is_recorded_in_order_because_the_order_is_the_subject() {
        let mut fs = FakeFs::mounted();
        let _ = fs.create_dir_all("/state/config");
        let _ = fs.write("/state/config/a", b"xy");
        let _ = fs.sync_file("/state/config/a");
        assert_eq!(
            fs.calls,
            vec![
                Call::CreateDirAll("/state/config".to_owned()),
                Call::Write("/state/config/a".to_owned(), 2),
                Call::SyncFile("/state/config/a".to_owned()),
            ]
        );
    }
}
