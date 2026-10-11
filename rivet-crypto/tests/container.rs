//! The container, driven with the **real** algorithms.
//!
//! `rivet-core`'s suite proves `ADR-029` §5's ordering and refusals against
//! fakes, which is what makes the property *"nothing is written before
//! verification"* testable at all (`ADR-029` §10). It proves nothing about
//! Ed25519 or BLAKE3. This file closes that gap: every refusal that matters is
//! driven again over bytes a real signer produced and a real verifier rejected.
//!
//! # Key material
//!
//! `AGENTS.md` §8.3 — the suite mints its own. The seed below is derived by
//! hashing a label, in memory; **nothing is written to disk and nothing is
//! real**, which is a stronger form of the rule than a temporary file, because
//! there is no file to leave behind.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use rivet_core::package::update::PRODUCT;
use rivet_core::package::error::PackageError;
use rivet_core::package::format::{HEADER_BYTES, SIGNATURE_OFFSET, SIGNED_BYTES};
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest, MaySign, PublicKey, Sign, TrustSet};
use rivet_core::package::read::{Package, open};
use rivet_core::package::update::{Payload, SlotVariance};
use rivet_core::package::write::{Builder, NewMember};
use rivet_core::state::Slot;
use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier, SEED_BYTES};

/// What the Raspberry Pi reserves on a boot-directory target — `ADR-029` §4.
/// Spelled here rather than imported so that this file does not depend on the
/// boot adapter; the drift guard that ties the two together lives in
/// `rivet-adapter-image`, which can see both.
const RESERVED: &[&str] = &["autoboot.txt", "rivet.toml"];

fn test_seed(label: &str) -> [u8; SEED_BYTES] {
    let mut digest = Blake3::new();
    digest.of(label.as_bytes())
}

fn signer() -> Ed25519Signer {
    Ed25519Signer::from_seed(&test_seed(
        "rivet container suite signing seed — not a key, never written down",
    ))
}

fn trust_for(key: PublicKey) -> TrustSet {
    let mut digest = Blake3::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        key,
        "test-update",
        MaySign::these(&[Kind::update()]),
    );
    trust
}

fn trust() -> TrustSet {
    trust_for(signer().public_key())
}

fn read_with(bytes: &[u8], trust: &TrustSet) -> Result<Package, PackageError> {
    let mut digest = Blake3::new();
    let verifier = Ed25519Verifier::new();
    open(bytes, bytes.len() as u64, &mut digest, &verifier, trust)
}

fn read(bytes: &[u8]) -> Result<Package, PackageError> {
    read_with(bytes, &trust())
}

/// `ADR-029` §6's member set, at a hundredth of the size: one whole partition,
/// three invariant boot files, and `cmdline.txt` once per slot.
fn members() -> Vec<(&'static str, u8, u8, Vec<u8>)> {
    vec![
        ("system.img", 0, 0, vec![0x21; 64 * 1024]),
        ("kernel8.img", 1, 0, vec![0x11; 8192]),
        ("config.txt", 1, 0, b"arm_64bit=1\ntryboot_a_b=1\n".to_vec()),
        (
            "cmdline.txt",
            1,
            SlotVariance::SlotA.as_byte(),
            b"root=PARTLABEL=system-a rootfstype=ext4 cgroup_enable=memory\n".to_vec(),
        ),
        (
            "cmdline.txt",
            1,
            SlotVariance::SlotB.as_byte(),
            b"root=PARTLABEL=system-b rootfstype=ext4 cgroup_enable=memory\n".to_vec(),
        ),
    ]
}

