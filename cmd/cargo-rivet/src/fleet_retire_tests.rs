//! `fleet retire`, `hide` and `unhide`, against real fleet repositories in
//! scratch directories — `ADR-052` §7, `.rulings/` R-225.
//!
//! The manifests are written by the module's own `render_manifest`, over
//! hand-composed entries whose objects are plain bytes named by their real
//! BLAKE3 (a retire reads no object, except an `apk`'s, and those are real
//! signed artifacts published here with a seed minted here). Documents are
//! hand-composed in the shape `rivet-config` parses; registers are built in
//! memory. The fleet's server is an in-memory fake.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rivet_core::artifact::Digest;
use rivet_core::repository_objects::Held;

use super::super::{MANIFEST_FILE, OBJECTS_DIR, git, init, read_hidden, read_manifest, render_manifest};
use super::*;

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!("fleet-retire-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn hex_of(bytes: &[u8]) -> String {
    use rivet_core::package::port::Digest as _;
    crate::key::hex(&rivet_crypto::Blake3::new().of(bytes))
}

/// A key nothing in these tests should ever be asked for.
fn no_key() -> Result<PublicKey, String> {
    panic!("the application key was asked for, and nothing here retires an apk's last version")
}

/// A fleet repository with hand-composed entries, committed on `main`.
struct Repo(PathBuf);

impl Repo {
    fn new(label: &str) -> Self {
        let root = scratch(label).join("fleet");
        init(&root).expect("fleet init");
        Self(root)
    }

    /// Add an entry and its object, and commit the manifest.
    fn add(&self, product: &str, version: &str, kind: &str) -> String {
        let bytes = format!("{product} {version} {kind}").into_bytes();
        let digest = hex_of(&bytes);
        std::fs::write(self.0.join(OBJECTS_DIR).join(&digest), &bytes).expect("an object");
        let mut entries = read_manifest(&self.0).expect("entries");
        entries.push(Entry {
            digest: digest.clone(),
            product: product.to_owned(),
            version: version.to_owned(),
            arch: if kind == "apk" { "any" } else { "aarch64" }.to_owned(),
            platform: (kind == "apk").then(|| "android".to_owned()),
            kind: kind.to_owned(),
            size: bytes.len() as u64,
            added: format!("2026-09-27T00:00:0{}Z", entries.len() % 10),
            schema_version: None,
        });
        let hidden = read_hidden(&self.0).expect("hidden");
        std::fs::write(self.0.join(MANIFEST_FILE), render_manifest(&entries, &hidden)).expect("manifest");
        git(&self.0, &["commit", "-q", "-am", "add"]).expect("commit");
        digest
    }

    /// Commit a machine document under `documents/`.
    fn document(&self, machine: &str, text: &str) {
        let dir = self.0.join(rivetos_fleet_repo::DOCUMENTS_DIR);
        std::fs::create_dir_all(&dir).expect("documents/");
        std::fs::write(dir.join(format!("{machine}{}", rivetos_fleet_repo::DOCUMENT_SUFFIX)), text).expect("document");
    }

    fn object(&self, digest: &str) -> PathBuf {
        self.0.join(OBJECTS_DIR).join(digest)
    }

    fn products(&self) -> Vec<(String, String)> {
        read_manifest(&self.0).expect("entries").into_iter().map(|e| (e.product, e.version)).collect()
    }
}

fn empty_register(label: &str) -> rivetos_register::Register {
    rivetos_register::Register::empty(scratch(label))
}

fn device(id: &str, name: &str) -> rivetos_register::Device {
    rivetos_register::Device { id: id.to_owned(), name: name.to_owned(), ..rivetos_register::Device::default() }
}

// ---------------------------------------------------------------------------
// retire
// ---------------------------------------------------------------------------

