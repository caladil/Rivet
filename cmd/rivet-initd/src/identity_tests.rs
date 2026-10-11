//! What `crate::identity` guarantees, proved rather than asserted.
//!
//! The one that matters most is
//! `the_seed_is_removed_from_the_card_only_after_state_has_it`: a power cut in
//! the wrong order destroys the only copy of a key `ADR-025` §3 says the fleet
//! did not keep.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use crate::identity::{
    CERTIFICATE_FILE, PEERS_FILE, Provisioned, SECRETS_DIR, SEED_FILE, provision_identity,
};
use crate::kernel::{Errno, MountFlags};
use crate::provision::BOOT_PARTITION_MOUNT;
use crate::testkernel::{FakeKernel, Script};
use crate::unbooted::UNBOOTED_PARTITION_MOUNT;

const SEED: [u8; 32] = [7u8; 32];
const CERT: [u8; 6] = [0x30, 0x82, 1, 2, 3, 4];

fn on_card(name: &str) -> String {
    format!("{BOOT_PARTITION_MOUNT}/{name}")
}

fn in_state(name: &str) -> String {
    format!("{SECRETS_DIR}/{name}")
}

/// The same file on the boot partition the firmware did NOT boot. The fake
/// answers reads by path, so the two partitions are told apart by the mount
/// point each is read through.
fn on_unbooted(name: &str) -> String {
    format!("{UNBOOTED_PARTITION_MOUNT}/{name}")
}

/// Every mount of the unbooted partition is matched by an unmount, so it is
/// never left mounted, and it was found where `boot_partition_device` says the
/// other slot's partition is: `board_with` boots `p1`, so the other is `p2`.
fn assert_unbooted_left_unmounted(k: &FakeKernel) {
    let mounts: Vec<_> = k
        .mount_calls
        .iter()
        .filter(|c| c.target == UNBOOTED_PARTITION_MOUNT)
        .collect();
    assert!(
        mounts.iter().all(|c| c.source == "/dev/mmcblk0p2"),
        "the unbooted partition was looked for somewhere else: {mounts:?}"
    );
    let unmounts = k
        .umount_calls
        .iter()
        .filter(|t| *t == UNBOOTED_PARTITION_MOUNT)
        .count();
    assert_eq!(mounts.len(), unmounts, "the unbooted partition was left mounted");
}

/// A machine whose boot partition mounts and which holds whatever the test
/// says. The `/proc/device-tree` read that finds the boot device is the same
/// one `crate::provision`'s own tests script.
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

/// **The whole point.** An identity on the card ends up in `/state/secrets`,
/// the seed goes through the writer that sets `0600`, and the card's copy is
/// gone.
#[test]
fn an_identity_on_the_card_is_installed_and_the_seed_is_blanked() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&on_card(PEERS_FILE), b"aabb\n".to_vec()),
    ]);

    let outcome = provision_identity(&mut k);
    assert_eq!(outcome, Provisioned::Installed, "{}", outcome.summary());

    assert!(
        k.private_dirs.iter().any(|d| d == SECRETS_DIR),
        "the directory was not created privately: {:?}",
        k.private_dirs
    );
    assert_eq!(
        k.secret_writes,
        vec![(in_state(SEED_FILE), SEED.to_vec())],
        "the seed must go through write_secret and nothing else may"
    );
    assert!(
        k.write_calls
            .iter()
            .any(|(p, b)| *p == in_state(CERTIFICATE_FILE) && *b == CERT),
        "the certificate did not reach /state: {:?}",
        k.write_calls.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    assert!(
        k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)),
        "the seed was not removed from the boot partition: {:?}",
        k.remove_calls
    );
    assert!(
        !k.remove_calls
            .iter()
            .any(|p| *p == on_card(CERTIFICATE_FILE)),
        "the certificate was removed and it is not a secret"
    );
}

