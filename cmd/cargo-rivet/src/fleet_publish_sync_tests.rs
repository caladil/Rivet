//! `cargo rivet fleet publish FILE --sync [confirm]` — `ADR-061` §4,
//! `.rulings/` R-222.
//!
//! Three things, tested apart because each has its own failure mode:
//!
//! 1. **The grammar** — `parse()` alone, never `run()`: `--sync` and `confirm`
//!    read into `SubVerb::Publish`, and `confirm` without `--sync` is refused.
//! 2. **Finding the fleet server** — [`super::resolve_fleet_server`], pure over
//!    an in-memory [`rivetos_register::Register`]. No register file, and
//!    nothing on disk at all.
//! 3. **The composition** — [`super::sync_then_push`], against a single
//!    in-memory fake that is both `sync::ObjectStore` and `push::Upstream`,
//!    exactly as one real `rivet_remote::tls::Tls` is both on a real run. The
//!    fake's `exchange` (the `git` half) always refuses, so these tests prove
//!    the ORDER — files before the list, and never the list after a failed
//!    send — not a real push landing; `fleet_push_tests.rs`'s real CGI harness
//!    already proves a real push lands.
//!
//! None of this touches `crate::device::default_register_dir` or a real
//! network: [`super::sync_to_fleet_server`] (which does) is exercised only by
//! reading it, not by calling it here.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use rivet_core::artifact::Digest;
use rivet_core::repository_objects::{Held, Listing, Manifest};

use super::*;

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_owned()).collect()
}

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!(
        "fleet-publish-sync-test-{label}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn hex_of(bytes: &[u8]) -> String {
    use rivet_core::package::port::Digest as _;
    crate::key::hex(&rivet_crypto::Blake3::new().of(bytes))
}

// ---------------------------------------------------------------------------
// 1. The grammar
// ---------------------------------------------------------------------------

fn parsed_publish(
    list: &[&str],
) -> Result<(bool, bool, rivet_remote::endpoint::Endpoint), String> {
    match parse(&words(list))? {
        FleetArgs { verb: SubVerb::Publish { sync, sync_confirm, endpoint, .. }, .. } => {
            Ok((sync, sync_confirm, endpoint))
        }
        _ => panic!("{list:?} did not parse as a publish"),
    }
}

#[test]
fn sync_alone_plans_and_sync_confirm_confirms() {
    let (sync, confirm, endpoint) =
        parsed_publish(&["publish", "x.rvt", "--trusted-key", "k.pub", "--sync"]).expect("parses");
    assert!(sync && !confirm, "--sync alone is a plan");
    assert!(endpoint.host.is_none());

    let (sync, confirm, _) =
        parsed_publish(&["publish", "x.rvt", "--trusted-key", "k.pub", "--sync", "confirm"])
            .expect("parses");
    assert!(sync && confirm);
}

/// **`confirm` belongs to `--sync`.** A plain publish needs none (`ADR-052`
/// §4), and a stray `confirm` after a publish with no `--sync` is refused
/// rather than silently ignored — nobody should have to wonder whether it did
/// something.
#[test]
fn confirm_without_sync_is_refused() {
    let refused = parse(&words(&["publish", "x.rvt", "--trusted-key", "k.pub", "confirm"]))
        .err()
        .expect("refused");
    assert!(refused.contains("`confirm` is `--sync`'s"), "{refused}");
}

/// **`--sync` reads the same three overrides `fleet push`/`fleet sync` do.**
#[test]
fn sync_reads_host_identity_and_pin() {
    let (sync, confirm, endpoint) = parsed_publish(&[
        "publish",
        "x.rvt",
        "--trusted-key",
        "k.pub",
        "--sync",
        "--host",
        "10.0.0.9:7777",
        "--identity",
        "./op",
        "--pin",
        "ab",
        "confirm",
    ])
    .expect("parses");
    assert!(sync && confirm);
    assert_eq!(endpoint.host.as_deref(), Some("10.0.0.9:7777"));
    assert_eq!(endpoint.identity, Some(PathBuf::from("./op")));
    assert_eq!(endpoint.pin.as_deref(), Some("ab"));
}

