//! `cargo rivet artifacts`, driven by a script — `ADR-052` §7.
//!
//! **What stands in**: the terminal is a list of typed answers and a
//! transcript; the fleet's server is an in-memory fake that counts every send,
//! push and prune. The fleet repository, its commits and its objects are real,
//! in a scratch directory, with hand-composed entries. What this cannot prove
//! is a real terminal's behaviour, which `tests/pty_interview.rs`'s harness
//! is the shape for and which is not written for this menu.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rivet_core::artifact::Digest;
use rivet_core::repository_objects::{Held, Listing, Manifest};

use super::super::{MANIFEST_FILE, OBJECTS_DIR, git, init, read_hidden, read_manifest, render_manifest};
use super::*;

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!("fleet-artifacts-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn hex_of(bytes: &[u8]) -> String {
    use rivet_core::package::port::Digest as _;
    crate::key::hex(&rivet_crypto::Blake3::new().of(bytes))
}

fn no_key() -> Result<PublicKey, String> {
    panic!("the application key was asked for, and no apk is retired here")
}

/// The typed answers, in order, and everything shown.
struct Script {
    answers: VecDeque<String>,
    transcript: String,
}

impl Script {
    fn new(answers: &[&str]) -> Self {
        Self { answers: answers.iter().map(|a| (*a).to_owned()).collect(), transcript: String::new() }
    }
}

impl Console for Script {
    fn say(&mut self, text: &str) {
        self.transcript.push_str(text);
    }

    fn ask(&mut self, prompt: &str) -> Result<String, String> {
        self.transcript.push_str(prompt);
        let answer = self.answers.pop_front().ok_or_else(|| {
            format!("the script ran out of answers at {prompt:?}; transcript so far:\n{}", self.transcript)
        })?;
        self.transcript.push_str(&answer);
        self.transcript.push('\n');
        Ok(answer)
    }
}

/// A fleet repository with hand-composed entries and objects.
fn repository(label: &str, products: &[(&str, &str, &str)], hidden: &[(&str, &str)]) -> (PathBuf, Vec<String>) {
    let root = scratch(label).join("fleet");
    init(&root).expect("fleet init");
    let mut entries = Vec::new();
    let mut digests = Vec::new();
    for (n, (product, version, kind)) in products.iter().enumerate() {
        let bytes = format!("{product} {version} {kind}").into_bytes();
        let digest = hex_of(&bytes);
        std::fs::write(root.join(OBJECTS_DIR).join(&digest), &bytes).expect("an object");
        entries.push(Entry {
            digest: digest.clone(),
            product: (*product).to_owned(),
            version: (*version).to_owned(),
            arch: if *kind == "apk" { "any" } else { "aarch64" }.to_owned(),
            platform: (*kind == "apk").then(|| "android".to_owned()),
            kind: (*kind).to_owned(),
            size: bytes.len() as u64,
            added: format!("2026-09-27T00:00:0{n}Z"),
            schema_version: None,
        });
        digests.push(digest);
    }
    let hidden: Vec<super::super::Hidden> = hidden
        .iter()
        .map(|(p, k)| super::super::Hidden { product: (*p).to_owned(), kind: (*k).to_owned() })
        .collect();
    std::fs::write(root.join(MANIFEST_FILE), render_manifest(&entries, &hidden)).expect("manifest");
    git(&root, &["commit", "-q", "-am", "fixture"]).expect("commit");
    (root, digests)
}

/// **The fleet's server, in memory**, holding `held` and naming what this
/// repository names once pushed.
struct FakeFleet {
    root: PathBuf,
    sends: RefCell<Vec<bool>>,
    commit: RefCell<String>,
    named: RefCell<Vec<Digest>>,
    held: RefCell<Vec<Held>>,
    prunes: RefCell<usize>,
}

impl FakeFleet {
    fn holding(root: &Path, digests: &[String]) -> Self {
        Self {
            root: root.to_path_buf(),
            sends: RefCell::new(Vec::new()),
            commit: RefCell::new("0".repeat(40)),
            named: RefCell::new(Vec::new()),
            held: RefCell::new(
                digests.iter().map(|d| Held { digest: Digest::new(d).expect("digest"), name: format!("{d}.rvt") }).collect(),
            ),
            prunes: RefCell::new(0),
        }
    }
}

impl FleetServer for &FakeFleet {
    fn label(&self) -> &str {
        "rivet-01"
    }

