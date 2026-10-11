//! Tests for `crate::carry` — Wave C3 (`.rulings/` R-178 items 1, P1, P5).
//!
//! **No image, no GPT, no FAT anywhere in this file.** [`gather`] never opens
//! an image — it only reads credential and fleet-repository files off disk
//! and returns bytes — so every test here is against real scratch files with
//! no partition machinery at all. `write_image_tests.rs` is where the widened
//! `inject` itself (against a real two-partition image) is proven.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::credentials::Overrides;
use rivet_core::package::application::{Manifest, ProgramKind, TargetClass, VARIANT_NONE};
use rivet_core::package::write::{Builder, NewMember};
use std::sync::atomic::{AtomicU32, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!(
        "carry-test-{tag}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A complete scratch `~/.rivet`-shaped host — every required credential,
/// including the board identity `gather` reads for item 1's identity trio —
/// `crate::setup::run`'s own cold-run path, the same one `setup_tests.rs`'s
/// own fixture takes.
fn complete_host(at: &Path) {
    crate::setup::run(&crate::setup::SetupArgs {
        at: Some(at.to_path_buf()),
        name: Some("carry-test-operator".to_owned()),
        check: false,
        over: Overrides::default(),
    })
    .expect("a cold setup run mints every required credential");
}

/// A stand-alone document enabling one application the card must carry,
/// pinned or not. **Not the proxy**, which is part of the image (`ADR-060`) and
/// never carried; any application off `IMAGE_SERVICE_NAMES` is carried alike.
fn carrying_document(pin: Option<&str>) -> rivet_config::Document {
    let carry = pin.map_or_else(String::new, |v| format!("\n[carry.{CARRIED}]\nversion = \"{v}\"\n"));
    let text = format!(
        "version = 1\nhostname = \"example-standalone\"\n\n[services.{CARRIED}]\nenabled = true\n{carry}"
    );
    rivet_config::Document::parse(&text).expect("a valid carrying document")
}

/// The application [`carrying_document`] carries.
const CARRIED: &str = "example-app";

/// A member document: `rivet-proxyd` enabled, and — since `ADR-060` — nothing
/// carried for it.
fn member_document(pin: Option<&str>) -> rivet_config::Document {
    let carry = pin.map_or_else(String::new, |v| format!("\n[carry.rivet-proxyd]\nversion = \"{v}\"\n"));
    let text = format!(
        "version = 1\nhostname = \"example-member\"\n\n[network.eth0]\nmethod = \"dhcp\"\n\n\
         [control-plane]\nlisten = \"0.0.0.0:7777\"\n\n[applications.rivet-proxyd]\n\
         upstream-name = \"example-member.local\"\n\n[services.rivet-proxyd]\nenabled = true\n{carry}"
    );
    rivet_config::Document::parse(&text).expect("a valid member document")
}

/// A server document — `rivet-fleetd` AND `rivet-proxyd`, both pinned.
fn server_document() -> rivet_config::Document {
    let text = "version = 1\nhostname = \"example-server\"\n\n[network.eth0]\nmethod = \"dhcp\"\n\n\
                [control-plane]\nlisten = \"0.0.0.0:7777\"\n\n[applications.rivet-proxyd]\n\
                upstream-name = \"example-server.local\"\n\n[services.rivet-fleetd]\nenabled = true\n\n\
                [services.rivet-proxyd]\nenabled = true\n\n[carry.rivet-fleetd]\nversion = \"1.0.0\"\n\n\
                [carry.rivet-proxyd]\nversion = \"1.0.0\"\n";
    rivet_config::Document::parse(text).expect("a valid server document")
}

fn standalone_document() -> rivet_config::Document {
    rivet_config::Document::parse("version = 1\nhostname = \"example-standalone\"\n")
        .expect("a valid stand-alone document")
}

/// Write an empty-but-valid `ADR-049` staging bundle — `require_staging` only
/// asks that one parse, and these tests are not about its own contents.
fn write_empty_staging_bundle(document_path: &Path) -> PathBuf {
    let staging = crate::provision::staging_path_beside(document_path).expect("a .provisioning.toml path");
    std::fs::write(&staging, rivet_core::staging::render(&[])).expect("write the staging bundle");
    staging
}

fn write_document_text(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write the document");
    path
}

/// A real, signed application payload — `application.rs::a_payload`'s own
/// shape, in `cmd/rivet-image`, restated here for the reason `carry.rs`'s own
/// module doc gives `verify_carried`: that crate is bin-only.
fn a_payload(product: &str, seed: &[u8; 32]) -> Vec<u8> {
    let manifest = Manifest::new(ProgramKind::Daemon).render().into_bytes();
    let program = b"\x7fELF not really, and nothing here execs it".to_vec();

    let mut digest = rivet_crypto::Blake3::new();
    let mut signer = rivet_crypto::Ed25519Signer::from_seed(seed);
    let mut builder =
        Builder::new(product, "1.0.0", "aarch64", None, rivet_core::package::Kind::application())
            .expect("valid header fields");
    builder
        .add(
            NewMember::stored(
                rivet_core::package::application::MANIFEST_NAME,
                TargetClass::Manifest.as_byte(),
                VARIANT_NONE,
                &manifest,
                &mut digest,
            )
            .expect("a manifest member"),
        )
        .expect("the manifest fits");
    builder
        .add(
            NewMember::stored(product, TargetClass::Program.as_byte(), VARIANT_NONE, &program, &mut digest)
                .expect("a program member"),
        )
        .expect("the program fits");
    builder.seal_in_memory(&mut digest, &mut signer, &[&manifest, &program]).expect("the payload seals")
}

/// A scratch fleet repository — `manifest.toml` beside `objects/`, built by
/// hand rather than through `crate::fleet::init`/`publish_verified`: these
/// tests are about `gather`, and `crate::fleet`'s own suite already proves
/// `publish` produces this exact shape.
struct Repo(PathBuf);

impl Repo {
    fn new(tag: &str) -> Self {
        let root = scratch(tag);
        std::fs::create_dir_all(root.join("objects")).expect("objects/");
        Self(root)
    }

    fn add_application(&self, product: &str, version: &str, seed: &[u8; 32]) -> String {
        let bytes = a_payload(product, seed);
        let hex = crate::key::hex(&blake3_of(&bytes));
        std::fs::write(self.0.join("objects").join(&hex), &bytes).expect("write the object");
        let manifest = self.0.join("manifest.toml");
        let mut text = std::fs::read_to_string(&manifest).unwrap_or_default();
        text.push_str(&format!(
            "\n[[artifact]]\ndigest = \"{hex}\"\nproduct = \"{product}\"\nversion = \"{version}\"\n\
             arch = \"aarch64\"\nkind = \"application\"\nsize = {}\nadded = \"2026-09-13\"\n",
            bytes.len(),
        ));
        std::fs::write(&manifest, text).expect("write the manifest");
        hex
    }

    /// An `[[artifact]]` entry with NO object behind it — a `card`/`update`
    /// entry (excluded by kind, `ADR-054` §5), or an `application`/`library`
    /// entry whose object `objects/` no longer holds (a pruned repository,
    /// `ADR-052` §7). Either way: this repository's manifest NAMES it and
    /// cannot hand a server its bytes.
    fn add_entry_without_object(&self, product: &str, kind: &str) -> String {
        let hex = "e".repeat(64);
        let manifest = self.0.join("manifest.toml");
        let mut text = std::fs::read_to_string(&manifest).unwrap_or_default();
        text.push_str(&format!(
            "\n[[artifact]]\ndigest = \"{hex}\"\nproduct = \"{product}\"\nversion = \"1.0.0\"\n\
             arch = \"aarch64\"\nkind = \"{kind}\"\nsize = 0\nadded = \"2026-09-13\"\n",
        ));
        std::fs::write(&manifest, text).expect("write the manifest");
        hex
    }
}

fn blake3_of(bytes: &[u8]) -> [u8; 32] {
    use rivet_core::package::port::Digest as _;
    rivet_crypto::Blake3::new().of(bytes)
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn default_args(at: &Path, fleet_repo: Option<&Path>) -> ProvisionArgs {
    ProvisionArgs {
        image_path: None,
        document_path: None,
        device_path: None,
        confirm: false,
        not_mine: false,
        at: Some(at.to_path_buf()),
        verify_only: false,
        elevated_write_task: None,
        elevated_verify_task: None,
        elevated_task_digest: None,
        staging_path: None,
        identity_path: None,
        fleet_repo_root: fleet_repo.map(Path::to_path_buf),
        offer_device_menu: false,
        image_from_repository: false,
    }
}

// ---------------------------------------------------------------------------
// Identity — always carried, on `credentials::Survey`'s own footing.
// ---------------------------------------------------------------------------

#[test]
fn a_stand_alone_document_carries_identity_and_nothing_else() {
    let at = scratch("standalone");
    complete_host(&at);
    let survey = Survey::take(&at, &Overrides::default());
    let doc = standalone_document();
    let doc_path = write_document_text(&at, "doc.provisioning.toml", "version = 1\nhostname = \"example-standalone\"\n");
    let args = default_args(&at, None);

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("a stand-alone document needs nothing more");

    let names: Vec<&str> = extras.files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"device.seed"), "{names:?}");
    assert!(names.contains(&"device.crt"), "{names:?}");
    assert!(names.contains(&"peers"), "{names:?}");
    assert!(!names.contains(&"staged.txt"), "a stand-alone document carries no application service, so no staging bundle: {names:?}");
    assert!(extras.report.contains("identity:"), "{}", extras.report);

    let _ = std::fs::remove_dir_all(&at);
}

#[test]
fn a_document_carrying_an_application_with_no_fleet_repository_is_refused_by_name() {
    let at = scratch("no-repo");
    complete_host(&at);
    let survey = Survey::take(&at, &Overrides::default());
    let doc = carrying_document(None);
    let doc_path = write_document_text(&at, "carrying.provisioning.toml", "irrelevant — parsed doc is what matters");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, None);

    let err = gather(&doc_path, &doc, &survey, &args, &at).expect_err("the application must be carried");
    assert!(err.contains(CARRIED), "{err}");
    assert!(err.contains("fleet repository"), "{err}");

    let _ = std::fs::remove_dir_all(&at);
}

