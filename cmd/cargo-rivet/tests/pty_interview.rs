//! **R-178 P6 — the interactive interview, proven in a REAL terminal.**
//!
//! `tools/provision/rehearse-menu.sh` already drives `cmd/rivet-provision`'s
//! own menu under a pty, but only through `script -q -c`, a POSIX utility
//! with no Windows counterpart — `SPEC-05` names the platform this project
//! is developed on, and `.rulings/` R-178 P6 asks for a harness proven
//! there, not only in the Linux build container. This file is that harness:
//! `portable-pty` opens a real ConPTY (Windows) or pty (everywhere else —
//! `native_pty_system()` picks the backend), spawns the real
//! `cargo rivet provision` binary attached to it — never piped stdin, which
//! `std::io::IsTerminal` would see as no terminal at all, and
//! `write_image.rs::refuse_unless_terminal` exists precisely to refuse — and
//! types the keystrokes a person would.
//!
//! `ADR-009` §3 (its own "fourth worked survey" subsection) records the
//! dependency survey; `cmd/cargo-rivet/Cargo.toml`'s own
//! comment beside `portable-pty` is the short form.
//!
//! # What this proves, and what it cannot
//!
//! **No test here ever writes a device.** The bare interactive run offers the
//! removable devices this host enumerates (2026-09-14, `.rulings/` R-174), so
//! the happy path meets one of two endings: no removable device at all, or the
//! device menu, where it picks `1` and answers `N` at `Write it? [y/N]` —
//! before any administrator prompt exists. Either way it asserts that nothing
//! was recorded and that the rerun command is named. `--device` only accepts
//! a path this host's own disk enumeration vouches for
//! (`cmd/cargo-rivet/src/enumerate.rs`), so there is no file or scratch path a
//! test could hand it, and writing a real disk is out of bounds in any case.
//! **The summary (`ADR-054` §7) is consequently unproven by this
//! harness** — it prints only after a verified device write — and stays
//! proven the way it already was: by the fake-writer unit tests in
//! `cmd/cargo-rivet/src/provision_tests.rs`. See this file's own module-level
//! `README` note in each test for what IS proven instead.
//!
//! # Running it
//!
//! **Gated with `#[ignore]`, the same convention `cmd/rivet-deploy/src/bench.rs`
//! and `cmd/rivet-image/src/disk.rs`'s determinism test already use for a
//! test that is slow, spawns real processes, or needs something `cargo test`
//! cannot assume** — here, a terminal `cargo test`'s own harness may not have
//! one, and each test spawns two real binaries and a handful of `cargo rivet`
//! setup calls.
//!
//! ```text
//! cargo build --release -p cargo-rivet
//! cargo build --release -p rivetos-provision
//! cargo test --release -p cargo-rivet --test pty_interview -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! The release build matters for more than speed: `SubprocessInterview`
//! (`cmd/cargo-rivet/src/provision.rs`) looks for `rivet-provision` BESIDE
//! `cargo-rivet`'s own binary first, and only falls back to `cargo run` —
//! slow, and a second compilation this harness does not need — when that
//! sibling is absent. Building both in the same profile first makes every
//! run here fast and deterministic.
//!
//! **Container:** this should also run under
//! `tools/container/exec.sh cargo test --release -p cargo-rivet --test
//! pty_interview -- --ignored --test-threads=1 --nocapture` — `native_pty_system()`
//! opens a Unix pty there — but it was not run there in this session; only
//! Windows was, which is `SPEC-05`'s own platform and the one this proof was
//! ruled for. Say so rather than claiming both.
//!
//! # If a real terminal cannot be driven here
//!
//! Each test fails LOUDLY rather than falling back to a pipe: `openpty`
//! failing, the binaries being absent, or an `expect` timing out are all
//! `panic!`s that name what did not happen, never a silent skip that would
//! make this pass while proving nothing (`AGENTS.md` §7).
//!
//! # Nothing under the real home, 2026-09-14
//!
//! **This harness is what found the defect it now proves is fixed.** The
//! first version, before `Session::spawn_provision` redirected `HOME`/
//! `USERPROFILE` at all, surfaced `resume which?` naming a REAL saved
//! notebook from this machine's real operator history before a single
//! keystroke was sent — `cargo rivet provision --at DIR` set
//! `RIVET_FLEET_REPOSITORY` on the interview and nothing else, so its CA,
//! its saved-answers notebook and the breadcrumb it wrote all came straight
//! off this host's real `HOME`/`USERPROFILE`. The workaround at the time was
//! to redirect `HOME`/`USERPROFILE` to a scratch directory by hand and keep
//! it in lock-step with `--at` — fragile, and true only because this
//! harness kept the two in agreement on purpose.
//!
//! `cmd/cargo-rivet/src/provision.rs`'s own "2026-09-14" module-doc section
//! is the real fix: `--at DIR` now reaches the interview (`RIVET_AT`, set on
//! its child environment) and the register
//! (`crate::provision::register_root_for`) directly, so `HOME`/
//! `USERPROFILE` no longer need to be redirected to anywhere REAL at all.
//! `Session::spawn_provision` now points them at `Fixtures::sentinel_home` —
//! a directory that must never come to exist — and `sentinel_untouched`,
//! called at the end of every test below, asserts exactly that: proof, not
//! just an assumption, that nothing under the real home is consulted any
//! more.

// A test that cannot fail loudly is not a test — `provision_tests.rs`'s own
// header, restated: every `expect`/`panic!` below is deliberate, and this is
// what a real terminal not answering as expected is supposed to look like.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
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
        "pty-interview-{tag}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Run a `cargo-rivet` subcommand to completion with NO pty — the
