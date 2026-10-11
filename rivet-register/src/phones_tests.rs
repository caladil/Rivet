// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::store;
use crate::{DEVICES_DIR, Device, Register};

/// A real directory under the OS temporary directory, removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rivet-register-phones-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A hand-composed identity: 43 characters of the alphabet, nothing hashed.
fn identity(fill: char) -> String {
    std::iter::repeat_n(fill, IDENTITY_CHARS).collect()
}

/// A hand-composed phone. The key bytes are arbitrary: this crate stores a
/// key, and whether it is a P-256 key is the enrolling program's check.
fn phone(fill: char, label: &str) -> Phone {
    Phone::new(&identity(fill), label, vec![0x30, 0x59, 1, 2, 3], "2000-01-02T03:04:05Z")
        .expect("a valid hand-composed phone")
}

#[test]
fn a_phone_written_is_the_phone_read_back() {
    let scratch = Scratch::new("roundtrip");
    let written = phone('A', "Pixel 8");
    let path = write(&scratch.0, &written).expect("writable");
    assert_eq!(path, scratch.0.join(PHONES_DIR).join(format!("{}.toml", identity('A'))));
    assert_eq!(read(&scratch.0, &identity('A')).expect("readable"), written);
    assert_eq!(list(&scratch.0).expect("listable"), vec![written]);
}

/// The file's text, pinned: fields in the table's order and the key as
/// standard padded base64.
#[test]
fn a_phone_file_holds_the_four_fields_in_order_with_the_key_as_padded_base64() {
    let scratch = Scratch::new("text");
    let path = write(&scratch.0, &phone('A', "Pixel 8")).expect("writable");
    let text = fs::read_to_string(path).expect("readable");
    assert_eq!(
        text,
        format!(
            "identity = \"{}\"\nlabel = \"Pixel 8\"\npublic_key = \"MFkBAgM=\"\nenrolled = \
             \"2000-01-02T03:04:05Z\"\n",
            identity('A')
        )
    );
}

#[test]
fn a_list_is_sorted_by_identity_whatever_order_it_was_written_in() {
    let scratch = Scratch::new("sorted");
    for fill in ['z', 'A', '-', 'm'] {
        write(&scratch.0, &phone(fill, "p")).expect("writable");
    }
    let identities: Vec<String> =
        list(&scratch.0).expect("listable").into_iter().map(|p| p.identity).collect();
    let mut sorted = identities.clone();
    sorted.sort();
    assert_eq!(identities, sorted);
    assert_eq!(identities.len(), 4);
}

#[test]
fn writing_again_replaces_the_record_and_leaves_one_file() {
    let scratch = Scratch::new("replace");
    write(&scratch.0, &phone('A', "old")).expect("writable");
    write(&scratch.0, &phone('A', "new")).expect("writable");
    let all = list(&scratch.0).expect("listable");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].label, "new");
    let files: Vec<_> = fs::read_dir(scratch.0.join(PHONES_DIR))
        .expect("the directory")
        .map(|e| e.expect("an entry").file_name())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
}

#[test]
fn a_register_with_no_phones_directory_has_no_phones() {
    let scratch = Scratch::new("absent");
    assert!(list(&scratch.0).expect("an absent directory is no phones").is_empty());
    assert!(list(&scratch.0.join("never-created")).expect("nor an absent register").is_empty());
}

/// **The compatibility claim, on bytes this crate did not write.** A register
/// written before phones existed — hand-composed text — loads, holds no phones,
/// and re-saves byte for byte without growing a `phones/` directory.
#[test]
fn a_register_from_before_phones_loads_unchanged_and_resaves_byte_identical() {
    let scratch = Scratch::new("before");
    let fleet_text = "name = \"home\"\n";
    let device_text = "id = \"0011223344556677\"\nname = \"kitchen-pi\"\n";
    fs::write(scratch.0.join("register.toml"), fleet_text).expect("writable");
    fs::create_dir_all(scratch.0.join(DEVICES_DIR)).expect("creatable");
    let device_path = scratch.0.join(DEVICES_DIR).join("0011223344556677.toml");
    fs::write(&device_path, device_text).expect("writable");

    let register = store::load(&scratch.0).expect("an old register loads");
    assert_eq!(register.devices.len(), 1);
    assert!(list(&scratch.0).expect("listable").is_empty());

    store::save(&register).expect("writable");
    assert_eq!(fs::read_to_string(scratch.0.join("register.toml")).expect("readable"), fleet_text);
    assert_eq!(fs::read_to_string(&device_path).expect("readable"), device_text);
    assert!(!scratch.0.join(PHONES_DIR).exists(), "a save grew a phones directory");
}

/// A workstation register never writes `phones/`, and a register holding
/// phones keeps them through a save of its devices: the two never touch.
#[test]
fn saving_the_register_neither_creates_nor_disturbs_phone_files() {
    let scratch = Scratch::new("disjoint");
    let mut register = Register::empty(&scratch.0);
    register
        .enroll(Device {
            id: "0011223344556677".to_owned(),
            name: "kitchen-pi".to_owned(),
            ..Device::default()
        })
        .expect("fresh");
    let saved = store::save(&register).expect("writable");
    assert!(saved.iter().all(|p| !p.starts_with(scratch.0.join(PHONES_DIR))));
    assert!(!scratch.0.join(PHONES_DIR).exists());

    let path = write(&scratch.0, &phone('A', "Pixel 8")).expect("writable");
    let before = fs::read(&path).expect("readable");
    store::save(&store::load(&scratch.0).expect("readable")).expect("writable");
    assert_eq!(fs::read(&path).expect("still there"), before);
}