/// **A member's card carries no application and needs no fleet repository**
/// — `ADR-060`: its front door is part of the image. It still carries its
/// staging bundle, because the proxy is handed its certificate from it.
#[test]
fn a_member_document_carries_nothing_and_needs_no_fleet_repository() {
    let at = scratch("member-no-repo");
    complete_host(&at);
    let survey = Survey::take(&at, &Overrides::default());
    let doc = member_document(None);
    let doc_path = write_document_text(&at, "member.provisioning.toml", "irrelevant");
    let staging = crate::provision::staging_path_beside(&doc_path).expect("a .provisioning.toml path");
    let certificate = rivet_core::staging::Staged::new("rivet-proxyd", "tls-cert", b"CERT".to_vec())
        .expect("a legal entry");
    std::fs::write(&staging, rivet_core::staging::render(&[certificate])).expect("the bundle");
    let args = default_args(&at, None);

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("a member carries nothing");
    let names: Vec<&str> = extras.files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(!names.iter().any(|n| n.ends_with(".rvt")), "{names:?}");
    assert!(names.contains(&"staged.txt"), "the member's proxy needs its certificate: {names:?}");

    let _ = std::fs::remove_dir_all(&at);
}

// ---------------------------------------------------------------------------
// R-178 P5 — signature verification against the workstation's own key.
// ---------------------------------------------------------------------------

