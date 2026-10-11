// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// A scratch directory this test owns outright, removed when it drops —
/// `provision_tests.rs`'s own `Scratch`, restated here since these tests are
/// a different module and this crate's binaries carry no shared test helper
/// crate.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(crate::scratch::dir(&format!(
            "provision-image-{label}-{n}"
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_entry(product: &str, version: &str, platform: Option<&str>, schema: Option<u32>) -> Entry {
    Entry {
        // A digest with no relation to `product`/`version`'s own text, so a
        // test asserting on the description's digest prefix cannot
        // accidentally also match the product name.
        digest: format!("{:0<64}", format!("deadbeef-{version}")),
        product: product.to_owned(),
        version: version.to_owned(),
        arch: "aarch64".to_owned(),
        platform: platform.map(ToOwned::to_owned),
        kind: "card".to_owned(),
        size: 12345,
        added: "2026-09-13T00:00:00Z".to_owned(),
        schema_version: schema,
    }
}

// ---------------------------------------------------------------------------
// Item 1 — the fleet-repository offer.
// ---------------------------------------------------------------------------

#[test]
fn wants_fleet_init_treats_a_bare_return_and_yes_as_yes() {
    assert!(wants_fleet_init(""));
    assert!(wants_fleet_init("\n"));
    assert!(wants_fleet_init("y"));
    assert!(wants_fleet_init("Y\n"));
    assert!(wants_fleet_init("yes"));
    assert!(wants_fleet_init("YES\n"));
}

#[test]
fn wants_fleet_init_treats_no_and_anything_else_as_no() {
    assert!(!wants_fleet_init("n"));
    assert!(!wants_fleet_init("no"));
    assert!(!wants_fleet_init("NO\n"));
    assert!(!wants_fleet_init("whatever"));
}

#[test]
fn ensure_fleet_repository_leaves_an_existing_repository_untouched() {
    let scratch = Scratch::new("offer-present");
    std::fs::create_dir_all(scratch.0.join(".git")).expect("fixture .git");
    ensure_fleet_repository(&scratch.0, true).expect("already there -- nothing to ask");
    // No manifest.toml was created by this call; the fixture's own .git is
    // untouched, which is the whole point -- an existing repository is
    // never re-initialised.
    assert!(!scratch.0.join("manifest.toml").is_file());
}

#[test]
fn ensure_fleet_repository_with_no_terminal_declines_silently_and_creates_nothing() {
    let scratch = Scratch::new("offer-no-terminal");
    let target = scratch.0.join("fleet");
    ensure_fleet_repository(&target, false).expect("no terminal -- nothing to ask, nothing refused");
    assert!(!target.exists(), "nothing must be created when there is no terminal to ask at");
}

// ---------------------------------------------------------------------------
// Items 2/3 — platform.
// ---------------------------------------------------------------------------

#[test]
fn distinct_platforms_is_sorted_and_deduplicated() {
    let images = vec![
        fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(1)),
        fixture_entry("rivet-system", "0.1.2", Some("rpi5"), Some(1)),
        fixture_entry("rivet-system", "0.1.0", Some("rpi4"), Some(1)),
    ];
    assert_eq!(distinct_platforms(&images), vec!["rpi4".to_owned(), "rpi5".to_owned()]);
}

#[test]
fn distinct_platforms_excludes_entries_with_no_platform() {
    let images = vec![fixture_entry("rivet-system", "0.1.1", None, Some(1))];
    assert!(distinct_platforms(&images).is_empty());
}

#[test]
fn decide_platform_refuses_verbatim_with_no_images_at_all() {
    let err = decide_platform(&[], None).expect_err("no images at all");
    assert_eq!(err, NO_IMAGES_REFUSAL);
}

#[test]
fn decide_platform_with_no_images_refuses_verbatim_even_with_a_requested_token() {
    let err = decide_platform(&[], Some("rpi4")).expect_err("no images at all");
    assert_eq!(err, NO_IMAGES_REFUSAL);
}

#[test]
fn decide_platform_uses_the_one_platform_present_without_asking() {
    let platforms = vec!["rpi4".to_owned()];
    let decision = decide_platform(&platforms, None).expect("one platform");
    assert_eq!(decision, PlatformDecision::Resolved("rpi4".to_owned()));
}

