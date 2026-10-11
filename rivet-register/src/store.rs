//! **The register's I/O**, and the only module here that touches a disk.
//!
//! `ADR-014` §3 puts I/O at the composition root, and `crate::register`
//! carries the argument for it in the other direction: rendering is separate
//! from writing, so every property the files must have — stable ordering, no
//! timestamps, one device per file — is asserted without a filesystem. This
//! module is the thin half that remains.
//!
//! # Every write here goes through `durably`
//!
//! `save`'s device records and `save_fleet`'s fleet-wide file are both
//! published by the private `durably` helper below, on one path rather than
//! two — see its own doc comment for what that buys and what it costs on a
//! platform with no equivalent. On Unix `durably` borrows
//! `rivet_store::atomic::commit`'s write-sync-rename-sync-directory sequence
//! rather than re-implementing it, because that sequence's *order* is the
//! entire guarantee and a second copy is a second place for a step to go
//! missing in a later refactor.
//!
//! # Why a register needs that guarantee at all
//!
//! It did not always. `ADR-034` §2 once kept the register on a workstation,
//! version-controlled by the operator, where the worst failure was a file the
//! operator could see was half-written and could restore from their own
//! history — paying `rivet-store`'s price here would have borrowed a
//! guarantee from a different problem. Dropping that mode moved the register
//! onto a fleet server's `/state` instead — a Rivet machine that can lose
//! power mid-write, exactly what `rivet-store` is careful about and exactly
//! what a workstation is not. `ADR-034` §3 is the remaining consolation for
//! whatever a platform's own gap leaves open: a register can always be
//! rebuilt by asking the fleet.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    DEVICES_DIR, Fleet, RECORD_SUFFIX, Register, RegisterError, read_device, render,
};

/// **Write one file so that a power cut leaves the old contents or the new
/// ones and never a mixture** — where that can be honestly promised.
///
/// # Why this is not `fs::write`
///
/// It was, and the argument for that is struck at the top of this module: it
/// rested on the register living on a workstation, and `ADR-034` §2 moved it
/// onto `/state` on a machine that loses power. `rivet_store::atomic::commit` is
/// the sequence, borrowed rather than re-implemented, because its own module
/// says why a second copy is the wrong answer: four calls whose *order* is the
/// entire guarantee, failing only when somebody pulls the power out.
///
/// # Why Windows writes plainly, and says so
///
/// `rivet_store::StdFilesystem` is `cfg(unix)` on purpose — `sync_dir` has no
/// Windows equivalent, and an implementation returning `Ok(())` would report a
/// durability barrier that never happened. **That refusal is right and it is
/// kept**: this falls back to a plain write rather than pretending, because
/// `AGENTS.md` §3 is exactly the rule — stubs refuse, they never fake success.
///
/// The fallback is also the honest deployment story rather than a gap.
/// `ADR-034` §2 makes the workstation a **client**: it does not open these
/// files, it asks a fleet server. So the Windows path here writes a register
/// only in tests and in tooling, where the original argument — a person can see
/// a half-written file and has their own history — still stands.
///
/// # Errors
///
/// [`RegisterError::Io`] naming the path, whichever path was taken.
pub(crate) fn durably(path: &Path, bytes: &[u8]) -> Result<(), RegisterError> {
    #[cfg(unix)]
    {
        let mut filesystem = rivet_store::StdFilesystem::new();
        durably_via(&mut filesystem, path, bytes)
    }
    #[cfg(not(unix))]
    {
        fs::write(path, bytes).map_err(|e| io(path, &e))
    }
}

/// `durably`'s Unix sequence, with the filesystem taken as a parameter so a
/// test can substitute one that fails exactly where it is told to.
///
/// `durably` itself always passes `rivet_store::StdFilesystem`; this split
/// exists so a test can pass something else instead, without changing what a
/// real save does. A post-rename directory-durability failure needs exactly
/// this kind of substitution to reach at all — nothing on a development disk
/// fails there without an actual power cut.
///
/// # Errors
///
/// As `durably`.
#[cfg(any(test, unix))]
fn durably_via(
    filesystem: &mut dyn rivet_store::Filesystem,
    path: &Path,
    bytes: &[u8],
) -> Result<(), RegisterError> {
    let Some(dir) = path.parent().and_then(Path::to_str) else {
        return Err(RegisterError::Io {
            path: path.to_path_buf(),
            detail: "this path has no directory to commit into".to_owned(),
        });
    };
    let Some(target) = path.to_str() else {
        return Err(RegisterError::Io {
            path: path.to_path_buf(),
            detail: "this path is not UTF-8".to_owned(),
        });
    };
    // A sibling, because `rename(2)` is atomic only within one filesystem.
    let temp = format!("{target}.new");
    rivet_store::atomic::commit(filesystem, dir, target, &temp, bytes).map_err(|e| {
        RegisterError::Io {
            path: path.to_path_buf(),
            detail: format!("{e:?}"),
        }
    })
}

