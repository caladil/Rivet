//! Tests for `cargo rivet image` — `R-177`.
//!
//! **A real GPT+FAT image, real zstd, real BLAKE3 and real Ed25519** — this is
//! the one verb in the crate whose whole job is bytes moving through a real
//! codec, so a toy compressor would prove nothing (`AGENTS.md` §4). The image
//! itself is a few kilobytes, not 2 GiB — the end-to-end proof against a real
//! container-built card is `docs/PLAN.md`'s job, not a unit test's.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs::{File, OpenOptions};

use super::*;
use crate::gpt;
use crate::write_image::{BOOT_LABELS, DOCUMENT_NAME, Partition};

pub(crate) fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!(
        "image-test-{tag}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A small GPT image with a real `boot-a` FAT partition, optionally carrying
/// `rivet.toml` declaring `schema_version` — built the way
/// `write_image_tests.rs`'s `image_with_peers` builds one, for the identical
/// reason: a reader can see which bytes the check is being asked to believe.
///
/// `tail` is appended after the boot partition, standing in for the `/system`
/// and other partitions a real card has — its only job is to give the whole
/// file recognisable content so a round trip can be checked byte for byte.
pub(crate) fn fixture_image(tag: &str, schema_version: Option<i64>, tail: &[u8]) -> PathBuf {
    const BOOT_SECTORS: u64 = 8192; // 4 MiB — the smallest fatfs will format.
    let path = scratch(tag).join("fixture-card.img");

    let first = 2048u64;
    let last = first + BOOT_SECTORS - 1;
    {
        let mut file = File::create(&path).expect("a temp image");
        let mut mbr = vec![0u8; gpt::SECTOR_BYTES as usize];
        mbr[450] = 0xEE;
        file.write_all(&mbr).expect("mbr");

        let mut header = vec![0u8; gpt::SECTOR_BYTES as usize];
        header[0..8].copy_from_slice(b"EFI PART");
        header[72..80].copy_from_slice(&2u64.to_le_bytes());
        header[80..84].copy_from_slice(&1u32.to_le_bytes());
        header[84..88].copy_from_slice(&128u32.to_le_bytes());
        file.write_all(&header).expect("header");

        let mut entry = vec![0u8; 128];
        entry[0] = 0x01;
        entry[32..40].copy_from_slice(&first.to_le_bytes());
        entry[40..48].copy_from_slice(&last.to_le_bytes());
        for (i, unit) in BOOT_LABELS[0].encode_utf16().enumerate() {
            let byte = 56 + i * 2;
            entry[byte..byte + 2].copy_from_slice(&unit.to_le_bytes());
        }
        file.write_all(&entry).expect("entry");
        file.set_len((last + 1) * gpt::SECTOR_BYTES).expect("size");
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    {
        let mut view = Partition::new(
            &mut file,
            first * gpt::SECTOR_BYTES,
            BOOT_SECTORS * gpt::SECTOR_BYTES,
        );
        fatfs::format_volume(&mut view, fatfs::FormatVolumeOptions::new()).expect("format");
    }
    if let Some(version) = schema_version {
        let mut view = Partition::new(
            &mut file,
            first * gpt::SECTOR_BYTES,
            BOOT_SECTORS * gpt::SECTOR_BYTES,
        );
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
        let mut out = fs.root_dir().create_file(DOCUMENT_NAME).expect("create");
        out.write_all(format!("version = {version}\n").as_bytes())
            .expect("write");
    }
    drop(file);

    if !tail.is_empty() {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("reopen to append");
        file.write_all(tail).expect("append the tail");
    }
    path
}

pub(crate) fn write_seed(dir: &Path, name: &str, byte: u8) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, [byte; 32]).expect("write a seed");
    path
}

// ---------------------------------------------------------------------------
// `schema_version_of` — item 3
// ---------------------------------------------------------------------------

#[test]
fn schema_version_is_read_from_the_images_own_document() {
    let image = fixture_image("schema-ok", Some(1), b"");
    assert_eq!(schema_version_of(&image).expect("readable"), 1);
}

#[test]
fn an_image_with_no_document_at_all_is_refused_naming_why() {
    let image = fixture_image("schema-none", None, b"");
    let refused = schema_version_of(&image).expect_err("no document, no schema version");
    assert!(refused.contains("no rivet.toml"), "{refused}");
}

