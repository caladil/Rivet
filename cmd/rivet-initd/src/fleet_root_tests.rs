//! What `crate::fleet_root` guarantees, driven through `crate::staging` as a
//! first boot drives it. The randomness is the fake kernel's counter,
//! **self-generated**; the chain itself is judged in `rivet-fleet-root`'s own
//! tests.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use rivet_core::fleet_root::{
    MINTED_FOR_FILE, REQUEST_APPLICATION, REQUEST_PATH, ROOT_CERTIFICATE_FILE, ROOT_KEY_FILE,
    Request, SERVER_CERTIFICATE_FILE, SERVER_KEY_FILE,
};
use rivet_core::staging;

use crate::provision::BOOT_PARTITION_MOUNT;
use crate::staging::{BUNDLE_FILE, Staged, provision_staging};
use crate::testkernel::{FakeKernel, Script};

fn on_card() -> String {
    format!("{BOOT_PARTITION_MOUNT}/{BUNDLE_FILE}")
}

fn secret(file: &str) -> String {
    format!("/state/secrets/{file}")
}

fn request() -> Request {
    Request::new(
        "dev-fleet",
        "dev-01",
        &["local".to_owned(), "dev-fleet.home.arpa".to_owned()],
    )
    .expect("legal")
}

/// A fleet-root card's bundle: the request, and what a workstation still
/// stages for `rivet-fleetd` — its operator credential and its register.
fn a_fleet_root_bundle(request: &Request) -> String {
    staging::render(&[
        staging::Staged::new(REQUEST_APPLICATION, REQUEST_PATH, request.render().into_bytes())
            .expect("legal"),
        staging::Staged::new("rivet-fleetd", "operator-device.seed.hex", b"00ff\n".to_vec())
            .expect("legal"),
        staging::Staged::new("rivet-fleetd", "register/register.toml", b"name = \"x\"\n".to_vec())
            .expect("legal"),
        staging::Staged::new("rivet-proxyd", "passkeys", b"a passkey line\n".to_vec())
            .expect("legal"),
    ])
}

fn board_with(card: &[(&str, Vec<u8>)]) -> FakeKernel {
    let mut k = FakeKernel::new();
    k.files_mut().insert(
        "/proc/device-tree/chosen/bootloader/partition".to_string(),
        Script::always(Ok("1".to_string())),
    );
    k.blobs_mut().insert(
        "/proc/device-tree/chosen/bootloader/partition".to_string(),
        Script::always(Ok(vec![0, 0, 0, 1])),
    );
    for (name, bytes) in card {
        k.blobs_mut()
            .insert((*name).to_string(), Script::always(Ok(bytes.clone())));
    }
    k
}

fn written<'a>(k: &'a FakeKernel, path: &str) -> Option<&'a [u8]> {
    k.secret_writes
        .iter()
        .rev()
        .find(|(p, _)| p == path)
        .map(|(_, b)| b.as_slice())
}

fn staged_for(k: &FakeKernel, application: &str) -> Vec<staging::Staged> {
    let body = written(k, &staging::staged_bundle(application))
        .unwrap_or_else(|| panic!("nothing staged for {application}: {:?}", k.order));
    staging::parse(core::str::from_utf8(body).expect("text")).expect("a bundle")
}

fn file<'a>(items: &'a [staging::Staged], path: &str) -> &'a [u8] {
    &items
        .iter()
        .find(|i| i.path == path)
        .unwrap_or_else(|| panic!("{path} was not staged"))
        .bytes
}