/// **The ordering guarantee, and the reason this module exists.** A crash
/// between the delete and the copy would destroy the only copy of a key the
/// fleet did not keep (`ADR-025` §3). The seed must be on `/state` and flushed
/// before the card's copy is touched.
#[test]
fn the_seed_is_removed_from_the_card_only_after_state_has_it() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    assert_eq!(provision_identity(&mut k), Provisioned::Installed);

    let wrote = k
        .order
        .iter()
        .position(|step| step == &format!("write_secret {}", in_state(SEED_FILE)))
        .expect("the seed was never written");
    let synced = k
        .order
        .iter()
        .position(|step| step == "sync")
        .expect("nothing was synced");
    let removed = k
        .order
        .iter()
        .position(|step| step == &format!("remove_file {}", on_card(SEED_FILE)))
        .expect("the card's seed was never removed");

    assert!(
        wrote < synced,
        "the seed was written after the sync: {:?}",
        k.order
    );
    assert!(
        synced < removed,
        "the card's copy was destroyed before /state's was flushed: {:?}",
        k.order
    );
}

/// A card with no identity is the ordinary case, not a fault.
#[test]
fn a_card_with_no_identity_is_not_an_error() {
    let mut k = board_with(&[]);
    assert_eq!(provision_identity(&mut k), Provisioned::Nothing);
    assert!(k.secret_writes.is_empty());
    assert!(
        k.remove_calls.is_empty(),
        "nothing should have been deleted"
    );
}

