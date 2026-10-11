//! The container's suite — **and most of it is deliberately not a round trip.**
//!
//! `AGENTS.md` §4:
//!
//! > A test that round-trips a reader against its own writer proves the two
//! > agree with each other and nothing about the format.
//!
//! That matters more here than anywhere else in this repository, because the
//! reader is the security boundary (`ADR-028` §2). So exactly one test below is
//! a round trip, it says so, and everything else drives bytes the writer did
//! **not** produce: hand-built headers, truncations, single-bit flips, reordered
//! member records, a record claiming a length the file does not have, duplicate
//! names, a path traversal, a signature over different bytes, and an empty
//! archive.
//!
//! **Provenance:** every vector here is hand-composed, and that is the ceiling
//! on what they prove. There is no captured `.rvt` from another implementation
//! to test against, because there is no other implementation — `ADR-029` was
//! accepted on 2026-08-23 and this is the first code to read the format.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::error::PackageError;
use super::format::{
    DIGEST_BYTES, HEADER_BYTES, M_COMPRESSION, M_DIGEST, M_NAME, M_OFFSET, M_PLAIN_LEN,
    M_RESERVED_0, M_STORED_LEN, M_TARGET_CLASS, M_VARIANT, MEMBER_RECORD_BYTES,
    MEMBER_TABLE_OFFSET, OFF_ARCH, OFF_FLAGS, OFF_FORMAT_VERSION, OFF_HEADER_BYTES,
    OFF_HEADER_RESERVED, OFF_KEY_ID, OFF_MEMBER_COUNT, OFF_PLATFORM, OFF_SIG_ALG, SIGNATURE_OFFSET,
    SIGNED_BYTES, TextFault,
};
use super::kind::Kind;
use super::port::{Digest, DigestAlgorithm, KeyId, Sign, SignatureAlgorithm, TrustSet};
use super::read::{COMPRESSION_ZSTD, Package, open};
use super::testing::{Event, ToyDigest, ToyKey, ToySigner, ToyVerifier, Trace};
use super::update::{Payload, SlotVariance, TargetClass};
use super::write::{Builder, NewMember};
use crate::package::port::MaySign;
use crate::state::Slot;

// ---------------------------------------------------------------------------
// Fixtures. Hand-composed — see the module documentation.
// ---------------------------------------------------------------------------

const SIGNING_KEY: ToyKey = ToyKey(0x9c);
const RESERVED: &[&str] = &["autoboot.txt", "rivet.toml"];

fn trust() -> TrustSet {
    let mut digest = ToyDigest::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        SIGNING_KEY.public(),
        "test-update",
        MaySign::these(&[Kind::update()]),
    );
    trust
}

/// Read with a trust set that permits `kinds`, so a test about a *later*
/// refusal reaches it.
///
/// Added 2026-08-24 with `ADR-028` §5.5's entitlement check. There are now two
/// refusals about kind and they are at different layers: this crate refuses a
/// key that **may not sign** a kind, before anything is interpreted, and the
/// `update`/`application` readers refuse a payload that **is not** the kind
/// they read. A test about the second has to get past the first, and saying so
/// with a fixture is better than weakening the check to keep the test quiet.
fn read_trusting(bytes: &[u8], kinds: &[Kind]) -> Result<Package, PackageError> {
    let mut digest = ToyDigest::new();
    let verifier = ToyVerifier::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        SIGNING_KEY.public(),
        "test-any",
        MaySign::these(kinds),
    );
    open(
        &bytes[..HEADER_BYTES],
        bytes.len() as u64,
        &mut digest,
        &verifier,
        &trust,
    )
}

fn read(bytes: &[u8]) -> Result<Package, PackageError> {
    let mut digest = ToyDigest::new();
    let verifier = ToyVerifier::new();
    open(bytes, bytes.len() as u64, &mut digest, &verifier, &trust())
}

/// The member set an `ADR-029` §6 payload actually has: one whole partition,
/// three invariant boot files, and `cmdline.txt` twice — once per slot, which is
/// §3's measured 136 bytes of duplication.
fn member_bytes() -> Vec<(&'static str, u8, u8, Vec<u8>)> {
    vec![
        ("system.img", 0, 0, vec![0xab; 4096]),
        ("kernel8.img", 1, 0, vec![0x11; 700]),
        ("config.txt", 1, 0, b"arm_64bit=1\n".to_vec()),
        (
            "cmdline.txt",
            1,
            SlotVariance::SlotA.as_byte(),
            b"root=PARTLABEL=system-a\n".to_vec(),
        ),
        (
            "cmdline.txt",
            1,
            SlotVariance::SlotB.as_byte(),
            b"root=PARTLABEL=system-b\n".to_vec(),
        ),
    ]
}

