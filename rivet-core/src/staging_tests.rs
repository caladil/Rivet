//! The staging bundle's tests.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn one(app: &str, path: &str, bytes: &[u8]) -> Staged {
    Staged::new(app, path, bytes.to_vec()).expect("a legal entry")
}

/// The whole round trip, over the exact set `SPEC-05` §10 measured off the
/// machine that was fixed by hand.
#[test]
fn a_fleet_servers_own_set_survives_a_round_trip() {
    let items = vec![
        one("rivet-fleetd", "server.crt", b"-----BEGIN CERTIFICATE-----\n"),
        one("rivet-fleetd", "server.key", b"-----BEGIN PRIVATE KEY-----\n"),
        one("rivet-fleetd", "ca.crt", b"-----BEGIN CERTIFICATE-----\n"),
        one("rivet-fleetd", "operator-device.seed.hex", b"00ff\n"),
        one("rivet-fleetd", "operator-device.crt.hex", b"3082\n"),
        one("rivet-fleetd", "register/register.toml", b"name = \"my-fleet\"\n"),
        one(
            "rivet-fleetd",
            "register/devices/abc123.toml",
            b"id = \"abc123\"\n",
        ),
    ];
    let text = render(&items);
    let back = parse(&text).expect("what render wrote, parse reads");
    let mut sorted = items;
    sorted.sort_by(|a, b| a.application.cmp(&b.application).then(a.path.cmp(&b.path)));
    assert_eq!(back, sorted);
}

#[test]
fn rendering_is_a_function_of_the_set_and_not_of_the_order_it_arrived_in() {
    let a = vec![
        one("rivet-fleetd", "server.key", b"k"),
        one("rivet-fleetd", "ca.crt", b"c"),
        one("aura-controller", "tls-cert", b"t"),
    ];
    let b = vec![
        one("aura-controller", "tls-cert", b"t"),
        one("rivet-fleetd", "ca.crt", b"c"),
        one("rivet-fleetd", "server.key", b"k"),
    ];
    assert_eq!(render(&a), render(&b));
}

/// **The security boundary of the whole mechanism.** The supervisor joins these
/// onto a directory it writes as root.
#[test]
fn a_path_that_would_escape_the_data_directory_is_refused() {
    for bad in [
        "../../etc/passwd",
        "/etc/passwd",
        "register/../../secrets/device.seed",
        "..",
        ".",
        "a/./b",
        "a//b",
        "back\\slash",
        "with space",
        "",
    ] {
        assert!(
            check_path(bad).is_err(),
            "{bad:?} was accepted as a staged path"
        );
        assert!(
            Staged::new("rivet-fleetd", bad, vec![]).is_err(),
            "{bad:?} was accepted by the constructor"
        );
    }
}

#[test]
fn a_path_a_fleet_server_actually_needs_is_accepted() {
    for good in [
        "server.crt",
        "operator-device.seed.hex",
        "register/register.toml",
        "register/devices/kitchen-pi.toml",
    ] {
        check_path(good).unwrap_or_else(|e| panic!("{good:?} was refused: {e}"));
    }
}

#[test]
fn an_application_name_that_is_not_a_service_name_is_refused() {
    for bad in ["../rivet-fleetd", "rivet fleetd", "", "-leading-dash"] {
        assert!(
            Staged::new(bad, "server.crt", vec![]).is_err(),
            "{bad:?} was accepted as an application"
        );
    }
}

#[test]
fn parents_names_every_directory_outermost_first() {
    assert_eq!(parents("server.crt"), Vec::<String>::new());
    assert_eq!(parents("register/register.toml"), vec!["register"]);
    assert_eq!(
        parents("register/devices/a.toml"),
        vec!["register", "register/devices"]
    );
}

#[test]
fn a_bundle_with_no_version_is_refused_rather_than_read_as_empty() {
    assert_eq!(parse("# only a comment\n"), Err(Error::NoVersion));
    assert_eq!(
        parse("rivet-fleetd server.crt 00\n"),
        Err(Error::NoVersion)
    );
}

#[test]
fn a_version_this_build_does_not_know_is_refused_by_number() {
    let Err(Error::Version { given }) = parse("version = 99\n") else {
        panic!("a future version was accepted");
    };
    assert_eq!(given, "99");
}

#[test]
fn odd_hex_is_refused_and_the_value_is_not_in_the_message() {
    let text = "version = 1\nrivet-fleetd server.key deadbee\n";
    let Err(err) = parse(text) else {
        panic!("odd-length hex was accepted");
    };
    let said = err.to_string();
    assert!(said.contains("line 2"), "{said}");
    assert!(
        !said.contains("deadbee"),
        "the refusal quoted the value: {said}"
    );
}

#[test]
fn a_file_addressed_twice_is_refused() {
    let text = "version = 1\nrivet-fleetd server.crt 00\nrivet-fleetd server.crt 01\n";
    assert!(matches!(parse(text), Err(Error::Duplicate { .. })));
}