#[test]
fn a_document_with_no_version_key_is_refused() {
    let image_dir = scratch("schema-malformed");
    let image = image_dir.join("fixture-card.img");
    std::fs::rename(fixture_image("schema-malformed-src", Some(1), b""), &image).expect("move");
    // Overwrite with a document that parses as TOML but has no `version`.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&image)
        .expect("open");
    let first = 2048u64;
    const BOOT_SECTORS: u64 = 8192;
    {
        let mut view = Partition::new(
            &mut file,
            first * gpt::SECTOR_BYTES,
            BOOT_SECTORS * gpt::SECTOR_BYTES,
        );
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
        fs.root_dir().remove(DOCUMENT_NAME).expect("remove");
        let mut out = fs.root_dir().create_file(DOCUMENT_NAME).expect("create");
        out.write_all(b"hostname = \"nothing-to-do-with-a-version\"\n")
            .expect("write");
    }
    drop(file);

    let refused = schema_version_of(&image).expect_err("no version key");
    assert!(refused.contains("no integer `version`"), "{refused}");
}

// ---------------------------------------------------------------------------
// End to end, against a small fixture image and a scratch fleet repository.
// ---------------------------------------------------------------------------

/// **Wrap, compress, sign, publish — and the bytes come back identical.**
///
/// This is the round trip Wave B's proof asks for, at unit-test scale: a real
/// zstd encoder compresses a real file, `cargo rivet image` seals and signs
/// it with a scratch system seed, `fleet::publish_verified` stores it, and
/// `zstd::decode_all` against the stored object reproduces the original
/// bytes exactly.
#[test]
fn wrap_compress_sign_and_publish_round_trips_the_original_bytes() {
    let work = scratch("e2e");
    // A few KB of compressible content plus some incompressible-looking
    // bytes, so this is not merely proving zstd handles an all-zero buffer.
    let mut tail = vec![0u8; 4096];
    for (i, byte) in tail.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    let image = fixture_image("e2e-src", Some(1), &tail);
    let original = std::fs::read(&image).expect("read the fixture whole");

    let seed = write_seed(&work, "rivet-system.seed", 0x42);
    let fleet_root = work.join("fleet");
    let out_dir = work.join("out");

    let report = run(&[
        "--img".into(),
        image.display().to_string(),
        "--version".into(),
        "0.1.87".into(),
        "--arch".into(),
        "aarch64".into(),
        "--signing-key".into(),
        seed.display().to_string(),
        "--out-dir".into(),
        out_dir.display().to_string(),
        "--at".into(),
        fleet_root.display().to_string(),
    ])
    .expect("a valid fixture image wraps, compresses, signs and publishes");

    assert!(report.contains("kind     card"), "{report}");
    assert!(report.contains("schema   1"), "{report}");
    assert!(report.contains("platform rpi4"), "{report}");
    assert!(report.contains("MADE"), "{report}");

    let rvt = out_dir.join("rivet-0.1.87-aarch64-rpi4-card.rvt");
    assert!(
        rvt.is_file(),
        "the signed .rvt is left beside the image: {report}"
    );

    let entries = crate::fleet::read_manifest(&fleet_root).expect("manifest");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "card");
    assert_eq!(entries[0].platform.as_deref(), Some("rpi4"));
    assert_eq!(entries[0].schema_version, Some(1));

    let object = fleet_root.join("objects").join(&entries[0].digest);
    let stored = std::fs::read(&object).expect("the object is stored");

    // The header is 4096 bytes (`rivet_core::package::format::HEADER_BYTES`);
    // everything after it is the compressed member, verbatim.
    let compressed = &stored[4096..];
    let decompressed = zstd::decode_all(compressed).expect("zstd decodes what zstd wrote");
    assert_eq!(
        decompressed, original,
        "the decompressed bytes must equal the original image"
    );
}

/// **Refusing an image whose boot partition carries no schema version at
/// all**, before anything is compressed or signed.
#[test]
fn an_image_with_no_document_refuses_before_compressing_anything() {
    let work = scratch("e2e-no-doc");
    let image = fixture_image("e2e-no-doc-src", None, b"");
    let seed = write_seed(&work, "rivet-system.seed", 0x11);

    let refused = run(&[
        "--img".into(),
        image.display().to_string(),
        "--version".into(),
        "0.1.0".into(),
        "--arch".into(),
        "aarch64".into(),
        "--signing-key".into(),
        seed.display().to_string(),
        "--out-dir".into(),
        work.join("out").display().to_string(),
        "--at".into(),
        work.join("fleet").display().to_string(),
    ])
    .expect_err("no document, no schema version, no publish");
    assert!(refused.contains("no rivet.toml"), "{refused}");
    assert!(
        !work
            .join("out")
            .join("rivet-0.1.0-aarch64-rpi4-card.rvt")
            .exists()
    );
}