/// The same payload as [`sample`] but declaring a different `kind`.
///
/// Signed by [`SIGNING_KEY`] — the key the trust set holds — so everything
/// except the entitlement passes.
fn sample_of_kind(kind: Kind) -> Vec<u8> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let mut builder = Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), kind)
        .unwrap()
        .reserving(RESERVED);
    let members = member_bytes();
    for (name, class, variant, bytes) in &members {
        builder
            .add(NewMember::stored(name, *class, *variant, bytes, &mut digest).unwrap())
            .unwrap();
    }
    let slices: Vec<&[u8]> = members.iter().map(|(_, _, _, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .unwrap()
}

fn sample() -> Vec<u8> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let mut builder = Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update())
        .unwrap()
        .reserving(RESERVED);
    let members = member_bytes();
    for (name, class, variant, bytes) in &members {
        builder
            .add(NewMember::stored(name, *class, *variant, bytes, &mut digest).unwrap())
            .unwrap();
    }
    let slices: Vec<&[u8]> = members.iter().map(|(_, _, _, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .unwrap()
}

/// Re-sign the header in place, so a mutation to the *manifest* is a legitimate
/// signature over incoherent metadata rather than a broken signature.
///
/// This is how the manifest's own refusals are reached at all: everything below
/// `ADR-029` §5 step 7 is unreachable until the signature checks out.
fn resign(bytes: &mut [u8]) {
    let mut signer = ToySigner::new(SIGNING_KEY);
    let signature = signer.sign(&bytes[..SIGNED_BYTES]);
    bytes[SIGNATURE_OFFSET..HEADER_BYTES].copy_from_slice(&signature.0);
}

/// A payload whose header was edited and then legitimately re-signed.
fn hand_built(mutate: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut bytes = sample();
    mutate(&mut bytes);
    resign(&mut bytes);
    bytes
}

/// Offset of member `index`'s record inside the header.
fn record_at(index: usize) -> usize {
    MEMBER_TABLE_OFFSET + index * MEMBER_RECORD_BYTES
}

// ---------------------------------------------------------------------------
// One round trip, and it is labelled for what it is.
// ---------------------------------------------------------------------------

/// **This proves the writer and the reader agree with each other and nothing
/// about the format.** It is here so the rest of the suite has a fixture, and
/// because a writer that could not produce a readable file would be worth
/// finding in one assertion rather than forty.
#[test]
fn the_writer_and_the_reader_agree_which_is_all_a_round_trip_proves() {
    let bytes = sample();
    let package = read(&bytes).unwrap();

    assert_eq!(package.header().product(), "rivet");
    assert_eq!(package.header().version(), "0.2.0");
    assert_eq!(package.header().arch(), "aarch64");
    assert_eq!(package.header().platform(), Some("rpi4"));
    assert_eq!(package.header().kind(), &Kind::update());
    assert_eq!(
        package.header().signature_algorithm(),
        SignatureAlgorithm::ED25519
    );
    assert_eq!(package.header().digest_algorithm(), DigestAlgorithm::BLAKE3);
    assert_eq!(package.members().len(), 5);
    assert_eq!(package.members()[0].offset(), HEADER_BYTES as u64);
    assert_eq!(package.file_len(), bytes.len() as u64);

    let mut digest = ToyDigest::new();
    for (index, (name, _, _, content)) in member_bytes().iter().enumerate() {
        let verified = package.verify_member(index, &mut digest, content).unwrap();
        assert_eq!(verified.member().name(), *name);
    }
}

/// `ADR-029` §8's own example name, composed from the header the writer wrote.
#[test]
fn a_payload_names_itself_the_way_adr_027_3s_grammar_does() {
    let package = read(&sample()).unwrap();
    let h = package.header();
    let name = format!(
        "{}-{}-{}-{}-{}{}",
        h.product(),
        h.version(),
        h.arch(),
        h.platform().unwrap_or("noplatform"),
        h.kind(),
        super::kind::SUFFIX
    );
    assert_eq!(name, "rivet-0.2.0-aarch64-rpi4-update.rvt");
}

// ---------------------------------------------------------------------------
// ADR-029 §5 steps 1-7. Bytes the writer did not produce.
// ---------------------------------------------------------------------------

#[test]
fn step_1_a_file_shorter_than_the_header_is_refused_before_anything_else() {
    let bytes = sample();
    for len in [0usize, 1, 8, HEADER_BYTES - 1] {
        let mut digest = ToyDigest::new();
        let verifier = ToyVerifier::new();
        let outcome = open(&bytes[..len], len as u64, &mut digest, &verifier, &trust());
        assert_eq!(
            outcome,
            Err(PackageError::TooShortForHeader {
                file_len: len as u64
            }),
            "a {len}-byte file was not refused as too short"
        );
    }
}

/// The truncated-transfer case (`ADR-029` F1) with a *whole* header present: the
/// header is intact and the members are not all there.
#[test]
fn step_1s_sibling_a_truncated_transfer_is_refused_with_nothing_written() {
    let bytes = sample();
    for cut in [HEADER_BYTES, HEADER_BYTES + 1, bytes.len() - 1] {
        let mut digest = ToyDigest::new();
        let verifier = ToyVerifier::new();
        let outcome = open(&bytes, cut as u64, &mut digest, &verifier, &trust());
        assert!(
            matches!(outcome, Err(PackageError::MemberOverflowsFile { .. })),
            "a file truncated to {cut} gave {outcome:?}"
        );
    }
}

#[test]
fn a_caller_that_offers_a_short_buffer_is_told_so_rather_than_told_the_file_is_short() {
    let bytes = sample();
    let mut digest = ToyDigest::new();
    let verifier = ToyVerifier::new();
    let outcome = open(
        &bytes[..100],
        bytes.len() as u64,
        &mut digest,
        &verifier,
        &trust(),
    );
    assert_eq!(outcome, Err(PackageError::ShortHeaderBuffer { given: 100 }));
}

#[test]
fn step_2_anything_that_is_not_a_package_is_refused_at_byte_zero() {
    // Not built by the writer at all: eight bytes of somebody else's format.
    let mut bytes = vec![0u8; HEADER_BYTES];
    bytes[..8].copy_from_slice(b"\x7fELF\x02\x01\x01\x00");
    assert_eq!(
        read(&bytes),
        Err(PackageError::NotAPackage {
            found: *b"\x7fELF\x02\x01\x01\x00"
        })
    );

    // And one flipped bit in the magic of a real payload.
    let mut bytes = sample();
    bytes[7] ^= 0x01;
    assert!(matches!(
        read(&bytes),
        Err(PackageError::NotAPackage { .. })
    ));
}

#[test]
fn step_3_an_unimplemented_format_version_is_refused() {
    // 1 is in this list since 2026-08-24 and it is the point of the test:
    // `product` grew, every field after it moved, and a version-1 header is
    // not a header this reader may guess at. `ADR-029` §5 step 3.
    for version in [0u32, 1, 3, 0xffff_ffff] {
        let bytes = hand_built(|b| {
            b[OFF_FORMAT_VERSION..OFF_FORMAT_VERSION + 4].copy_from_slice(&version.to_le_bytes());
        });
        assert_eq!(
            read(&bytes),
            Err(PackageError::UnsupportedFormatVersion { found: version })
        );
    }
}

/// `ADR-029` §5 step 4: *"checked against the constant and never used to size a
/// read"*. The hostile value is the one that would be an allocation if it were
/// obeyed.
#[test]
fn step_4_header_bytes_is_checked_and_never_obeyed() {
    for declared in [0u32, 512, 8192, 0xffff_ffff] {
        let bytes = hand_built(|b| {
            b[OFF_HEADER_BYTES..OFF_HEADER_BYTES + 4].copy_from_slice(&declared.to_le_bytes());
        });
        assert_eq!(
            read(&bytes),
            Err(PackageError::HeaderBytesWrong { found: declared })
        );
    }
}

#[test]
fn step_5_an_algorithm_this_build_does_not_implement_is_refused() {
    // A signature scheme this build does not verify.
    let bytes = hand_built(|b| {
        b[OFF_SIG_ALG..OFF_SIG_ALG + 4].copy_from_slice(&7u32.to_le_bytes());
    });
    assert_eq!(
        read(&bytes),
        Err(PackageError::UnimplementedSignatureAlgorithm {
            found: SignatureAlgorithm(7),
            implemented: SignatureAlgorithm::ED25519,
        })
    );

    // And a build that implements a different digest refuses a payload it could
    // otherwise have verified — this one drives the *build's* side of step 5.
    let good = sample();
    let mut digest = ToyDigest::claiming(DigestAlgorithm(3));
    let verifier = ToyVerifier::new();
    assert_eq!(
        open(&good, good.len() as u64, &mut digest, &verifier, &trust()),
        Err(PackageError::UnimplementedDigestAlgorithm {
            found: DigestAlgorithm::BLAKE3,
            implemented: DigestAlgorithm(3),
        })
    );
}

#[test]
fn step_6_a_key_this_machine_does_not_trust_is_refused_before_the_signature_is_checked() {
    let bytes = sample();
    let empty = TrustSet::new();
    let mut digest = ToyDigest::new();
    let verifier = ToyVerifier::new();
    let outcome = open(&bytes, bytes.len() as u64, &mut digest, &verifier, &empty);
    assert!(matches!(outcome, Err(PackageError::UntrustedKey { .. })));

    // A trust set holding somebody *else's* key is the same refusal, not a
    // signature failure: step 6 is a table lookup and it happens first.
    let mut other = TrustSet::new();
    other.trust(
        &mut ToyDigest::new(),
        ToyKey(0x01).public(),
        "someone-else",
        MaySign::these(&[Kind::update()]),
    );
    let outcome = open(&bytes, bytes.len() as u64, &mut digest, &verifier, &other);
    assert!(matches!(outcome, Err(PackageError::UntrustedKey { .. })));
}

#[test]
fn a_forged_key_id_names_a_trusted_key_and_still_fails_the_signature() {
    // Take the key id of the trusted key and paste it into a payload signed by
    // a different one. Step 6 passes, step 7 does not.
    let mut digest = ToyDigest::new();
    let trusted_id = KeyId(digest.of(&SIGNING_KEY.public().0));

    let mut signer = ToySigner::new(ToyKey(0x02));
    let mut builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    builder
        .add(NewMember::stored("system.img", 0, 0, &[1u8; 16], &mut digest).unwrap())
        .unwrap();
    let mut bytes = builder
        .seal_in_memory(&mut digest, &mut signer, &[&[1u8; 16]])
        .unwrap();
    bytes[OFF_KEY_ID..OFF_KEY_ID + DIGEST_BYTES].copy_from_slice(&trusted_id.0);

    assert_eq!(
        read(&bytes),
        Err(PackageError::BadSignature { key_id: trusted_id })
    );
}

/// A signature over different bytes. Every one of these mutations is *inside*
/// the signed region, so the signature is genuine and covers something else.
#[test]
fn step_7_a_signature_over_different_bytes_is_refused() {
    let positions = [
        ("member count", OFF_MEMBER_COUNT),
        ("arch", OFF_ARCH),
        ("platform", OFF_PLATFORM),
        ("a member digest", record_at(0) + M_DIGEST),
        ("a member length", record_at(1) + M_STORED_LEN),
        ("the last signed byte", SIGNED_BYTES - 1),
    ];
    for (what, at) in positions {
        let mut bytes = sample();
        bytes[at] ^= 0x01;
        assert!(
            matches!(read(&bytes), Err(PackageError::BadSignature { .. })),
            "flipping a bit in {what} was not caught by the signature"
        );
    }
}

#[test]
fn a_flipped_bit_in_the_signature_itself_is_refused() {
    for bit in [0u8, 4, 7] {
        for byte in [0usize, 31, 63] {
            let mut bytes = sample();
            bytes[SIGNATURE_OFFSET + byte] ^= 1 << bit;
            assert!(matches!(
                read(&bytes),
                Err(PackageError::BadSignature { .. })
            ));
        }
    }
}

/// The property `ADR-029` §5 exists for, asserted rather than described:
/// **no member content is digested before the signature has been checked.**
#[test]
fn nothing_downstream_of_the_signature_check_runs_before_it() {
    let trace = Trace::new();
    let bytes = sample();
    let mut digest = ToyDigest::new().traced(&trace);
    let verifier = ToyVerifier::new().traced(&trace);
    let package = open(&bytes, bytes.len() as u64, &mut digest, &verifier, &trust()).unwrap();

    // Opening does exactly one thing that is not an integer comparison.
    assert_eq!(trace.events(), vec![Event::Verified(true)]);

    package
        .verify_member(0, &mut digest, &member_bytes()[0].3)
        .unwrap();
    let events = trace.events();
    let first = trace.first_verification().unwrap();
    assert!(
        events
            .iter()
            .enumerate()
            .all(|(i, e)| i <= first || !matches!(e, Event::Verified(_))),
        "the verifier ran twice"
    );
    assert!(
        events[..first]
            .iter()
            .all(|e| !matches!(e, Event::DigestWrite(_))),
        "member content was digested before the signature was checked: {events:?}"
    );
}

#[test]
fn a_refused_signature_leaves_the_members_untouched() {
    let trace = Trace::new();
    let mut bytes = sample();
    bytes[SIGNATURE_OFFSET] ^= 0xff;
    let mut digest = ToyDigest::new().traced(&trace);
    let verifier = ToyVerifier::new().traced(&trace);
    let outcome = open(&bytes, bytes.len() as u64, &mut digest, &verifier, &trust());
    assert!(matches!(outcome, Err(PackageError::BadSignature { .. })));
    // ADR-029 F2: "Refused at §5 step 7. Nothing written."
    assert_eq!(trace.events(), vec![Event::Verified(false)]);
}

// ---------------------------------------------------------------------------
// The manifest. Legitimately signed, structurally incoherent.
// ---------------------------------------------------------------------------

#[test]
fn more_members_than_the_format_allows_is_refused() {
    for count in [41u32, 100, 0xffff_ffff] {
        let bytes = hand_built(|b| {
            b[OFF_MEMBER_COUNT..OFF_MEMBER_COUNT + 4].copy_from_slice(&count.to_le_bytes());
        });
        assert_eq!(
            read(&bytes),
            Err(PackageError::TooManyMembers { found: count })
        );
    }
}

#[test]
fn a_flag_this_format_version_does_not_define_is_refused() {
    let bytes = hand_built(|b| {
        b[OFF_FLAGS..OFF_FLAGS + 4].copy_from_slice(&1u32.to_le_bytes());
    });
    assert_eq!(read(&bytes), Err(PackageError::UnknownFlags { flags: 1 }));
}

#[test]
fn a_reserved_byte_that_is_not_zero_is_refused_wherever_it_is() {
    let bytes = hand_built(|b| b[OFF_HEADER_RESERVED] = 1);
    assert_eq!(
        read(&bytes),
        Err(PackageError::ReservedBytesNotZero {
            field: "the end of the header"
        })
    );

    // The tail of the member table, above `member_count`. A signed header with
    // room for bytes nobody reads is a place to hide something.
    let bytes = hand_built(|b| b[record_at(5) + M_NAME] = b'x');
    assert_eq!(
        read(&bytes),
        Err(PackageError::ReservedBytesNotZero {
            field: "the unused member records"
        })
    );

    let bytes = hand_built(|b| b[record_at(0) + M_RESERVED_0] = 0xff);
    assert_eq!(
        read(&bytes),
        Err(PackageError::ReservedBytesNotZero {
            field: "a member record"
        })
    );
}

#[test]
fn a_member_claiming_a_length_the_file_does_not_have_is_refused() {
    let bytes = hand_built(|b| {
        let at = record_at(0) + M_STORED_LEN;
        b[at..at + 8].copy_from_slice(&(1u64 << 40).to_le_bytes());
        let at = record_at(0) + M_PLAIN_LEN;
        b[at..at + 8].copy_from_slice(&(1u64 << 40).to_le_bytes());
    });
    assert!(matches!(
        read(&bytes),
        Err(PackageError::MemberOverflowsFile { index: 0, .. })
    ));
}

/// `offset + stored_len` must not be allowed to wrap. The record says the member
/// starts where it should and runs to the end of the address space.
#[test]
fn a_length_that_would_wrap_the_address_space_is_refused_rather_than_wrapped() {
    let bytes = hand_built(|b| {
        let at = record_at(0) + M_STORED_LEN;
        b[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        let at = record_at(0) + M_PLAIN_LEN;
        b[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    });
    assert_eq!(read(&bytes), Err(PackageError::LengthOverflow));
}

#[test]
fn members_that_do_not_run_end_to_end_are_refused() {
    // A gap: bytes covered by no digest.
    let bytes = hand_built(|b| {
        let at = record_at(1) + M_OFFSET;
        let moved = u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) + 8;
        b[at..at + 8].copy_from_slice(&moved.to_le_bytes());
    });
    assert!(matches!(
        read(&bytes),
        Err(PackageError::MemberNotContiguous { index: 1, .. })
    ));

    // An overlap: two members claiming the same bytes.
    let bytes = hand_built(|b| {
        let at = record_at(1) + M_OFFSET;
        b[at..at + 8].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes());
    });
    assert!(matches!(
        read(&bytes),
        Err(PackageError::MemberNotContiguous { index: 1, .. })
    ));
}

/// Reordered member records — the classic way an archive reader is made to
/// disagree with its writer about which bytes are which member.
#[test]
fn reordered_member_records_are_refused() {
    let bytes = hand_built(|b| {
        let (a, c) = (record_at(1), record_at(2));
        let mut first = [0u8; MEMBER_RECORD_BYTES];
        first.copy_from_slice(&b[a..a + MEMBER_RECORD_BYTES]);
        let mut second = [0u8; MEMBER_RECORD_BYTES];
        second.copy_from_slice(&b[c..c + MEMBER_RECORD_BYTES]);
        b[a..a + MEMBER_RECORD_BYTES].copy_from_slice(&second);
        b[c..c + MEMBER_RECORD_BYTES].copy_from_slice(&first);
    });
    assert!(matches!(
        read(&bytes),
        Err(PackageError::MemberNotContiguous { index: 1, .. })
    ));
}

#[test]
fn trailing_bytes_no_digest_covers_are_refused() {
    let mut bytes = sample();
    let was = bytes.len() as u64;
    bytes.extend_from_slice(b"and one more thing");
    assert_eq!(
        read(&bytes),
        Err(PackageError::TrailingBytes {
            members_end: was,
            file_len: bytes.len() as u64
        })
    );
}

#[test]
fn a_duplicate_name_and_variant_is_refused_and_a_duplicate_name_alone_is_not() {
    // Two members with the same name AND the same variant: the pair is
    // indistinguishable, so an installer would have to pick one.
    let bytes = hand_built(|b| {
        let at = record_at(4) + M_VARIANT;
        b[at] = SlotVariance::SlotA.as_byte();
        let at = record_at(4) + M_NAME;
        b[at..at + 11].copy_from_slice(b"cmdline.txt");
    });
    assert!(matches!(
        read(&bytes),
        Err(PackageError::DuplicateMember { index: 4, .. })
    ));

    // But `cmdline.txt` twice with different variants is exactly what ADR-029
    // §3 requires, and the unmodified sample already carries it.
    let package = read(&sample()).unwrap();
    assert_eq!(package.members_named("cmdline.txt").count(), 2);
}

#[test]
fn a_member_name_that_traverses_is_refused() {
    for evil in [
        &b"../../etc/shadow"[..],
        &b"..\\windows"[..],
        &b"/absolute"[..],
        &b".."[..],
        &b"."[..],
    ] {
        let bytes = hand_built(|b| {
            let at = record_at(2) + M_NAME;
            b[at..at + 32].fill(0);
            b[at..at + evil.len()].copy_from_slice(evil);
        });
        let outcome = read(&bytes);
        assert!(
            matches!(
                outcome,
                Err(PackageError::MalformedMemberName { index: 2, .. })
            ),
            "{:?} was not refused: {outcome:?}",
            String::from_utf8_lossy(evil)
        );
    }
}

#[test]
fn a_nameless_member_is_refused() {
    let bytes = hand_built(|b| {
        let at = record_at(2) + M_NAME;
        b[at..at + 32].fill(0);
    });
    assert_eq!(
        read(&bytes),
        Err(PackageError::MalformedMemberName {
            index: 2,
            fault: TextFault::Empty
        })
    );
}

#[test]
fn a_compression_codec_this_build_does_not_implement_is_refused_rather_than_guessed_at() {
    for codec in [2u8, 255] {
        let bytes = hand_built(|b| b[record_at(1) + M_COMPRESSION] = codec);
        assert_eq!(
            read(&bytes),
            Err(PackageError::UnsupportedCompression {
                index: 1,
                found: codec
            })
        );
    }
}

/// **`R-177.1`: zstd is a codec this build recognises, and this fixture is an
/// `update` payload — so it is refused for the OTHER reason.** `sample()` is
/// signed and every other field of it is coherent; the only thing wrong is
/// that `update` is not `card`, and `ADR-029` §13/R-107 is why that refusal
/// exists at all: only a card image is ever decompressed off the workstation
/// alone, so only a card image may compress.
#[test]
fn zstd_is_recognised_but_refused_on_any_kind_other_than_card() {
    let bytes = hand_built(|b| b[record_at(1) + M_COMPRESSION] = COMPRESSION_ZSTD);
    assert_eq!(
        read(&bytes),
        Err(PackageError::CompressionNotPermittedForKind {
            index: 1,
            kind: "update".to_string(),
        })
    );
}

#[test]
fn an_uncompressed_member_whose_lengths_disagree_is_refused() {
    let bytes = hand_built(|b| {
        let at = record_at(1) + M_PLAIN_LEN;
        b[at..at + 8].copy_from_slice(&99u64.to_le_bytes());
    });
    assert!(matches!(
        read(&bytes),
        Err(PackageError::LengthsDisagree { index: 1, .. })
    ));
}

#[test]
fn a_header_text_field_with_bytes_behind_the_nul_is_refused() {
    let bytes = hand_built(|b| b[OFF_ARCH + 12] = b'!');
    assert_eq!(
        read(&bytes),
        Err(PackageError::MalformedText {
            field: "arch",
            fault: TextFault::TrailingBytesAfterNul
        })
    );
}

/// **An empty archive is structurally legal and semantically refused**, and the
/// split is where it belongs. A signed container with no members is a coherent
/// thing to have written; an *update* with no members is one that would report
/// success having changed nothing.
#[test]
fn an_empty_archive_reads_as_a_package_and_is_refused_as_an_update() {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    let bytes = builder
        .seal_in_memory(&mut digest, &mut signer, &[])
        .unwrap();

    assert_eq!(bytes.len(), HEADER_BYTES);
    let package = read(&bytes).unwrap();
    assert!(package.members().is_empty());
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::NothingToInstall
    );
}