/// **A product's entries and objects go, and the commit carries the manifest
/// alone** — an unrelated staged file stays staged.
#[test]
fn retiring_a_product_removes_every_entry_and_object_and_commits_only_the_manifest() {
    let repo = Repo::new("product");
    let one = repo.add("gantry-dev", "0.1.0", "application");
    let two = repo.add("gantry-dev", "0.1.1", "application");
    let kept = repo.add("hello", "0.1.0", "application");
    std::fs::write(repo.0.join("unrelated.txt"), b"not this retire's").expect("a file");
    git(&repo.0, &["add", "unrelated.txt"]).expect("staged");

    let report = retire(&repo.0, "gantry-dev", None, None, &empty_register("product"), &mut no_key)
        .expect("retired");
    assert!(report.contains("RETIRED  gantry-dev (application) — 2 entries removed"), "{report}");
    assert!(report.contains(&format!("removed   {one} gantry-dev 0.1.0")), "{report}");
    assert!(report.contains("What the fleets lose"), "{report}");
    assert!(report.contains("Nothing installed changes"), "{report}");

    assert_eq!(repo.products(), vec![("hello".to_owned(), "0.1.0".to_owned())]);
    assert!(!repo.object(&one).exists() && !repo.object(&two).exists());
    assert!(repo.object(&kept).is_file());

    let committed = git(&repo.0, &["show", "--name-only", "--format=%s", "HEAD"]).expect("show");
    assert!(committed.starts_with("Retire gantry-dev (application) from the fleet repository"), "{committed}");
    assert!(committed.contains(MANIFEST_FILE) && !committed.contains("unrelated.txt"), "{committed}");
    let status = git(&repo.0, &["status", "--porcelain"]).expect("status");
    assert!(status.lines().any(|l| l.starts_with('A') && l.ends_with("unrelated.txt")), "{status}");
}

/// **`--version` removes that version alone**, every build of it.
#[test]
fn retiring_one_version_leaves_the_others() {
    let repo = Repo::new("version");
    let old = repo.add("hello", "0.1.0", "application");
    let new = repo.add("hello", "0.2.0", "application");
    let report = retire(&repo.0, "hello", None, Some("0.1.0"), &empty_register("version"), &mut no_key)
        .expect("retired");
    assert!(report.contains("RETIRED  hello (application) 0.1.0 — 1 entry removed"), "{report}");
    assert_eq!(repo.products(), vec![("hello".to_owned(), "0.2.0".to_owned())]);
    assert!(!repo.object(&old).exists());
    assert!(repo.object(&new).is_file());

    let missing = retire(&repo.0, "hello", None, Some("9.9.9"), &empty_register("version"), &mut no_key)
        .expect_err("not held");
    assert!(missing.contains("it holds 0.2.0") && missing.contains("Nothing was changed"), "{missing}");
}

/// **One name under two kinds is refused without `--kind`**, and `--kind`
/// retires that kind alone.
#[test]
fn a_name_held_as_two_kinds_needs_a_kind() {
    let repo = Repo::new("ambiguous");
    repo.add("testbed", "0.1.0", "application");
    repo.add("testbed", "0.1.3", "apk");
    let refused = retire(&repo.0, "testbed", None, None, &empty_register("ambiguous"), &mut no_key)
        .expect_err("ambiguous");
    assert!(refused.contains("as 2 kinds — apk, application") && refused.contains("--kind"), "{refused}");
    assert_eq!(repo.products().len(), 2, "nothing was changed");

    retire(&repo.0, "testbed", Some("application"), None, &empty_register("ambiguous"), &mut no_key)
        .expect("retired");
    let left = read_manifest(&repo.0).expect("entries");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, "apk");
}

/// **A document that pins the product refuses its retire**, naming the
/// machine and how to unpin — and a version it does not pin can still go.
#[test]
fn a_document_pin_refuses_the_retire_and_names_the_machine() {
    let repo = Repo::new("document-pin");
    repo.add("hello", "0.1.0", "application");
    repo.add("hello", "0.2.0", "application");
    repo.document("kiosk-01", "version = 1\n[carry.hello]\nversion = \"0.2.0\"\n");
    let register = empty_register("document-pin");

    for version in [None, Some("0.2.0")] {
        let refused = retire(&repo.0, "hello", None, version, &register, &mut no_key).expect_err("pinned");
        assert!(refused.contains("REFUSED"), "{refused}");
        assert!(refused.contains("kiosk-01"), "{refused}");
        assert!(refused.contains("[carry.hello] version = \"0.2.0\""), "{refused}");
        assert!(refused.contains("unpin:"), "{refused}");
        assert!(refused.contains("Nothing was changed"), "{refused}");
    }
    assert_eq!(repo.products().len(), 2, "a refusal changed nothing");

    retire(&repo.0, "hello", None, Some("0.1.0"), &register, &mut no_key).expect("an unpinned version goes");
}

