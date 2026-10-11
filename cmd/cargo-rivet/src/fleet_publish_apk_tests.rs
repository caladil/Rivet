//! `fleet publish FILE.apk`, end to end against a real fleet repository —
//! `ADR-061` §4.
//!
//! The APKs are hand-composed by `crate::apk_read::tests`' builders (a real
//! zip, real binary XML, a signing block with made-up certificate bytes);
//! the repository, the `.rvt`, its BLAKE3 and its Ed25519 signature are real.
//! The seed is minted here, in a scratch directory.
// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::atomic::{AtomicU32, Ordering};

use super::super::{OBJECTS_DIR, read_manifest};
use super::*;
use crate::apk_read::tests::{V3, V3Signer, hand_apk, hand_apk_v2, signing_block, v3_block};

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!("fleet-apk-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

/// A fresh fleet repository, and an application seed beside it (never inside
/// it — `ADR-052` §4 refuses a repository holding one).
struct Repo {
    root: PathBuf,
    work: PathBuf,
    seed: PathBuf,
}

const SEED: [u8; 32] = [0x5a; 32];

fn repo(label: &str) -> Repo {
    let work = scratch(label);
    let root = work.join("fleet");
    super::super::init(&root).expect("a fleet repository");
    let seed = work.join("app.seed");
    std::fs::write(&seed, SEED).expect("a seed");
    Repo { root, work, seed }
}

impl Repo {
    fn apk(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.work.join(name);
        std::fs::write(&path, bytes).expect("an apk file");
        path
    }

    fn publish(&self, file: &Path, product: Option<&str>) -> Result<String, String> {
        let seed = self.seed.clone();
        publish(&self.root, file, product, &mut || Ok(seed.clone()))
    }
}

fn v3_signed(package: &str, version_code: u32, version_name: &str, certificate: &[u8], lineage: &[&[u8]]) -> Vec<u8> {
    let block = signing_block(&[(
        V3,
        v3_block(&[V3Signer {
            min_sdk: 28,
            max_sdk: 0x7fff_ffff,
            certificate,
            lineage: (!lineage.is_empty()).then(|| lineage.to_vec()),
        }]),
    )]);
    hand_apk(package, version_code, version_name, &block)
}

fn sha(bytes: &[u8]) -> String {
    crate::apk_read::sha256_hex(bytes)
}

// ---------------------------------------------------------------------------
// The first publish pins; the artifact is what ADR-061 §3 says.
// ---------------------------------------------------------------------------

#[test]
fn a_first_publish_pins_the_signer_and_says_so() {
    let repo = repo("first");
    let file = repo.apk("app.apk", &hand_apk_v2("com.example.gantry", 17, "0.1.17", b"key one"));

    let report = repo.publish(&file, None).expect("a first publish is accepted");
    assert!(
        report.contains(&format!("gantry is now pinned to signer {}", sha(b"key one"))),
        "{report}"
    );
    assert!(report.contains("MADE"), "{report}");

    let entries = read_manifest(&repo.root).expect("the manifest");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(
        (entry.product.as_str(), entry.version.as_str(), entry.kind.as_str(), entry.arch.as_str()),
        ("gantry", "0.1.17", "apk", "any")
    );
    assert_eq!(entry.platform.as_deref(), Some("android"));
}

/// **The `.rvt` verifies, opens with `rivet_core`'s apk module, carries an
/// `apk.json` that module accepts, and its `app.apk` is the input byte for
/// byte.**
#[test]
fn the_published_artifact_verifies_opens_as_an_apk_and_carries_the_apk_byte_for_byte() {
    let repo = repo("artifact");
    let bytes = hand_apk_v2("com.example.gantry.dev", 5, "0.1.5", b"key one");
    let file = repo.apk("app.apk", &bytes);
    repo.publish(&file, None).expect("published");

    let entry = read_manifest(&repo.root).expect("the manifest").remove(0);
    let object = repo.root.join(OBJECTS_DIR).join(&entry.digest);
    let public = Ed25519Signer::from_seed(&SEED).public_key();
    let inspected = crate::inspect::inspect(&object, public, &[], &mut Blake3::new(), &Ed25519Verifier::new())
        .expect("the stored object verifies against the application key");

    let crate::inspect::Details::Apk { metadata, icon, .. } = &inspected.details else {
        panic!("not read as an apk");
    };
    assert_eq!(metadata.application_id(), "com.example.gantry.dev");
    assert_eq!(metadata.version_code(), 5);
    assert_eq!(metadata.version_name(), "0.1.5");
    assert_eq!(metadata.label(), "Hand Composed");
    assert_eq!((metadata.min_sdk(), metadata.target_sdk()), (26, 34));
    assert_eq!(metadata.signer_sha256(), sha(b"key one"));
    assert!(metadata.development());
    assert!(icon.is_none());

    // `rivet_core::package::apk::Apk::open` is what inspect ran; run it
    // again directly, and take app.apk's bytes out at the offset it names.
    let apk = rivet_core::package::apk::Apk::open(&inspected.package).expect("opens as an apk");
    let member = apk.apk_member();
    let stored = std::fs::read(&object).expect("the object");
    let start = usize::try_from(member.offset()).unwrap();
    let len = usize::try_from(member.stored_len()).unwrap();
    assert_eq!(&stored[start..start + len], bytes.as_slice(), "app.apk is not the input");
    assert_eq!(entry.product, "gantry-dev");
}

// ---------------------------------------------------------------------------
// The version code.
// ---------------------------------------------------------------------------

#[test]
fn a_publish_whose_version_code_is_not_higher_is_refused_naming_both() {
    let repo = repo("version");
    repo.publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 17, "0.1.17", b"k")), None)
        .expect("first");

    for (code, name) in [(17, "0.1.17-rebuilt"), (16, "0.1.16")] {
        let err = repo
            .publish(&repo.apk("b.apk", &hand_apk_v2("com.example.gantry", code, name, b"k")), None)
            .expect_err("not higher is refused");
        assert!(err.contains(&format!("version code {code} ({name})")), "{err}");
        assert!(err.contains("newest in this repository is 17 (0.1.17)"), "{err}");
    }
    assert_eq!(read_manifest(&repo.root).expect("manifest").len(), 1, "nothing was published");

    repo.publish(&repo.apk("c.apk", &hand_apk_v2("com.example.gantry", 18, "0.1.18", b"k")), None)
        .expect("a higher version code is admitted");
}

