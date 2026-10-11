//! The two algorithms, against vectors **somebody else produced**.
//!
//! `AGENTS.md` §4: *"Label fixtures by provenance … every hand-composed vector
//! proves only that the code agrees with its own author."* Everything in this
//! file is externally sourced and says where from. That is the whole point of
//! the file: `rivet-core`'s suite drives the container against fakes, and a
//! fake cannot tell anyone whether this crate computes Ed25519.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use rivet_core::package::port::{Digest, PublicKey, Sign, Signature, Verify};
use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier, SEED_BYTES, public_key_for};

fn unhex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "odd-length hex");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn seed(text: &str) -> [u8; SEED_BYTES] {
    let bytes = unhex(text);
    let mut out = [0u8; SEED_BYTES];
    out.copy_from_slice(&bytes);
    out
}

/// **RFC 8032 §7.1, TEST 1.** Not composed here — this is the Ed25519
/// specification's own vector, and it is what makes "this crate computes
/// Ed25519" a claim with something behind it.
#[test]
fn rfc_8032_test_1() {
    let secret = seed("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
    let expected_public = unhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let expected_signature = unhex(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e397\
         01cf9b46bd25bf5f0595bbe24655141438e7a100b",
    );

    let mut signer = Ed25519Signer::from_seed(&secret);
    assert_eq!(signer.public_key().0.to_vec(), expected_public);
    assert_eq!(public_key_for(&secret).0.to_vec(), expected_public);

    let produced = signer.sign(b"");
    assert_eq!(produced.0.to_vec(), expected_signature);

    let mut key = [0u8; 32];
    key.copy_from_slice(&expected_public);
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&expected_signature);
    assert!(Ed25519Verifier::new().verify(&PublicKey(key), b"", &Signature(signature)));
}

