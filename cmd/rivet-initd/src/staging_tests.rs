//! What `crate::staging` guarantees.
//!
//! The two that matter are `the_bundle_is_destroyed_only_after_state_has_it` —
//! the ordering `crate::identity` measured on a board and this inherits — and
//! `nothing_is_delivered_to_an_application_here`, which is the whole reason
//! there is a second hop at all.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::provision::BOOT_PARTITION_MOUNT;
use crate::staging::{BUNDLE_FILE, STAGING_DIR, Staged, provision_staging};
use crate::testkernel::{FakeKernel, Script};

use rivet_core::staging;

fn on_card() -> String {
    format!("{BOOT_PARTITION_MOUNT}/{BUNDLE_FILE}")
}

/// A board whose boot partition mounts, carrying whatever the test says.
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

/// The fleet server's own set, as `SPEC-05` §10 measured it off the machine
/// that had to be fixed by hand.
fn a_fleet_servers_bundle() -> String {
    staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.crt", b"CERT".to_vec()).expect("legal"),
        staging::Staged::new("rivet-fleetd", "server.key", b"KEY".to_vec()).expect("legal"),
        staging::Staged::new("rivet-fleetd", "ca.crt", b"CA".to_vec()).expect("legal"),
        staging::Staged::new(
            "rivet-fleetd",
            "operator-device.seed.hex",
            b"00ff\n".to_vec(),
        )
        .expect("legal"),
        staging::Staged::new(
            "rivet-fleetd",
            "operator-device.crt.hex",
            b"3082\n".to_vec(),
        )
        .expect("legal"),
        staging::Staged::new(
            "rivet-fleetd",
            "register/register.toml",
            b"name = \"someone\"\n".to_vec(),
        )
        .expect("legal"),
        staging::Staged::new(
            "rivet-fleetd",
            "register/devices/abc.toml",
            b"id = \"abc\"\n".to_vec(),
        )
        .expect("legal"),
    ])
}

/// **The whole point.** Everything a fleet server needs lands under
/// `/state/staging` as that application's own bundle, unchanged in content, and
/// the card's copy is blanked because it holds a key.
#[test]
fn a_bundle_on_the_card_is_split_per_application_and_then_destroyed_on_the_card() {
    let bundle = a_fleet_servers_bundle();
    let mut k = board_with(&[(&on_card(), bundle.into_bytes())]);

    let outcome = provision_staging(&mut k);
    assert_eq!(
        outcome,
        Staged::Installed {
            written: 1,
            kept: 0
        },
        "{}",
        outcome.summary()
    );

    let at = staging::staged_bundle("rivet-fleetd");
    let (_, body) = k
        .secret_writes
        .iter()
        .find(|(p, _)| *p == at)
        .unwrap_or_else(|| panic!("{at} was not written: {:?}", k.secret_writes));
    let back = staging::parse(core::str::from_utf8(body).expect("text"))
        .expect("what PID 1 wrote is a bundle");
    assert_eq!(
        back.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(),
        vec![
            "ca.crt",
            "operator-device.crt.hex",
            "operator-device.seed.hex",
            "register/devices/abc.toml",
            "register/register.toml",
            "server.crt",
            "server.key",
        ],
        "the fleet server's own set did not survive the split"
    );

    // Through the writer that sets 0600, and never the one that does not.
    assert!(
        k.write_calls
            .iter()
            .all(|(p, _)| !p.starts_with(STAGING_DIR)),
        "a staged bundle went through write_file rather than write_secret"
    );
    assert!(
        k.private_dirs.iter().any(|d| d == STAGING_DIR),
        "the staging root was not created privately: {:?}",
        k.private_dirs
    );
    assert!(
        k.remove_calls.iter().any(|p| *p == on_card()),
        "the bundle was not removed from the boot partition: {:?}",
        k.remove_calls
    );
}

/// Two applications get two bundles, so an install of one does not have to wait
/// on the other.
#[test]
fn each_application_gets_its_own_bundle() {
    let bundle = staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.crt", b"C".to_vec()).expect("legal"),
        staging::Staged::new("aura-controller", "tls-cert", b"T".to_vec()).expect("legal"),
    ]);
    let mut k = board_with(&[(&on_card(), bundle.into_bytes())]);
    assert_eq!(
        provision_staging(&mut k),
        Staged::Installed {
            written: 2,
            kept: 0
        }
    );
    let paths: Vec<&str> = k.secret_writes.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"/state/staging/rivet-fleetd.txt"), "{paths:?}");
    assert!(
        paths.contains(&"/state/staging/aura-controller.txt"),
        "{paths:?}"
    );
}