// ---------------------------------------------------------------------------
// ADR-029 §5 steps 8-9: member content.
// ---------------------------------------------------------------------------

#[test]
fn a_member_whose_content_was_altered_is_refused_and_yields_no_token() {
    let package = read(&sample()).unwrap();
    let mut digest = ToyDigest::new();
    let mut content = member_bytes()[0].3.clone();
    content[2048] ^= 0x01;
    assert_eq!(
        package.verify_member(0, &mut digest, &content),
        Err(PackageError::MemberDigestMismatch {
            index: 0,
            name: "system.img".to_string()
        })
    );
}

#[test]
fn a_member_that_arrives_short_or_long_is_refused_before_the_digest_is_consulted() {
    let package = read(&sample()).unwrap();
    let mut digest = ToyDigest::new();
    let content = member_bytes()[0].3.clone();

    assert_eq!(
        package.verify_member(0, &mut digest, &content[..100]),
        Err(PackageError::MemberShort {
            index: 0,
            stored_len: 4096,
            seen: 100
        })
    );

    let mut long = content.clone();
    long.push(0);
    assert_eq!(
        package.verify_member(0, &mut digest, &long),
        Err(PackageError::MemberOverrun {
            index: 0,
            stored_len: 4096,
            offered: 4097
        })
    );
}

