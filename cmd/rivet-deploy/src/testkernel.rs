//! A [`Kernel`] that answers however a test needs it to.
//!
//! Compiled only under `cfg(test)`. The same shape as `cmd/rivet-initd`'s,
//! `cmd/rivet-timed`'s and `rivet-adapter-image`'s `testio`: every answer is
//! a [`Script`], so a *sequence* of answers can be expressed, and the defaults
//! are the boring happy path so a test only says what is unusual about the world
//! it is describing.
//!
//! # It models a card, not a filesystem
//!
//! Files are bytes in a `BTreeMap` and block devices are `Vec<u8>` of a declared
//! length, because the states this binary has to be right about are states no
//! developer's machine reaches: a card that has gone read-only halfway through a
//! 640 MiB write, a partition smaller than the member offered to it, a payload
//! that is shorter than its own header says. `ADR-026` §4 puts `/dev/mmcblk0` in
//! the column QEMU cannot reach at all, so without this they would be exercised
//! by nothing until a board.
//!
//! **What it cannot prove** is anything about a real SD card — the controller,
//! erase blocks, or a card that accepts a write and stores something else, which
//! is `ADR-029` §9.2 step 9's whole reason for existing. `AGENTS.md` §6.

use std::collections::{BTreeMap, VecDeque};

use rivet_adapter_image::Mounted;
use rivet_core::journal::Record;

use crate::kernel::{Errno, Kernel};

/// A queue of answers ending in one that repeats for ever.
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

    #[allow(dead_code)]
    pub fn sequence(steps: Vec<T>, forever: T) -> Self {
        Self {
            queue: steps.into(),
            forever,
        }
    }

    pub fn next(&mut self) -> T {
        self.queue
            .pop_front()
            .unwrap_or_else(|| self.forever.clone())
    }
}

/// One recorded [`Kernel::mount`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountCall {
    pub device: String,
    pub at: String,
    pub fstype: String,
    pub writable: bool,
}

/// One recorded [`Kernel::write_device`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceWrite {
    pub source: String,
    pub offset: u64,
    pub len: u64,
    pub device: String,
}

/// The fake.
pub struct FakeKernel {
    pub mono_ms: u64,
    pub wall_ms: Option<u64>,
    pub records: Vec<Record>,

    /// **Every filesystem operation, in the order it happened.**
    ///
    /// The per-operation vectors below each record their own kind and cannot
    /// say what came before what. That mattered on 2026-08-24: a test called
    /// `nothing_is_written_after_the_link_is_swapped` passed with the install's
    /// steps run in REVERSE, because it could only see that one rename happened
    /// and where it went. Ordering is the correctness argument of an atomic
    /// install, so it has to be observable.
    pub ops: Vec<String>,
    /// Directories this kernel was asked to make, with the mode asked for.
    pub mkdirs: Vec<(String, u32)>,
    /// Every path `unlink_symlink` was called with.
    pub unlinks: Vec<String>,
    /// `(path, mode)` for every `chmod`.
    pub chmods: Vec<(String, u32)>,
    /// `path -> target` for every symlink that currently exists.
    pub links: BTreeMap<String, String>,

    pub mount_calls: Vec<MountCall>,
    pub umount_calls: Vec<String>,
    pub device_writes: Vec<DeviceWrite>,
    pub synced_writes: Vec<(String, Vec<u8>)>,
    pub renames: Vec<(String, String)>,
    pub syncs: usize,

    pub mount_results: Script<Result<Mounted, Errno>>,
    /// Every `lock_within` (with its bound) and `unlock`, in order.
    pub lock_calls: Vec<String>,
    /// Every mount, unmount, lock and unlock, in the one order they happened,
    /// as `mount <at>`, `umount <at>`, `lock <path>`, `unlock <path>` — so a
    /// test can ask whether a lock was held across a mount.
    pub sequence: Vec<String>,
    /// A mount point at which every mount answers [`Mounted::Adopted`], as one
    /// already holding a filesystem would, whatever `mount_results` says.
    pub adopt_at: Option<String>,
    pub lock_results: Script<Result<(), Errno>>,
    pub write_results: Script<Result<(), Errno>>,
    pub rename_results: Script<Result<(), Errno>>,
    pub umount_results: Script<Result<(), Errno>>,
    pub device_write_results: Script<Result<(), Errno>>,