/// **The ordering guarantee.** The bundle holds `server.key` and an operator
/// seed; destroying the card's copy before `/state` is flushed loses them.
#[test]
fn the_bundle_is_destroyed_only_after_state_has_it() {
    let mut k = board_with(&[(&on_card(), a_fleet_servers_bundle().into_bytes())]);
    assert!(matches!(
        provision_staging(&mut k),
        Staged::Installed { .. }
    ));

    let last_write = k
        .order
        .iter()
        .rposition(|s| s.starts_with("write_secret /state/staging/"))
        .expect("nothing was staged");
    let blanked = k
        .order
        .iter()
        .position(|s| s == &format!("overwrite_in_place {}", on_card()))
        .expect("the bundle was never blanked");
    let removed = k
        .order
        .iter()
        .position(|s| s == &format!("remove_file {}", on_card()))
        .expect("the bundle was never removed");
    let synced = k
        .order
        .iter()
        .position(|s| s == "sync")
        .expect("nothing was synced");

    assert!(last_write < synced, "{:?}", k.order);
    assert!(synced < blanked, "the card's copy was blanked before /state was flushed: {:?}", k.order);
    assert!(blanked < removed, "it was unlinked before it was blanked, which on FAT leaves the bytes readable: {:?}", k.order);
}

/// **Nothing reaches an application here, and that is the design.** The uid an
/// application's directory is owned by is derived by the supervisor at
/// discovery; a file placed as root in that directory is invisible to the
/// application and reports nothing.
#[test]
fn nothing_is_delivered_to_an_application_here() {
    let mut k = board_with(&[(&on_card(), a_fleet_servers_bundle().into_bytes())]);
    assert!(matches!(
        provision_staging(&mut k),
        Staged::Installed { .. }
    ));

    for (path, _) in k.secret_writes.iter().chain(k.write_calls.iter()) {
        assert!(
            !path.starts_with("/state/app/"),
            "PID 1 wrote into an application's directory: {path}"
        );
    }
    for dir in &k.private_dirs {
        assert!(
            !dir.starts_with("/state/app/"),
            "PID 1 created a directory inside an application's tree: {dir}"
        );
    }
}

/// A card that stages nothing is the ordinary case, not a fault.
#[test]
fn a_card_with_no_bundle_is_not_an_error() {
    let mut k = board_with(&[]);
    assert_eq!(provision_staging(&mut k), Staged::Nothing);
    assert!(k.secret_writes.is_empty());
    assert!(k.remove_calls.is_empty(), "nothing should have been deleted");
}