    fn send_and_push(&self, root: &Path, confirm: bool) -> Result<String, String> {
        assert_eq!(root, self.root, "pushed from another repository");
        self.sends.borrow_mut().push(confirm);
        let entries = read_manifest(root).expect("entries");
        let hidden = read_hidden(root).expect("hidden");
        let hidden: Vec<(&str, &str)> = hidden.iter().map(|h| (h.product.as_str(), h.kind.as_str())).collect();
        *self.named.borrow_mut() = rivet_core::repository_objects::named(
            entries.iter().map(|e| (e.digest.as_str(), e.product.as_str(), e.kind.as_str())),
            &hidden,
        )
        .expect("names")
        .into_iter()
        .collect();
        *self.commit.borrow_mut() = git(root, &["rev-parse", "HEAD"]).expect("rev-parse").trim().to_owned();
        Ok("PUSHED (fake)\n".to_owned())
    }

    fn listing(&self) -> Result<Listing, String> {
        Ok(Listing {
            manifest: Manifest::Read {
                commit: self.commit.borrow().clone(),
                named: self.named.borrow().iter().cloned().collect(),
            },
            held: self.held.borrow().clone(),
        })
    }

    fn prune(&self) -> Result<String, String> {
        *self.prunes.borrow_mut() += 1;
        let remove = self.listing()?.prune()?;
        Ok(remove.iter().map(|h| format!("removed {} {}\n", h.digest, h.name)).collect())
    }
}

fn run_script(
    root: &Path,
    answers: &[&str],
    register: &rivetos_register::Register,
    server: Option<&'static FakeFleet>,
) -> (Result<String, String>, String) {
    let mut console = Script::new(answers);
    let mut connect = || -> Result<Box<dyn FleetServer>, String> {
        match server {
            Some(fake) => Ok(Box::new(fake)),
            None => panic!("the fleet's server was asked for, and nothing should have reached it"),
        }
    };
    let outcome = interview(&mut console, root, register, &mut no_key, &mut connect);
    (outcome, console.transcript)
}

fn leak(fake: FakeFleet) -> &'static FakeFleet {
    Box::leak(Box::new(fake))
}

fn register(label: &str) -> rivetos_register::Register {
    rivetos_register::Register::empty(scratch(label))
}

/// **The list groups by kind, folds hidden products into one line that opens,
/// and `x` leaves with nothing changed and nothing reached.**
#[test]
fn the_list_folds_hidden_products_and_x_leaves_changing_nothing() {
    let (root, _) = repository(
        "list",
        &[("hello", "0.1.0", "application"), ("hello", "0.2.0", "application"), ("testbed", "0.1.3", "apk"), ("gantry", "0.1.17", "apk")],
        &[("testbed", "apk")],
    );
    let (outcome, transcript) = run_script(&root, &["3", "x", "x"], &register("list"), None);
    assert_eq!(outcome.expect("exits"), "\nExited. Nothing was changed.\n");
    assert!(transcript.contains("\n  application\n    1) hello — 2 versions, newest 0.2.0\n"), "{transcript}");
    assert!(transcript.contains("\n  apk\n    2) gantry — 1 version, newest 0.1.17\n"), "{transcript}");
    assert!(transcript.contains("    3) 1 hidden — withheld from every fleet; open them\n"), "{transcript}");
    assert!(!transcript.contains("2) testbed"), "a hidden product is on the main list: {transcript}");
    assert!(transcript.contains("    1) testbed (apk) — 1 version, newest 0.1.3\n    x) back to the list"), "{transcript}");
    assert!(transcript.contains("    x) exit\n"), "{transcript}");
}

/// **A deleted version, then the way out: push `(Y/n)` defaults to yes, the
/// exact prune list is shown, and the prune `(y/N)` defaults to NO** — a bare
/// Return never prunes.
#[test]
fn deleting_a_version_then_leaving_pushes_shows_the_prune_list_and_enter_does_not_prune() {
    let (root, digests) = repository(
        "delete-version",
        &[("hello", "0.1.0", "application"), ("hello", "0.2.0", "application")],
        &[],
    );
    let server = leak(FakeFleet::holding(&root, &digests));
    // hello; delete one version; the second listed (0.1.0 — newest first); yes;
    // back; exit; push (Enter); prune (Enter).
    let answers = ["1", "2", "2", "y", "x", "x", "", ""];
    let (outcome, transcript) = run_script(&root, &answers, &register("delete-version"), Some(server));
    outcome.expect("exits");
    assert!(transcript.contains("delete hello 0.1.0 (application) from the fleet repository? (y/N)"), "{transcript}");
    assert!(transcript.contains("RETIRED  hello (application) 0.1.0"), "{transcript}");
    assert_eq!(read_manifest(&root).expect("entries").len(), 1);
    assert!(transcript.contains("push and sync to rivet-01? (Y/n)"), "{transcript}");
    assert_eq!(*server.sends.borrow(), vec![true]);
    assert!(transcript.contains(&format!("would prune {}", digests[0])), "{transcript}");
    assert!(!transcript.contains(&format!("would prune {}", digests[1])), "{transcript}");
    assert!(transcript.contains("prune it now? (y/N)"), "{transcript}");
    assert_eq!(*server.prunes.borrow(), 0, "Enter at the prune must never prune");
    assert!(transcript.contains("left in rivet-01's store"), "{transcript}");
}

