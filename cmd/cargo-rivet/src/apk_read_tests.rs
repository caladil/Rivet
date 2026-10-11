//! The APK reader, from the outside in — `ADR-061` §4.
//!
//! # Provenance of every vector here (`AGENTS.md` §4)
//!
//! * `tests/fixtures/apk/*.apk` are **derived from real debug builds**; their
//!   README says from what, how, and which values `aapt2 dump badging` and
//!   `apksigner verify --print-certs` printed for the originals.
//! * Everything built by the helpers below — [`zip`], [`signing_block`],
//!   [`v2_block`], [`v3_block`], [`manifest`], [`resource_table`] — is
//!   **hand-composed** from the published formats, and proves only that this
//!   reader agrees with its author's reading of them. The derived fixtures are
//!   what tie it to what Android's tools actually write.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::zip::{METHOD_DEFLATED, METHOD_STORED};
use super::*;

// ---------------------------------------------------------------------------
// Hand-composed builders — each writes one published format, nothing more.
// ---------------------------------------------------------------------------

/// One entry for [`zip`]: its bytes exactly as they will lie in the file.
pub(crate) struct Part<'a> {
    pub(crate) name: &'a str,
    pub(crate) method: u16,
    pub(crate) crc32: u32,
    pub(crate) uncompressed: u32,
    pub(crate) raw: &'a [u8],
}

impl<'a> Part<'a> {
    /// A stored entry. The CRC is left zero: this reader does not check it.
    pub(crate) fn stored(name: &'a str, raw: &'a [u8]) -> Self {
        Self { name, method: METHOD_STORED, crc32: 0, uncompressed: raw.len() as u32, raw }
    }
}

/// A zip of `parts`, with `block` — an APK Signing Block — between the last
/// entry and the central directory, as an APK lays one out.
pub(crate) fn zip(parts: &[Part<'_>], block: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for part in parts {
        offsets.push(out.len() as u32);
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes()); // version needed
        out.extend(0u16.to_le_bytes()); // flags
        out.extend(part.method.to_le_bytes());
        out.extend(0x0820u16.to_le_bytes()); // time
        out.extend(0x0221u16.to_le_bytes()); // date
        out.extend(part.crc32.to_le_bytes());
        out.extend((part.raw.len() as u32).to_le_bytes());
        out.extend(part.uncompressed.to_le_bytes());
        out.extend((part.name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes()); // extra
        out.extend(part.name.as_bytes());
        out.extend(part.raw);
    }
    if let Some(block) = block {
        out.extend(block);
    }
    let cd = out.len() as u32;
    for (part, offset) in parts.iter().zip(&offsets) {
        out.extend(0x0201_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes()); // made by
        out.extend(20u16.to_le_bytes()); // needed
        out.extend(0u16.to_le_bytes()); // flags
        out.extend(part.method.to_le_bytes());
        out.extend(0x0820u16.to_le_bytes());
        out.extend(0x0221u16.to_le_bytes());
        out.extend(part.crc32.to_le_bytes());
        out.extend((part.raw.len() as u32).to_le_bytes());
        out.extend(part.uncompressed.to_le_bytes());
        out.extend((part.name.len() as u16).to_le_bytes());
        out.extend([0u8; 12]); // extra, comment, disk, internal, external
        out.extend(offset.to_le_bytes());
        out.extend(part.name.as_bytes());
    }
    let cd_size = out.len() as u32 - cd;
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]); // disk numbers
    out.extend((parts.len() as u16).to_le_bytes());
    out.extend((parts.len() as u16).to_le_bytes());
    out.extend(cd_size.to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// `bytes`, prefixed with their length as a little-endian `u32`.
pub(crate) fn lp(bytes: &[u8]) -> Vec<u8> {
    let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
    out.extend(bytes);
    out
}

/// An APK Signing Block holding `pairs`.
pub(crate) fn signing_block(pairs: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, value) in pairs {
        body.extend(((value.len() + 4) as u64).to_le_bytes());
        body.extend(id.to_le_bytes());
        body.extend(value);
    }
    let size = (body.len() + 8 + 16) as u64;
    let mut out = size.to_le_bytes().to_vec();
    out.extend(body);
    out.extend(size.to_le_bytes());
    out.extend(b"APK Sig Block 42");
    out
}

pub(crate) const V2: u32 = 0x7109_871a;
pub(crate) const V3: u32 = 0xf053_68c0;
pub(crate) const V31: u32 = 0x1b93_ad61;

/// A v2 signature block value with one signer per certificate.
pub(crate) fn v2_block(certificates: &[&[u8]]) -> Vec<u8> {
    let mut signers = Vec::new();
    for certificate in certificates {
        let signed = [lp(b""), lp(&lp(certificate)), lp(b"")].concat();
        signers.extend(lp(&[lp(&signed), lp(b"signatures"), lp(b"public key")].concat()));
    }
    lp(&signers)
}