/// A source that lies about its length must not be able to make the reader hash
/// past the member and into whatever follows.
#[test]
fn a_stream_refuses_the_chunk_that_would_take_it_past_the_member() {
    let package = read(&sample()).unwrap();
    let mut digest = ToyDigest::new();
    let mut stream = package.member_stream(1, &mut digest).unwrap();
    stream.write(&[0u8; 700]).unwrap();
    assert!(matches!(
        stream.write(&[0u8; 1]),
        Err(PackageError::MemberOverrun { index: 1, .. })
    ));
}

#[test]
fn a_member_index_that_does_not_exist_is_refused() {
    let package = read(&sample()).unwrap();
    let mut digest = ToyDigest::new();
    assert_eq!(
        package.member_stream(9, &mut digest).err(),
        Some(PackageError::NoSuchMember {
            index: 9,
            member_count: 5
        })
    );
}

/// Two members' bytes, swapped. Each is individually well-formed and the pair is
/// jointly wrong — `AGENTS.md` §4's case, at the level of the format.
#[test]
fn swapping_two_members_content_is_caught_by_the_per_member_digests() {
    let package = read(&sample()).unwrap();
    let mut digest = ToyDigest::new();
    let members = member_bytes();
    assert!(matches!(
        package.verify_member(3, &mut digest, &members[4].3),
        Err(PackageError::MemberDigestMismatch { index: 3, .. })
    ));
    assert!(matches!(
        package.verify_member(4, &mut digest, &members[3].3),
        Err(PackageError::MemberDigestMismatch { index: 4, .. })
    ));
}

