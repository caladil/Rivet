// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

use rivet_core::package::application::{MANIFEST_NAME, ProgramKind, TargetClass, VARIANT_NONE};
use rivet_crypto::Ed25519Signer;

/// A fresh, empty directory nothing else is using — one per test, named for
/// it, so tests running concurrently in one process never collide the way
/// `AGENTS.md` §10.5 warns a shared cache does.
fn scratch(label: &str) -> PathBuf {
    crate::scratch::dir(&format!("fleet-test-{label}"))
}

/// A real, validly signed application `.rvt` — built with `crate::emit::write`
/// directly rather than through `cargo rivet package`, so this needs no
/// `cargo build` and no crate to build. **Real BLAKE3 and real Ed25519**, not
/// `rivet_core::package::testing`'s fakes — `publish` calls the same verifier
/// `cargo rivet inspect` does, and a toy signature would prove nothing about
/// that path (`AGENTS.md` §4).
///
/// Returns the artifact's path and the path of a `--trusted-key` file holding
/// the matching public key.
pub(super) fn fixture(dir: &Path, product: &str, version: &str, seed_byte: u8) -> (PathBuf, PathBuf) {
    let seed = [seed_byte; 32];
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let manifest_text = rivet_core::package::application::Manifest::new(ProgramKind::Daemon).render();
    let parts = vec![
        crate::emit::Part {
            name: MANIFEST_NAME.to_string(),
            class: TargetClass::Manifest.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(manifest_text.into_bytes()),
        },
        crate::emit::Part {
            name: "prog".to_string(),
            class: TargetClass::Program.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's program bytes".to_vec()),
        },
    ];

    let out = dir.join(rivet_remote::name::artifact(product, version, "aarch64", None, &Kind::application()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product,
            version,
            arch: "aarch64",
            platform: None,
            kind: Kind::application(),
        },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("a fixture is written");

    let trusted_key = dir.join(format!("{product}.pub"));
    std::fs::write(&trusted_key, signer.public_key().0).expect("write the trusted key");
    (out, trusted_key)
}

/// Like [`fixture`], but the application's manifest DECLARES a closure
/// (`SPEC-02` §5) — for R15's tests (`.rulings/` R-157), which check what
/// deploy does when a machine does or does not hold what is declared.
fn fixture_declaring(
    dir: &Path,
    product: &str,
    version: &str,
    seed_byte: u8,
    libraries: &[&str],
) -> (PathBuf, PathBuf) {
    let seed = [seed_byte; 32];
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let refs: Vec<rivet_core::package::library::Reference> = libraries
        .iter()
        .map(|r| rivet_core::package::library::Reference::parse(r).expect("a valid reference"))
        .collect();
    let manifest_text = rivet_core::package::application::Manifest::new(ProgramKind::Daemon)
        .with_library(refs)
        .expect("a closure within SPEC-02's bound")
        .render();
    let parts = vec![
        crate::emit::Part {
            name: MANIFEST_NAME.to_string(),
            class: TargetClass::Manifest.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(manifest_text.into_bytes()),
        },
        crate::emit::Part {
            name: "prog".to_string(),
            class: TargetClass::Program.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's program bytes".to_vec()),
        },
    ];

    std::fs::create_dir_all(dir).expect("a fixture directory");
    let out = dir.join(rivet_remote::name::artifact(product, version, "aarch64", None, &Kind::application()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product,
            version,
            arch: "aarch64",
            platform: None,
            kind: Kind::application(),
        },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("a fixture is written");

    let trusted_key = dir.join(format!("{product}.pub"));
    std::fs::write(&trusted_key, signer.public_key().0).expect("write the trusted key");
    (out, trusted_key)
}

/// Like [`fixture`], with whatever manifest the test hands it — for the tests
/// that tie `install`'s printed steps to the planner, which need a route and
/// material in one artifact.
fn fixture_with(
    dir: &Path,
    product: &str,
    version: &str,
    seed_byte: u8,
    manifest: &rivet_core::package::application::Manifest,
) -> PathBuf {
    let mut signer = Ed25519Signer::from_seed(&[seed_byte; 32]);
    let mut digest = Blake3::new();
    let parts = vec![
        crate::emit::Part {
            name: MANIFEST_NAME.to_string(),
            class: TargetClass::Manifest.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(manifest.render().into_bytes()),
        },
        crate::emit::Part {
            name: "prog".to_string(),
            class: TargetClass::Program.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's program bytes".to_vec()),
        },
    ];
    std::fs::create_dir_all(dir).expect("a fixture directory");
    let out = dir.join(rivet_remote::name::artifact(product, version, "aarch64", None, &Kind::application()));
    crate::emit::write(
        &out,
        &crate::emit::Identity { product, version, arch: "aarch64", platform: None, kind: Kind::application() },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("a fixture is written");
    out
}

/// Like [`fixture`], but the application's manifest DECLARES per-machine
/// material (`ADR-055` §3) — for R-194's tests, which check what deploy does
/// when a machine does or does not hold what was declared.
fn fixture_needing_material(
    dir: &Path,
    product: &str,
    version: &str,
    seed_byte: u8,
    material: &[rivet_core::package::application::Material],
) -> (PathBuf, PathBuf) {
    let seed = [seed_byte; 32];
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let manifest_text = rivet_core::package::application::Manifest::new(ProgramKind::Daemon)
        .with_material(material.to_vec())
        .render();
    let parts = vec![
        crate::emit::Part {
            name: MANIFEST_NAME.to_string(),
            class: TargetClass::Manifest.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(manifest_text.into_bytes()),
        },
        crate::emit::Part {
            name: "prog".to_string(),
            class: TargetClass::Program.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's program bytes".to_vec()),
        },
    ];

    std::fs::create_dir_all(dir).expect("a fixture directory");
    let out = dir.join(rivet_remote::name::artifact(product, version, "aarch64", None, &Kind::application()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product,
            version,
            arch: "aarch64",
            platform: None,
            kind: Kind::application(),
        },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("a fixture is written");

    let trusted_key = dir.join(format!("{product}.pub"));
    std::fs::write(&trusted_key, signer.public_key().0).expect("write the trusted key");
    (out, trusted_key)
}

/// A real, validly signed **library** `.rvt` — `SPEC-02` §2's shape: member 0
/// is the library manifest, then one shared object named by its SONAME. Built
/// with `crate::emit::write` for [`fixture`]'s reasons, with real BLAKE3 and
/// real Ed25519. **Hand-composed**, not captured: the object's bytes are not an
/// ELF, which nothing on the publish or drift path reads — `cargo rivet
/// library` is what reads a `DT_SONAME`, and it is not under test here.
///
/// `object` varies the payload, so two calls with one product and version make
/// two different builds — different bytes, a different reference.
///
/// Shared with `fleet_drift_tests.rs` rather than copied: it signs, and a
/// second copy of a signing fixture is a second place for it to go wrong.
pub(super) fn library_fixture(
    dir: &Path,
    product: &str,
    version: &str,
    object: &[u8],
    seed_byte: u8,
) -> (PathBuf, PathBuf) {
    use rivet_core::package::library::{self, TargetClass as LibraryClass};

    let seed = [seed_byte; 32];
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let soname = format!("{product}.so.1");
    let manifest_text = library::Manifest::new(vec![soname.clone()])
        .expect("a valid SONAME")
        .render();
    let parts = vec![
        crate::emit::Part {
            name: library::MANIFEST_NAME.to_string(),
            class: LibraryClass::Manifest.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(manifest_text.into_bytes()),
        },
        crate::emit::Part {
            name: soname,
            class: LibraryClass::Object.as_byte(),
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(object.to_vec()),
        },
    ];

    std::fs::create_dir_all(dir).expect("a fixture directory");
    let out = dir.join(rivet_remote::name::artifact(product, version, "aarch64", None, &Kind::library()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product,
            version,
            arch: "aarch64",
            platform: None,
            kind: Kind::library(),
        },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("a library fixture is written");

    let trusted_key = dir.join(format!("{product}-library.pub"));
    std::fs::write(&trusted_key, signer.public_key().0).expect("write the trusted key");
    (out, trusted_key)
}

/// A real, validly signed `apk` `.rvt` — `ADR-061` §3. `app.apk` and
/// `apk.json` only, no icon, exactly the shape [`crate::package::apk::Apk::open`]
/// requires: `platform = "android"`, `arch = "any"`.
pub(super) fn apk_fixture(
    dir: &Path,
    product: &str,
    version: &str,
    seed_byte: u8,
) -> (PathBuf, PathBuf) {
    use rivet_core::package::apk::{APK_NAME, METADATA_NAME};

    let seed = [seed_byte; 32];
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let metadata = format!(
        "{{\"application_id\":\"com.example.{product}\",\"version_code\":1,\
         \"version_name\":\"{version}\",\"label\":\"{product}\",\"min_sdk\":21,\
         \"target_sdk\":34,\"signer_sha256\":\"{}\",\"development\":true}}",
        "ab".repeat(32)
    );
    let parts = vec![
        crate::emit::Part {
            name: APK_NAME.to_string(),
            class: 0,
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's apk bytes".to_vec()),
        },
        crate::emit::Part {
            name: METADATA_NAME.to_string(),
            class: 0,
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(metadata.into_bytes()),
        },
    ];

    std::fs::create_dir_all(dir).expect("a fixture directory");
    let out = dir.join(rivet_remote::name::artifact(product, version, "any", Some("android"), &Kind::apk()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product,
            version,
            arch: "any",
            platform: Some("android"),
            kind: Kind::apk(),
        },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("an apk fixture is written");

    let trusted_key = dir.join(format!("{product}-apk.pub"));
    std::fs::write(&trusted_key, signer.public_key().0).expect("write the trusted key");
    (out, trusted_key)
}

// ---------------------------------------------------------------------------
// ADR-052 §4 — the refusal
// ---------------------------------------------------------------------------

/// **The refusal that is the point of §4.** A `.seed` anywhere under the
/// target directory stops `init` before `git init` ever runs — which is what
/// catches the worst case, `--at ~/.rivet` itself.
#[test]
fn a_tree_holding_a_seed_refuses_to_initialise() {
    let dir = scratch("init-seed");
    std::fs::create_dir_all(dir.join("keys")).expect("make keys/");
    std::fs::write(dir.join("keys").join("rivet-app.seed"), [7u8; 32]).expect("plant a seed");

    let err = init(&dir).expect_err("a seed must refuse initialisation");
    assert!(err.contains("rivet-app.seed"), "{err}");
    assert!(err.contains("ADR-052 §4"), "{err}");
    assert!(!dir.join(".git").exists(), "git init must not have run");
}

/// **Broken on purpose, then fixed, per `AGENTS.md` §4** — a refusal that
/// cannot fail proves nothing. Remove the seed and the same call succeeds.
#[test]
fn removing_the_seed_lets_the_same_directory_initialise() {
    let dir = scratch("init-seed-removed");
    let seed = dir.join("device.seed");
    std::fs::write(&seed, [9u8; 32]).expect("plant a seed");
    assert!(init(&dir).is_err(), "the seed must refuse it first");

    std::fs::remove_file(&seed).expect("remove it");
    let report = init(&dir).expect("no seed remains");
    assert!(report.contains("MADE"), "{report}");
    assert!(dir.join(".git").is_dir());
}

/// `publish` checks too, not just `init` — ADR-052 §4 covers a COMMIT as much
/// as the first one.
#[test]
fn publishing_into_a_tree_that_gained_a_seed_is_refused() {
    let dir = scratch("publish-seed");
    init(&dir).expect("a clean init");
    std::fs::write(dir.join("rivet-system.seed"), [1u8; 32]).expect("plant a seed afterwards");

    let (artifact, key) = fixture(&scratch("publish-seed-fixture"), "seed-check-rivet", "0.1.0", 3);
    let err = publish(&dir, &artifact, &key).expect_err("the seed must refuse publishing");
    assert!(err.contains("rivet-system.seed"), "{err}");
    assert!(list(&dir).expect("list still works").contains("empty"), "nothing was stored");
}

// ---------------------------------------------------------------------------
// init — running it twice
// ---------------------------------------------------------------------------

#[test]
fn init_twice_is_a_report_and_not_a_second_repository() {
    let dir = scratch("init-twice");
    let first = init(&dir).expect("first init");
    assert!(first.contains("MADE"), "{first}");

    let second = init(&dir).expect("second init");
    assert!(second.contains("have"), "{second}");
    assert!(second.contains("already initialised"), "{second}");
}

// ---------------------------------------------------------------------------
// publish — verification, storage, idempotence
// ---------------------------------------------------------------------------

#[test]
fn publish_refuses_a_corrupt_file_and_stores_nothing() {
    let dir = scratch("publish-corrupt");
    init(&dir).expect("init");
    let junk = dir.join("junk.rvt");
    std::fs::write(&junk, b"not a package header at all").expect("write junk");
    let key = dir.join("some.pub");
    std::fs::write(&key, [4u8; 32]).expect("write a key");

    let err = publish(&dir, &junk, &key).expect_err("junk bytes must be refused");
    assert!(!err.is_empty());
    assert!(
        std::fs::read_dir(dir.join(OBJECTS_DIR)).expect("objects/ exists").next().is_none(),
        "nothing must have been stored for a refused artifact"
    );
    assert!(list(&dir).expect("list").contains("empty"));
}

/// A well-formed, correctly signed artifact — verified by the WRONG public
/// key — is refused exactly as a corrupt one is. Distinguishes "not signed"
/// from "signed by somebody else", which is the case an operator publishing
/// somebody else's build will actually hit.
#[test]
fn publish_refuses_an_artifact_signed_by_a_different_key() {
    let dir = scratch("publish-wrong-key");
    init(&dir).expect("init");
    let (artifact, _) = fixture(&scratch("publish-wrong-key-fixture"), "wrongkey-rivet", "0.1.0", 5);
    let (_, other_key) = fixture(&scratch("publish-wrong-key-other"), "other-rivet", "0.1.0", 6);

    let err = publish(&dir, &artifact, &other_key).expect_err("the wrong key must be refused");
    assert!(!err.is_empty());
    assert!(list(&dir).expect("list").contains("empty"));
}

/// **An application under an image service's name is not published** —
/// `ADR-060` §4. The proxy ships in the image, and a machine's door refuses
/// its payload, so the repository never takes one.
#[test]
fn publish_refuses_an_application_named_like_a_service_the_image_provides() {
    let dir = scratch("publish-image-service");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("publish-image-service-fixture"), "rivet-proxyd", "0.1.113", 12);

    let err = publish(&dir, &artifact, &key).expect_err("the image provides rivet-proxyd");
    assert!(err.contains("rivet-proxyd"), "{err}");
    assert!(err.contains("Nothing was stored"), "{err}");
    assert!(list(&dir).expect("list").contains("empty"));
}

#[test]
fn publish_stores_a_valid_artifact_and_records_it() {
    let dir = scratch("publish-ok");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("publish-ok-fixture"), "reliquary-rivet", "0.1.1", 11);

    let report = publish(&dir, &artifact, &key).expect("a valid artifact");
    assert!(report.contains("MADE"), "{report}");
    assert!(report.contains("reliquary-rivet"), "{report}");

    let entries = read_manifest(&dir).expect("read the manifest back");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].product, "reliquary-rivet");
    assert_eq!(entries[0].version, "0.1.1");
    assert_eq!(entries[0].kind, "application");
    assert_eq!(entries[0].digest.len(), 64, "a BLAKE3 hex digest is 64 characters");

    let stored = dir.join(OBJECTS_DIR).join(&entries[0].digest);
    assert!(stored.is_file(), "the bytes must be under objects/ by digest");
    assert_eq!(
        std::fs::read(&stored).expect("read the stored bytes"),
        std::fs::read(&artifact).expect("read the original"),
        "the stored bytes must be identical to what was published"
    );
}

/// **A publish commits `MANIFEST_FILE` only** — `ADR-061` §4, `rivet-fleet-repo`'s
/// own `commit_only`. `~/.rivet/fleet` is the maintainer's own working tree, not
/// a dedicated checkout, so something else can have a file staged in the same
/// index when a publish runs; that file must survive the publish exactly as
/// staged, and the publish commit must not carry it.
#[test]
fn publish_commits_only_the_manifest_and_leaves_an_unrelated_staged_file_staged() {
    let dir = scratch("publish-only-manifest");
    init(&dir).expect("init");
    let (artifact, key) =
        fixture(&scratch("publish-only-manifest-fixture"), "reliquary-rivet", "0.1.1", 22);

    // Something else has a file staged in this same repository already.
    std::fs::write(dir.join("unrelated.txt"), b"not this publish's business")
        .expect("write the unrelated file");
    git(&dir, &["add", "unrelated.txt"]).expect("stage the unrelated file");

    publish(&dir, &artifact, &key).expect("a valid artifact");

    let status = git(&dir, &["status", "--porcelain"]).expect("status");
    assert!(
        status.lines().any(|l| l.starts_with('A') && l.ends_with("unrelated.txt")),
        "the unrelated file must still be staged, added, uncommitted: {status}"
    );

    let committed = git(&dir, &["show", "--stat", "--format=", "HEAD"]).expect("show HEAD");
    assert!(
        !committed.contains("unrelated.txt"),
        "the publish commit must not include the unrelated file: {committed}"
    );
}

/// **A shared rivet publishes as a `library`**, deliberately rather than by
/// accident — `PLAN-Feature-Set-2026-09-12` §3 and `SPEC-02` §10. The manifest
/// records the header's kind, no platform (a library is architecture-shaped),
/// `list` says `library`, and `deploy --artifact` hands back a file named with
/// the `library` kind token, which is what the machine's store parses.
#[test]
fn publish_stores_a_library_and_records_it_as_a_library() {
    let dir = scratch("publish-library");
    init(&dir).expect("init");
    let (artifact, key) = library_fixture(
        &scratch("publish-library-fixture"),
        "libgreet",
        "1.0.0",
        b"a fixture's shared object bytes",
        55,
    );

    let report = publish(&dir, &artifact, &key).expect("a valid library");
    assert!(report.contains("MADE"), "{report}");
    assert!(
        report.lines().any(|l| l.split_whitespace().eq(["kind", "library"])),
        "{report}"
    );

    let entries = read_manifest(&dir).expect("read the manifest back");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].product, "libgreet");
    assert_eq!(entries[0].version, "1.0.0");
    assert_eq!(entries[0].kind, "library");
    assert_eq!(entries[0].platform, None);
    assert_eq!(
        std::fs::read(dir.join(OBJECTS_DIR).join(&entries[0].digest)).expect("stored bytes"),
        std::fs::read(&artifact).expect("the original"),
    );

    let listed = list(&dir).expect("list");
    assert!(listed.contains("libgreet 1.0.0 (library, aarch64)"), "{listed}");

    let materialised = materialise_for_deploy(&dir, &entries[0].digest[..12]).expect("resolves");
    assert_eq!(
        materialised.file_name().and_then(|n| n.to_str()),
        Some("libgreet-1.0.0-aarch64-library.rvt"),
    );
}