#[test]
fn decide_platform_asks_when_more_than_one_platform_is_present_and_none_was_requested() {
    let platforms = vec!["rpi4".to_owned(), "rpi5".to_owned()];
    let decision = decide_platform(&platforms, None).expect("more than one");
    assert_eq!(decision, PlatformDecision::Ask(platforms));
}

#[test]
fn decide_platform_uses_a_requested_token_that_has_images_without_asking() {
    let platforms = vec!["rpi4".to_owned(), "rpi5".to_owned()];
    let decision = decide_platform(&platforms, Some("rpi5")).expect("named and present");
    assert_eq!(decision, PlatformDecision::Resolved("rpi5".to_owned()));
}

#[test]
fn decide_platform_refuses_a_requested_token_with_no_images_naming_what_does_have_some() {
    let platforms = vec!["rpi4".to_owned()];
    let err = decide_platform(&platforms, Some("uefi")).expect_err("uefi has nothing published");
    assert!(err.contains("uefi"), "{err}");
    assert!(err.contains("rpi4"), "{err}");
}

// ---------------------------------------------------------------------------
// Item 4 — schema mismatch.
// ---------------------------------------------------------------------------

#[test]
fn schema_block_reason_is_none_when_the_schema_matches() {
    let entry = fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(1));
    assert_eq!(schema_block_reason(&entry, 1), None);
}

#[test]
fn schema_block_reason_names_both_versions_on_a_mismatch() {
    let entry = fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(2));
    let reason = schema_block_reason(&entry, 1).expect("mismatched");
    assert!(reason.contains('2'), "{reason}");
    assert!(reason.contains('1'), "{reason}");
}

#[test]
fn schema_block_reason_treats_a_missing_schema_version_as_blocked() {
    let entry = fixture_entry("rivet-system", "0.1.1", Some("rpi4"), None);
    assert!(schema_block_reason(&entry, 1).is_some());
}

#[test]
fn mark_schema_preserves_order_and_marks_each_entry_independently() {
    let images = vec![
        fixture_entry("rivet-system", "0.1.2", Some("rpi4"), Some(1)),
        fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(2)),
    ];
    let marked = mark_schema(images, 1);
    assert_eq!(marked[0].entry.version, "0.1.2");
    assert!(marked[0].blocked.is_none());
    assert_eq!(marked[1].entry.version, "0.1.1");
    assert!(marked[1].blocked.is_some());
}

#[test]
fn none_selectable_refusal_names_the_platform_and_every_blocked_reason() {
    let blocked = mark_schema(vec![fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(2))], 1);
    let msg = none_selectable_refusal("rpi4", &blocked, 1);
    assert!(msg.contains("rpi4"), "{msg}");
    assert!(msg.contains("0.1.1"), "{msg}");
    assert!(msg.contains("needs schema 2"), "{msg}");
    assert!(msg.contains("this tool speaks 1"), "{msg}");
}

#[test]
fn describe_entry_names_product_version_schema_size_and_a_short_digest() {
    let entry = fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(1));
    let described = describe_entry(&entry);
    assert!(described.contains("rivet-system"), "{described}");
    assert!(described.contains("0.1.1"), "{described}");
    assert!(described.contains("schema 1"), "{described}");
    assert!(described.contains("12345 bytes"), "{described}");
    assert_eq!(described.matches(&entry.digest[..12]).count(), 1, "{described}");
}

// ---------------------------------------------------------------------------
// Item 6 — the scratch working directory and materialise's key lookup.
// ---------------------------------------------------------------------------

#[test]
fn scratch_dir_is_under_provision_and_named_by_this_process() {
    let at = Path::new("/home/j/.rivet");
    let dir = scratch_dir(at);
    assert_eq!(dir.parent(), Some(at.join("provision").as_path()));
    assert!(dir.file_name().unwrap().to_string_lossy().contains(&std::process::id().to_string()));
}