// ---------------------------------------------------------------------------
// ADR-029 §4 and the `update` kind's interpretation.
// ---------------------------------------------------------------------------

#[test]
fn the_two_reserved_names_refuse_the_whole_payload() {
    for reserved in ["autoboot.txt", "rivet.toml"] {
        let bytes = hand_built(|b| {
            let at = record_at(2) + M_NAME;
            b[at..at + 32].fill(0);
            b[at..at + reserved.len()].copy_from_slice(reserved.as_bytes());
        });
        // It reads as a package — the container has no opinion about names a
        // *platform* reserves.
        let package = read(&bytes).unwrap();
        // And it is refused as an update payload, whole.
        assert_eq!(
            Payload::open(&package, RESERVED).unwrap_err(),
            PackageError::ReservedMemberName {
                index: 2,
                name: reserved.to_string()
            }
        );
        // A platform that reserves nothing accepts it, which is what makes the
        // list the port's answer rather than this crate's.
        assert!(Payload::open(&package, &[]).is_ok());
    }
}

#[test]
fn the_writer_refuses_a_reserved_name_at_the_point_it_is_added() {
    let mut digest = ToyDigest::new();
    let mut builder = Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update())
        .unwrap()
        .reserving(RESERVED);
    let member = NewMember::stored("autoboot.txt", 1, 0, b"[all]\n", &mut digest).unwrap();
    assert_eq!(
        builder.add(member),
        Err(PackageError::ReservedMemberName {
            index: 0,
            name: "autoboot.txt".to_string()
        })
    );
}