/// **Publishing the newest held APK's exact bytes again is the idempotent add,
/// not a refusal**, as it is for every other kind (`ADR-052` §7). A `--sync`
/// that failed after a publish has to be retryable without rebuilding the app,
/// and the retry is this same command again. Nothing new is made.
#[test]
fn the_same_apk_published_again_is_already_held_and_makes_nothing() {
    let repo = repo("same-again");
    let bytes = hand_apk_v2("com.example.gantry", 17, "0.1.17", b"k");
    repo.publish(&repo.apk("a.apk", &bytes), None).expect("first");
    let again = repo.publish(&repo.apk("again.apk", &bytes), None).expect("the same bytes again");
    assert!(again.contains("already holds"), "{again}");
    assert_eq!(read_manifest(&repo.root).expect("manifest").len(), 1, "nothing new was published");
}

// ---------------------------------------------------------------------------
// The version name — `ADR-061` §4: must be one the repository can order,
// and must order the same way as versionCode.
// ---------------------------------------------------------------------------

/// A `versionName` this repository's version ordering cannot read (not
/// dot-separated numbers) is refused, naming the rule and the value — even on
/// a first publish, before there is anything to compare against.
#[test]
fn a_version_name_the_repository_cannot_order_is_refused_naming_the_rule_and_the_value() {
    let repo = repo("unorderable-name");
    let err = repo
        .publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 1, "one.oh", b"k")), None)
        .expect_err("an unorderable versionName is refused");
    assert!(err.contains("one.oh"), "{err}");
    assert!(err.contains("version ordering cannot read"), "{err}");
    assert_eq!(read_manifest(&repo.root).expect("manifest").len(), 0, "nothing was published");
}