/// **A platform this build has no `BootSlots` for is refused, naming what
/// does exist** — the same posture `cargo rivet update` already takes.
#[test]
fn an_unimplemented_platform_is_refused_and_names_what_exists() {
    let work = scratch("bad-platform");
    let image = fixture_image("bad-platform-src", Some(1), b"");
    let seed = write_seed(&work, "rivet-system.seed", 0x22);

    let refused = run(&[
        "--img".into(),
        image.display().to_string(),
        "--version".into(),
        "0.1.0".into(),
        "--arch".into(),
        "aarch64".into(),
        "--platform".into(),
        "uefi".into(),
        "--signing-key".into(),
        seed.display().to_string(),
        "--out-dir".into(),
        work.join("out").display().to_string(),
        "--at".into(),
        work.join("fleet").display().to_string(),
    ])
    .expect_err("uefi has no BootSlots implementor yet");
    assert!(refused.contains("rpi4"), "{refused}");
    assert!(refused.contains("uefi"), "{refused}");
}

/// Every required flag is refused rather than defaulted, for the same reason
/// `cargo rivet update` refuses them: this tool did not build the bytes.
#[test]
fn version_and_arch_are_required() {
    let work = scratch("required-flags");
    let image = fixture_image("required-flags-src", Some(1), b"");
    let seed = write_seed(&work, "rivet-system.seed", 0x33);
    let base = |extra: &[(&str, &str)]| -> Vec<String> {
        let mut out = vec![
            "--img".to_string(),
            image.display().to_string(),
            "--signing-key".to_string(),
            seed.display().to_string(),
        ];
        for (flag, value) in extra {
            out.push((*flag).to_string());
            out.push((*value).to_string());
        }
        out
    };

    let no_version = run(&base(&[("--arch", "aarch64")])).expect_err("must refuse");
    assert!(no_version.contains("--version is required"), "{no_version}");

    let no_arch = run(&base(&[("--version", "0.1.0")])).expect_err("must refuse");
    assert!(no_arch.contains("--arch is required"), "{no_arch}");
}

// ---------------------------------------------------------------------------
// The read side — item 9. `list_images`, `resolve_image`, `materialise_image`.
// ---------------------------------------------------------------------------

/// **`list_images` reports the newest first, `resolve_image` finds it by
/// prefix, and `materialise_image` decompresses it back to bytes identical to
/// the original** — item 9's whole round trip, against a real scratch fleet
/// repository this test builds with two published versions.
#[test]
fn list_resolve_and_materialise_round_trip_a_published_card() {
    let work = scratch("read-side");
    let seed_bytes = [0x66u8; 32];
    let seed = write_seed(&work, "rivet-system.seed", 0x66);
    let fleet_root = work.join("fleet");

    let mut originals = Vec::new();
    for (version, fill) in [("0.1.0", 0xAAu8), ("0.1.1", 0xBBu8)] {
        let image = fixture_image(&format!("read-side-{version}"), Some(1), &vec![fill; 2048]);
        originals.push((version, std::fs::read(&image).expect("read the fixture")));
        run(&[
            "--img".into(),
            image.display().to_string(),
            "--version".into(),
            version.into(),
            "--arch".into(),
            "aarch64".into(),
            "--signing-key".into(),
            seed.display().to_string(),
            "--out-dir".into(),
            work.join("out").display().to_string(),
            "--at".into(),
            fleet_root.display().to_string(),
        ])
        .expect("publish");
    }

    let images = crate::fleet::list_images(&fleet_root, None).expect("list_images");
    assert_eq!(images.len(), 2, "both published versions are card images");
    assert_eq!(images[0].version, "0.1.1", "newest first");
    assert_eq!(images[1].version, "0.1.0");
    assert!(images.iter().all(|e| e.platform.as_deref() == Some("rpi4")));
    assert!(images.iter().all(|e| e.schema_version == Some(1)));

    // Narrowed by platform: the only platform this build has an implementor
    // for, so the filter must still find both.
    let narrowed = crate::fleet::list_images(&fleet_root, Some("rpi4")).expect("narrowed");
    assert_eq!(narrowed.len(), 2);
    let none_for_a_platform_nothing_published =
        crate::fleet::list_images(&fleet_root, Some("rpizw")).expect("narrowed to nothing");
    assert!(none_for_a_platform_nothing_published.is_empty());

    let newest = crate::fleet::resolve_image(&images, &images[0].digest[..12]).expect("resolves");
    assert_eq!(newest.version, "0.1.1");

    let public = rivet_crypto::Ed25519Signer::from_seed(&seed_bytes).public_key();
    let materialise_dir = work.join("materialised");
    let materialised =
        crate::fleet::materialise_image(&fleet_root, newest, public, &materialise_dir)
            .expect("materialises and decompresses");
    let materialised_bytes = std::fs::read(&materialised).expect("read the materialised image");
    let (_, expected) = originals
        .iter()
        .find(|(v, _)| *v == "0.1.1")
        .expect("the 0.1.1 fixture");
    assert_eq!(
        &materialised_bytes, expected,
        "the materialised image must equal the original byte for byte"
    );
}