/// The target's own comparison is case-insensitive (`ADR-029` §4's platform is
/// FAT), so a reserved name has to be refused under every ASCII case, not only
/// the one the reader happens to spell it.
#[test]
fn the_reader_refuses_a_reserved_name_under_any_ascii_case() {
    for reserved in ["AUTOBOOT.TXT", "Rivet.Toml"] {
        let bytes = hand_built(|b| {
            let at = record_at(2) + M_NAME;
            b[at..at + 32].fill(0);
            b[at..at + reserved.len()].copy_from_slice(reserved.as_bytes());
        });
        let package = read(&bytes).unwrap();
        assert_eq!(
            Payload::open(&package, RESERVED).unwrap_err(),
            PackageError::ReservedMemberName {
                index: 2,
                name: reserved.to_string()
            }
        );
    }
}

/// The same widening as the reader's, at the point a member is added, so a
/// build cannot emit what its own verifier would refuse.
#[test]
fn the_writer_refuses_a_reserved_name_under_any_ascii_case() {
    let mut digest = ToyDigest::new();
    let mut builder = Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update())
        .unwrap()
        .reserving(RESERVED);
    let member = NewMember::stored("AUTOBOOT.TXT", 1, 0, b"[all]\n", &mut digest).unwrap();
    assert_eq!(
        builder.add(member),
        Err(PackageError::ReservedMemberName {
            index: 0,
            name: "AUTOBOOT.TXT".to_string()
        })
    );
}

/// Two `Invariant` boot members whose names agree once ASCII case is folded —
/// distinct as far as this container's own duplicate check is concerned, but
/// one file on a target that compares names the way FAT does.
#[test]
fn two_invariant_boot_members_whose_names_agree_only_once_case_is_folded_are_refused() {
    let bytes = hand_built(|b| {
        // config.txt -> KERNEL8.IMG, still Invariant: now a case-only alias
        // of member 1's `kernel8.img`.
        let at = record_at(2) + M_NAME;
        b[at..at + 32].fill(0);
        b[at..at + "KERNEL8.IMG".len()].copy_from_slice(b"KERNEL8.IMG");
    });
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::BootMemberNameCollides {
            index: 1,
            name: "kernel8.img".to_string(),
            other_index: 2,
            other_name: "KERNEL8.IMG".to_string(),
        }
    );
}

/// `cmdline.txt` carried three ways at once — `Invariant`, `SlotA` and
/// `SlotB` — passes the completeness check (each slot-variant copy has its
/// counterpart) and the generic duplicate check (no two records share a
/// `(name, variant)` pair), yet every slot would receive two records for one
/// destination.
#[test]
fn an_invariant_member_sharing_a_name_with_the_slot_variant_pair_is_refused() {
    let bytes = hand_built(|b| {
        // config.txt -> cmdline.txt, still Invariant.
        let at = record_at(2) + M_NAME;
        b[at..at + 32].fill(0);
        b[at..at + "cmdline.txt".len()].copy_from_slice(b"cmdline.txt");
    });
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::BootMemberNameCollides {
            index: 2,
            name: "cmdline.txt".to_string(),
            other_index: 3,
            other_name: "cmdline.txt".to_string(),
        }
    );
}

/// A member named another member's install-time staging name: the second
/// member's write lands on the first member's already-installed file. Refused
/// regardless of which of the two ASCII case picks the suffix up under.
#[test]
fn a_member_named_another_members_temporary_name_is_refused() {
    for temp_name in ["kernel8.img.new", "KERNEL8.IMG.NEW"] {
        let bytes = hand_built(|b| {
            // config.txt -> kernel8.img's staging name, still Invariant.
            let at = record_at(2) + M_NAME;
            b[at..at + 32].fill(0);
            b[at..at + temp_name.len()].copy_from_slice(temp_name.as_bytes());
        });
        let package = read(&bytes).unwrap();
        assert_eq!(
            Payload::open(&package, RESERVED).unwrap_err(),
            PackageError::BootMemberNameCollidesWithTemporary {
                index: 2,
                name: temp_name.to_string(),
                other_index: 1,
                other_name: "kernel8.img".to_string(),
            },
            "{temp_name}"
        );
    }
}

/// The ordinary shapes: a same-spelling A/B `cmdline.txt` pair beside its
/// invariant siblings, unmodified from [`sample`]. Refusing the new cases must
/// not touch this one.
#[test]
fn the_ordinary_ab_pair_and_its_invariant_siblings_still_open() {
    let package = read(&sample()).unwrap();
    assert!(Payload::open(&package, RESERVED).is_ok());
}

#[test]
fn an_update_carries_exactly_one_whole_partition_member() {
    // Zero: the boot half alone.
    let bytes = hand_built(|b| b[record_at(0) + M_TARGET_CLASS] = 1);
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::SystemMemberCount { found: 0 }
    );

    // Two: which one does the slot become?
    let bytes = hand_built(|b| b[record_at(1) + M_TARGET_CLASS] = 0);
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::SystemMemberCount { found: 2 }
    );
}

#[test]
fn a_target_class_or_variant_this_kind_does_not_define_is_refused() {
    let bytes = hand_built(|b| b[record_at(1) + M_TARGET_CLASS] = 9);
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::UnknownTargetClass { index: 1, found: 9 }
    );

    let bytes = hand_built(|b| b[record_at(1) + M_VARIANT] = 3);
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::UnknownVariant { index: 1, found: 3 }
    );
}

/// A payload carrying `cmdline.txt` for one slot only installs cleanly into that
/// slot and silently omits a file in the other — `ADR-029` §6.2's *"this artifact
/// plus whatever happened to be there"*.
#[test]
fn a_slot_variant_member_present_for_one_slot_only_is_refused() {
    let bytes = hand_built(|b| {
        // Turn the slot-B copy into a second slot-A one under a different name,
        // leaving slot A's `cmdline.txt` with no counterpart.
        let at = record_at(4) + M_NAME;
        b[at..at + 32].fill(0);
        b[at..at + 8].copy_from_slice(b"spare.tx");
        b[record_at(4) + M_VARIANT] = SlotVariance::SlotA.as_byte();
    });
    let package = read(&bytes).unwrap();
    let outcome = Payload::open(&package, RESERVED).unwrap_err();
    assert!(
        matches!(outcome, PackageError::SlotVarianceIncomplete { missing, .. } if missing == "slot B"),
        "{outcome:?}"
    );
}

#[test]
fn the_system_member_may_not_claim_to_differ_between_slots() {
    let bytes = hand_built(|b| b[record_at(0) + M_VARIANT] = SlotVariance::SlotA.as_byte());
    let package = read(&bytes).unwrap();
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::SystemMemberIsSlotVariant {
            index: 0,
            variant: 1
        }
    );
}