/// non-interactive setup steps (`setup`, `fleet init`, `package`,
/// `fleet publish`, `image`) need none of this file's own machinery, only
/// `real_end_to_end`'s ordinary `std::process::Command`.
fn run_ok(args: &[&str]) -> String {
    let out = Command::new(cargo_rivet_exe())
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("spawn cargo-rivet {args:?}: {e}"));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "cargo-rivet {args:?} failed:\n{combined}");
    combined
}

/// The newest real `*-card.img` this checkout happens to have in `out/` —
/// `cmd/cargo-rivet/src/provision_tests.rs`'s own `real_end_to_end` convention,
/// restated here rather than shared, because that module is not `pub` and
/// this file is a separate compilation unit (an integration test, `tests/`).
///
/// `None` (and every test here reports SKIPPED) when nothing has built one —
/// this harness proves the INTERVIEW, not the card build, and reuses
/// whatever image is already on disk exactly as `real_end_to_end` does.
fn newest_card_image() -> Option<PathBuf> {
    let dir = repo_root().join("out");
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with("-card.img"))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    found.sort_by_key(|(when, _)| *when);
    found.pop().map(|(_, p)| p)
}

/// A scratch `--at` root and fleet repository, both freshly minted with the
/// real `cargo rivet setup`/`fleet init` — never faked, on the same posture
/// `provision_tests.rs::real_end_to_end` already takes for the byte-for-byte
/// proof.
struct Fixtures {
    /// The `--at` value for every `cargo-rivet` command below — `setup`,
    /// `package`, `fleet publish`, `image`, and this harness's own `--at` on
    /// `provision`. `cargo-rivet` treats `--at` as the credentials root
    /// directly.
    ///
    /// **`cmd/rivet-provision` now reads the identical directory**, via
    /// `RIVET_AT` (`cmd/cargo-rivet/src/provision.rs`'s `SubprocessInterview`
    /// sets it on the child, to the same `at` this process already resolved
    /// for itself) — the 2026-09-14 fix this harness exists to prove. Before
    /// it, the interview read `HOME`/`USERPROFILE` directly instead, and the
    /// two had to be kept in agreement by hand (`home.join(".rivet")`,
    /// `HOME` pointed at `home`); see `sentinel_home`'s own doc for how this
    /// harness now checks that route is well and truly gone.
    at: PathBuf,
    /// **A directory that must NEVER come to exist.** `Session::spawn_provision`
    /// points `HOME`/`USERPROFILE` here instead of anywhere real, so a
    /// regression that starts reading either of them again — the exact
    /// defect this harness caught the first time it ran — has to make this
    /// directory spring into existence for that read (or, worse, a write) to
    /// succeed. `sentinel_untouched` asserts it never does. Deliberately
    /// never created by `scratch()`, which `create_dir_all`s its own
    /// argument — creating this one would defeat the guard.
    sentinel_home: PathBuf,
    fleet: PathBuf,
}

/// A path that must never be created — see [`Fixtures::sentinel_home`].
fn sentinel_path(tag: &str) -> PathBuf {
    scratch::absent(&format!(
        "pty-interview-{tag}-REAL-HOME-MUST-NOT-BE-TOUCHED"
    ))
}

/// [`Fixtures::sentinel_home`] is still absent — the harness's own proof
/// that nothing under the (fake, poisoned) `HOME`/`USERPROFILE` was ever
/// read or written by the run just finished.
///
/// **One exception, and it is not Rivet's.** The device menu the bare
/// interactive run now offers (2026-09-14) enumerates disks through Windows
/// PowerShell, and PowerShell itself creates an empty `AppData\Roaming` under
/// whatever `USERPROFILE` it inherits — measured by running it alone with
/// `USERPROFILE` pointed at a fresh directory. That empty pair of directories,
/// and nothing else, is tolerated: any file, or any other directory, still
/// fails.
fn sentinel_untouched(fixtures: &Fixtures) {
    let home = &fixtures.sentinel_home;
    let only_powershells_empty_appdata = home.exists() && {
        let roaming = home.join("AppData").join("Roaming");
        let names = |dir: &Path| -> Vec<std::ffi::OsString> {
            std::fs::read_dir(dir).map(|it| it.filter_map(Result::ok).map(|e| e.file_name()).collect()).unwrap_or_default()
        };
        names(home) == ["AppData"] && names(&home.join("AppData")) == ["Roaming"] && roaming.is_dir() && names(&roaming).is_empty()
    };
    assert!(
        !home.exists() || only_powershells_empty_appdata,
        "{} was created -- something read or wrote through HOME/USERPROFILE \
         again instead of RIVET_AT/--at",
        fixtures.sentinel_home.display()
    );
}

fn setup_fixtures(tag: &str) -> Fixtures {
    let at = scratch(&format!("{tag}-at"));
    let fleet = scratch(&format!("{tag}-fleet"));
    run_ok(&["setup", "--at", at.to_str().expect("utf8"), "--name", "pty-proof-operator"]);
    run_ok(&["fleet", "init", "--at", fleet.to_str().expect("utf8")]);
    Fixtures { at, sentinel_home: sentinel_path(tag), fleet }
}