/// **Both halves or neither.** A seed with no certificate is refused, and the
/// seed is LEFT on the card so that fixing it and rebooting completes the job.
#[test]
fn a_seed_without_a_certificate_is_refused_and_the_seed_is_left_alone() {
    let mut k = board_with(&[(&on_card(SEED_FILE), SEED.to_vec())]);
    let outcome = provision_identity(&mut k);
    match &outcome {
        Provisioned::Refused(why) => {
            assert!(why.contains(CERTIFICATE_FILE), "{why}");
            assert!(why.contains("cargo rivet identity new"), "{why}");
            assert!(why.contains("LEFT on the boot partition"), "{why}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(
        k.secret_writes.is_empty(),
        "a half identity was written to /state"
    );
    assert!(
        k.remove_calls.is_empty(),
        "the seed was deleted despite the refusal — that is the unrecoverable case"
    );
}

/// **D15: `/state`'s copy wins, and the card's is LEFT IN PLACE.** A seed on
/// the booted partition that differs from the complete identity `/state`
/// holds is not this machine's key: `/state` keeps its own, the mismatch is
/// said, and a key nobody here holds is never destroyed on the strength of a
/// comparison that failed — the rule the other partition already follows.
#[test]
fn a_different_identity_on_state_is_kept_and_the_cards_seed_left_in_place() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&in_state(SEED_FILE), vec![9u8; 32]),
        (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    let outcome = provision_identity(&mut k);
    assert_eq!(outcome, Provisioned::KeptDifferent, "{}", outcome.summary());
    assert!(
        k.secret_writes.is_empty(),
        "the machine's own identity was overwritten from the card"
    );
    assert!(
        k.write_calls.iter().all(|(p, _)| !p.starts_with(SECRETS_DIR)),
        "the card's identity was written into /state: {:?}",
        k.write_calls
    );
    assert!(
        k.remove_calls.is_empty(),
        "a seed this machine does not hold was destroyed: {:?}",
        k.remove_calls
    );
    assert!(
        !k.write_calls
            .iter()
            .any(|(p, b)| *p == on_card(SEED_FILE) && b.iter().all(|x| *x == 0)),
        "the card's seed was overwritten with zeros"
    );
    assert!(
        k.lines.iter().any(|l| l.contains("does not match")),
        "the mismatch was not said: {:?}",
        k.lines
    );
}

/// A copy that succeeded and a delete that failed says the key is still on the
/// card, in capitals, because that is the state an operator must act on.
#[test]
fn a_delete_that_fails_after_a_good_copy_says_the_key_is_still_on_the_card() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    k.remove_results = Script::always(Err(Errno::EROFS));
    match provision_identity(&mut k) {
        Provisioned::Refused(why) => {
            assert!(why.contains("STILL ON THE BOOT PARTITION"), "{why}");
            assert!(why.contains("next boot will try again"), "{why}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// Every outcome says something an operator could act on.
#[test]
fn every_outcome_summarises_itself() {
    for outcome in [
        Provisioned::Nothing,
        Provisioned::Installed,
        Provisioned::AlreadyHeld,
        Provisioned::KeptDifferent,
        Provisioned::Refused("because".to_string()),
    ] {
        let said = outcome.summary();
        assert!(said.len() > 30, "{outcome:?} says too little: {said}");
    }
}

/// **The measurement, as a test.** `remove_file` alone leaves the bytes in FAT
/// free space — proved on a board on 2026-08-28, where the seed was still
/// readable at offset 13621248 of the boot partition after a successful
/// provision. The zeros must be written, and flushed, BEFORE the unlink.
#[test]
fn the_seed_is_overwritten_with_zeros_before_it_is_unlinked() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    assert_eq!(provision_identity(&mut k), Provisioned::Installed);

    let zeroed = k
        .write_calls
        .iter()
        .find(|(p, _)| *p == on_card(SEED_FILE))
        .expect("the card's seed was never overwritten, only unlinked");
    assert_eq!(
        zeroed.1,
        vec![0u8; SEED.len()],
        "the overwrite must be zeros and the SAME LENGTH, or the tail of the old \
         file stays allocated and untouched"
    );

    let blanked = k
        .order
        .iter()
        .position(|s| s == &format!("overwrite_in_place {}", on_card(SEED_FILE)))
        .expect("no overwrite");
    let removed = k
        .order
        .iter()
        .position(|s| s == &format!("remove_file {}", on_card(SEED_FILE)))
        .expect("no unlink");
    let synced_between = k.order[blanked..removed].iter().any(|s| s == "sync");
    assert!(
        blanked < removed,
        "unlinked before it was blanked: {:?}",
        k.order
    );
    assert!(
        synced_between,
        "nothing flushed the zeros before the unlink, so the erase is hopeful rather than \
         ordered: {:?}",
        k.order
    );
}

/// The already-provisioned path blanks too — it is the same secret on the same
/// card, and an exception there would be the whole defect back again.
#[test]
fn an_already_held_identity_still_blanks_the_cards_copy() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&in_state(SEED_FILE), SEED.to_vec()),
        (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    assert_eq!(provision_identity(&mut k), Provisioned::AlreadyHeld);
    assert!(
        k.write_calls
            .iter()
            .any(|(p, b)| *p == on_card(SEED_FILE) && b.iter().all(|x| *x == 0)),
        "the leftover seed was unlinked without being blanked"
    );
}

// -- B12: an interrupted first boot must not destroy the only good copy -----
//
// `read_bytes(...).is_ok()` used to be the whole test for "already held", so
// a partial write an earlier boot left behind was trusted and the card's
// complete copy was destroyed on the strength of it. Each test below injects
// a failure at one of the four points the repair names, then runs
// `provision_identity` again on the SAME fake kernel as the next boot.

/// **Fault point 1 — after the temporary is created.** A crash the instant
/// after the destination existed, before any byte reached it, leaves an EMPTY
/// file — the shape a plain `create`-and-truncate write_secret used to use
/// would produce from a crash at this exact point.
#[test]
fn a_crash_right_after_the_seed_file_is_created_is_not_mistaken_for_already_held() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&in_state(SEED_FILE), Vec::new()),
    ]);
    // This boot's own attempt to complete the recovery also fails, so the
    // property under test is that the card survives even a REPEATED
    // interruption, not just a single clean retry.
    k.write_results = Script::sequence(vec![Err(Errno::EIO)], Ok(()));

    let first = provision_identity(&mut k);
    assert!(
        matches!(first, Provisioned::Refused(_)),
        "an empty existing seed was trusted as already held: {first:?}"
    );
    assert!(
        k.remove_calls.is_empty(),
        "the card's complete seed was destroyed while /state's copy was still empty: {:?}",
        k.remove_calls
    );

    let second = provision_identity(&mut k);
    assert_eq!(second, Provisioned::Installed, "{}", second.summary());
    assert!(
        k.secret_writes
            .iter()
            .any(|(p, b)| *p == in_state(SEED_FILE) && *b == SEED.to_vec()),
        "the full seed was not (re)written: {:?}",
        k.secret_writes
    );
    assert!(k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)));
}