#[test]
fn a_carried_application_signed_by_the_hosts_own_key_is_placed_after_verification() {
    let at = scratch("carry-ok");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("carry-ok");
    repo.add_application(CARRIED, "1.0.0", &app_seed_bytes);

    let survey = Survey::take(&at, &Overrides::default());
    let doc = carrying_document(Some("1.0.0"));
    let doc_path = write_document_text(&at, "carrying.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("the payload verifies against keys/rivet-app.pub");

    let card_file = format!("app-{CARRIED}.rvt");
    let carried = extras.files.iter().find(|(name, _)| *name == card_file);
    assert!(carried.is_some(), "{:?}", extras.files.iter().map(|(n, _)| n).collect::<Vec<_>>());
    assert_eq!(carried.unwrap().1, a_payload(CARRIED, &app_seed_bytes), "the bytes must be the object's, unchanged");
    assert!(extras.report.contains("carried:"), "{}", extras.report);
    assert!(extras.report.contains("NOTE:"), "P5's trust-store note must be printed once carrying happened: {}", extras.report);
    // **About the published image, never a build** (`R-175`): it names the
    // workstation's own key and no build variable.
    let key = at.join("keys").join("rivet-app.pub").display().to_string();
    assert!(extras.report.contains(&format!("checked against {key}")), "{}", extras.report);
    assert!(extras.report.contains("published trusting a different application key"), "{}", extras.report);
    for stale in ["RIVET_APP_KEY", "RIVET_TRUSTED_KEY", "built with"] {
        assert!(!extras.report.contains(stale), "{stale} in the report: {}", extras.report);
    }

    let _ = std::fs::remove_dir_all(&at);
}

#[test]
fn a_carried_application_signed_by_the_wrong_key_is_refused_before_anything_is_written() {
    let at = scratch("carry-wrong-key");
    complete_host(&at);
    let wrong_seed = [0x99u8; 32]; // Deliberately NOT keys/rivet-app.seed's own.

    let repo = Repo::new("carry-wrong-key");
    repo.add_application(CARRIED, "1.0.0", &wrong_seed);

    let survey = Survey::take(&at, &Overrides::default());
    let doc = carrying_document(Some("1.0.0"));
    let doc_path = write_document_text(&at, "carrying.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let err = gather(&doc_path, &doc, &survey, &args, &at).expect_err("signed by a key keys/rivet-app.pub does not vouch for");
    assert!(err.contains("refused"), "{err}");

    let _ = std::fs::remove_dir_all(&at);
}

#[test]
fn a_missing_workstation_trust_key_refuses_rather_than_carrying_unverified() {
    let at = scratch("carry-no-trust-key");
    complete_host(&at);
    // Remove the very file `verify_carried` must read — `keys/rivet-app.pub`.
    std::fs::remove_file(at.join("keys").join("rivet-app.pub")).expect("remove the public key");

    let repo = Repo::new("carry-no-trust-key");
    repo.add_application(CARRIED, "1.0.0", &[7u8; 32]);

    let survey = Survey::take(&at, &Overrides::default());
    let doc = carrying_document(Some("1.0.0"));
    let doc_path = write_document_text(&at, "carrying.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let err = gather(&doc_path, &doc, &survey, &args, &at).expect_err("no key to verify against");
    assert!(err.contains("rivet-app.pub"), "{err}");
    assert!(!err.contains("NOTHING WAS WRITTEN") || err.contains("cannot"), "{err}");

    let _ = std::fs::remove_dir_all(&at);
}

// ---------------------------------------------------------------------------
// R-178 P1, revised by R-182 — a fleet server's card carries the repository's
// text, and none of its objects.
// ---------------------------------------------------------------------------

#[test]
fn a_server_document_carries_the_repositorys_text_and_no_object() {
    let at = scratch("repo-carry");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("repo-carry");
    repo.add_application("rivet-fleetd", "1.0.0", &app_seed_bytes);
    repo.add_application("rivet-proxyd", "1.0.0", &app_seed_bytes);
    std::fs::create_dir_all(repo.0.join("documents")).expect("documents/");
    std::fs::write(repo.0.join("documents").join("example-server.provisioning.toml"), "version = 1\n")
        .expect("a recorded document");
    std::fs::create_dir_all(repo.0.join("register")).expect("register/");
    std::fs::write(repo.0.join("register").join("register.toml"), "# a register projection\n")
        .expect("a register projection");

    let survey = Survey::take(&at, &Overrides::default());
    let doc = server_document();
    let doc_path = write_document_text(&at, "server.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("a small repository fits the staging cap");

    let staged = extras.files.iter().find(|(name, _)| name == "staged.txt").expect("a staging bundle must exist");
    let items = rivet_core::staging::parse(std::str::from_utf8(&staged.1).expect("utf8")).expect("it parses");
    let paths: Vec<&str> = items.iter().map(|s| s.path.as_str()).collect();
    assert!(paths.contains(&"fleet-repository/manifest.toml"), "{paths:?}");
    assert!(
        paths.iter().any(|p| p.starts_with("fleet-repository/documents/")),
        "{paths:?}"
    );
    assert!(paths.iter().any(|p| p.starts_with("fleet-repository/register/")), "{paths:?}");
    // **R-182: no object rides in the bundle**, however many the manifest
    // names. Asserted by absence of the whole prefix rather than by a count,
    // so a single object sneaking back in fails this.
    assert!(
        !paths.iter().any(|p| p.contains("/objects/") || p.ends_with("/objects")),
        "a server's card must carry no repository object (R-182): {paths:?}"
    );
    // Both applications are still NAMED by the carried manifest — the text
    // half says what the server's repository holds; the bytes follow later.
    let manifest = items.iter().find(|s| s.path == "fleet-repository/manifest.toml").expect("the manifest");
    let manifest_text = std::str::from_utf8(&manifest.bytes).expect("utf8 manifest");
    assert!(manifest_text.contains("rivet-fleetd") && manifest_text.contains("rivet-proxyd"), "{manifest_text}");
    assert!(extras.report.contains("R-182"), "{}", extras.report);
    assert!(extras.report.contains("NOT on this card"), "{}", extras.report);

    let _ = std::fs::remove_dir_all(&at);
}

/// **An empty file under `documents/` must not void the whole card.**
/// `carry::walk` reads every non-directory file under `documents/` and
/// `register/` with no empty-file filter, so a repository holding a
/// zero-byte placeholder — `documents/.gitkeep` is the shape a real
/// repository has one in — could make `rivet_core::staging::parse` refuse
/// the ENTIRE bundle at the two-space-separated-fields line the placeholder's
/// empty hex produces, losing the identity trio and every other carried file
/// along with it. This drives the real carry code (`gather`, unchanged from
/// the test above it) rather than `rivet_core::staging` directly, so it is
/// the production trigger and not just the format's own contract.
#[test]
fn an_empty_document_alongside_a_real_one_both_survive_the_real_carry_code() {
    let at = scratch("repo-carry-empty-file");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("repo-carry-empty-file");
    repo.add_application("rivet-fleetd", "1.0.0", &app_seed_bytes);
    repo.add_application("rivet-proxyd", "1.0.0", &app_seed_bytes);
    std::fs::create_dir_all(repo.0.join("documents")).expect("documents/");
    let real_document = b"version = 1\nhostname = \"example-server\"\n".to_vec();
    std::fs::write(repo.0.join("documents").join("example-server.provisioning.toml"), &real_document)
        .expect("a recorded document");
    // The shape a real repository's own empty placeholder takes.
    std::fs::write(repo.0.join("documents").join(".gitkeep"), b"").expect("an empty placeholder");
    std::fs::create_dir_all(repo.0.join("register")).expect("register/");
    std::fs::write(repo.0.join("register").join("register.toml"), "# a register projection\n")
        .expect("a register projection");

    let survey = Survey::take(&at, &Overrides::default());
    let doc = server_document();
    let doc_path = write_document_text(&at, "server.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let extras = gather(&doc_path, &doc, &survey, &args, &at)
        .expect("an empty placeholder beside a real document must not refuse the whole card");

    let staged = extras.files.iter().find(|(name, _)| name == "staged.txt").expect("a staging bundle must exist");
    let items = rivet_core::staging::parse(std::str::from_utf8(&staged.1).expect("utf8"))
        .expect("the bundle a real carry produced must parse");

    let real = items
        .iter()
        .find(|s| s.path == "fleet-repository/documents/example-server.provisioning.toml")
        .expect("the non-empty document must still be staged");
    assert_eq!(real.bytes, real_document);

    let empty = items
        .iter()
        .find(|s| s.path == "fleet-repository/documents/.gitkeep")
        .expect("the empty placeholder must still be staged, not silently dropped");
    assert_eq!(empty.bytes, Vec::<u8>::new());

    let _ = std::fs::remove_dir_all(&at);
}

/// **`.rulings/` R-178 item 1's own fix.** `manifest.toml` used to ride into
/// the staging bundle verbatim, so a `card`/`update` entry — or an
/// `application`/`library` entry whose object had been pruned — still NAMED
/// itself in the copy `rivet-fleetd` reads, and a server asked for it would
/// have a manifest line with no bytes anywhere behind it. The carried manifest
/// must name only `application`/`library` entries this workstation holds an
/// object for — the set that can follow over the control plane (R-182).
#[test]
fn the_carried_manifest_names_only_entries_this_workstation_holds_an_object_for() {
    let at = scratch("manifest-filter");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("manifest-filter");
    repo.add_application("rivet-fleetd", "1.0.0", &app_seed_bytes);
    repo.add_application("rivet-proxyd", "1.0.0", &app_seed_bytes);
    // A `card` entry — excluded by kind (`ADR-054` §5) — and an `application`
    // entry with no object behind it (a pruned repository). Neither has bytes
    // in `objects/`, and both must be ABSENT from the carried manifest text.
    repo.add_entry_without_object("rivet", "card");
    repo.add_entry_without_object("rivet-orphan", "application");

    let survey = Survey::take(&at, &Overrides::default());
    let doc = server_document();
    let doc_path = write_document_text(&at, "server.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("the two real applications fit the cap");

    let staged = extras.files.iter().find(|(name, _)| name == "staged.txt").expect("a staging bundle must exist");
    let items = rivet_core::staging::parse(std::str::from_utf8(&staged.1).expect("utf8")).expect("it parses");
    let manifest_item = items
        .iter()
        .find(|s| s.path == "fleet-repository/manifest.toml")
        .expect("the manifest is carried");
    let manifest_text = std::str::from_utf8(&manifest_item.bytes).expect("utf8 manifest");

    assert!(manifest_text.contains("rivet-fleetd"), "{manifest_text}");
    assert!(manifest_text.contains("rivet-proxyd"), "{manifest_text}");
    assert!(
        !manifest_text.contains("\"rivet\"") && !manifest_text.contains("kind    = \"card\""),
        "a card entry with no object must not be named in the carried manifest: {manifest_text}"
    );
    assert!(
        !manifest_text.contains("rivet-orphan"),
        "an application entry whose object is not in objects/ must not be named: {manifest_text}"
    );

    // And no object bytes at all, the two real applications' included (R-182).
    assert!(
        items.iter().all(|s| !s.path.contains("/objects/")),
        "{:?}",
        items.iter().map(|s| &s.path).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&at);
}

#[test]
fn a_member_document_never_carries_the_repository() {
    let at = scratch("repo-not-for-member");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("repo-not-for-member");
    repo.add_application("rivet-proxyd", "1.0.0", &app_seed_bytes);
    std::fs::create_dir_all(repo.0.join("documents")).expect("documents/");
    std::fs::write(repo.0.join("documents").join("something.provisioning.toml"), "version = 1\n")
        .expect("a recorded document");

    let survey = Survey::take(&at, &Overrides::default());
    let doc = member_document(Some("1.0.0"));
    let doc_path = write_document_text(&at, "member.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("a member document carries its own application");

    if let Some((_, bytes)) = extras.files.iter().find(|(name, _)| name == "staged.txt") {
        let items = rivet_core::staging::parse(std::str::from_utf8(bytes).expect("utf8")).expect("it parses");
        assert!(
            items.iter().all(|s| !s.path.starts_with("fleet-repository/")),
            "a member must never carry the repository (R-178 P1: server only): {items:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&at);
}

/// **R-182's own context, as a test.** A real server provision was refused
/// because `rivet-fleetd` and `rivet-proxyd` alone were about 3.9 MB of hex
/// in a bundle capped at 5 MiB. An object at the whole cap's size now costs
/// the bundle nothing, because no object is carried.
#[test]
fn a_repository_whose_objects_exceed_the_staging_cap_still_carries_its_text() {
    let at = scratch("repo-big-objects");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("repo-big-objects");
    repo.add_application("rivet-fleetd", "1.0.0", &app_seed_bytes);
    repo.add_application("rivet-proxyd", "1.0.0", &app_seed_bytes);
    // One "application" object as large as the whole cap — carried as bytes,
    // it alone would overrun the bundle twice over once hex-encoded.
    let huge_digest = "f".repeat(64);
    std::fs::write(repo.0.join("objects").join(&huge_digest), vec![0u8; rivet_core::staging::MAX_BUNDLE_BYTES])
        .expect("a huge fixture object");
    let manifest = repo.0.join("manifest.toml");
    let mut text = std::fs::read_to_string(&manifest).expect("the manifest exists already");
    text.push_str(&format!(
        "\n[[artifact]]\ndigest = \"{huge_digest}\"\nproduct = \"huge-fixture\"\nversion = \"1.0.0\"\n\
         arch = \"aarch64\"\nkind = \"application\"\nsize = {}\nadded = \"2026-09-13\"\n",
        rivet_core::staging::MAX_BUNDLE_BYTES,
    ));
    std::fs::write(&manifest, text).expect("write the manifest");

    let survey = Survey::take(&at, &Overrides::default());
    let doc = server_document();
    let doc_path = write_document_text(&at, "server.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let extras = gather(&doc_path, &doc, &survey, &args, &at).expect("text alone fits the cap");
    let staged = extras.files.iter().find(|(name, _)| name == "staged.txt").expect("a staging bundle must exist");
    assert!(staged.1.len() < rivet_core::staging::MAX_BUNDLE_BYTES, "{} bytes", staged.1.len());
    let items = rivet_core::staging::parse(std::str::from_utf8(&staged.1).expect("utf8")).expect("it parses");
    let manifest_item = items.iter().find(|s| s.path == "fleet-repository/manifest.toml").expect("the manifest");
    assert!(std::str::from_utf8(&manifest_item.bytes).expect("utf8").contains("huge-fixture"));
    assert!(items.iter().all(|s| !s.path.contains("/objects/")));

    let _ = std::fs::remove_dir_all(&at);
}

/// **The cap still binds the text.** Nothing about R-182 lifts
/// `rivet_core::staging::MAX_BUNDLE_BYTES`; a repository whose `documents/`
/// alone overruns it is refused by name, before anything is written.
#[test]
fn a_repository_text_too_large_for_the_staging_cap_is_refused_by_name() {
    let at = scratch("repo-too-large");
    complete_host(&at);
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");

    let repo = Repo::new("repo-too-large");
    repo.add_application("rivet-fleetd", "1.0.0", &app_seed_bytes);
    repo.add_application("rivet-proxyd", "1.0.0", &app_seed_bytes);
    std::fs::create_dir_all(repo.0.join("documents")).expect("documents/");
    std::fs::write(
        repo.0.join("documents").join("huge.provisioning.toml"),
        vec![b'#'; rivet_core::staging::MAX_BUNDLE_BYTES],
    )
    .expect("an oversized document");

    let survey = Survey::take(&at, &Overrides::default());
    let doc = server_document();
    let doc_path = write_document_text(&at, "server.provisioning.toml", "irrelevant");
    write_empty_staging_bundle(&doc_path);
    let args = default_args(&at, Some(&repo.0));

    let err = gather(&doc_path, &doc, &survey, &args, &at).expect_err("the text is too large for the cap");
    assert!(err.contains("staging bundle"), "{err}");
    assert!(err.contains("MAX_BUNDLE_BYTES") || err.contains("limit"), "{err}");
    assert!(err.contains("nothing was written"), "{err}");

    let _ = std::fs::remove_dir_all(&at);
}
