//! **`rivet-reset`'s ordinary mode (not `--keep-authority`), run as a
//! program against scratch trees.** `SPEC-05` §2, B13: a `--backup` this
//! run would itself delete is not a backup.
//!
//! Every test always passes `--ca`, `--register` and `--provision`
//! explicitly, so nothing here can resolve to a real `~/.rivet` even though
//! `HOME`/`USERPROFILE` are also pointed at a scratch directory as a second,
//! independent guard.
//!
//! **`root.crt`'s bytes are synthetic and deliberately not a certificate —
//! see [`synthetic_root_bytes`].** `backup_covers`/`same_or_inside` only
//! ever compare raw bytes, so a real certificate would prove nothing more;
//! using one would also make `root_der` parse successfully and send the run
//! into `store::survey()`, which opens the REAL Windows certificate store —
//! exactly what these file-system-only tests must not do.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch base directory, removed at the end.
struct Base(PathBuf);

impl Base {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rivet-reset-backup-safety-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch base");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Base {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Stand-in "certificate" bytes for `root.crt`, salted so two calls never
/// collide. Hand-composed and deliberately NOT valid UTF-8 (so never valid
/// PEM) — see the module doc for why that is load-bearing here rather than
/// an oversight.
fn synthetic_root_bytes(salt: u8) -> Vec<u8> {
    let mut bytes = vec![0xFFu8, 0xFEu8, 0x00u8];
    bytes.extend_from_slice(b"synthetic non-certificate bytes standing in for an authority's root ");
    bytes.push(salt);
    bytes
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
    std::fs::write(path, bytes).expect("written");
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 scratch path")
}

/// Run the built `rivet-reset` binary in its ORDINARY mode (never
/// `--keep-authority`). `HOME`/`USERPROFILE` point at a scratch directory
/// that nothing here names by any flag, so a bug that fell back to a
/// default would land on scratch, not on this machine's real fleet.
fn reset(base: &Base, args: &[&str]) -> (Output, String) {
    let unused_home = base.join("unused-home");
    let out = Command::new(env!("CARGO_BIN_EXE_rivet-reset"))
        .args(args)
        .env("HOME", &unused_home)
        .env("USERPROFILE", &unused_home)
        .output()
        .expect("rivet-reset runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out, text)
}

/// **Same directory: `--backup` names the authority itself.** Before the
/// fix, `ca/root.crt` trivially byte-matched itself, so this read as "a
/// backup was found" and the confirmed run proceeded to delete the
/// authority along with its own supposed backup.
#[test]
fn a_backup_matching_the_authority_directory_itself_is_refused() {
    let base = Base::new("same-dir");
    let ca = base.join("ca");
    write(&ca.join("root.crt"), &synthetic_root_bytes(1));
    write(&ca.join("root.key"), b"the fleet's only copy of its trust");
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision-does-not-exist");

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--backup",
            path_str(&ca),
            "--confirm",
        ],
    );
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("REFUSED"), "{text}");
    assert_eq!(
        std::fs::read(ca.join("root.crt")).expect("root.crt must survive"),
        synthetic_root_bytes(1)
    );
    assert!(ca.join("root.key").exists(), "root.key must survive: {text}");
}

/// **Nested: `--backup` is a location INSIDE a different removal root**
/// (`--provision`, not the authority) that happens to hold a byte-for-byte
/// copy. Diversifies B13 coverage beyond the authority's own directory —
/// the same defect reaches every removal root, not only `--ca`.
#[test]
fn a_backup_nested_inside_the_provision_root_is_refused() {
    let base = Base::new("nested");
    let ca = base.join("ca");
    write(&ca.join("root.crt"), &synthetic_root_bytes(2));
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision");
    let nested_backup = provision.join("manual-backup");
    write(&nested_backup.join("root.crt"), &synthetic_root_bytes(2));

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--backup",
            path_str(&nested_backup),
            "--confirm",
        ],
    );
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("REFUSED"), "{text}");
    assert!(ca.join("root.crt").exists(), "the authority must survive: {text}");
    assert!(nested_backup.join("root.crt").exists(), "the nested 'backup' must survive too: {text}");
}