/// **An APK publishes as `apk`** — `ADR-061` §3: the manifest records the
/// header's kind, `platform = "android"`, `arch = "any"`, and `deploy
/// --artifact` hands back a file named with the `apk` kind token, which is
/// what a fleet server's store parses (`Entry::file_name`).
#[test]
fn publish_stores_an_apk_and_records_it_as_an_apk() {
    let dir = scratch("publish-apk");
    init(&dir).expect("init");
    let (artifact, key) = apk_fixture(&scratch("publish-apk-fixture"), "gantry", "0.1.17", 66);

    let report = publish(&dir, &artifact, &key).expect("a valid apk");
    assert!(report.contains("MADE"), "{report}");
    assert!(
        report.lines().any(|l| l.split_whitespace().eq(["kind", "apk"])),
        "{report}"
    );

    let entries = read_manifest(&dir).expect("read the manifest back");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].product, "gantry");
    assert_eq!(entries[0].version, "0.1.17");
    assert_eq!(entries[0].kind, "apk");
    assert_eq!(entries[0].platform.as_deref(), Some("android"));
    assert_eq!(entries[0].arch, "any");
    assert_eq!(
        std::fs::read(dir.join(OBJECTS_DIR).join(&entries[0].digest)).expect("stored bytes"),
        std::fs::read(&artifact).expect("the original"),
    );

    let listed = list(&dir).expect("list");
    assert!(listed.contains("gantry 0.1.17 (apk, any android)"), "{listed}");

    let materialised = materialise_for_deploy(&dir, &entries[0].digest[..12]).expect("resolves");
    assert_eq!(
        materialised.file_name().and_then(|n| n.to_str()),
        Some("gantry-0.1.17-any-android-apk.rvt"),
    );
}

/// **ADR-052's own words: "the digest is the name, so it either matches or it
/// is a different artifact."** Publishing the same file twice changes
/// nothing the second time.
#[test]
fn publishing_the_same_artifact_twice_is_idempotent() {
    let dir = scratch("publish-twice");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("publish-twice-fixture"), "notes-rivet", "0.2.1", 22);

    let first = publish(&dir, &artifact, &key).expect("first publish");
    assert!(first.contains("MADE"), "{first}");
    let second = publish(&dir, &artifact, &key).expect("second publish");
    assert!(second.contains("have"), "{second}");

    let entries = read_manifest(&dir).expect("manifest");
    assert_eq!(entries.len(), 1, "a repeat must not duplicate the entry");
    let objects: Vec<_> =
        std::fs::read_dir(dir.join(OBJECTS_DIR)).expect("objects/").collect();
    assert_eq!(objects.len(), 1, "a repeat must not duplicate the stored bytes");
}

// ---------------------------------------------------------------------------
// Retention — `ADR-052` §7's rule ported to the workstation. `R-177` D4.
// ---------------------------------------------------------------------------

/// **The repository keeps the three newest of one product, kind, platform and
/// arch — a fourth publish evicts the oldest, not the one that just landed.**
#[test]
fn a_fourth_publish_of_one_product_prunes_the_oldest_and_keeps_three() {
    let dir = scratch("publish-prune");
    init(&dir).expect("init");
    let src = scratch("publish-prune-fixtures");

    let mut digests = Vec::new();
    for version in ["0.1.0", "0.1.1", "0.1.2", "0.1.3"] {
        let (artifact, key) = fixture(&src, "reliquary-rivet", version, 44);
        let report = publish(&dir, &artifact, &key).expect("publish");
        let entries = read_manifest(&dir).expect("manifest");
        let entry = entries
            .iter()
            .find(|e| e.version == version)
            .expect("the version just published is in the manifest");
        digests.push(entry.digest.clone());
        if version == "0.1.3" {
            assert!(
                report.contains("pruned 1"),
                "the fourth publish must report exactly one pruned: {report}"
            );
            assert!(report.contains("0.1.0"), "{report}");
        } else {
            assert!(!report.contains("pruned"), "{report}");
        }
    }

    let entries = read_manifest(&dir).expect("manifest");
    let versions: Vec<&str> = entries.iter().map(|e| e.version.as_str()).collect();
    assert_eq!(versions.len(), 3, "exactly three of the four survive: {versions:?}");
    assert!(!versions.contains(&"0.1.0"), "the oldest must be the one dropped: {versions:?}");
    assert!(versions.contains(&"0.1.3"), "the one just published must survive: {versions:?}");

    // The dropped object's bytes are gone; the three survivors' are not.
    assert!(!dir.join(OBJECTS_DIR).join(&digests[0]).exists(), "the pruned object must be removed");
    for digest in &digests[1..] {
        assert!(dir.join(OBJECTS_DIR).join(digest).is_file(), "a kept object must still be stored");
    }
}

/// A host's record in `dir`'s own `register/`, naming `digest` as what its
/// slot `slot` holds — the projection `rivet-fleet` and `fleet record` write,
/// written here through `rivetos_register` so the bytes are the register's own.
fn record_host(dir: &Path, id: &str, name: &str, slot: &str, digest: &str) {
    let root = dir.join(rivetos_fleet_repo::REGISTER_DIR);
    let mut register = rivetos_register::store::load(&root).expect("the projection reads");
    let mut device = register.devices.remove(id).unwrap_or_else(|| rivetos_register::Device {
        id: id.to_owned(),
        name: name.to_owned(),
        ..rivetos_register::Device::default()
    });
    device.system_slots.set(slot, digest).expect("a slot");
    register.devices.insert(id.to_owned(), device);
    rivetos_register::store::save(&register).expect("the projection is written");
}

/// **A digest a host's record names is never pruned, even as the oldest of
/// five** (`ADR-052` §7, R-233). Keep-three still keeps the three newest; the
/// named one is kept on top of them, and the report says which host named it.
#[test]
fn a_digest_a_host_names_survives_even_as_the_oldest_of_five() {
    let dir = scratch("publish-prune-named");
    init(&dir).expect("init");
    let src = scratch("publish-prune-named-fixtures");

    let mut digests = Vec::new();
    let mut last_report = String::new();
    for version in ["0.1.0", "0.1.1", "0.1.2", "0.1.3", "0.1.4"] {
        let (artifact, key) = fixture(&src, "reliquary-rivet", version, 46);
        last_report = publish(&dir, &artifact, &key).expect("publish");
        let entries = read_manifest(&dir).expect("manifest");
        let entry = entries.iter().find(|e| e.version == version).expect("published");
        digests.push(entry.digest.clone());
        if version == "0.1.0" {
            record_host(&dir, "0011223344556677", "kitchen-pi", "a", &entry.digest);
        }
    }

    let entries = read_manifest(&dir).expect("manifest");
    let mut versions: Vec<&str> = entries.iter().map(|e| e.version.as_str()).collect();
    versions.sort_unstable();
    assert_eq!(versions, vec!["0.1.0", "0.1.2", "0.1.3", "0.1.4"], "the named oldest is kept on top of three");
    assert!(dir.join(OBJECTS_DIR).join(&digests[0]).is_file(), "the named object must still be stored");
    assert!(!dir.join(OBJECTS_DIR).join(&digests[1]).exists(), "an unnamed older object is pruned");
    assert!(last_report.contains("kitchen-pi"), "the report names the host that kept it: {last_report}");
    assert!(last_report.contains(&digests[0]), "{last_report}");
}

/// **A repository with no host recorded behaves exactly as keep-three**, and
/// an unreadable record set keeps everything rather than pruning blind.
#[test]
fn an_unreadable_record_set_prunes_nothing() {
    let dir = scratch("publish-prune-unreadable");
    init(&dir).expect("init");
    let src = scratch("publish-prune-unreadable-fixtures");
    let devices = dir.join(rivetos_fleet_repo::REGISTER_DIR).join(rivetos_register::DEVICES_DIR);
    std::fs::create_dir_all(&devices).expect("devices/");
    std::fs::write(devices.join("0011223344556677.toml"), "this is not a record").expect("a broken record");

    let mut report = String::new();
    for version in ["0.1.0", "0.1.1", "0.1.2", "0.1.3"] {
        let (artifact, key) = fixture(&src, "reliquary-rivet", version, 47);
        report = publish(&dir, &artifact, &key).expect("publish");
    }
    assert_eq!(read_manifest(&dir).expect("manifest").len(), 4, "nothing is pruned blind");
    assert!(report.contains("NOTHING WAS PRUNED"), "{report}");
}

// ---------------------------------------------------------------------------
// A manifest names objects by digest and nothing else — the audit's F03.
// ---------------------------------------------------------------------------

/// Hand-composed hostile manifest entries: each digest names a file OUTSIDE
/// `objects/`, by traversal or by an absolute path, and each such file is a
/// sentinel that must outlive whatever this repository does.
struct Hostile {
    root: PathBuf,
    sentinels: Vec<PathBuf>,
}

fn hostile_repository(label: &str, product: &str, kind: &str) -> Hostile {
    let base = scratch(label);
    let root = base.join("fleet");
    init(&root).expect("init");
    std::fs::create_dir_all(root.join(OBJECTS_DIR)).expect("objects/");
    let traversal = base.join("traversal-sentinel.txt");
    let absolute = base.join("absolute-sentinel.txt");
    let backslash = base.join("backslash-sentinel.txt");
    for sentinel in [&traversal, &absolute, &backslash] {
        std::fs::write(sentinel, b"must survive").expect("a sentinel");
    }
    let digests = [
        "../../traversal-sentinel.txt".to_owned(),
        absolute.display().to_string(),
        "..\\..\\backslash-sentinel.txt".to_owned(),
    ];
    let entries: Vec<Entry> = digests
        .iter()
        .enumerate()
        .map(|(i, digest)| Entry {
            digest: digest.clone(),
            product: product.to_owned(),
            version: format!("0.0.{i}"),
            arch: "aarch64".to_owned(),
            platform: None,
            kind: kind.to_owned(),
            size: 1,
            added: "2026-01-01T00:00:00Z".to_owned(),
            schema_version: None,
        })
        .collect();
    // Committed, so every verb meets it exactly as a manifest somebody pushed.
    commit_manifest(&root, &entries, &[], "a hostile manifest").expect("a hostile manifest");
    Hostile { root, sentinels: vec![traversal, absolute, backslash] }
}

impl Hostile {
    fn assert_sentinels_survive(&self) {
        for sentinel in &self.sentinels {
            assert!(sentinel.is_file(), "{} was deleted from outside objects/", sentinel.display());
        }
    }
}

/// **A digest is exactly 64 lowercase hex characters, refused at parse with
/// the manifest named** — before anything joins it onto a path.
#[test]
fn a_manifest_digest_that_is_not_a_digest_is_refused_at_parse() {
    let hostile = hostile_repository("digest-parse", "evil", "library");
    let refused = read_manifest(&hostile.root).expect_err("a traversal digest");
    assert!(refused.contains(MANIFEST_FILE), "the refusal names the manifest: {refused}");
    assert!(refused.contains("64 lowercase hexadecimal"), "{refused}");

    std::fs::write(
        hostile.root.join(MANIFEST_FILE),
        render(&[entry(&"A".repeat(64), "upper")]),
    )
    .expect("an uppercase digest");
    assert!(read_manifest(&hostile.root).is_err(), "uppercase is not this repository's spelling");
    std::fs::write(hostile.root.join(MANIFEST_FILE), render(&[entry(&"a".repeat(64), "fine")])).expect("a fine one");
    assert_eq!(read_manifest(&hostile.root).expect("a real digest reads").len(), 1);
}

/// **A name that would leave its directory is refused at parse too** — the
/// fields a materialised file is named from.
#[test]
fn a_manifest_name_field_that_could_leave_a_directory_is_refused_at_parse() {
    let dir = scratch("name-parse");
    init(&dir).expect("init");
    for (field, value) in [("product", "../up"), ("version", "1/2"), ("arch", "a\\b"), ("platform", ".."), ("version", "C:x")] {
        let mut bad = entry(&"b".repeat(64), "fine");
        match field {
            "product" => bad.product = value.to_owned(),
            "version" => bad.version = value.to_owned(),
            "arch" => bad.arch = value.to_owned(),
            _ => bad.platform = Some(value.to_owned()),
        }
        std::fs::write(dir.join(MANIFEST_FILE), render(&[bad])).expect("a manifest");
        let refused = read_manifest(&dir).expect_err("a path-shaped name");
        assert!(refused.contains(field), "{field}={value:?}: {refused}");
    }
}

