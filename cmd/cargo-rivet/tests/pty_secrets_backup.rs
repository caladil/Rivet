//! **`cargo rivet backup-secrets`/`restore-secrets`'s passphrase prompt, under a
//! REAL terminal, against a fake control plane over REAL TLS.** `.rulings/`
//! R-185 D1.
//!
//! `machine_secrets_tests.rs` proves the envelope, the archive and every
//! refusal with the passphrase handed in as a closure and the board as a fake
//! at the `CallsTools` seam. What it cannot prove is the path the shipped binary
//! takes: `rpassword` reading a real terminal with no echo, the prompt twice at
//! backup and once at restore, and the whole verb reaching a board over a
//! mutually-authenticated connection. This file is that proof, on
//! `pty_key_backup.rs`'s harness (`ADR-009` §3): `portable-pty` opens a real
//! ConPTY (Windows) or pty (elsewhere), spawns the real `cargo-rivet` binary
//! attached to it, and types what a person would.
//!
//! # The fake control plane
//!
//! A `rustls` server on a loopback port, presenting a THROWAWAY board identity
//! and admitting a THROWAWAY operator key — the pinning `rivet-mcpd` does
//! (`ADR-025` §2), from `rivetos_tls::server_config`. Behind it, a map standing
//! in for `/state/secrets`, answering the two calls these verbs make — `get
//! file` and `apply restore-secret` — in the JSON-RPC lines `rivet_remote`
//! reads. It is not `rivet-mcpd` and not `rivet mcp`; the QEMU proof
//! (`tools/qemu/adr-030-secrets-proof.sh`) is where the real board side runs.
//!
//! # Running it
//!
//! Gated with `#[ignore]`, as its siblings are: each test spawns real processes
//! under a real terminal.
//!
//! ```text
//! cargo test --release -p cargo-rivet --test pty_secrets_backup -- --ignored --test-threads=1
//! ```
//!
//! # Never the real `HOME`, never a real key
//!
//! Every process spawned here has `HOME`/`USERPROFILE` pointed at a scratch
//! directory, and every identity is minted by `cargo rivet identity new` into
//! one. Nothing here reads a key of anybody's.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use rivet_core::json::Json;

fn cargo_rivet_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cargo-rivet"))
}

fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    scratch::dir(&format!(
        "pty-secrets-{tag}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

// ---------------------------------------------------------------------------
// The terminal — `pty_key_backup.rs`'s `Session`, restated (each `tests/*.rs`
// file is its own compilation unit)
// ---------------------------------------------------------------------------

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
    fn spawn(home: &Path, args: &[String]) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize { rows: 40, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap_or_else(|e| panic!("openpty: {e}"));
        let mut cmd = CommandBuilder::new(cargo_rivet_exe());
        for a in args {
            cmd.arg(a);
        }
        cmd.cwd(home);
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
        // `pty_interview.rs`'s reason: dropping the ConPTY master can tear the
        // pseudo-console down before `wait()` reads the exit code.
        std::mem::forget(pair.master);
        Self { writer, transcript: String::new(), rx, child, answered_cursor_query: false }
    }

    fn answer_cursor_queries(&mut self) {
        if !self.answered_cursor_query && self.transcript.contains("\u{1b}[6n") {
            self.answered_cursor_query = true;
            let _ = self.writer.write_all(b"\x1b[40;1R");
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

    /// Wait for the `nth` (1-based) appearance of `pattern`.
    fn expect_nth(&mut self, pattern: &str, nth: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain();
            if self.transcript.matches(pattern).count() >= nth {
                return;
            }
            if Instant::now() >= deadline {
                panic!("timed out after {timeout:?} waiting for {pattern:?} #{nth}. Transcript:\n{}", self.transcript);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => {
                    self.transcript.push_str(&String::from_utf8_lossy(&chunk));
                    self.answer_cursor_queries();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.drain();
                    if self.transcript.matches(pattern).count() >= nth {
                        return;
                    }
                    panic!("the pty closed before {pattern:?} #{nth} appeared. Transcript:\n{}", self.transcript);
                }
            }
        }
    }

    fn expect(&mut self, pattern: &str, timeout: Duration) {
        self.expect_nth(pattern, 1, timeout);
    }

    fn wait(&mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            self.drain();
            if let Ok(Some(status)) = self.child.try_wait() {
                // The reader thread may still hold the last of the output.
                std::thread::sleep(Duration::from_millis(200));
                self.drain();
                return status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("cargo-rivet did not exit within 60s. Transcript:\n{}", self.transcript);
    }
}

// ---------------------------------------------------------------------------
// Throwaway identities
// ---------------------------------------------------------------------------

/// Mint an identity with the shipped binary into `dir` and return its key.
fn mint(dir: &Path, name: &str) -> String {
    let status = std::process::Command::new(cargo_rivet_exe())
        .args(["identity", "new", "--directory"])
        .arg(dir)
        .args(["--name", name])
        .env("HOME", dir)
        .env("USERPROFILE", dir)
        .output()
        .expect("run cargo-rivet identity new");
    assert!(status.status.success(), "identity new: {}", String::from_utf8_lossy(&status.stderr));
    let identity = rivetos_tls::read_identity(dir).expect("the minted identity");
    rivetos_tls::encode_hex(&identity.public_key().expect("a key"))
}

// ---------------------------------------------------------------------------
// The fake control plane
// ---------------------------------------------------------------------------

const STORE: &str = "/state/secrets";

#[derive(Default)]
struct BoardState {
    store: BTreeMap<String, Vec<u8>>,
    /// Every `tools/call`, as the words its arguments carried.
    calls: Vec<Vec<String>>,
}

impl BoardState {
    fn applies(&self) -> usize {
        self.calls.iter().filter(|w| w.first().map(String::as_str) == Some("restore-secret")).count()
    }

    fn secret_reads(&self) -> Vec<String> {
        self.calls
            .iter()
            .filter(|w| w.first().map(String::as_str) == Some("file"))
            .filter_map(|w| w.get(1).cloned())
            .filter(|p| p != STORE)
            .collect()
    }
}

fn words_of(arguments: &Json) -> Vec<String> {
    let Json::Object(fields) = arguments else { panic!("arguments are not an object: {arguments:?}") };
    let mut out = Vec::new();
    for (_, value) in fields {
        match value {
            Json::Str(s) => out.push(s.clone()),
            Json::Array(items) => out.extend(items.iter().filter_map(|i| i.as_str().map(str::to_string))),
            other => panic!("unexpected {other:?}"),
        }
    }
    out
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One operation's answer, in the shapes `rivet_core::operation`'s `file` and
/// `restore_secret` produce.
fn answer(state: &mut BoardState, tool: &str, words: &[String]) -> Result<String, String> {
    match (tool, words.first().map(String::as_str)) {
        ("rivet_get", Some("file")) => {
            let path = words.get(1).cloned().unwrap_or_default();
            if path == STORE {
                let entries: Vec<String> = state
                    .store
                    .iter()
                    .map(|(n, b)| format!(r#"{{"name":"{n}","type":"file","size_bytes":{}}}"#, b.len()))
                    .collect();
                return Ok(format!(
                    r#"{{"subject":"file","path":"{path}","present":true,"type":"directory","entry_count":{},"entries":[{}],"entries_truncated":false}}"#,
                    entries.len(),
                    entries.join(",")
                ));
            }
            let name = path.strip_prefix(&format!("{STORE}/")).unwrap_or_default();
            Ok(match state.store.get(name) {
                None => format!(r#"{{"subject":"file","path":"{path}","present":false}}"#),
                Some(bytes) => format!(
                    r#"{{"subject":"file","path":"{path}","present":true,"type":"file","size_bytes":{},"content":"{}","content_encoding":"base64"}}"#,
                    bytes.len(),
                    rivet_core::base64::encode(bytes)
                ),
            })
        }
        ("rivet_apply", Some("restore-secret")) => {
            let name = words.get(1).cloned().unwrap_or_default();
            let bytes = rivet_core::secret::bytes_from_hex(words.get(3).map_or("", String::as_str))
                .map_err(|_| "the fake board could not decode the hex".to_string())?;
            let len = bytes.len();
            state.store.insert(name.clone(), bytes);
            Ok(format!(r#"{{"subject":"restore-secret","name":"{name}","bytes":{len}}}"#))
        }
        _ => Err(format!("the fake board does not answer {tool} {words:?}")),
    }
}

/// Serve `state` on a loopback port as `board`, admitting `operator`. Runs until
/// the test process ends.
fn serve(board: rivetos_tls::Identity, operator: rivetos_tls::Pin, state: Arc<Mutex<BoardState>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let host = listener.local_addr().expect("an address").to_string();
    let config = Arc::new(
        rivetos_tls::server_config(&board, vec![operator], rivetos_tls::provider()).expect("a server config"),
    );
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { continue };
            let Ok(connection) = rustls::ServerConnection::new(Arc::clone(&config)) else { continue };
            let mut stream = rustls::StreamOwned::new(connection, socket);
            let mut request = Vec::new();
            if stream.read_to_end(&mut request).is_err() {
                continue;
            }
            let mut reply = String::new();
            for line in String::from_utf8_lossy(&request).lines() {
                let Ok(json) = rivet_core::json::parse(line) else { continue };
                if json.get("method").and_then(Json::as_str) != Some("tools/call") {
                    continue;
                }
                let id = json.get("id").and_then(Json::as_integer).unwrap_or(0);
                let params = json.get("params").expect("params");
                let tool = params.get("name").and_then(Json::as_str).unwrap_or_default().to_string();
                let words = words_of(params.get("arguments").expect("arguments"));
                let mut state = state.lock().expect("the board");
                state.calls.push(words.clone());
                let (text, is_error) = match answer(&mut state, &tool, &words) {
                    Ok(text) => (text, false),
                    Err(text) => (text, true),
                };
                reply.push_str(&format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"type":"text","text":{}}}],"isError":{is_error}}}}}"#,
                    json_string(&text)
                ));
                reply.push('\n');
            }
            let _ = stream.write_all(reply.as_bytes());
            stream.conn.send_close_notify();
            let _ = stream.flush();
        }
    });
    host
}

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