/// A publish with no `--sync` at all still parses exactly as it always did —
/// `sync`/`sync_confirm` are both false and no endpoint flag is required.
#[test]
fn a_plain_publish_needs_no_sync_flag_at_all() {
    let (sync, confirm, endpoint) =
        parsed_publish(&["publish", "x.rvt", "--trusted-key", "k.pub"]).expect("parses");
    assert!(!sync && !confirm);
    assert!(endpoint.host.is_none());
}

// ---------------------------------------------------------------------------
// 2. Finding the fleet server — pure over an in-memory register
// ---------------------------------------------------------------------------

fn device(id: &str, name: &str, role: Option<&str>) -> rivetos_register::Device {
    rivetos_register::Device {
        id: id.to_owned(),
        name: name.to_owned(),
        role: role.map(str::to_owned),
        ..rivetos_register::Device::default()
    }
}

#[test]
fn no_fleet_server_record_is_refused_by_name() {
    let mut register = rivetos_register::Register::empty(scratch("no-server"));
    register.enroll(device("0011223344556677", "rivet-01", Some("member"))).expect("enroll");
    let refused = resolve_fleet_server(&register).expect_err("no fleet-server role");
    assert!(refused.contains("role = \"fleet-server\""), "{refused}");
    assert!(refused.contains("Nothing was sent"), "{refused}");
}

#[test]
fn exactly_one_fleet_server_record_resolves() {
    let mut register = rivetos_register::Register::empty(scratch("one-server"));
    register.enroll(device("0011223344556677", "member-01", Some("member"))).expect("enroll");
    register.enroll(device("1122334455667788", "fleet-hub", Some("fleet-server"))).expect("enroll");
    let resolved = resolve_fleet_server(&register).expect("resolves");
    assert_eq!(resolved.name, "fleet-hub");
}

/// **The workstation's register marks its server with `fleet_role`, not
/// `role`.** The operator's own register — the one `--sync` loads — records
/// the fleet server as `fleet_role = "server"` and carries no `role` at all;
/// `role = "fleet-server"` is the spelling the server's own copy carries. A
/// resolver that read only `role` refused the maintainer's real register.
#[test]
fn a_workstation_record_marked_by_fleet_role_resolves() {
    let mut register = rivetos_register::Register::empty(scratch("fleet-role"));
    register.enroll(device("0011223344556677", "dev-01", None)).expect("enroll");
    let mut server = device("1122334455667788", "rivet-01", None);
    server.fleet_role = Some(rivetos_register::FleetRole::Server);
    register.enroll(server).expect("enroll");
    let resolved = resolve_fleet_server(&register).expect("resolves by fleet_role");
    assert_eq!(resolved.name, "rivet-01");
}

/// One machine carrying both markings is one fleet server, not two.
#[test]
fn a_record_carrying_both_markings_is_one_server() {
    let mut register = rivetos_register::Register::empty(scratch("both-markings"));
    let mut server = device("1122334455667788", "rivet-01", Some("fleet-server"));
    server.fleet_role = Some(rivetos_register::FleetRole::Server);
    register.enroll(server).expect("enroll");
    let resolved = resolve_fleet_server(&register).expect("one server");
    assert_eq!(resolved.name, "rivet-01");
}

#[test]
fn two_fleet_server_records_are_refused_naming_both() {
    let mut register = rivetos_register::Register::empty(scratch("two-servers"));
    register.enroll(device("0011223344556677", "hub-a", Some("fleet-server"))).expect("enroll");
    register.enroll(device("1122334455667788", "hub-b", Some("fleet-server"))).expect("enroll");
    let refused = resolve_fleet_server(&register).expect_err("ambiguous");
    assert!(refused.contains("hub-a") && refused.contains("hub-b"), "{refused}");
    assert!(refused.contains("will not choose"), "{refused}");
}

// ---------------------------------------------------------------------------
// 3. The composition — files, then the list, never the other order
// ---------------------------------------------------------------------------

/// A scratch fleet repository with a manifest and objects, committed on
/// `main` — the same shape `fleet_sync_tests.rs`'s `Repo` builds, kept small
/// here because only one artifact is ever named.
struct Repo(PathBuf);

