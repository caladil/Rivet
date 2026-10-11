//! Bookmarks: the file's round trip, the refusals that protect a pin, and a
//! bookmark read from an in-memory register. No real `~/.rivet`, no real
//! register and no network.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!("fleet-bookmark-test-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_owned()).collect()
}

const PIN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn dev() -> Bookmark {
    Bookmark {
        fleet: "dev-fleet".to_owned(),
        server: "dev-01".to_owned(),
        host: "dev-01.local:7777".to_owned(),
        pin: PIN.to_owned(),
        identity: PathBuf::from("identity-dir"),
    }
}

#[test]
fn a_bookmark_written_reads_back_the_same_and_an_absent_file_holds_none() {
    let dir = scratch("round-trip");
    let file = dir.join(FILE);
    assert!(read(&file).expect("an absent file reads").is_empty());

    let made = add(&file, &dev(), false).expect("added");
    assert!(made.starts_with("MADE  dev-fleet"), "{made}");
    assert_eq!(read(&file).expect("reads"), vec![dev()]);
    assert_eq!(find(&file, "dev-fleet").expect("found"), dev());
    let text = std::fs::read_to_string(&file).expect("the file");
    assert!(text.contains("[fleet.dev-fleet]"), "{text}");
    assert!(text.contains("ADR-063 §2"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A pin is a trust decision**: the same bookmark again changes nothing, a
/// different one for the same fleet is refused and changes nothing, and
/// `--replace` writes it.
#[test]
fn a_different_bookmark_for_a_fleet_is_refused_without_replace() {
    let dir = scratch("replace");
    let file = dir.join(FILE);
    add(&file, &dev(), false).expect("added");
    let again = add(&file, &dev(), false).expect("the same again");
    assert!(again.starts_with("have"), "{again}");

    let mut moved = dev();
    moved.pin = "f".repeat(64);
    let refused = add(&file, &moved, false).expect_err("a different pin");
    assert!(refused.contains("--replace"), "{refused}");
    assert_eq!(read(&file).expect("reads"), vec![dev()], "a refusal changed the file");

    add(&file, &moved, true).expect("replaced");
    assert_eq!(read(&file).expect("reads"), vec![moved]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_fleet_sits_beside_the_first_and_remove_takes_only_its_own() {
    let dir = scratch("two");
    let file = dir.join(FILE);
    add(&file, &dev(), false).expect("added");
    let mut prod = dev();
    prod.fleet = "home-fleet".to_owned();
    prod.server = "rivet-01".to_owned();
    prod.host = "192.168.50.10:7777".to_owned();
    add(&file, &prod, false).expect("added");
    assert_eq!(read(&file).expect("reads").len(), 2);

    let removed = remove(&file, "home-fleet").expect("removed");
    assert!(removed.contains("REMOVED  home-fleet"), "{removed}");
    assert_eq!(read(&file).expect("reads"), vec![dev()]);
    let missing = find(&file, "home-fleet").expect_err("gone");
    assert!(missing.contains("it has: dev-fleet"), "{missing}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bookmark_missing_a_field_or_misnamed_is_refused_whole() {
    let dir = scratch("malformed");
    let file = dir.join(FILE);
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(&file, "[fleet.dev-fleet]\nserver = \"dev-01\"\nhost = \"x:7777\"\n").expect("write");
    let refused = read(&file).expect_err("no pin");
    assert!(refused.contains("no string `pin`"), "{refused}");

    std::fs::write(
        &file,
        format!("[fleet.Bad_Name]\nserver = \"s\"\nhost = \"x:7777\"\npin = \"{PIN}\"\nidentity = \"i\"\n"),
    )
    .expect("write");
    let refused = read(&file).expect_err("not a label");
    assert!(refused.contains("not a fleet name"), "{refused}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn record(name: &str, address: Option<&str>, pin: Option<&str>) -> rivetos_register::Device {
    let mut interfaces = std::collections::BTreeMap::new();
    interfaces.insert(
        "eth0".to_owned(),
        rivetos_register::Interface {
            method: if address.is_some() { "static" } else { "dhcp" }.to_owned(),
            address: address.map(str::to_owned),
            gateway: None,
        },
    );
    rivetos_register::Device {
        id: format!("{:0>16}", name.len()),
        name: name.to_owned(),
        pin: pin.map(str::to_owned),
        interfaces,
        ..rivetos_register::Device::default()
    }
}

/// **A DHCP server is bookmarked by its `.local` name**, not the address it
/// resolves to today; a static one by its address; and the pin comes from
/// the record.
#[test]
fn a_bookmark_takes_the_records_address_or_local_name_and_its_pin() {
    let mut register = rivetos_register::Register::empty(scratch("register"));
    register.enroll(record("dev-01", None, Some(PIN))).expect("enroll");
    register.enroll(record("rivet-01", Some("192.168.50.10/24"), Some(PIN))).expect("enroll");
    let none = Endpoint::default();

    let made = from_register("dev-fleet", "dev-01", &register, &none, Some(PathBuf::from("op"))).expect("dhcp");
    assert_eq!(made.host, "dev-01.local:7777");
    assert_eq!(made.pin, PIN);
    assert_eq!(made.identity, PathBuf::from("op"));

    let made = from_register("home-fleet", "rivet-01", &register, &none, Some(PathBuf::from("op"))).expect("static");
    assert_eq!(made.host, "192.168.50.10:7777");

    let given = Endpoint {
        host: Some("10.0.0.9:7777".to_owned()),
        identity: Some(PathBuf::from("other")),
        pin: Some("e".repeat(64)),
    };
    let made = from_register("dev-fleet", "dev-01", &register, &given, Some(PathBuf::from("op"))).expect("flags");
    assert_eq!((made.host.as_str(), made.pin.as_str()), ("10.0.0.9:7777", "e".repeat(64).as_str()));
    assert_eq!(made.identity, PathBuf::from("other"));
}

#[test]
fn a_server_with_no_record_no_pin_or_a_bad_fleet_name_is_refused() {
    let mut register = rivetos_register::Register::empty(scratch("register-refusals"));
    register.enroll(record("nopin-01", None, None)).expect("enroll");
    let none = Endpoint::default();
    let op = Some(PathBuf::from("op"));

    let refused = from_register("dev-fleet", "ghost", &register, &none, op.clone()).expect_err("no record");
    assert!(refused.contains("no --host was given"), "{refused}");
    let refused = from_register("dev-fleet", "nopin-01", &register, &none, op.clone()).expect_err("no pin");
    assert!(refused.contains("nothing to pin"), "{refused}");
    let refused = from_register("FLEET", "nopin-01", &register, &none, op).expect_err("not a label");
    assert!(refused.contains("not a fleet name"), "{refused}");
}

#[test]
fn the_grammar_reads_add_list_and_remove() {
    match parse(&words(&["bookmark", "add", "dev-fleet", "dev-01", "--replace"])).expect("add") {
        BookmarkArgs::Add { fleet, server, replace, endpoint } => {
            assert_eq!((fleet.as_str(), server.as_str(), replace), ("dev-fleet", "dev-01", true));
            assert!(endpoint.host.is_none());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(parse(&words(&["bookmark", "list"])).expect("list"), BookmarkArgs::List));
    assert!(matches!(
        parse(&words(&["bookmark", "remove", "dev-fleet"])).expect("remove"),
        BookmarkArgs::Remove { .. }
    ));
    assert!(parse(&words(&["bookmark"])).is_err());
    assert!(parse(&words(&["bookmark", "add", "dev-fleet"])).is_err(), "add needs a server");
    assert!(parse(&words(&["bookmark", "list", "--replace"])).is_err(), "--replace is add's");
    assert!(parse(&words(&["bookmark", "add", "x", "y", "--identity", "d"])).is_err(), "half the trust");
}