/// **RFC 8032 §7.1, TEST 2.** A one-byte message, so the empty-message case
/// above is not the only shape this is checked on.
#[test]
fn rfc_8032_test_2() {
    let secret = seed("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb");
    let expected_public = unhex("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
    let expected_signature = unhex(
        "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f361\
         3d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
    );

    let mut signer = Ed25519Signer::from_seed(&secret);
    assert_eq!(signer.public_key().0.to_vec(), expected_public);
    assert_eq!(signer.sign(&[0x72]).0.to_vec(), expected_signature);
}

/// **BLAKE3's published vector for the empty input**, from the reference
/// implementation's own test vectors.
#[test]
fn blake3_of_nothing_is_blake3s_published_value() {
    let mut digest = Blake3::new();
    assert_eq!(
        digest.of(b""),
        seed("af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
    );
}

/// A digest that did not reset between uses would make every second member in a
/// payload fail, and the failure would look like a corrupt payload.
#[test]
fn a_digest_is_the_same_whether_it_is_fresh_or_reused() {
    let mut fresh = Blake3::new();
    let once = fresh.of(b"the same bytes");

    let mut reused = Blake3::new();
    let _ = reused.of(b"something else entirely");
    assert_eq!(reused.of(b"the same bytes"), once);

    // And streamed in pieces, which is how a 640 MiB member arrives.
    let mut streamed = Blake3::new();
    streamed.start();
    streamed.write(b"the same");
    streamed.write(b" bytes");
    assert_eq!(streamed.finish(), once);
}

/// `AGENTS.md` §4: *"When you add a guard, break it and watch the test fail."*
/// This is the guard for every member-content check in the container, driven at
/// the level of the algorithm rather than the format.
#[test]
fn a_single_flipped_bit_anywhere_in_a_megabyte_changes_the_digest() {
    let mut digest = Blake3::new();
    let base = vec![0x5au8; 1024 * 1024];
    let reference = digest.of(&base);
    for position in [0usize, 1, 65_535, 524_288, base.len() - 1] {
        let mut altered = base.clone();
        altered[position] ^= 0x01;
        assert_ne!(digest.of(&altered), reference, "byte {position}");
    }
}

/// The signature covers the message and nothing else about it may be changed.
#[test]
fn a_signature_does_not_survive_a_change_to_the_message_or_to_the_key() {
    let secret = seed("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
    let mut signer = Ed25519Signer::from_seed(&secret);
    let verifier = Ed25519Verifier::new();
    let message = b"the four thousand and thirty-two signed bytes";
    let signature = signer.sign(message);
    let key = signer.public_key();

    assert!(verifier.verify(&key, message, &signature));

    let mut altered = message.to_vec();
    altered[0] ^= 0x01;
    assert!(!verifier.verify(&key, &altered, &signature));

    // Truncated and extended messages, which is the failure a length field would
    // otherwise have to catch.
    assert!(!verifier.verify(&key, &message[..message.len() - 1], &signature));
    let mut longer = message.to_vec();
    longer.push(0);
    assert!(!verifier.verify(&key, &longer, &signature));

    // Every byte of the signature matters.
    for position in [0usize, 31, 32, 63] {
        let mut bent = signature;
        bent.0[position] ^= 0x01;
        assert!(
            !verifier.verify(&key, message, &bent),
            "a flipped bit at {position} of the signature was accepted"
        );
    }

    // A different key.
    let other = public_key_for(&seed(
        "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
    ));
    assert!(!verifier.verify(&other, message, &signature));
}

/// A public key that is not a point on the curve is a key that verifies nothing,
/// and it must not be an error a caller could mistake for a failure to check.
#[test]
fn a_public_key_that_is_not_a_key_refuses_rather_than_anything_else() {
    let verifier = Ed25519Verifier::new();
    for junk in [[0xffu8; 32], [0x00; 32]] {
        assert!(!verifier.verify(&PublicKey(junk), b"anything", &Signature([0u8; 64])));
    }
}

/// **`verify_strict` rather than `verify`, proved by a vector that tells the two
/// apart.**
///
/// The first version of this test asserted that a small-order public key with an
/// all-zero signature is refused. It passed — and it **also passed with
/// `verify_strict` swapped for the lenient `verify`**, which was found by running
/// exactly that injection (`AGENTS.md` §4: *"a regression test that cannot fail
/// proves nothing"*). An all-zero signature fails both ways, so the test said
/// nothing about strictness at all.
///
/// This vector does tell them apart, and it is the classic one:
///
/// * the public key `A` is the **identity point**, encoded `01 00 … 00`;
/// * `R` is the identity as well;
/// * `s` is zero.
///
/// The cofactorless verification equation is then `[s]B == R + [k]A`, which
/// reduces to `identity == identity + [k]·identity` — true for **every message**.
/// So a lenient verifier accepts this signature for any bytes at all, and the
/// only defence is refusing the key for being small-order. That is precisely
/// what `verify_strict` adds, and it is why `rivet-crypto` calls it.
///
/// Anyone can construct this pair; it needs no private key.
#[test]
fn a_small_order_public_key_with_a_signature_a_lenient_verifier_accepts_is_refused() {
    let verifier = Ed25519Verifier::new();
    let identity = unhex("0100000000000000000000000000000000000000000000000000000000000000");

    let mut key = [0u8; 32];
    key.copy_from_slice(&identity);
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(&identity); // R = identity
    // s = 0, already zero, and canonical.

    for message in [&b""[..], b"a message", b"a completely different message"] {
        assert!(
            !verifier.verify(&PublicKey(key), message, &Signature(signature)),
            "the identity public key verified {:?}",
            String::from_utf8_lossy(message)
        );
    }
}

/// The other small-order encodings, with a signature that is nonsense. This is
/// the weaker assertion the test above used to make on its own, kept because it
/// covers the decode path — but it is **not** the strictness test, and saying so
/// here is the point of splitting them.
#[test]
fn the_small_order_encodings_verify_nothing() {
    let verifier = Ed25519Verifier::new();
    for hex in [
        "0100000000000000000000000000000000000000000000000000000000000000",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "0000000000000000000000000000000000000000000000000000000000000080",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let mut key = [0u8; 32];
        key.copy_from_slice(&unhex(hex));
        assert!(
            !verifier.verify(&PublicKey(key), b"a message", &Signature([0u8; 64])),
            "a small-order key ({hex}) verified something"
        );
    }
}