/// Package a fictional application, signed with `fixtures`' own scratch app
/// seed, and publish it into `fixtures`' own fleet repository —
/// `cmd/cargo-rivet/tests/roundtrip.rs`'s `--binary` shape, so this needs no
/// cross toolchain: `bytes` stands in for a real program (never read as an
/// ELF, never executed), and two calls with different `bytes` mint two
/// genuinely different, really signed versions of the SAME product name.
fn publish_fictional_application(fixtures: &Fixtures, product: &str, crate_version: &str, bytes: &[u8]) {
    let dir = scratch(&format!("{product}-{}", bytes.len()));
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]\n\n[package]\nname = \"{product}\"\nversion = \"{crate_version}\"\nedition = \"2024\"\n\
             publish = false\n\n[package.metadata.rivet]\nkind = \"daemon\"\nrestart = \"on-failure\"\n\
             ready = \"listening\"\nnetwork = \"group:media\"\nstate = \"private\"\nmemory = \"128M\"\n\
             listen = [\"tcp:8080\"]\n"
        ),
    )
    .expect("write a fixture Cargo.toml");
    std::fs::create_dir_all(dir.join("src")).expect("src/");
    std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").expect("write a fixture main.rs");
    let binary = dir.join("fake.bin");
    std::fs::write(&binary, bytes).expect("write the fixture payload");

    run_ok(&[
        "package",
        "--manifest-path",
        dir.join("Cargo.toml").to_str().expect("utf8"),
        "--binary",
        binary.to_str().expect("utf8"),
        "--signing-key",
        fixtures.at.join("keys").join("rivet-app.seed").to_str().expect("utf8"),
        "--out-dir",
        dir.to_str().expect("utf8"),
    ]);
    let rvt = std::fs::read_dir(&dir)
        .expect("read the fixture dir back")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "rvt"))
        .unwrap_or_else(|| panic!("no .rvt in {}", dir.display()));
    run_ok(&[
        "fleet",
        "publish",
        rvt.to_str().expect("utf8"),
        "--trusted-key",
        fixtures.at.join("keys").join("rivet-app.pub").to_str().expect("utf8"),
        "--at",
        fixtures.fleet.to_str().expect("utf8"),
    ]);
}

/// Publish `image` into `fixtures`' fleet repository under `version` —
/// republishing the SAME underlying `.img` bytes under two different
/// `--version` strings is how this harness gets two genuinely distinct,
/// really signed `card` entries to offer the image-selection MENU without a
/// second, multi-minute `tools/container/build-card.sh` run.
fn publish_card_image(fixtures: &Fixtures, image: &Path, version: &str) {
    run_ok(&[
        "image",
        "--img",
        image.to_str().expect("utf8"),
        "--version",
        version,
        "--arch",
        "aarch64",
        "--platform",
        "rpi4",
        "--signing-key",
        fixtures.at.join("keys").join("rivet-system.seed").to_str().expect("utf8"),
        "--at",
        fixtures.fleet.to_str().expect("utf8"),
    ]);
}

// ---------------------------------------------------------------------------
// The pty session itself
// ---------------------------------------------------------------------------

/// A running `cargo rivet provision`, attached to a real pty, with everything
/// this file's `expect`/`send_line` need to drive it like a person typing.
struct Session {
    writer: Box<dyn Write + Send>,
    /// Every byte read so far, lossily decoded — ANSI colour codes and all.
    /// `expect` greps this rather than the raw bytes, which is enough: the
    /// screen renderer wraps colour AROUND a prompt's words, never inside
    /// one, so a plain substring search still finds them.
    transcript: String,
    /// New chunks arrive here from the reader thread; `expect` drains it.
    rx: mpsc::Receiver<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// **ConPTY's own handshake.** On Windows, `conhost`/ConPTY queries the
    /// terminal's cursor position (`ESC[6n`, a DSR request) as part of
    /// bringing the pseudo-console up, and a real terminal answers it
    /// (`ESC[row;colR`) without the user doing anything. Nothing here reads
    /// column/row for real — any plausible position works, since nothing the
    /// interview prints depends on it — but leaving the query unanswered
    /// left the child's own output stalled indefinitely the first time this
    /// harness ran, which is what this field and `answer_cursor_queries`
    /// exist to fix.
    answered_cursor_query: bool,
}

impl Drop for Session {
    /// **Never leave a child running past its test.** A panic inside one of
    /// this file's tests (a timed-out `expect`, a failed assertion) unwinds
    /// past every `session.wait()` below it — without this, that leaves a
    /// real `cargo-rivet.exe`/`rivet-provision.exe` pair attached to a pty
    /// nothing is reading any more, blocked on a prompt nobody will ever
    /// answer, for the rest of this test binary's run.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl Session {
    /// Spawn `cargo rivet provision` (never `cmd/rivet-provision` directly —
    /// this proves the real orchestrator a person types, which launches the
    /// interview as its own child) attached to a fresh pty.
    fn spawn_provision(fixtures: &Fixtures, extra_args: &[&str]) -> Self {
        Self::spawn_provision_from(fixtures, extra_args, &cargo_rivet_exe(), &repo_root())
    }

    /// [`Self::spawn_provision`] with the BINARY and the WORKING DIRECTORY
    /// chosen by the caller — `.rulings/` R-191/R-192's own two variables.
    ///
    /// `cargo-rivet`'s own directory decides whether a sibling
    /// `rivet-provision` is found (a `cargo install`ed pair has one; a
    /// scratch directory holding a lone copy of `cargo-rivet` does not), and
    /// the working directory decides whether the `cargo run` fallback has a
    /// checkout to build from. Between them they select which of
    /// `provision.rs`'s three interview sources a run takes, which is the
    /// whole subject of `an_installed_pair_launches_the_interview_from_
    /// anywhere_and_a_lone_cargo_rivet_falls_back_to_cargo_run`.
    fn spawn_provision_from(
        fixtures: &Fixtures,
        extra_args: &[&str],
        exe: &Path,
        cwd: &Path,
    ) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            // **Wide on purpose.** The pseudo-console wraps at `cols`, and the
            // rerun command a declined write names is one long line this
            // harness reads back whole.
            .openpty(PtySize { rows: 40, cols: 1000, pixel_width: 0, pixel_height: 0 })
            .unwrap_or_else(|e| panic!("openpty: {e}"));

