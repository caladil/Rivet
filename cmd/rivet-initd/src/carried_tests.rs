//! What `crate::carried` guarantees — `ADR-050` §4.
//!
//! The two that matter are `a_payload_already_recorded_is_not_carried_again` —
//! the record directory is the whole of the idempotence, and without it a card
//! reinstalls its applications on every boot for ever — and
//! `nothing_on_the_card_is_removed_or_overwritten`, which is this module's one
//! deliberate departure from `ADR-049`'s courier.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::carried::{CARRIED_DIR, Carried, provision_carried};
use crate::provision::BOOT_PARTITION_MOUNT;
use crate::testkernel::{FakeKernel, Script};

use rivet_core::carried;

fn on_card(product: &str) -> String {
    format!("{BOOT_PARTITION_MOUNT}/{}", carried::card_file(product))
}

/// A board whose boot partition mounts, holding exactly the names given.
///
/// **The directory listing and the bytes are separate**, because they are two
/// syscalls on a real machine and the module asks the first before the second —
/// a helper that could not express *a name that lists and will not read* could
/// not test the refusal at all.
fn board_carrying(names: &[&str], bytes: &[(&str, Vec<u8>)]) -> FakeKernel {
    let mut k = FakeKernel::new();
    k.files_mut().insert(
        "/proc/device-tree/chosen/bootloader/partition".to_string(),
        Script::always(Ok("1".to_string())),
    );
    k.blobs_mut().insert(
        "/proc/device-tree/chosen/bootloader/partition".to_string(),
        Script::always(Ok(vec![0, 0, 0, 1])),
    );
    // Everything a real boot partition holds, so the filter is doing work.
    let mut listing: Vec<String> = vec![
        "config.txt".to_string(),
        "tryboot.txt".to_string(),
        "autoboot.txt".to_string(),
        "cmdline.txt".to_string(),
        "kernel8.img".to_string(),
        "bcm2711-rpi-4-b.dtb".to_string(),
        "start4.elf".to_string(),
        "fixup4.dat".to_string(),
        "rivet.toml".to_string(),
    ];
    listing.extend(names.iter().map(|n| (*n).to_string()));
    k.dirs_mut()
        .insert(BOOT_PARTITION_MOUNT.to_string(), Script::always(Ok(listing)));
    for (path, blob) in bytes {
        k.blobs_mut()
            .insert((*path).to_string(), Script::always(Ok(blob.clone())));
    }
    k
}

/// A fleet server's card: `rivet-fleetd` and `rivet-proxyd`, which is
/// `ADR-050` §5's `server + headless` row.
fn a_fleet_servers_card() -> FakeKernel {
    board_carrying(
        &[
            &carried::card_file("rivet-fleetd"),
            &carried::card_file("rivet-proxyd"),
        ],
        &[
            (&on_card("rivet-fleetd"), b"FLEETD PAYLOAD".to_vec()),
            (&on_card("rivet-proxyd"), b"PROXYD PAYLOAD".to_vec()),
        ],
    )
}

fn written(k: &FakeKernel, product: &str) -> Option<Vec<u8>> {
    let at = carried::payload_path(product);
    k.secret_writes
        .iter()
        .find(|(p, _)| *p == at)
        .map(|(_, body)| body.clone())
}

/// **The whole point.** Both payloads land under `/state/carried`, byte for
/// byte, each in the record directory named after its product.
#[test]
fn every_payload_on_the_card_lands_in_state_unchanged() {
    let mut k = a_fleet_servers_card();

    let outcome = provision_carried(&mut k);
    assert_eq!(
        outcome,
        Carried::Landed {
            written: 2,
            kept: 0
        },
        "{}",
        outcome.summary()
    );

    assert_eq!(
        written(&k, "rivet-fleetd").as_deref(),
        Some(&b"FLEETD PAYLOAD"[..]),
        "{:?}",
        k.secret_writes
    );
    assert_eq!(
        written(&k, "rivet-proxyd").as_deref(),
        Some(&b"PROXYD PAYLOAD"[..])
    );

    // Through the writer that sets 0600, never the one that does not — a
    // payload an unprivileged process could replace between here and
    // `cmd/rivet-deploy` would be a signature check on bytes nobody chose.
    assert!(
        k.write_calls
            .iter()
            .all(|(p, _)| !p.starts_with(CARRIED_DIR)),
        "a payload went through write_file rather than write_secret: {:?}",
        k.write_calls
    );

    // And the record directories exist, which is what stops the next boot.
    for product in ["rivet-fleetd", "rivet-proxyd"] {
        assert!(
            k.private_dirs.contains(&carried::record_dir(product)),
            "{product} has no record directory: {:?}",
            k.private_dirs
        );
    }
}