#[test]
fn each_slot_gets_the_invariant_boot_files_and_exactly_its_own_cmdline() {
    let bytes = sample();
    let package = read(&bytes).unwrap();
    let payload = Payload::open(&package, RESERVED).unwrap();

    assert_eq!(payload.system_member().name(), "system.img");

    for (slot, wanted) in [(Slot::A, "system-a"), (Slot::B, "system-b")] {
        let chosen: Vec<&str> = payload.boot_members_for(slot).map(|m| m.name()).collect();
        assert_eq!(chosen, vec!["kernel8.img", "config.txt", "cmdline.txt"]);
        let cmdline = payload
            .boot_members_for(slot)
            .find(|m| m.name() == "cmdline.txt")
            .unwrap();
        let start = cmdline.offset() as usize;
        let end = start + cmdline.stored_len() as usize;
        let text = String::from_utf8(bytes[start..end].to_vec()).unwrap();
        assert!(text.contains(wanted), "slot {slot:?} got {text:?}");
        // And the system member is never in the boot half.
        assert!(
            payload
                .boot_members_for(slot)
                .all(|m| m.name() != "system.img"),
            "the whole-partition member leaked into the boot directory list"
        );
    }
}

#[test]
fn a_payload_for_another_board_is_refused_before_any_write() {
    let bytes = sample();
    let package = read(&bytes).unwrap();
    let payload = Payload::open(&package, RESERVED).unwrap();
    assert!(payload.accepts("rivet", "aarch64", "rpi4").is_ok());
    assert!(payload.accepts("rivet", "x86_64", "rpi4").is_err());
    assert!(payload.accepts("rivet", "aarch64", "cedar").is_err());
    assert!(payload.accepts("somethingelse", "aarch64", "rpi4").is_err());
}

#[test]
fn a_package_of_another_kind_is_refused_as_an_update() {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let mut builder = Builder::new(
        "rivet",
        "0.2.0",
        "aarch64",
        None,
        Kind::parse("experiment").unwrap(),
    )
    .unwrap();
    builder
        .add(NewMember::stored("thing", 0, 0, b"x", &mut digest).unwrap())
        .unwrap();
    let bytes = builder
        .seal_in_memory(&mut digest, &mut signer, &[b"x"])
        .unwrap();
    // Trusted for `experiment`, so the entitlement check passes and the
    // *interpretation* is what refuses — which is what this test is about.
    let package = read_trusting(&bytes, &[Kind::parse("experiment").unwrap()]).unwrap();
    // The container reads it; the `update` interpretation refuses it. That is
    // §11's whole shape: one verifier, several interpretations.
    assert_eq!(package.header().platform(), None);
    assert_eq!(
        Payload::open(&package, RESERVED).unwrap_err(),
        PackageError::WrongKind {
            expected: "update".to_string(),
            found: "experiment".to_string()
        }
    );
}

/// `ADR-027` §9 items 3–4 are open, so a kind that carries no platform must be
/// expressible. If this test ever has to change, the maintainer has ruled.
#[test]
fn a_package_may_carry_no_platform_at_all() {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let builder = Builder::new("rivet", "0.2.0", "aarch64", None, Kind::update()).unwrap();
    let bytes = builder
        .seal_in_memory(&mut digest, &mut signer, &[])
        .unwrap();
    assert_eq!(read(&bytes).unwrap().header().platform(), None);
}

// ---------------------------------------------------------------------------
// The writer's own refusals.
// ---------------------------------------------------------------------------

#[test]
fn the_writer_holds_itself_to_the_readers_rules() {
    let mut digest = ToyDigest::new();
    assert!(matches!(
        NewMember::stored("../etc/passwd", 1, 0, b"x", &mut digest),
        Err(PackageError::MalformedMemberName { .. })
    ));
    assert!(matches!(
        NewMember::stored("", 1, 0, b"x", &mut digest),
        Err(PackageError::MalformedMemberName { .. })
    ));
    assert!(matches!(
        NewMember::stored("a b", 1, 0, b"x", &mut digest),
        Err(PackageError::MalformedMemberName { .. })
    ));
    assert!(matches!(
        NewMember::stored(&"n".repeat(33), 1, 0, b"x", &mut digest),
        Err(PackageError::TextTooLong { .. })
    ));
    // `2` names no codec this build implements at all — unlike `1` (zstd,
    // `COMPRESSION_ZSTD`), which `NewMember::streamed` now accepts on its own
    // terms. Whether THIS payload's kind may carry it is `Builder::add`'s
    // question, not this constructor's, which has no `Kind` to ask.
    assert!(matches!(
        NewMember::streamed("x", 1, 0, 2, [0; DIGEST_BYTES], 1, 1),
        Err(PackageError::UnsupportedCompression { .. })
    ));
}

/// **`R-177.1`: zstd is legal at the constructor, and the kind gate is
/// `Builder::add`'s.** A member built with `COMPRESSION_ZSTD` is accepted by
/// `NewMember::streamed` and then refused the moment it is added to a builder
/// whose kind is not `card` — so the writer holds itself to exactly the rule
/// `read::parse_manifest` enforces on the way back in.
#[test]
fn a_compressed_member_is_refused_by_the_builder_for_any_kind_other_than_card() {
    let member =
        NewMember::streamed("image", 0, 0, COMPRESSION_ZSTD, [0; DIGEST_BYTES], 1, 2).unwrap();
    let mut builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    assert!(matches!(
        builder.add(member),
        Err(PackageError::CompressionNotPermittedForKind { index: 0, .. })
    ));
}

/// The positive case: a `card` builder accepts exactly the same member.
#[test]
fn a_compressed_member_is_accepted_by_a_card_builder() {
    let member =
        NewMember::streamed("image", 0, 0, COMPRESSION_ZSTD, [0; DIGEST_BYTES], 1, 2).unwrap();
    let mut builder =
        Builder::new("rivet", "0.1.87", "aarch64", Some("rpi4"), Kind::card()).unwrap();
    assert!(builder.add(member).is_ok());
}

#[test]
fn the_writer_refuses_a_forty_first_member() {
    let mut digest = ToyDigest::new();
    let mut builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    for index in 0..40 {
        let name = format!("m{index}");
        builder
            .add(NewMember::stored(&name, 1, 0, b"x", &mut digest).unwrap())
            .unwrap();
    }
    assert_eq!(
        builder.add(NewMember::stored("m40", 1, 0, b"x", &mut digest).unwrap()),
        Err(PackageError::TooManyMembers { found: 41 })
    );
}

