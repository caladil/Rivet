//! The APK reader against vectors derived from real builds and a real
//! `apksigner` rotation — `ADR-061` §4. `tests/fixtures/apk/README.md` says
//! where each file came from and what Android's own tools printed for it.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::tests::*;
use super::*;

// ---------------------------------------------------------------------------
// The derived real fixtures — their README records the aapt2 and apksigner
// output these values come from.
// ---------------------------------------------------------------------------

const HC_WRITER: &[u8] = include_bytes!("../tests/fixtures/apk/hc-writer-derived.apk");
const HARNESS: &[u8] = include_bytes!("../tests/fixtures/apk/rivet-harness-derived.apk");

/// `apksigner verify --print-certs` on the committed fixtures: the certificate
/// of the throwaway key `CN=Rivet Test One`, which signs both derived fixtures
/// and is the OLD key of the rotation — see the README.
pub(crate) const TEST_KEY_ONE_SHA256: &str =
    "de920ba08faf66400adf285fef796a374da7111a98b7ba0bdd087f2cd69ef06f";
/// The throwaway key `CN=Rivet Test Two`, the NEW key of the rotation.
pub(crate) const TEST_KEY_TWO_SHA256: &str =
    "acd2503f132b6970932f302fd40ee91df3030bdda60037c9bac2c192056d740f";

/// **Every `apk.json` manifest field read from the derived hc-writer fixture
/// matches what `aapt2 dump badging` printed for the original build**, and the
/// signer is the throwaway key it was re-signed with (values in the README).
#[test]
fn the_derived_hc_writer_fixture_reads_as_aapt2_and_apksigner_read_the_original() {
    let info = read(HC_WRITER).expect("the derived fixture reads");
    assert_eq!(info.application_id, "io.github.caladil.hcwriter");
    assert_eq!(info.version_code, 1);
    assert_eq!(info.version_code_major, None);
    assert_eq!(info.version_name, "0.0.1");
    assert_eq!(info.label, "HC Writer");
    assert_eq!(info.min_sdk, 31);
    assert_eq!(info.target_sdk, 36);
    assert_eq!(info.scheme, Scheme::V2);
    assert_eq!(info.signer_sha256(), TEST_KEY_ONE_SHA256);
    assert!(info.lineage.is_empty());
}

/// The same, for the derived harness fixture, whose `resources.arsc` is
/// deflated in the derived zip — so the inflate path reads a real table too.
#[test]
fn the_derived_harness_fixture_reads_as_aapt2_and_apksigner_read_the_original() {
    let info = read(HARNESS).expect("the derived fixture reads");
    assert_eq!(info.application_id, "io.github.caladil.rivetharness");
    assert_eq!(info.version_code, 16);
    assert_eq!(info.version_name, "0.1.16");
    assert_eq!(info.label, "Rivet Pair Harness");
    assert_eq!(info.min_sdk, 31);
    assert_eq!(info.target_sdk, 36);
    assert_eq!(info.scheme, Scheme::V2);
    assert_eq!(info.signer_sha256(), TEST_KEY_ONE_SHA256);
    assert!(info.lineage.is_empty());
}

// ---------------------------------------------------------------------------
// Truncation and bad offsets — refused by name, never a panic, and never an
// answer that differs from the whole structure's.
// ---------------------------------------------------------------------------

/// The harness's label, which its manifest holds as a reference into its
/// resource table (hc-writer's holds a literal string, so the harness is the
/// fixture that exercises the resolution).
fn harness_label_id() -> u32 {
    let archive = zip::Archive::open(HARNESS).expect("opens");
    let manifest_bytes = archive.read("AndroidManifest.xml").expect("inflates");
    let parsed = axml::read_manifest(&manifest_bytes).expect("parses");
    let axml::Value::Reference(label_id) = parsed.label else {
        panic!("the harness's label is a reference");
    };
    label_id
}

#[test]
fn the_two_real_manifests_hold_their_labels_the_two_ways_there_are() {
    let archive = zip::Archive::open(HC_WRITER).expect("opens");
    let parsed = axml::read_manifest(&archive.read("AndroidManifest.xml").expect("inflates"))
        .expect("parses");
    assert_eq!(parsed.label, axml::Value::Str("HC Writer".to_string()));
    assert_eq!(harness_label_id(), 0x7f0c_001c);
}