/// **The whole journey.** The request mints a root into `/state/secrets`, the
/// server's certificate reaches both programs under their own names with the
/// root beside it, the request itself reaches no application, and the card's
/// bundle is destroyed only after all of it is on `/state`.
#[test]
fn a_request_mints_the_root_and_hands_the_leaf_to_the_proxy_and_fleetd() {
    let mut k = board_with(&[(&on_card(), a_fleet_root_bundle(&request()).into_bytes())]);
    k.random_works = true;

    let outcome = provision_staging(&mut k);
    assert_eq!(outcome, Staged::Installed { written: 2, kept: 0 }, "{}", outcome.summary());

    let root = written(&k, &secret(ROOT_CERTIFICATE_FILE)).expect("the root rests");
    let server = written(&k, &secret(SERVER_CERTIFICATE_FILE)).expect("the leaf rests");
    let server_key = written(&k, &secret(SERVER_KEY_FILE)).expect("the leaf's key rests");
    assert!(written(&k, &secret(ROOT_KEY_FILE)).is_some(), "the root's key rests");
    assert_eq!(
        written(&k, &secret(MINTED_FOR_FILE)),
        Some(request().render().as_bytes()),
        "the request it was minted for rests beside it"
    );
    assert!(core::str::from_utf8(root).unwrap().contains("BEGIN CERTIFICATE"));

    let proxy = staged_for(&k, "rivet-proxyd");
    assert_eq!(file(&proxy, "tls-cert"), server);
    assert_eq!(file(&proxy, "tls-key"), server_key);
    assert_eq!(file(&proxy, "tls-root"), root);
    assert_eq!(file(&proxy, "passkeys"), b"a passkey line\n", "the carried passkeys ride along");
    let fleetd = staged_for(&k, "rivet-fleetd");
    assert_eq!(file(&fleetd, "server.crt"), server);
    assert_eq!(file(&fleetd, "server.key"), server_key);
    assert_eq!(file(&fleetd, "ca.crt"), root);
    assert_eq!(file(&fleetd, "operator-device.seed.hex"), b"00ff\n");

    assert!(
        written(&k, &staging::staged_bundle(REQUEST_APPLICATION)).is_none(),
        "the request reached /state/staging, where no application reads it"
    );

    // The root's certificate is the LAST secret written, and everything is on
    // /state before the card's bundle is blanked.
    let position = |needle: &str| {
        k.order
            .iter()
            .position(|line| line == needle)
            .unwrap_or_else(|| panic!("{needle} not in {:?}", k.order))
    };
    let root_at = position(&format!("write_secret {}", secret(ROOT_CERTIFICATE_FILE)));
    for other in [MINTED_FOR_FILE, SERVER_KEY_FILE, SERVER_CERTIFICATE_FILE, ROOT_KEY_FILE] {
        assert!(position(&format!("write_secret {}", secret(other))) < root_at, "{other}");
    }
    let proxy_at = position(&format!("write_secret {}", staging::staged_bundle("rivet-proxyd")));
    let blank_at = position(&format!("overwrite_in_place {}", on_card()));
    assert!(root_at < proxy_at && proxy_at < blank_at, "{:?}", k.order);
    assert!(k.lines.iter().any(|l| l.contains("minted this fleet's root")), "{:?}", k.lines);
}

/// **No random source, no root** — and the card keeps its bundle, so a boot
/// that has one finishes the job.
#[test]
fn without_randomness_nothing_is_minted_and_the_card_keeps_its_bundle() {
    let mut k = board_with(&[(&on_card(), a_fleet_root_bundle(&request()).into_bytes())]);
    assert!(!k.random_works);

    match provision_staging(&mut k) {
        Staged::Refused(why) => assert!(why.contains("random source"), "{why}"),
        other => panic!("minted with no random source: {other:?}"),
    }
    assert!(k.secret_writes.is_empty(), "something was written: {:?}", k.order);
    assert!(!k.remove_calls.iter().any(|p| *p == on_card()), "the card's bundle was removed");
}

