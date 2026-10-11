//! **`rivet-reset`'s ordinary mode (not `--keep-authority`), run as a
//! program against scratch trees.** `SPEC-05` §2, B14: a directory link
//! inside a removal root is never followed.
//!
//! Every test always passes `--ca`, `--register` and `--provision`
//! explicitly, so nothing here can resolve to a real `~/.rivet` even though
//! `HOME`/`USERPROFILE` are also pointed at a scratch directory as a second,
//! independent guard.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// A scratch base directory, removed at the end.
struct Base(PathBuf);

impl Base {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rivet-reset-link-containment-{label}-{}-{n}",
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

/// **`reset`, bounded.** For the link-loop tests: nothing here should ever
/// take more than an instant once B14 is fixed (the walk never opens a
/// link, so nothing can recurse through one), so a generous bound catches
/// the old behavior — a runaway recursion, however it actually terminates
/// on this platform — without hanging the test suite. `None` means the
/// timeout was hit and the process was killed.
fn reset_bounded(base: &Base, args: &[&str], timeout: Duration) -> Option<(Output, String)> {
    let unused_home = base.join("unused-home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rivet-reset"))
        .args(args)
        .env("HOME", &unused_home)
        .env("USERPROFILE", &unused_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("rivet-reset spawns");
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_end(&mut stderr);
    }
    let text =
        format!("{}{}", String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr));
    Some((Output { status, stdout, stderr }, text))
}

/// **A symlink inside `--provision` points at an unrelated scratch tree.**
/// The run must remove the link itself and must never read, size, or
/// delete anything the link points to.
#[cfg(unix)]
#[test]
fn a_symlink_inside_a_removal_root_does_not_reach_outside_it() {
    let base = Base::new("link-contain-unix");
    let ca = base.join("ca-empty");
    std::fs::create_dir_all(&ca).expect("an empty authority directory");
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision");
    std::fs::create_dir_all(&provision).expect("a provision directory");
    let unrelated = base.join("unrelated");
    write(&unrelated.join("secret.txt"), b"must survive, byte for byte");
    std::os::unix::fs::symlink(&unrelated, provision.join("link")).expect("a symlink");

    let (out, text) = reset(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--proceed-without-backup",
            "--confirm",
        ],
    );
    assert!(out.status.success(), "{text}");
    assert_eq!(
        std::fs::read(unrelated.join("secret.txt")).expect("the unrelated file must survive"),
        b"must survive, byte for byte"
    );
    assert!(!provision.exists(), "the (now-empty) removal root should be gone: {text}");
}

/// **A Windows junction inside `--provision` points at an unrelated scratch
/// tree** — the Windows counterpart of the Unix symlink-containment test
/// above, created with `cmd /C mklink /J`.
#[cfg(windows)]
#[test]
fn a_junction_inside_a_removal_root_does_not_reach_outside_it() {
    let base = Base::new("link-contain-windows");
    let ca = base.join("ca-empty");
    std::fs::create_dir_all(&ca).expect("an empty authority directory");
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision");
    std::fs::create_dir_all(&provision).expect("a provision directory");
    let unrelated = base.join("unrelated");
    write(&unrelated.join("secret.txt"), b"must survive, byte for byte");

    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(provision.join("link"))
        .arg(&unrelated)
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
            "--proceed-without-backup",
            "--confirm",
        ],
    );
    assert!(out.status.success(), "{text}");
    assert_eq!(
        std::fs::read(unrelated.join("secret.txt")).expect("the unrelated file must survive"),
        b"must survive, byte for byte"
    );
    assert!(!provision.exists(), "the (now-empty) removal root should be gone: {text}");
}

/// **A symlink loop terminates.** `provision/self` points back at
/// `provision` itself; the walk must never recurse through it. Bounded so
/// the OLD behavior — however it actually ends on this platform — cannot
/// hang the test suite.
#[cfg(unix)]
#[test]
fn a_symlink_loop_terminates() {
    let base = Base::new("loop-unix");
    let ca = base.join("ca-empty");
    std::fs::create_dir_all(&ca).expect("an empty authority directory");
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision");
    std::fs::create_dir_all(&provision).expect("a provision directory");
    std::os::unix::fs::symlink(&provision, provision.join("self"))
        .expect("a self-referential symlink");

    let outcome = reset_bounded(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--dry-run",
        ],
        Duration::from_secs(20),
    );
    let Some((out, text)) = outcome else {
        panic!("rivet-reset did not terminate within the bound — a symlink loop was followed");
    };
    assert!(out.status.success(), "{text}");
}

/// **A junction "loop" terminates** — the Windows counterpart, using a
/// junction pointing back at its own parent.
#[cfg(windows)]
#[test]
fn a_junction_loop_terminates() {
    let base = Base::new("loop-windows");
    let ca = base.join("ca-empty");
    std::fs::create_dir_all(&ca).expect("an empty authority directory");
    let register = base.join("register-does-not-exist");
    let provision = base.join("provision");
    std::fs::create_dir_all(&provision).expect("a provision directory");

    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(provision.join("self"))
        .arg(&provision)
        .status()
        .expect("mklink runs");
    assert!(status.success(), "mklink /J failed: {status}");

    let outcome = reset_bounded(
        &base,
        &[
            "--ca",
            path_str(&ca),
            "--register",
            path_str(&register),
            "--provision",
            path_str(&provision),
            "--dry-run",
        ],
        Duration::from_secs(20),
    );
    let Some((out, text)) = outcome else {
        panic!("rivet-reset did not terminate within the bound — a junction loop was followed");
    };
    assert!(out.status.success(), "{text}");
}