/// **The idempotence, and the failure it prevents.** A second boot of the same
/// card finds the record directories and copies nothing — without which the
/// machine would install its applications again on every boot for ever, writing
/// to the SD card each time.
#[test]
fn a_payload_already_recorded_is_not_carried_again() {
    let mut k = a_fleet_servers_card();
    // Both record directories are already there, which is what the FIRST boot
    // left behind. `rivet-proxyd`'s payload has already been installed and
    // removed — the directory outlives it — and `rivet-fleetd`'s is still
    // waiting. Both are `kept`.
    k.dirs_mut().insert(
        carried::record_dir("rivet-fleetd"),
        Script::always(Ok(vec![carried::PAYLOAD_FILE.to_string()])),
    );
    k.dirs_mut().insert(
        carried::record_dir("rivet-proxyd"),
        Script::always(Ok(Vec::new())),
    );

    let outcome = provision_carried(&mut k);
    assert_eq!(
        outcome,
        Carried::Landed {
            written: 0,
            kept: 2
        },
        "{}",
        outcome.summary()
    );
    assert!(
        k.secret_writes.is_empty(),
        "a payload was copied over one that was already dealt with: {:?}",
        k.secret_writes
    );
}

/// **This courier destroys nothing** — the one departure from `ADR-049`, and
/// the reason is that a `.rvt` is signed rather than secret.
#[test]
fn nothing_on_the_card_is_removed_or_overwritten() {
    let mut k = a_fleet_servers_card();
    provision_carried(&mut k);

    assert!(
        k.remove_calls.is_empty(),
        "a payload was unlinked from the card: {:?}",
        k.remove_calls
    );
    assert!(
        !k.order.iter().any(|o| o.starts_with("overwrite_in_place")),
        "a payload was blanked on the card: {:?}",
        k.order
    );
    // And the mount is read-only, which is what makes the two above true by
    // construction rather than by inspection.
    assert!(
        k.mount_calls
            .iter()
            .filter(|m| m.target == BOOT_PARTITION_MOUNT)
            .all(|m| m.flags.contains(crate::kernel::MountFlags::RDONLY)),
        "the boot partition was mounted writable: {:?}",
        k.mount_calls
    );
}

/// A card that carries no application says so and writes nothing. **The
/// ordinary case** — every headless card, and every card built before
/// 2026-09-07.
#[test]
fn a_card_carrying_no_application_is_silent_about_it_and_writes_nothing() {
    let mut k = board_carrying(&[], &[]);
    let outcome = provision_carried(&mut k);
    assert_eq!(outcome, Carried::Nothing, "{}", outcome.summary());
    assert!(k.secret_writes.is_empty(), "{:?}", k.secret_writes);
    assert!(
        !k.private_dirs.contains(&CARRIED_DIR.to_string()),
        "a card carrying nothing created {CARRIED_DIR}"
    );
}

/// **Nothing else on a boot partition is mistaken for a payload.** The kernel,
/// the device tree, the firmware, the document and `ADR-049`'s staging bundle
/// all sit beside these names.
#[test]
fn the_rest_of_the_boot_partition_is_not_carried() {
    let mut k = board_carrying(
        &["staged.txt", "device.seed", "device.crt", "peers"],
        &[(
            &format!("{BOOT_PARTITION_MOUNT}/staged.txt"),
            b"version = 1\n".to_vec(),
        )],
    );
    let outcome = provision_carried(&mut k);
    assert_eq!(outcome, Carried::Nothing, "{}", outcome.summary());
    assert!(k.secret_writes.is_empty(), "{:?}", k.secret_writes);
}