/// **Fault point 2 — during the write.** The same shape one step further in: a
/// PARTIAL seed, not an empty one.
#[test]
fn a_crash_mid_write_of_the_seed_file_is_not_mistaken_for_already_held() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&in_state(SEED_FILE), SEED[..16].to_vec()),
    ]);
    k.write_results = Script::sequence(vec![Err(Errno::EIO)], Ok(()));

    let first = provision_identity(&mut k);
    assert!(
        matches!(first, Provisioned::Refused(_)),
        "a half-written seed was trusted as already held: {first:?}"
    );
    assert!(k.remove_calls.is_empty(), "{:?}", k.remove_calls);

    let second = provision_identity(&mut k);
    assert_eq!(second, Provisioned::Installed, "{}", second.summary());
    assert!(
        k.secret_writes
            .iter()
            .any(|(p, b)| *p == in_state(SEED_FILE) && *b == SEED.to_vec()),
        "the full seed was not (re)written: {:?}",
        k.secret_writes
    );
    assert!(k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)));
}

/// **Fault point 3 — after the seed is published.** The seed reached `/state`
/// correctly and completely; the boot was interrupted before the certificate
/// arrived. The pre-B12 `AlreadyHeld` check never looked past the seed, so it
/// would have blanked the card's seed here and left `/state` with a seed and
/// no certificate — a machine that cannot serve TLS, and cannot recover
/// either half from the card, because the card's copy would now be gone too.
#[test]
fn a_seed_published_with_no_certificate_yet_does_not_blank_the_cards_seed() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&in_state(SEED_FILE), SEED.to_vec()),
    ]);
    // The seed already matches the card, so the FIRST write this boot makes is
    // the certificate's.
    k.write_results = Script::sequence(vec![Err(Errno::EIO)], Ok(()));

    let first = provision_identity(&mut k);
    assert!(
        matches!(first, Provisioned::Refused(_)),
        "a complete seed with no certificate in /state was treated as a finished identity: \
         {first:?}"
    );
    assert!(
        k.remove_calls.is_empty(),
        "the card's seed was blanked while /state still had no certificate: {:?}",
        k.remove_calls
    );
    assert!(
        k.secret_writes.iter().all(|(p, _)| *p != in_state(SEED_FILE)),
        "a seed that already matched the card was rewritten anyway: {:?}",
        k.secret_writes
    );

    let second = provision_identity(&mut k);
    assert_eq!(second, Provisioned::Installed, "{}", second.summary());
    assert!(
        k.write_calls
            .iter()
            .any(|(p, b)| *p == in_state(CERTIFICATE_FILE) && *b == CERT.to_vec()),
        "the certificate never reached /state: {:?}",
        k.write_calls
    );
    assert!(k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)));
}

/// **Fault point 4 — before the certificate is published**, tested one step
/// later than fault point 3 so the two are not the same call under a
/// different name: the seed AND the certificate are already correct in
/// `/state`, and only the (optional) peers file has not arrived yet.
#[test]
fn a_seed_and_certificate_published_with_peers_still_pending_does_not_blank_the_cards_seed() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&on_card(PEERS_FILE), b"aabb\n".to_vec()),
        (&in_state(SEED_FILE), SEED.to_vec()),
        (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    // Both already match, so the FIRST write this boot makes is the peers
    // file's.
    k.write_results = Script::sequence(vec![Err(Errno::EIO)], Ok(()));

    let first = provision_identity(&mut k);
    assert!(
        matches!(first, Provisioned::Refused(_)),
        "a complete seed and certificate with no peers file yet was treated as finished: \
         {first:?}"
    );
    assert!(
        k.remove_calls.is_empty(),
        "the card's seed was blanked while /state still had no peers file: {:?}",
        k.remove_calls
    );

    let second = provision_identity(&mut k);
    assert_eq!(second, Provisioned::Installed, "{}", second.summary());
    assert!(
        k.write_calls
            .iter()
            .any(|(p, b)| *p == in_state(PEERS_FILE) && *b == b"aabb\n".to_vec()),
        "the peers file never reached /state: {:?}",
        k.write_calls
    );
    assert!(k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)));
}