/// **A key not entitled to `card` is refused by `materialise_image`**, the
/// read side's own entitlement check rather than a hard-link that trusts
/// whatever the repository already accepted.
#[test]
fn materialise_image_refuses_a_key_not_entitled_to_card() {
    let work = scratch("read-side-wrong-key");
    let seed = write_seed(&work, "rivet-system.seed", 0x77);
    let fleet_root = work.join("fleet");
    let image = fixture_image("read-side-wrong-key-src", Some(1), b"content");

    run(&[
        "--img".into(),
        image.display().to_string(),
        "--version".into(),
        "0.1.0".into(),
        "--arch".into(),
        "aarch64".into(),
        "--signing-key".into(),
        seed.display().to_string(),
        "--out-dir".into(),
        work.join("out").display().to_string(),
        "--at".into(),
        fleet_root.display().to_string(),
    ])
    .expect("publish");

    let images = crate::fleet::list_images(&fleet_root, None).expect("list");
    let wrong_key = rivet_crypto::Ed25519Signer::from_seed(&[0x99; 32]).public_key();
    let refused = crate::fleet::materialise_image(
        &fleet_root,
        &images[0],
        wrong_key,
        &work.join("materialised"),
    )
    .expect_err("a stranger's key must not verify this image");
    assert!(refused.contains("does not verify"), "{refused}");
}

/// **`fleet publish` still accepts the exact `.rvt` this verb writes, by
/// hand** — `R-177` D1's explicit requirement.
#[test]
fn the_signed_rvt_is_still_accepted_by_fleet_publish_by_hand() {
    let work = scratch("hand-publish");
    let image = fixture_image("hand-publish-src", Some(1), b"tail-bytes-for-content");
    let seed = write_seed(&work, "rivet-system.seed", 0x55);
    let out_dir = work.join("out");

    run(&[
        "--img".into(),
        image.display().to_string(),
        "--version".into(),
        "0.1.0".into(),
        "--arch".into(),
        "aarch64".into(),
        "--signing-key".into(),
        seed.display().to_string(),
        "--out-dir".into(),
        out_dir.display().to_string(),
        // No --at: this run only WRAPS, and never publishes, so a second
        // repository can `fleet publish` the result by hand exactly as R-177
        // D1 requires — but `run` always publishes. So instead this proves
        // the file itself is independently acceptable: a fresh repository,
        // a fresh `fleet publish`.
        "--at".into(),
        work.join("first-fleet").display().to_string(),
    ])
    .expect("wraps and publishes once");

    let rvt = out_dir.join("rivet-0.1.0-aarch64-rpi4-card.rvt");
    assert!(rvt.is_file());

    let public = work.join("rivet-system.pub");
    let public_key_bytes = rivet_crypto::Ed25519Signer::from_seed(&[0x55; 32]).public_key();
    std::fs::write(&public, public_key_bytes.0).expect("write the public key");

    let second_fleet = work.join("second-fleet");
    crate::fleet::init(&second_fleet).expect("init a second repository");
    let report = crate::fleet::publish(&second_fleet, &rvt, &public)
        .expect("fleet publish accepts it by hand");
    assert!(report.contains("MADE"), "{report}");
}