/// **The register's pins refuse it too** — a device's own and the fleet's —
/// each with its `rivet-fleet` command.
#[test]
fn a_register_pin_refuses_the_retire_and_names_the_unpin_command() {
    let repo = Repo::new("register-pin");
    repo.add("hello", "0.1.0", "application");
    let mut register = empty_register("register-pin");
    let mut pinned = device("0011223344556677", "dev-01");
    pinned.carry.insert("hello".to_owned(), "0.1.0".to_owned());
    register.enroll(pinned).expect("enroll");
    register.fleet.carry.insert("hello".to_owned(), "0.1.0".to_owned());

    let refused = retire(&repo.0, "hello", None, None, &register, &mut no_key).expect_err("pinned");
    assert!(refused.contains("dev-01"), "{refused}");
    assert!(refused.contains("rivet-fleet set carry --device dev-01 --product hello --version none"), "{refused}");
    assert!(refused.contains("rivet-fleet set fleet-carry --product hello --version none"), "{refused}");
    assert_eq!(repo.products().len(), 1);
}

/// **An enabled, unpinned service carries whichever single version is held**:
/// its product cannot go, nor its last version, but one of two versions can.
#[test]
fn a_carried_but_unpinned_product_refuses_losing_its_last_version_only() {
    let repo = Repo::new("carried");
    repo.add("rivet-fleetd", "0.1.0", "application");
    repo.add("rivet-fleetd", "0.1.1", "application");
    repo.document("rivet-01", "version = 1\nhostname = \"rivet-01\"\n\n[services.rivet-fleetd]\nenabled = true\n");
    let mut register = empty_register("carried");
    let mut kiosk = device("1122334455667788", "kiosk-02");
    kiosk.kiosk_auto_run = Some("rivet-fleetd".to_owned());
    register.enroll(kiosk).expect("enroll");

    let refused = retire(&repo.0, "rivet-fleetd", None, None, &register, &mut no_key).expect_err("carried");
    assert!(refused.contains("rivet-01") && refused.contains("enables [services.rivet-fleetd]"), "{refused}");
    assert!(refused.contains("kiosk-02") && refused.contains("kiosk auto-run"), "{refused}");

    retire(&repo.0, "rivet-fleetd", None, Some("0.1.0"), &register, &mut no_key).expect("one of two goes");
    let last = retire(&repo.0, "rivet-fleetd", None, Some("0.1.1"), &register, &mut no_key)
        .expect_err("the last one is carried");
    assert!(last.contains("rivet-01"), "{last}");
}

/// **Only an application is ever carried** — an `apk` of a name a document
/// enables is not refused on that ground.
#[test]
fn a_document_enabling_the_name_never_refuses_another_kind() {
    let repo = Repo::new("other-kind");
    repo.add("hello", "0.1.0", "library");
    repo.document("rivet-01", "version = 1\n[services.hello]\nenabled = true\n");
    retire(&repo.0, "hello", None, None, &empty_register("other-kind"), &mut no_key).expect("a library goes");
}

/// A document that does not parse refuses the retire of an application,
/// naming the file: what it carries cannot be known.
#[test]
fn an_unreadable_document_refuses_rather_than_guessing() {
    let repo = Repo::new("bad-document");
    repo.add("hello", "0.1.0", "application");
    repo.document("rivet-01", "this is not = = toml\n");
    let refused = retire(&repo.0, "hello", None, None, &empty_register("bad-document"), &mut no_key)
        .expect_err("refused");
    assert!(refused.contains("rivet-01.provisioning.toml does not parse"), "{refused}");
    assert_eq!(repo.products().len(), 1);
}

/// **An `apk`'s signer goes with its last version, and is printed.** The APK
/// is hand-composed; the artifact, its signature and the repository are real.
#[test]
fn retiring_an_apk_prints_the_signer_it_drops() {
    const SEED: [u8; 32] = [0x6b; 32];
    let work = scratch("apk");
    let root = work.join("fleet");
    init(&root).expect("fleet init");
    let seed = work.join("app.seed");
    std::fs::write(&seed, SEED).expect("a seed");
    let apk = work.join("app.apk");
    std::fs::write(&apk, crate::apk_read::tests::hand_apk_v2("com.example.testbed", 3, "0.1.3", b"key one"))
        .expect("an apk");
    super::super::publish_apk::publish(&root, &apk, None, &mut || Ok(seed.clone())).expect("published");

    use rivet_core::package::port::Sign as _;
    let mut key = || Ok(rivet_crypto::Ed25519Signer::from_seed(&SEED).public_key());
    let report = retire(&root, "testbed", None, None, &empty_register("apk"), &mut key).expect("retired");
    let signer = crate::apk_read::sha256_hex(b"key one");
    assert!(report.contains(&format!("signer    {signer} (pinned by testbed 0.1.3) is dropped")), "{report}");
    assert!(report.contains("pins afresh"), "{report}");
    assert!(report.contains("A phone holding testbed keeps it"), "{report}");
    assert!(read_manifest(&root).expect("entries").is_empty());
}