/// A small whole APK composed from REAL parts: the harness's own deflated
/// manifest bytes and its own signing block, around a hand-composed one-entry
/// resource table that answers the manifest's label reference.
fn small_real_parts_apk() -> Vec<u8> {
    let archive = zip::Archive::open(HARNESS).expect("opens");
    let entry = archive.find("AndroidManifest.xml").expect("has a manifest").clone();
    let raw = archive.raw(&entry).expect("raw bytes");
    let table = resource_table(harness_label_id(), "Rivet Pair Harness");
    let block = signing_block_of(HARNESS);
    zip(
        &[
            Part {
                name: "AndroidManifest.xml",
                method: entry.method,
                crc32: entry.crc32,
                uncompressed: entry.uncompressed as u32,
                raw,
            },
            Part::stored("resources.arsc", &table),
        ],
        Some(&block),
    )
}

#[test]
fn the_small_real_parts_apk_reads_whole() {
    let info = read(&small_real_parts_apk()).expect("reads");
    assert_eq!(info.application_id, "io.github.caladil.rivetharness");
    assert_eq!(info.label, "Rivet Pair Harness");
    assert_eq!(info.version_name, "0.1.16");
}

/// **Every prefix of a whole APK is refused**, and each refusal is one of the
/// named variants — never a panic, and never a result read from a file that
/// was cut short. A prefix loses the end-of-central-directory record, so this
/// is the refusal every one of them must give.
#[test]
fn every_truncation_of_a_whole_apk_is_refused_by_name() {
    let apk = small_real_parts_apk();
    for len in 0..apk.len() {
        assert_eq!(
            read(&apk[..len]),
            Err(ApkError::NoEndOfCentralDirectory),
            "a prefix of {len} of {} bytes",
            apk.len()
        );
    }
}

/// `data` cut to `len` bytes, with the outer chunk's own size field (at
/// bytes 4..8) rewritten to match — so the cut is found by whatever INSIDE
/// the structure it lands in, rather than by the outer header every time.
fn cut_and_reframe(data: &[u8], len: usize) -> Vec<u8> {
    let mut cut = data[..len].to_vec();
    if len >= 8 {
        cut[4..8].copy_from_slice(&(len as u32).to_le_bytes());
    }
    cut
}

/// **Every truncation of a real manifest is refused, or reads exactly what the
/// whole one reads.** Cut as it lies, the header's own size refuses every
/// prefix; re-framed, the cut lands inside the string pool, an element or an
/// attribute and is refused there. A cut that falls just after `<application>`
/// opens can legitimately read the same answer — nothing after it is needed —
/// and no other success is allowed.
#[test]
fn every_truncation_of_a_real_manifest_is_refused_or_reads_the_same() {
    let archive = zip::Archive::open(HARNESS).expect("opens");
    let manifest = archive.read("AndroidManifest.xml").expect("inflates");
    let whole = axml::read_manifest(&manifest).expect("the whole one reads");
    let mut refused_inside = 0;
    for len in 0..manifest.len() {
        let err = axml::read_manifest(&manifest[..len]).expect_err("a cut manifest is refused");
        assert!(matches!(err, ApkError::Truncated { .. }), "a prefix of {len}: {err}");
        match axml::read_manifest(&cut_and_reframe(&manifest, len)) {
            Ok(read) => assert_eq!(read, whole, "a re-framed prefix of {len} read something else"),
            Err(_) => refused_inside += 1,
        }
    }
    assert!(refused_inside > manifest.len() / 2, "{refused_inside} of {}", manifest.len());
}

/// **A truncation of a real resource table is refused, or resolves the same
/// label**, sampled every 7 bytes over the whole table, re-framed as above.
#[test]
fn a_truncation_of_a_real_resource_table_is_refused_or_resolves_the_same() {
    let archive = zip::Archive::open(HARNESS).expect("opens");
    let table = archive.read("resources.arsc").expect("reads");
    let label_id = harness_label_id();
    assert_eq!(arsc::resolve_string(&table, label_id).expect("whole"), "Rivet Pair Harness");
    for len in (0..table.len()).step_by(7) {
        let err = arsc::resolve_string(&table[..len], label_id).expect_err("a cut table is refused");
        assert!(matches!(err, ApkError::Truncated { .. }), "a prefix of {len}: {err}");
        if let Ok(label) = arsc::resolve_string(&cut_and_reframe(&table, len), label_id) {
            assert_eq!(label, "Rivet Pair Harness", "a re-framed prefix of {len}");
        }
    }
}

