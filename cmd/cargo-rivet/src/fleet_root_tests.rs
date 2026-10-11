//! `cargo rivet fleet root`'s tests, against a root minted by
//! `rivet-fleet-root` exactly as a board mints one, and answers composed in
//! the shape `rivet get fleet-root` answers. Self-generated.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn minted() -> rivet_fleet_root::Minted {
    let request = rivet_core::fleet_root::Request::new(
        "dev-fleet",
        "dev-01",
        &rivet_core::fleet_root::default_suffixes("dev-fleet"),
    )
    .expect("legal");
    let mut n = 3u8;
    let mut random = |len: usize| -> Result<Vec<u8>, String> {
        Ok((0..len)
            .map(|_| {
                n = n.wrapping_add(29);
                n
            })
            .collect())
    };
    rivet_fleet_root::mint(&request, &mut random, 0).expect("mints")
}

fn answer(pem: &str, sha256: &str) -> String {
    let escaped = pem.replace('\n', "\\n");
    format!(
        "{{\"subject\":\"fleet-root\",\"path\":\"/state/secrets/fleet-root.crt\",\"present\":true,\
         \"certificate\":\"{escaped}\",\"sha256\":\"{sha256}\",\"means\":\"x\"}}"
    )
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rivet-fleet-root-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// **What is saved is what was minted**, byte for byte in both encodings,
/// named for the fleet, with the fingerprint a trust store shows.
#[test]
fn the_fetched_root_is_saved_exactly_as_minted() {
    let minted = minted();
    let sha = rivet_fleet_root::fingerprint(&minted.root.certificate_der);
    let fetched = read_answer(&answer(&minted.root.certificate, &sha)).expect("reads");
    assert_eq!(fetched.der, minted.root.certificate_der);
    assert_eq!(fetched.pem, minted.root.certificate);

    let dir = scratch("save");
    let said = save(&fetched, &dir, "dev-01").expect("saved");
    assert_eq!(std::fs::read(dir.join("dev-fleet-root.cer")).unwrap(), minted.root.certificate_der);
    assert_eq!(std::fs::read_to_string(dir.join("dev-fleet-root.crt")).unwrap(), minted.root.certificate);
    assert!(said.contains(&colon_hex(&sha)), "{said}");
    assert!(said.contains("certutil -user -addstore Root"), "{said}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A fingerprint that does not match the certificate is refused before
/// anything is written.
#[test]
fn a_fingerprint_that_does_not_match_is_refused() {
    let minted = minted();
    let why = read_answer(&answer(&minted.root.certificate, &"0".repeat(64))).expect_err("refused");
    assert!(why.contains("NOTHING WAS SAVED"), "{why}");
}

#[test]
fn a_machine_with_no_root_is_named_as_such() {
    let why = read_answer("{\"subject\":\"fleet-root\",\"present\":false}").expect_err("refused");
    assert!(why.contains("holds no fleet root"), "{why}");
}

/// **With all three connection flags, the register is never consulted** —
/// `cargo rivet run --host …`'s shape — so a machine no register holds can
/// still be asked for its root. The positional machine names the files only.
#[test]
fn every_connection_flag_given_skips_the_register() {
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let parsed = parse(&words(&[
        "root",
        "dev-01",
        "--host",
        "dev-01.local:7777",
        "--identity",
        "operator",
        "--pin",
        &"ab".repeat(32),
        "--out",
        "certs",
    ]))
    .expect("parses");
    // An empty register: consulting it refuses, exactly as a real one that
    // does not hold the machine does.
    let register = scratch("empty-register");
    std::fs::create_dir_all(&register).expect("a directory");
    let empty = register.clone();
    let endpoint = endpoint_for(&parsed, &move || Ok(empty.clone())).expect("no register lookup");
    assert_eq!(endpoint.host.as_deref(), Some("dev-01.local:7777"));
    assert_eq!(endpoint.identity, Some(PathBuf::from("operator")));
    assert_eq!(endpoint.pin, Some("ab".repeat(32)));

    // Without them the register is still what names the machine, and an
    // empty one says so.
    let bare = parse(&words(&["root", "dev-01"])).expect("parses");
    let empty = register.clone();
    let why = endpoint_for(&bare, &move || Ok(empty.clone())).expect_err("the register holds nothing");
    assert!(why.contains("answers to no device"), "{why}");
    let _ = std::fs::remove_dir_all(register);
}

#[test]
fn the_verb_takes_one_machine_and_an_output_directory() {
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let parsed = parse(&words(&["root", "dev-01", "--out", "certs"])).expect("parses");
    assert_eq!(parsed.host, "dev-01");
    assert_eq!(parsed.out, PathBuf::from("certs"));
    assert!(parse(&words(&["root"])).is_err());
    assert!(parse(&words(&["root", "a", "b"])).is_err());
    assert!(parse(&words(&["root", "a", "--bogus"])).is_err());
}