/// **Retiring a hidden product drops it from the hidden set.**
#[test]
fn retiring_a_hidden_product_drops_it_from_the_hidden_set() {
    let repo = Repo::new("hidden");
    repo.add("testbed", "0.1.0", "application");
    repo.add("hello", "0.1.0", "application");
    hide(&repo.0, "testbed", None).expect("hidden");
    let report = retire(&repo.0, "testbed", None, None, &empty_register("hidden"), &mut no_key).expect("retired");
    assert!(report.contains("unhidden  nothing of testbed (application) remains"), "{report}");
    assert!(report.contains("it was hidden, so no fleet was offered it"), "{report}");
    assert!(read_hidden(&repo.0).expect("hidden").is_empty());
}

// ---------------------------------------------------------------------------
// hide and unhide
// ---------------------------------------------------------------------------

/// **Hide and unhide round-trip**, each committing the manifest alone, and a
/// second hide changes nothing. A hide is allowed even for a pinned product.
#[test]
fn hide_and_unhide_round_trip() {
    let repo = Repo::new("round-trip");
    let digest = repo.add("hello", "0.1.0", "application");
    repo.document("kiosk-01", "version = 1\n[carry.hello]\nversion = \"0.1.0\"\n");
    let before = std::fs::read_to_string(repo.0.join(MANIFEST_FILE)).expect("manifest");

    let report = hide(&repo.0, "hello", None).expect("a pinned product may be hidden");
    assert!(report.contains("HIDDEN  hello (application)"), "{report}");
    assert!(report.contains(&format!("{:<9} {digest}", "kept")), "{report}");
    assert_eq!(read_hidden(&repo.0).expect("hidden").len(), 1);
    assert!(repo.object(&digest).is_file(), "a hide keeps the object");
    let subject = git(&repo.0, &["log", "-1", "--format=%s"]).expect("log");
    assert_eq!(subject.trim(), "Hide hello (application) from every fleet");

    let again = hide(&repo.0, "hello", None).expect("a report");
    assert!(again.contains("already hidden. Nothing was changed"), "{again}");

    let report = unhide(&repo.0, "hello", None).expect("unhidden");
    assert!(report.contains("UNHIDDEN  hello (application)"), "{report}");
    assert!(read_hidden(&repo.0).expect("hidden").is_empty());
    assert_eq!(std::fs::read_to_string(repo.0.join(MANIFEST_FILE)).expect("manifest"), before);

    let not = unhide(&repo.0, "hello", None).expect("a report");
    assert!(not.contains("is not hidden. Nothing was changed"), "{not}");
    let absent = hide(&repo.0, "nonesuch", None).expect_err("not held");
    assert!(absent.contains("holds no \"nonesuch\""), "{absent}");
}

// ---------------------------------------------------------------------------
// --sync: push, then the prune's plan, and never the prune
// ---------------------------------------------------------------------------

/// **The fleet's server, in memory.** A push makes its manifest this
/// repository's, named by the server's own rule; a prune is counted, and
/// these tests assert it is never reached.
struct FakeFleet {
    sends: RefCell<Vec<bool>>,
    commit: RefCell<String>,
    named: RefCell<Vec<Digest>>,
    held: RefCell<Vec<Held>>,
    prunes: RefCell<usize>,
}

impl FakeFleet {
    fn holding(held: &[(&str, &str)], root: &Path) -> Self {
        let fake = Self {
            sends: RefCell::new(Vec::new()),
            commit: RefCell::new("0".repeat(40)),
            named: RefCell::new(Vec::new()),
            held: RefCell::new(
                held.iter().map(|(d, n)| Held { digest: Digest::new(d).expect("digest"), name: (*n).to_owned() }).collect(),
            ),
            prunes: RefCell::new(0),
        };
        fake.take_text_from(root);
        fake
    }

