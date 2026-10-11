//! Fakes for the container's ports, and a trace so a test can assert *order*.
//!
//! # What these prove, and what they cannot
//!
//! `AGENTS.md` §6, stated up front because this module is exactly where it is
//! easy to overclaim. These fakes prove the **ordering and the refusals** of
//! `ADR-029` §5 — that nothing downstream of the signature check runs before it,
//! that each of steps 1 to 6 refuses the input it is supposed to, and that a
//! member's bytes are never handed back before its digest matched.
//!
//! **They prove nothing whatever about Ed25519 or BLAKE3.** [`ToyDigest`] is a
//! mixing function with no security claim and [`ToyVerifier`] accepts anything
//! the matching [`ToySigner`] produced. `rivet-crypto`'s suite is where the
//! real algorithms are driven, and neither suite substitutes for the other.
//!
//! # `AGENTS.md` §4, on fixtures
//!
//! *"Label fixtures by provenance."* Everything this module makes is
//! **hand-composed**, which means it proves the code agrees with its own author.
//! The vectors that are worth more than that are the ones in
//! `crate::package::tests` that were **not** produced by the writer — bit flips,
//! truncations, hand-built headers — and they say so at each test.

use std::cell::RefCell;
use std::rc::Rc;

use crate::package::format::{DIGEST_BYTES, PUBLIC_KEY_BYTES, SIGNATURE_BYTES};
use crate::package::port::{
    Digest, DigestAlgorithm, PublicKey, Sign, Signature, SignatureAlgorithm, Verify,
};

/// What a port was asked to do, in the order it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    DigestStart,
    DigestWrite(usize),
    DigestFinish,
    /// The verifier ran, and what it said.
    Verified(bool),
}

/// A log shared between the fakes, so a test can assert that a digest of member
/// content never happened before a [`Event::Verified`].
#[derive(Debug, Clone, Default)]
pub struct Trace(Rc<RefCell<Vec<Event>>>);

impl Trace {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }

    #[must_use]
    pub fn events(&self) -> Vec<Event> {
        self.0.borrow().clone()
    }

    /// Index of the first [`Event::Verified`], if any.
    #[must_use]
    pub fn first_verification(&self) -> Option<usize> {
        self.0
            .borrow()
            .iter()
            .position(|e| matches!(e, Event::Verified(_)))
    }
}

/// A 256-bit mixing function with **no security claim at all**.
///
/// It exists so the container's plumbing can be driven without linking
/// cryptography into `rivet-core`, which `ADR-014` §3 forbids. It is a
/// function of every input byte and of their order, which is the only property
/// the tests need of it: a flipped bit changes the output, so a test that
/// asserts "a corrupted member is refused" is not passing vacuously.
#[derive(Debug, Clone)]
pub struct ToyDigest {
    algorithm: DigestAlgorithm,
    state: [u64; 4],
    at: usize,
    trace: Option<Trace>,
}

const MIX: u64 = 0x100_0000_01b3;

impl Default for ToyDigest {
    fn default() -> Self {
        Self::new()
    }
}

impl ToyDigest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            algorithm: DigestAlgorithm::BLAKE3,
            state: Self::SEED,
            at: 0,
            trace: None,
        }
    }

    /// Claim a different algorithm, to drive `ADR-029` §5 step 5.
    #[must_use]
    pub fn claiming(algorithm: DigestAlgorithm) -> Self {
        Self {
            algorithm,
            ..Self::new()
        }
    }

    #[must_use]
    pub fn traced(mut self, trace: &Trace) -> Self {
        self.trace = Some(trace.clone());
        self
    }

    const SEED: [u64; 4] = [
        0xcbf2_9ce4_8422_2325,
        0x9e37_79b9_7f4a_7c15,
        0xff51_afd7_ed55_8ccd,
        0xc4ce_b9fe_1a85_ec53,
    ];
}

impl Digest for ToyDigest {
    fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    fn start(&mut self) {
        if let Some(trace) = &self.trace {
            trace.push(Event::DigestStart);
        }
        self.state = Self::SEED;
        self.at = 0;
    }

    fn write(&mut self, bytes: &[u8]) {
        if let Some(trace) = &self.trace {
            trace.push(Event::DigestWrite(bytes.len()));
        }
        for byte in bytes {
            let lane = self.at % 4;
            let next = (self.at + 1) % 4;
            self.state[lane] = (self.state[lane] ^ u64::from(*byte)).wrapping_mul(MIX);
            self.state[next] = self.state[next].rotate_left(7) ^ self.state[lane];
            self.at = self.at.wrapping_add(1);
        }
    }

    fn finish(&mut self) -> [u8; DIGEST_BYTES] {
        if let Some(trace) = &self.trace {
            trace.push(Event::DigestFinish);
        }
        // Fold the length in, so that appending zero bytes is not the identity.
        let len = self.at as u64;
        for lane in 0..4 {
            self.state[lane] =
                (self.state[lane] ^ len.rotate_left(lane as u32 * 11)).wrapping_mul(MIX);
        }
        let mut out = [0u8; DIGEST_BYTES];
        for (lane, chunk) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            chunk.copy_from_slice(&self.state[lane].to_le_bytes());
        }
        out
    }
}

/// The `Digest` [`crate::package::port::TrustSet`]'s own tests use, and the name
/// the rest of the suite reaches for.
pub type CountingDigest = ToyDigest;

