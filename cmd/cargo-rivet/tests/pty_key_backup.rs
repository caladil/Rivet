//! **`cargo rivet key backup`/`restore`'s passphrase prompt, under a REAL
//! terminal.** `.rulings/` R-183 D3.
//!
//! `key_backup.rs`'s own unit tests (`cmd/cargo-rivet/src/key_backup.rs`,
//! `backup_with_passphrase`/`restore_with_passphrase`) prove the envelope,
//! the encryption, and every refusal — all with the passphrase handed in as
//! a plain `&str`, never through a terminal. What those tests cannot prove
//! is the one thing `rpassword::prompt_password` actually does: read a real
//! terminal in no-echo mode and NOT see a piped stdin as one at all
//! (`std::io::IsTerminal`, the same gate `write_image.rs::refuse_unless_terminal`
//! and `cmd/cargo-rivet/tests/pty_interview.rs` already exist to close for the
//! interview's own prompts). This file is that proof, on the identical
//! harness `pty_interview.rs` built and `ADR-009` surveyed:
//! `portable-pty` opens a real ConPTY (Windows) or pty (elsewhere), spawns
//! the real `cargo-rivet` binary attached to it, and types the keystrokes a
//! person would — never piped stdin, which `IsTerminal` would see as no
//! terminal at all.
//!
//! **A smaller, self-contained harness than `pty_interview.rs`'s**,
//! deliberately: that file's `Session` carries machinery this one does not
//! need (ConPTY cursor-query handling still applies and is kept; the fleet
//! repository, image-menu and register plumbing do not, because `key
//! backup`/`restore` touch neither). Not shared between the two files for
//! the reason `pty_interview.rs`'s own module doc gives for not sharing with
//! `provision_tests.rs`: each `tests/*.rs` file is its own compilation unit.
//!
//! # Running it
//!
//! Gated with `#[ignore]`, `pty_interview.rs`'s own convention — a terminal
//! `cargo test`'s own harness may not have one, and each test spawns a real
//! process.
//!
//! ```text
//! cargo build --release -p cargo-rivet
//! cargo test --release -p cargo-rivet --test pty_key_backup -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! **Run in this session:** on Windows, `SPEC-05`'s own development
//! platform. **Not run:** inside the Linux build container — `native_pty_system()`
//! would open a real Unix pty there and nothing in this file is
//! platform-specific, but it was not exercised there this session, so this
//! says so rather than claiming both.
//!
//! # Never the real `HOME`
//!
//! Every process this file spawns has `HOME`/`USERPROFILE` pointed at a
//! fresh scratch directory this test created — never this machine's real
//! one. This session's hard rule ("never read the maintainer's real
//! `~/.rivet`") binds this file exactly as it binds every other tool call in
//! it: every seed here is a fictional 32-byte fixture this test minted
//! itself, in a scratch directory, and nothing here ever points at a real
//! key.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

fn cargo_rivet_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cargo-rivet"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cmd/")
        .parent()
        .expect("the repository root")
        .to_path_buf()
}

fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    scratch::dir(&format!(
        "pty-key-backup-{tag}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A running `cargo-rivet`, attached to a real pty.
struct Session {
    writer: Box<dyn Write + Send>,
    transcript: String,
    rx: mpsc::Receiver<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    answered_cursor_query: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl Session {
    /// Spawn `cargo-rivet <args>`, with `HOME`/`USERPROFILE` pointed at
    /// `home` — a scratch directory this test created, never the real one.
    fn spawn(home: &Path, args: &[&str]) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 100, pixel_width: 0, pixel_height: 0 })
            .unwrap_or_else(|e| panic!("openpty: {e}"));

        let mut cmd = CommandBuilder::new(cargo_rivet_exe());
        for a in args {
            cmd.arg(a);
        }
        cmd.cwd(repo_root());
        cmd.env("HOME", home);
        cmd.env("USERPROFILE", home);

        let child = pair.slave.spawn_command(cmd).unwrap_or_else(|e| panic!("spawn cargo-rivet: {e}"));
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap_or_else(|e| panic!("clone pty reader: {e}"));
        let writer = pair.master.take_writer().unwrap_or_else(|e| panic!("take pty writer: {e}"));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        // See `pty_interview.rs`'s identical `forget` for why: dropping the
        // ConPTY master can tear the pseudo-console down before `wait()`
        // reads the child's exit code.
        std::mem::forget(pair.master);

        Self { writer, transcript: String::new(), rx, child, answered_cursor_query: false }
    }

    fn answer_cursor_queries(&mut self) {
        if self.answered_cursor_query {
            return;
        }
        if self.transcript.contains("\u{1b}[6n") {
            self.answered_cursor_query = true;
            let _ = self.writer.write_all(b"\x1b[24;1R");
            let _ = self.writer.flush();
        }
    }

    fn send_line(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).expect("write to the pty");
        self.writer.write_all(b"\r\n").expect("write CRLF to the pty");
        self.writer.flush().ok();
    }

    fn drain(&mut self) {
        while let Ok(chunk) = self.rx.try_recv() {
            self.transcript.push_str(&String::from_utf8_lossy(&chunk));
        }
        self.answer_cursor_queries();
    }

    fn expect(&mut self, pattern: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain();
            if self.transcript.contains(pattern) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {pattern:?}. Transcript so far:\n{}",
                    self.transcript
                );
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => {
                    self.transcript.push_str(&String::from_utf8_lossy(&chunk));
                    self.answer_cursor_queries();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.drain();
                    if self.transcript.contains(pattern) {
                        return;
                    }
                    panic!("the pty closed before {pattern:?} ever appeared. Transcript:\n{}", self.transcript);
                }
            }
        }
    }

    fn wait(&mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            self.drain();
            if let Ok(Some(status)) = self.child.try_wait() {
                self.drain();
                return status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("cargo-rivet did not exit within 30s. Transcript:\n{}", self.transcript);
    }
}