/// **Also cover: a complete identity that equals the card's.** The whole set
/// already matches, so nothing is rewritten — proving the `Matches` path is a
/// true no-op and not an unconditional rewrite that merely happens to write
/// the same bytes back.
#[test]
fn a_seed_and_certificate_that_already_match_the_card_are_not_rewritten() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&in_state(SEED_FILE), SEED.to_vec()),
        (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
    ]);

    let outcome = provision_identity(&mut k);
    assert_eq!(outcome, Provisioned::AlreadyHeld, "{}", outcome.summary());
    assert!(k.secret_writes.is_empty(), "{:?}", k.secret_writes);
    assert!(
        k.write_calls.iter().all(|(p, _)| !p.starts_with(SECRETS_DIR)),
        "an identity that already matched the card was rewritten into /state: {:?}",
        k.write_calls
    );
    assert!(k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)));
}

// -- F01: the boot partition this boot did NOT come from ---------------------
//
// `cmd/rivet-image` writes the identity onto both boot partitions. These model
// the second one through its own mount point, with `/state` answering ENOENT
// to the first boot's own comparison and holding the identity once installed.

/// **(a) Neither boot partition keeps the private key once `/state` has it.**
/// The first boot installs from the partition it booted and then blanks, by
/// the same zero-then-unlink, the copy on the other one — after `/state`'s
/// copy is written and flushed, never before.
#[test]
fn the_unbooted_partitions_seed_is_blanked_once_state_holds_the_identity() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
        (&on_card(PEERS_FILE), b"aabb\n".to_vec()),
        (&on_unbooted(SEED_FILE), SEED.to_vec()),
        (&on_unbooted(CERTIFICATE_FILE), CERT.to_vec()),
        (&on_unbooted(PEERS_FILE), b"aabb\n".to_vec()),
    ]);
    k.blobs_mut().insert(
        in_state(SEED_FILE),
        Script::sequence(vec![Err(Errno::ENOENT)], Ok(SEED.to_vec())),
    );
    k.blobs_mut().insert(
        in_state(CERTIFICATE_FILE),
        Script::sequence(vec![Err(Errno::ENOENT)], Ok(CERT.to_vec())),
    );

    assert_eq!(provision_identity(&mut k), Provisioned::Installed);

    assert!(
        k.write_calls
            .iter()
            .any(|(p, b)| *p == on_unbooted(SEED_FILE) && *b == vec![0u8; SEED.len()]),
        "the unbooted partition's seed was not overwritten with zeros: {:?}",
        k.order
    );
    let installed = k
        .order
        .iter()
        .position(|s| s == &format!("write_secret {}", in_state(SEED_FILE)))
        .expect("the seed was never installed");
    let blanked = k
        .order
        .iter()
        .position(|s| s == &format!("overwrite_in_place {}", on_unbooted(SEED_FILE)))
        .expect("the unbooted copy was never blanked");
    let removed = k
        .order
        .iter()
        .position(|s| s == &format!("remove_file {}", on_unbooted(SEED_FILE)))
        .expect("the unbooted copy was never unlinked");
    assert!(installed < blanked && blanked < removed, "{:?}", k.order);
    assert!(k.order[installed..blanked].iter().any(|s| s == "sync"), "{:?}", k.order);
    assert!(
        k.mount_calls.iter().any(|c| c.target == UNBOOTED_PARTITION_MOUNT
            && !c.flags.contains(MountFlags::RDONLY)),
        "the unbooted partition was never mounted writable to remove the seed"
    );
    assert_unbooted_left_unmounted(&k);
}