/// A higher `versionCode` whose `versionName` does not order strictly above
/// the product's newest is refused, naming both — the two must agree, because
/// retention orders by `versionName` while Android compares `versionCode`.
#[test]
fn a_higher_version_code_whose_version_name_does_not_order_above_the_newest_is_refused_naming_both() {
    let repo = repo("disagreeing-order");
    repo.publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 1, "0.2.0", b"k")), None)
        .expect("first");

    for name in ["0.2.0", "0.1.0"] {
        let err = repo
            .publish(&repo.apk("b.apk", &hand_apk_v2("com.example.gantry", 2, name, b"k")), None)
            .expect_err("a versionName not strictly above the newest is refused");
        assert!(err.contains(name), "{err}");
        assert!(err.contains("0.2.0"), "{err}");
    }
    assert_eq!(read_manifest(&repo.root).expect("manifest").len(), 1, "nothing was published");

    repo.publish(&repo.apk("c.apk", &hand_apk_v2("com.example.gantry", 2, "0.3.0", b"k")), None)
        .expect("a versionName strictly above the newest, with a higher versionCode, is admitted");
}

// ---------------------------------------------------------------------------
// The signer.
// ---------------------------------------------------------------------------

#[test]
fn a_publish_signed_by_another_key_is_refused_naming_both_fingerprints() {
    let repo = repo("other-key");
    repo.publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 1, "0.1.1", b"pinned")), None)
        .expect("first");
    let err = repo
        .publish(&repo.apk("b.apk", &hand_apk_v2("com.example.gantry", 2, "0.1.2", b"stranger")), None)
        .expect_err("another key is refused");
    assert!(err.contains(&sha(b"stranger")), "{err}");
    assert!(err.contains(&sha(b"pinned")), "{err}");
    assert!(err.contains("no rotation lineage"), "{err}");
    assert_eq!(read_manifest(&repo.root).expect("manifest").len(), 1);
}

/// **A rotated key whose lineage links to the pinned one is admitted**, and
/// the product is then pinned to the new key: a later APK signed by the new
/// key alone, with no lineage, matches.
#[test]
fn a_rotated_key_whose_lineage_links_to_the_pinned_one_is_admitted() {
    let repo = repo("rotated");
    repo.publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 1, "0.1.1", b"old")), None)
        .expect("first");
    let report = repo
        .publish(&repo.apk("b.apk", &v3_signed("com.example.gantry", 2, "0.1.2", b"new", &[b"old", b"new"])), None)
        .expect("the rotation is admitted");
    assert!(
        report.contains(&format!("signer {} is linked to gantry's pinned signer {}", sha(b"new"), sha(b"old"))),
        "{report}"
    );
    repo.publish(&repo.apk("c.apk", &hand_apk_v2("com.example.gantry", 3, "0.1.3", b"new")), None)
        .expect("the new key is now the pinned one");
}

#[test]
fn a_lineage_that_does_not_include_the_pinned_key_is_refused_naming_both() {
    let repo = repo("bad-lineage");
    repo.publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 1, "0.1.1", b"old")), None)
        .expect("first");
    let err = repo
        .publish(&repo.apk("b.apk", &v3_signed("com.example.gantry", 2, "0.1.2", b"new", &[b"other", b"new"])), None)
        .expect_err("refused");
    assert!(err.contains(&sha(b"new")) && err.contains(&sha(b"old")), "{err}");
    assert!(err.contains("lineage does not include the pinned certificate"), "{err}");
}

/// A stored object the manifest names and `objects/` lacks cannot be compared
/// against, so the publish is refused rather than treated as a first one.
#[test]
fn a_missing_stored_object_refuses_rather_than_pins_again() {
    let repo = repo("missing-object");
    repo.publish(&repo.apk("a.apk", &hand_apk_v2("com.example.gantry", 1, "0.1.1", b"k")), None)
        .expect("first");
    let entry = read_manifest(&repo.root).expect("manifest").remove(0);
    std::fs::remove_file(repo.root.join(OBJECTS_DIR).join(&entry.digest)).expect("removed");
    let err = repo
        .publish(&repo.apk("b.apk", &hand_apk_v2("com.example.gantry", 2, "0.1.2", b"k")), None)
        .expect_err("refused");
    assert!(err.contains("objects/ does not hold it"), "{err}");
}