impl Repo {
    fn new(label: &str) -> Self {
        let root = scratch(label);
        super::init(&root).expect("fleet init");
        Self(root)
    }

    fn add(&self, product: &str, version: &str) -> String {
        let bytes = format!("{product} {version}").into_bytes();
        let hex = hex_of(&bytes);
        std::fs::write(self.0.join("objects").join(&hex), &bytes).expect("an object");
        let manifest = self.0.join("manifest.toml");
        let mut text = std::fs::read_to_string(&manifest).unwrap_or_default();
        text.push_str(&format!(
            "\n[[artifact]]\ndigest = \"{hex}\"\nproduct = \"{product}\"\nversion = \"{version}\"\n\
             arch = \"aarch64\"\nkind = \"application\"\nsize = {}\nadded = \"2026-09-14\"\n",
            bytes.len()
        ));
        std::fs::write(&manifest, text).expect("the manifest");
        hex
    }

    fn commit(&self) -> String {
        super::git(&self.0, &["add", "-A"]).expect("add");
        super::git(&self.0, &["commit", "-q", "--allow-empty", "-m", "manifest"]).expect("commit");
        super::git(&self.0, &["rev-parse", "HEAD"]).expect("rev-parse").trim().to_owned()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// **One fake, both traits** — exactly what one real `rivet_remote::tls::Tls`
/// is on a real run, so [`sync_then_push`] can be handed a single `Arc` for
/// both halves.
///
/// `exchange` (the `git` half `push::push` speaks) always refuses: proving a
/// real push land is `fleet_push_tests.rs`'s real-CGI job. What these tests
/// need from it is only whether it was reached at all, and `exchanged` counts
/// that.
#[derive(Default)]
struct FakeServer {
    commit: Mutex<String>,
    named: Mutex<Vec<String>>,
    held: Mutex<BTreeMap<String, String>>,
    puts: Mutex<Vec<String>>,
    fail_put: Mutex<bool>,
    exchanged: Mutex<usize>,
}

impl FakeServer {
    fn pushed(&self, repo: &Repo, commit: &str, digest: &str) {
        *self.commit.lock().expect("lock") = commit.to_owned();
        self.named.lock().expect("lock").push(digest.to_owned());
        let _ = repo;
    }

    fn set_fail_put(&self, fail: bool) {
        *self.fail_put.lock().expect("lock") = fail;
    }

    fn puts(&self) -> Vec<String> {
        self.puts.lock().expect("lock").clone()
    }

    fn exchanged(&self) -> usize {
        *self.exchanged.lock().expect("lock")
    }

    fn listing(&self) -> Listing {
        let manifest = Manifest::Read {
            commit: self.commit.lock().expect("lock").clone(),
            named: self
                .named
                .lock()
                .expect("lock")
                .iter()
                .map(|d| Digest::new(d).expect("digest"))
                .collect(),
        };
        let held = self
            .held
            .lock()
            .expect("lock")
            .iter()
            .map(|(d, n)| Held { digest: Digest::new(d).expect("digest"), name: n.clone() })
            .collect();
        Listing { manifest, held }
    }
}

impl sync::ObjectStore for FakeServer {
    fn list(&self) -> Result<String, String> {
        Ok(self.listing().render())
    }

    fn put(&self, local: &Path, name: &str) -> Result<String, String> {
        self.puts.lock().expect("lock").push(name.to_owned());
        if *self.fail_put.lock().expect("lock") {
            return Err("rivet REFUSED THE ARTIFACT: the fake store never holds one".to_owned());
        }
        let bytes = std::fs::read(local).map_err(|e| e.to_string())?;
        self.held.lock().expect("lock").insert(hex_of(&bytes), name.to_owned());
        Ok("201".to_owned())
    }

    fn prune(&self) -> Result<String, String> {
        Ok(String::new())
    }
}

impl push::Upstream for FakeServer {
    fn exchange(&self, _head: &str, _body: &[u8]) -> Result<Vec<u8>, String> {
        *self.exchanged.lock().expect("lock") += 1;
        Err("the fake upstream answers no real git request".to_owned())
    }
}

/// **A failed send means `main` is never pushed** — `sync::sync`'s own `?`
/// inside [`sync_then_push`] is what stops it, and this proves it from the
/// outside: the fake's `exchange` (the `git` half) is never reached.
#[test]
fn a_failed_send_means_the_list_is_never_pushed() {
    let repo = Repo::new("failed-send");
    let digest = repo.add("hello", "0.1.0");
    let commit = repo.commit();
    let server = Arc::new(FakeServer::default());
    server.pushed(&repo, &commit, &digest);
    server.set_fail_put(true);

    let report = sync_then_push(&repo.0, Arc::clone(&server), "rivet-01", true)
        .expect_err("a failed send is not a complete sync");
    assert!(report.contains("failed") || report.contains("NOT COMPLETE"), "{report}");
    assert_eq!(server.exchanged(), 0, "the list must never reach the server after a failed send");
}

/// **A complete send is followed by the push** — the fake's `exchange` is
/// reached at least once, and the combined report carries both halves: the
/// objects report (which says SYNCED) and the push's own words (which is an
/// error here, since the fake speaks no real git — that error surfacing at
/// all is what proves the push was attempted after a successful sync).
#[test]
fn a_complete_send_is_followed_by_the_push() {
    let repo = Repo::new("complete-send");
    let digest = repo.add("hello", "0.1.0");
    let commit = repo.commit();
    let server = Arc::new(FakeServer::default());
    server.pushed(&repo, &commit, &digest);

    let report =
        sync_then_push(&repo.0, Arc::clone(&server), "rivet-01", true).expect_err("the fake push fails");
    assert!(report.contains("SYNCED"), "the objects half must have completed: {report}");
    assert!(
        server.exchanged() > 0,
        "the push half must have been attempted once the sync completed: {report}"
    );
}

/// **Without `confirm`, nothing is sent** — `sync::sync`'s own plan text
/// says so, and no object ever reaches `put`. (`push::push`'s own plan
/// reads the server's `main` even to print one, which this fake — built to
/// prove reachability, not to speak real git — cannot answer; that half is
/// `fleet_push_tests.rs`'s `a_plan_pushes_nothing_and_confirm_creates_then_
/// fast_forwards_the_servers_main`'s job, against a real git server.)
#[test]
fn without_confirm_nothing_is_sent() {
    let repo = Repo::new("plan");
    let digest = repo.add("hello", "0.1.0");
    let commit = repo.commit();
    let server = Arc::new(FakeServer::default());
    server.pushed(&repo, &commit, &digest);

    let report = sync_then_push(&repo.0, Arc::clone(&server), "rivet-01", false)
        .expect_err("the fake cannot answer even a read-only plan");
    assert!(report.contains("would send"), "{report}");
    assert!(report.contains("Nothing was sent"), "{report}");
    assert!(server.puts().is_empty(), "a plan sent {:?}", server.puts());
}

/// **`--sync`'s posture on uncommitted files**: `sync_then_push` passes
/// `strict = false` into `push::push`, so an uncommitted or untracked file
/// is named as left behind rather than refusing the whole run — the
/// standalone `fleet push` (`fleet_push_tests.rs`'s
/// `uncommitted_changes_are_refused_before_the_server_is_asked`) keeps the
/// strict refusal; this is the other caller `push()`'s `strict` flag exists
/// for.
#[test]
fn uncommitted_files_are_named_left_behind_rather_than_blocking() {
    let repo = Repo::new("uncommitted");
    let digest = repo.add("hello", "0.1.0");
    let commit = repo.commit();
    // A stray file, never committed — exactly what `fleet push` alone refuses.
    std::fs::write(repo.0.join("stray.txt"), b"never committed").expect("a stray file");
    let server = Arc::new(FakeServer::default());
    server.pushed(&repo, &commit, &digest);

    let report =
        sync_then_push(&repo.0, Arc::clone(&server), "rivet-01", true).expect_err("the fake push fails");
    assert!(report.contains("left behind"), "{report}");
    assert!(report.contains("stray.txt"), "{report}");
}