        let mut cmd = CommandBuilder::new(exe);
        cmd.arg("provision");
        cmd.arg("--at");
        cmd.arg(&fixtures.at);
        // **No `--not-mine`, since the gate rehearsal of 2026-09-14.** This
        // harness passed it, and so masked the defect that refused every real
        // provision of a generic published image ("carries NO peers file")
        // after the interview. The card's `peers` now comes from the identity
        // this run injects, which names this fixture's own operator, so the
        // run must pass exactly as the maintainer types it.
        for a in extra_args {
            cmd.arg(a);
        }
        cmd.cwd(cwd);
        cmd.env("RIVET_FLEET_REPOSITORY", &fixtures.fleet);
        // **`--at DIR` alone now governs the whole run — the 2026-09-14 fix
        // this harness exists to prove.** `SubprocessInterview` sets
        // `RIVET_AT` on the child to the same `at` this orchestrator already
        // resolved for itself, and `cmd/rivet-provision`'s own
        // `authority_directory`/`save::directory` (its CA, its saved-answers
        // notebook, the document and the staging bundle it writes, the
        // breadcrumb) all read `RIVET_AT` first now instead of `HOME`/
        // `USERPROFILE` directly. The register follows `--at` too
        // (`crate::provision::register_root_for`, keyed off the literal flag
        // rather than a config lookup) — R-178 Q7's enrollment check no
        // longer touches `ignition::config`/`%APPDATA%` at all on this
        // route, so there is no register-specific override left to set
        // either.
        //
        // **The guard**, restated from `tools/provision/rehearse-menu.sh`'s
        // own ("HOME is moved, and that is the whole safety story"): `HOME`/
        // `USERPROFILE` point at a directory that must never come to exist
        // (`Fixtures::sentinel_home`) rather than at a real, usable scratch
        // directory — so if anything in this run regresses and starts
        // reading or writing through them again, that directory has to
        // spring into existence for the read/write to succeed, and
        // `sentinel_untouched` (called after every run below) asserts it
        // never does.
        cmd.env("HOME", &fixtures.sentinel_home);
        cmd.env("USERPROFILE", &fixtures.sentinel_home);

        let child =
            pair.slave.spawn_command(cmd).unwrap_or_else(|e| panic!("spawn cargo rivet provision: {e}"));
        // The slave handle is only needed to spawn; dropping it here (by not
        // storing it) matches `portable_pty`'s own examples and avoids
        // holding an extra fd/handle open for a process this harness does
        // not otherwise touch through it.
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
        // `pair.master` itself is intentionally leaked past this function —
        // on Windows, dropping the ConPTY's master handle can tear down the
        // pseudo-console (and so the child's console) before `wait()` below
        // has a chance to read its exit code. `Session` never needs it
        // again once `writer`/`reader` are taken.
        std::mem::forget(pair.master);

        Self { writer, transcript: String::new(), rx, child, answered_cursor_query: false }
    }

    /// Answer ConPTY's own `ESC[6n` cursor-position query the first time it
    /// appears — see [`Self::answered_cursor_query`]'s own doc for why this
    /// exists at all. A fixed `ESC[24;1R` (row 24, column 1) is handed back;
    /// nothing here reads it.
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

    /// Type `text` followed by Enter — a real terminal sends CR, and ConPTY
    /// (like every pty) translates it into the newline the child's
    /// `read_line` blocks on.
    fn send_line(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).expect("write to the pty");
        self.writer.write_all(b"\r\n").expect("write CRLF to the pty");
        self.writer.flush().ok();
    }

    /// Drain whatever the reader thread has produced so far into
    /// `transcript`, without blocking.
    fn drain(&mut self) {
        while let Ok(chunk) = self.rx.try_recv() {
            self.transcript.push_str(&String::from_utf8_lossy(&chunk));
        }
        self.answer_cursor_queries();
    }

    /// Block until `pattern` has appeared anywhere in the transcript, or
    /// `timeout` has passed — whichever comes first. Panics on timeout,
    /// printing the whole transcript so a failure is diagnosable rather than
    /// a bare "timed out".
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
                    // The child closed its output. One more drain in case
                    // the pattern arrived in the final chunk, then give up.
                    self.drain();
                    if self.transcript.contains(pattern) {
                        return;
                    }
                    panic!(
                        "the pty closed before {pattern:?} ever appeared. Transcript:\n{}",
                        self.transcript
                    );
                }
            }
        }
    }

    /// [`Self::expect`] for whichever of `patterns` appears first — its index.
    fn expect_any(&mut self, patterns: &[&str], timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain();
            if let Some(index) = patterns.iter().position(|p| self.transcript.contains(p)) {
                return index;
            }
            if Instant::now() >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for any of {patterns:?}. Transcript so far:\n{}",
                    self.transcript
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Wait for the child to exit, draining any remaining output first.
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
        panic!("cargo rivet provision did not exit within 30s. Transcript:\n{}", self.transcript);
    }
}

// ---------------------------------------------------------------------------
// The three proofs
// ---------------------------------------------------------------------------

/// **Preflight and image selection run, and refuse, BEFORE the interview is
/// ever launched** — `SPEC-05` §4's own ordering (step 2 before step 3),
/// R-178. An
/// empty fleet repository (`fleet init`, nothing published) must be refused
/// by the exact sentence `provision_image::NO_IMAGES_REFUSAL` names, with NO
/// interview question ever appearing — asserted by the ABSENCE of the
/// first-provision question (`new fleet, or join one?`) `cmd/rivet-provision`'s
/// own `ensure_authority` would otherwise print first.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc for how to run it"]
fn an_empty_fleet_repository_is_refused_before_any_question_is_asked() {
    let fixtures = setup_fixtures("empty-repo");
    let mut session = Session::spawn_provision(&fixtures, &[]);
    session.expect("[1/3] listing images for this platform", Duration::from_secs(20));
    session.expect(
        "No images available for provisioning. Deploy a system image to your artifact repository \
         prior to provisioning.",
        Duration::from_secs(10),
    );
    let status = session.wait();
    assert!(!status.success(), "an empty fleet repository must refuse, not succeed");
    assert!(
        !session.transcript.contains("join one?"),
        "the CA prompt must never appear -- image selection refuses first:\n{}",
        session.transcript
    );
    sentinel_untouched(&fixtures);
    eprintln!("--- transcript ---\n{}", session.transcript);
}