/// **Retention never deletes outside `objects/`**: a publish into a repository
/// whose manifest names hostile digests is refused, and every sentinel lives.
#[test]
fn retention_against_hostile_digests_deletes_nothing_outside_objects() {
    let hostile = hostile_repository("digest-retention", "reliquary-rivet", "application");
    let (artifact, key) = fixture(&scratch("digest-retention-fixture"), "reliquary-rivet", "0.1.0", 48);
    let _ = publish(&hostile.root, &artifact, &key);
    hostile.assert_sentinels_survive();
}

/// **`fleet retire` never deletes outside `objects/`** either.
#[test]
fn retire_against_hostile_digests_deletes_nothing_outside_objects() {
    let hostile = hostile_repository("digest-retire", "evil", "library");
    let register = rivetos_register::Register::empty(scratch("digest-retire-register"));
    let _ = retire::retire(&hostile.root, "evil", None, None, &register, &mut || {
        panic!("a library retire never asks for the apk key")
    });
    hostile.assert_sentinels_survive();
}

/// **The destructive sink checks for itself**: handed an entry whose digest
/// is not a digest, it removes nothing and says so.
#[test]
fn removing_objects_refuses_a_digest_that_is_not_one() {
    let hostile = hostile_repository("digest-sink", "evil", "library");
    let victims: Vec<Entry> = ["../../traversal-sentinel.txt", "..\\..\\backslash-sentinel.txt"]
        .iter()
        .map(|d| entry(d, "evil"))
        .collect();
    let failures = remove_objects(&hostile.root, &[], &victims);
    assert_eq!(failures.len(), 2, "{failures:?}");
    hostile.assert_sentinels_survive();
}

// ---------------------------------------------------------------------------
// A publish commits before it deletes — the audit's F04.
// ---------------------------------------------------------------------------

/// Three versions of one product published, and their digests — the state a
/// fourth publish would prune from.
fn three_published(label: &str, seed: u8) -> (PathBuf, PathBuf, Vec<String>) {
    let dir = scratch(label);
    init(&dir).expect("init");
    let src = scratch(&format!("{label}-fixtures"));
    for version in ["0.1.0", "0.1.1", "0.1.2"] {
        let (artifact, key) = fixture(&src, "reliquary-rivet", version, seed);
        publish(&dir, &artifact, &key).expect("publish");
    }
    let digests = read_manifest(&dir).expect("manifest").into_iter().map(|e| e.digest).collect();
    (dir, src, digests)
}

fn assert_every_named_object_survives(dir: &Path, digests: &[String]) {
    let named: Vec<String> = read_manifest(dir).expect("the manifest still reads").into_iter().map(|e| e.digest).collect();
    for digest in digests {
        assert!(named.contains(digest), "the manifest still names {digest}");
        assert!(dir.join(OBJECTS_DIR).join(digest).is_file(), "{digest} is named and its object must survive");
    }
}

/// **A bad `[[hidden]]` row refuses the publish before anything is removed.**
#[test]
fn an_invalid_hidden_row_refuses_a_publish_before_any_object_is_removed() {
    let (dir, src, digests) = three_published("publish-bad-hidden", 49);
    let manifest = dir.join(MANIFEST_FILE);
    let mut text = std::fs::read_to_string(&manifest).expect("manifest");
    text.push_str("\n[[hidden]]\nproduct = \"reliquary-rivet\"\n");
    std::fs::write(&manifest, text).expect("a hidden row with no kind");

    let (artifact, key) = fixture(&src, "reliquary-rivet", "0.1.3", 49);
    let refused = publish(&dir, &artifact, &key).expect_err("a hidden row with no kind");
    assert!(refused.contains("[[hidden]]"), "{refused}");
    assert_every_named_object_survives(&dir, &digests);
}

/// **A commit that fails removes nothing, and leaves the manifest as `HEAD`
/// has it.** The failure is a `pre-commit` hook that refuses, which is how
/// git refuses for real; nothing is bypassed to make it succeed.
#[test]
fn a_failed_commit_removes_no_object_and_restores_the_manifest() {
    let (dir, src, digests) = three_published("publish-commit-fails", 50);
    let before = std::fs::read_to_string(dir.join(MANIFEST_FILE)).expect("manifest");
    let hooks = dir.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).expect("hooks/");
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").expect("a refusing hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(hooks.join("pre-commit"), std::fs::Permissions::from_mode(0o755)).expect("executable");
    }

    let (artifact, key) = fixture(&src, "reliquary-rivet", "0.1.3", 50);
    let refused = publish(&dir, &artifact, &key).expect_err("the commit is refused");
    assert!(refused.contains("git commit"), "it was the commit that refused: {refused}");
    assert_every_named_object_survives(&dir, &digests);
    assert_eq!(
        std::fs::read_to_string(dir.join(MANIFEST_FILE)).expect("manifest"),
        before,
        "the manifest on disk is what HEAD holds"
    );
}

/// **Retention is scoped per `(product, kind, platform, arch)`, not per
/// product alone** — a workstation repository holds several of each for one
/// product, unlike a board's own store. A different arch's fourth publish
/// must not touch this product's `aarch64` entries at all.
#[test]
fn retention_does_not_cross_a_different_arch_of_the_same_product() {
    let dir = scratch("publish-prune-arch");
    init(&dir).expect("init");
    let src = scratch("publish-prune-arch-fixtures");

    // Three on aarch64 — exactly at the limit, nothing pruned yet.
    for version in ["0.1.0", "0.1.1", "0.1.2"] {
        let (artifact, key) = fixture(&src, "reliquary-rivet", version, 55);
        publish(&dir, &artifact, &key).expect("publish");
    }
    // A library of the SAME PRODUCT NAME — a different kind entirely — must
    // not be counted against the application's three, or vice versa.
    let (artifact, key) =
        library_fixture(&src, "reliquary-rivet", "2.0.0", b"unrelated object bytes", 55);
    publish(&dir, &artifact, &key).expect("publish the library");

    let entries = read_manifest(&dir).expect("manifest");
    let applications: Vec<&Entry> = entries.iter().filter(|e| e.kind == "application").collect();
    let libraries: Vec<&Entry> = entries.iter().filter(|e| e.kind == "library").collect();
    assert_eq!(applications.len(), 3, "three applications, none pruned by the library");
    assert_eq!(libraries.len(), 1, "the library is its own group of one");
}