/// **A bundle that will not parse is refused whole and LEFT on the card.** A
/// half-applied staging is a state nobody designed; the card still holds
/// everything, so a corrected card is the repair.
#[test]
fn a_bundle_that_will_not_parse_is_refused_and_left_in_place() {
    let mut k = board_with(&[(&on_card(), b"version = 1\nnot three fields\n".to_vec())]);
    let outcome = provision_staging(&mut k);
    match &outcome {
        Staged::Refused(why) => {
            assert!(why.contains("LEFT on the boot partition"), "{why}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(k.secret_writes.is_empty(), "a partial staging was written");
    assert!(
        k.remove_calls.is_empty(),
        "the bundle was destroyed despite the refusal"
    );
}

/// **Refused by `stat`, before any allocation.** `file_size` is asked first,
/// so an oversized bundle is never read into memory at all — R-2: the read
/// this module used to perform before `staging::parse` ever checked the cap.
#[test]
fn an_oversized_bundle_is_refused_by_stat_and_never_read() {
    let mut k = board_with(&[]);
    k.file_sizes_mut().insert(
        on_card(),
        Script::always(Ok(staging::MAX_BUNDLE_BYTES as u64 + 1)),
    );

    let outcome = provision_staging(&mut k);
    let Staged::Refused(why) = &outcome else {
        unreachable!("an oversized bundle was staged: {outcome:?}")
    };
    assert!(
        why.contains(&staging::MAX_BUNDLE_BYTES.to_string()),
        "the refusal does not say what the cap is: {why}"
    );
    assert!(
        !k.read_bytes_calls.contains(&on_card()),
        "the bundle was read into memory despite being refused by size: {:?}",
        k.read_bytes_calls
    );
}

/// A bundle whose path tries to leave the data directory is a bundle this
/// refuses, and the refusal is `rivet-core`'s so there is only one rule.
#[test]
fn a_bundle_that_tries_to_escape_a_data_directory_is_refused() {
    let mut k = board_with(&[(
        &on_card(),
        b"version = 1\nrivet-fleetd ../../secrets/device.seed 00\n".to_vec(),
    )]);
    assert!(matches!(provision_staging(&mut k), Staged::Refused(_)));
    assert!(k.secret_writes.is_empty());
}

/// **`/state` wins.** A staged file already present is kept, and the card is
/// still cleared — a private key on a FAT partition is the thing this prevents.
///
/// **The fixture is a genuine, complete rendering, not a placeholder.** It
/// differs from the card's current bundle for `rivet-fleetd` only in the
/// certificate's content ("OLD" rather than "NEW"), so it is the same length —
/// exactly the shape a complete bundle from an earlier, different provisioning
/// of this card would have. `a_truncated_existing_bundle_is_overwritten_rather_
/// than_kept` below covers the OTHER existing-but-different shape, where it
/// must NOT be kept.
#[test]
fn a_bundle_state_already_holds_is_kept_and_the_card_is_still_cleared() {
    let bundle = staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.crt", b"NEW".to_vec()).expect("legal"),
        staging::Staged::new("aura-controller", "tls-cert", b"NEW".to_vec()).expect("legal"),
    ]);
    let already_held = staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.crt", b"OLD".to_vec()).expect("legal"),
    ]);
    assert_eq!(
        already_held.len(),
        staging::render(&[
            staging::Staged::new("rivet-fleetd", "server.crt", b"NEW".to_vec()).expect("legal")
        ])
        .len(),
        "the fixture must be the same length as what the card would render, or this test \
         exercises the truncated case instead of the different-but-complete one"
    );
    let mut k = board_with(&[
        (&on_card(), bundle.into_bytes()),
        ("/state/staging/rivet-fleetd.txt", already_held.into_bytes()),
    ]);

    let outcome = provision_staging(&mut k);
    assert_eq!(
        outcome,
        Staged::Installed {
            written: 1,
            kept: 1
        },
        "{}",
        outcome.summary()
    );
    assert_eq!(
        k.secret_writes
            .iter()
            .map(|(p, _)| p.as_str())
            .collect::<Vec<_>>(),
        vec!["/state/staging/aura-controller.txt"],
        "the bundle /state already held was overwritten"
    );
    assert!(k.remove_calls.iter().any(|p| *p == on_card()));
}

/// **B12, staging's shape of it.** A bundle a previous, interrupted boot only
/// half wrote is readable — `read_bytes(...).is_ok()` used to be the whole
/// test — and accepting it as "already staged" would destroy the card's
/// complete copy while leaving the application with a bundle it cannot parse.
#[test]
fn a_truncated_existing_bundle_is_overwritten_rather_than_kept() {
    let full = staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.crt", b"CERT-BYTES".to_vec())
            .expect("legal"),
    ]);
    // A genuine prefix, the shape a write cut off mid-way leaves — not an
    // arbitrary placeholder.
    let truncated = full.as_bytes()[..full.len() / 2].to_vec();
    assert_ne!(
        truncated.len(),
        full.len(),
        "the fixture must be a different length or this does not exercise Incomplete"
    );

    let mut k = board_with(&[
        (&on_card(), full.clone().into_bytes()),
        ("/state/staging/rivet-fleetd.txt", truncated),
    ]);

    let outcome = provision_staging(&mut k);
    assert_eq!(
        outcome,
        Staged::Installed {
            written: 1,
            kept: 0
        },
        "a truncated existing bundle must be rewritten, not kept: {}",
        outcome.summary()
    );
    assert_eq!(
        k.secret_writes
            .iter()
            .find(|(p, _)| p == "/state/staging/rivet-fleetd.txt")
            .map(|(_, b)| b.as_slice()),
        Some(full.as_bytes()),
        "the bundle was not rewritten to the complete, correct content"
    );
    assert!(
        k.remove_calls.iter().any(|p| *p == on_card()),
        "the card's complete copy must still be cleared once the recovery is complete"
    );
}