const A_PASSPHRASE: &str = "pty-proof secrets passphrase, fictional and fixed";
const A_PSK: &[u8] = b"70726f62652d7073652d76616c7565";

struct Fixture {
    home: PathBuf,
    operator_dir: PathBuf,
    board_key: String,
    host: String,
    state: Arc<Mutex<BoardState>>,
}

fn fixture(tag: &str) -> Fixture {
    let home = scratch(tag);
    let board_dir = home.join("board");
    let operator_dir = home.join("operator");
    let board_key = mint(&board_dir, "pty-board");
    let operator_key = mint(&operator_dir, "pty-operator");

    let mut state = BoardState::default();
    for name in [rivetos_tls::SEED_FILE, rivetos_tls::CERTIFICATE_FILE] {
        state.store.insert(name.to_string(), std::fs::read(board_dir.join(name)).expect(name));
    }
    state.store.insert("peers".to_string(), format!("# the operator\n{operator_key}\n").into_bytes());
    state.store.insert("wlan0.psk".to_string(), A_PSK.to_vec());
    let state = Arc::new(Mutex::new(state));

    let identity = rivetos_tls::read_identity(&board_dir).expect("the board identity");
    let operator = rivetos_tls::read_identity(&operator_dir)
        .expect("the operator identity")
        .public_key()
        .expect("a key");
    let host = serve(identity, operator, Arc::clone(&state));
    Fixture { home, operator_dir, board_key, host, state }
}

impl Fixture {
    fn args(&self, verb: &str, dir_flag: &str, dir: &Path, extra: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = [
            verb,
            "--host",
            &self.host,
            "--identity",
            &self.operator_dir.display().to_string(),
            "--pin",
            &self.board_key,
            dir_flag,
            &dir.display().to_string(),
        ]
        .iter()
        .map(|w| (*w).to_string())
        .collect();
        out.extend(extra.iter().map(|w| (*w).to_string()));
        out
    }

    fn backup(&self, into: &Path) -> Session {
        let mut session = Session::spawn(&self.home, &self.args("backup-secrets", "--into", into, &["confirm"]));
        session.expect("FORGOTTEN PASSPHRASE", Duration::from_secs(20));
        session.expect("passphrase for this secrets backup:", Duration::from_secs(20));
        session.send_line(A_PASSPHRASE);
        session.expect("type it again to confirm:", Duration::from_secs(20));
        session.send_line(A_PASSPHRASE);
        session
    }
}

// ---------------------------------------------------------------------------
// The proofs
// ---------------------------------------------------------------------------

