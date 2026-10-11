//! `fleet publish --fleet` and `fleet fetch`, end to end with no board: the
//! server's text is a real bare repository behind this host's own `git
//! http-backend` (the program `cmd/rivet-mcpd` runs), reached through the real
//! loopback relay; its repository store is in memory, and hands an object back
//! only when its bytes hash to its digest, as `rivet_remote::tls`'s reader does.
//! The artifacts are real signed `.rvt`s (`fleet_tests.rs`'s `fixture`).

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use rivet_core::artifact::Digest;
use rivet_core::repository_objects::{Held, Listing, Manifest};

use super::*;

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!("fleet-remote-test-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn hex_of(bytes: &[u8]) -> String {
    use rivet_core::package::port::Digest as _;
    crate::key::hex(&rivet_crypto::Blake3::new().of(bytes))
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// **A fleet server with no board**: `git http-backend` over a bare
/// `fleet.git` for the text, and an in-memory store for the objects whose
/// listing reads the manifest out of that same repository, as `rivet-mcpd`'s
/// does.
struct Server {
    project_root: PathBuf,
    held: Mutex<BTreeMap<String, (String, Vec<u8>)>>,
    /// Hand back these bytes for every object read, whatever is held — a
    /// server whose store was tampered with.
    corrupt: Mutex<bool>,
    /// Refuse every `put`, as a door refuses an artifact.
    refuse_put: Mutex<bool>,
}

impl Server {
    fn new(base: &Path) -> Self {
        let project_root = base.join("state-git");
        std::fs::create_dir_all(&project_root).expect("server root");
        git_in(&project_root, &["init", "--quiet", "--bare", "--initial-branch=main", "fleet.git"]);
        Self {
            project_root,
            held: Mutex::new(BTreeMap::new()),
            corrupt: Mutex::new(false),
            refuse_put: Mutex::new(false),
        }
    }

    fn main(&self) -> Option<String> {
        let out = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "refs/heads/main"])
            .current_dir(self.project_root.join("fleet.git"))
            .output()
            .expect("git runs");
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn named(&self) -> Manifest {
        let Some(commit) = self.main() else {
            return Manifest::Unavailable("no main".to_owned());
        };
        let text = git_in(&self.project_root.join("fleet.git"), &["show", &format!("{commit}:manifest.toml")]);
        let named = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("digest"))
            .filter_map(|rest| rest.split('"').nth(1))
            .filter_map(Digest::new)
            .collect();
        Manifest::Read { commit, named }
    }

    fn held_digests(&self) -> Vec<String> {
        self.held.lock().expect("lock").keys().cloned().collect()
    }
}

impl sync::ObjectStore for Server {
    fn list(&self) -> Result<String, String> {
        let held = self
            .held
            .lock()
            .expect("lock")
            .iter()
            .map(|(d, (n, _))| Held { digest: Digest::new(d).expect("digest"), name: n.clone() })
            .collect();
        Ok(Listing { manifest: self.named(), held }.render())
    }

    fn put(&self, local: &Path, name: &str) -> Result<String, String> {
        if *self.refuse_put.lock().expect("lock") {
            return Err("dev-01 REFUSED THE ARTIFACT: the test's door refuses everything".to_owned());
        }
        let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
        self.held.lock().expect("lock").insert(hex_of(&bytes), (name.to_owned(), bytes));
        Ok("201 Created\n".to_owned())
    }

    fn prune(&self) -> Result<String, String> {
        Err("not in these tests".to_owned())
    }
}

impl FleetServer for Server {
    fn object(&self, digest: &str, into: &Path) -> Result<String, String> {
        let (name, mut bytes) =
            self.held.lock().expect("lock").get(digest).cloned().ok_or_else(|| format!("404 {digest}"))?;
        if *self.corrupt.lock().expect("lock") {
            bytes[0] ^= 0xff;
        }
        std::fs::write(into, &bytes).map_err(|e| e.to_string())?;
        if hex_of(&bytes) != digest {
            let _ = std::fs::remove_file(into);
            return Err(format!("the bytes received do not hash to {digest}"));
        }
        Ok(name)
    }
}