/// A "signature" that is a keyed digest of the message.
///
/// **Forgeable by anyone holding the public key**, which is the whole point: it
/// is not a signature scheme, it is an oracle that says truthfully whether these
/// are the bytes that were signed. Everything `ADR-029` §5 needs to be tested
/// about *ordering* follows from that; nothing about Ed25519 does.
#[derive(Debug, Clone, Copy)]
pub struct ToyKey(pub u8);

impl ToyKey {
    #[must_use]
    pub fn public(self) -> PublicKey {
        PublicKey([self.0; PUBLIC_KEY_BYTES])
    }

    fn sign_bytes(self, message: &[u8]) -> Signature {
        let mut out = [0u8; SIGNATURE_BYTES];
        for (half, salt) in out.as_chunks_mut::<DIGEST_BYTES>().0.iter_mut().zip([0x5au8, 0xa5]) {
            let mut digest = ToyDigest::new();
            digest.start();
            digest.write(&[self.0, salt]);
            digest.write(message);
            half.copy_from_slice(&digest.finish());
        }
        Signature(out)
    }
}

/// The signing half. Host-side only; no Rivet machine has one.
#[derive(Debug, Clone)]
pub struct ToySigner {
    key: ToyKey,
    algorithm: SignatureAlgorithm,
}

impl ToySigner {
    #[must_use]
    pub fn new(key: ToyKey) -> Self {
        Self {
            key,
            algorithm: SignatureAlgorithm::ED25519,
        }
    }

    /// Claim a different scheme, to drive `ADR-029` §5 step 5.
    #[must_use]
    pub fn claiming(key: ToyKey, algorithm: SignatureAlgorithm) -> Self {
        Self { key, algorithm }
    }
}

impl Sign for ToySigner {
    fn algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }
    fn public_key(&self) -> PublicKey {
        self.key.public()
    }
    fn sign(&mut self, message: &[u8]) -> Signature {
        self.key.sign_bytes(message)
    }
}

/// The verifying half.
#[derive(Debug, Clone)]
pub struct ToyVerifier {
    algorithm: SignatureAlgorithm,
    trace: Option<Trace>,
}

impl Default for ToyVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ToyVerifier {
    #[must_use]
    pub fn new() -> Self {
        Self {
            algorithm: SignatureAlgorithm::ED25519,
            trace: None,
        }
    }

    #[must_use]
    pub fn claiming(algorithm: SignatureAlgorithm) -> Self {
        Self {
            algorithm,
            trace: None,
        }
    }

    #[must_use]
    pub fn traced(mut self, trace: &Trace) -> Self {
        self.trace = Some(trace.clone());
        self
    }
}

impl Verify for ToyVerifier {
    fn algorithm(&self) -> SignatureAlgorithm {
        self.algorithm
    }

    fn verify(&self, key: &PublicKey, message: &[u8], signature: &Signature) -> bool {
        let expected = ToyKey(key.0[0]).sign_bytes(message);
        let outcome = expected.0 == signature.0;
        if let Some(trace) = &self.trace {
            trace.push(Event::Verified(outcome));
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A fake whose output did not depend on every input byte would make every
    /// corruption test in this crate pass for the wrong reason. Checked rather
    /// than assumed.
    #[test]
    fn the_toy_digest_notices_a_single_flipped_bit_anywhere() {
        let base = vec![0x42u8; 512];
        let mut digest = ToyDigest::new();
        let reference = digest.of(&base);
        for position in [0usize, 1, 255, 510, 511] {
            for bit in [0u8, 3, 7] {
                let mut altered = base.clone();
                altered[position] ^= 1 << bit;
                assert_ne!(
                    digest.of(&altered),
                    reference,
                    "flipping bit {bit} of byte {position} did not change the digest"
                );
            }
        }
    }

    #[test]
    fn the_toy_digest_is_order_sensitive_and_length_sensitive() {
        let mut digest = ToyDigest::new();
        assert_ne!(digest.of(b"ab"), digest.of(b"ba"));
        assert_ne!(digest.of(b"a"), digest.of(b"a\0"));
        assert_ne!(digest.of(b""), digest.of(b"\0"));
    }

    #[test]
    fn a_restarted_digest_forgets_everything_before_it() {
        let mut digest = ToyDigest::new();
        let once = digest.of(b"hello");
        digest.start();
        digest.write(b"rubbish");
        digest.start();
        digest.write(b"hello");
        assert_eq!(digest.finish(), once);
    }

    #[test]
    fn the_toy_signature_covers_the_message_and_the_key() {
        let mut signer = ToySigner::new(ToyKey(1));
        let verifier = ToyVerifier::new();
        let signature = signer.sign(b"the bytes");
        assert!(verifier.verify(&ToyKey(1).public(), b"the bytes", &signature));
        assert!(!verifier.verify(&ToyKey(1).public(), b"other bytes", &signature));
        assert!(!verifier.verify(&ToyKey(2).public(), b"the bytes", &signature));
    }

    #[test]
    fn the_trace_records_what_happened_in_order() {
        let trace = Trace::new();
        let mut digest = ToyDigest::new().traced(&trace);
        digest.start();
        digest.write(&[1, 2, 3]);
        let _ = digest.finish();
        assert_eq!(
            trace.events(),
            vec![
                Event::DigestStart,
                Event::DigestWrite(3),
                Event::DigestFinish
            ]
        );
        assert_eq!(trace.first_verification(), None);
    }
}