// ---------------------------------------------------------------------------
// The product name.
// ---------------------------------------------------------------------------

#[test]
fn the_product_is_the_last_segment_with_dev_as_a_suffix() {
    assert_eq!(derive_product("io.github.example.gantry.dev"), "gantry-dev");
    assert_eq!(derive_product("io.github.example.gantry"), "gantry");
    assert_eq!(derive_product("com.example.logbook"), "logbook");
    assert_eq!(derive_product("a.dev"), "a-dev");
    assert_eq!(derive_product("single"), "single");
}

#[test]
fn a_product_name_outside_the_grammar_is_refused() {
    for good in ["gantry", "gantry-dev", "hc-writer", "a1", &"a".repeat(32)] {
        check_product(good).unwrap_or_else(|why| panic!("{good:?} {why}"));
    }
    // `app-1.2.3` would be read back as product `app`, version `1.2.3-…`.
    for bad in ["", "MyApp", "my_app", "-gantry", "gantry-", "gan--try", &"a".repeat(33), "app-1.2.3"] {
        assert!(check_product(bad).is_err(), "{bad:?} was accepted");
    }
}

#[test]
fn a_derived_product_outside_the_grammar_is_refused_naming_product_and_product_overrides_it() {
    let repo = repo("product");
    let file = repo.apk("a.apk", &hand_apk_v2("com.example.MyApp", 1, "0.1.1", b"k"));
    let err = repo.publish(&file, None).expect_err("refused");
    assert!(err.contains("\"MyApp\"") && err.contains("--product NAME"), "{err}");

    let err = repo.publish(&file, Some("Also_Bad")).expect_err("refused");
    assert!(err.starts_with("--product \"Also_Bad\""), "{err}");

    let report = repo.publish(&file, Some("myapp")).expect("--product names it");
    assert!(report.contains("myapp (from --product)"), "{report}");
    assert_eq!(read_manifest(&repo.root).expect("manifest")[0].product, "myapp");
}

// ---------------------------------------------------------------------------
// apk.json.
// ---------------------------------------------------------------------------

/// **The `apk.json` written is accepted by `rivet_core`'s validator**, awkward
/// characters included, and reads back to what the APK said.
#[test]
fn apk_json_reads_back_through_rivet_core_with_awkward_characters() {
    let info = ApkInfo {
        application_id: "com.example.tool.dev".to_string(),
        version_code: (1i64 << 32) | 7,
        version_code_major: Some(1),
        version_name: "1.0 \"beta\"".to_string(),
        label: "Back\\slash\tand \u{e9}".to_string(),
        min_sdk: 24,
        target_sdk: 35,
        scheme: crate::apk_read::Scheme::V2,
        certificate: b"cert".to_vec(),
        lineage: Vec::new(),
    };
    let metadata = Metadata::parse(apk_json(&info).as_bytes()).expect("rivet_core accepts it");
    assert_eq!(metadata.application_id(), info.application_id);
    assert_eq!(metadata.version_code(), info.version_code);
    assert_eq!(metadata.version_name(), info.version_name);
    assert_eq!(metadata.label(), info.label);
    assert_eq!((metadata.min_sdk(), metadata.target_sdk()), (24, 35));
    assert_eq!(metadata.signer_sha256(), sha(b"cert"));
    assert!(metadata.development());
}

// ---------------------------------------------------------------------------
// What the file is, and which key.
// ---------------------------------------------------------------------------

#[test]
fn a_file_named_apk_that_is_not_a_zip_is_refused() {
    let dir = scratch("classify-a");
    let file = dir.join("app.apk");
    std::fs::write(&file, b"not a zip").expect("write");
    let err = classify(&file).expect_err("refused");
    assert!(err.contains("is named .apk and is not one"), "{err}");
}

#[test]
fn a_zip_not_named_apk_is_refused() {
    let dir = scratch("classify-b");
    let file = dir.join("app.rvt");
    std::fs::write(&file, hand_apk_v2("a.b.c", 1, "1.0.0", b"k")).expect("write");
    let err = classify(&file).expect_err("refused");
    assert!(err.contains("is a zip"), "{err}");
}