/// **The whole round trip with real typed passphrases**: back up with the
/// passphrase typed twice; lose a file on the board; restore with the
/// passphrase typed once; the board holds the original bytes again.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc"]
fn a_typed_passphrase_backs_up_and_restores_a_boards_secrets() {
    let f = fixture("round-trip");
    let into = f.home.join("backup");

    let mut session = f.backup(&into);
    session.expect("backed up 4 file(s)", Duration::from_secs(60));
    let status = session.wait();
    assert!(status.success(), "backup-secrets should have succeeded:\n{}", session.transcript);
    assert!(!session.transcript.contains(A_PASSPHRASE), "the passphrase was echoed:\n{}", session.transcript);

    let sealed = std::fs::read(into.join("secrets-backup.enc")).expect("the envelope");
    assert_eq!(&sealed[..7], b"RVTSECR");
    assert!(!sealed.windows(A_PSK.len()).any(|w| w == A_PSK), "the psk is readable in the envelope");
    assert!(!into.join("files").exists() && !into.join("secrets-backup.txt").exists());

    // Lose a secret on the board.
    f.state.lock().expect("the board").store.remove("wlan0.psk");
    let applies_before = f.state.lock().expect("the board").applies();

    let mut session = Session::spawn(&f.home, &f.args("restore-secrets", "--from", &into, &["confirm"]));
    session.expect("passphrase for this secrets backup:", Duration::from_secs(20));
    session.send_line(A_PASSPHRASE);
    session.expect("read back identical", Duration::from_secs(60));
    let status = session.wait();
    assert!(status.success(), "restore-secrets should have succeeded:\n{}", session.transcript);

    let state = f.state.lock().expect("the board");
    assert_eq!(state.store.get("wlan0.psk").map(Vec::as_slice), Some(A_PSK));
    assert_eq!(state.applies(), applies_before + 1, "only the lost file is written");
    eprintln!("--- restore transcript ---\n{}", session.transcript);
}

/// **A wrong typed passphrase at restore is refused, and nothing is sent to
/// the board** — not a read of what it holds, not a write.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc"]
fn a_wrong_typed_passphrase_at_restore_sends_nothing() {
    let f = fixture("wrong");
    let into = f.home.join("backup");
    let mut session = f.backup(&into);
    session.expect("backed up", Duration::from_secs(60));
    assert!(session.wait().success(), "{}", session.transcript);

    let calls_before = f.state.lock().expect("the board").calls.len();
    let mut session = Session::spawn(&f.home, &f.args("restore-secrets", "--from", &into, &["--replace", "confirm"]));
    session.expect("passphrase for this secrets backup:", Duration::from_secs(20));
    session.send_line("this is not the passphrase that sealed it");
    session.expect("wrong passphrase", Duration::from_secs(30));
    let status = session.wait();
    assert!(!status.success(), "a wrong passphrase must refuse:\n{}", session.transcript);
    assert!(session.transcript.contains("NOTHING WAS SENT"), "{}", session.transcript);
    assert_eq!(f.state.lock().expect("the board").calls.len(), calls_before, "the board was asked something");
}

/// **Two entries that differ at backup refuse, write nothing, and fetch no
/// secret** — only the listing and the public certificate were read.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc"]
fn a_mistyped_confirmation_at_backup_writes_nothing_and_fetches_no_secret() {
    let f = fixture("mistyped");
    let into = f.home.join("backup");
    let mut session = Session::spawn(&f.home, &f.args("backup-secrets", "--into", &into, &["confirm"]));
    session.expect("passphrase for this secrets backup:", Duration::from_secs(20));
    session.send_line(A_PASSPHRASE);
    session.expect("type it again to confirm:", Duration::from_secs(20));
    session.send_line("something else entirely");
    session.expect("did not match", Duration::from_secs(20));
    let status = session.wait();
    assert!(!status.success(), "{}", session.transcript);
    assert!(!into.exists(), "something was written");
    assert_eq!(
        f.state.lock().expect("the board").secret_reads(),
        vec![format!("{STORE}/{}", rivetos_tls::CERTIFICATE_FILE)],
        "a secret crossed the wire before a passphrase existed"
    );
}

/// The crate's own scratch root — see the note in `tests/update.rs`.
#[path = "../src/scratch.rs"]
#[allow(dead_code)]
mod scratch;