/// **R-178 P2 — a schema-mismatched image is marked and skipped, not
/// silently offered.** One published `card` entry, hand-edited to claim a
/// schema version this build does not speak (the manifest is a text file
/// `cargo rivet fleet publish` writes; editing one field of it to test a
/// mismatch this checkout's own schema will never naturally have is the same
/// fixture shape `cmd/cargo-rivet/src/fleet_tests.rs` and `carry_tests.rs`
/// already use for their own hand-built manifests). With the only image
/// blocked, nothing is selectable and the run refuses by name, naming BOTH
/// the platform and the schema each image needs.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc for how to run it"]
fn a_schema_mismatched_image_is_marked_and_the_run_is_refused() {
    let Some(image) = newest_card_image() else {
        eprintln!("SKIPPED: no *-card.img in out/ -- see real_end_to_end in provision_tests.rs for how one gets there");
        return;
    };
    let fixtures = setup_fixtures("schema-mismatch");
    publish_card_image(&fixtures, &image, "1.0.0-pty-mismatch");

    let manifest = fixtures.fleet.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).expect("read the manifest back");
    assert!(text.contains("schema_version = "), "{text}");
    let edited = text.replacen("schema_version = 1", "schema_version = 999999", 1);
    assert_ne!(text, edited, "the schema_version line was not found to edit:\n{text}");
    std::fs::write(&manifest, edited).expect("write the edited manifest");

    let mut session = Session::spawn_provision(&fixtures, &[]);
    session.expect("[1/3] listing images for this platform", Duration::from_secs(20));
    session.expect("is not selectable: needs schema 999999, this tool speaks", Duration::from_secs(10));
    session.expect("every image the fleet repository holds for it needs a different schema", Duration::from_secs(10));
    let status = session.wait();
    assert!(!status.success(), "a fleet repository with only a mismatched image must refuse");
    sentinel_untouched(&fixtures);
    eprintln!("--- transcript ---\n{}", session.transcript);
}