/// **Cards and system updates keep three each, as every other kind does** —
/// the retention a fleet server's store inherits by pruning to the manifest
/// (`ADR-063` §6). Four of each keep the newest three of each, and neither
/// kind is counted against the other.
#[test]
fn cards_and_updates_keep_three_each() {
    let mut entries = Vec::new();
    for (i, version) in ["0.1.0", "0.1.1", "0.1.2", "0.1.3"].iter().enumerate() {
        for (kind, fill) in [("card", 'c'), ("update", 'd')] {
            let digest = format!("{fill}{i}").repeat(32);
            entries.push(tied_entry(&digest, "rivet", kind, version, "2026-09-29T00:00:00Z"));
        }
    }
    let (dropped, kept) = prune_keeping_three(&mut entries, "", &BTreeSet::new());
    assert!(kept.is_empty(), "nothing is named, so keep-three alone decides");
    let mut gone: Vec<(&str, &str)> = dropped.iter().map(|e| (e.kind.as_str(), e.version.as_str())).collect();
    gone.sort_unstable();
    assert_eq!(gone, vec![("card", "0.1.0"), ("update", "0.1.0")]);
    for kind in ["card", "update"] {
        assert_eq!(entries.iter().filter(|e| e.kind == kind).count(), 3, "{kind}");
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn list_against_an_empty_repository_says_so() {
    let dir = scratch("list-empty");
    init(&dir).expect("init");
    let report = list(&dir).expect("list");
    assert!(report.contains("empty"), "{report}");
}

#[test]
fn list_against_a_populated_repository_names_the_artifact() {
    let dir = scratch("list-populated");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("list-populated-fixture"), "tempmon-rivet", "0.1.5", 33);
    publish(&dir, &artifact, &key).expect("publish");

    let report = list(&dir).expect("list");
    assert!(report.contains("tempmon-rivet"), "{report}");
    assert!(report.contains("0.1.5"), "{report}");
    assert!(!report.contains("empty"), "{report}");
}

// ---------------------------------------------------------------------------
// digest / prefix resolution — including the ambiguous case
// ---------------------------------------------------------------------------

fn entry(digest: &str, product: &str) -> Entry {
    Entry {
        digest: digest.to_string(),
        product: product.to_string(),
        version: "0.1.0".to_string(),
        arch: "aarch64".to_string(),
        platform: None,
        kind: "application".to_string(),
        size: 100,
        added: "2026-09-09T00:00:00Z".to_string(),
        schema_version: None,
    }
}

#[test]
fn a_full_digest_resolves_exactly() {
    let entries = vec![entry(&"a".repeat(64), "one"), entry(&"b".repeat(64), "two")];
    let found = resolve(&entries, &"a".repeat(64)).expect("an exact match");
    assert_eq!(found.product, "one");
}

#[test]
fn an_unambiguous_prefix_resolves() {
    let entries = vec![entry("aaaa1111", "one"), entry("bbbb2222", "two")];
    let found = resolve(&entries, "aaaa").expect("one match");
    assert_eq!(found.product, "one");
    // Case-insensitively, since a person is likely to have typed it that way.
    let found = resolve(&entries, "AAAA").expect("case-insensitive match");
    assert_eq!(found.product, "one");
}

/// **The ambiguous case.** Two entries sharing a prefix must refuse rather
/// than pick one — and the refusal must name both, so an operator can tell
/// how much more of the digest they need.
#[test]
fn an_ambiguous_prefix_is_refused_and_names_every_match() {
    let entries = vec![entry("abcd1111", "one"), entry("abcd2222", "two")];
    let err = resolve(&entries, "abcd").expect_err("two entries share this prefix");
    assert!(err.contains("ambiguous"), "{err}");
    assert!(err.contains("one"), "{err}");
    assert!(err.contains("two"), "{err}");

    // Lengthening the prefix by the byte that actually distinguishes them
    // resolves it.
    assert_eq!(resolve(&entries, "abcd1").expect("now unique").product, "one");
}

#[test]
fn an_empty_or_unmatched_spec_is_refused() {
    let entries = vec![entry(&"c".repeat(64), "only")];
    assert!(resolve(&entries, "").is_err());
    assert!(resolve(&entries, "  ").is_err());
    let err = resolve(&entries, "ffff").expect_err("nothing starts with ffff");
    assert!(err.contains("no artifact"), "{err}");
}

// ---------------------------------------------------------------------------
// "newest" pickers order by version, then by `added` — `.rulings/` R-213
// ---------------------------------------------------------------------------

/// An `Entry` with every field a tie-break test needs to vary — [`entry`]'s
/// reason, extended with `kind` and `added` because these tests are about
/// breaking a VERSION tie by arrival, which `entry` (a fixed `"0.1.0"` and one
/// fixed `added`) cannot express.
fn tied_entry(digest: &str, product: &str, kind: &str, version: &str, added: &str) -> Entry {
    Entry {
        digest: digest.to_string(),
        product: product.to_string(),
        version: version.to_string(),
        arch: "aarch64".to_string(),
        platform: Some("rpi4".to_string()),
        kind: kind.to_string(),
        size: 100,
        added: added.to_string(),
        schema_version: None,
    }
}

/// **`list_images`'s own case.** Two `card` builds share one version's
/// numeric core — the ordinary shape of one workspace version published as
/// more than one card image — and the one added to the repository more
/// recently must be marked newest and preselected, not whichever this
/// manifest happens to list first. Written in manifest (append) order —
/// oldest first, exactly as `publish_verified` always writes it — which is
/// the shape that made the oldest of three real `0.1.114` cards win.
#[test]
fn list_images_breaks_a_version_tie_by_the_later_added_stamp() {
    let dir = scratch("list-images-tie");
    let older =
        tied_entry(&"a".repeat(64), "rivet", "card", "0.1.114-gaaaaaaa", "2026-09-24T10:00:00Z");
    let newer =
        tied_entry(&"b".repeat(64), "rivet", "card", "0.1.114-gbbbbbbb", "2026-09-24T11:00:00Z");
    std::fs::write(dir.join(MANIFEST_FILE), render(&[older, newer])).expect("write manifest");

    let images = list_images(&dir, None).expect("list");
    assert_eq!(images.len(), 2);
    assert_eq!(
        images[0].digest,
        "b".repeat(64),
        "the later-added build of a tied version must be listed first"
    );
    assert_eq!(images[1].digest, "a".repeat(64));
}

/// **`resolve_carried_object`'s ambiguity suggestion, the same rule.** Two
/// `application` builds tie on version; the suggestion must name the one
/// added more recently REGARDLESS of which order the manifest lists them in
/// — proven both ways, since a manifest this tool writes always lists them
/// oldest-first but nothing enforces that of a hand-edited one, and the
/// suggestion must be a fact about the builds, not about how they were read.
#[test]
fn resolve_carried_object_suggests_the_most_recently_added_build_regardless_of_manifest_order() {
    let older = tied_entry(
        &"a".repeat(64),
        "tempmon-rivet",
        "application",
        "0.1.114-gaaaaaaa",
        "2026-09-24T10:00:00Z",
    );
    let newer = tied_entry(
        &"b".repeat(64),
        "tempmon-rivet",
        "application",
        "0.1.114-gbbbbbbb",
        "2026-09-24T11:00:00Z",
    );

    for (label, entries) in [
        ("append-order", vec![older.clone(), newer.clone()]),
        ("reverse-order", vec![newer.clone(), older.clone()]),
    ] {
        let dir = scratch(&format!("resolve-carried-tie-{label}"));
        std::fs::write(dir.join(MANIFEST_FILE), render(&entries)).expect("write manifest");
        let err = resolve_carried_object(&dir, "tempmon-rivet", None)
            .expect_err("two candidates with no pin must refuse");
        assert!(
            err.contains("newest published is tempmon-rivet 0.1.114-gbbbbbbb"),
            "{label}: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// deploy --artifact — additive, and today's invocation is unchanged
// ---------------------------------------------------------------------------

/// Every argv shape `cmd/cargo-rivet/src/deploy.rs`'s own test suite already
/// exercises — no `--artifact` and no `--at` anywhere in it — comes back
/// byte-for-byte identical from [`route_deploy`], unselected.
#[test]
fn without_the_artifact_flag_argv_passes_through_unchanged() {
    let cases: Vec<Vec<String>> = vec![
        vec![],
        vec!["--host".into(), "pi".into(), "confirm".into()],
        vec!["--host".into(), "pi".into(), "--rvt".into(), "x.rvt".into()],
        vec![
            "--host".into(),
            "pi".into(),
            "--signing-key".into(),
            "app.key".into(),
            "--bin".into(),
            "todo".into(),
            "confirm".into(),
        ],
    ];
    for args in cases {
        let out = route_deploy(&args).expect("no --artifact means no failure mode");
        assert_eq!(out.args, args, "argv with no --artifact must not be rewritten at all");
        assert!(!out.selected);
        assert!(out.at.is_none());
    }
}

/// `--at` names the repository a deploy publishes into, so it is taken out of
/// argv on a crate deploy too — `package` would refuse it.
#[test]
fn at_is_taken_out_of_argv_without_the_artifact_flag() {
    let args: Vec<String> = ["--host", "pi", "--at", "somewhere", "--bin", "todo", "confirm"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let out = route_deploy(&args).expect("routes");
    assert_eq!(out.args, ["--host", "pi", "--bin", "todo", "confirm"]);
    assert_eq!(out.at, Some(PathBuf::from("somewhere")));
    assert!(!out.selected);
}

/// What `deploy::parse` produces from an ordinary command line is identical
/// whether it goes through [`route_deploy`] first or not.
#[test]
fn deploy_parse_agrees_with_and_without_the_hook_when_there_is_no_artifact_flag() {
    let args: Vec<String> = ["--host", "pi", "--rvt", "x.rvt", "confirm"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let direct = crate::deploy::parse(&args).expect("direct parse");
    let hooked_args = route_deploy(&args).expect("the hook").args;
    let hooked = crate::deploy::parse(&hooked_args).expect("parse after the hook");
    assert_eq!(direct.host, hooked.host);
    assert_eq!(direct.rvt, hooked.rvt);
    assert_eq!(direct.confirmed, hooked.confirmed);
    assert_eq!(direct.package_args, hooked.package_args);
}

#[test]
fn artifact_together_with_rvt_is_refused() {
    let args: Vec<String> = ["--host", "pi", "--artifact", "abcd", "--rvt", "x.rvt"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let err = route_deploy(&args).expect_err("both were given");
    assert!(err.contains("--artifact"), "{err}");
    assert!(err.contains("--rvt"), "{err}");
}

/// **The flag actually works** — a published artifact resolves to a local
/// file carrying the canonical `ADR-027` §7 name, whose bytes match what was
/// published, and which `deploy::parse` accepts through the same `--rvt`
/// mechanism it always had.
#[test]
fn artifact_resolves_to_a_correctly_named_local_copy() {
    let dir = scratch("deploy-artifact");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("deploy-artifact-fixture"), "aura-controller", "0.1.0", 44);
    publish(&dir, &artifact, &key).expect("publish");
    let entries = read_manifest(&dir).expect("manifest");
    let digest = entries[0].digest.clone();

    let args: Vec<String> = [
        "--host",
        "pi",
        "--artifact",
        &digest[..8],
        "--at",
        &dir.display().to_string(),
        "confirm",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    let route = route_deploy(&args).expect("resolves");
    assert!(route.selected);
    assert_eq!(route.at.as_deref(), Some(dir.as_path()));
    let rewritten = route.args;
    assert!(!rewritten.iter().any(|a| a == "--artifact" || a == "--at"), "{rewritten:?}");
    let rvt_at = rewritten.iter().position(|a| a == "--rvt").expect("an --rvt was added");
    let resolved_path = PathBuf::from(&rewritten[rvt_at + 1]);
    assert_eq!(
        resolved_path.file_name().and_then(|n| n.to_str()),
        Some("aura-controller-0.1.0-aarch64-application.rvt"),
        "the ADR-027 §7 name must be recomposed from the manifest's fields"
    );
    assert_eq!(
        std::fs::read(&resolved_path).expect("read the resolved copy"),
        std::fs::read(&artifact).expect("read the original"),
        "the resolved file's bytes must be exactly what was published"
    );

    // And it flows through `deploy::parse` exactly like any other --rvt.
    let parsed = crate::deploy::parse(&rewritten).expect("deploy::parse accepts it");
    assert_eq!(parsed.rvt, Some(resolved_path));
    assert!(parsed.confirmed);
}

// ---------------------------------------------------------------------------
// deploy publishes, then sends — ADR-052 §7
// ---------------------------------------------------------------------------

/// A machine that is not there, which remembers what reached it and **whether
/// the fleet repository already held each payload at the moment it was
/// pushed** — the order is the property under test, not merely the outcome.
struct Machine {
    fleet: PathBuf,
    pushed: Vec<PathBuf>,
    published_when_pushed: Vec<bool>,
    stored: std::collections::HashMap<String, [u8; 32]>,
    installed: Vec<String>,
    /// **R15** (`.rulings/` R-157): what `installed_libraries` answers — this
    /// machine's `/state/lib`, as a plain list of reference strings. Empty by
    /// default, the honest answer for a machine that holds no shared rivet.
    library_shelf: Vec<String>,
    /// **`.rulings/` R-194**: what `data_directory_files` answers, and the
    /// `None` is load-bearing — no data directory at all, which is a machine
    /// where this application was never installed. `Some(vec![])` is an
    /// installed application whose directory holds nothing.
    data_directory: Option<Vec<String>>,
    /// Whether anything ever asked. An application declaring no material must
    /// cost the machine no round trip at all.
    data_directory_asked: bool,
    /// The configuration document, when this machine has one.
    document: Option<String>,
    /// How every install asked of this machine ends, as its installer's
    /// `outcome` token and `get install`'s `succeeded`.
    ends: (&'static str, bool),
    /// Every catalogue operation performed, in order.
    performed: Vec<String>,
}

impl Machine {
    /// A machine with a minimal document — every real Rivet has one, and an
    /// install now reads it before it sends, to plan what comes after.
    fn watching(fleet: &Path) -> Self {
        Self {
            fleet: fleet.to_path_buf(),
            pushed: Vec::new(),
            published_when_pushed: Vec::new(),
            stored: std::collections::HashMap::new(),
            installed: Vec::new(),
            library_shelf: Vec::new(),
            data_directory: None,
            data_directory_asked: false,
            document: Some("version = 1\n".to_string()),
            ends: ("installed-application", true),
            performed: Vec::new(),
        }
    }

    /// Every install asked of this machine ends with `outcome`.
    fn ending(mut self, outcome: &'static str, succeeded: bool) -> Self {
        self.ends = (outcome, succeeded);
        self
    }

    /// `get install`'s answer, in `rivet_core::operation::install`'s shape —
    /// HAND-COMPOSED (`AGENTS.md` §4): the latest install is the last one asked
    /// for, and a machine asked for none has no correlation at all.
    fn get_install(&self) -> String {
        let n = self.installed.len();
        if n == 0 {
            return "{\"subject\":\"install\",\"installs\":0,\"records\":[],\"returned\":0,\
                    \"unreadable\":0,\"source\":\"/state/log\",\"next\":\"no install\"}"
                .to_string();
        }
        let (outcome, succeeded) = self.ends;
        format!(
            "{{\"subject\":\"install\",\"installs\":{n},\"correlation\":\"corr-{n}\",\"boot\":\"b\",\
             \"finished\":true,\"outcome\":\"{outcome}\",\"succeeded\":{succeeded},\
             \"means\":\"it ended\",\"errors\":0,\"warnings\":0,\"last\":\"the installer said so\",\
             \"records\":[],\"returned\":0,\"unreadable\":0,\"source\":\"/state/log\",\"next\":\"-\"}}"
        )
    }

    /// This machine already holds `references` under `/state/lib` —
    /// R15's tests set up what a deploy's pre-check should find.
    fn holding_libraries(mut self, references: &[&str]) -> Self {
        self.library_shelf = references.iter().map(ToString::to_string).collect();
        self
    }

    /// The application is installed here and its data directory holds exactly
    /// `files` — `Some(&[])` for an installed application given nothing.
    fn holding_data(mut self, files: &[&str]) -> Self {
        self.data_directory = Some(files.iter().map(ToString::to_string).collect());
        self
    }

    /// This machine's configuration document, which is where R-194's refusal
    /// reads the hostname and whether the service is already enabled.
    fn with_document(mut self, text: &str) -> Self {
        self.document = Some(text.to_string());
        self
    }
}

impl rivet_remote::deploy::Remote for Machine {
    fn push(&mut self, local: &Path, remote_path: &str) -> Result<String, String> {
        let (digest, _) = blake3_of(local)?;
        let hex = crate::key::hex(&digest);
        let held = read_manifest(&self.fleet)
            .map(|entries| entries.iter().any(|e| e.digest == hex))
            .unwrap_or(false);
        self.pushed.push(local.to_path_buf());
        self.published_when_pushed.push(held);
        self.stored.insert(remote_path.to_string(), digest);
        Ok(String::new())
    }
    fn blake3(&mut self, remote_path: &str) -> Result<[u8; 32], String> {
        self.stored
            .get(remote_path)
            .copied()
            .ok_or_else(|| format!("nothing was pushed to {remote_path}"))
    }
    fn install(&mut self, digest: &str) -> Result<String, String> {
        self.installed.push(digest.to_string());
        Ok("{\"delivered\":true}".to_string())
    }
    fn read_document(&mut self) -> Result<Option<String>, String> {
        // **An error when a test took the document away**, rather than `None`
        // — a fake that answered `None` would be a machine that has been asked
        // and said "none", and that is a different thing.
        self.document
            .clone()
            .map(Some)
            .ok_or_else(|| "this fake holds no document".to_owned())
    }
    /// **Actually applies the edit**, so `--enable` is exercised against a
    /// document that then says what it says.
    fn edit_document(&mut self, change: &rivet_remote::deploy::ConfigChange) -> Result<String, String> {
        let mut doc = self.document.clone().ok_or("this fake holds no document")?;
        match change {
            rivet_remote::deploy::ConfigChange::Append { text } => doc.push_str(text),
            rivet_remote::deploy::ConfigChange::Replace { expect, replace } => {
                doc = doc.replacen(expect.as_str(), replace.as_str(), 1);
            }
        }
        self.document = Some(doc);
        Ok(String::new())
    }
    fn put_document(&mut self, _: &str) -> Result<(), String> {
        Err("this fake holds no document".to_owned())
    }
    fn ask(&mut self, _: &[&str]) -> Result<String, String> {
        Err("this fake runs no commands".to_owned())
    }
    fn perform_operation(&mut self, words: &[&str]) -> Result<String, String> {
        self.performed.push(words.join(" "));
        match words {
            ["get", "install"] => Ok(self.get_install()),
            _ => Err(format!("this fake performs `get install` alone, and was asked {words:?}")),
        }
    }
    fn installed_libraries(&mut self) -> Result<Vec<String>, String> {
        Ok(self.library_shelf.clone())
    }
    fn data_directory_files(&mut self, _: &str) -> Result<Option<Vec<String>>, String> {
        self.data_directory_asked = true;
        Ok(self.data_directory.clone())
    }
}

fn deploy_args(rvt: Option<PathBuf>, confirmed: bool) -> rivet_remote::deploy::DeployArgs {
    rivet_remote::deploy::DeployArgs {
        host: "pi:7777".to_string(),
        rvt,
        package_args: Vec::new(),
        remote_dir: rivet_remote::deploy::DEFAULT_REMOTE_DIR.to_string(),
        confirmed,
        enable: false,
        identity: None,
        pin: None,
        // What `cargo rivet install` does unless told `--no-wait`.
        wait: true,
    }
}

/// A seed file of `seed_byte`, **outside** any fleet repository — `ADR-052` §4
/// would refuse a repository holding one, and the point is that it holds none.
fn seed_file(label: &str, seed_byte: u8) -> PathBuf {
    let path = scratch(label).join("app.seed");
    std::fs::write(&path, [seed_byte; 32]).expect("write a scratch seed");
    path
}

fn never_builds(_: &[String]) -> Result<(PathBuf, String), String> {
    panic!("a deploy handed a payload must not build")
}

/// **Handed a crate: build, publish, then send the published digest.** The
/// repository is absent beforehand, so this also covers a first deploy making
/// it.
#[test]
fn deploy_from_a_crate_publishes_then_sends_the_published_digest() {
    let fleet = scratch("deploy-crate").join("fleet");
    let build_dir = scratch("deploy-crate-build");
    let seed = seed_file("deploy-crate-seed", 51);
    let mut built = None;
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture(&build_dir, "crate-rivet", "0.1.0", 51);
        built = Some(artifact.clone());
        Ok((artifact, "built    crate-rivet\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet);

    let report = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("a crate deploy publishes and sends");

    let entries = read_manifest(&fleet).expect("the repository was made and written");
    assert_eq!(entries.len(), 1, "{report}");
    let digest = entries[0].digest.clone();
    assert_eq!(machine.installed, [digest.clone()], "the install names the published digest");
    assert_eq!(machine.published_when_pushed, [true], "published BEFORE it was sent");
    assert_eq!(
        std::fs::read(&machine.pushed[0]).expect("the sent file"),
        std::fs::read(fleet.join(OBJECTS_DIR).join(&digest)).expect("the stored object"),
        "what was sent is the repository's object"
    );
    assert_eq!(
        std::fs::read(&machine.pushed[0]).expect("the sent file"),
        std::fs::read(built.expect("it built")).expect("the built file"),
    );
    assert!(report.contains("published into the fleet repository"), "{report}");
    // **R14** (`.rulings/` R-157): a deploy that found no fleet repository
    // creates one and says so prominently, once.
    assert!(
        report.contains(&format!("CREATED the fleet repository at {}", fleet.display())),
        "a first deploy into an absent repository must announce creation:\n{report}"
    );
}

/// **R14** (`.rulings/` R-157): a SECOND deploy into a repository this
/// workstation already made must not announce creation again — only the
/// deploy that actually made it does.
#[test]
fn a_second_deploy_into_an_existing_repository_does_not_announce_creation() {
    let fleet = scratch("deploy-created-once").join("fleet");
    let build_dir = scratch("deploy-created-once-build");
    let seed = seed_file("deploy-created-once-seed", 62);
    let mut machine = Machine::watching(&fleet);

    let mut first_build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture(&build_dir, "created-once-rivet", "0.1.0", 62);
        Ok((artifact, "built    created-once-rivet\n".to_string()))
    };
    let first = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut first_build,
        &mut machine,
    )
    .expect("the first deploy creates the repository");
    assert!(first.contains("CREATED the fleet repository"), "{first}");

    let mut second_build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture(&build_dir, "created-once-rivet", "0.2.0", 62);
        Ok((artifact, "built    created-once-rivet\n".to_string()))
    };
    let second = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut second_build,
        &mut machine,
    )
    .expect("the second deploy sends into the now-existing repository");
    assert!(
        !second.contains("CREATED the fleet repository"),
        "a repository that already existed before this deploy must not be announced as \
         created:\n{second}"
    );
}

/// **Handed a `.rvt`: publish it first, then the same path.**
#[test]
fn deploy_of_an_rvt_publishes_then_sends_the_published_digest() {
    let fleet = scratch("deploy-rvt");
    init(&fleet).expect("init");
    let (artifact, _) = fixture(&scratch("deploy-rvt-fixture"), "rvt-rivet", "0.2.0", 52);
    let seed = seed_file("deploy-rvt-seed", 52);
    let mut machine = Machine::watching(&fleet);

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(artifact.clone()), true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut never_builds,
        &mut machine,
    )
    .expect("an --rvt deploy publishes and sends");

    let entries = read_manifest(&fleet).expect("manifest");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].product, "rvt-rivet");
    assert_eq!(machine.installed, [entries[0].digest.clone()]);
    assert_eq!(machine.published_when_pushed, [true]);
    // An application install records nothing about slots and points at no
    // recording step (`ADR-052` §7).
    assert!(!report.contains("fleet record"), "{report}");
    assert!(!fleet.join(rivetos_fleet_repo::REGISTER_DIR).exists());
}

/// **A publish that refuses sends nothing.** An artifact the application seed
/// does not vouch for stops at the workstation.
#[test]
fn an_artifact_the_seed_does_not_vouch_for_is_neither_published_nor_sent() {
    let fleet = scratch("deploy-wrong-seed");
    init(&fleet).expect("init");
    let (artifact, _) = fixture(&scratch("deploy-wrong-seed-fixture"), "stranger-rivet", "0.1.0", 53);
    let seed = seed_file("deploy-wrong-seed-seed", 54);
    let mut machine = Machine::watching(&fleet);

    let err = publish_then_send(
        &fleet,
        deploy_args(Some(artifact), true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut never_builds,
        &mut machine,
    )
    .expect_err("the wrong seed refuses the publish");
    assert!(err.contains("NOTHING WAS SENT"), "{err}");
    assert!(machine.pushed.is_empty());
    assert!(read_manifest(&fleet).expect("manifest").is_empty());
}

/// **A missing seed is refused by name, naming both places looked** —
/// `.rulings/` R-155 R3 — and nothing is published, made or sent.
#[test]
fn a_missing_seed_is_refused_naming_both_places_it_looked() {
    let rivet_root = scratch("deploy-no-seed-home");
    let fleet = rivet_root.join("fleet");
    let (artifact, _) = fixture(&scratch("deploy-no-seed-fixture"), "seedless-rivet", "0.1.0", 55);
    let mut machine = Machine::watching(&fleet);
    let over = crate::credentials::Overrides::default();

    let err = publish_then_send(
        &fleet,
        deploy_args(Some(artifact), true),
        &Invocation::default(),
        &mut || deploy_app_seed(&over, Some(&rivet_root)),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut never_builds,
        &mut machine,
    )
    .expect_err("no seed anywhere");

    // Each place on its own line, with what was found there — a name that only
    // appears in the advice afterwards is not a place the refusal says it looked.
    let default = rivet_root.join("keys").join("rivet-app.seed").display().to_string();
    assert!(
        err.lines().any(|l| l.contains("$RIVET_APP_SEED") && l.contains("not set")),
        "{err}"
    );
    assert!(
        err.lines().any(|l| l.contains(&default) && l.contains("not a file")),
        "{err}"
    );
    assert!(err.contains("NOTHING WAS PUBLISHED AND NOTHING WAS SENT"), "{err}");
    assert!(machine.pushed.is_empty());
    assert!(!fleet.exists(), "no repository is made for a deploy that cannot publish");
}

/// The lookup's other two answers: the environment wins, and the default is
/// used when it is a file.
#[test]
fn the_seed_lookup_takes_the_environment_then_the_default() {
    let rivet_root = scratch("seed-lookup-home");
    let named = PathBuf::from("named-by-the-environment.seed");
    let over = crate::credentials::Overrides {
        app_seed: Some(named.clone()),
        ..Default::default()
    };
    assert_eq!(deploy_app_seed(&over, Some(&rivet_root)).expect("named"), named);

    let default = rivet_root.join("keys").join("rivet-app.seed");
    std::fs::create_dir_all(default.parent().expect("keys/")).expect("keys/");
    std::fs::write(&default, [1u8; 32]).expect("a default seed");
    let none = crate::credentials::Overrides::default();
    assert_eq!(deploy_app_seed(&none, Some(&rivet_root)).expect("the default"), default);
}

/// **Re-deploying an artifact the repository already holds does not fail at
/// publish**: it is `publish`'s idempotent `have`, and the deploy goes on.
#[test]
fn redeploying_an_already_published_artifact_does_not_fail_at_publish() {
    let fleet = scratch("deploy-again");
    init(&fleet).expect("init");
    let (artifact, key) = fixture(&scratch("deploy-again-fixture"), "again-rivet", "0.3.0", 56);
    publish(&fleet, &artifact, &key).expect("published beforehand");
    let seed = seed_file("deploy-again-seed", 56);
    let mut machine = Machine::watching(&fleet);

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(artifact), true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut never_builds,
        &mut machine,
    )
    .expect("an already-published artifact deploys");

    assert!(report.contains("have"), "{report}");
    let entries = read_manifest(&fleet).expect("manifest");
    assert_eq!(entries.len(), 1, "no second entry");
    assert_eq!(machine.installed, [entries[0].digest.clone()]);
}

/// **Without `confirm`, nothing is published** — the plan changes nothing on
/// either machine, and the repository is on this one. It LOOKS for the seed,
/// to name it among its NEEDS, and reads none of it.
#[test]
fn a_plan_publishes_nothing() {
    let fleet = scratch("deploy-plan").join("fleet");
    let (artifact, _) = fixture(&scratch("deploy-plan-fixture"), "plan-rivet", "0.1.0", 57);
    let seed = seed_file("deploy-plan-seed", 57);
    let mut machine = Machine::watching(&fleet);

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(artifact), false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application's plan must not name the system seed"),
        &mut never_builds,
        &mut machine,
    )
    .expect("a plan");
    assert!(report.contains("would publish"), "{report}");
    assert!(!fleet.exists());
    assert!(machine.pushed.is_empty());
}

/// **A plan ends by naming what the run with `confirm` will touch** — the
/// network destination, the files it reads and which hold private keys, the
/// repository it writes, and what happens on the machine — composed from the
/// arguments that run will use.
#[test]
fn a_plan_names_what_the_confirmed_run_needs() {
    let fleet = scratch("deploy-needs").join("fleet");
    let (artifact, _) = fixture(&scratch("deploy-needs-fixture"), "needs-rivet", "0.1.0", 63);
    let seed = seed_file("deploy-needs-seed", 63);
    let identity = scratch("deploy-needs-identity");
    let mut args = deploy_args(Some(artifact.clone()), false);
    args.identity = Some(identity.clone());
    args.pin = Some("ab".repeat(32));
    args.enable = true;
    let invocation = Invocation { device: Some("dev-01".to_owned()), ..Invocation::default() };

    let report = publish_then_send(
        &fleet,
        args,
        &invocation,
        &mut || Ok(seed.clone()),
        &mut || panic!("an application's plan must not name the system seed"),
        &mut never_builds,
        &mut Machine::watching(&fleet),
    )
    .expect("a plan");
    let needs = &report[report.find("\nNEEDS").expect("a NEEDS block closes the plan")..];
    assert!(needs.contains("network   pi:7777 (TCP)"), "{needs}");
    assert!(needs.contains("rivet-fleet's register, to turn `dev-01`"), "{needs}");
    assert!(needs.contains(&format!("{}: the operator identity, which holds a private key", identity.display())), "{needs}");
    assert!(needs.contains(&format!("{}: the application signing seed, a private key\n", seed.display())), "{needs}");
    assert!(needs.contains(&format!("{}: the payload", artifact.display())), "{needs}");
    assert!(needs.contains(&format!("writes    {}: the fleet repository", fleet.display())), "{needs}");
    assert!(needs.contains("on dev-01: sends "), "{needs}");
    assert!(needs.contains("appending its enable to /state/config/machine.toml"), "{needs}");
    assert!(!needs.contains("cargo, to build"), "a payload handed in is not built: {needs}");
}

/// **Selected from the repository**: nothing is published, the seed is read
/// only to plan the steps after the install, and what it writes is the copy
/// the plan has already made.
#[test]
fn a_plan_of_a_selected_artifact_publishes_nothing_and_says_why_it_reads_the_seed() {
    let fleet = scratch("deploy-needs-selected").join("fleet");
    let (artifact, _) = fixture(&scratch("deploy-needs-selected-copy"), "chosen-rivet", "0.1.0", 64);
    let seed = seed_file("deploy-needs-selected-seed", 64);
    let invocation = Invocation { selected: true, ..Invocation::default() };

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(artifact.clone()), false),
        &invocation,
        &mut || Ok(seed.clone()),
        &mut || panic!("an application's plan must not name the system seed"),
        &mut never_builds,
        &mut Machine::watching(&fleet),
    )
    .expect("a plan");
    let needs = &report[report.find("\nNEEDS").expect("a NEEDS block")..];
    assert!(needs.contains("only to plan the steps that remain"), "{needs}");
    let copy_dir = artifact.parent().expect("the copy's directory");
    assert!(needs.contains(&format!("writes    {}: a copy of the payload", copy_dir.display())), "{needs}");
    assert!(!needs.contains("the fleet repository;"), "{needs}");
}

