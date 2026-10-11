//! BLAKE3 and Ed25519, behind `rivet_core::package`'s ports. `ADR-029` §7.
//!
//! # What is here and what deliberately is not
//!
//! Three implementors and nothing else:
//!
//! | Type | Port | `ADR-029` |
//! | :-- | :-- | :-- |
//! | [`Blake3`] | `package::Digest` | §7.2 — BLAKE3, **pure Rust specifically** |
//! | [`Ed25519Verifier`] | `package::Verify` | §7.1 — Ed25519, raw |
//! | [`Ed25519Signer`] | `package::Sign` | §7.1, host side only |
//!
//! **No I/O, no key storage, no trust policy.** Where a machine's root key set
//! comes from is `ADR-028` §5.2's — it ships in `/system`, which is read-only by
//! format — and nothing in this crate reads a file. `ADR-015` §4.3 makes the
//! maintainer the gatekeeper for signing keys and there is no key management
//! anywhere in this project yet; [`Ed25519Signer::from_seed`] takes 32 bytes and
//! asks no questions about where they came from.
//!
//! # Verification is strict, and that is a decision rather than a default
//!
//! [`Ed25519Verifier`] calls dalek's `verify_strict`, not `verify`. Ed25519 has
//! a long-documented interop hazard: RFC 8032 under-specifies whether a
//! small-order public key, a non-canonical point encoding or a non-canonical
//! scalar should be accepted, and implementations disagree. The consequence is
//! not academic — it is that two implementations can disagree about whether a
//! given payload is validly signed.
//!
//! For Rivet the strict direction is the safe one in both roles it plays. On
//! the board, refusing an oddly-encoded signature costs a re-sign; accepting one
//! costs whatever the ambiguity is worth to an attacker. On the build host,
//! `cargo rivet package` (`ADR-015` §3.2) is the only signer there will ever
//! be (`ADR-028` §5.3 — *"Rivet has **one signer** for the foreseeable
//! future"*), so nothing legitimate produces the encodings strictness rejects.
//!
//! # What has been run, and what has not — `AGENTS.md` §6
//!
//! The suite below runs on the development host and under WSL. It includes
//! **RFC 8032 §7.1's own Ed25519 test vector**, which is worth more than
//! anything hand-composed because it was produced by someone else
//! (`AGENTS.md` §4 — label fixtures by provenance).
//!
//! **Nothing in this crate has run on a Raspberry Pi.** `ADR-029` §7.2's
//! measurement — hash 640 MiB on the board with BLAKE3 and SHA-256 — is still
//! not taken, and `cmd/rivet`'s `measure hash` is the instrument that would
//! take it. Until it is, "BLAKE3 is fast enough on an A72" is a reasoned
//! expectation and not a reading.

#![forbid(unsafe_code)]

// `verify_strict` is an inherent method on `VerifyingKey`, not the `Verifier`
// trait's `verify` — importing the trait would make it easy to call the lenient
// one by accident, which is the whole thing this crate is choosing against.
use ed25519_dalek::{Signature as DalekSignature, Signer as _, SigningKey, VerifyingKey};
use rivet_core::package::format::{DIGEST_BYTES, PUBLIC_KEY_BYTES, SIGNATURE_BYTES};
use rivet_core::package::port::{
    Digest, DigestAlgorithm, PublicKey, Sign, Signature, SignatureAlgorithm, Verify,
};

/// The number of bytes an Ed25519 private seed is. Not a key: the key is derived
/// from it.
pub const SEED_BYTES: usize = 32;

/// `ADR-029` §7.2's digest.
///
/// Streaming, because the largest member is a whole `/system` filesystem and the
/// board it is checked on has 4 GB and no swap.
pub mod pmk;

pub struct Blake3 {
    hasher: blake3::Hasher,
}

impl Default for Blake3 {
    fn default() -> Self {
        Self::new()
    }
}

impl Blake3 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
        }
    }
}

impl Digest for Blake3 {
    fn algorithm(&self) -> DigestAlgorithm {
        DigestAlgorithm::BLAKE3
    }

    fn start(&mut self) {
        self.hasher.reset();
    }

    fn write(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    fn finish(&mut self) -> [u8; DIGEST_BYTES] {
        let out = *self.hasher.finalize().as_bytes();
        self.hasher.reset();
        out
    }
}

/// `ADR-029` §7.1's verification. **This is the security boundary.**
#[derive(Debug, Clone, Copy, Default)]
pub struct Ed25519Verifier;

impl Ed25519Verifier {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Verify for Ed25519Verifier {
    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ED25519
    }

    fn verify(&self, key: &PublicKey, message: &[u8], signature: &Signature) -> bool {
        // A key that is not a valid point is not an error state to report; it is
        // a key that verifies nothing. `Verify::verify` returns `bool` precisely
        // so there is no third answer for a caller to mishandle.
        let Ok(verifying) = VerifyingKey::from_bytes(&key.0) else {
            return false;
        };
        let candidate = DalekSignature::from_bytes(&signature.0);
        // `verify_strict`, not `verify` — see the module documentation.
        verifying.verify_strict(message, &candidate).is_ok()
    }
}

/// `ADR-029` §7.1's signing. **A host-tool type**; no Rivet machine has one.
///
/// It is here rather than in a build tool for the reason `rivet_core::package`
/// puts the writer beside the reader: the two must agree about what is signed,
/// and one place is what stops them diverging.
pub struct Ed25519Signer {
    key: SigningKey,
}

impl Ed25519Signer {
    /// Build a signer from 32 raw bytes.
    ///
    /// **No generation, and no file.** `AGENTS.md` §8.3 forbids a key in a
    /// tracked file, and `ADR-015` §4.3 makes key custody the maintainer's; a
    /// constructor that could mint or load one would be this crate having an
    /// opinion about both. The test suite mints its own into a temporary
    /// directory, which is exactly what §8.3 asks for.
    #[must_use]
    pub fn from_seed(seed: &[u8; SEED_BYTES]) -> Self {
        Self {
            key: SigningKey::from_bytes(seed),
        }
    }
}

impl Sign for Ed25519Signer {
    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ED25519
    }

    fn public_key(&self) -> PublicKey {
        PublicKey(self.key.verifying_key().to_bytes())
    }

    fn sign(&mut self, message: &[u8]) -> Signature {
        let mut out = [0u8; SIGNATURE_BYTES];
        out.copy_from_slice(&self.key.sign(message).to_bytes());
        Signature(out)
    }
}

/// The public key `seed` produces, without building a signer.
#[must_use]
pub fn public_key_for(seed: &[u8; SEED_BYTES]) -> PublicKey {
    PublicKey(SigningKey::from_bytes(seed).verifying_key().to_bytes())
}

// The port's constants and dalek's must be the same numbers, or this crate would
// be copying a signature into a buffer of the wrong size. Checked at compile
// time rather than believed.
const _: () = assert!(SIGNATURE_BYTES == ed25519_dalek::SIGNATURE_LENGTH);
const _: () = assert!(PUBLIC_KEY_BYTES == ed25519_dalek::PUBLIC_KEY_LENGTH);
const _: () = assert!(SEED_BYTES == ed25519_dalek::SECRET_KEY_LENGTH);