/// A fictional 32-byte seed — never a real one. `home/.rivet/keys/<name>` is
/// created holding it.
fn write_fixture_seed(home: &Path, name: &str, fill: u8) {
    let keys = home.join(".rivet").join("keys");
    std::fs::create_dir_all(&keys).expect("a keys directory");
    std::fs::write(keys.join(name), [fill; 32]).expect("write the fixture seed");
}

const SYSTEM_SEED_FILE: &str = "rivet-system.seed";
const A_PASSPHRASE: &str = "pty-proof passphrase, fictional and fixed";

/// **The full happy path**: back up one fixture seed with a real typed
/// passphrase (entered twice, as `key backup` requires), then restore it
/// into a FRESH `HOME` with the same typed passphrase, and check the
/// restored bytes match the fixture exactly.
#[test]
#[ignore = "spawns a real process under a real pty; see this file's module doc for how to run it"]
fn backup_then_restore_round_trips_a_real_typed_passphrase() {
    let backup_home = scratch("backup-home");
    write_fixture_seed(&backup_home, SYSTEM_SEED_FILE, 0xAB);
    let backup_dir = scratch("backup-dir");

    let mut session = Session::spawn(&backup_home, &["key", "backup", backup_dir.to_str().expect("utf8")]);
    session.expect("passphrase for this seed backup:", Duration::from_secs(15));
    session.send_line(A_PASSPHRASE);
    session.expect("type it again to confirm:", Duration::from_secs(10));
    session.send_line(A_PASSPHRASE);
    session.expect("file(s) written", Duration::from_secs(15));
    let status = session.wait();
    assert!(status.success(), "key backup should have succeeded:\n{}", session.transcript);

    let envelope_path = backup_dir.join(format!("{SYSTEM_SEED_FILE}.enc"));
    assert!(envelope_path.is_file(), "no envelope written at {}", envelope_path.display());
    let envelope = std::fs::read(&envelope_path).expect("read the envelope back");
    assert_ne!(
        envelope[envelope.len().saturating_sub(32)..],
        [0xABu8; 32][..],
        "the envelope must not contain the plaintext seed"
    );

    // Restore into a DIFFERENT, fresh HOME — never back onto backup_home,
    // which still holds the live fixture and would hit the overwrite refusal.
    let restore_home = scratch("restore-home");
    let mut session = Session::spawn(&restore_home, &["key", "restore", backup_dir.to_str().expect("utf8")]);
    session.expect("passphrase for this seed backup:", Duration::from_secs(15));
    session.send_line(A_PASSPHRASE);
    session.expect("seed(s) to", Duration::from_secs(15));
    let status = session.wait();
    assert!(status.success(), "key restore should have succeeded:\n{}", session.transcript);

    let restored = std::fs::read(restore_home.join(".rivet").join("keys").join(SYSTEM_SEED_FILE))
        .expect("the restored seed file");
    assert_eq!(restored, vec![0xABu8; 32], "the restored seed must match the original fixture exactly");

    eprintln!("--- backup+restore transcript ---\n{}", session.transcript);
}

/// **A wrong typed passphrase at restore time is refused, and writes
/// nothing** — the real-terminal half of what `key_backup::tests::
/// restore_with_the_wrong_passphrase_writes_nothing` already proves with a
/// passphrase handed in directly.
#[test]
#[ignore = "spawns a real process under a real pty; see this file's module doc for how to run it"]
fn a_wrong_typed_passphrase_at_restore_is_refused_and_writes_nothing() {
    let backup_home = scratch("wrong-pass-backup-home");
    write_fixture_seed(&backup_home, SYSTEM_SEED_FILE, 0xCD);
    let backup_dir = scratch("wrong-pass-backup-dir");

    let mut session = Session::spawn(&backup_home, &["key", "backup", backup_dir.to_str().expect("utf8")]);
    session.expect("passphrase for this seed backup:", Duration::from_secs(15));
    session.send_line(A_PASSPHRASE);
    session.expect("type it again to confirm:", Duration::from_secs(10));
    session.send_line(A_PASSPHRASE);
    session.expect("file(s) written", Duration::from_secs(15));
    assert!(session.wait().success());

    let restore_home = scratch("wrong-pass-restore-home");
    let mut session = Session::spawn(&restore_home, &["key", "restore", backup_dir.to_str().expect("utf8")]);
    session.expect("passphrase for this seed backup:", Duration::from_secs(15));
    session.send_line("this is not the passphrase that was used");
    session.expect("wrong passphrase", Duration::from_secs(15));
    let status = session.wait();
    assert!(!status.success(), "a wrong passphrase must refuse, not succeed");
    assert!(
        !restore_home.join(".rivet").join("keys").join(SYSTEM_SEED_FILE).exists(),
        "nothing should have been written on a wrong passphrase"
    );

    eprintln!("--- wrong-passphrase transcript ---\n{}", session.transcript);
}

/// The crate's own scratch root — see the note in `tests/update.rs`.
#[path = "../src/scratch.rs"]
#[allow(dead_code)]
mod scratch;