pub(crate) fn io(path: &Path, error: &std::io::Error) -> RegisterError {
    RegisterError::Io {
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

/// Read the whole register from `root`.
///
/// **A directory that is not there is an empty register, not a failure.** A
/// first run has nothing to read, and a tool that refused would make enrolling
/// the first device impossible. A directory that exists and cannot be read is a
/// different thing and is reported.
///
/// # Errors
///
/// [`RegisterError::Io`] for a path that exists and will not be read,
/// [`RegisterError::Malformed`] for a file that is not a record, and
/// [`RegisterError::Misfiled`] for one whose `id` and file name disagree.
pub fn load(root: &Path) -> Result<Register, RegisterError> {
    let mut register = Register::empty(root);
    register.fleet = load_fleet(root)?;

    let devices_dir = root.join(DEVICES_DIR);
    let entries = match fs::read_dir(&devices_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(register),
        Err(e) => return Err(io(&devices_dir, &e)),
    };

    // Collected and sorted rather than taken in directory order: `read_dir`
    // makes no ordering promise and the register's whole value depends on two
    // runs producing the same thing.
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io(&devices_dir, &e))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| {
            RECORD_SUFFIX
                .strip_prefix('.')
                .is_some_and(|want| e.eq_ignore_ascii_case(want))
        }) {
            paths.push(path);
        }
    }
    paths.sort();

    for path in paths {
        let text = fs::read_to_string(&path).map_err(|e| io(&path, &e))?;
        let device = read_device(&path, &text)?;
        register.devices.insert(device.id.clone(), device);
    }
    Ok(register)
}

/// Read the fleet-wide file alone, without reading any device record.
///
/// A caller that needs only the fleet's own facts — its name — must not be
/// refused because one device record beside it is unreadable. An absent file
/// is the default fleet, as [`load`] treats it.
///
/// # Errors
///
/// [`RegisterError::Io`] for a file that exists and will not be read, and
/// [`RegisterError::Malformed`] for one that is not a fleet record.
pub fn load_fleet(root: &Path) -> Result<Fleet, RegisterError> {
    let fleet_path = Register::empty(root).fleet_path();
    match fs::read_to_string(&fleet_path) {
        Ok(text) => toml_edit::de::from_str(&text).map_err(|e| RegisterError::Malformed {
            path: fleet_path.clone(),
            detail: e.to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Fleet::default()),
        Err(e) => Err(io(&fleet_path, &e)),
    }
}

/// Write the whole register to its root, creating directories as needed.
///
/// **Every file is rewritten**, which is what makes the output a function of
/// the state rather than of the history of edits — and is why
/// `crate::register::render` refuses to put a timestamp in one.
///
/// # Errors
///
/// [`RegisterError::Io`] for anything the filesystem refuses, and
/// [`RegisterError::Malformed`] if a record will not render.
pub fn save(register: &Register) -> Result<Vec<PathBuf>, RegisterError> {
    let files = render(register)?;
    let mut written = Vec::with_capacity(files.len());
    for (path, text) in files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, &e))?;
        }
        durably(&path, text.as_bytes())?;
        written.push(path);
    }
    Ok(written)
}

/// Remove one device's record from disk.
///
/// **The file is deleted and nothing else is touched.** Removing a device from
/// the register says nothing about the machine — `crate::operation`'s summary
/// for `remove device` says so in the words the operator reads, and this is the
/// implementation agreeing with it.
///
/// # Errors
///
/// [`RegisterError::Io`] for a file that exists and will not be removed. A file
/// that is already gone is not an error: the requested state is the state.
pub fn forget(register: &Register, id: &str) -> Result<(), RegisterError> {
    let path = register.record_path(id);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(&path, &e)),
    }
}

