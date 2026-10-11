//! A [`BootMedium`] that answers however a test needs it to.
//!
//! Compiled only under `cfg(test)`. It exists because the states this crate has
//! to be right about are states no developer's machine and no emulator ever
//! reaches: `ADR-026` §4 says QEMU does not emulate
//! `/proc/device-tree/chosen/bootloader` **at all**, so without this every
//! decision in [`crate::slots`] would be exercised by nothing until it reached a
//! board.
//!
//! The same shape as `cmd/rivet-initd`'s `testkernel` and `rivet-store`'s
//! `testfs`, deliberately: every answer is a [`Script`], so a *sequence* of
//! answers can be expressed, and the defaults are the boring happy path so a
//! test only has to say what is unusual about the world it is describing.

use std::collections::{BTreeMap, VecDeque};

use crate::port::{BootMedium, DirEntry, Errno, Listing, Mounted};

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
        self.queue
            .pop_front()
            .unwrap_or_else(|| self.forever.clone())
    }
}

/// One recorded [`BootMedium::mount_boot_filesystem`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountCall {
    pub device: String,
    pub at: String,
    pub fstype: String,
    pub writable: bool,
}

/// The fake medium.
pub struct FakeMedium {
    pub mount_calls: Vec<MountCall>,
    pub unmount_calls: Vec<String>,
    pub write_calls: Vec<(String, Vec<u8>)>,
    pub rename_calls: Vec<(String, String)>,
    pub sync_calls: usize,

    pub mount_results: Script<Result<Mounted, Errno>>,
    pub write_results: Script<Result<(), Errno>>,
    /// Every lock and unlock, in order, as `lock <path>` / `unlock <path>`,
    /// interleaved with `mount` and `unmount` so a test can assert the lock
    /// brackets the mount.
    pub order: Vec<String>,
    pub lock_results: Script<Result<(), Errno>>,
    /// Every mount point [`BootMedium::reclaimed_orphan`] was told about.
    pub reclaimed: Vec<String>,
    pub rename_results: Script<Result<(), Errno>>,

    /// Every `list_dir` asked for, as `(path, limit)`.
    pub list_calls: Vec<(String, usize)>,
    /// What `list_dir` answers, before the limit is applied. An unscripted
    /// fake answers `ENOENT`, as an absent directory does.
    pub list_results: Script<Result<Vec<DirEntry>, Errno>>,

    /// Per-path text answers. A path with no entry reads `ENOENT`, which is what
    /// an absent file actually does.
    files: BTreeMap<String, Script<Result<String, Errno>>>,
    /// Per-path byte answers, for the firmware's device-tree properties.
    properties: BTreeMap<String, Script<Result<Vec<u8>, Errno>>>,
}

impl FakeMedium {
    pub fn new() -> Self {
        Self {
            mount_calls: Vec::new(),
            unmount_calls: Vec::new(),
            write_calls: Vec::new(),
            rename_calls: Vec::new(),
            sync_calls: 0,
            mount_results: Script::always(Ok(Mounted::Made)),
            write_results: Script::always(Ok(())),
            order: Vec::new(),
            lock_results: Script::always(Ok(())),
            reclaimed: Vec::new(),
            rename_results: Script::always(Ok(())),
            list_calls: Vec::new(),
            list_results: Script::always(Err(Errno::ENOENT)),
            files: BTreeMap::new(),
            properties: BTreeMap::new(),
        }
    }

    pub fn files_mut(&mut self) -> &mut BTreeMap<String, Script<Result<String, Errno>>> {
        &mut self.files
    }

    pub fn properties_mut(&mut self) -> &mut BTreeMap<String, Script<Result<Vec<u8>, Errno>>> {
        &mut self.properties
    }
}

impl Default for FakeMedium {
    fn default() -> Self {
        Self::new()
    }
}

impl BootMedium for FakeMedium {
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        match self.properties.get_mut(path) {
            Some(script) => script.next(),
            // Falling back to the text map keeps a test from having to say the
            // same thing twice when a path is read both ways.
            None => self.read_file(path).map(String::into_bytes),
        }
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        match self.files.get_mut(path) {
            Some(script) => script.next(),
            None => Err(Errno::ENOENT),
        }
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        self.write_calls.push((path.to_string(), contents.to_vec()));
        self.write_results.next()
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        self.rename_calls.push((from.to_string(), to.to_string()));
        self.rename_results.next()
    }

    fn sync(&mut self) {
        self.sync_calls += 1;
    }

    fn mount_boot_filesystem(
        &mut self,
        device: &str,
        at: &str,
        fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno> {
        self.order.push(format!("mount {at}"));
        self.mount_calls.push(MountCall {
            device: device.to_string(),
            at: at.to_string(),
            fstype: fstype.to_string(),
            writable,
        });
        self.mount_results.next()
    }

    fn unmount(&mut self, at: &str) {
        self.order.push(format!("unmount {at}"));
        self.unmount_calls.push(at.to_string());
    }

    fn reclaimed_orphan(&mut self, at: &str) {
        self.reclaimed.push(at.to_string());
    }

    fn lock(&mut self, path: &str) -> Result<(), Errno> {
        self.order.push(format!("lock {path}"));
        self.lock_results.next()
    }

    fn unlock(&mut self, path: &str) {
        self.order.push(format!("unlock {path}"));
    }

    fn list_dir(&mut self, path: &str, limit: usize) -> Result<Listing, Errno> {
        self.list_calls.push((path.to_string(), limit));
        let mut entries = self.list_results.next()?;
        let total = entries.len();
        entries.truncate(limit);
        Ok(Listing { entries, total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_repeats_its_last_answer_for_ever() {
        let mut s = Script::sequence(vec![1, 2], 9);
        assert_eq!([s.next(), s.next(), s.next(), s.next()], [1, 2, 9, 9]);
    }

    /// The fake must model an absent file as `ENOENT` rather than as a panic or
    /// an empty string, because "absent" is the state under test in half of
    /// these suites — it is what every emulator run answers for a firmware
    /// property (`ADR-026` §4).
    #[test]
    fn an_unconfigured_path_reads_as_absent_in_both_forms() {
        let mut m = FakeMedium::new();
        assert_eq!(m.read_file("/anything"), Err(Errno::ENOENT));
        assert_eq!(m.read_property("/anything"), Err(Errno::ENOENT));
    }

    #[test]
    fn a_text_file_can_also_be_read_as_a_property() {
        let mut m = FakeMedium::new();
        m.files_mut()
            .insert("/f".to_string(), Script::always(Ok("1".to_string())));
        assert_eq!(m.read_property("/f"), Ok(b"1".to_vec()));
    }
}
