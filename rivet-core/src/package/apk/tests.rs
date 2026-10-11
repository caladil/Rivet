//! `apk`, from the outside in — `ADR-061` §3.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::package::port::{Digest, MaySign, TrustSet};
use crate::package::testing::{ToyDigest, ToyKey, ToySigner, ToyVerifier};
use crate::package::write::{Builder, NewMember};

const TRUSTED: ToyKey = ToyKey(0x11);

fn trusting(kinds: &[Kind]) -> TrustSet {
    let mut trust = TrustSet::new();
    trust.trust(&mut ToyDigest::new(), TRUSTED.public(), "test-apps", MaySign::these(kinds));
    trust
}

fn well_formed_json() -> String {
    "{\"application_id\":\"com.example.gantry\",\"version_code\":17,\
     \"version_name\":\"0.1.17\",\"label\":\"Gantry\",\"min_sdk\":21,\
     \"target_sdk\":34,\"signer_sha256\":\"".to_string()
        + &"ab".repeat(32)
        + "\",\"development\":true}"
}

// ---------------------------------------------------------------------------
// Metadata::parse — ADR-061 §3.
// ---------------------------------------------------------------------------

#[test]
fn a_well_formed_apk_json_is_read_back() {
    let metadata = Metadata::parse(well_formed_json().as_bytes()).expect("well-formed metadata");
    assert_eq!(metadata.application_id(), "com.example.gantry");
    assert_eq!(metadata.version_code(), 17);
    assert_eq!(metadata.version_name(), "0.1.17");
    assert_eq!(metadata.label(), "Gantry");
    assert_eq!(metadata.min_sdk(), 21);
    assert_eq!(metadata.target_sdk(), 34);
    assert_eq!(metadata.signer_sha256(), "ab".repeat(32));
    assert!(metadata.development());
}

#[test]
fn apk_json_that_is_not_utf8_is_refused() {
    assert_eq!(Metadata::parse(&[0xff, 0xfe, 0xfd]), Err(MetadataFault::NotText));
}

#[test]
fn apk_json_that_is_not_json_is_refused() {
    assert_eq!(Metadata::parse(b"not json at all"), Err(MetadataFault::NotJson));
}

#[test]
fn apk_json_that_is_not_an_object_is_refused() {
    assert_eq!(Metadata::parse(b"[1, 2, 3]"), Err(MetadataFault::NotObject));
}

#[test]
fn a_key_this_build_does_not_define_is_refused_by_name() {
    let with_extra = well_formed_json().replace('}', ",\"extra\":1}");
    assert_eq!(
        Metadata::parse(with_extra.as_bytes()),
        Err(MetadataFault::UnknownKey {
            key: "extra".to_string()
        })
    );
}

#[test]
fn each_missing_key_is_refused_by_name() {
    for key in KEYS {
        // Build an object missing exactly this key, by parsing the whole
        // thing back and only ever removing one field's text — the simplest
        // hand-composed way to knock one out without a JSON writer, and it
        // is fine here because MetadataFault::MissingKey is what is under
        // test, not the surrounding syntax.
        let full = well_formed_json();
        let json = crate::json::parse(&full).expect("the fixture itself parses");
        let Json::Object(fields) = json else {
            panic!("fixture is not an object");
        };
        let mut rebuilt = String::from("{");
        let mut first = true;
        for (k, v) in &fields {
            if k == key {
                continue;
            }
            if !first {
                rebuilt.push(',');
            }
            first = false;
            rebuilt.push_str(&format!("{k:?}:"));
            rebuilt.push_str(&render(v));
        }
        rebuilt.push('}');
        assert_eq!(
            Metadata::parse(rebuilt.as_bytes()),
            Err(MetadataFault::MissingKey { key }),
            "removing {key:?} did not refuse as missing"
        );
    }
}

#[test]
fn version_code_and_the_two_sdk_levels_refuse_a_negative_number() {
    for key in ["version_code", "min_sdk", "target_sdk"] {
        let bad = well_formed_json().replacen(&format!("\"{key}\":"), &format!("\"{key}\":-"), 1);
        // The replacement above turns e.g. `"version_code":17` into
        // `"version_code":-17`, which is negative and must be refused.
        assert_eq!(
            Metadata::parse(bad.as_bytes()),
            Err(MetadataFault::BadValue { key: leak(key) }),
            "{key} accepted a negative number"
        );
    }
}

#[test]
fn a_signer_hash_that_is_not_64_lowercase_hex_characters_is_refused() {
    for bad in ["AB".repeat(32), "ab".repeat(31), String::new(), "zz".repeat(32)] {
        let doc = well_formed_json().replace(&"ab".repeat(32), &bad);
        assert_eq!(
            Metadata::parse(doc.as_bytes()),
            Err(MetadataFault::BadValue {
                key: "signer_sha256"
            }),
            "{bad:?} was accepted as a signer hash"
        );
    }
}