impl push::Upstream for Server {
    fn exchange(&self, head: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or_default().split(' ');
        let method = first.next().unwrap_or_default().to_owned();
        let target = first.next().unwrap_or_default().to_owned();
        let header = |name: &str| {
            head.split("\r\n")
                .filter_map(|l| l.split_once(':'))
                .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_owned())
        };
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let mut command = Command::new("git");
        command
            .arg("http-backend")
            .env("REQUEST_METHOD", &method)
            .env("PATH_INFO", path)
            .env("QUERY_STRING", query)
            .env("GIT_PROJECT_ROOT", &self.project_root)
            .env("GIT_HTTP_EXPORT_ALL", "1")
            .env("REMOTE_USER", "pinned-test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(kind) = header("content-type") {
            command.env("CONTENT_TYPE", kind);
        }
        if let Some(encoding) = header("content-encoding") {
            command.env("HTTP_CONTENT_ENCODING", encoding);
        }
        if method == "POST" {
            command.env("CONTENT_LENGTH", body.len().to_string());
        }
        let mut child = command.spawn().map_err(|e| format!("git http-backend: {e}"))?;
        let mut stdin = child.stdin.take().expect("stdin");
        let body = body.to_vec();
        let feeder = std::thread::spawn(move || {
            let _ = stdin.write_all(&body);
        });
        let output = child.wait_with_output().map_err(|e| format!("git http-backend: {e}"))?;
        let _ = feeder.join();
        let raw = output.stdout;
        let (at, gap) = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| (p, 4))
            .or_else(|| raw.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2)))
            .ok_or_else(|| format!("no CGI head: {}", String::from_utf8_lossy(&output.stderr)))?;
        let cgi_head = String::from_utf8_lossy(&raw[..at]).into_owned();
        let mut status = "200 OK".to_owned();
        let mut headers = String::new();
        for line in cgi_head.lines() {
            if let Some(value) = line.strip_prefix("Status:") {
                status = value.trim().to_owned();
            } else if !line.trim().is_empty() {
                headers.push_str(line.trim_end());
                headers.push_str("\r\n");
            }
        }
        let mut answer = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n").into_bytes();
        answer.extend_from_slice(&raw[at + gap..]);
        Ok(answer)
    }
}

struct Rig {
    base: PathBuf,
    server: Arc<Server>,
    clone: PathBuf,
}

impl Rig {
    fn new(label: &str) -> Self {
        let base = scratch(label);
        std::fs::create_dir_all(&base).expect("base");
        let server = Arc::new(Server::new(&base));
        let clone = base.join("fleets").join("dev-fleet");
        Self { base, server, clone }
    }

    fn artifact(&self, version: &str) -> (PathBuf, PathBuf) {
        let dir = self.base.join("out");
        std::fs::create_dir_all(&dir).expect("out");
        super::super::tests::fixture(&dir, "hello", version, 7)
    }

    fn publish(&self, file: &Path, key: &Path, confirm: bool) -> Result<String, String> {
        let mut trusted = || crate::key::read_32(key, "trusted public key").map(PublicKey);
        publish(&work_dir(&self.clone), &self.clone, &self.server, "dev-fleet", file, &mut trusted, confirm)
    }

