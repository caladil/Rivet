//! The cryptography seam — **and the first domain port this repository has.**
//!
//! `AGENTS.md` §3.1 draws a line between a *domain port*, which exists so the
//! domain can express an operation it cannot perform itself, and an *adapter's
//! own I/O seam*, which exists so an adapter can be tested without its
//! platform. It then says, of the domain kind: *"none yet — see below"*.
//!
//! **These are the first ones, and they are the domain kind.** The test
//! `AGENTS.md` §3.1 gives is whether the trait's method signatures mention
//! anything the core is forbidden to name. They do not: everything below moves
//! `&[u8]` and fixed-size arrays. Nothing here touches the world — hashing a
//! buffer is pure computation. What `rivet-core` cannot do is not *reach* the
//! world; it is **link an implementation**, because `ADR-014` §3 keeps its
//! `[dependencies]` table empty so that every forbidden edge fails to compile.
//!
//! `ADR-029` §10 rules exactly this placement, in its own words:
//!
//! > **The header layout, ordering and refusals are pure computation and belong
//! > in `rivet-core`** — which has an empty dependency table by design —
//! > behind a `Verifier`/`Digest` port, with the crypto crate landing at the
//! > composition root exactly as `Kernel` and `BootMedium` already do. This also
//! > makes the property that matters — *nothing is written before verification*
//! > — testable against a fake, which a hard crypto dependency would not.
//!
//! # What "testable against a fake" does and does not buy
//!
//! Said plainly, because it is the exact place `AGENTS.md` §6 bites. A fake
//! [`Digest`] and a fake [`Verify`] prove the **ordering** — that no member's
//! bytes are handed back to a caller before a signature has been checked, and
//! that a refusal happens at the step `ADR-029` §5 says it does. They prove
//! **nothing about Ed25519 or BLAKE3**. That is what `rivet-crypto`'s suite is
//! for, and neither suite substitutes for the other.

use crate::package::format::{DIGEST_BYTES, PUBLIC_KEY_BYTES, SIGNATURE_BYTES};
use crate::package::kind::Kind;

/// `digest_alg`, header bytes 20..24. `ADR-029` §3.
///
/// A `u32` rather than an enum, deliberately. `ADR-029` §5 step 5 refuses any
/// value *"this build implements"* does not cover, and what a build implements
/// is a property of the [`Digest`] it was handed — so a closed enum here would
/// be `rivet-core` claiming to know the set of algorithms that exist, which is
/// exactly the claim it is in no position to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigestAlgorithm(pub u32);

impl DigestAlgorithm {
    /// `ADR-029` §7.2, ruled 2026-08-23: BLAKE3, 256-bit output, pure Rust.
    pub const BLAKE3: Self = Self(1);
}

impl core::fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BLAKE3 => write!(f, "blake3 ({})", self.0),
            _ => write!(f, "digest algorithm {}", self.0),
        }
    }
}

/// `sig_alg`, header bytes 16..20. `ADR-029` §3, and a `u32` for the reason
/// [`DigestAlgorithm`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureAlgorithm(pub u32);

impl SignatureAlgorithm {
    /// `ADR-029` §7.1, ruled 2026-08-23: Ed25519, raw. No X.509, no ASN.1, no
    /// DER — *"a public key that is 32 raw bytes has no parser to get wrong"*.
    pub const ED25519: Self = Self(1);
}

impl core::fmt::Display for SignatureAlgorithm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::ED25519 => write!(f, "ed25519 ({})", self.0),
            _ => write!(f, "signature algorithm {}", self.0),
        }
    }
}

/// A raw public key. **Not a certificate, not a SubjectPublicKeyInfo, not PEM.**
///
/// `ADR-029` §7.1's whole argument in one type: there is no length prefix, no
/// OID and no parser, so there is nothing here to get wrong before the
/// signature check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKey(pub [u8; PUBLIC_KEY_BYTES]);

/// The digest of a [`PublicKey`], as `ADR-028` §5 names it — header bytes
/// 24..56.
///
/// **Not the key itself**, which is what makes `ADR-029` §5 step 6 a table
/// lookup rather than a parse: the header says *which* key, the trust set says
/// what that key is, and an untrusted `key_id` is refused before any byte of it
/// is used for anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyId(pub [u8; DIGEST_BYTES]);

impl core::fmt::Display for KeyId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The first eight bytes, which is what an operator can compare by eye
        // and what a journal line has room for. Never the whole thing: a
        // 64-character hex string in a console message is a line nobody reads.
        for byte in &self.0[..8] {
            write!(f, "{byte:02x}")?;
        }
        write!(f, "…")
    }
}

/// Header bytes 4032..4096.
#[derive(Clone, Copy)]
pub struct Signature(pub [u8; SIGNATURE_BYTES]);

impl core::fmt::Debug for Signature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 64 bytes of hex in a `{:?}` is noise that hides the field beside it.
        write!(f, "Signature(64 bytes)")
    }
}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Signature {}