#[test]
fn an_identity_that_is_not_43_base64url_characters_is_refused() {
    let short: String = identity('A').chars().skip(1).collect();
    let long = format!("{}A", identity('A'));
    let dotted = format!("..{}", "A".repeat(IDENTITY_CHARS - 2));
    let slashed = format!("A/{}", "A".repeat(IDENTITY_CHARS - 2));
    let padded = format!("{}=", "A".repeat(IDENTITY_CHARS - 1));
    let plus = format!("{}+", "A".repeat(IDENTITY_CHARS - 1));
    for bad in [String::new(), short, long, dotted, slashed, padded, plus] {
        assert!(!is_identity(&bad), "{bad:?} was accepted");
        let error = Phone::new(&bad, "p", vec![1], "2000-01-02T03:04:05Z")
            .expect_err("refused");
        assert!(matches!(error, PhoneError::BadIdentity { .. }), "{error:?}");
        let scratch = Scratch::new("bad-identity");
        assert!(matches!(read(&scratch.0, &bad), Err(PhoneError::BadIdentity { .. })));
        assert!(matches!(remove(&scratch.0, &bad), Err(PhoneError::BadIdentity { .. })));
        assert!(record_path(&scratch.0, &bad).is_none());
    }
    assert!(is_identity(&identity('-')));
    assert!(is_identity(&identity('_')));
}

#[test]
fn a_label_that_is_empty_too_long_or_holds_a_control_character_is_refused() {
    let longest = "é".repeat(LABEL_MAX_CHARS);
    check_label(&longest).expect("64 characters is the limit, and they need not be ASCII");
    for bad in [
        String::new(),
        "x".repeat(LABEL_MAX_CHARS + 1),
        "Pixel\n8".to_owned(),
        "Pixel\t8".to_owned(),
        "Pixel\u{7f}".to_owned(),
        "Pixel\u{85}".to_owned(),
    ] {
        let error = check_label(&bad).expect_err("refused");
        assert!(matches!(error, PhoneError::BadLabel { .. }), "{bad:?}: {error:?}");
        let scratch = Scratch::new("bad-label");
        let refused = Phone {
            label: bad.clone(),
            ..phone('A', "ok")
        };
        assert!(matches!(write(&scratch.0, &refused), Err(PhoneError::BadLabel { .. })));
        assert!(!scratch.0.join(PHONES_DIR).exists(), "{bad:?} reached the disk");
    }
}

#[test]
fn an_enrollment_time_that_is_not_rfc3339_utc_to_the_second_is_refused() {
    for bad in [
        "2000-01-02T03:04:05",
        "2000-01-02T03:04:05.000Z",
        "2000-01-02 03:04:05Z",
        "2000-01-02T03:04:05+00:00",
    ] {
        let error = Phone::new(&identity('A'), "p", vec![1], bad).expect_err("refused");
        assert!(matches!(error, PhoneError::BadEnrolled { .. }), "{bad:?}");
    }
}

#[test]
fn removing_a_phone_removes_its_file_and_no_other() {
    let scratch = Scratch::new("remove");
    write(&scratch.0, &phone('A', "a")).expect("writable");
    write(&scratch.0, &phone('B', "b")).expect("writable");
    remove(&scratch.0, &identity('A')).expect("removable");
    let left: Vec<String> =
        list(&scratch.0).expect("listable").into_iter().map(|p| p.identity).collect();
    assert_eq!(left, vec![identity('B')]);
}

#[test]
fn removing_a_phone_that_is_not_enrolled_is_a_named_error() {
    let scratch = Scratch::new("remove-missing");
    let error = remove(&scratch.0, &identity('A')).expect_err("nothing to remove");
    assert_eq!(error, PhoneError::NoSuchPhone { identity: identity('A') });
    assert!(error.to_string().contains(&identity('A')), "{error}");
    let error = read(&scratch.0, &identity('A')).expect_err("nothing to read");
    assert_eq!(error, PhoneError::NoSuchPhone { identity: identity('A') });
}

#[test]
fn a_record_renamed_by_hand_is_refused_when_listed() {
    let scratch = Scratch::new("misfiled");
    let path = write(&scratch.0, &phone('A', "a")).expect("writable");
    fs::rename(&path, scratch.0.join(PHONES_DIR).join(format!("{}.toml", identity('B'))))
        .expect("renamable");
    let error = list(&scratch.0).expect_err("misfiled");
    assert!(matches!(error, PhoneError::Misfiled { .. }), "{error:?}");
}

#[test]
fn a_file_that_is_not_a_phone_record_is_refused_and_a_non_record_file_is_left_alone() {
    let scratch = Scratch::new("stranger");
    write(&scratch.0, &phone('A', "a")).expect("writable");
    fs::write(scratch.0.join(PHONES_DIR).join("README.md"), "notes\n").expect("writable");
    assert_eq!(list(&scratch.0).expect("a README is not a record").len(), 1);

    fs::write(
        scratch.0.join(PHONES_DIR).join(format!("{}.toml", identity('B'))),
        format!(
            "identity = \"{}\"\nlabel = \"b\"\npublic_key = \"not base64!\"\nenrolled = \
             \"2000-01-02T03:04:05Z\"\n",
            identity('B')
        ),
    )
    .expect("writable");
    let error = list(&scratch.0).expect_err("a key that is not base64");
    assert!(matches!(error, PhoneError::Malformed { .. }), "{error:?}");
}

/// The identity encoding against a vector this crate did not compute: the
/// SHA-256 of the empty string, whose base64url is published widely as
/// `47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU`.
#[test]
fn an_identity_is_the_unpadded_base64url_of_the_digest() {
    let empty_sha256: [u8; 32] = [
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9,
        0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
        0xb8, 0x55,
    ];
    let id = identity_from_sha256(&empty_sha256);
    assert_eq!(id, "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU");
    assert!(is_identity(&id));
}
