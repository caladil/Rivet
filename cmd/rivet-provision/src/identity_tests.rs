//! `crate::identity`, in scratch directories. Nothing here reads `~/.rivet`
//! or a real register.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let at = std::env::temp_dir().join(format!(
            "rivet-identity-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("a scratch directory");
        Self(at)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fields(hostname: &str, identity: &str) -> Vec<Field> {
    vec![
        Field::text("Hostname", hostname, ""),
        Field::text("Identity", identity, ""),
    ]
}

fn set(fields: &mut [Field], label: &str, value: &str) {
    let field = fields.iter_mut().find(|f| f.label == label).expect("the field");
    field.set(value, crate::checker(label));
}

fn identity_of(fields: &[Field]) -> &Field {
    fields.iter().find(|f| f.label == "Identity").expect("Identity")
}

/// An operator identity in `at`: a certificate only, which is all `mint` reads.
fn operator(at: &Path) -> (PathBuf, String) {
    let dir = at.join("identity").join("operator");
    let minted = crate::stage::mint_self_signed("operator").expect("minted");
    std::fs::create_dir_all(&dir).expect("a directory");
    std::fs::write(dir.join(rivetos_tls::CERTIFICATE_FILE), &minted.certificate_der)
        .expect("written");
    (dir, rivetos_tls::encode_hex(&minted.public))
}

/// A one-record register at `at/register`, the record carrying `pin`.
fn register_with(at: &Path, name: &str, id: &str, pin: &str) -> PathBuf {
    let root = at.join("register");
    let mut register = rivetos_register::Register::empty(&root);
    register
        .enroll(rivetos_register::Device {
            id: id.to_owned(),
            name: name.to_owned(),
            pin: Some(pin.to_owned()),
            ..rivetos_register::Device::default()
        })
        .expect("enrolled");
    rivetos_register::store::save(&register).expect("saved");
    root
}

/// **The default follows the hostname**, and stops following the moment the
/// operator types an Identity of their own.
#[test]
fn the_default_follows_the_hostname_until_an_identity_is_typed() {
    let root = Path::new("scratch-root");
    let mut form = fields("rivet-00001", &directory_for(root, "rivet-00001").display().to_string());

    set(&mut form, "Hostname", "cedar");
    follow_hostname(&mut form, Some(root));
    let expected = directory_for(root, "cedar").display().to_string();
    assert_eq!(identity_of(&form).value, expected);
    assert_eq!(identity_of(&form).default, expected);

    set(&mut form, "Identity", "D:/somewhere/else");
    set(&mut form, "Hostname", "vega");
    follow_hostname(&mut form, Some(root));
    assert_eq!(identity_of(&form).value, "D:/somewhere/else", "a typed Identity moved");
    assert_eq!(identity_of(&form).default, directory_for(root, "vega").display().to_string());
}

/// Following the hostname touches nothing on disk: the directory it names is
/// minted at `w` and not before.
#[test]
fn following_the_hostname_creates_nothing() {
    let scratch = Scratch::new("follow");
    let mut form = fields("rivet-00001", &directory_for(&scratch.0, "rivet-00001").display().to_string());
    set(&mut form, "Hostname", "cedar");
    follow_hostname(&mut form, Some(&scratch.0));
    assert!(!scratch.0.join("identity").exists(), "editing a field created an identity directory");
}

/// **A missing directory is minted whole**: a seed, a certificate, and a peers
/// file naming the operator and nothing else.
#[test]
fn a_missing_directory_is_minted_with_the_operator_in_its_peers() {
    let scratch = Scratch::new("mint");
    let (operator_dir, operator_key) = operator(&scratch.0);
    let dir = directory_for(&scratch.0, "cedar");
    assert!(is_absent(&dir));

    let pin = mint(&dir, &operator_dir).expect("minted");

    assert_eq!(
        std::fs::read(dir.join(rivetos_tls::SEED_FILE)).expect("a seed").len(),
        rivetos_tls::KEY_BYTES
    );
    assert_eq!(crate::stage::device_pin(&dir).expect("a certificate"), pin);
    let peers = std::fs::read_to_string(dir.join(rivetos_tls::PEERS_FILE)).expect("peers");
    assert_eq!(peers, format!("{operator_key}\n"));
}

/// An identity already there is never minted over.
#[test]
fn an_existing_identity_is_not_minted_over() {
    let scratch = Scratch::new("no-overwrite");
    let (operator_dir, _) = operator(&scratch.0);
    let dir = directory_for(&scratch.0, "cedar");
    mint(&dir, &operator_dir).expect("minted");
    let before = std::fs::read(dir.join(rivetos_tls::SEED_FILE)).expect("a seed");
    let err = mint(&dir, &operator_dir).expect_err("refused");
    assert!(err.contains("refuses to overwrite"), "{err}");
    assert_eq!(std::fs::read(dir.join(rivetos_tls::SEED_FILE)).expect("a seed"), before);
}

/// Without an operator certificate nothing is minted: a peers file naming
/// nobody is a machine that refuses its operator.
#[test]
fn no_operator_mints_nothing() {
    let scratch = Scratch::new("no-operator");
    let dir = directory_for(&scratch.0, "cedar");
    let err = mint(&dir, &scratch.0.join("identity").join("operator")).expect_err("refused");
    assert!(err.contains("cargo rivet setup"), "{err}");
    assert!(!dir.exists(), "a refused mint left a directory behind");
}

/// **An identity whose pin is on another hostname's record is refused**, and
/// the refusal names that record's hostname and identifier.
#[test]
fn a_pin_in_the_register_under_another_hostname_is_refused_by_name() {
    let scratch = Scratch::new("collision");
    let (operator_dir, _) = operator(&scratch.0);
    let shared = scratch.0.join("identity").join("board");
    let pin = rivetos_tls::encode_hex(&mint(&shared, &operator_dir).expect("minted"));
    let register = register_with(&scratch.0, "board", "1f0532852afc2842", &pin);

    let err = check(&shared, "rivet-new", &register).expect_err("refused");
    assert!(err.contains("\"board\""), "{err}");
    assert!(err.contains("1f0532852afc2842"), "{err}");
    assert!(err.contains("Pick or mint another identity directory"), "{err}");

    // The same hostname is the same machine, refused elsewhere naming `--existing`.
    check(&shared, "board", &register).expect("the same hostname passes here");
    // A directory that holds no identity is minted, and a fresh key is in no record.
    check(&directory_for(&scratch.0, "rivet-new"), "rivet-new", &register).expect("absent passes");
}
