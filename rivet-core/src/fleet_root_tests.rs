//! What `crate::fleet_root` guarantees: a request names the fleet's own names
//! and nothing else, and reads back as it was written.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// **The rule this module exists for.** Every suffix outside the fleet's own
/// three is refused — including the ones that would make the root vouch for
/// everything, and a sibling fleet's.
#[test]
fn a_suffix_outside_the_fleets_names_is_refused() {
    for bad in ["", ".", "com", "home.arpa", "lan", "their-fleet.home.arpa", "example.com"] {
        match Request::new("dev-fleet", "dev-01", &owned(&["local", bad])) {
            Err(Error::Suffix { .. }) => {}
            other => panic!("{bad:?} was not refused as outside the fleet's names: {other:?}"),
        }
    }
}

#[test]
fn every_one_of_the_fleets_names_is_accepted_and_kept_in_order() {
    let request = Request::new(
        "dev-fleet",
        "dev-01",
        &owned(&["local", "dev-fleet.home.arpa", "dev-fleet.lan"]),
    )
    .expect("the fleet's three names");
    assert_eq!(
        request.names(),
        owned(&["dev-01.local", "dev-01.dev-fleet.home.arpa", "dev-01.dev-fleet.lan"])
    );
}

#[test]
fn a_request_with_no_suffix_is_refused() {
    assert_eq!(
        Request::new("dev-fleet", "dev-01", &[]),
        Err(Error::NoSuffix)
    );
}

#[test]
fn a_repeated_suffix_is_refused_rather_than_merged() {
    assert_eq!(
        Request::new("f", "h", &owned(&["local", "local."])),
        Err(Error::Repeated("local".to_owned()))
    );
}

#[test]
fn a_fleet_or_host_that_is_not_one_label_is_refused() {
    for bad in ["", "Fleet", "a.b", "-a", "a-", "a b", &"x".repeat(64)] {
        assert!(
            matches!(Request::new(bad, "h", &owned(&["local"])), Err(Error::Fleet(_))),
            "fleet {bad:?}"
        );
        assert!(
            matches!(Request::new("f", bad, &owned(&["local"])), Err(Error::Host(_))),
            "host {bad:?}"
        );
    }
}

#[test]
fn a_request_reads_back_as_it_was_written() {
    let request = Request::new(
        "dev-fleet",
        "dev-01",
        &default_suffixes("dev-fleet"),
    )
    .expect("the defaults");
    assert_eq!(parse(&request.render()), Ok(request));
}

#[test]
fn the_defaults_are_local_and_home_arpa_and_never_lan() {
    assert_eq!(
        default_suffixes("dev-fleet"),
        owned(&["local", "dev-fleet.home.arpa"])
    );
}

#[test]
fn parse_refuses_an_unknown_key_a_repeated_host_and_another_version() {
    let good = "version = 1\nfleet = f\nhost = h\nsuffix = local\n";
    assert!(parse(good).is_ok());
    assert_eq!(parse(&good.replace("version = 1", "version = 2")), Err(Error::Version("2".into())));
    assert_eq!(parse(&format!("{good}names = x\n")), Err(Error::Line(5)));
    assert_eq!(parse(&format!("{good}host = other\n")), Err(Error::Line(5)));
    assert!(matches!(parse("fleet = f\nhost = h\nsuffix = local\n"), Err(Error::Version(_))));
}

/// The request's addressee is a name the staging bundle accepts, so it
/// travels in the bundle with no change to the format.
#[test]
fn the_request_is_a_legal_staging_entry() {
    let request = Request::new("f", "h", &owned(&["local"])).expect("legal");
    crate::staging::Staged::new(REQUEST_APPLICATION, REQUEST_PATH, request.render().into_bytes())
        .expect("a request is an ordinary staging entry");
}

#[test]
fn the_minted_material_is_handed_to_the_proxy_and_fleetd_under_their_own_names() {
    assert_eq!(
        handed_paths(),
        vec![
            ("rivet-proxyd", "tls-cert"),
            ("rivet-proxyd", "tls-key"),
            ("rivet-proxyd", "tls-root"),
            ("rivet-fleetd", "server.crt"),
            ("rivet-fleetd", "server.key"),
            ("rivet-fleetd", "ca.crt"),
        ]
    );
    assert_eq!(root_certificate_path(), "/state/secrets/fleet-root.crt");
    assert_eq!(root_key_path(), "/state/secrets/fleet-root.key");
}