#[test]
fn comments_and_blank_lines_are_ignored_wherever_they_fall() {
    let text = "\n# a header\nversion = 1\n\n# a note\nrivet-fleetd server.crt 6869\n";
    let items = parse(text).expect("comments are legal");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].bytes, b"hi");
}

#[test]
fn a_bundle_larger_than_the_limit_is_refused_before_it_is_read() {
    let text = "x".repeat(MAX_BUNDLE_BYTES + 1);
    assert!(matches!(parse(&text), Err(Error::TooLarge { .. })));
}

#[test]
fn hex_round_trips_every_byte() {
    let all: Vec<u8> = (0u8..=255).collect();
    let text = format!("version = 1\nrivet-fleetd blob {}\n", hex(&all));
    let items = parse(&text).expect("every byte");
    assert_eq!(items[0].bytes, all);
}

#[test]
fn an_applications_share_waits_under_the_staging_directory_named_for_it() {
    assert_eq!(
        staged_bundle("rivet-fleetd"),
        "/state/staging/rivet-fleetd.txt"
    );
}

/// Splitting and re-rendering loses nothing, and the pieces are what the
/// supervisor will read back one application at a time.
#[test]
fn a_bundle_splits_into_one_bundle_per_application_and_nothing_is_lost() {
    let items = vec![
        one("rivet-fleetd", "server.crt", b"c"),
        one("aura-controller", "tls-cert", b"t"),
        one("rivet-fleetd", "server.key", b"k"),
    ];
    let split = by_application(&items);
    assert_eq!(
        split.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        vec!["aura-controller", "rivet-fleetd"],
        "the split is not in name order"
    );
    let total: usize = split.iter().map(|(_, v)| v.len()).sum();
    assert_eq!(total, items.len());

    for (name, mine) in split {
        let back = parse(&render(&mine)).expect("each piece is a bundle in its own right");
        assert!(back.iter().all(|i| i.application == name));
    }
}

/// The bundle carries a private key onto a FAT partition, so its name has to be
/// one `cmd/rivet-image`'s `check_name` accepts — a name that partition cannot
/// hold is a card whose credentials silently did not travel.
#[test]
fn the_bundle_file_is_a_name_fat_can_hold() {
    assert!(BUNDLE_FILE.is_ascii());
    assert!(!BUNDLE_FILE.contains('/'));
    assert!(!BUNDLE_FILE.contains('\\'));
    assert!(!BUNDLE_FILE.ends_with('.'));
    assert!(BUNDLE_FILE.len() <= 255);
}

/// `render` of an empty entry writes a line with nothing after the path —
/// the hex of zero bytes is the empty string — and `parse` must read that
/// back as the same entry rather than refusing the whole bundle over a field
/// it trimmed away.
#[test]
fn an_empty_entry_round_trips_through_render_and_parse() {
    let items = vec![one("rivet-fleetd", "placeholder", b"")];
    let text = render(&items);
    let back = parse(&text).expect("an empty entry is not a malformed bundle");
    assert_eq!(back, items);
}

/// **At the exact mechanism.** The line `parse` sees for an empty entry has
/// two space-separated fields, not three, because this parser trims the
/// whole line before splitting it and there is nothing after the path to
/// survive that trim. Hand-composed, to test the parser's own contract
/// rather than through `render`.
#[test]
fn a_hand_composed_two_field_line_parses_as_a_zero_byte_entry() {
    let items = parse("version = 1\nrivet-fleetd placeholder \n").expect("two fields is legal");
    assert_eq!(items, vec![one("rivet-fleetd", "placeholder", b"")]);
}

/// **The production trigger**: `cmd/cargo-rivet/src/carry.rs` reads a fleet
/// repository's `documents/` and `register/` directories with no empty-file
/// filter, so a bundle mixing empty and non-empty entries across two
/// applications must round-trip exactly, byte for byte, entry for entry.
#[test]
fn a_mix_of_empty_and_non_empty_entries_across_two_applications_round_trips_exactly() {
    let items = vec![
        one("rivet-fleetd", "server.crt", b"cert bytes"),
        one("rivet-fleetd", "documents/placeholder", b""),
        one("aura-controller", "documents/.gitkeep", b""),
        one("aura-controller", "tls-cert", b"t"),
    ];
    let text = render(&items);
    let back = parse(&text).expect("empty and non-empty entries together are not malformed");
    let mut sorted = items;
    sorted.sort_by(|a, b| a.application.cmp(&b.application).then(a.path.cmp(&b.path)));
    assert_eq!(back, sorted);
}

/// **The grammar is not thrown open.** A line with one field, or four, still
/// refuses exactly as before — only the two-field case gained a meaning.
#[test]
fn a_line_with_one_field_or_four_still_refuses_with_fields() {
    let Err(Error::Fields { line, fields }) = parse("version = 1\nrivet-fleetd\n") else {
        panic!("a one-field line was accepted, or refused some other way");
    };
    assert_eq!((line, fields), (2, 1));

    let Err(Error::Fields { line, fields }) =
        parse("version = 1\nrivet-fleetd server.crt 00 extra\n")
    else {
        panic!("a four-field line was accepted, or refused some other way");
    };
    assert_eq!((line, fields), (2, 4));
}