/// **A later boot of an already-provisioned machine finishes the job**, for a
/// card whose second copy outlived the first boot.
#[test]
fn a_later_boot_blanks_a_seed_left_on_the_unbooted_partition() {
    let mut k = board_with(&[
        (&on_unbooted(SEED_FILE), SEED.to_vec()),
        (&in_state(SEED_FILE), SEED.to_vec()),
        (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    assert_eq!(provision_identity(&mut k), Provisioned::Nothing);
    assert!(
        k.remove_calls.iter().any(|p| *p == on_unbooted(SEED_FILE)),
        "a private key /state already holds was left on the unbooted partition: {:?}",
        k.remove_calls
    );
    assert_unbooted_left_unmounted(&k);
}

/// **The mismatch rule, on the second partition.** A seed there that differs
/// from the complete identity `/state` holds is not this machine's key: it is
/// left in place, the mismatch is said, and the partition is never mounted
/// writable for it.
#[test]
fn a_seed_on_the_unbooted_partition_that_differs_from_states_is_left_in_place() {
    let mut k = board_with(&[
        (&on_unbooted(SEED_FILE), SEED.to_vec()),
        (&in_state(SEED_FILE), vec![9u8; 32]),
        (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    assert_eq!(provision_identity(&mut k), Provisioned::Nothing);
    assert!(
        !k.remove_calls.iter().any(|p| *p == on_unbooted(SEED_FILE)),
        "a seed /state does not hold was destroyed: {:?}",
        k.remove_calls
    );
    assert!(
        k.mount_calls
            .iter()
            .filter(|c| c.target == UNBOOTED_PARTITION_MOUNT)
            .all(|c| c.flags.contains(MountFlags::RDONLY)),
        "the unbooted partition was mounted writable for a seed it keeps"
    );
    assert!(
        k.lines.iter().any(|l| l.contains("does not match")),
        "the mismatch was not said: {:?}",
        k.lines
    );
    assert_unbooted_left_unmounted(&k);
}

/// **And a seed `/state` does not hold at all may be the last copy**, so a boot
/// whose own provisioning was refused leaves the other partition's alone.
#[test]
fn a_seed_on_the_unbooted_partition_is_kept_while_state_holds_no_identity() {
    let mut k = board_with(&[
        (&on_card(SEED_FILE), SEED.to_vec()),
        (&on_unbooted(SEED_FILE), SEED.to_vec()),
        (&on_unbooted(CERTIFICATE_FILE), CERT.to_vec()),
    ]);
    assert!(matches!(provision_identity(&mut k), Provisioned::Refused(_)));
    assert!(
        k.remove_calls.is_empty(),
        "a seed was destroyed while /state held none: {:?}",
        k.remove_calls
    );
    assert_unbooted_left_unmounted(&k);
}

/// **(b) A later first boot of the other slot never restores a stale `peers`.**
/// The machine's seed and certificate match the card, and its peers file has
/// changed since the card was written — an enrollment or a revocation. The
/// card's copy is NOT installed over it, and the card's seed is still blanked.
#[test]
fn a_matching_seed_never_lets_the_cards_peers_replace_the_machines_own() {
    for live in [b"ccdd\n".to_vec(), b"aabb\nccdd\n".to_vec()] {
        let mut k = board_with(&[
            (&on_card(SEED_FILE), SEED.to_vec()),
            (&on_card(CERTIFICATE_FILE), CERT.to_vec()),
            (&on_card(PEERS_FILE), b"aabb\n".to_vec()),
            (&in_state(SEED_FILE), SEED.to_vec()),
            (&in_state(CERTIFICATE_FILE), CERT.to_vec()),
            (&in_state(PEERS_FILE), live.clone()),
        ]);
        let outcome = provision_identity(&mut k);
        assert_eq!(outcome, Provisioned::AlreadyHeld, "{}", outcome.summary());
        assert!(
            k.write_calls.iter().all(|(p, _)| *p != in_state(PEERS_FILE)),
            "the card's stale peers file replaced the machine's live one: {:?}",
            k.write_calls
        );
        assert!(
            k.remove_calls.iter().any(|p| *p == on_card(SEED_FILE)),
            "the card's seed was left behind: {:?}",
            k.remove_calls
        );
    }
}