    fn objects_in(&self, clone: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(clone.join("objects"))
            .map(|e| e.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        out.sort();
        out
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// **The first publish to an empty fleet** creates its `main`, puts the object
/// in its store, and leaves the clone holding both — the clone's `main` is the
/// server's and its one object hashes to its name.
#[test]
fn a_first_publish_creates_the_servers_main_and_the_clone_fetches_it() {
    let rig = Rig::new("first");
    let (file, key) = rig.artifact("0.1.0");
    let report = rig.publish(&file, &key, true).expect("published");
    assert!(report.contains("  sent      "), "{report}");
    assert!(report.contains("PUSHED"), "{report}");
    assert!(report.contains("CLONE  dev-fleet  main"), "{report}");

    let main = rig.server.main().expect("the server has a main");
    assert_eq!(git_in(&rig.clone, &["rev-parse", "refs/heads/main"]), main);
    let digest = hex_of(&std::fs::read(&file).expect("the artifact"));
    assert_eq!(rig.server.held_digests(), vec![digest.clone()]);
    assert_eq!(rig.objects_in(&rig.clone), vec![digest]);
    assert!(!work_dir(&rig.clone).exists(), "the scratch repository was left behind");
    assert!(
        std::fs::read_to_string(rig.clone.join("manifest.toml")).expect("manifest").contains("hello"),
        "the clone's manifest does not name the artifact"
    );
}

/// **Without `confirm` nothing reaches the server**, and the clone is not
/// made.
#[test]
fn a_plan_sends_nothing_and_pushes_nothing() {
    let rig = Rig::new("plan");
    let (file, key) = rig.artifact("0.1.0");
    let report = rig.publish(&file, &key, false).expect("a plan");
    assert!(report.contains("would send"), "{report}");
    assert!(report.contains("Nothing reached dev-fleet"), "{report}");
    assert_eq!(rig.server.main(), None);
    assert!(rig.server.held_digests().is_empty());
    assert!(!work_dir(&rig.clone).exists());
}

/// **Files, then the list**: an object the door refuses means `main` is
/// never pushed, so the server's manifest never names what its store lacks.
#[test]
fn a_refused_object_means_main_is_not_pushed() {
    let rig = Rig::new("refused");
    let (file, key) = rig.artifact("0.1.0");
    *rig.server.refuse_put.lock().expect("lock") = true;
    let report = rig.publish(&file, &key, true).expect_err("refused");
    assert!(report.contains("NOT PUBLISHED"), "{report}");
    assert_eq!(rig.server.main(), None, "main was pushed after a refused object");
}

/// **A second publish composes on top of the first**, the clone
/// fast-forwards, and **a second, empty clone fetched afterwards agrees with
/// the first** — the same `main` and the same objects: stale or current,
/// never contradictory.
#[test]
fn two_clones_fetched_from_one_server_agree() {
    let rig = Rig::new("agree");
    let (one, key) = rig.artifact("0.1.0");
    rig.publish(&one, &key, true).expect("first");
    let first_main = rig.server.main().expect("main");
    let (two, _) = rig.artifact("0.2.0");
    let report = rig.publish(&two, &key, true).expect("second");
    assert!(report.contains("fast-forward"), "{report}");
    let second_main = rig.server.main().expect("main");
    assert_ne!(first_main, second_main);
    assert_eq!(git_in(&rig.server.project_root.join("fleet.git"), &["rev-list", "--count", "main"]), "3");

    let other = rig.base.join("second-clone");
    let fetched = fetch(&other, &rig.server, "dev-fleet").expect("fetched");
    assert!(fetched.contains("2 fetched"), "{fetched}");
    assert_eq!(git_in(&other, &["rev-parse", "refs/heads/main"]), second_main);
    assert_eq!(git_in(&rig.clone, &["rev-parse", "refs/heads/main"]), second_main);
    assert_eq!(rig.objects_in(&other), rig.objects_in(&rig.clone));
    assert_eq!(rig.objects_in(&other).len(), 2);

    let again = fetch(&other, &rig.server, "dev-fleet").expect("again");
    assert!(again.contains("0 fetched, 2 already here and hashed again"), "{again}");
}

/// **An object that does not hash to its digest is not kept**, and a copy
/// in the clone that stopped matching its name is replaced.
#[test]
fn a_corrupt_object_is_not_kept_and_a_corrupt_copy_is_replaced() {
    let rig = Rig::new("corrupt");
    let (file, key) = rig.artifact("0.1.0");
    rig.publish(&file, &key, true).expect("published");
    let digest = hex_of(&std::fs::read(&file).expect("the artifact"));

    std::fs::write(rig.clone.join("objects").join(&digest), b"rotted").expect("rot the copy");
    let report = fetch(&rig.clone, &rig.server, "dev-fleet").expect("fetched");
    assert!(report.contains("replace"), "{report}");
    assert_eq!(hex_of(&std::fs::read(rig.clone.join("objects").join(&digest)).expect("copy")), digest);

    let other = rig.base.join("tampered-clone");
    *rig.server.corrupt.lock().expect("lock") = true;
    let report = fetch(&other, &rig.server, "dev-fleet").expect_err("a corrupt store");
    assert!(report.contains("NOT COMPLETE"), "{report}");
    assert!(rig.objects_in(&other).is_empty(), "a corrupt object was kept: {:?}", rig.objects_in(&other));
}

/// **An object the server no longer holds leaves the clone.**
#[test]
fn an_object_the_server_stopped_holding_is_removed_from_the_clone() {
    let rig = Rig::new("pruned");
    let (file, key) = rig.artifact("0.1.0");
    rig.publish(&file, &key, true).expect("published");
    let digest = hex_of(&std::fs::read(&file).expect("the artifact"));
    rig.server.held.lock().expect("lock").clear();
    let report = fetch(&rig.clone, &rig.server, "dev-fleet").expect("fetched");
    assert!(report.contains(&format!("removed   {digest}")), "{report}");
    assert!(rig.objects_in(&rig.clone).is_empty());
}

/// **A clone with history the server lacks is refused and left alone** — a
/// clone never commits, so nothing here decides which side is right.
#[test]
fn a_clone_that_committed_is_refused_and_left_alone() {
    let rig = Rig::new("diverged");
    let (file, key) = rig.artifact("0.1.0");
    rig.publish(&file, &key, true).expect("published");
    super::super::ensure_local_identity(&rig.clone).expect("identity");
    std::fs::write(rig.clone.join("stray.txt"), b"a local commit").expect("write");
    git_in(&rig.clone, &["add", "stray.txt"]);
    git_in(&rig.clone, &["commit", "--quiet", "-m", "a clone that committed"]);
    let local = git_in(&rig.clone, &["rev-parse", "HEAD"]);
    let (two, _) = rig.artifact("0.2.0");
    // The publish itself lands on the server; its fetch is what refuses.
    let report = rig.publish(&two, &key, true).expect_err("the fetch refuses");
    assert!(report.contains("not an ancestor"), "{report}");
    assert_eq!(git_in(&rig.clone, &["rev-parse", "HEAD"]), local, "the refusal changed the clone");
}

/// A directory that is not a clone and not empty is never fetched into.
#[test]
fn a_directory_that_is_not_a_clone_is_refused() {
    let rig = Rig::new("not-a-clone");
    std::fs::create_dir_all(&rig.clone).expect("dir");
    std::fs::write(rig.clone.join("somebody-elses.txt"), b"x").expect("write");
    let refused = fetch(&rig.clone, &rig.server, "dev-fleet").expect_err("refused");
    assert!(refused.contains("not a clone this tool made"), "{refused}");
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_owned()).collect()
}

/// **`--fleet` takes `confirm` and nothing that names another repository or
/// server**; `fetch` takes one fleet and `--into`.
#[test]
fn the_grammar_reads_publish_to_a_fleet_and_fetch() {
    let parsed = super::super::parse(&words(&["publish", "x.rvt", "--trusted-key", "k.pub", "--fleet", "dev-fleet", "confirm"]))
        .expect("parses");
    match parsed.verb {
        super::super::SubVerb::Publish { fleet, sync_confirm, sync, .. } => {
            assert_eq!(fleet.as_deref(), Some("dev-fleet"));
            assert!(sync_confirm && !sync);
        }
        _ => panic!("not a publish"),
    }
    for refused in [
        vec!["publish", "x.rvt", "--trusted-key", "k", "--fleet", "dev-fleet", "--sync"],
        vec!["publish", "x.rvt", "--trusted-key", "k", "--fleet", "dev-fleet", "--at", "d"],
        vec!["publish", "x.rvt", "--trusted-key", "k", "--fleet", "dev-fleet", "--host", "h:1"],
        vec!["publish", "x.rvt", "--trusted-key", "k", "--fleet", "Not_A_Label"],
    ] {
        assert!(super::super::parse(&words(&refused)).is_err(), "{refused:?} was accepted");
    }
    match super::super::parse(&words(&["fetch", "dev-fleet", "--into", "d"])).expect("fetch").verb {
        super::super::SubVerb::Fetch { fleet, into } => {
            assert_eq!(fleet, "dev-fleet");
            assert_eq!(into, Some(PathBuf::from("d")));
        }
        _ => panic!("not a fetch"),
    }
    assert!(super::super::parse(&words(&["fetch"])).is_err());
    assert!(super::super::parse(&words(&["fetch", "a", "b"])).is_err());
}

#[test]
fn the_scratch_repository_sits_beside_the_clone() {
    let clone = PathBuf::from("rivet").join("fleets").join("dev-fleet");
    assert_eq!(work_dir(&clone), PathBuf::from("rivet").join("fleets").join(".dev-fleet.publishing"));
}