/// **Equivalent spelling: `--backup` reaches the authority through `..`**
/// rather than its own name — lexically a different `PathBuf`, the same
/// directory once resolved. No symlink needed, so this runs on every
/// platform.
#[test]
fn a_backup_reached_through_an_equivalent_spelling_is_refused() {
    let base = Base::new("equivalent");
    let ca = base.join("ca");
    write(&ca.join("root.crt"), &synthetic_root_bytes(3));
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision-does-not-exist");
    let equivalent = base.join("ca").join("..").join("ca");
    assert_ne!(equivalent, ca, "the point of this test is a DIFFERENT spelling");

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--backup",
            path_str(&equivalent),
            "--confirm",
        ],
    );
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("REFUSED"), "{text}");
    assert!(ca.join("root.crt").exists(), "the authority must survive: {text}");
}

/// **Symlink alias: `--backup` is a symlink pointing AT the authority
/// directory.** The alias case `same_or_inside`'s canonicalization exists
/// for, and it needs B14's own link machinery to already be in place: the
/// walk must not follow the alias into the authority either, or removal
/// could reach outside `ca` while the "backup" reads as found.
#[cfg(unix)]
#[test]
fn a_backup_reached_through_a_symlink_alias_is_refused() {
    let base = Base::new("symlink-alias");
    let ca = base.join("ca");
    write(&ca.join("root.crt"), &synthetic_root_bytes(4));
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision-does-not-exist");
    let alias = base.join("alias-of-ca");
    std::os::unix::fs::symlink(&ca, &alias).expect("a symlink");

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--backup",
            path_str(&alias),
            "--confirm",
        ],
    );
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("REFUSED"), "{text}");
    assert!(ca.join("root.crt").exists(), "the authority must survive: {text}");
}

/// **Junction alias: `--backup` is a Windows junction pointing AT the
/// authority directory**, created the way `cmd /C mklink /J` does it — the
/// Windows counterpart of the Unix symlink-alias test above.
#[cfg(windows)]
#[test]
fn a_backup_reached_through_a_junction_alias_is_refused() {
    let base = Base::new("junction-alias");
    let ca = base.join("ca");
    write(&ca.join("root.crt"), &synthetic_root_bytes(5));
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision-does-not-exist");
    let alias = base.join("alias-of-ca");

    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&alias)
        .arg(&ca)
        .status()
        .expect("mklink runs");
    assert!(status.success(), "mklink /J failed: {status}");

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--backup",
            path_str(&alias),
            "--confirm",
        ],
    );
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("REFUSED"), "{text}");
    assert!(ca.join("root.crt").exists(), "the authority must survive: {text}");
}

/// **The control case: an independent backup, outside every removal root,
/// is NOT refused — and survives a confirmed reset byte for byte.** A hard
/// refusal that overreached into this case would defeat the whole feature:
/// `--backup` exists so a real, external copy can be trusted.
#[test]
fn an_independent_backup_is_accepted_and_survives_a_confirmed_reset() {
    let base = Base::new("independent");
    let ca = base.join("ca");
    write(&ca.join("root.crt"), &synthetic_root_bytes(6));
    write(&ca.join("root.key"), b"the fleet's only copy of its trust");
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision-does-not-exist");
    let backup = base.join("far-away-backup");
    write(&backup.join("root.crt"), &synthetic_root_bytes(6));

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--backup",
            path_str(&backup),
            "--confirm",
        ],
    );
    assert!(out.status.success(), "{text}");
    assert!(!text.contains("REFUSED"), "{text}");
    assert!(!ca.join("root.crt").exists(), "the authority should have been removed: {text}");
    assert_eq!(
        std::fs::read(backup.join("root.crt")).expect("the independent backup must survive"),
        synthetic_root_bytes(6),
        "the independent backup's bytes must be exactly what they were"
    );
}