/// One v3 signer: its SDK range, certificate, and optional lineage.
pub(crate) struct V3Signer<'a> {
    pub(crate) min_sdk: u32,
    pub(crate) max_sdk: u32,
    pub(crate) certificate: &'a [u8],
    pub(crate) lineage: Option<Vec<&'a [u8]>>,
}

/// A proof-of-rotation attribute value over `certificates`, oldest first.
pub(crate) fn lineage_value(certificates: &[&[u8]], version: u32) -> Vec<u8> {
    let mut out = version.to_le_bytes().to_vec();
    for certificate in certificates {
        let signed = [lp(certificate), 0x0103u32.to_le_bytes().to_vec()].concat();
        let node = [
            lp(&signed),
            0u32.to_le_bytes().to_vec(),
            0x0103u32.to_le_bytes().to_vec(),
            lp(b"a signature by the previous key"),
        ]
        .concat();
        out.extend(lp(&node));
    }
    out
}

/// A v3 (or v3.1) signature block value.
pub(crate) fn v3_block(signers: &[V3Signer<'_>]) -> Vec<u8> {
    let mut all = Vec::new();
    for signer in signers {
        let attributes = match &signer.lineage {
            Some(certificates) => lp(&[
                0x3ba0_6f8cu32.to_le_bytes().to_vec(),
                lineage_value(certificates, 1),
            ]
            .concat()),
            None => Vec::new(),
        };
        let signed = [
            lp(b""),
            lp(&lp(signer.certificate)),
            signer.min_sdk.to_le_bytes().to_vec(),
            signer.max_sdk.to_le_bytes().to_vec(),
            lp(&attributes),
        ]
        .concat();
        all.extend(lp(&[
            lp(&signed),
            signer.min_sdk.to_le_bytes().to_vec(),
            signer.max_sdk.to_le_bytes().to_vec(),
            lp(b"signatures"),
            lp(b"public key"),
        ]
        .concat()));
    }
    lp(&all)
}

/// An attribute value for [`manifest`].
#[derive(Clone)]
pub(crate) enum Attr<'a> {
    Str(&'a str),
    Int(u32),
    Ref(u32),
}

const ANDROID: &str = "http://schemas.android.com/apk/res/android";

/// A UTF-16 string pool chunk of `strings` — the encoding `aapt2` gives a
/// compiled manifest.
fn string_pool(strings: &[String], utf8: bool) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for s in strings {
        offsets.push(data.len() as u32);
        if utf8 {
            let units = s.encode_utf16().count();
            data.push(units as u8);
            data.push(s.len() as u8);
            data.extend(s.as_bytes());
            data.push(0);
        } else {
            let units: Vec<u16> = s.encode_utf16().collect();
            data.extend((units.len() as u16).to_le_bytes());
            for u in units {
                data.extend(u.to_le_bytes());
            }
            data.extend(0u16.to_le_bytes());
        }
    }
    while data.len() % 4 != 0 {
        data.push(0);
    }
    let strings_start = 28 + 4 * strings.len() as u32;
    let size = strings_start + data.len() as u32;
    let mut out = Vec::new();
    out.extend(0x0001u16.to_le_bytes());
    out.extend(28u16.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend((strings.len() as u32).to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((if utf8 { 1u32 << 8 } else { 0 }).to_le_bytes());
    out.extend(strings_start.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    for o in offsets {
        out.extend(o.to_le_bytes());
    }
    out.extend(data);
    out
}

/// A compiled `AndroidManifest.xml`: `<manifest>` with `manifest_attrs`, then
/// `<uses-sdk>` (when given) and `<application>` as its children. Every
/// attribute but `package` is in the `android:` namespace.
pub(crate) fn manifest(
    manifest_attrs: &[(&str, Attr<'_>)],
    uses_sdk: Option<&[(&str, Attr<'_>)]>,
    application: &[(&str, Attr<'_>)],
) -> Vec<u8> {
    let mut strings: Vec<String> = vec![ANDROID.to_string()];
    let index = |s: &str, strings: &mut Vec<String>| -> u32 {
        if let Some(i) = strings.iter().position(|t| t == s) {
            return i as u32;
        }
        strings.push(s.to_string());
        (strings.len() - 1) as u32
    };

    let mut elements: Vec<(String, Vec<(String, Attr<'_>)>)> = Vec::new();
    elements.push((
        "manifest".to_string(),
        manifest_attrs.iter().map(|(n, v)| ((*n).to_string(), v.clone())).collect(),
    ));
    if let Some(attrs) = uses_sdk {
        elements.push((
            "uses-sdk".to_string(),
            attrs.iter().map(|(n, v)| ((*n).to_string(), v.clone())).collect(),
        ));
    }
    elements.push((
        "application".to_string(),
        application.iter().map(|(n, v)| ((*n).to_string(), v.clone())).collect(),
    ));

    let mut body = Vec::new();
    let mut end_manifest = Vec::new();
    for (depth, (name, attrs)) in elements.iter().enumerate() {
        let name_index = index(name, &mut strings);
        let mut chunk = Vec::new();
        chunk.extend(0x0102u16.to_le_bytes());
        chunk.extend(16u16.to_le_bytes());
        chunk.extend(0u32.to_le_bytes()); // size, patched below
        chunk.extend(1u32.to_le_bytes()); // line
        chunk.extend(0xffff_ffffu32.to_le_bytes()); // comment
        chunk.extend(0xffff_ffffu32.to_le_bytes()); // ns
        chunk.extend(name_index.to_le_bytes());
        chunk.extend(20u16.to_le_bytes()); // attributeStart
        chunk.extend(20u16.to_le_bytes()); // attributeSize
        chunk.extend((attrs.len() as u16).to_le_bytes());
        chunk.extend([0u8; 6]);
        for (attr_name, attr_value) in attrs {
            // String 0 is the android namespace URI; `package` has none.
            let ns: u32 = if attr_name == "package" { 0xffff_ffff } else { 0 };
            let name_at = index(attr_name, &mut strings);
            let (raw, data_type, data) = match attr_value {
                Attr::Str(s) => {
                    let i = index(s, &mut strings);
                    (i, 0x03u8, i)
                }
                Attr::Int(n) => (0xffff_ffff, 0x10, *n),
                Attr::Ref(id) => (0xffff_ffff, 0x01, *id),
            };
            chunk.extend(ns.to_le_bytes());
            chunk.extend(name_at.to_le_bytes());
            chunk.extend(raw.to_le_bytes());
            chunk.extend(8u16.to_le_bytes());
            chunk.push(0);
            chunk.push(data_type);
            chunk.extend(data.to_le_bytes());
        }
        let size = chunk.len() as u32;
        chunk[4..8].copy_from_slice(&size.to_le_bytes());
        body.extend(chunk);

        let mut end = Vec::new();
        end.extend(0x0103u16.to_le_bytes());
        end.extend(16u16.to_le_bytes());
        end.extend(24u32.to_le_bytes());
        end.extend(1u32.to_le_bytes());
        end.extend(0xffff_ffffu32.to_le_bytes());
        end.extend(0xffff_ffffu32.to_le_bytes());
        end.extend(name_index.to_le_bytes());
        if depth == 0 {
            end_manifest = end;
        } else {
            body.extend(end);
        }
    }
    body.extend(end_manifest);

    let pool = string_pool(&strings, false);
    let size = (8 + pool.len() + body.len()) as u32;
    let mut out = Vec::new();
    out.extend(0x0003u16.to_le_bytes());
    out.extend(8u16.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(pool);
    out.extend(body);
    out
}

/// A `resources.arsc` defining exactly one resource, `id`, as `value` in the
/// default configuration. Its package header carries no type or key string
/// pools, which this reader does not read.
pub(crate) fn resource_table(id: u32, value: &str) -> Vec<u8> {
    let entry = (id & 0xffff) as usize;
    let count = entry + 1;
    let header = 20 + 64u32; // ResTable_type fields + a 64-byte config
    let entries_start = header + 4 * count as u32;
    let mut type_chunk = Vec::new();
    type_chunk.extend(0x0201u16.to_le_bytes());
    type_chunk.extend((header as u16).to_le_bytes());
    type_chunk.extend((entries_start + 16).to_le_bytes());
    type_chunk.push(((id >> 16) & 0xff) as u8);
    type_chunk.push(0); // flags
    type_chunk.extend(0u16.to_le_bytes());
    type_chunk.extend((count as u32).to_le_bytes());
    type_chunk.extend(entries_start.to_le_bytes());
    type_chunk.extend(64u32.to_le_bytes());
    type_chunk.extend([0u8; 60]);
    for i in 0..count {
        let offset: u32 = if i == entry { 0 } else { 0xffff_ffff };
        type_chunk.extend(offset.to_le_bytes());
    }
    type_chunk.extend(8u16.to_le_bytes()); // entry size
    type_chunk.extend(0u16.to_le_bytes()); // flags
    type_chunk.extend(0u32.to_le_bytes()); // key
    type_chunk.extend(8u16.to_le_bytes()); // Res_value size
    type_chunk.push(0);
    type_chunk.push(0x03);
    type_chunk.extend(0u32.to_le_bytes()); // string 0

    let mut package = Vec::new();
    package.extend(0x0200u16.to_le_bytes());
    package.extend(288u16.to_le_bytes());
    package.extend((288 + type_chunk.len() as u32).to_le_bytes());
    package.extend((id >> 24).to_le_bytes());
    package.extend([0u8; 288 - 12]);
    package.extend(type_chunk);

    let pool = string_pool(&[value.to_string()], true);
    let mut out = Vec::new();
    out.extend(0x0002u16.to_le_bytes());
    out.extend(12u16.to_le_bytes());
    out.extend(((12 + pool.len() + package.len()) as u32).to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(pool);
    out.extend(package);
    out
}

/// A whole hand-composed APK: `version_code` and `version_name`, a literal
/// label, SDK 26/34, signed (as far as a reader can tell) by `block`.
pub(crate) fn hand_apk(package: &str, version_code: u32, version_name: &str, block: &[u8]) -> Vec<u8> {
    let manifest = manifest(
        &[
            ("package", Attr::Str(package)),
            ("versionCode", Attr::Int(version_code)),
            ("versionName", Attr::Str(version_name)),
        ],
        Some(&[("minSdkVersion", Attr::Int(26)), ("targetSdkVersion", Attr::Int(34))]),
        &[("label", Attr::Str("Hand Composed"))],
    );
    zip(&[Part::stored("AndroidManifest.xml", &manifest)], Some(block))
}

/// A hand-composed APK signed by one v2 certificate.
pub(crate) fn hand_apk_v2(package: &str, version_code: u32, version_name: &str, certificate: &[u8]) -> Vec<u8> {
    hand_apk(package, version_code, version_name, &signing_block(&[(V2, v2_block(&[certificate]))]))
}

/// The APK Signing Block of `apk`, verbatim.
pub(crate) fn signing_block_of(apk: &[u8]) -> Vec<u8> {
    let cd = zip::Archive::open(apk).expect("opens").central_directory_offset();
    let size = u64::from_le_bytes(apk[cd - 24..cd - 16].try_into().unwrap()) as usize;
    apk[cd - size - 8..cd].to_vec()
}

// ---------------------------------------------------------------------------
// Truncation and bad offsets — refused by name, never a panic. The same
// properties against real structures are in `apk_read_fixture_tests.rs`.
// ---------------------------------------------------------------------------

/// **Every prefix of a whole hand-composed APK — label resolved through a
/// resource table, a v3 lineage in its signing block — is refused**, and by
/// the one refusal a prefix can earn: it has lost its end-of-central-directory
/// record.
#[test]
fn every_truncation_of_a_hand_composed_apk_is_refused_by_name() {
    let body = manifest(
        &[("package", Attr::Str("a.b.gantry")), ("versionCode", Attr::Int(3)), ("versionName", Attr::Str("0.1.3"))],
        Some(&[("minSdkVersion", Attr::Int(31))]),
        &[("label", Attr::Ref(0x7f0c_0002))],
    );
    let table = resource_table(0x7f0c_0002, "Gantry");
    let block = signing_block(&[(
        V3,
        v3_block(&[V3Signer { min_sdk: 28, max_sdk: 0x7fff_ffff, certificate: b"new", lineage: Some(vec![b"old", b"new"]) }]),
    )]);
    let apk = zip(
        &[Part::stored("AndroidManifest.xml", &body), Part::stored("resources.arsc", &table)],
        Some(&block),
    );
    assert_eq!(read(&apk).expect("the whole one reads").label, "Gantry");
    for len in 0..apk.len() {
        assert_eq!(read(&apk[..len]), Err(ApkError::NoEndOfCentralDirectory), "a prefix of {len}");
    }
}

#[test]
fn a_central_directory_offset_outside_the_file_is_refused_by_name() {
    let mut apk = hand_apk_v2("a.b.c", 1, "1.0.0", b"cert");
    let eocd = apk.len() - 22;
    apk[eocd + 16..eocd + 20].copy_from_slice(&0x7fff_0000u32.to_le_bytes());
    assert!(matches!(read(&apk), Err(ApkError::Malformed { structure, .. }) if structure.contains("end-of-central")));
}

#[test]
fn a_local_header_offset_outside_the_file_is_refused_by_name() {
    let mut apk = hand_apk_v2("a.b.c", 1, "1.0.0", b"cert");
    let cd = zip::Archive::open(&apk).expect("opens").central_directory_offset();
    apk[cd + 42..cd + 46].copy_from_slice(&0x7fff_0000u32.to_le_bytes());
    assert!(matches!(read(&apk), Err(ApkError::Truncated { structure: "zip local header", .. })));
}

#[test]
fn an_entry_whose_size_runs_into_the_central_directory_is_refused_by_name() {
    let mut apk = hand_apk_v2("a.b.c", 1, "1.0.0", b"cert");
    let cd = zip::Archive::open(&apk).expect("opens").central_directory_offset();
    // Stored, so the directory's two sizes must agree: raise both.
    apk[cd + 20..cd + 24].copy_from_slice(&0x0010_0000u32.to_le_bytes());
    apk[cd + 24..cd + 28].copy_from_slice(&0x0010_0000u32.to_le_bytes());
    assert!(matches!(read(&apk), Err(ApkError::Truncated { structure: "zip entry data", .. })));
}

#[test]
fn a_string_index_outside_the_pool_is_refused_by_name() {
    let mut body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    // The first element's name index sits 8 + pool + 16 + 4 bytes in.
    let pool_size = u32::from_le_bytes(body[12..16].try_into().unwrap()) as usize;
    let at = 8 + pool_size + 16 + 4;
    body[at..at + 4].copy_from_slice(&9999u32.to_le_bytes());
    let err = axml::read_manifest(&body).expect_err("refused");
    assert!(matches!(err, ApkError::Malformed { structure: "string pool", .. }), "{err}");
}

#[test]
fn a_string_pool_count_that_would_overflow_is_refused_by_name() {
    let mut body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    body[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    let err = axml::read_manifest(&body).expect_err("refused");
    assert!(
        matches!(err, ApkError::Truncated { structure: "string pool", .. } | ApkError::Malformed { .. }),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// The zip's own refusals.
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_compression_method_is_refused_by_name() {
    let manifest = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(
        &[Part { name: "AndroidManifest.xml", method: 12, crc32: 0, uncompressed: manifest.len() as u32, raw: &manifest }],
        Some(&signing_block(&[(V2, v2_block(&[b"cert"]))])),
    );
    assert_eq!(
        read(&apk),
        Err(ApkError::UnknownCompression { entry: "AndroidManifest.xml".to_string(), method: 12 })
    );
}

#[test]
fn a_deflated_entry_that_does_not_inflate_is_refused_by_name() {
    let apk = zip(
        &[Part { name: "AndroidManifest.xml", method: METHOD_DEFLATED, crc32: 0, uncompressed: 100, raw: &[0xff; 16] }],
        Some(&signing_block(&[(V2, v2_block(&[b"cert"]))])),
    );
    assert!(matches!(read(&apk), Err(ApkError::Inflate { .. })));
}

#[test]
fn a_deflated_entry_of_the_wrong_length_is_refused_by_name() {
    let manifest = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    let deflated = miniz_oxide::deflate::compress_to_vec(&manifest, 6);
    let apk = zip(
        &[Part {
            name: "AndroidManifest.xml",
            method: METHOD_DEFLATED,
            crc32: 0,
            uncompressed: manifest.len() as u32 + 1,
            raw: &deflated,
        }],
        Some(&signing_block(&[(V2, v2_block(&[b"cert"]))])),
    );
    assert!(matches!(read(&apk), Err(ApkError::InflatedSize { .. })));
}

#[test]
fn an_encrypted_entry_is_refused_by_name() {
    let mut apk = hand_apk_v2("a.b.c", 1, "1.0.0", b"cert");
    let cd = zip::Archive::open(&apk).expect("opens").central_directory_offset();
    apk[cd + 8] |= 1;
    assert_eq!(read(&apk), Err(ApkError::Encrypted { entry: "AndroidManifest.xml".to_string() }));
}

#[test]
fn a_zip64_marker_is_refused_by_name() {
    let mut apk = hand_apk_v2("a.b.c", 1, "1.0.0", b"cert");
    let eocd = apk.len() - 22;
    apk[eocd + 8..eocd + 12].copy_from_slice(&[0xff; 4]);
    assert_eq!(read(&apk), Err(ApkError::Zip64 { field: "entry count" }));
}

#[test]
fn an_apk_with_no_manifest_is_refused_by_name() {
    let apk = zip(&[Part::stored("classes.dex", b"dex")], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
    assert_eq!(read(&apk), Err(ApkError::MissingEntry { name: "AndroidManifest.xml" }));
}

#[test]
fn an_apk_naming_its_manifest_twice_is_refused_by_name() {
    let manifest = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(
        &[Part::stored("AndroidManifest.xml", &manifest), Part::stored("AndroidManifest.xml", &manifest)],
        Some(&signing_block(&[(V2, v2_block(&[b"c"]))])),
    );
    assert_eq!(read(&apk), Err(ApkError::DuplicateEntry { name: "AndroidManifest.xml" }));
}

#[test]
fn a_file_that_is_not_a_zip_is_refused_by_name() {
    assert_eq!(read(b"not an apk at all, just some text"), Err(ApkError::NoEndOfCentralDirectory));
}

// ---------------------------------------------------------------------------
// The manifest.
// ---------------------------------------------------------------------------

#[test]
fn a_hand_composed_manifest_reads_every_field() {
    let info = read(&hand_apk_v2("com.example.gantry.dev", 17, "0.1.17", b"cert")).expect("reads");
    assert_eq!(info.application_id, "com.example.gantry.dev");
    assert_eq!(info.version_code, 17);
    assert_eq!(info.version_name, "0.1.17");
    assert_eq!(info.label, "Hand Composed");
    assert_eq!((info.min_sdk, info.target_sdk), (26, 34));
    assert_eq!(info.certificate, b"cert");
}

/// **`versionCodeMajor` is the high half of the number Android compares**,
/// and is reported alongside it.
#[test]
fn version_code_major_is_the_high_32_bits_of_the_version_code() {
    let body = manifest(
        &[
            ("package", Attr::Str("a.b.c")),
            ("versionCode", Attr::Int(5)),
            ("versionCodeMajor", Attr::Int(2)),
            ("versionName", Attr::Str("2.0.5")),
        ],
        None,
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
    let info = read(&apk).expect("reads");
    assert_eq!(info.version_code_major, Some(2));
    assert_eq!(info.version_code, (2i64 << 32) | 5);
}

/// Android's defaults: no `minSdkVersion` is 1, and no `targetSdkVersion` is
/// the minimum.
#[test]
fn absent_sdk_levels_take_androids_own_defaults() {
    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        Some(&[("minSdkVersion", Attr::Int(24))]),
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
    let info = read(&apk).expect("reads");
    assert_eq!((info.min_sdk, info.target_sdk), (24, 24));

    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
    let info = read(&apk).expect("reads");
    assert_eq!((info.min_sdk, info.target_sdk), (1, 1));
}

#[test]
fn a_codename_sdk_level_is_refused_by_name() {
    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        Some(&[("minSdkVersion", Attr::Str("Tiramisu"))]),
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
    assert!(matches!(
        read(&apk),
        Err(ApkError::BadAttribute { attribute: "android:minSdkVersion", .. })
    ));
}

/// `(name, value)` pairs for one element's attributes.
type Attrs<'a> = &'a [(&'a str, Attr<'a>)];

#[test]
fn a_manifest_missing_what_apk_json_needs_is_refused_by_name() {
    let cases: [(Attrs<'_>, Attrs<'_>, ApkError); 4] = [
        (
            &[("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1"))],
            &[("label", Attr::Str("L"))],
            ApkError::MissingAttribute { element: "manifest", attribute: "package" },
        ),
        (
            &[("package", Attr::Str("a.b")), ("versionName", Attr::Str("1"))],
            &[("label", Attr::Str("L"))],
            ApkError::MissingAttribute { element: "manifest", attribute: "android:versionCode" },
        ),
        (
            &[("package", Attr::Str("a.b")), ("versionCode", Attr::Int(1))],
            &[("label", Attr::Str("L"))],
            ApkError::MissingAttribute { element: "manifest", attribute: "android:versionName" },
        ),
        (
            &[("package", Attr::Str("a.b")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1"))],
            &[],
            ApkError::MissingAttribute { element: "application", attribute: "android:label" },
        ),
    ];
    for (manifest_attrs, application, expected) in cases {
        let body = manifest(manifest_attrs, None, application);
        let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
        assert_eq!(read(&apk), Err(expected));
    }
}

/// A label that is a reference, and no resource table to resolve it through.
#[test]
fn a_label_reference_with_no_resource_table_is_refused_by_name() {
    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Ref(0x7f0e_0001))],
    );
    let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], Some(&signing_block(&[(V2, v2_block(&[b"c"]))])));
    assert_eq!(read(&apk), Err(ApkError::MissingEntry { name: "resources.arsc" }));
}

#[test]
fn a_label_reference_resolves_through_a_hand_composed_table() {
    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Ref(0x7f0e_0003))],
    );
    let table = resource_table(0x7f0e_0003, "Gantry");
    let apk = zip(
        &[Part::stored("AndroidManifest.xml", &body), Part::stored("resources.arsc", &table)],
        Some(&signing_block(&[(V2, v2_block(&[b"c"]))])),
    );
    assert_eq!(read(&apk).expect("reads").label, "Gantry");

    // An id the table does not define is refused, naming it.
    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Ref(0x7f0e_0009))],
    );
    let apk = zip(
        &[Part::stored("AndroidManifest.xml", &body), Part::stored("resources.arsc", &table)],
        Some(&signing_block(&[(V2, v2_block(&[b"c"]))])),
    );
    assert!(matches!(read(&apk), Err(ApkError::UnresolvedReference { resource: 0x7f0e_0009, .. })));
}

// ---------------------------------------------------------------------------
// The signing block.
// ---------------------------------------------------------------------------

#[test]
fn an_apk_with_no_signing_block_is_refused_by_name() {
    let body = manifest(
        &[("package", Attr::Str("a.b.c")), ("versionCode", Attr::Int(1)), ("versionName", Attr::Str("1.0.0"))],
        None,
        &[("label", Attr::Str("L"))],
    );
    let apk = zip(&[Part::stored("AndroidManifest.xml", &body)], None);
    assert_eq!(read(&apk), Err(ApkError::NoSigningBlock));
}

#[test]
fn a_signing_block_with_no_signer_is_refused_by_name() {
    let apk = hand_apk("a.b.c", 1, "1.0.0", &signing_block(&[(0x4272_6577, vec![0; 16])]));
    assert_eq!(read(&apk), Err(ApkError::NoSigner));
}

#[test]
fn a_v2_block_with_two_signers_is_refused_by_name() {
    let apk = hand_apk("a.b.c", 1, "1.0.0", &signing_block(&[(V2, v2_block(&[b"one", b"two"]))]));
    assert_eq!(read(&apk), Err(ApkError::SignerCount { scheme: Scheme::V2, count: 2 }));
}

#[test]
fn signing_block_sizes_that_disagree_are_refused_by_name() {
    let mut block = signing_block(&[(V2, v2_block(&[b"c"]))]);
    block[0] ^= 1;
    let apk = hand_apk("a.b.c", 1, "1.0.0", &block);
    assert!(matches!(read(&apk), Err(ApkError::Malformed { structure: "APK Signing Block", .. })));
}

/// **The rotation lineage, from a hand-composed v3 block**: oldest first,
/// ending at the signer.
#[test]
fn a_two_certificate_lineage_is_read_from_a_hand_composed_v3_block() {
    let block = signing_block(&[(
        V3,
        v3_block(&[V3Signer {
            min_sdk: 28,
            max_sdk: 0x7fff_ffff,
            certificate: b"new key",
            lineage: Some(vec![b"old key", b"new key"]),
        }]),
    )]);
    let info = read(&hand_apk("a.b.c", 2, "1.0.1", &block)).expect("reads");
    assert_eq!(info.scheme, Scheme::V3);
    assert_eq!(info.certificate, b"new key");
    assert_eq!(info.lineage, vec![b"old key".to_vec(), b"new key".to_vec()]);
    assert_eq!(info.lineage_sha256(), vec![sha256_hex(b"old key"), sha256_hex(b"new key")]);
}

#[test]
fn a_lineage_that_does_not_end_at_the_signer_is_refused_by_name() {
    let block = signing_block(&[(
        V3,
        v3_block(&[V3Signer {
            min_sdk: 28,
            max_sdk: 0x7fff_ffff,
            certificate: b"new key",
            lineage: Some(vec![b"old key", b"another key"]),
        }]),
    )]);
    assert_eq!(
        read(&hand_apk("a.b.c", 2, "1.0.1", &block)),
        Err(ApkError::LineageDoesNotEndAtSigner {
            signer_sha256: sha256_hex(b"new key"),
            last_sha256: sha256_hex(b"another key"),
        })
    );
}

#[test]
fn a_lineage_of_an_unknown_version_is_refused_by_name() {
    let attribute = lp(&[0x3ba0_6f8cu32.to_le_bytes().to_vec(), lineage_value(&[b"k"], 2)].concat());
    let signed = [lp(b""), lp(&lp(b"k")), 28u32.to_le_bytes().to_vec(), 99u32.to_le_bytes().to_vec(), lp(&attribute)].concat();
    let signer = [lp(&signed), 28u32.to_le_bytes().to_vec(), 99u32.to_le_bytes().to_vec(), lp(b""), lp(b"")].concat();
    let block = signing_block(&[(V3, lp(&lp(&signer)))]);
    assert!(matches!(
        read(&hand_apk("a.b.c", 1, "1", &block)),
        Err(ApkError::Malformed { structure: "v3 proof-of-rotation lineage", .. })
    ));
}

/// **v3.1 over v3 over v2**: a block carrying all three is read from v3.1,
/// and one carrying v3 and v2 from v3.
#[test]
fn the_newest_scheme_present_names_the_current_certificate() {
    let v31 = v3_block(&[V3Signer { min_sdk: 33, max_sdk: 0x7fff_ffff, certificate: b"v3.1 key", lineage: None }]);
    let v3 = v3_block(&[V3Signer { min_sdk: 24, max_sdk: 32, certificate: b"v3 key", lineage: None }]);
    let v2 = v2_block(&[b"v2 key"]);

    let all = signing_block(&[(V2, v2.clone()), (V3, v3.clone()), (V31, v31)]);
    let info = read(&hand_apk("a.b.c", 1, "1", &all)).expect("reads");
    assert_eq!((info.scheme, info.certificate.as_slice()), (Scheme::V31, &b"v3.1 key"[..]));

    let two = signing_block(&[(V2, v2), (V3, v3)]);
    let info = read(&hand_apk("a.b.c", 1, "1", &two)).expect("reads");
    assert_eq!((info.scheme, info.certificate.as_slice()), (Scheme::V3, &b"v3 key"[..]));
}

/// Within one v3 block, the signer whose SDK range reaches highest is current.
#[test]
fn the_v3_signer_reaching_the_highest_sdk_is_current() {
    let block = signing_block(&[(
        V3,
        v3_block(&[
            V3Signer { min_sdk: 24, max_sdk: 32, certificate: b"older phones", lineage: None },
            V3Signer { min_sdk: 33, max_sdk: 0x7fff_ffff, certificate: b"newer phones", lineage: None },
        ]),
    )]);
    let info = read(&hand_apk("a.b.c", 1, "1", &block)).expect("reads");
    assert_eq!(info.certificate, b"newer phones");
}

// ---------------------------------------------------------------------------
// A real APK, for the lead: `RIVET_APK_FIXTURE=<path> cargo test -p
// cargo-rivet apk_read -- --ignored --nocapture`.
// ---------------------------------------------------------------------------

/// Read the whole APK named by `RIVET_APK_FIXTURE` and print what was found,
/// in `aapt2 dump badging`'s and `apksigner`'s terms for comparison.
#[test]
#[ignore = "needs RIVET_APK_FIXTURE naming a real APK"]
fn a_real_apk_named_by_the_environment_reads() {
    let path = std::env::var_os("RIVET_APK_FIXTURE").expect("RIVET_APK_FIXTURE names an APK");
    let bytes = std::fs::read(&path).expect("the APK reads");
    let archive = zip::Archive::open(&bytes).expect("opens");
    let manifest = axml::read_manifest(&archive.read("AndroidManifest.xml").expect("inflates"))
        .expect("parses");
    println!("manifest label: {}; versionName: {}", manifest.label, manifest.version_name);
    let info = read(&bytes).expect("the APK parses");
    println!("package: name='{}' versionCode='{}' versionName='{}'", info.application_id, info.version_code, info.version_name);
    println!("versionCodeMajor: {:?}", info.version_code_major);
    println!("minSdkVersion:'{}'", info.min_sdk);
    println!("targetSdkVersion:'{}'", info.target_sdk);
    println!("application-label:'{}'", info.label);
    println!("signer ({}) certificate SHA-256 digest: {}", info.scheme, info.signer_sha256());
    for (i, sha) in info.lineage_sha256().iter().enumerate() {
        println!("lineage #{i}: {sha}");
    }
}

/// Write a small fixture derived from the real APK named by
/// `RIVET_APK_FIXTURE` to `RIVET_APK_FIXTURE_OUT`: its own
/// `AndroidManifest.xml` and `resources.arsc` entries, verbatim, in a new
/// **unsigned** zip — the input `apksigner sign` turns into a committed
/// fixture with a throwaway key (see the README), so the build's own signing
/// certificate is never committed. With `RIVET_APK_FIXTURE_DEFLATE_ARSC` set,
/// `resources.arsc` is deflated (the same bytes, compressed) to keep the
/// fixture small.
///
/// Before writing, the new zip is checked by reading it with the original's
/// signing block put back in: it must read to exactly what the original does.
#[test]
#[ignore = "needs RIVET_APK_FIXTURE and RIVET_APK_FIXTURE_OUT"]
fn derive_a_fixture_from_a_real_apk() {
    let path = std::env::var_os("RIVET_APK_FIXTURE").expect("RIVET_APK_FIXTURE names an APK");
    let out = std::env::var_os("RIVET_APK_FIXTURE_OUT").expect("RIVET_APK_FIXTURE_OUT names the output");
    let deflate_arsc = std::env::var_os("RIVET_APK_FIXTURE_DEFLATE_ARSC").is_some();
    let bytes = std::fs::read(&path).expect("the APK reads");
    let archive = zip::Archive::open(&bytes).expect("opens");
    let manifest = archive.find("AndroidManifest.xml").expect("has one").clone();
    let resources = archive.find("resources.arsc").expect("has one").clone();
    let manifest_raw = archive.raw(&manifest).expect("raw");
    let resources_raw = archive.raw(&resources).expect("raw");
    let (resources_method, resources_bytes) = if deflate_arsc && resources.method == METHOD_STORED {
        (METHOD_DEFLATED, miniz_oxide::deflate::compress_to_vec(resources_raw, 10))
    } else {
        (resources.method, resources_raw.to_vec())
    };
    let parts = [
        Part {
            name: "AndroidManifest.xml",
            method: manifest.method,
            crc32: manifest.crc32,
            uncompressed: manifest.uncompressed as u32,
            raw: manifest_raw,
        },
        Part {
            name: "resources.arsc",
            method: resources_method,
            crc32: resources.crc32,
            uncompressed: resources.uncompressed as u32,
            raw: &resources_bytes,
        },
    ];
    let checked = zip(&parts, Some(&signing_block_of(&bytes)));
    assert_eq!(read(&checked).expect("the derived zip reads"), read(&bytes).expect("the original reads"));
    let derived = zip(&parts, None);
    std::fs::write(&out, &derived).expect("the fixture is written");
    println!("wrote {} bytes", derived.len());
}

/// Write the original's manifest alone, stored, in an UNSIGNED zip at
/// `RIVET_APK_FIXTURE_OUT` — the input `apksigner sign` turns into
/// `rotated-v3.apk` (see the README).
#[test]
#[ignore = "needs RIVET_APK_FIXTURE and RIVET_APK_FIXTURE_OUT"]
fn derive_an_unsigned_manifest_only_zip_from_a_real_apk() {
    let path = std::env::var_os("RIVET_APK_FIXTURE").expect("RIVET_APK_FIXTURE names an APK");
    let out = std::env::var_os("RIVET_APK_FIXTURE_OUT").expect("RIVET_APK_FIXTURE_OUT names the output");
    let bytes = std::fs::read(&path).expect("the APK reads");
    let archive = zip::Archive::open(&bytes).expect("opens");
    let manifest = archive.find("AndroidManifest.xml").expect("has one").clone();
    let raw = archive.raw(&manifest).expect("raw");
    let unsigned = zip(
        &[Part {
            name: "AndroidManifest.xml",
            method: manifest.method,
            crc32: manifest.crc32,
            uncompressed: manifest.uncompressed as u32,
            raw,
        }],
        None,
    );
    std::fs::write(&out, &unsigned).expect("written");
    println!("wrote {} bytes", unsigned.len());
}