#[test]
fn an_apk_is_known_by_its_name_in_any_case_and_its_magic() {
    let dir = scratch("classify-c");
    let file = dir.join("APP.APK");
    std::fs::write(&file, hand_apk_v2("a.b.c", 1, "1.0.0", b"k")).expect("write");
    assert_eq!(classify(&file), Ok(Handed::Apk));
    let other = dir.join("thing.rvt");
    std::fs::write(&other, b"RVT\0 whatever").expect("write");
    assert_eq!(classify(&other), Ok(Handed::Artifact));
}

/// A file that does not read as an APK is refused before any key is asked for.
#[test]
fn an_unreadable_apk_is_refused_before_the_seed_is_looked_for() {
    let repo = repo("unreadable");
    let file = repo.apk("app.apk", b"PK\x03\x04 and then nothing an APK has");
    let err = publish(&repo.root, &file, None, &mut || panic!("the seed was asked for"))
        .expect_err("refused");
    assert!(err.contains("cannot be read as an APK"), "{err}");
}

#[test]
fn the_seed_is_the_named_one_then_the_environment_then_the_default() {
    let dir = scratch("seed");
    let named = dir.join("named.seed");
    let from_env = dir.join("env.seed");
    let over = crate::credentials::Overrides { app_seed: Some(from_env.clone()), ..Default::default() };
    assert_eq!(app_seed(Some(&named), &over, Some(&dir)).expect("named"), named);
    assert_eq!(app_seed(None, &over, Some(&dir)).expect("env"), from_env);

    let none = crate::credentials::Overrides::default();
    let err = app_seed(None, &none, Some(&dir)).expect_err("none");
    assert!(err.contains("--signing-key") && err.contains("RIVET_APP_SEED"), "{err}");
    std::fs::create_dir_all(dir.join("keys")).expect("keys");
    std::fs::write(dir.join("keys").join("rivet-app.seed"), SEED).expect("seed");
    assert_eq!(app_seed(None, &none, Some(&dir)).expect("default"), dir.join("keys").join("rivet-app.seed"));
}

// ---------------------------------------------------------------------------
// The command line.
// ---------------------------------------------------------------------------

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_string()).collect()
}

#[test]
fn the_command_line_publishes_an_apk_with_a_named_seed_and_product() {
    let repo = repo("cli");
    let file = repo.apk("app.apk", &hand_apk_v2("com.example.gantry", 3, "0.1.3", b"k"));
    let args = super::super::parse(&words(&[
        "publish",
        file.to_str().unwrap(),
        "--product",
        "gantry-test",
        "--signing-key",
        repo.seed.to_str().unwrap(),
        "--at",
        repo.root.to_str().unwrap(),
    ]))
    .expect("parses");
    let report = super::super::run(&args).expect("publishes");
    assert!(report.contains("gantry-test is now pinned to signer"), "{report}");
}

#[test]
fn each_kind_refuses_the_other_kinds_flags() {
    let repo = repo("cli-flags");
    let file = repo.apk("app.apk", &hand_apk_v2("com.example.gantry", 3, "0.1.3", b"k"));
    let args = super::super::parse(&words(&[
        "publish",
        file.to_str().unwrap(),
        "--trusted-key",
        "some.pub",
        "--at",
        repo.root.to_str().unwrap(),
    ]))
    .expect("parses");
    let err = super::super::run(&args).expect_err("refused");
    assert!(err.starts_with("--trusted-key is an .rvt's"), "{err}");

    let rvt = repo.work.join("thing.rvt");
    std::fs::write(&rvt, b"RVT").expect("write");
    let args = super::super::parse(&words(&[
        "publish",
        rvt.to_str().unwrap(),
        "--product",
        "x",
        "--at",
        repo.root.to_str().unwrap(),
    ]))
    .expect("parses");
    let err = super::super::run(&args).expect_err("refused");
    assert!(err.starts_with("--product and --signing-key are an .apk's"), "{err}");
}

#[test]
fn product_and_signing_key_are_publish_flags_alone() {
    let err = super::super::parse(&words(&["list", "--product", "x"])).err().expect("refused");
    assert!(err.contains("\"--product\" is not an option of `fleet list`"), "{err}");
}