/// **The full happy path, real keystrokes throughout**: TWO published card
/// images force the image-selection MENU (answered with a bare Return, the
/// R-176 Q6 "newest preselected" default); the interview mints a fleet
/// authority, takes a hostname and `Fleet = server`; TWO published versions
/// of `rivet-fleetd` force the application-version MENU (answered with an
/// explicit, non-default `2`, proving a typed pick works and not only the
/// default); `rivet-proxyd` is enabled and not carried, because it is part of
/// the image (`ADR-060`); `w` composes, mints the certificate, writes the document,
/// and this checks the SAME parser a board refuses with read it back —
/// every one of these is a real, distinguishable line this test asserts on,
/// not an inferred success from a zero exit code.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc for how to run it"]
fn the_interview_selects_an_image_picks_a_carried_version_and_writes_the_document() {
    let Some(image) = newest_card_image() else {
        eprintln!("SKIPPED: no *-card.img in out/ -- see real_end_to_end in provision_tests.rs for how one gets there");
        return;
    };
    let fixtures = setup_fixtures("happy-path");
    // Two really-signed, really-distinct card entries -- forces the image menu.
    publish_card_image(&fixtures, &image, "1.0.0-pty-a");
    publish_card_image(&fixtures, &image, "2.0.0-pty-b");
    // Two really-signed, really-distinct rivet-fleetd payloads -- forces the
    // application-version menu. No rivet-proxyd: the image provides it
    // (`ADR-060`), and publishing one is refused.
    publish_fictional_application(&fixtures, "rivet-fleetd", "0.1.0", &[1u8; 4096]);
    publish_fictional_application(&fixtures, "rivet-fleetd", "0.1.1", &[2u8; 4200]);

    let host = "example-pty-server";
    let mut session = Session::spawn_provision(&fixtures, &[]);

    // --- stage 1: image selection, a real menu, answered with the default --
    session.expect("[1/3] listing images for this platform", Duration::from_secs(20));
    session.expect("system image:", Duration::from_secs(10));
    session.expect("(default)", Duration::from_secs(5));
    session.expect("choose 1-2 [1]:", Duration::from_secs(5));
    session.send_line(""); // bare Return -- the newest, preselected
    session.expect("[1/3] done", Duration::from_secs(15));
    session.expect("[2/3] verifying and decompressing", Duration::from_secs(10));
    session.expect("[2/3] done", Duration::from_secs(60));
    session.expect("[3/3] running the interview", Duration::from_secs(10));

    // --- stage 2: cmd/rivet-provision's own authority + question menu -------
    session.expect("new fleet, or join one?", Duration::from_secs(15));
    session.send_line("1"); // a new fleet: mint a fresh authority in this scratch HOME
    session.expect("fleet", Duration::from_secs(10)); // the fleet-name prompt
    session.send_line("pty-proof-fleet");
    session.expect("suffix", Duration::from_secs(10));
    session.send_line("local");
    session.expect("keep these?", Duration::from_secs(10));
    session.send_line("y");
    session.expect("operator", Duration::from_secs(10));
    session.send_line("PTY Proof Operator");

    session.expect("Hostname", Duration::from_secs(15));
    session.send_line("1"); // edit field 1, Hostname
    session.expect(">", Duration::from_secs(10));
    session.send_line(host);
    session.expect("Fleet", Duration::from_secs(10));
    session.send_line("6"); // edit field 6, Fleet
    session.expect("Fleet", Duration::from_secs(10));
    session.send_line("2"); // server

    // **The timezone is REQUIRED** (`SPEC-05` §5) -- `w` refuses to write
    // while it is unanswered, so this is not optional scaffolding.
    session.expect("Timezone", Duration::from_secs(10));
    session.send_line("7"); // edit field 7, Timezone
    session.expect(">", Duration::from_secs(10));
    session.send_line("America/Chicago");

    // --- stage 3: `w` -- and R-178 P3's own version menu, before anything
    //     is printed or written -----------------------------------------
    session.send_line("w");
    session.expect("rivet-fleetd", Duration::from_secs(10));
    session.expect("published versions:", Duration::from_secs(5));
    session.expect("(newest)", Duration::from_secs(5));
    session.expect("carry [1]:", Duration::from_secs(5));
    session.send_line("2"); // the OLDER one, an explicit non-default pick

    session.expect("certificate", Duration::from_secs(15));
    session.expect("written", Duration::from_secs(10));
    session.expect("parsed by rivet-config", Duration::from_secs(10));
    // **`.rulings/` R-189 D2**: the Identity default followed the hostname,
    // and that directory did not exist, so `w` minted it.
    session.expect("minted identity", Duration::from_secs(10));

    // --- stage 4: back in the orchestrator -- staging, enrollment, and the
    //     device step. The bare interactive run offers the removable devices
    //     (R-174, 2026-09-14); this harness never lets it write: it answers N
    //     at `Write it? [y/N]`, before any administrator prompt exists. ------
    session.expect("[3/3] done", Duration::from_secs(10));
    let declined_normally = match session.expect_any(
        &["removable devices:", "sees no removable device"],
        Duration::from_secs(60),
    ) {
        0 => {
            session.send_line("1");
            match session.expect_any(&["Write it? [y/N]", "does not fit"], Duration::from_secs(15)) {
                0 => {
                    session.send_line("N");
                    true
                }
                // A removable device smaller than the image is refused
                // before the question -- still nothing written.
                _ => false,
            }
        }
        _ => true,
    };
    // "Nothing"/"nothing": the declined and refused endings both say it.
    session.expect("othing was recorded", Duration::from_secs(15));
    session.expect("cargo rivet provision --document ", Duration::from_secs(5));

    let status = session.wait();
    assert!(
        !declined_normally || status.success(),
        "declining is an ordinary outcome and should exit 0:\n{}",
        session.transcript
    );
    assert!(!session.transcript.contains("image is ready at"),"never point at the scratch copy:\n{}", session.transcript);
    // **The interview says nothing about building a card** — this run writes
    // one itself (`ADR-054` §2). Found on the maintainer's first real run,
    // 2026-09-14, where the interview named `tools/card/build.sh`.
    // Nor does the orchestrator's own output name a build variable: the carried
    // applications' trust note is about the published image, and this
    // checkout's `keys/*.pub` note does not apply to one.
    for stale in ["build.sh", "RIVET_IDENTITY", "RIVET_APP_KEY", "RIVET_TRUSTED_KEY"] {
        assert!(!session.transcript.contains(stale), "{stale} in the transcript:\n{}", session.transcript);
    }
    assert!(
        session.transcript.contains("published trusting a different application key"),
        "the carried applications' trust note is missing:\n{}",
        session.transcript
    );
    assert!(
        !session.transcript.contains("Windows will now ask for administrator"),
        "no administrator prompt may be raised:\n{}",
        session.transcript
    );
    let rerun = session
        .transcript
        .split("cargo rivet provision --document ")
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .unwrap_or_default()
        .to_owned();
    assert!(
        rerun.contains("--identity ") && rerun.contains("--image ") && rerun.contains("--at "),
        "{rerun}\n--- transcript ---\n{}",
        session.transcript
    );
    assert!(rerun.trim_end().ends_with("confirm"), "{rerun}\n--- transcript ---\n{}", session.transcript);

    let document = fixtures.at.join("provision").join(format!("{host}.provisioning.toml"));
    let text = std::fs::read_to_string(&document)
        .unwrap_or_else(|e| panic!("the interview's own `w` must have written {}: {e}", document.display()));
    assert!(text.contains(&format!("hostname = \"{host}\"")), "{text}");
    // **Addressing was left at dhcp, so no nameservers were asked and no
    // resolver was declared** — the lease fills it. A static machine is asked;
    // `tools/provision/rehearse-member.sh`'s server is that run.
    assert!(!text.contains("[network.resolver]"), "a dhcp document declares a resolver:\n{text}");
    assert!(
        !session.transcript.contains("Nameservers"),
        "a dhcp machine was shown or asked Nameservers:\n{}",
        session.transcript
    );
    // **The explicit, non-default pick (option 2, the OLDER `0.1.0`) actually
    // reached the document.** `0.1.1` (the newer one, published second so
    // `versions::list`'s newest-first ordering makes it the default) is what
    // a bare Return would have carried; asserting `0.1.0` specifically is
    // what proves the TYPED "2" was read, not merely that the menu ran.
    assert!(text.contains("[carry.rivet-fleetd]\nversion = \"0.1.0\""), "{text}");
    assert!(
        !text.contains("[carry.rivet-proxyd]"),
        "the proxy is part of the image and is never carried (ADR-060): {text}"
    );
    // **`.rulings/` R-184 D3: a server's document declares `[repos.fleet]`**,
    // with an identifier `w` minted — sixteen lowercase hex characters, which
    // the board's own parser is the authority on.
    let repos = text
        .split_once("[repos.fleet]\nid = \"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(id, _)| id.to_owned())
        .unwrap_or_else(|| panic!("a server's document must declare [repos.fleet]:\n{text}"));
    assert!(
        repos.len() == 16 && repos.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "the [repos.fleet] id is not a minted identifier: {repos:?}"
    );

    // The decompressed working copy AND its `image-work-<pid>` directory are
    // gone once the run exits.
    let left: Vec<String> = std::fs::read_dir(fixtures.at.join("provision"))
        .expect("read <at>/provision back")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("image-work-"))
        .collect();
    assert!(left.is_empty(), "left behind under <at>/provision: {left:?}");

    let staging =fixtures.at.join("provision").join(format!("{host}.staged.txt"));
    assert!(staging.is_file(), "the interview's own `w` must also write the ADR-049 staging bundle: {}", staging.display());

    // **The machine's own identity, minted at `w`** — `.rulings/` R-189 D2:
    // a seed, a certificate, and a peers file naming this fixture's operator
    // and the fleet server's own operator key. The rerun command names it.
    let identity = fixtures.at.join("identity").join(host);
    for file in ["device.seed", "device.crt"] {
        assert!(identity.join(file).is_file(), "no {file} minted in {}", identity.display());
    }
    let peers = std::fs::read_to_string(identity.join("peers")).expect("the minted peers file");
    assert_eq!(peers.lines().filter(|l| !l.trim().is_empty()).count(), 2, "{peers}");
    assert!(rerun.contains(&identity.display().to_string()), "{rerun}");

    // **The register itself is NOT proven here.** R-178 Q7's enrollment
    // (`RealFleetRecorder::commit`) runs only after `write-image` has
    // actually written and verified a card on a route that targeted a
    // device (R-176 Q10) — and this run, like every other in this file,
    // stops at "Nothing was written to a device" on purpose (this session's
    // hard rule: never write a disk). `register_root_for` putting the
    // register under `--at` rather than behind `ignition::config`/
    // `%APPDATA%` is proven directly instead, in two pieces:
    // `provision::tests::given_at_the_register_root_is_under_it` (pure —
    // `register_root_for(Some(at)) == at.join("register")`), and
    // `provision::tests::scratch_end_to_end_the_document_route_commits_and_enrolls_after_a_successful_write`
    // (a `RealFleetRecorder` given exactly that shape of root actually
    // enrolling a device into a real, scratch `rivetos_register` register,
    // against a fake writer that never touches a disk either).
    sentinel_untouched(&fixtures);
    eprintln!("--- the composed document ---\n{text}");
    eprintln!("--- full transcript ---\n{}", session.transcript);
}