#[test]
fn development_must_be_a_boolean() {
    let bad = well_formed_json().replace("true", "\"true\"");
    assert_eq!(
        Metadata::parse(bad.as_bytes()),
        Err(MetadataFault::BadValue { key: "development" })
    );
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// A minimal re-renderer for [`each_missing_key_is_refused_by_name`]'s
/// reconstruction — only as rich as the fixture's own values need.
fn render(value: &Json) -> String {
    match value {
        Json::Str(s) => format!("{s:?}"),
        Json::Integer(n) => n.to_string(),
        Json::Bool(b) => b.to_string(),
        other => panic!("the fixture does not use {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Apk::open — the container, ADR-061 §3.
// ---------------------------------------------------------------------------

fn built(platform: Option<&str>, arch: &str, members: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(TRUSTED);
    let mut builder =
        Builder::new("gantry", "0.1.17", arch, platform, Kind::apk()).expect("a builder");
    for (name, bytes) in members {
        builder
            .add(NewMember::stored(name, 0, 0, bytes, &mut digest).expect("a member"))
            .expect("added");
    }
    let slices: Vec<&[u8]> = members.iter().map(|(_, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .expect("sealed")
}

fn ordinary_members() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (APK_NAME, b"pretend-this-is-a-zip".to_vec()),
        (METADATA_NAME, well_formed_json().into_bytes()),
    ]
}

/// Open the bytes through the real verifier — `read::open` — and then this
/// module's own `Apk::open`, exactly as an installer or an inspector would.
fn opened(bytes: &[u8]) -> Result<Package, PackageError> {
    crate::package::read::open(
        bytes,
        bytes.len() as u64,
        &mut ToyDigest::new(),
        &ToyVerifier::new(),
        &trusting(&[Kind::apk()]),
    )
}

fn open_bytes(bytes: &[u8]) -> Result<(), PackageError> {
    let package = opened(bytes)?;
    Apk::open(&package).map(|_| ())
}

#[test]
fn a_well_formed_apk_artifact_opens() {
    let bytes = built(Some("android"), "any", &ordinary_members());
    assert!(open_bytes(&bytes).is_ok(), "{:?}", open_bytes(&bytes));
}

#[test]
fn a_missing_app_apk_is_refused_by_name() {
    let members = vec![(METADATA_NAME, well_formed_json().into_bytes())];
    let bytes = built(Some("android"), "any", &members);
    assert_eq!(
        open_bytes(&bytes),
        Err(PackageError::ApkMemberCount {
            name: APK_NAME,
            found: 0
        })
    );
}

#[test]
fn a_missing_apk_json_is_refused_by_name() {
    let members = vec![(APK_NAME, b"pretend-this-is-a-zip".to_vec())];
    let bytes = built(Some("android"), "any", &members);
    assert_eq!(
        open_bytes(&bytes),
        Err(PackageError::ApkMemberCount {
            name: METADATA_NAME,
            found: 0
        })
    );
}

#[test]
fn a_foreign_member_is_refused_by_name() {
    let mut members = ordinary_members();
    members.push(("payload.dex", b"not a legal member".to_vec()));
    let bytes = built(Some("android"), "any", &members);
    assert_eq!(
        open_bytes(&bytes),
        Err(PackageError::ApkForeignMember {
            name: "payload.dex".to_string()
        })
    );
}

#[test]
fn the_wrong_platform_is_refused_by_name() {
    let bytes = built(Some("rpi4"), "any", &ordinary_members());
    assert_eq!(
        open_bytes(&bytes),
        Err(PackageError::ApkWrongPlatform {
            found: Some("rpi4".to_string())
        })
    );
}

#[test]
fn no_platform_at_all_is_refused() {
    let bytes = built(None, "any", &ordinary_members());
    assert_eq!(
        open_bytes(&bytes),
        Err(PackageError::ApkWrongPlatform { found: None })
    );
}

#[test]
fn the_wrong_arch_is_refused_by_name() {
    let bytes = built(Some("android"), "aarch64", &ordinary_members());
    assert_eq!(
        open_bytes(&bytes),
        Err(PackageError::ApkWrongArch {
            found: "aarch64".to_string()
        })
    );
}

#[test]
fn an_icon_is_optional_and_a_well_formed_artifact_carries_none() {
    let bytes = built(Some("android"), "any", &ordinary_members());
    let package = opened(&bytes).expect("verifies");
    let apk = Apk::open(&package).expect("opens without an icon");
    assert_eq!(apk.icon_member(), None);
}

/// **`app.apk` is stored, never compressed** — `ADR-061` §3. Enforced
/// generically, not by this module: [`Builder::add`] refuses any non-`card`
/// kind's compressed member (`R-177.1`), and this exercises that refusal for
/// `apk` specifically, naming the kind.
#[test]
fn a_compressed_app_apk_is_refused_at_build_time_by_name() {
    let mut digest = ToyDigest::new();
    let mut builder =
        Builder::new("gantry", "0.1.17", "any", Some("android"), Kind::apk()).expect("a builder");
    let stored = b"pretend-this-is-zstd-compressed-bytes";
    let member = NewMember::streamed(
        APK_NAME,
        0,
        0,
        crate::package::read::COMPRESSION_ZSTD,
        digest.of(stored),
        stored.len() as u64,
        stored.len() as u64 * 4,
    )
    .expect("a well-formed member record");
    let refusal = builder
        .add(member)
        .expect_err("apk must never carry a compressed member");
    assert!(
        matches!(
            &refusal,
            PackageError::CompressionNotPermittedForKind { kind, .. } if kind == "apk"
        ),
        "{refusal:?}"
    );
}

#[test]
fn an_icon_is_accepted_when_present() {
    let mut members = ordinary_members();
    members.push((ICON_NAME, b"pretend-this-is-a-png".to_vec()));
    let bytes = built(Some("android"), "any", &members);
    let package = opened(&bytes).expect("verifies");
    let apk = Apk::open(&package).expect("opens with an icon");
    assert!(apk.icon_member().is_some());
}

#[test]
fn read_metadata_reads_back_the_verified_bytes() {
    let bytes = built(Some("android"), "any", &ordinary_members());
    let package = opened(&bytes).expect("verifies");
    let apk = Apk::open(&package).expect("opens");
    let at = apk.metadata_member().index();
    let raw = well_formed_json().into_bytes();
    let verified = package.verify_member(at, &mut ToyDigest::new(), &raw).expect("verifies");
    let metadata = apk.read_metadata(verified, &raw).expect("reads apk.json");
    assert_eq!(metadata.application_id(), "com.example.gantry");
}

#[test]
fn read_metadata_refuses_the_wrong_member() {
    let bytes = built(Some("android"), "any", &ordinary_members());
    let package = opened(&bytes).expect("verifies");
    let apk = Apk::open(&package).expect("opens");
    let wrong_index = apk.apk_member().index();
    let raw = b"pretend-this-is-a-zip".to_vec();
    let verified = package
        .verify_member(wrong_index, &mut ToyDigest::new(), &raw)
        .expect("verifies the apk member");
    let refusal = apk.read_metadata(verified, &raw).expect_err("wrong member verified");
    assert_eq!(
        refusal,
        PackageError::WrongMemberVerified {
            expected: apk.metadata_member().index(),
            found: wrong_index,
        }
    );
}

// ---------------------------------------------------------------------------
// Trust — ADR-028 §5: `apk` joins the application key's mask.
// ---------------------------------------------------------------------------

#[test]
fn an_apk_signed_by_a_key_entitled_to_apk_verifies() {
    let bytes = built(Some("android"), "any", &ordinary_members());
    let trust = trusting(&[Kind::application(), Kind::library(), Kind::apk()]);
    let package = crate::package::read::open(
        &bytes,
        bytes.len() as u64,
        &mut ToyDigest::new(),
        &ToyVerifier::new(),
        &trust,
    )
    .expect("an apk-entitled key verifies its own apk");
    assert_eq!(*package.header().kind(), Kind::apk());
}

/// **The system key's mask never includes `apk`** — `ADR-028` §5, `.rulings/`
/// R-222. A key entitled only to `{update, extension, card}` is refused BY
/// NAME, the same entitlement check every other kind goes through.
#[test]
fn an_apk_signed_by_a_key_not_entitled_to_apk_is_refused_by_name() {
    let bytes = built(Some("android"), "any", &ordinary_members());
    let trust = trusting(&[Kind::update(), Kind::card()]);
    let refusal = crate::package::read::open(
        &bytes,
        bytes.len() as u64,
        &mut ToyDigest::new(),
        &ToyVerifier::new(),
        &trust,
    )
    .expect_err("a system-only key must not authorise an apk");
    let PackageError::KindNotPermitted { label, kind, may_sign, .. } = &refusal else {
        panic!("{refusal:?}");
    };
    assert_eq!(label, "test-apps");
    assert_eq!(kind, "apk");
    assert_eq!(may_sign, &vec!["update".to_string(), "card".to_string()]);
}