/// **What is missing is named as the refusal it will be.** No seed where the
/// lookup looks, and no identity given: the plan still prints, and says the
/// run with `confirm` would stop on each.
#[test]
fn a_plan_names_a_missing_seed_and_a_missing_identity_as_the_refusals_they_will_be() {
    let fleet = scratch("deploy-needs-missing").join("fleet");
    let (artifact, _) = fixture(&scratch("deploy-needs-missing-fixture"), "short-rivet", "0.1.0", 65);

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(artifact), false),
        &Invocation::default(),
        &mut || Err("no application signing seed was found".to_owned()),
        &mut || panic!("an application's plan must not name the system seed"),
        &mut never_builds,
        &mut Machine::watching(&fleet),
    )
    .expect("a plan prints whatever is missing");
    assert!(report.contains("the application signing seed: NONE FOUND"), "{report}");
    assert!(report.contains("an operator identity and the machine's pin: NONE GIVEN"), "{report}");
}

/// **Handed a crate, the plan says it will build** — cargo runs, and fetches
/// what it has not cached, which is network a sandbox has to allow.
#[test]
fn a_plan_of_a_crate_says_cargo_will_build_it() {
    let fleet = scratch("deploy-needs-crate").join("fleet");
    let seed = seed_file("deploy-needs-crate-seed", 66);
    let report = publish_then_send(
        &fleet,
        deploy_args(None, false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application's plan must not name the system seed"),
        &mut never_builds,
        &mut Machine::watching(&fleet),
    )
    .expect("a plan builds nothing");
    assert!(report.contains("cargo, to build for the board's target"), "{report}");
    assert!(report.contains("the crate `cargo rivet package` builds"), "{report}");
}

/// **A system update's plan names the SYSTEM seed and the slot it writes**, and
/// says nothing reboots until the trial is asked for.
#[test]
fn a_plan_of_a_system_update_names_the_system_seed_and_the_inactive_slot() {
    let fleet = scratch("deploy-needs-update").join("fleet");
    let dir = scratch("deploy-needs-update-fixture");
    let mut signer = Ed25519Signer::from_seed(&[67u8; 32]);
    let mut digest = Blake3::new();
    let out = dir.join(rivet_remote::name::artifact("rivet", "0.1.99", "aarch64", Some("rpi4"), &Kind::update()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product: "rivet",
            version: "0.1.99",
            arch: "aarch64",
            platform: Some("rpi4"),
            kind: Kind::update(),
        },
        &[crate::emit::Part {
            name: "system".to_string(),
            class: 0,
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's system bytes".to_vec()),
        }],
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("an update fixture is written");
    let system_seed = seed_file("deploy-needs-update-seed", 67);

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(out), false),
        &Invocation::default(),
        &mut || panic!("a system update's plan must not name the application seed"),
        &mut || Ok(system_seed.clone()),
        &mut never_builds,
        &mut Machine::watching(&fleet),
    )
    .expect("a plan");
    assert!(report.contains(&format!("{}: the system signing seed", system_seed.display())), "{report}");
    assert!(report.contains("writes the slot it is NOT running"), "{report}");
    assert!(report.contains("Nothing reboots"), "{report}");
}

/// **`R-177` D6: a system update publishes too, against the SYSTEM seed —
/// never the application one — and is sent from the repository like
/// everything else.** This is the end of the exception the superseded test
/// name recorded; see this file's own history for what it read before.
#[test]
fn a_system_update_publishes_against_the_system_seed_then_sends() {
    let fleet = scratch("deploy-update").join("fleet");
    let dir = scratch("deploy-update-fixture");
    let system_seed_bytes = [58u8; 32];
    let mut signer = Ed25519Signer::from_seed(&system_seed_bytes);
    let mut digest = Blake3::new();
    let out = dir.join(rivet_remote::name::artifact("rivet", "0.1.99", "aarch64", Some("rpi4"), &Kind::update()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product: "rivet",
            version: "0.1.99",
            arch: "aarch64",
            platform: Some("rpi4"),
            kind: Kind::update(),
        },
        &[crate::emit::Part {
            name: "system".to_string(),
            class: 0, // `TargetClass::WholePartition` — required, exactly one.
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's system bytes".to_vec()),
        }],
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("an update fixture is written");
    let system_seed = seed_file("deploy-update-system-seed", 58);
    let mut machine = Machine::watching(&fleet);

    let report = publish_then_send(
        &fleet,
        deploy_args(Some(out), true),
        &Invocation::default(),
        &mut || panic!("a system update must not reach for the application seed"),
        &mut || Ok(system_seed.clone()),
        &mut never_builds,
        &mut machine,
    )
    .expect("an update publishes and is sent");
    assert!(report.contains("published into the fleet repository"), "{report}");
    assert!(!report.contains("NOT PUBLISHED"), "{report}");

    let entries = read_manifest(&fleet).expect("the update was published");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "update");
    assert_eq!(entries[0].product, "rivet");
    assert_eq!(machine.published_when_pushed, [true], "published BEFORE it was sent");
    assert_eq!(machine.pushed.len(), 1);
    assert_eq!(machine.installed.len(), 1);

    // **The install records nothing**: it returns before the trial, so what the
    // machine runs is recorded by `fleet record` once it has committed, and the
    // report says so (`ADR-052` §7, R-233).
    assert!(report.contains("fleet record"), "{report}");
    assert!(report.contains("trial-restart"), "{report}");
    assert!(!fleet.join(rivetos_fleet_repo::REGISTER_DIR).exists(), "an install writes no host record");
}

/// **A system update signed by a key that is not the system seed refuses at
/// publish**, exactly as any other wrongly-signed artifact does — the
/// application seed does not vouch for it and `deploy_system_seed` is never
/// even asked, since the seed here is named directly.
#[test]
fn a_system_update_signed_by_the_wrong_key_is_neither_published_nor_sent() {
    let fleet = scratch("deploy-update-wrong-key").join("fleet");
    let dir = scratch("deploy-update-wrong-key-fixture");
    let mut signer = Ed25519Signer::from_seed(&[59u8; 32]);
    let mut digest = Blake3::new();
    let out = dir.join(rivet_remote::name::artifact("rivet", "0.1.99", "aarch64", Some("rpi4"), &Kind::update()));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product: "rivet",
            version: "0.1.99",
            arch: "aarch64",
            platform: Some("rpi4"),
            kind: Kind::update(),
        },
        &[crate::emit::Part {
            name: "system".to_string(),
            class: 1,
            variant: VARIANT_NONE,
            source: crate::emit::Source::Bytes(b"a fixture's system bytes".to_vec()),
        }],
        &[],
        &mut digest,
        &mut signer,
    )
    .expect("an update fixture is written");
    // A DIFFERENT seed from the one that signed it.
    let wrong_seed = seed_file("deploy-update-wrong-key-seed", 60);
    let mut machine = Machine::watching(&fleet);

    let err = publish_then_send(
        &fleet,
        deploy_args(Some(out), true),
        &Invocation::default(),
        &mut || panic!("a system update must not reach for the application seed"),
        &mut || Ok(wrong_seed.clone()),
        &mut never_builds,
        &mut machine,
    )
    .expect_err("the wrong system key refuses the publish");
    assert!(err.contains("NOTHING WAS SENT"), "{err}");
    assert!(machine.pushed.is_empty());
    assert!(read_manifest(&fleet).expect("manifest").is_empty());
}

// ---------------------------------------------------------------------------
// R15 (`.rulings/` R-157): deploy refuses an application whose declared
// closure this machine does not hold, before anything is sent.
// ---------------------------------------------------------------------------