/// A streaming digest.
///
/// **Streaming rather than `fn digest(&[u8]) -> [u8; 32]`**, because the largest
/// member is a whole `/system` filesystem — 640 MiB on the card
/// `cmd/rivet-image` builds today — and a board with 4 GB and no swap must not
/// need it resident to check it. `cmd/rivet`'s `measure hash` already reads
/// that partition a megabyte at a time for the same reason.
pub trait Digest {
    /// Which algorithm this is. `ADR-029` §5 step 5 compares it against the
    /// header's `digest_alg` and refuses a mismatch, so a build that implements
    /// only BLAKE3 refuses a SHA-256 payload rather than mis-hashing it.
    fn algorithm(&self) -> DigestAlgorithm;

    /// Begin a new digest, discarding anything in progress.
    fn start(&mut self);

    /// Absorb the next chunk.
    fn write(&mut self, bytes: &[u8]);

    /// Finish, and return the digest.
    fn finish(&mut self) -> [u8; DIGEST_BYTES];

    /// One shot, for the small things — a public key, a 4 KiB header, a
    /// `cmdline.txt`.
    fn of(&mut self, bytes: &[u8]) -> [u8; DIGEST_BYTES] {
        self.start();
        self.write(bytes);
        self.finish()
    }
}

/// Signature verification. **This is the security boundary** — `ADR-028` §2's
/// last row.
pub trait Verify {
    /// Which scheme. Compared against the header's `sig_alg` at `ADR-029` §5
    /// step 5.
    fn algorithm(&self) -> SignatureAlgorithm;

    /// Does `signature` verify over `message` under `key`?
    ///
    /// **A `bool`, and never a `Result`.** There is exactly one useful outcome
    /// besides success, and giving an implementor a way to say "I could not
    /// tell" would create a third state that some caller would eventually treat
    /// as the first.
    fn verify(&self, key: &PublicKey, message: &[u8], signature: &Signature) -> bool;
}

/// Signing. **Not present on any Rivet machine** — this is a host-tool port,
/// for `cargo rivet package` (`ADR-015` §3.2) and for the test suite.
///
/// It is on the same port module as [`Verify`] rather than in a build tool
/// because the writer and the reader must agree about *what* is signed
/// (`[0 .. 4032)`), and putting them in one place is what stops the two
/// spellings diverging.
pub trait Sign {
    /// Which scheme. The writer stamps this into `sig_alg`, so a header cannot
    /// claim an algorithm other than the one that signed it.
    fn algorithm(&self) -> SignatureAlgorithm;

    /// The public half, which the writer digests into `key_id`.
    fn public_key(&self) -> PublicKey;

    /// Sign `message`.
    fn sign(&mut self, message: &[u8]) -> Signature;
}

/// The keys a verification will accept — `ADR-029` §5 step 6.
///
/// # What this type deliberately does not decide
///
/// **Whether the three artifact kinds share a trust set is open** —
/// `ADR-029` §12 item 1 and `ADR-028` §8 item 3, and both say a shared container
/// makes the question sharper rather than softer. So this type holds keys and
/// knows nothing about kinds: a caller that wants one set for everything passes
/// one, and a caller that wants a set per kind passes a different one each time.
/// Neither is expressed here, because expressing either would be ruling it.
///
/// **Nor does it decide where the keys came from.** `ADR-028` §5.2 rules a root
/// set that ships in `/system` plus countersigned configuration keys, and
/// nothing in this crate reads either — a trust set is built by the composition
/// root out of whatever it is entitled to trust.
///
/// # Amended 2026-08-24: an entry now says which kinds its key may sign
///
/// The paragraph above says this type "holds keys and knows nothing about
/// kinds", because *"expressing either would be ruling it"*. **It is ruled**
/// (`ADR-028` §5.5), so it is expressed here.
///
/// The maintainer keeps a key for Rivet itself and a key for the
/// applications he writes, and the reason is a capability rather than
/// bookkeeping: **the key that signs an application must not be able to ship a
/// base image.** Without that, compromising the machine that signs a hobby
/// application yields a signed operating system for every board in the fleet.
///
/// The set is arbitrary and `system`/`extension` are **not** welded together
/// (`ADR-028` §5.5.2, ruling 3): in practice one key signs
/// `{update, extension}` and another signs `{application}`, but nothing here
/// assumes that, so granting a third party the right to ship one extension is
/// configuration rather than a redesign.
#[derive(Debug, Clone, Default)]
pub struct TrustSet {
    entries: Vec<TrustEntry>,
}

/// Which artifact kinds one trusted key may sign.
///
/// **A list rather than a bitmask, because [`Kind`] is open.** `Kind::parse`
/// accepts any well-formed name up to `KIND_BYTES`; it is not a closed enum, so
/// a bitmask would have had to close it as a side effect of describing trust.
/// `ADR-028` §5.5.2 asked for an arbitrary subset and this is what an arbitrary
/// subset of an open set looks like.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaySign(Vec<Kind>);