fn payload_bytes() -> Vec<u8> {
    let mut digest = Blake3::new();
    let mut signer = signer();
    let mut builder = Builder::new(PRODUCT, "0.2.0", "aarch64", Some("rpi4"), Kind::update())
        .unwrap()
        .reserving(RESERVED)
        .build_epoch(1_755_000_000);
    let set = members();
    for (name, class, variant, bytes) in &set {
        builder
            .add(NewMember::stored(name, *class, *variant, bytes, &mut digest).unwrap())
            .unwrap();
    }
    let slices: Vec<&[u8]> = set.iter().map(|(_, _, _, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .unwrap()
}

/// Re-sign after editing the header, so a mutation to the manifest reaches the
/// checks below `ADR-029` §5 step 7 instead of stopping at it.
fn resign(bytes: &mut [u8]) {
    let signature = signer().sign(&bytes[..SIGNED_BYTES]);
    bytes[SIGNATURE_OFFSET..HEADER_BYTES].copy_from_slice(&signature.0);
}

// ---------------------------------------------------------------------------

#[test]
fn a_real_payload_verifies_end_to_end_and_every_member_matches() {
    let bytes = payload_bytes();
    let package = read(&bytes).unwrap();
    let payload = Payload::open(&package, RESERVED).unwrap();
    assert!(payload.accepts(PRODUCT, "aarch64", "rpi4").is_ok());

    let mut digest = Blake3::new();
    for (index, (name, _, _, content)) in members().iter().enumerate() {
        // Streamed in 4 KiB chunks, which is how a real installer reads it.
        let mut stream = package.member_stream(index, &mut digest).unwrap();
        for chunk in content.chunks(4096) {
            stream.write(chunk).unwrap();
        }
        assert_eq!(stream.finish().unwrap().member().name(), *name);
    }
}

/// The whole point of `ADR-029` §5: a payload signed by a key the machine does
/// not hold is refused, and it is refused **at the table lookup**, before the
/// signature is even computed.
#[test]
fn a_payload_signed_by_another_key_is_refused() {
    let mut digest = Blake3::new();
    let mut other = Ed25519Signer::from_seed(&test_seed("a key this machine never trusted"));
    let mut builder =
        Builder::new(PRODUCT, "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    builder
        .add(NewMember::stored("system.img", 0, 0, &[9u8; 32], &mut digest).unwrap())
        .unwrap();
    let bytes = builder
        .seal_in_memory(&mut digest, &mut other, &[&[9u8; 32]])
        .unwrap();

    assert!(matches!(
        read(&bytes),
        Err(PackageError::UntrustedKey { .. })
    ));

    // And it verifies perfectly well against a machine that *does* trust it,
    // so the refusal above is about trust rather than about the bytes.
    assert!(read_with(&bytes, &trust_for(other.public_key())).is_ok());
}

/// Every one of these mutations is inside the signed region, so a real Ed25519
/// signature is what catches them.
#[test]
fn ed25519_catches_a_change_to_any_part_of_the_signed_header() {
    let reference = payload_bytes();
    for at in [
        0usize,
        8,
        16,
        24,
        56,
        64,
        80,
        160,
        168,
        200,
        SIGNED_BYTES - 1,
    ] {
        let mut bytes = reference.clone();
        bytes[at] ^= 0x01;
        let outcome = read(&bytes);
        assert!(
            outcome.is_err(),
            "flipping a bit at {at} of the signed header was accepted"
        );
        // The first four offsets are checked *before* the signature is, so the
        // refusal there names the field rather than the signature. Everything
        // from `key_id` on reaches the signature check.
        if at >= 56 {
            assert!(
                matches!(outcome, Err(PackageError::BadSignature { .. })),
                "at {at} the refusal was {outcome:?} rather than a signature failure"
            );
        }
    }
}

/// BLAKE3 catches a change to a member's content, and the container reports
/// which member.
#[test]
fn blake3_catches_a_change_to_a_members_content() {
    let bytes = payload_bytes();
    let package = read(&bytes).unwrap();
    let mut digest = Blake3::new();
    let set = members();

    for (index, member) in set.iter().enumerate() {
        let mut content = member.3.clone();
        content[0] ^= 0x80;
        assert!(
            matches!(
                package.verify_member(index, &mut digest, &content),
                Err(PackageError::MemberDigestMismatch { .. })
            ),
            "member {index} accepted altered content"
        );
    }
}

/// `AGENTS.md` §4: *"break it and watch the test fail."* The guard being broken
/// here is the digest comparison itself — a member checked against **another
/// member's** signed record. Both are legitimate bytes from the same signed
/// payload, so nothing but the per-member digest can tell them apart.
#[test]
fn a_member_checked_against_another_members_record_is_refused() {
    let bytes = payload_bytes();
    let package = read(&bytes).unwrap();
    let mut digest = Blake3::new();
    let set = members();
    assert!(matches!(
        package.verify_member(3, &mut digest, &set[4].3),
        Err(PackageError::MemberDigestMismatch { index: 3, .. })
    ));
}

#[test]
fn a_truncated_transfer_is_refused_with_the_signature_still_good() {
    let bytes = payload_bytes();
    let mut digest = Blake3::new();
    let verifier = Ed25519Verifier::new();
    let short = bytes.len() as u64 - 1;
    let outcome = open(&bytes, short, &mut digest, &verifier, &trust());
    assert!(matches!(
        outcome,
        Err(PackageError::MemberOverflowsFile { .. })
    ));
}

#[test]
fn a_reserved_name_refuses_the_whole_payload_under_real_signatures() {
    let mut digest = Blake3::new();
    let mut signer = signer();
    // Built deliberately without `reserving`, which is how a builder that had
    // not asked the port would produce one.
    let mut builder =
        Builder::new(PRODUCT, "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    let system = vec![0x21u8; 1024];
    let autoboot = b"[all]\nboot_partition=1\n".to_vec();
    builder
        .add(NewMember::stored("system.img", 0, 0, &system, &mut digest).unwrap())
        .unwrap();
    builder
        .add(NewMember::stored("autoboot.txt", 1, 0, &autoboot, &mut digest).unwrap())
        .unwrap();
    let bytes = builder
        .seal_in_memory(&mut digest, &mut signer, &[&system, &autoboot])
        .unwrap();

    // Correctly signed, structurally perfect, and refused whole.
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::ReservedMemberName {
            index: 1,
            name: "autoboot.txt".to_string()
        }
    );
}

/// A legitimately re-signed header carrying incoherent metadata: the signature
/// is real, so this exercises the manifest's own refusals rather than the
/// verifier's.
#[test]
fn a_correctly_signed_but_incoherent_manifest_is_still_refused() {
    let mut bytes = payload_bytes();
    // `member_count` = 41, above the format's ceiling.
    bytes[56..60].copy_from_slice(&41u32.to_le_bytes());
    resign(&mut bytes);
    assert_eq!(
        read(&bytes),
        Err(PackageError::TooManyMembers { found: 41 })
    );
}

#[test]
fn each_slot_gets_its_own_command_line_and_the_same_kernel() {
    let bytes = payload_bytes();
    let package = read(&bytes).unwrap();
    let payload = Payload::open(&package, RESERVED).unwrap();

    for (slot, wanted) in [(Slot::A, "system-a"), (Slot::B, "system-b")] {
        let names: Vec<&str> = payload.boot_members_for(slot).map(|m| m.name()).collect();
        assert_eq!(names, vec!["kernel8.img", "config.txt", "cmdline.txt"]);

        let cmdline = payload
            .boot_members_for(slot)
            .find(|m| m.name() == "cmdline.txt")
            .unwrap();
        let start = cmdline.offset() as usize;
        let text = String::from_utf8(bytes[start..start + cmdline.stored_len() as usize].to_vec())
            .unwrap();
        assert!(text.contains(wanted), "slot {slot:?} was given {text:?}");
    }
}

/// `ADR-029` §9.2 step 8's skip-if-identical, which §9 calls a **safety**
/// property rather than an optimisation: the usual update never opens the FAT
/// filesystem at all, which is most of what bounds F5.
///
/// The comparison is a digest against a digest, so this is the whole of it.
#[test]
fn a_member_already_on_the_target_is_identifiable_without_writing_anything() {
    let bytes = payload_bytes();
    let package = read(&bytes).unwrap();
    let payload = Payload::open(&package, RESERVED).unwrap();
    let mut digest = Blake3::new();

    let kernel = payload
        .boot_members_for(Slot::B)
        .find(|m| m.name() == "kernel8.img")
        .unwrap();

    // What is already on the target, as an installer would read it back.
    let on_target = vec![0x11u8; 8192];
    assert_eq!(&digest.of(&on_target), kernel.digest());

    let stale = vec![0x11u8; 8191];
    assert_ne!(&digest.of(&stale), kernel.digest());
}