#[test]
fn the_writer_refuses_a_header_field_that_would_not_fit() {
    assert!(matches!(
        // 33 characters. It was 24 until `product` grew from 16 to 32 on
        // 2026-08-24, at which point the old name started fitting and this
        // assertion stopped testing anything.
        Builder::new(
            "a-product-name-that-is-far-too-long",
            "0.1",
            "aarch64",
            None,
            Kind::update(),
        ),
        Err(PackageError::TextTooLong {
            field: "product",
            ..
        })
    ));
    assert!(matches!(
        Builder::new("rivet", "", "aarch64", None, Kind::update()),
        Err(PackageError::MissingText { field: "version" })
    ));
    assert!(matches!(
        Builder::new("rivet", "0.1", "aarch64 v8", None, Kind::update()),
        Err(PackageError::MalformedText { field: "arch", .. })
    ));
}

#[test]
fn an_in_memory_write_refuses_bytes_that_are_not_the_ones_it_was_told_about() {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let mut builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    builder
        .add(NewMember::stored("system.img", 0, 0, b"the real bytes", &mut digest).unwrap())
        .unwrap();
    assert_eq!(
        builder.seal_in_memory(&mut digest, &mut signer, &[b"the OTHER bytes"]),
        Err(PackageError::MemberBytesDisagree { index: 0 })
    );
    assert_eq!(
        builder.seal_in_memory(&mut digest, &mut signer, &[]),
        Err(PackageError::MemberBytesDisagree { index: 0 })
    );
}

#[test]
fn the_writer_places_members_end_to_end_from_the_header_on() {
    let mut digest = ToyDigest::new();
    let mut builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    let first = builder
        .add(NewMember::stored("a", 1, 0, &[0u8; 10], &mut digest).unwrap())
        .unwrap();
    let second = builder
        .add(NewMember::stored("b", 1, 0, &[0u8; 20], &mut digest).unwrap())
        .unwrap();
    assert_eq!(first, HEADER_BYTES as u64);
    assert_eq!(second, HEADER_BYTES as u64 + 10);
    assert_eq!(builder.total_len(), HEADER_BYTES as u64 + 30);
}

/// The two enums `update` defines must round-trip through the byte they are
/// stored as, or the writer and the reader mean different things by 1.
#[test]
fn the_update_kinds_metadata_round_trips_through_its_bytes() {
    for class in [TargetClass::WholePartition, TargetClass::BootDirectory] {
        assert_eq!(TargetClass::from_byte(class.as_byte()), Some(class));
    }
    for variance in [
        SlotVariance::Invariant,
        SlotVariance::SlotA,
        SlotVariance::SlotB,
    ] {
        assert_eq!(SlotVariance::from_byte(variance.as_byte()), Some(variance));
    }
    assert_eq!(TargetClass::from_byte(2), None);
    assert_eq!(SlotVariance::from_byte(3), None);
    assert_eq!(SlotVariance::of(Slot::A), SlotVariance::SlotA);
    assert_eq!(SlotVariance::of(Slot::B), SlotVariance::SlotB);
    assert!(SlotVariance::Invariant.applies_to(Slot::A));
    assert!(SlotVariance::Invariant.applies_to(Slot::B));
    assert!(!SlotVariance::SlotA.applies_to(Slot::B));
}

/// `ADR-029` §3's own byte counts, which the ADR states as facts about the
/// format rather than about this implementation.
#[test]
fn the_header_is_four_kibibytes_and_a_member_record_is_ninety_six() {
    let bytes = sample();
    let package = read(&bytes).unwrap();
    assert_eq!(package.members()[0].offset(), 4096);
    assert_eq!(MEMBER_RECORD_BYTES, 96);
    // 184 + 40 * 96 = 4024, and the signature starts at 4032 — eight reserved
    // bytes, down from twenty-four before `product` grew on 2026-08-24.
    assert_eq!(MEMBER_TABLE_OFFSET + 40 * MEMBER_RECORD_BYTES, 4024);
    assert_eq!(SIGNATURE_OFFSET, 4032);
}

/// **A valid signature from a trusted key, refused anyway.**
///
/// `ADR-028` §5.5, ruled 2026-08-24. This is the case the maintainer separated
/// his two keys to create: the key that signs applications must not be able to
/// ship a base image. Every earlier step passes — the magic, the version, the
/// key id, and the Ed25519 signature over the signed region — and step 8
/// refuses on the one question the signature cannot answer.
///
/// **The baseline is asserted in the same test**, so a refusal caused by
/// something else could not pass for this one: the identical payload declaring
/// `update` is accepted by the identical trust set.
#[test]
fn a_key_trusted_for_updates_may_not_sign_an_application() {
    // The baseline: this trust set accepts this key for `update`.
    assert!(
        read(&sample()).is_ok(),
        "the baseline must pass, or the refusal below proves nothing"
    );

    let application = sample_of_kind(Kind::application());
    let refusal = read(&application).expect_err("an application signed by the update key");
    let PackageError::KindNotPermitted {
        label,
        kind,
        may_sign,
        ..
    } = &refusal
    else {
        panic!("expected KindNotPermitted, got {refusal:?}");
    };
    assert_eq!(kind, "application");
    assert_eq!(label, "test-update");
    assert_eq!(may_sign, &vec!["update".to_string()]);

    // The refusal has to say which key, in the operator's own name for it, and
    // what that key may sign — ADR-006's explainability at the sharpest point.
    let said = refusal.to_string();
    assert!(said.contains("test-update"), "{said}");
    assert!(said.contains("application"), "{said}");
    assert!(said.contains("update"), "{said}");
}

/// A key trusted for **nothing** signs nothing, and the message says so in
/// words rather than as an empty list.
#[test]
fn a_key_trusted_for_nothing_is_refused_for_everything() {
    let mut digest = ToyDigest::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        SIGNING_KEY.public(),
        "revoked",
        MaySign::nothing(),
    );
    let verifier = ToyVerifier::new();
    let bytes = sample();
    let refusal = open(
        &bytes[..HEADER_BYTES],
        bytes.len() as u64,
        &mut digest,
        &verifier,
        &trust,
    )
    .expect_err("a key entitled to nothing");
    assert!(
        matches!(refusal, PackageError::KindNotPermitted { .. }),
        "{refusal:?}"
    );
    assert!(refusal.to_string().contains("nothing at all"), "{refusal}");
}