    files: BTreeMap<String, Script<Result<String, Errno>>>,
    bytes: BTreeMap<String, Script<Result<Vec<u8>, Errno>>>,
    /// The content of a whole file or block device, for
    /// [`Kernel::read_range`] and [`Kernel::size_of`].
    blobs: BTreeMap<String, Vec<u8>>,
    /// A declared size that is **not** the blob's length, for a block device
    /// whose content the test does not care about. `ADR-029` F8's fit check.
    sizes: BTreeMap<String, u64>,
    /// Devices that **accept a write and store something else.**
    ///
    /// This is the one hazard `ADR-029` §9.2 step 9 exists for, in its own
    /// words: *"SD cards do accept writes and store something else, and this is
    /// the only defence against that."* A loop device cannot do it and neither
    /// can a file, so the read-back's whole reason for existing would otherwise
    /// be exercised by nothing. A device named here flips one byte of whatever
    /// is written to it, silently, exactly as a dying card does.
    lying: std::collections::BTreeSet<String>,
    /// Paths that were on the machine **before** this run — a directory an
    /// earlier install left behind. `ADR-015` §7.2's whole subject, and it has
    /// to be expressible without performing an install first, so that a test can
    /// state the precondition it is about.
    pre_existing: std::collections::BTreeSet<String>,
}

impl FakeKernel {
    pub fn new() -> Self {
        Self {
            mono_ms: 0,
            wall_ms: Some(1_755_820_800_000),
            records: Vec::new(),
            ops: Vec::new(),
            mkdirs: Vec::new(),
            unlinks: Vec::new(),
            chmods: Vec::new(),
            links: BTreeMap::new(),
            mount_calls: Vec::new(),
            umount_calls: Vec::new(),
            device_writes: Vec::new(),
            synced_writes: Vec::new(),
            renames: Vec::new(),
            syncs: 0,
            mount_results: Script::always(Ok(Mounted::Made)),
            lock_calls: Vec::new(),
            sequence: Vec::new(),
            adopt_at: None,
            lock_results: Script::always(Ok(())),
            write_results: Script::always(Ok(())),
            rename_results: Script::always(Ok(())),
            umount_results: Script::always(Ok(())),
            device_write_results: Script::always(Ok(())),
            files: BTreeMap::new(),
            bytes: BTreeMap::new(),
            blobs: BTreeMap::new(),
            sizes: BTreeMap::new(),
            lying: std::collections::BTreeSet::new(),
            pre_existing: std::collections::BTreeSet::new(),
        }
    }

    /// Put a directory on the machine that nothing in this run created —
    /// `ADR-015` §7.2's precondition.
    pub fn already_there(&mut self, path: &str) {
        self.pre_existing.insert(path.to_string());
    }

    /// Make `path` a device that accepts a write and stores something else —
    /// see [`Self::lying`].
    pub fn make_it_lie(&mut self, path: &str) {
        self.lying.insert(path.to_string());
    }

    pub fn files_mut(&mut self) -> &mut BTreeMap<String, Script<Result<String, Errno>>> {
        &mut self.files
    }

    pub fn bytes_mut(&mut self) -> &mut BTreeMap<String, Script<Result<Vec<u8>, Errno>>> {
        &mut self.bytes
    }

    /// Put a whole file or block device on the machine. Its length becomes what
    /// [`Kernel::size_of`] answers, unless [`Self::declare_size`] says otherwise.
    pub fn put(&mut self, path: &str, content: Vec<u8>) {
        self.blobs.insert(path.to_string(), content);
    }

    /// Declare a size without content — a 640 MiB partition a test does not
    /// want to allocate.
    pub fn declare_size(&mut self, path: &str, bytes: u64) {
        self.sizes.insert(path.to_string(), bytes);
    }

    /// What a device holds after the writes this fake recorded.
    pub fn device_content(&self, path: &str) -> Option<&[u8]> {
        self.blobs.get(path).map(Vec::as_slice)
    }

    /// Every path this fake holds bytes for, in order. What an install test
    /// asserts over when the question is *what is on the filesystem now*.
    pub fn paths(&self) -> Vec<&str> {
        self.blobs.keys().map(String::as_str).collect()
    }

    /// Every record's message, in order — what a test asserts a sequence over.
    pub fn messages(&self) -> Vec<String> {
        self.records.iter().map(|r| r.msg.clone()).collect()
    }

    /// The bytes at `path` reached through one of this fake's symbolic links,
    /// resolved as the kernel resolves a relative target: against the link's
    /// own directory.
    fn through_a_link(&self, path: &str) -> Option<Vec<u8>> {
        self.links.iter().find_map(|(link, target)| {
            let rest = path.strip_prefix(link.as_str())?.strip_prefix('/')?;
            let (dir, _) = link.rsplit_once('/')?;
            self.blobs.get(&format!("{dir}/{target}/{rest}")).cloned()
        })
    }
}

impl Default for FakeKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl Kernel for FakeKernel {
    fn now_ms(&mut self) -> u64 {
        self.mono_ms
    }

    fn wall_ms(&mut self) -> Option<u64> {
        self.wall_ms
    }

    fn emit(&mut self, record: &Record) {
        self.records.push(record.clone());
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        match self.files.get_mut(path) {
            Some(script) => script.next(),
            None => match self.blobs.get(path) {
                Some(bytes) => String::from_utf8(bytes.clone()).map_err(|_| Errno::EINVAL),
                None => Err(Errno::ENOENT),
            },
        }
    }