#[test]
fn remove_scratch_keeps_a_directory_holding_a_file_this_run_did_not_write() {
    let scratch = Scratch::new("remove-scratch-foreign");
    let work_dir = scratch_dir(&scratch.0);
    std::fs::create_dir_all(&work_dir).expect("the fixture image-work directory");
    let image = work_dir.join("materialised.img");
    std::fs::write(&image, b"decompressed bytes").expect("the fixture working copy");
    let foreign = work_dir.join("not-ours.txt");
    std::fs::write(&foreign, b"someone else's").expect("the foreign file");

    remove_scratch(&image);

    assert!(!image.exists(), "the working copy itself is always removed");
    assert!(foreign.is_file(), "a file this run did not write must survive");
}

#[test]
fn remove_scratch_never_removes_a_directory_not_named_for_this_process() {
    let scratch = Scratch::new("remove-scratch-elsewhere");
    let other = scratch.0.join("provision").join("image-work-0");
    std::fs::create_dir_all(&other).expect("another run's directory");
    let image = other.join("materialised.img");
    std::fs::write(&image, b"decompressed bytes").expect("the fixture working copy");

    remove_scratch(&image);

    assert!(other.is_dir(), "only this process's own image-work directory may be removed");
}

/// A moment later than any live run's scratch could be young.
fn much_later() -> std::time::SystemTime {
    std::time::SystemTime::now() + STALE_SCRATCH_AFTER + std::time::Duration::from_secs(60)
}

/// **Scratch an earlier run left is removed by the next one**, once it is older
/// than any live run could be — the card image and the identity injected into
/// it go with the directory.
#[test]
fn a_stale_scratch_directory_from_another_run_is_swept() {
    let scratch = Scratch::new("sweep-stale");
    let other = scratch.0.join("provision").join("image-work-4242");
    std::fs::create_dir_all(&other).expect("an earlier run's directory");
    std::fs::write(other.join("materialised.img"), b"card image bytes").expect("its image");

    let removed = sweep_stale_scratch(&scratch.0, much_later());
    assert_eq!(removed.len(), 1, "{removed:?}");
    assert!(removed[0].contains("image-work-4242"), "{removed:?}");
    assert!(removed[0].contains("16 bytes"), "{removed:?}");
    assert!(!other.exists(), "the stale directory is gone");
}

/// **Young scratch, this run's own, and everything else in the directory are
/// left alone.** Only another run's `image-work-<digits>`, once stale, goes.
#[test]
fn the_sweep_leaves_young_own_and_foreign_entries_alone() {
    let scratch = Scratch::new("sweep-keep");
    let provision = scratch.0.join("provision");
    let young = provision.join("image-work-4243");
    let mine = scratch_dir(&scratch.0);
    let foreign = provision.join("image-work-notapid");
    let document = provision.join("example.provisioning.toml");
    for dir in [&young, &mine, &foreign] {
        std::fs::create_dir_all(dir).expect("a fixture directory");
    }
    std::fs::write(&document, b"version = 1\n").expect("a document");

    assert!(
        sweep_stale_scratch(&scratch.0, std::time::SystemTime::now()).is_empty(),
        "nothing is stale yet"
    );
    assert!(young.is_dir());

    let removed = sweep_stale_scratch(&scratch.0, much_later());
    assert_eq!(removed.len(), 1, "{removed:?}");
    assert!(!young.exists(), "another run's stale scratch goes");
    assert!(mine.is_dir(), "this run's own scratch is never swept");
    assert!(foreign.is_dir(), "a name that is not image-work-<digits> is not this sweep's");
    assert!(document.is_file(), "a document beside the scratch is untouched");
}

/// No `provision` directory at all is the ordinary first run, not a fault.
#[test]
fn the_sweep_of_a_missing_directory_removes_nothing() {
    let scratch = Scratch::new("sweep-missing");
    assert!(sweep_stale_scratch(&scratch.0, much_later()).is_empty());
}

#[test]
fn materialise_refuses_by_name_when_no_public_key_is_at_the_established_location() {
    let scratch = Scratch::new("materialise-no-key");
    let entry = fixture_entry("rivet-system", "0.1.1", Some("rpi4"), Some(1));
    let err = materialise(&scratch.0, &entry, &scratch.0, &scratch.0.join("work"))
        .expect_err("no keys/rivet-system.pub exists in this scratch tree");
    assert!(err.contains("rivet-system.pub"), "{err}");
}