/// **`.rulings/` R-191/R-192 — the verb works outside a checkout, and the
/// two routes are timed against each other.**
///
/// The maintainer's first real `cargo rivet provision` was refused outside
/// the repository root and then, once he was inside it, spent about two
/// minutes before the first question: the interview was being rebuilt from
/// source with `cargo run`, because `cargo install --path cmd/cargo-rivet`
/// never installed `rivet-provision` beside it.
///
/// One set of fixtures, one published image, two runs — so the difference
/// between them is the interview's launch and nothing else:
///
/// 1. **A pair, from a directory that is not a checkout.** The binary is this
///    test's own `cargo-rivet`, which has `rivet-provision` beside it exactly
///    as an installed pair does, and the working directory is a scratch
///    directory with no `cmd/` in it at all. The old refusal must be absent,
///    the stage line must name the sibling BINARY, and the interview's first
///    question must arrive.
/// 2. **A lone `cargo-rivet`, from inside the checkout.** A copy of the same
///    binary in a scratch directory with no sibling, run from the repository
///    root: the `cargo run` fallback, which must still work, because that is
///    what makes a developer's edit to `cmd/rivet-provision` take effect.
///
/// **What the timings are and are not.** Each is wall-clock from spawn to the
/// interview's first prompt, and to the `[3/3]` stage line, printed rather
/// than asserted — a threshold here would fail on a slower host and prove
/// nothing about the code. Run 2's number is a WARM `cargo run`: the crate is
/// already built in this tree's `target/release`, so it is the cheap end of
/// that route, and the expensive end is what the maintainer measured.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc for how to run it"]
fn an_installed_pair_launches_the_interview_from_anywhere_and_a_lone_cargo_rivet_falls_back_to_cargo_run()
{
    let Some(image) = newest_card_image() else {
        eprintln!("SKIPPED: no *-card.img in out/ -- see real_end_to_end in provision_tests.rs for how one gets there");
        return;
    };
    let fixtures = setup_fixtures("outside-a-checkout");
    // ONE published image: nothing to choose, so neither run pays for a menu
    // and the two are comparable.
    publish_card_image(&fixtures, &image, "1.0.0-pty-outside");

    // --- run 1: the installed shape, from a directory that is not a checkout
    let elsewhere = scratch("outside-a-checkout-cwd");
    assert!(
        !elsewhere.join("cmd").join("rivet-provision").join("Cargo.toml").is_file(),
        "the fixture directory must not be a Rivet checkout: {}",
        elsewhere.display()
    );
    let sibling =
        cargo_rivet_exe().with_file_name(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX));
    assert!(
        sibling.is_file(),
        "build both first: cargo build --release -p cargo-rivet -p rivetos-provision ({} is absent)",
        sibling.display()
    );
    let (paired_to_stage, paired_to_prompt) =
        time_to_first_prompt(&fixtures, &cargo_rivet_exe(), &elsewhere, &sibling.display().to_string());

    // --- run 2: a lone cargo-rivet, from inside the checkout ---------------
    let lone = scratch("lone-cargo-rivet");
    let copied = lone.join(format!("cargo-rivet{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(cargo_rivet_exe(), &copied).expect("copy cargo-rivet into a directory of its own");
    assert!(
        !lone.join(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX)).exists(),
        "the lone copy must have no sibling"
    );
    let (cargo_run_to_stage, cargo_run_to_prompt) =
        time_to_first_prompt(&fixtures, &copied, &repo_root(), "cargo run -p rivetos-provision");

    eprintln!(
        "--- time to the interview's first prompt ---\n\
         \x20 sibling, outside a checkout: {paired_to_prompt:?} total, {:?} after the stage line\n\
         \x20 cargo run, inside one (warm): {cargo_run_to_prompt:?} total, {:?} after the stage line",
        paired_to_prompt.saturating_sub(paired_to_stage),
        cargo_run_to_prompt.saturating_sub(cargo_run_to_stage),
    );
    sentinel_untouched(&fixtures);
}