    fn read_bytes(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        match self.bytes.get_mut(path) {
            Some(script) => script.next(),
            None => match self.blobs.get(path) {
                Some(bytes) => Ok(bytes.clone()),
                // **Through a symbolic link**, because `read(2)` follows one and
                // `current/package-header` is how an install asks what an
                // application is running. Only a link this fake made, and only a
                // relative target, which is the only kind `Step::Activate` makes.
                None => match self.through_a_link(path) {
                    Some(bytes) => Ok(bytes),
                    // An absent file is `ENOENT`, which is what one really does —
                    // and which is what every firmware read answers under an
                    // emulator (`ADR-026` §4).
                    None => self.read_file(path).map(String::into_bytes),
                },
            },
        }
    }

    fn size_of(&mut self, path: &str) -> Result<u64, Errno> {
        if let Some(size) = self.sizes.get(path) {
            return Ok(*size);
        }
        match self.blobs.get(path) {
            Some(bytes) => Ok(bytes.len() as u64),
            None => Err(Errno::ENOENT),
        }
    }

    fn read_range(
        &mut self,
        path: &str,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), Errno> {
        let Some(blob) = self.blobs.get(path) else {
            return Err(Errno::ENOENT);
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let want = usize::try_from(len).unwrap_or(usize::MAX);
        let Some(end) = start.checked_add(want) else {
            return Err(Errno::EIO);
        };
        if end > blob.len() {
            // A short file, which the signed header says cannot be. `ADR-029`
            // F1's truncated transfer.
            return Err(Errno::EIO);
        }
        // Chunked, so a test drives the same loop the real one does.
        let mut at = start;
        while at < end {
            let take = (end - at).min(64 * 1024);
            if !sink(&blob[at..at + take]) {
                return Ok(());
            }
            at += take;
        }
        Ok(())
    }

    fn write_device(&mut self, src: &str, offset: u64, len: u64, dst: &str) -> Result<u64, Errno> {
        self.device_writes.push(DeviceWrite {
            source: src.to_string(),
            offset,
            len,
            device: dst.to_string(),
        });
        self.device_write_results.next()?;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let want = usize::try_from(len).unwrap_or(usize::MAX);
        let Some(source) = self.blobs.get(src) else {
            return Err(Errno::ENOENT);
        };
        let Some(end) = start.checked_add(want) else {
            return Err(Errno::EIO);
        };
        if end > source.len() {
            return Err(Errno::EIO);
        }
        let mut content = source[start..end].to_vec();
        if self.lying.contains(dst)
            && let Some(byte) = content.last_mut()
        {
            // A single flipped bit, at the far end, silently. The write reports
            // success, as a dying card's does.
            *byte ^= 0x01;
        }
        let target = self.blobs.entry(dst.to_string()).or_default();
        // A block device is not truncated: the bytes past the member's length
        // are whatever was there. That is what makes the read-back assertion in
        // the install tests meaningful.
        if target.len() < content.len() {
            target.resize(content.len(), 0);
        }
        target[..content.len()].copy_from_slice(&content);
        Ok(len)
    }

    fn write_file_synced(&mut self, path: &str, bytes: &[u8]) -> Result<(), Errno> {
        self.ops.push(format!("write {path}"));
        self.synced_writes.push((path.to_string(), bytes.to_vec()));
        self.write_results.next()?;
        self.blobs.insert(path.to_string(), bytes.to_vec());
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        self.ops.push(format!("rename {from} -> {to}"));
        self.renames.push((from.to_string(), to.to_string()));
        self.rename_results.next()?;
        if let Some(bytes) = self.blobs.remove(from) {
            self.blobs.insert(to.to_string(), bytes);
        }
        // A symlink renamed over another is how `current` moves, so the fake
        // has to model it or the atomic swap is untested.
        if let Some(target) = self.links.remove(from) {
            self.links.insert(to.to_string(), target);
        }
        Ok(())
    }

    fn mkdir_mode(&mut self, path: &str, mode: u32) -> Result<(), Errno> {
        self.ops.push(format!("mkdir {path}"));
        self.mkdirs.push((path.to_string(), mode));
        Ok(())
    }

    /// **Everything this fake has been asked to make or given**, which is what
    /// makes the model honest for `ADR-015` §7.2: a second install against the
    /// same fake sees the directories the first one created, exactly as a second
    /// install on a board sees them.
    fn path_exists(&mut self, path: &str) -> bool {
        self.pre_existing.contains(path)
            || self.mkdirs.iter().any(|(made, _)| made == path)
            || self.blobs.contains_key(path)
            || self.links.contains_key(path)
    }

    fn unlink_symlink(&mut self, path: &str) -> Result<(), Errno> {
        self.ops.push(format!("unlink {path}"));
        self.unlinks.push(path.to_string());
        if self.blobs.contains_key(path) {
            // A regular file at the staging name is refused, exactly as the
            // real one refuses it.
            return Err(Errno(libc::EPERM));
        }
        self.links.remove(path);
        Ok(())
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), Errno> {
        self.ops.push(format!("chmod {path}"));
        self.chmods.push((path.to_string(), mode));
        Ok(())
    }

    fn symlink(&mut self, target: &str, path: &str) -> Result<(), Errno> {
        // Refuses an existing path, exactly as `symlink(2)` does — otherwise a
        // test would pass for a sequence that cannot work on a board.
        if self.links.contains_key(path) || self.blobs.contains_key(path) {
            return Err(Errno(libc::EEXIST));
        }
        self.ops.push(format!("symlink {path} -> {target}"));
        self.links.insert(path.to_string(), target.to_string());
        Ok(())
    }

    fn sync(&mut self) {
        self.syncs += 1;
    }

    fn mount(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno> {
        self.mount_calls.push(MountCall {
            device: device.to_string(),
            at: at.to_string(),
            fstype: fstype.to_string(),
            writable,
        });
        self.sequence.push(format!("mount {at}"));
        let answer = self.mount_results.next();
        if answer.is_ok() && self.adopt_at.as_deref() == Some(at) {
            return Ok(Mounted::Adopted);
        }
        answer
    }

    fn umount(&mut self, at: &str) -> Result<(), Errno> {
        self.umount_calls.push(at.to_string());
        self.sequence.push(format!("umount {at}"));
        self.umount_results.next()
    }

    /// Records the bound asked for and takes no time, whatever it answers.
    fn lock_within(&mut self, path: &str, bound_ms: u64) -> Result<(), Errno> {
        self.lock_calls.push(format!("lock {path} within {bound_ms}"));
        let answer = self.lock_results.next();
        if answer.is_ok() {
            self.sequence.push(format!("lock {path}"));
        }
        answer
    }

    fn unlock(&mut self, path: &str) {
        self.lock_calls.push(format!("unlock {path}"));
        self.sequence.push(format!("unlock {path}"));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_script_repeats_its_last_answer_for_ever() {
        let mut s = Script::sequence(vec![1, 2], 9);
        assert_eq!([s.next(), s.next(), s.next(), s.next()], [1, 2, 9, 9]);
    }

    /// An absent path must be `ENOENT` rather than a panic or an empty answer:
    /// "absent" is the state under test in half of these suites.
    #[test]
    fn an_unconfigured_path_is_absent_in_every_form() {
        let mut k = FakeKernel::new();
        assert_eq!(k.read_file("/anything"), Err(Errno::ENOENT));
        assert_eq!(k.read_bytes("/anything"), Err(Errno::ENOENT));
        assert_eq!(k.size_of("/anything"), Err(Errno::ENOENT));
    }

    /// A block device longer than what is written to it keeps its tail. Without
    /// that, a read-back test would pass by comparing a truncated device
    /// against a truncated expectation.
    #[test]
    fn a_device_write_leaves_the_bytes_past_the_member_alone() {
        let mut k = FakeKernel::new();
        k.put("/payload", vec![1, 2, 3, 4]);
        k.put("/dev/x", vec![9; 8]);
        assert_eq!(k.write_device("/payload", 1, 2, "/dev/x"), Ok(2));
        assert_eq!(
            k.device_content("/dev/x"),
            Some(&[2, 3, 9, 9, 9, 9, 9, 9][..])
        );
    }

    /// A read past the end of a blob is `EIO` rather than a short answer. The
    /// length came from a signed header (`ADR-029` §5), so a file that cannot
    /// supply it is a truncated transfer.
    #[test]
    fn a_read_past_the_end_is_an_error_rather_than_a_short_answer() {
        let mut k = FakeKernel::new();
        k.put("/f", vec![0; 4]);
        let mut seen = 0usize;
        let outcome = k.read_range("/f", 0, 8, &mut |c| {
            seen += c.len();
            true
        });
        assert_eq!(outcome, Err(Errno::EIO));
        assert_eq!(seen, 0);
    }

    /// A sink that says stop must stop, and the stop must not be an error: the
    /// caller holds the reason.
    #[test]
    fn a_sink_that_refuses_stops_the_read_without_making_it_a_failure() {
        let mut k = FakeKernel::new();
        k.put("/f", vec![0; 256 * 1024]);
        let mut seen = 0usize;
        let outcome = k.read_range("/f", 0, 256 * 1024, &mut |c| {
            seen += c.len();
            false
        });
        assert_eq!(outcome, Ok(()));
        assert_eq!(seen, 64 * 1024, "the read did not stop at the first chunk");
    }
}