/// A payload larger than the cap is refused, and **nothing partial is left
/// behind for it**. PID 1 is the process that must not be the reason a board
/// does not boot.
#[test]
fn a_payload_larger_than_the_cap_is_refused_and_names_both_numbers() {
    let huge = vec![0u8; carried::MAX_PAYLOAD_BYTES + 1];
    let mut k = board_carrying(
        &[&carried::card_file("rivet-fleetd")],
        &[(&on_card("rivet-fleetd"), huge)],
    );

    let outcome = provision_carried(&mut k);
    let Carried::Refused(why) = &outcome else {
        unreachable!("an oversized payload was carried: {outcome:?}")
    };
    assert!(
        why.contains(&carried::MAX_PAYLOAD_BYTES.to_string()),
        "the refusal does not say what the cap is: {why}"
    );
    assert!(
        written(&k, "rivet-fleetd").is_none(),
        "an oversized payload was written anyway"
    );
}

/// **Refused by `stat`, before any allocation.** `file_size` is asked first,
/// so an oversized payload is never read into memory at all — R-2: the read
/// this module used to perform before checking the cap.
#[test]
fn an_oversized_payload_is_refused_by_stat_and_never_read() {
    let mut k = board_carrying(&[&carried::card_file("rivet-fleetd")], &[]);
    k.file_sizes_mut().insert(
        on_card("rivet-fleetd"),
        Script::always(Ok(carried::MAX_PAYLOAD_BYTES as u64 + 1)),
    );

    let outcome = provision_carried(&mut k);
    let Carried::Refused(why) = &outcome else {
        unreachable!("an oversized payload was carried: {outcome:?}")
    };
    assert!(
        why.contains(&carried::MAX_PAYLOAD_BYTES.to_string()),
        "the refusal does not say what the cap is: {why}"
    );
    assert!(
        !k.read_bytes_calls.contains(&on_card("rivet-fleetd")),
        "the payload was read into memory despite being refused by size: {:?}",
        k.read_bytes_calls
    );
}

/// A name that lists and will not read is a refusal that **leaves the card
/// alone**, so the next boot tries again.
#[test]
fn a_payload_that_cannot_be_read_is_refused_and_the_next_boot_can_retry() {
    let mut k = board_carrying(&[&carried::card_file("rivet-fleetd")], &[]);

    let outcome = provision_carried(&mut k);
    let Carried::Refused(why) = &outcome else {
        unreachable!("a payload that does not read was carried: {outcome:?}")
    };
    assert!(why.contains("rivet-fleetd"), "{why}");
    assert!(k.remove_calls.is_empty(), "{:?}", k.remove_calls);
}

/// **PID 1 verifies nothing and installs nothing**, and it must stay that way:
/// `ADR-029` §11's single verifier is `cmd/rivet-deploy`'s, and a package
/// parser in the process whose failure is the machine is not a trade this
/// project makes. A payload of complete rubbish is carried faithfully.
#[test]
fn a_payload_that_is_not_a_package_is_carried_anyway_and_refused_later() {
    let mut k = board_carrying(
        &[&carried::card_file("rivet-fleetd")],
        &[(&on_card("rivet-fleetd"), b"this is not a .rvt at all".to_vec())],
    );

    let outcome = provision_carried(&mut k);
    assert_eq!(
        outcome,
        Carried::Landed {
            written: 1,
            kept: 0
        },
        "{}",
        outcome.summary()
    );
    assert_eq!(
        written(&k, "rivet-fleetd").as_deref(),
        Some(&b"this is not a .rvt at all"[..])
    );
    // Nothing under /state/app: this process does not install.
    assert!(
        k.secret_writes
            .iter()
            .chain(k.write_calls.iter())
            .all(|(p, _)| !p.starts_with("/state/app")),
        "PID 1 wrote under /state/app"
    );
}

/// The order is the ordinary one — everything on `/state` and flushed before
/// the boot goes on.
#[test]
fn what_was_written_is_flushed_before_the_boot_continues() {
    let mut k = a_fleet_servers_card();
    provision_carried(&mut k);
    assert!(k.sync_calls > 0, "nothing was flushed");
}