/// **`.rulings/` R-191 D3, live: a sibling that cannot say which build it is
/// never gets to ask a question.**
///
/// The pair share the interview's files, its breadcrumb and the saved-answers
/// notebook, so `cargo rivet provision` asks the sibling's `--version` before
/// running it. A build old enough to refuse that flag is exactly the stale
/// half this exists to catch, and it cannot be caught by comparing two
/// strings when one of them never arrives.
///
/// **The stand-in is this test's own binary**, copied in beside a copy of
/// `cargo-rivet` under the name the sibling lookup uses: a real executable
/// that really refuses `--version`, so the spawn, the read and the refusal
/// are all the real ones. Building a second `rivet-provision` with a
/// different stamp would prove the same branch at the cost of a full
/// compile inside a test; the wording of the mismatch itself is proven
/// without any process at all, in `provision_tests.rs`.
#[test]
#[ignore = "spawns real processes under a real pty; see this file's module doc for how to run it"]
fn a_sibling_that_cannot_say_which_build_it_is_never_reaches_a_question() {
    let Some(image) = newest_card_image() else {
        eprintln!("SKIPPED: no *-card.img in out/ -- see real_end_to_end in provision_tests.rs for how one gets there");
        return;
    };
    let fixtures = setup_fixtures("mismatched-pair");
    publish_card_image(&fixtures, &image, "1.0.0-pty-mismatched-pair");

    let bin = scratch("mismatched-pair-bin");
    let cargo_rivet = bin.join(format!("cargo-rivet{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(cargo_rivet_exe(), &cargo_rivet).expect("copy cargo-rivet into a scratch bin");
    let planted = bin.join(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX));
    let stand_in = std::env::current_exe().expect("this test binary's own path");
    std::fs::copy(&stand_in, &planted).expect("plant a sibling that refuses --version");

    let elsewhere = scratch("mismatched-pair-cwd");
    let mut session = Session::spawn_provision_from(&fixtures, &[], &cargo_rivet, &elsewhere);
    session.expect("refuses a mismatched pair", Duration::from_secs(120));
    // Both sides are named, and so is the way out.
    session.expect("cargo-rivet", Duration::from_secs(5));
    session.expect("rivet-provision", Duration::from_secs(5));
    session.expect("cargo install --path cmd/rivet-provision --force", Duration::from_secs(5));
    let status = session.wait();
    assert!(!status.success(), "a mismatched pair must refuse:\n{}", session.transcript);
    // The interview never ran, so nothing was ever asked.
    assert!(
        !session.transcript.contains("join one?"),
        "a question was asked despite the refusal:\n{}",
        session.transcript
    );
    sentinel_untouched(&fixtures);
    eprintln!("--- the refusal ---\n{}", session.transcript);
}

/// Spawn one run, drive it to the interview's first question, and leave
/// nothing behind — the body both halves of the test above share.
///
/// Returns how long the `[3/3]` stage line and the interview's own first
/// prompt each took from the spawn. `route` is asserted to appear in the
/// stage line, which is what proves WHICH interview source the run took
/// rather than merely that one of them worked.
fn time_to_first_prompt(
    fixtures: &Fixtures,
    exe: &Path,
    cwd: &Path,
    route: &str,
) -> (Duration, Duration) {
    let started = Instant::now();
    let mut session = Session::spawn_provision_from(fixtures, &[], exe, cwd);
    // **The mismatch refusal is watched for too** (`.rulings/` R-191 D3), so
    // a tree where only one of the two binaries has been rebuilt says so in
    // two seconds instead of timing out five minutes later on a stage line
    // that is never going to be printed.
    if session.expect_any(
        &["[3/3] running the interview", "refuses a mismatched pair"],
        Duration::from_secs(300),
    ) == 1
    {
        panic!(
            "the pair beside {} do not match -- rebuild both (cargo build --release -p \
             cargo-rivet -p rivetos-provision):\n{}",
            exe.display(),
            session.transcript
        );
    }
    let to_stage = started.elapsed();
    // **The old gate's own words must never appear again.** It refused every
    // route on the working directory, including the three that never start an
    // interview at all.
    assert!(
        !session.transcript.contains("has to run from the repository root"),
        "the working-directory gate is back:\n{}",
        session.transcript
    );
    assert!(
        session.transcript.contains(route),
        "the stage line does not name {route:?}, so this run took a different interview source:\n{}",
        session.transcript
    );
    // `new fleet, or join one?` is `cmd/rivet-provision`'s own first question
    // on a scratch credential root with no authority in it (`ADR-063` §8) —
    // the first thing a person is asked, and so the end of the launch this
    // measures.
    session.expect("new fleet, or join one?", Duration::from_secs(300));
    let to_prompt = started.elapsed();

    // Leave the question with `x`: nothing is minted, composed or written, the
    // interview exits unsuccessfully, and the orchestrator reports it leaving
    // no document — which is itself proof that it really ran the interview.
    session.send_line("x");
    let status = session.wait();
    assert!(!status.success(), "a quit interview writes no document, so the run must fail:\n{}", session.transcript);
    assert!(
        session.transcript.contains("without writing a document"),
        "the orchestrator did not report the missing document:\n{}",
        session.transcript
    );
    (to_stage, to_prompt)
}

/// The crate's own scratch root — see the note in `tests/update.rs`.
#[path = "../src/scratch.rs"]
#[allow(dead_code)]
mod scratch;