/// A bundle `/state` already holds byte-for-byte is left alone — no rewrite,
/// same outcome as before this module started comparing content.
#[test]
fn a_bundle_that_exactly_matches_the_card_is_kept_without_a_rewrite() {
    let bundle = staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.crt", b"SAME".to_vec()).expect("legal"),
    ]);
    let mut k = board_with(&[
        (&on_card(), bundle.clone().into_bytes()),
        ("/state/staging/rivet-fleetd.txt", bundle.into_bytes()),
    ]);

    let outcome = provision_staging(&mut k);
    assert_eq!(
        outcome,
        Staged::Installed {
            written: 0,
            kept: 1
        },
        "{}",
        outcome.summary()
    );
    assert!(k.secret_writes.is_empty(), "an exact match must not be rewritten");
}

/// An empty bundle is evidence of a provisioning run that went wrong, so it is
/// kept rather than tidied away.
#[test]
fn an_empty_bundle_is_refused_and_kept_as_evidence() {
    let mut k = board_with(&[(&on_card(), b"version = 1\n".to_vec())]);
    match provision_staging(&mut k) {
        Staged::Refused(why) => assert!(why.contains("stages no files"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(k.remove_calls.is_empty());
}

/// **F01: the bundle on the boot partition this boot did NOT come from goes
/// too**, once the same bytes have been staged from the booted one. A bundle
/// there that differs is not proven to be on `/state`, so it is left.
#[test]
fn the_unbooted_partitions_identical_bundle_is_blanked_and_a_different_one_kept() {
    let unbooted = format!("{}/{BUNDLE_FILE}", crate::unbooted::UNBOOTED_PARTITION_MOUNT);
    let bundle = a_fleet_servers_bundle().into_bytes();
    let other = staging::render(&[
        staging::Staged::new("rivet-fleetd", "server.key", b"ANOTHER".to_vec()).expect("legal"),
    ])
    .into_bytes();

    for (on_other, removed) in [(bundle.clone(), true), (other, false)] {
        let mut k = board_with(&[(&on_card(), bundle.clone()), (&unbooted, on_other.clone())]);
        let outcome = provision_staging(&mut k);
        assert!(matches!(outcome, Staged::Installed { .. }), "{}", outcome.summary());
        assert_eq!(
            k.remove_calls.contains(&unbooted),
            removed,
            "removed {:?} from the unbooted partition, expected removed = {removed}",
            k.remove_calls
        );
        if removed {
            assert!(
                k.write_calls
                    .iter()
                    .any(|(p, b)| *p == unbooted && *b == vec![0u8; on_other.len()]),
                "unlinked without being blanked: {:?}",
                k.order
            );
            let staged = k
                .order
                .iter()
                .position(|s| s.starts_with("write_secret /state/staging/"))
                .expect("nothing staged");
            let blanked = k
                .order
                .iter()
                .position(|s| s == &format!("overwrite_in_place {unbooted}"))
                .expect("not blanked");
            assert!(staged < blanked, "{:?}", k.order);
        }
        let mounts = k
            .mount_calls
            .iter()
            .filter(|c| c.target == crate::unbooted::UNBOOTED_PARTITION_MOUNT)
            .count();
        let unmounts = k
            .umount_calls
            .iter()
            .filter(|t| *t == crate::unbooted::UNBOOTED_PARTITION_MOUNT)
            .count();
        assert!(mounts > 0, "the unbooted partition was never looked at");
        assert_eq!(mounts, unmounts, "the unbooted partition was left mounted");
    }
}

/// Every outcome says something a person can act on.
#[test]
fn every_outcome_says_more_than_a_word() {
    for outcome in [
        Staged::Nothing,
        Staged::Installed {
            written: 3,
            kept: 0,
        },
        Staged::Refused("because".to_string()),
    ] {
        assert!(outcome.summary().len() > 20, "{outcome:?}");
    }
}