/// **The prune runs only on a typed yes, as its own step**, and removes
/// exactly what was listed.
#[test]
fn the_prune_runs_only_on_a_typed_yes() {
    let (root, digests) =
        repository("prune-yes", &[("gantry-dev", "0.1.0", "application"), ("hello", "0.1.0", "application")], &[]);
    let server = leak(FakeFleet::holding(&root, &digests));
    // gantry-dev (listed first); delete the product; yes; exit; push; prune yes.
    let answers = ["1", "3", "y", "x", "", "y"];
    let (outcome, transcript) = run_script(&root, &answers, &register("prune-yes"), Some(server));
    outcome.expect("exits");
    assert!(transcript.contains("RETIRED  gantry-dev (application) — 1 entry removed"), "{transcript}");
    assert_eq!(*server.prunes.borrow(), 1);
    assert!(transcript.contains(&format!("removed {}", digests[0])), "{transcript}");
}

/// **A refusal from the core is shown as it came**, nothing changes, and the
/// server is never reached.
#[test]
fn a_refusal_is_shown_verbatim_and_nothing_is_sent() {
    let (root, _) = repository("refused", &[("hello", "0.1.0", "application")], &[]);
    let dir = root.join(rivetos_fleet_repo::DOCUMENTS_DIR);
    std::fs::create_dir_all(&dir).expect("documents/");
    std::fs::write(dir.join("kiosk-01.provisioning.toml"), "version = 1\n[carry.hello]\nversion = \"0.1.0\"\n")
        .expect("document");
    let (outcome, transcript) = run_script(&root, &["1", "3", "y", "x", "x"], &register("refused"), None);
    assert_eq!(outcome.expect("exits"), "\nExited. Nothing was changed.\n");
    assert!(transcript.contains("REFUSED: hello is what 1 machine card(s) carry"), "{transcript}");
    assert!(transcript.contains("kiosk-01"), "{transcript}");
    assert_eq!(read_manifest(&root).expect("entries").len(), 1);
}

/// **A hide, then declining the push**: the hidden set is committed, nothing
/// is sent, and the commands for later are named.
#[test]
fn hiding_then_declining_the_push_sends_nothing() {
    let (root, digests) = repository("hide", &[("testbed", "0.1.0", "application")], &[]);
    let server = leak(FakeFleet::holding(&root, &digests));
    let (outcome, transcript) = run_script(&root, &["1", "1", "x", "x", "n"], &register("hide"), Some(server));
    outcome.expect("exits");
    assert!(transcript.contains("HIDDEN  testbed (application)"), "{transcript}");
    assert_eq!(read_hidden(&root).expect("hidden").len(), 1);
    assert!(server.sends.borrow().is_empty());
    assert!(transcript.contains("nothing was sent; `cargo rivet fleet push <server> confirm`"), "{transcript}");
}

/// **An action menu takes nothing on a bare Return**, and a delete answered
/// with anything but yes is not a delete.
#[test]
fn a_bare_return_on_an_action_menu_does_nothing_and_a_delete_needs_yes() {
    let (root, _) = repository("careful", &[("hello", "0.1.0", "application")], &[]);
    let (outcome, transcript) = run_script(&root, &["1", "", "3", "", "3", "yeah", "x", "x"], &register("careful"), None);
    assert_eq!(outcome.expect("exits"), "\nExited. Nothing was changed.\n");
    assert!(transcript.contains("nothing was chosen: a number, or x"), "{transcript}");
    assert_eq!(transcript.matches("  not deleted\n").count(), 2, "{transcript}");
    assert_eq!(read_manifest(&root).expect("entries").len(), 1);
}