impl MaySign {
    /// A key entitled to sign exactly these kinds.
    #[must_use]
    pub fn these(kinds: &[Kind]) -> Self {
        let mut out: Vec<Kind> = Vec::new();
        for k in kinds {
            if !out.contains(k) {
                out.push(k.clone());
            }
        }
        Self(out)
    }

    /// **Nothing.** A key that may sign nothing is refused for everything, which
    /// is a usable state: it is what a revoked entry becomes before the image
    /// carrying its removal is built.
    #[must_use]
    pub fn nothing() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn permits(&self, kind: &Kind) -> bool {
        self.0.contains(kind)
    }

    #[must_use]
    pub fn kinds(&self) -> &[Kind] {
        &self.0
    }
}

/// One key this machine trusts, and the limits of that trust.
#[derive(Debug, Clone)]
pub struct TrustEntry {
    id: KeyId,
    key: PublicKey,
    label: String,
    may_sign: MaySign,
}

impl TrustEntry {
    #[must_use]
    pub fn id(&self) -> KeyId {
        self.id
    }

    #[must_use]
    pub fn key(&self) -> &PublicKey {
        &self.key
    }

    /// A name for refusals, and **never** for authority.
    ///
    /// It exists so a refusal can say *"signed by `joel-apps`, which may not
    /// sign a system image"* rather than *"untrusted key"* — `ADR-006`'s
    /// explainability argument applied to the one place in the project where a
    /// bad diagnosis costs the most. Nothing is decided by comparing it.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn may_sign(&self) -> &MaySign {
        &self.may_sign
    }
}

impl TrustSet {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a key, computing its id with `digest`.
    ///
    /// **The id is derived here rather than supplied**, so a caller cannot file
    /// a key under an id that is not its digest — which would make the header's
    /// `key_id` name one key and the verification use another.
    pub fn trust(
        &mut self,
        digest: &mut dyn Digest,
        key: PublicKey,
        label: &str,
        may_sign: MaySign,
    ) -> KeyId {
        let id = KeyId(digest.of(&key.0));
        if !self.entries.iter().any(|e| e.id == id) {
            self.entries.push(TrustEntry {
                id,
                key,
                label: label.to_string(),
                may_sign,
            });
        }
        id
    }

    /// The entry `id` names, if this set has it.
    ///
    /// **Returns the entry rather than the key**, since 2026-08-24: a caller
    /// that got only the key could verify a signature and would have nothing
    /// left to check the kind against, which is the whole of `ADR-028` §5.5.
    pub fn find(&self, id: &KeyId) -> Option<&TrustEntry> {
        self.entries.iter().find(|e| e.id == *id)
    }

    /// Every entry, for `ADR-028` §5.5.6 — a machine should be able to say what
    /// it trusts.
    pub fn entries(&self) -> &[TrustEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::package::testing::CountingDigest;

    #[test]
    fn a_key_is_found_by_the_digest_of_itself_and_not_by_anything_else() {
        let mut digest = CountingDigest::new();
        let mut trust = TrustSet::new();
        let key = PublicKey([7u8; PUBLIC_KEY_BYTES]);
        let id = trust.trust(&mut digest, key, "a", MaySign::these(&[Kind::update()]));

        let found = trust.find(&id).expect("the key just added");
        assert_eq!(found.key(), &key);
        assert_eq!(found.label(), "a");
        assert!(found.may_sign().permits(&Kind::update()));
        assert!(!found.may_sign().permits(&Kind::application()));
        assert!(trust.find(&KeyId([0u8; DIGEST_BYTES])).is_none());
        assert_eq!(id, KeyId(digest.of(&key.0)));
    }

    #[test]
    fn trusting_the_same_key_twice_does_not_grow_the_set() {
        let mut digest = CountingDigest::new();
        let mut trust = TrustSet::new();
        let key = PublicKey([1u8; PUBLIC_KEY_BYTES]);
        let first = trust.trust(&mut digest, key, "a", MaySign::these(&[Kind::update()]));
        let second = trust.trust(&mut digest, key, "a", MaySign::these(&[Kind::update()]));
        assert_eq!(first, second);
        assert_eq!(trust.len(), 1);
    }

    #[test]
    fn an_algorithm_names_itself_and_an_unknown_one_is_a_number() {
        assert!(DigestAlgorithm::BLAKE3.to_string().contains("blake3"));
        assert_eq!(DigestAlgorithm(9).to_string(), "digest algorithm 9");
        assert!(SignatureAlgorithm::ED25519.to_string().contains("ed25519"));
        assert_eq!(SignatureAlgorithm(9).to_string(), "signature algorithm 9");
    }

    #[test]
    fn a_key_id_prints_a_prefix_rather_than_sixty_four_characters() {
        let id = KeyId([0xab; DIGEST_BYTES]);
        assert_eq!(id.to_string(), "abababababababab…");
    }
}