/// **A second boot never re-mints.** One that finds the card's bundle again —
/// a first boot cut off after minting — hands over the resting set byte for
/// byte; one that finds no bundle draws no randomness at all.
#[test]
fn a_second_boot_never_re_mints() {
    let bundle = a_fleet_root_bundle(&request()).into_bytes();
    let mut first = board_with(&[(&on_card(), bundle.clone())]);
    first.random_works = true;
    assert!(matches!(provision_staging(&mut first), Staged::Installed { .. }));

    // The retried first boot: /state holds what the first wrote, the card
    // still holds its bundle.
    let mut again = board_with(&[(&on_card(), bundle)]);
    again.random_works = true;
    for file in [
        MINTED_FOR_FILE,
        SERVER_KEY_FILE,
        SERVER_CERTIFICATE_FILE,
        ROOT_KEY_FILE,
        ROOT_CERTIFICATE_FILE,
    ] {
        let bytes = written(&first, &secret(file)).expect("the first boot wrote it").to_vec();
        again.blobs_mut().insert(secret(file), Script::always(Ok(bytes)));
    }
    assert!(matches!(provision_staging(&mut again), Staged::Installed { .. }));
    assert_eq!(again.random_drawn, 0, "the retried boot drew randomness");
    assert!(
        written(&again, &secret(ROOT_CERTIFICATE_FILE)).is_none(),
        "a root was written over the resting one"
    );
    assert!(again.lines.iter().any(|l| l.contains("NOTHING WAS MINTED")), "{:?}", again.lines);
    assert_eq!(
        file(&staged_for(&again, "rivet-proxyd"), "tls-root"),
        written(&first, &secret(ROOT_CERTIFICATE_FILE)).unwrap(),
        "the retried boot handed over a different root"
    );

    // An ordinary second boot: no bundle on the card.
    let mut later = board_with(&[]);
    later.random_works = true;
    assert_eq!(provision_staging(&mut later), Staged::Nothing);
    assert_eq!(later.random_drawn, 0);
    assert!(later.secret_writes.is_empty());
}

/// A resting root minted for another fleet is never minted over.
#[test]
fn a_resting_root_for_another_request_is_refused() {
    let mut k = board_with(&[(&on_card(), a_fleet_root_bundle(&request()).into_bytes())]);
    k.random_works = true;
    let other = Request::new("someone-else", "dev-01", &["local".to_owned()]).expect("legal");
    for (file, text) in [
        (MINTED_FOR_FILE, other.render()),
        (ROOT_KEY_FILE, "KEY".to_owned()),
        (SERVER_CERTIFICATE_FILE, "LEAF".to_owned()),
        (SERVER_KEY_FILE, "LEAFKEY".to_owned()),
        (ROOT_CERTIFICATE_FILE, "ROOT".to_owned()),
    ] {
        k.blobs_mut().insert(secret(file), Script::always(Ok(text.into_bytes())));
    }
    match provision_staging(&mut k) {
        Staged::Refused(why) => assert!(why.contains("another request"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(k.secret_writes.is_empty());
    assert_eq!(k.random_drawn, 0);
}

/// A root resting without the rest of its set refuses rather than minting a
/// second root beside it.
#[test]
fn a_resting_root_missing_its_key_is_refused() {
    let mut k = board_with(&[(&on_card(), a_fleet_root_bundle(&request()).into_bytes())]);
    k.random_works = true;
    k.blobs_mut()
        .insert(secret(ROOT_CERTIFICATE_FILE), Script::always(Ok(b"ROOT".to_vec())));
    match provision_staging(&mut k) {
        Staged::Refused(why) => assert!(why.contains("NOTHING WAS MINTED"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(k.secret_writes.is_empty());
}

/// A bundle carrying the request AND a workstation's leaf has two answers for
/// one file, and is refused whole.
#[test]
fn a_request_beside_a_workstation_leaf_is_refused() {
    let bundle = staging::render(&[
        staging::Staged::new(REQUEST_APPLICATION, REQUEST_PATH, request().render().into_bytes())
            .expect("legal"),
        staging::Staged::new("rivet-proxyd", "tls-cert", b"CERT".to_vec()).expect("legal"),
    ]);
    let mut k = board_with(&[(&on_card(), bundle.into_bytes())]);
    k.random_works = true;
    match provision_staging(&mut k) {
        Staged::Refused(why) => assert!(why.contains("two answers"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(k.random_drawn, 0);
}

/// A request that will not read — here, a suffix outside the fleet's names —
/// mints nothing.
#[test]
fn a_request_naming_a_suffix_outside_the_fleet_is_refused() {
    let text = "version = 1\nfleet = dev-fleet\nhost = dev-01\nsuffix = example.com\n";
    let bundle = staging::render(&[
        staging::Staged::new(REQUEST_APPLICATION, REQUEST_PATH, text.as_bytes().to_vec())
            .expect("legal"),
    ]);
    let mut k = board_with(&[(&on_card(), bundle.into_bytes())]);
    k.random_works = true;
    assert!(matches!(provision_staging(&mut k), Staged::Refused(_)));
    assert_eq!(k.random_drawn, 0);
    assert!(k.secret_writes.is_empty());
}