/// **Every truncation of a real signing block's pairs is refused, or reads the
/// same certificate**: the block is cut, then re-framed with correct size
/// fields, so the cut is found in the pairs and the v2 structure inside them.
/// A cut on a pair boundary after the v2 pair is a valid, shorter block.
#[test]
fn every_truncation_inside_a_real_signing_block_is_refused_or_reads_the_same() {
    let block = signing_block_of(HC_WRITER);
    let whole = read(&hand_apk("a.b.c", 1, "1.0.0", &block)).expect("the whole block reads");
    let region = &block[8..block.len() - 24];
    for len in 0..region.len() {
        let mut framed = ((len + 24) as u64).to_le_bytes().to_vec();
        framed.extend(&region[..len]);
        framed.extend(((len + 24) as u64).to_le_bytes());
        framed.extend(b"APK Sig Block 42");
        match read(&hand_apk("a.b.c", 1, "1.0.0", &framed)) {
            Ok(info) => assert_eq!(info, whole, "a region of {len} read something else"),
            Err(err) => assert!(
                matches!(err, ApkError::Truncated { .. } | ApkError::Malformed { .. } | ApkError::NoSigner),
                "a region of {len}: {err}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The lineage of a REAL rotated APK — signed by apksigner with two keys minted
// for the purpose; see the fixture README.
// ---------------------------------------------------------------------------

const ROTATED_V3: &[u8] = include_bytes!("../tests/fixtures/apk/rotated-v3.apk");
const ROTATED_V31: &[u8] = include_bytes!("../tests/fixtures/apk/rotated-v31.apk");

/// `apksigner lineage --print-certs` on both rotated fixtures — see the README.
const ROTATED_OLD_SHA256: &str = TEST_KEY_ONE_SHA256;
const ROTATED_NEW_SHA256: &str = TEST_KEY_TWO_SHA256;

/// **No committed fixture carries a real date or the Android debug
/// certificate.** Every certificate in them is a throwaway key's, generated
/// with a fixed start of 2000-01-01 (keytool writes that local midnight, which
/// here is 06:00 UTC) and a fixed end in 2099 — so the only times in the files
/// are those two, and the debug key's subject appears nowhere.
#[test]
fn no_fixture_carries_a_real_date_or_the_debug_certificate() {
    for (name, bytes) in [
        ("hc-writer-derived.apk", HC_WRITER),
        ("rivet-harness-derived.apk", HARNESS),
        ("rotated-v3.apk", ROTATED_V3),
        ("rotated-v31.apk", ROTATED_V31),
    ] {
        assert!(!contains(bytes, b"Android Debug"), "{name} carries the debug certificate");
        // Every DER UTCTime (YYMMDDhhmmssZ) and GeneralizedTime
        // (YYYYMMDDhhmmssZ) in the file.
        let mut times = Vec::new();
        for at in 0..bytes.len() {
            for (tag, len) in [(0x17u8, 13usize), (0x18, 15)] {
                if bytes[at] == tag
                    && bytes.get(at + 1) == Some(&(len as u8))
                    && let Some(text) = bytes.get(at + 2..at + 2 + len)
                    && text[len - 1] == b'Z'
                    && text[..len - 1].iter().all(u8::is_ascii_digit)
                {
                    times.push(String::from_utf8_lossy(text).into_owned());
                }
            }
        }
        assert!(!times.is_empty(), "{name}: no certificate time was found at all");
        for time in &times {
            assert!(
                time == "000101060000Z" || time == "20991207060000Z",
                "{name} carries the time {time}"
            );
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// **A real v3 rotation**, signed by `apksigner` with rotation from SDK 28:
/// the lineage is the old key then the new one, and the new one is current.
#[test]
fn the_lineage_of_a_real_v3_rotated_apk_is_read() {
    let info = read(ROTATED_V3).expect("reads whole");
    assert_eq!(info.application_id, "io.github.caladil.hcwriter");
    assert_eq!(info.scheme, Scheme::V3);
    assert_eq!(info.signer_sha256(), ROTATED_NEW_SHA256);
    assert_eq!(info.lineage_sha256(), vec![ROTATED_OLD_SHA256, ROTATED_NEW_SHA256]);
}

/// **A real v3.1 rotation** — `apksigner`'s default, rotation from SDK 33:
/// the v3 block still names the OLD key for older phones, and the v3.1 block
/// names the new one. The current certificate is v3.1's.
#[test]
fn the_current_signer_of_a_real_v31_rotated_apk_is_the_v31_one() {
    let info = read(ROTATED_V31).expect("reads whole");
    assert_eq!(info.scheme, Scheme::V31);
    assert_eq!(info.signer_sha256(), ROTATED_NEW_SHA256);
    assert_eq!(info.lineage_sha256(), vec![ROTATED_OLD_SHA256, ROTATED_NEW_SHA256]);
}