/// **All present: the deploy proceeds.** The ordinary case for an application
/// that declares a closure and the machine already holds every one of it.
#[test]
fn an_application_whose_declared_libraries_are_all_present_deploys() {
    let fleet = scratch("deploy-libs-present").join("fleet");
    let build_dir = scratch("deploy-libs-present-build");
    let seed = seed_file("deploy-libs-present-seed", 63);
    let reference = format!("libgreet-2.0.0-{}", "ab".repeat(32));
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_declaring(&build_dir, "needs-libgreet", "0.1.0", 63, &[&reference]);
        Ok((artifact, "built    needs-libgreet\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet).holding_libraries(&[&reference]);

    let report = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("every declared library is already on the machine, so the deploy proceeds");

    assert_eq!(machine.installed.len(), 1, "{report}");
}

/// **One missing: refused, nothing sent, named, and a command given.**
#[test]
fn an_application_missing_one_declared_library_is_refused_and_nothing_is_sent() {
    let fleet = scratch("deploy-libs-missing-one").join("fleet");
    let build_dir = scratch("deploy-libs-missing-one-build");
    let seed = seed_file("deploy-libs-missing-one-seed", 64);
    let reference = format!("libgreet-2.0.0-{}", "cd".repeat(32));
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) =
            fixture_declaring(&build_dir, "needs-libgreet-2", "0.1.0", 64, &[&reference]);
        Ok((artifact, "built    needs-libgreet-2\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet); // holds nothing

    let err = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("a machine missing a declared library refuses the deploy");

    assert!(err.contains("NOTHING WAS SENT"), "{err}");
    assert!(err.contains(&reference), "{err}");
    assert!(
        err.contains("cargo rivet fleet publish") || err.contains("cargo rivet install --artifact"),
        "R15 must give the exact command that deploys the missing library:\n{err}"
    );
    assert!(machine.pushed.is_empty(), "nothing must reach the machine: {:?}", machine.pushed);
    assert!(machine.installed.is_empty());
    // The publish itself — a workstation-side act, distinct from SENDING —
    // still went through: R15 refuses the send, not the publish.
    assert_eq!(
        read_manifest(&fleet).expect("the manifest").len(),
        1,
        "the application is still published into the fleet repository"
    );
}

/// **Several missing: every one is named.**
#[test]
fn an_application_missing_several_declared_libraries_names_all_of_them() {
    let fleet = scratch("deploy-libs-missing-several").join("fleet");
    let build_dir = scratch("deploy-libs-missing-several-build");
    let seed = seed_file("deploy-libs-missing-several-seed", 65);
    let one = format!("libgreet-2.0.0-{}", "11".repeat(32));
    let two = format!("libaudio-3.1.0-{}", "22".repeat(32));
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) =
            fixture_declaring(&build_dir, "needs-two-libs", "0.1.0", 65, &[&one, &two]);
        Ok((artifact, "built    needs-two-libs\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet);

    let err = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("two missing declared libraries refuse the deploy");

    assert!(err.contains(&one), "{err}");
    assert!(err.contains(&two), "{err}");
    assert!(machine.pushed.is_empty());
}

/// **The command given names an `--artifact` already in this fleet repository**
/// when the missing library happens to be published there — the honest,
/// specific case rather than the generic fallback.
#[test]
fn a_missing_librarys_command_names_the_artifact_already_published_for_it() {
    let fleet = scratch("deploy-libs-missing-published").join("fleet");
    init(&fleet).expect("init");
    let (library_artifact, library_key) = library_fixture(
        &scratch("deploy-libs-missing-published-lib"),
        "libgreet",
        "2.0.0",
        b"a published libgreet build",
        66,
    );
    publish(&fleet, &library_artifact, &library_key).expect("the library publishes beforehand");
    let reference = crate::library::reference_of(&library_artifact, "libgreet", "2.0.0")
        .expect("a reference for the fixture");

    let build_dir = scratch("deploy-libs-missing-published-build");
    let seed = seed_file("deploy-libs-missing-published-seed", 67);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_declaring(
            &build_dir,
            "needs-published-libgreet",
            "0.1.0",
            67,
            &[reference.as_str()],
        );
        Ok((artifact, "built    needs-published-libgreet\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet); // does not hold the library yet

    let err = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("the machine does not hold the library yet");

    assert!(err.contains("cargo rivet install --artifact"), "{err}");
    assert!(
        !err.contains("not published in this fleet repository"),
        "the library IS published; the fallback wording must not appear:\n{err}"
    );
}

/// **An application with no declared closure is unaffected** — `SPEC-02` §5's
/// ordinary case — and the machine is never asked about `/state/lib` at all.
#[test]
fn an_application_with_no_declared_closure_never_asks_about_libraries() {
    let fleet = scratch("deploy-no-closure").join("fleet");
    let build_dir = scratch("deploy-no-closure-build");
    let seed = seed_file("deploy-no-closure-seed", 68);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture(&build_dir, "no-closure-rivet", "0.1.0", 68);
        Ok((artifact, "built    no-closure-rivet\n".to_string()))
    };
    struct RefusingLibraryReads(Machine);
    impl rivet_remote::deploy::Remote for RefusingLibraryReads {
        fn push(&mut self, l: &Path, r: &str) -> Result<String, String> {
            self.0.push(l, r)
        }
        fn blake3(&mut self, r: &str) -> Result<[u8; 32], String> {
            self.0.blake3(r)
        }
        fn install(&mut self, digest: &str) -> Result<String, String> {
            self.0.install(digest)
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            self.0.read_document()
        }
        fn edit_document(&mut self, c: &rivet_remote::deploy::ConfigChange) -> Result<String, String> {
            self.0.edit_document(c)
        }
        fn put_document(&mut self, t: &str) -> Result<(), String> {
            self.0.put_document(t)
        }
        fn ask(&mut self, w: &[&str]) -> Result<String, String> {
            self.0.ask(w)
        }
        fn installed_libraries(&mut self) -> Result<Vec<String>, String> {
            panic!("an application with no declared closure must never ask a machine anything")
        }
        fn perform_operation(&mut self, w: &[&str]) -> Result<String, String> {
            self.0.perform_operation(w)
        }
    }
    let mut machine = RefusingLibraryReads(Machine::watching(&fleet));

    let report = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("an application with no closure deploys as before");
    assert_eq!(machine.0.installed.len(), 1, "{report}");
}

// ---------------------------------------------------------------------------
// R-194: deploy refuses to put an application into service without the
// per-machine material its manifest declares — ADR-055 §3, and it mints none.
// ---------------------------------------------------------------------------

use rivet_core::package::application::Material;

/// A document that names this machine and says the service should run — the
/// two facts R-194's refusal reads out of one `get document`.
fn document_enabling(hostname: &str, service: &str) -> String {
    format!("version = 1\nhostname = \"{hostname}\"\n\n[services.{service}]\nenabled = true\n")
}

/// Deploy's arguments with an identity and a pin, so the refusal's suggested
/// command can be checked for the values this invocation already knows rather
/// than for placeholders.
fn deploy_args_reaching(rvt: Option<PathBuf>, enable: bool) -> rivet_remote::deploy::DeployArgs {
    let mut args = deploy_args(rvt, true);
    args.enable = enable;
    args.identity = Some(PathBuf::from("id-dir"));
    args.pin = Some("abc123".to_string());
    args
}

/// **Present: the deploy proceeds.** The ordinary case for an application that
/// declares material and whose machine was given it — nothing is refused and
/// the install happens.
#[test]
fn an_application_whose_declared_material_is_present_deploys() {
    let fleet = scratch("deploy-material-present").join("fleet");
    let build_dir = scratch("deploy-material-present-build");
    let seed = seed_file("deploy-material-present-seed", 71);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) =
            fixture_needing_material(&build_dir, "has-tls-rivet", "0.1.0", 71, &[Material::FleetTls]);
        Ok((artifact, "built    has-tls-rivet\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet)
        .holding_data(&["tls-cert", "tls-key", "tls-root", "config.json"])
        .with_document(&document_enabling("rivet-06f5c", "has-tls-rivet"));

    let report = publish_then_send(
        &fleet,
        deploy_args_reaching(None, false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("the machine holds every declared file, so the deploy proceeds");

    assert_eq!(machine.installed.len(), 1, "{report}");
    assert!(machine.data_directory_asked, "the machine must have been asked");
    assert!(
        report.contains("holds every file"),
        "a satisfied declaration is reported, not silent:\n{report}"
    );
}

/// **Missing, and the application would be RUNNING: refused, nothing sent.**
/// This is `ADR-055` §1's failure — a service enabled without the material it
/// declared is a crash-loop read out of a journal on a headless machine.
///
/// It also pins the whole of the refusal's text, which is the deliverable:
/// the application, each missing file, the one `rivet-provision ca server`
/// line and that it is per MACHINE, the `cargo rivet secrets` line with this
/// invocation's own host, identity and pin filled in, and the sentence that
/// says to install the material and run this again.
#[test]
fn material_missing_on_a_service_that_would_run_is_refused_and_names_the_commands() {
    let fleet = scratch("deploy-material-missing").join("fleet");
    let build_dir = scratch("deploy-material-missing-build");
    let seed = seed_file("deploy-material-missing-seed", 72);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_needing_material(
            &build_dir,
            "needs-tls-rivet",
            "0.1.0",
            72,
            &[Material::FleetTls],
        );
        Ok((artifact, "built    needs-tls-rivet\n".to_string()))
    };
    // Installed — the directory exists — and holding nothing of what it
    // declared, with the document already saying it should run.
    let mut machine = Machine::watching(&fleet)
        .holding_data(&["config.json"])
        .with_document(&document_enabling("rivet-06f5c", "needs-tls-rivet"));

    let err = publish_then_send(
        &fleet,
        deploy_args_reaching(None, false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("an enabled application missing its declared material refuses the deploy");

    assert!(err.contains("NOTHING WAS SENT"), "{err}");
    // 1 — the application.
    assert!(err.contains("needs-tls-rivet"), "{err}");
    // 2 — which files are missing, and where they go.
    for file in Material::FleetTls.files() {
        assert!(err.contains(file), "{file} is not named:\n{err}");
    }
    assert!(err.contains("/state/app/needs-tls-rivet/data"), "{err}");
    // 3 — the authority line, once per MACHINE, with the machine's own name.
    assert!(
        err.contains("rivet-provision ca server rivet-06f5c "),
        "the hostname read off the machine must be filled in:\n{err}"
    );
    assert!(err.contains("once per MACHINE"), "{err}");
    // 4 — the install line, with what this invocation already knows.
    assert!(
        err.contains(
            "cargo rivet secrets needs-tls-rivet --host pi:7777 --identity id-dir --pin abc123 \
             --name rivet-06f5c.local confirm"
        ),
        "the secrets command must carry this deploy's own values:\n{err}"
    );
    // 5 — and that installing it and re-running is the way forward.
    assert!(
        err.contains("Install it yourself, then run this install again"),
        "{err}"
    );
    // Nothing reached the machine, and nothing was minted here.
    assert!(machine.pushed.is_empty(), "{:?}", machine.pushed);
    assert!(machine.installed.is_empty());
    assert!(
        !err.contains("minting") && err.contains("never by a deploy"),
        "the refusal must say the authority is the operator's:\n{err}"
    );
}

/// **`--enable` is the other half of "would run"**, and it is refused even
/// though the document says nothing yet: the flag is a request to start the
/// application, and starting it without its material is the same crash-loop.
/// The refusal also says how to get out of it, because an application with no
/// data directory cannot be given material until something makes one.
#[test]
fn enable_without_the_material_is_refused_and_says_to_install_first() {
    let fleet = scratch("deploy-material-enable").join("fleet");
    let build_dir = scratch("deploy-material-enable-build");
    let seed = seed_file("deploy-material-enable-seed", 73);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_needing_material(
            &build_dir,
            "fresh-tls-rivet",
            "0.1.0",
            73,
            &[Material::FleetTls],
        );
        Ok((artifact, "built    fresh-tls-rivet\n".to_string()))
    };
    // Never installed here: no data directory at all.
    let mut machine = Machine::watching(&fleet)
        .with_document("version = 1\nhostname = \"rivet-06f5c\"\n");

    let err = publish_then_send(
        &fleet,
        deploy_args_reaching(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("--enable without the declared material refuses the deploy");

    assert!(err.contains("NOTHING WAS SENT"), "{err}");
    assert!(
        err.contains("WITHOUT --enable first"),
        "a first deploy must be told the order that works:\n{err}"
    );
    assert!(machine.pushed.is_empty());
    // The document must not have been edited: `--enable` writes BEFORE the
    // payload is sent, and this refusal comes first.
    assert!(machine.installed.is_empty());
}

/// **Missing, and nothing would run: REPORTED, and the deploy proceeds.**
///
/// Installing is not running (`ADR-015` §3), and this is the case that
/// must not be refused: `apply install-secret` refuses a data directory that
/// is not there, so installing the application is what makes the place its
/// material goes. A refusal here would be a wall.
#[test]
fn material_missing_with_nothing_enabled_is_reported_and_the_deploy_proceeds() {
    let fleet = scratch("deploy-material-idle").join("fleet");
    let build_dir = scratch("deploy-material-idle-build");
    let seed = seed_file("deploy-material-idle-seed", 74);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_needing_material(
            &build_dir,
            "idle-tls-rivet",
            "0.1.0",
            74,
            &[Material::FleetTls],
        );
        Ok((artifact, "built    idle-tls-rivet\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet)
        .with_document("version = 1\nhostname = \"rivet-06f5c\"\n");

    let report = publish_then_send(
        &fleet,
        deploy_args_reaching(None, false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("an install that starts nothing is not refused");

    assert_eq!(machine.installed.len(), 1, "{report}");
    assert!(report.contains("cargo rivet secrets idle-tls-rivet"), "{report}");
    assert!(report.contains("rivet-provision ca server rivet-06f5c"), "{report}");
    assert!(report.contains("sit idle"), "{report}");
}

/// **On a machine that holds its fleet's root, `--enable` without the material
/// is NOT refused** — `ADR-063` §4. The machine issues the material itself at
/// this install, so the install proceeds, the report says so, and no
/// `cargo rivet secrets` or `ca server` step is printed.
#[test]
fn enable_on_a_fleet_root_machine_proceeds_and_offers_no_secrets_step() {
    let fleet = scratch("deploy-material-fleet-root").join("fleet");
    let build_dir = scratch("deploy-material-fleet-root-build");
    let seed = seed_file("deploy-material-fleet-root-seed", 76);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_needing_material(
            &build_dir,
            "root-tls-rivet",
            "0.1.0",
            76,
            &[Material::FleetTls],
        );
        Ok((artifact, "built    root-tls-rivet\n".to_string()))
    };
    struct FleetRoot(Machine);
    impl rivet_remote::deploy::Remote for FleetRoot {
        fn push(&mut self, l: &Path, r: &str) -> Result<String, String> {
            self.0.push(l, r)
        }
        fn blake3(&mut self, r: &str) -> Result<[u8; 32], String> {
            self.0.blake3(r)
        }
        fn install(&mut self, digest: &str) -> Result<String, String> {
            self.0.install(digest)
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            self.0.read_document()
        }
        fn edit_document(&mut self, c: &rivet_remote::deploy::ConfigChange) -> Result<String, String> {
            self.0.edit_document(c)
        }
        fn put_document(&mut self, t: &str) -> Result<(), String> {
            self.0.put_document(t)
        }
        fn ask(&mut self, w: &[&str]) -> Result<String, String> {
            self.0.ask(w)
        }
        fn data_directory_files(&mut self, a: &str) -> Result<Option<Vec<String>>, String> {
            self.0.data_directory_files(a)
        }
        fn holds_fleet_root(&mut self) -> Result<bool, String> {
            Ok(true)
        }
        fn perform_operation(&mut self, w: &[&str]) -> Result<String, String> {
            self.0.perform_operation(w)
        }
    }
    // Never installed here, and asked to run: on any other machine this is
    // `enable_without_the_material_is_refused_and_says_to_install_first`.
    let mut machine = FleetRoot(
        Machine::watching(&fleet).with_document("version = 1\nhostname = \"dev-01\"\n"),
    );

    let report = publish_then_send(
        &fleet,
        deploy_args_reaching(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("a fleet-root machine issues the material, so nothing is refused");

    assert_eq!(machine.0.installed.len(), 1, "{report}");
    assert!(report.contains("issues root-tls-rivet its fleet-tls itself"), "{report}");
    assert!(!report.contains("cargo rivet secrets"), "{report}");
    assert!(!report.contains("ca server"), "{report}");
}

/// **A machine that cannot say what its data directory holds refuses the
/// deploy**, rather than being read as holding nothing — `AGENTS.md` §5's
/// refuse-rather-than-guess, at the point where guessing means either a
/// crash-loop or a refusal nobody needed.
#[test]
fn a_machine_that_will_not_answer_about_material_refuses_the_deploy() {
    let fleet = scratch("deploy-material-silent").join("fleet");
    let build_dir = scratch("deploy-material-silent-build");
    let seed = seed_file("deploy-material-silent-seed", 75);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture_needing_material(
            &build_dir,
            "silent-tls-rivet",
            "0.1.0",
            75,
            &[Material::FleetTls],
        );
        Ok((artifact, "built    silent-tls-rivet\n".to_string()))
    };
    struct Silent(Machine);
    impl rivet_remote::deploy::Remote for Silent {
        fn push(&mut self, l: &Path, r: &str) -> Result<String, String> {
            self.0.push(l, r)
        }
        fn blake3(&mut self, r: &str) -> Result<[u8; 32], String> {
            self.0.blake3(r)
        }
        fn install(&mut self, digest: &str) -> Result<String, String> {
            self.0.install(digest)
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            self.0.read_document()
        }
        fn edit_document(&mut self, c: &rivet_remote::deploy::ConfigChange) -> Result<String, String> {
            self.0.edit_document(c)
        }
        fn put_document(&mut self, t: &str) -> Result<(), String> {
            self.0.put_document(t)
        }
        fn ask(&mut self, w: &[&str]) -> Result<String, String> {
            self.0.ask(w)
        }
        fn data_directory_files(&mut self, _: &str) -> Result<Option<Vec<String>>, String> {
            Err("the machine did not answer".to_string())
        }
        fn perform_operation(&mut self, w: &[&str]) -> Result<String, String> {
            self.0.perform_operation(w)
        }
    }
    let mut machine = Silent(Machine::watching(&fleet));

    let err = publish_then_send(
        &fleet,
        deploy_args_reaching(None, false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("a machine that will not answer is not a machine that holds it");

    assert!(err.contains("NOTHING WAS SENT"), "{err}");
    assert!(err.contains("did not answer"), "{err}");
    assert!(machine.0.pushed.is_empty());
}

/// **An application declaring no material is unaffected** — every `.rvt`
/// published before `ADR-055` — and the machine is never asked about a data
/// directory at all.
#[test]
fn an_application_declaring_no_material_never_asks_about_a_data_directory() {
    let fleet = scratch("deploy-plain").join("fleet");
    let build_dir = scratch("deploy-plain-build");
    let seed = seed_file("deploy-plain-seed", 76);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        let (artifact, _) = fixture(&build_dir, "plain-rivet", "0.1.0", 76);
        Ok((artifact, "built    plain-rivet\n".to_string()))
    };
    let mut machine = Machine::watching(&fleet);

    let report = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("an application declaring no material deploys as before");

    assert_eq!(machine.installed.len(), 1, "{report}");
    assert!(
        !machine.data_directory_asked,
        "a declaration nobody made must cost the machine no round trip"
    );
    assert!(!report.contains("material"), "{report}");
}

// ---------------------------------------------------------------------------
// One planner, two front ends: what `install` prints is what the interview
// would offer — `ADR-055` §3.
// ---------------------------------------------------------------------------

/// A routed application declaring `fleet-tls`: every kind of step the planner
/// knows can apply to it.
fn routed_needing_material() -> rivet_core::package::application::Manifest {
    rivet_core::package::application::Manifest::new(ProgramKind::Daemon)
        .with_material(vec![Material::FleetTls])
        .with_proxy_path(Some("tie".to_string()))
        .expect("a valid proxy path")
}

/// A fleet server's document, naming the machine and saying nothing about the
/// application — the state a first install meets.
const SERVER_DOCUMENT: &str =
    "version = 1\nhostname = \"rivet-06f5c\"\n\n[services.rivet-fleetd]\nenabled = true\n";

/// The numbered step headings in `install`'s report, in order.
fn printed_steps(report: &str) -> Vec<String> {
    report
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("  ")?;
            let (number, heading) = rest.split_once(". ")?;
            number.parse::<usize>().ok()?;
            // `render`'s own bracketed danger annotation ("[handles a private
            // key]", or a stale-reader's own cost) is appended after the
            // heading `describe` produced — strip from its first "  [" so
            // this compares against `describe`'s own text, whichever
            // annotation (if any) `render` added.
            Some(heading.split("  [").next().unwrap_or(heading).to_string())
        })
        .collect()
}

/// **THE test that ties the two front ends.** For one artifact on one machine,
/// the steps `cargo rivet install` prints after installing are exactly the
/// steps the interview offers for the same artifact on the same machine —
/// computed here by the interview's OWN functions (`read_artifact_plan_with`,
/// `read_host_state_for`, `dependency_step_group`, `deploy_plan::plan`): the
/// unmet dependency's steps first, then the artifact's less the install it
/// just did, then the one converge the interview adds after an enable, then
/// the fleet sync that is `install`'s alone. A routed application's list
/// carries the proxy restart; a fleet server's ends with the sync.
#[test]
fn the_steps_install_prints_are_the_steps_the_interview_would_offer() {
    let fleet = scratch("tie-planner").join("fleet");
    let build_dir = scratch("tie-planner-build");
    let seed = seed_file("tie-planner-seed", 81);
    // Routed, declaring material, and connecting to a system service this
    // machine has not switched on — every kind of step there is.
    let manifest = routed_needing_material()
        .with_connects(vec![
            rivet_core::package::application::Endpoint::parse("connects", "rivet-bluetoothd:advertisements")
                .expect("a valid endpoint"),
        ])
        .expect("a valid connects");
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        Ok((fixture_with(&build_dir, "tie-rivet", "0.1.0", 81, &manifest), String::new()))
    };
    let mut machine = Machine::watching(&fleet).with_document(SERVER_DOCUMENT);

    let report = publish_then_send(
        &fleet,
        deploy_args_reaching(None, false),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("an application deploy must not reach for the system seed"),
        &mut build,
        &mut machine,
    )
    .expect("an install that only installs proceeds");

    // What the interview would offer, from its own readers against the machine
    // as it stood before the install.
    let entry = read_manifest(&fleet).expect("manifest").into_iter().find(|e| e.product == "tie-rivet").expect("published");
    let artifact = interview::read_artifact_plan_with(&fleet, &entry, &seed).expect("its needs");
    let mut as_it_stood = Machine::watching(&fleet).with_document(SERVER_DOCUMENT);
    let (state, _) = interview::read_host_state_for(&mut as_it_stood, &artifact).expect("the machine");
    let (_, _, dependency_steps, _) = interview::dependency_step_group(
        &mut as_it_stood,
        SERVER_DOCUMENT,
        "tie-rivet",
        "rivet-bluetoothd",
        None,
    )
    .expect("the machine")
    .expect("rivet-bluetoothd is switched off here, so the interview offers its enable");
    let offered = deploy_plan::plan(&artifact, &state);
    assert!(
        matches!(offered.first().map(|s| &s.kind), Some(deploy_plan::StepKind::Install { .. })),
        "{offered:?}"
    );
    let mut expected: Vec<String> = dependency_steps
        .iter()
        .chain(offered.iter().skip(1))
        .map(|step| interview::describe(&step.kind))
        .collect();
    expected.push(interview::describe(&deploy_plan::StepKind::Realize));

    let printed = printed_steps(&report);
    assert!(!printed.is_empty(), "install printed no steps:\n{report}");
    assert_eq!(&printed[..printed.len() - 1], expected.as_slice(), "{report}");
    let restart_step = offered
        .iter()
        .find(|s| matches!(s.kind, deploy_plan::StepKind::RestartReader { .. }))
        .expect("a routed application's plan always offers the proxy's stale-reader restart");
    assert!(
        printed.contains(&interview::describe(&restart_step.kind)),
        "a routed application must be told to restart the proxy:\n{report}"
    );
    assert!(printed.last().is_some_and(|l| l.contains("fleet server")), "{report}");
    assert!(report.contains("cargo rivet fleet sync --host pi:7777"), "{report}");
    // And it waited for its own install before saying any of it.
    assert_eq!(machine.performed, ["get install", "get install"], "{report}");
}

/// **R-194's refusal agrees with the step `install` prints.** The same
/// application on the same machine is refused with `--enable` and installed
/// without it; the refusal names the same missing files and the same two
/// commands the printed `StageMaterial` step does.
#[test]
fn the_material_refusal_names_what_the_printed_step_names() {
    let seed = seed_file("tie-refusal-seed", 82);
    let run = |label: &str, enable: bool| -> Result<String, String> {
        let fleet = scratch(label).join("fleet");
        let build_dir = scratch(&format!("{label}-build"));
        let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
            Ok((fixture_with(&build_dir, "agree-rivet", "0.1.0", 82, &routed_needing_material()), String::new()))
        };
        let mut machine = Machine::watching(&fleet).with_document(SERVER_DOCUMENT);
        publish_then_send(
            &fleet,
            deploy_args_reaching(None, enable),
            &Invocation::default(),
            &mut || Ok(seed.clone()),
            &mut || panic!("no system seed"),
            &mut build,
            &mut machine,
        )
    };
    let refusal = run("tie-refusal-enable", true).expect_err("--enable without material is refused");
    let report = run("tie-refusal-idle", false).expect("an install that only installs proceeds");

    let step = report.split("  1. stage material").nth(1).expect("the printed step");
    let lines: Vec<&str> = step.lines().skip(1).take(2).map(str::trim).collect();
    assert_eq!(lines.len(), 2, "{report}");
    for command in &lines {
        assert!(refusal.contains(command), "the refusal does not name `{command}`:\n{refusal}");
    }
    for file in Material::FleetTls.files() {
        assert!(refusal.contains(file) && step.contains(file), "{file}");
    }
}

/// **A failed install exits non-zero, naming its outcome**, through the whole
/// verb — and prints no next steps, because nothing after it can work.
#[test]
fn a_failed_install_is_a_failure_naming_its_outcome() {
    let fleet = scratch("install-fails").join("fleet");
    let build_dir = scratch("install-fails-build");
    let seed = seed_file("install-fails-seed", 83);
    let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
        Ok((fixture(&build_dir, "fails-rivet", "0.1.0", 83).0, String::new()))
    };
    let mut machine = Machine::watching(&fleet).ending("refused-payload", false);

    let err = publish_then_send(
        &fleet,
        deploy_args(None, true),
        &Invocation::default(),
        &mut || Ok(seed.clone()),
        &mut || panic!("no system seed"),
        &mut build,
        &mut machine,
    )
    .expect_err("the installer refused the payload");
    assert!(err.contains("THE INSTALL FAILED"), "{err}");
    assert!(err.contains("refused-payload"), "{err}");
    assert!(!err.contains("next     "), "a failed install prints no next steps:\n{err}");
}

/// **`--artifact` reads the manifest too**, so an agent's usual route gets the
/// same steps — and a workstation with no application seed still installs,
/// and is told the steps could not be planned and why.
#[test]
fn a_selected_artifact_is_planned_and_a_missing_seed_only_costs_the_plan() {
    let fleet = scratch("select-plan");
    init(&fleet).expect("init");
    let build_dir = scratch("select-plan-build");
    let artifact = fixture_with(&build_dir, "picked-rivet", "0.1.0", 84, &routed_needing_material());
    let key = build_dir.join("picked.pub");
    std::fs::write(&key, Ed25519Signer::from_seed(&[84; 32]).public_key().0).expect("a key");
    publish(&fleet, &artifact, &key).expect("published beforehand");
    let digest = read_manifest(&fleet).expect("manifest")[0].digest.clone();
    let selected = Invocation { selected: true, device: Some("dev-01".to_string()), at: None };

    let seed = seed_file("select-plan-seed", 84);
    let mut machine = Machine::watching(&fleet).with_document(SERVER_DOCUMENT);
    let report = publish_then_send(
        &fleet,
        deploy_args(Some(materialise_for_deploy(&fleet, &digest).expect("a local copy")), true),
        &selected,
        &mut || Ok(seed.clone()),
        &mut || panic!("no system seed"),
        &mut never_builds,
        &mut machine,
    )
    .expect("installed");
    assert!(report.contains("selected from the fleet repository"), "{report}");
    assert!(report.contains("cargo rivet run dev-01 -- apply service-restart rivet-proxyd confirm"), "{report}");
    assert!(report.contains("cargo rivet fleet sync dev-01 confirm"), "{report}");

    let mut seedless = Machine::watching(&fleet).with_document(SERVER_DOCUMENT);
    let report = publish_then_send(
        &fleet,
        deploy_args(Some(materialise_for_deploy(&fleet, &digest).expect("a local copy")), true),
        &selected,
        &mut || Err("no application signing seed was found".to_string()),
        &mut || panic!("no system seed"),
        &mut never_builds,
        &mut seedless,
    )
    .expect("a missing seed does not refuse an --artifact install");
    assert_eq!(seedless.installed.len(), 1);
    assert!(report.contains("COULD NOT BE PLANNED"), "{report}");
    assert!(report.contains("no application signing seed was found"), "{report}");
}

/// **The three names are ONE fact.** `cargo rivet secrets` spells them for the
/// five foreign programs that read them, and `Material::FleetTls` is what the
/// manifest and this refusal mean by `fleet-tls`; two lists that could drift
/// would make a refusal name files the install verb does not write.
#[test]
fn the_kinds_files_are_exactly_what_cargo_rivet_secrets_installs() {
    assert_eq!(
        Material::FleetTls.files(),
        [
            crate::secrets::CERTIFICATE,
            crate::secrets::KEY,
            crate::secrets::AUTHORITY
        ]
    );
}

// ---------------------------------------------------------------------------
// fleet objects backup / restore — ADR-052, R-183 D4
// ---------------------------------------------------------------------------

/// **Referenced-only scope backs up exactly what the manifest names, never
/// what is only sitting in `objects/`** — a stray file dropped into
/// `objects/` by hand (standing in for a superseded, pruned digest nothing
/// references any more) must not be copied without `--all`.
#[test]
fn objects_backup_default_scope_copies_only_referenced_digests() {
    let dir = scratch("objects-backup-referenced");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("objects-backup-referenced-fixture"), "reliquary-rivet", "0.1.1", 21);
    publish(&dir, &artifact, &key).expect("publish");
    let entries = read_manifest(&dir).expect("read the manifest back");
    let referenced_digest = entries[0].digest.clone();

    // A stray object nothing references — simulating a pruned/superseded
    // digest still sitting on disk (`ADR-052`'s own "harmless, never swept"
    // posture for exactly this shape of file).
    let stray_digest = "f".repeat(64);
    std::fs::write(dir.join(OBJECTS_DIR).join(&stray_digest), b"nobody references this").expect("write");

    let backup_dir = scratch("objects-backup-referenced-dest");
    let report = objects_backup(&dir, &backup_dir, false, false).expect("backs up");
    assert!(report.contains("1 object(s)"), "{report}");
    assert!(report.contains("referenced"), "{report}");

    let backed_up_objects = backup_dir.join(OBJECTS_DIR);
    assert!(backed_up_objects.join(&referenced_digest).is_file(), "the referenced digest must be backed up");
    assert!(
        !backed_up_objects.join(&stray_digest).exists(),
        "an unreferenced object must NOT be backed up without --all"
    );
}

/// **`--all` backs up every digest `objects/` holds**, referenced or not.
#[test]
fn objects_backup_all_copies_every_digest() {
    let dir = scratch("objects-backup-all");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("objects-backup-all-fixture"), "reliquary-rivet", "0.1.1", 22);
    publish(&dir, &artifact, &key).expect("publish");
    let entries = read_manifest(&dir).expect("read the manifest back");
    let referenced_digest = entries[0].digest.clone();

    let stray_digest = "e".repeat(64);
    std::fs::write(dir.join(OBJECTS_DIR).join(&stray_digest), b"a superseded build").expect("write");

    let backup_dir = scratch("objects-backup-all-dest");
    let report = objects_backup(&dir, &backup_dir, true, false).expect("backs up");
    assert!(report.contains("2 object(s)"), "{report}");
    assert!(report.contains("--all"), "{report}");

    let backed_up_objects = backup_dir.join(OBJECTS_DIR);
    assert!(backed_up_objects.join(&referenced_digest).is_file());
    assert!(backed_up_objects.join(&stray_digest).is_file(), "--all must copy the unreferenced object too");
}

/// **An empty repository backs up as 0 objects, not a refusal** — the
/// deliberate difference from the CA/seed backups' "empty source is
/// refused" posture, recorded in `objects_backup`'s own doc.
#[test]
fn objects_backup_of_a_freshly_initialised_repository_reports_zero_rather_than_refusing() {
    let dir = scratch("objects-backup-empty");
    init(&dir).expect("init");
    let backup_dir = scratch("objects-backup-empty-dest");

    let report = objects_backup(&dir, &backup_dir, false, false).expect("an empty scope is not a refusal");
    assert!(report.contains('0'), "{report}");

    let report = objects_backup(&dir, &backup_dir, true, false).expect("--all against empty is not a refusal");
    assert!(report.contains('0'), "{report}");
}

/// **Backing up onto an existing object at the destination refuses, unless
/// `--replace`.**
#[test]
fn objects_backup_refuses_to_overwrite_an_existing_object_without_replace() {
    let dir = scratch("objects-backup-overwrite");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("objects-backup-overwrite-fixture"), "reliquary-rivet", "0.1.1", 23);
    publish(&dir, &artifact, &key).expect("publish");
    let entries = read_manifest(&dir).expect("read the manifest back");
    let digest = entries[0].digest.clone();

    let backup_dir = scratch("objects-backup-overwrite-dest");
    std::fs::create_dir_all(backup_dir.join(OBJECTS_DIR)).expect("dest objects dir");
    std::fs::write(backup_dir.join(OBJECTS_DIR).join(&digest), b"stale, different bytes").expect("write");

    let err = objects_backup(&dir, &backup_dir, false, false).expect_err("must refuse an overwrite");
    assert!(err.contains("already holds"), "{err}");
    assert_eq!(
        std::fs::read(backup_dir.join(OBJECTS_DIR).join(&digest)).expect("unchanged"),
        b"stale, different bytes"
    );

    let report = objects_backup(&dir, &backup_dir, false, true).expect("--replace overrides");
    assert!(report.contains("1 object(s)"), "{report}");
    assert_eq!(
        std::fs::read(backup_dir.join(OBJECTS_DIR).join(&digest)).expect("now current"),
        std::fs::read(&artifact).expect("the real artifact bytes")
    );
}

/// **A manifest digest that traverses outside `objects/` refuses, and the
/// file it points at never reaches the backup.** A hand-edited or corrupted
/// manifest is exactly the "repository is inconsistent" case `objects_backup`
/// already reports for a digest `objects/` does not hold — this proves a
/// digest is confined to `objects/` on the way there too, not merely absent
/// or present within it.
#[test]
fn objects_backup_refuses_a_manifest_digest_that_reaches_outside_objects() {
    let dir = scratch("objects-backup-traversal");
    init(&dir).expect("init");

    // Sits beside `objects/`, never inside it — what a manifest digest of
    // `../outside-note.txt` would actually select if it were trusted.
    std::fs::write(dir.join("outside-note.txt"), b"not part of any backup").expect("write");

    let entry = Entry {
        digest: "../outside-note.txt".to_owned(),
        product: "reliquary-rivet".to_owned(),
        version: "0.1.1".to_owned(),
        arch: "aarch64".to_owned(),
        platform: None,
        kind: "application".to_owned(),
        size: 22,
        added: "2026-01-01T00:00:00Z".to_owned(),
        schema_version: None,
    };
    std::fs::write(dir.join(MANIFEST_FILE), render(&[entry])).expect("write manifest");

    let backup_dir = scratch("objects-backup-traversal-dest");
    let err = objects_backup(&dir, &backup_dir, false, false)
        .expect_err("a digest reaching outside objects/ must be refused");
    // Refused where the manifest is read, before any path is built from it.
    assert!(err.contains("not 64 lowercase hexadecimal"), "{err}");
    assert!(err.contains("outside-note.txt"), "{err}");
    assert!(
        !backup_dir.join(OBJECTS_DIR).exists(),
        "nothing should have been created at the backup destination"
    );
}

/// **The round trip**: back up, destroy the "live" object the way a reset
/// would, restore, and check the bytes are back and still verify.
#[test]
fn objects_backup_then_restore_round_trips() {
    let dir = scratch("objects-round-trip");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("objects-round-trip-fixture"), "reliquary-rivet", "0.1.1", 24);
    publish(&dir, &artifact, &key).expect("publish");
    let entries = read_manifest(&dir).expect("read the manifest back");
    let digest = entries[0].digest.clone();
    let original_bytes = std::fs::read(dir.join(OBJECTS_DIR).join(&digest)).expect("read stored bytes");

    let backup_dir = scratch("objects-round-trip-dest");
    objects_backup(&dir, &backup_dir, false, false).expect("backs up");

    // Destroy the "live" object.
    std::fs::remove_file(dir.join(OBJECTS_DIR).join(&digest)).expect("remove");
    assert!(!dir.join(OBJECTS_DIR).join(&digest).exists());

    let report = objects_restore(&dir, &backup_dir, false).expect("restores");
    assert!(report.contains("1 object(s)"), "{report}");
    assert!(report.contains("recomputed and verified"), "{report}");
    assert_eq!(std::fs::read(dir.join(OBJECTS_DIR).join(&digest)).expect("restored"), original_bytes);
}

/// **A backed-up object whose bytes were altered since the backup was made
/// is refused by its digest, and NOTHING is restored** — even a second,
/// genuinely intact object in the same backup directory is not written.
#[test]
fn objects_restore_refuses_a_digest_mismatch_and_writes_nothing() {
    let dir = scratch("objects-restore-mismatch");
    init(&dir).expect("init");
    let (artifact_a, key_a) = fixture(&scratch("objects-restore-mismatch-a"), "reliquary-rivet", "0.1.1", 25);
    publish(&dir, &artifact_a, &key_a).expect("publish a");
    let (artifact_b, key_b) = fixture(&scratch("objects-restore-mismatch-b"), "notes-rivet", "0.1.0", 26);
    publish(&dir, &artifact_b, &key_b).expect("publish b");
    let entries = read_manifest(&dir).expect("read the manifest back");
    let (digest_a, digest_b) = (entries[0].digest.clone(), entries[1].digest.clone());

    let backup_dir = scratch("objects-restore-mismatch-dest");
    objects_backup(&dir, &backup_dir, false, false).expect("backs up both");

    // Corrupt ONE of the two backed-up objects — bytes no longer match its
    // own filename's digest.
    std::fs::write(backup_dir.join(OBJECTS_DIR).join(&digest_a), b"corrupted after the backup was made")
        .expect("corrupt one backed-up object");

    // A fresh repository (never published into) to restore onto, so a
    // silent no-op could not be mistaken for a real restore.
    let fresh = scratch("objects-restore-mismatch-fresh");
    init(&fresh).expect("init a fresh repository");

    let err = objects_restore(&fresh, &backup_dir, false).expect_err("a digest mismatch must refuse");
    assert!(err.contains(&digest_a), "{err}");
    assert!(err.contains("do not match their own filename"), "{err}");
    assert!(
        !fresh.join(OBJECTS_DIR).join(&digest_a).exists() && !fresh.join(OBJECTS_DIR).join(&digest_b).exists(),
        "neither object may be written when the backup directory contains any mismatch"
    );
}

/// **Restoring onto a repository that already holds a conflicting object
/// refuses, unless `--replace`.**
#[test]
fn objects_restore_refuses_to_overwrite_without_replace() {
    let dir = scratch("objects-restore-overwrite");
    init(&dir).expect("init");
    let (artifact, key) = fixture(&scratch("objects-restore-overwrite-fixture"), "reliquary-rivet", "0.1.1", 27);
    publish(&dir, &artifact, &key).expect("publish");
    let entries = read_manifest(&dir).expect("read the manifest back");
    let digest = entries[0].digest.clone();

    let backup_dir = scratch("objects-restore-overwrite-dest");
    objects_backup(&dir, &backup_dir, false, false).expect("backs up");

    // The "live" object is still there (never removed) — restoring onto it
    // must refuse rather than silently accept, even though the bytes would
    // in fact agree (defence against a corrupted destination, per this
    // verb's own doc).
    let err = objects_restore(&dir, &backup_dir, false).expect_err("must refuse an overwrite");
    assert!(err.contains("already holds"), "{err}");

    let report = objects_restore(&dir, &backup_dir, true).expect("--replace overrides");
    assert!(report.contains("1 object(s)"), "{report}");
    let _ = digest;
}

/// **A backup directory holding nothing to restore is refused by name.**
#[test]
fn objects_restore_with_an_empty_backup_directory_is_refused() {
    let dir = scratch("objects-restore-empty");
    init(&dir).expect("init");
    let backup_dir = scratch("objects-restore-empty-source");
    std::fs::create_dir_all(backup_dir.join(OBJECTS_DIR)).expect("an empty objects/ dir");

    let err = objects_restore(&dir, &backup_dir, false).expect_err("nothing to restore");
    assert!(err.contains("no objects to restore"), "{err}");
}

/// **Neither direction works against a directory that was never `fleet
/// init`'d** — the same refusal `publish` already gives, restated for a verb
/// that never touches git.
#[test]
fn objects_backup_and_restore_both_refuse_an_uninitialised_directory() {
    let dir = scratch("objects-uninitialised");
    let backup_dir = scratch("objects-uninitialised-dest");

    let err = objects_backup(&dir, &backup_dir, false, false).expect_err("not a fleet repository");
    assert!(err.contains("is not a fleet repository"), "{err}");

    std::fs::create_dir_all(backup_dir.join(OBJECTS_DIR)).expect("an objects dir to restore from");
    let err = objects_restore(&dir, &backup_dir, false).expect_err("not a fleet repository");
    assert!(err.contains("is not a fleet repository"), "{err}");
}

/// **I01, second site.** An entry in the backup directory that cannot even be
/// stat'ed — a dangling symlink, the narrowest real trigger available — must
/// be reported, not silently read as "not a file" and skipped. Before this
/// was fixed, `entries.flatten()` dropped the directory-entry read failure
/// and `.filter(|p| p.is_file())` read the stat failure as "not a file", so a
/// restore could run to completion short one object and call it done, the
/// same shape `rivet_filecopy::flat_files` closed for the backup side.
#[cfg(unix)]
#[test]
fn objects_restore_reports_an_entry_it_cannot_stat_instead_of_skipping_it() {
    let dir = scratch("objects-restore-unreadable-entry");
    init(&dir).expect("init");

    let backup_dir = scratch("objects-restore-unreadable-entry-source");
    let backup_objects = backup_dir.join(OBJECTS_DIR);
    std::fs::create_dir_all(&backup_objects).expect("dest objects dir");
    std::fs::write(backup_objects.join("aaaa"), b"a real object").expect("a real file");
    std::os::unix::fs::symlink(backup_dir.join("nowhere-xyz"), backup_objects.join("dangling"))
        .expect("a dangling symlink");

    let err = objects_restore(&dir, &backup_dir, false).expect_err("an unreadable entry must refuse");
    assert!(err.contains("dangling"), "{err}");
    assert!(
        !dir.join(OBJECTS_DIR).join("aaaa").exists(),
        "nothing should be written when an entry could not be examined"
    );
}

// -- `fleet objects backup`/`restore` argument parsing ----------------------

#[test]
fn parse_plan_reads_the_host_and_at() {
    let args =
        vec!["plan".to_owned(), "rivet-01".to_owned(), "--at".to_owned(), "D:\\fleet".to_owned()];
    let parsed = parse(&args).expect("parses");
    assert_eq!(parsed.at, Some(PathBuf::from("D:\\fleet")));
    match parsed.verb {
        SubVerb::Plan { host, all } => {
            assert_eq!(host, "rivet-01");
            assert!(!all, "the default is the filtered view");
        }
        _ => panic!("expected Plan"),
    }
}

#[test]
fn parse_plan_reads_all() {
    let parsed =
        parse(&["plan".to_owned(), "rivet-01".to_owned(), "--all".to_owned()]).expect("parses");
    match parsed.verb {
        SubVerb::Plan { host, all } => {
            assert_eq!(host, "rivet-01");
            assert!(all);
        }
        _ => panic!("expected Plan"),
    }
}

#[test]
fn parse_plan_needs_a_host() {
    let err = parse(&["plan".to_owned()]).err().expect("no host");
    assert!(err.contains("needs a host"), "{err}");
}

#[test]
fn parse_all_is_refused_outside_plan() {
    let err = parse(&["list".to_owned(), "--all".to_owned()]).err().expect("--all not on list");
    assert!(err.contains("not an option"), "{err}");
}

#[test]
fn parse_objects_backup_reads_all_and_replace_and_at() {
    let args = vec![
        "objects".to_owned(),
        "backup".to_owned(),
        "D:\\dest".to_owned(),
        "--all".to_owned(),
        "--replace".to_owned(),
        "--at".to_owned(),
        "D:\\fleet".to_owned(),
    ];
    let parsed = parse(&args).expect("parses");
    assert_eq!(parsed.at, Some(PathBuf::from("D:\\fleet")));
    match parsed.verb {
        SubVerb::ObjectsBackup { dest, all, replace } => {
            assert_eq!(dest, PathBuf::from("D:\\dest"));
            assert!(all);
            assert!(replace);
        }
        _ => panic!("expected ObjectsBackup"),
    }
}

#[test]
fn parse_objects_restore_refuses_all() {
    let args = vec!["objects".to_owned(), "restore".to_owned(), "D:\\dest".to_owned(), "--all".to_owned()];
    let err = parse(&args).err().expect("--all is backup-only");
    assert!(err.contains("no --all"), "{err}");
}

#[test]
fn parse_objects_requires_a_directory() {
    let err = parse(&["objects".to_owned(), "backup".to_owned()]).err().expect("no directory");
    assert!(err.contains("needs a directory"), "{err}");
}

#[test]
fn parse_objects_refuses_an_unknown_action() {
    let err = parse(&["objects".to_owned(), "frobnicate".to_owned(), "D:\\x".to_owned()])
        .err()
        .expect("not an action");
    assert!(err.contains("is not an action"), "{err}");
}

#[test]
fn parse_objects_with_no_action_at_all_says_so() {
    let err = parse(&["objects".to_owned()]).err().expect("no action given");
    assert!(err.contains("needs an action"), "{err}");
}