    fn take_text_from(&self, root: &Path) {
        *self.named.borrow_mut() = named_here(root).expect("names").into_iter().collect();
        *self.commit.borrow_mut() =
            git(root, &["rev-parse", "HEAD"]).expect("rev-parse").trim().to_owned();
    }
}

impl FleetServer for FakeFleet {
    fn label(&self) -> &str {
        "rivet-01"
    }

    fn send_and_push(&self, root: &Path, confirm: bool) -> Result<String, String> {
        self.sends.borrow_mut().push(confirm);
        if confirm {
            self.take_text_from(root);
            Ok("PUSHED (fake)\n".to_owned())
        } else {
            Ok("Nothing was pushed (fake plan).\n".to_owned())
        }
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
        Err("the fake refuses: --sync must never prune".to_owned())
    }
}

/// **`--sync confirm` pushes, prints the server's prune plan, and never
/// prunes.**
#[test]
fn sync_confirm_pushes_and_prints_the_prune_plan_and_never_prunes() {
    let repo = Repo::new("sync-confirm");
    let gone = repo.add("gantry-dev", "0.1.0", "application");
    let kept = repo.add("hello", "0.1.0", "application");
    let server = FakeFleet::holding(
        &[(&gone, "gantry-dev-0.1.0-aarch64-application.rvt"), (&kept, "hello-0.1.0-aarch64-application.rvt")],
        &repo.0,
    );
    retire(&repo.0, "gantry-dev", None, None, &empty_register("sync-confirm"), &mut no_key).expect("retired");

    let report = then_sync(&repo.0, &server, true).expect("synced");
    assert_eq!(*server.sends.borrow(), vec![true], "{report}");
    assert!(report.contains("PUSHED (fake)"), "{report}");
    assert!(report.contains("rivet-01's prune would remove 1 object(s)"), "{report}");
    assert!(report.contains(&format!("would prune {gone}")), "{report}");
    assert!(!report.contains(&format!("would prune {kept}")), "{report}");
    assert!(report.contains("NOTHING WAS PRUNED"), "{report}");
    assert_eq!(*server.prunes.borrow(), 0, "--sync must never prune");
}

/// **`--sync` without `confirm` pushes nothing** and says what the prune would
/// remove once the text is pushed — here a hidden product's object.
#[test]
fn sync_without_confirm_plans_the_push_and_the_prune_that_would_follow() {
    let repo = Repo::new("sync-plan");
    let testbed = repo.add("testbed", "0.1.3", "apk");
    let hello = repo.add("hello", "0.1.0", "application");
    let server = FakeFleet::holding(
        &[(&testbed, "testbed-0.1.3-any-android-apk.rvt"), (&hello, "hello-0.1.0-aarch64-application.rvt")],
        &repo.0,
    );
    hide(&repo.0, "testbed", None).expect("hidden");

    let report = then_sync(&repo.0, &server, false).expect("a plan");
    assert_eq!(*server.sends.borrow(), vec![false]);
    assert!(report.contains("Once pushed, rivet-01's prune would remove 1 object(s)"), "{report}");
    assert!(report.contains(&format!("would prune {testbed}")), "{report}");
    assert_eq!(*server.prunes.borrow(), 0);
}

/// The grammar: `--version` is `retire`'s, `confirm` is `--sync`'s.
#[test]
fn the_grammar_of_retire_hide_and_unhide() {
    let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    match super::super::parse(&words(&["retire", "hello", "--kind", "apk", "--version", "0.1.0", "--sync", "confirm"]))
        .expect("parses")
    {
        super::super::FleetArgs { verb: super::super::SubVerb::Circulation(c), .. } => {
            assert_eq!(c.product, "hello");
            assert_eq!(c.kind.as_deref(), Some("apk"));
            assert_eq!(c.action, super::super::CirculationAction::Retire { version: Some("0.1.0".to_owned()) });
            assert!(c.sync && c.sync_confirm);
        }
        _ => panic!("not a retire"),
    }
    let refused = super::super::parse(&words(&["hide", "hello", "--version", "1"])).err().expect("refused");
    assert!(refused.contains("--version"), "{refused}");
    let refused = super::super::parse(&words(&["retire", "hello", "confirm"])).err().expect("refused");
    assert!(refused.contains("`confirm` is `--sync`'s"), "{refused}");
    let refused = super::super::parse(&words(&["unhide"])).err().expect("refused");
    assert!(refused.contains("needs a product"), "{refused}");
}