/// Write the fleet-wide file alone, without rewriting every device.
///
/// # Errors
///
/// As [`save`].
pub fn save_fleet(root: &Path, fleet: &Fleet) -> Result<PathBuf, RegisterError> {
    let mut register = Register::empty(root);
    register.fleet = fleet.clone();
    let path = register.fleet_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io(parent, &e))?;
    }
    let text =
        toml_edit::ser::to_string_pretty(fleet).map_err(|e| RegisterError::Malformed {
            path: path.clone(),
            detail: e.to_string(),
        })?;
    durably(&path, text.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::Device;

    /// A real directory under the OS temporary directory, removed at the end.
    ///
    /// **`std::env::temp_dir` and a counter**, rather than a crate: the
    /// workspace has no `tempfile` and taking one to write four tests would be
    /// a dependency decision (`ADR-009` §4) rather than a test.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("rivet-fleet-{label}-{n}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn device(id: &str, name: &str) -> Device {
        Device {
            id: id.to_owned(),
            name: name.to_owned(),
            ..Device::default()
        }
    }

    /// **A first run has nothing to read and must still work.** A tool that
    /// refused here could never enroll the first device.
    #[test]
    fn a_register_that_does_not_exist_yet_is_an_empty_one() {
        let scratch = Scratch::new("absent");
        let root = scratch.0.join("never-created");
        let register = load(&root).expect("an absent register is empty");
        assert!(register.devices.is_empty());
        assert_eq!(register.fleet, Fleet::default());
    }

    /// The round trip, on a real filesystem: what was saved is what loads.
    #[test]
    fn what_was_written_is_what_comes_back() {
        let scratch = Scratch::new("roundtrip");
        let mut register = Register::empty(&scratch.0);
        register.fleet.name = Some("home".to_owned());
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        register.enroll(device("8899aabbccddeeff", "garage-pi")).expect("fresh");

        let written = save(&register).expect("writable");
        assert_eq!(written.len(), 3, "{written:?}");

        let back = load(&scratch.0).expect("readable");
        assert_eq!(back, register);
    }

    /// Saving twice writes the same bytes, which is the property that makes a
    /// register worth committing.
    #[test]
    fn saving_twice_changes_nothing_on_disk() {
        let scratch = Scratch::new("idempotent");
        let mut register = Register::empty(&scratch.0);
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");

        let paths = save(&register).expect("writable");
        let first: Vec<String> = paths
            .iter()
            .map(|p| fs::read_to_string(p).expect("readable"))
            .collect();
        save(&register).expect("writable");
        let second: Vec<String> = paths
            .iter()
            .map(|p| fs::read_to_string(p).expect("readable"))
            .collect();
        assert_eq!(first, second);
    }

    /// **A hand-renamed record is refused on load**, not repaired — the same
    /// refusal `crate::register::read_device` makes, reaching the operator
    /// through the path they would actually hit.
    #[test]
    fn a_record_renamed_by_hand_is_refused_when_the_register_is_read() {
        let scratch = Scratch::new("misfiled");
        let mut register = Register::empty(&scratch.0);
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        save(&register).expect("writable");

        let from = register.record_path("0011223344556677");
        let to = scratch.0.join(DEVICES_DIR).join("kitchen-pi.toml");
        fs::rename(&from, &to).expect("renamable");

        let error = load(&scratch.0).expect_err("misfiled");
        assert!(matches!(error, RegisterError::Misfiled { .. }), "{error:?}");
    }

    /// Forgetting removes the record and leaves everything else standing.
    #[test]
    fn forgetting_a_device_removes_its_file_and_no_other() {
        let scratch = Scratch::new("forget");
        let mut register = Register::empty(&scratch.0);
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        register.enroll(device("8899aabbccddeeff", "garage-pi")).expect("fresh");
        save(&register).expect("writable");

        forget(&register, "0011223344556677").expect("removable");
        let back = load(&scratch.0).expect("readable");
        assert_eq!(back.devices.len(), 1);
        assert!(back.devices.contains_key("8899aabbccddeeff"));
    }

    /// **Already gone is not an error.** The requested state is the state, and
    /// a second `remove` is far more often a repeated command than a mistake
    /// worth failing over.
    #[test]
    fn forgetting_something_already_forgotten_is_not_a_failure() {
        let scratch = Scratch::new("forget-twice");
        let register = Register::empty(&scratch.0);
        forget(&register, "0011223344556677").expect("a record that was never there");
    }

    /// **A whole register written before `carry` existed still loads** —
    /// `.rulings/` R-160 R26. Hand-written rather than round-tripped through
    /// `save`, so this proves backward compatibility rather than agreement
    /// with the current writer.
    #[test]
    fn a_register_with_no_carry_anywhere_still_loads() {
        let scratch = Scratch::new("no-carry");
        let fleet_path = Register::empty(&scratch.0).fleet_path();
        fs::write(&fleet_path, "name = \"home\"\n").expect("writable");
        let devices = scratch.0.join(DEVICES_DIR);
        fs::create_dir_all(&devices).expect("creatable");
        fs::write(
            devices.join("0011223344556677.toml"),
            "id = \"0011223344556677\"\nname = \"kitchen-pi\"\n",
        )
        .expect("writable");

        let register = load(&scratch.0).expect("an old register still loads");
        assert!(register.fleet.carry.is_empty());
        assert!(register.devices["0011223344556677"].carry.is_empty());
    }

    /// A file that is not a record is refused with what the parser said, on the
    /// path an operator would meet it.
    #[test]
    fn a_file_that_is_not_a_record_is_refused_by_name() {
        let scratch = Scratch::new("malformed");
        let devices = scratch.0.join(DEVICES_DIR);
        fs::create_dir_all(&devices).expect("creatable");
        fs::write(devices.join("0011223344556677.toml"), "not toml {{{").expect("writable");

        let error = load(&scratch.0).expect_err("malformed");
        match &error {
            RegisterError::Malformed { path, .. } => {
                assert!(path.to_string_lossy().contains("0011223344556677"), "{path:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Something that is not a record file is ignored rather than refused — a
    /// `README` beside the records is an operator's business, and a register
    /// that refused to load because of one would be one nobody could document.
    #[test]
    fn a_file_that_is_not_a_record_at_all_is_left_alone() {
        let scratch = Scratch::new("stranger");
        let mut register = Register::empty(&scratch.0);
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        save(&register).expect("writable");
        fs::write(scratch.0.join(DEVICES_DIR).join("README.md"), "notes\n").expect("writable");

        assert_eq!(load(&scratch.0).expect("readable").devices.len(), 1);
    }

    // -- B08: save_fleet is durable too --------------------------------------

    /// A minimal in-memory `rivet_store::Filesystem`, built only to prove that
    /// `durably_via`'s mapping into `RegisterError` keeps a publish failure and
    /// a post-commit durability failure reading as different things — a
    /// distinction real Unix syscalls cannot be made to show on a developer's
    /// disk without an actual power cut.
    #[derive(Default)]
    struct MemFs {
        files: std::collections::BTreeMap<String, Vec<u8>>,
        fail_rename: bool,
        fail_sync_dir: bool,
    }

    impl rivet_store::Filesystem for MemFs {
        fn create_dir_all(&mut self, _path: &str) -> Result<(), rivet_store::Errno> {
            Ok(())
        }
        fn read(&mut self, path: &str) -> Result<Vec<u8>, rivet_store::Errno> {
            self.files.get(path).cloned().ok_or(rivet_store::Errno::ENOENT)
        }
        fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), rivet_store::Errno> {
            self.files.insert(path.to_owned(), bytes.to_vec());
            Ok(())
        }
        fn sync_file(&mut self, _path: &str) -> Result<(), rivet_store::Errno> {
            Ok(())
        }
        fn rename(&mut self, from: &str, to: &str) -> Result<(), rivet_store::Errno> {
            if self.fail_rename {
                return Err(rivet_store::Errno::EIO);
            }
            let bytes = self.files.remove(from).ok_or(rivet_store::Errno::ENOENT)?;
            self.files.insert(to.to_owned(), bytes);
            Ok(())
        }
        fn sync_dir(&mut self, _path: &str) -> Result<(), rivet_store::Errno> {
            if self.fail_sync_dir {
                return Err(rivet_store::Errno::EIO);
            }
            Ok(())
        }
        fn remove_file(&mut self, path: &str) -> Result<(), rivet_store::Errno> {
            self.files.remove(path);
            Ok(())
        }
    }

    /// **Acceptance 2.** A rename that is refused (nothing published) and a
    /// directory flush that is refused after a successful rename (published,
    /// durability still pending) must not read as the same failure.
    #[test]
    fn a_refused_rename_and_a_refused_directory_flush_are_reported_differently() {
        let mut refusing_rename = MemFs { fail_rename: true, ..MemFs::default() };
        let publish_err = durably_via(&mut refusing_rename, Path::new("/reg/fleet.toml"), b"x")
            .expect_err("a refused rename is a failure");

        let mut refusing_flush = MemFs { fail_sync_dir: true, ..MemFs::default() };
        let durability_err = durably_via(&mut refusing_flush, Path::new("/reg/fleet.toml"), b"x")
            .expect_err("a refused directory flush is still a failure");

        let publish_detail = match &publish_err {
            RegisterError::Io { detail, .. } => detail,
            other => panic!("{other:?}"),
        };
        let durability_detail = match &durability_err {
            RegisterError::Io { detail, .. } => detail,
            other => panic!("{other:?}"),
        };
        assert_ne!(publish_detail, durability_detail, "these must read as different failures");
        assert!(publish_detail.contains("Rename"), "{publish_detail}");
        assert!(durability_detail.contains("NotDurable"), "{durability_detail}");
        // The flush failure did publish: the new bytes are under the target
        // name even though `durably_via` returned `Err`, which is the whole
        // reason it is not reported the same way as the rename above.
        assert_eq!(refusing_flush.files.get("/reg/fleet.toml"), Some(&b"x".to_vec()));
    }

    /// **Acceptance 1.** `save`'s device records have gone through `durably`
    /// since `ADR-034` §2 moved the register onto a fleet server;
    /// `save_fleet` did not, so a write that failed before it could rename
    /// could truncate the fleet file it was replacing. This is that failure,
    /// injected by pre-creating `durably`'s own temp name as a directory so
    /// the write it must do before any rename is refused — on every real
    /// filesystem, with no permission trick a container's root user could
    /// ignore.
    #[cfg(unix)]
    #[test]
    fn a_fleet_save_that_fails_before_the_rename_leaves_the_old_file_byte_identical() {
        let scratch = Scratch::new("fleet-save-fails-before-rename");
        let mut register = Register::empty(&scratch.0);
        register.fleet.name = Some("old-name".to_owned());
        save_fleet(&scratch.0, &register.fleet).expect("the first save succeeds");
        let fleet_path = register.fleet_path();
        let before = fs::read(&fleet_path).expect("the old file is there");

        let temp = format!("{}.new", fleet_path.to_str().expect("scratch paths are UTF-8"));
        fs::create_dir_all(&temp).expect("a directory standing where the temp file would go");

        let new_fleet = Fleet { name: Some("new-name".to_owned()), ..Fleet::default() };
        let err = save_fleet(&scratch.0, &new_fleet).expect_err("the write is refused");
        assert!(matches!(err, RegisterError::Io { .. }), "{err:?}");

        let after = fs::read(&fleet_path).expect("the old file is still there");
        assert_eq!(before, after, "a failure before the rename must not touch the old file");
        let reloaded = load(&scratch.0).expect("still loadable");
        assert_eq!(reloaded.fleet.name.as_deref(), Some("old-name"));
    }

    /// **Acceptance 3.** `save_fleet` writes its own file and nothing else.
    #[test]
    fn saving_fleet_alone_does_not_rewrite_any_device_file() {
        let scratch = Scratch::new("fleet-save-leaves-devices-alone");
        let mut register = Register::empty(&scratch.0);
        register.enroll(device("0011223344556677", "kitchen-pi")).expect("fresh");
        save(&register).expect("writable");
        let device_path = register.record_path("0011223344556677");
        let before = fs::read(&device_path).expect("the device record is there");

        let mut fleet = register.fleet.clone();
        fleet.name = Some("renamed".to_owned());
        save_fleet(&scratch.0, &fleet).expect("writable");

        let after = fs::read(&device_path).expect("the device record is still there");
        assert_eq!(before, after, "save_fleet must not rewrite a device file");
        let reloaded = load(&scratch.0).expect("readable");
        assert_eq!(reloaded.devices.len(), 1);
        assert_eq!(reloaded.fleet.name.as_deref(), Some("renamed"));
    }
}
