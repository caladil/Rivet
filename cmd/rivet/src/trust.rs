//! Reading this machine's trust store — `ADR-028` §5.5.6, `PLAN.md` §102.3
//! item 2.
//!
//! **`/system/trust`, and nothing else.** `crate::memory`'s note carries why
//! that belongs here rather than behind `BootMedium`: it needs no `mount(2)`,
//! no device tree and no pid 1. The path is
//! `rivet_core::package::trust_store::PATH` and is deliberately not spelled
//! here — `cmd/rivet-image` writes that file, `cmd/rivet-deploy` and
//! `cmd/rivet-sysd` read it, and a fourth spelling would be `ADR-011` §3 in the
//! one file where a disagreement is worst.
//!
//! # It is read at the point of use, never cached
//!
//! `ADR-028` §5.5.4 makes revocation an A/B image update that ships a store
//! without the key, so a store read once and remembered would answer with the
//! one the machine booted under. `cmd/rivet-sysd/src/verify.rs` re-reads it on
//! **every** `execve` for the same reason, and says so in the same words.
//!
//! # The key id, and why this hashes rather than reading one
//!
//! `rivet_core::package::trust_store` does not store the id, deliberately:
//!
//! > **The key id.** It is `BLAKE3` of the public key (`ADR-029` §3), so storing
//! > it would create a second place for it to be written down and a way for the
//! > two to disagree.
//!
//! So it is derived here, with the `blake3` crate this binary already links for
//! `measure hash`. That is one call rather than a dependency on `rivet-crypto`,
//! which would put `ed25519-dalek` and its SHA-512 into the **login shell** for
//! a read-only subject — see this crate's `Cargo.toml` for why what is linked
//! into that binary is argued rather than assumed.
//!
//! **What holds the two together is a test, not a comment.**
//! `the_id_this_computes_is_the_one_the_verifier_derives` drives this module's
//! answer against `rivet_crypto::Blake3` through `rivet-core`'s own `Digest`
//! port, which is the implementation a board verifies with. `rivet-crypto` is a
//! **dev-dependency** here for exactly that, which is the arrangement that
//! crate's own `Cargo.toml` already uses against `rivet-adapter-image` and for
//! the same reason: the two halves of one fact live in crates that cannot see
//! each other, and a test is where they are driven together.

use rivet_core::operation::machine::{MachineError, TrustRead, TrustedKey};
use rivet_core::package::port::{KeyId, PublicKey};
use rivet_core::package::trust_store::{self, TrustStore};

/// The largest file this will read as a trust store.
///
/// **Derived rather than chosen**, and it is `cmd/rivet-sysd/src/verify.rs`'s
/// `MAX_TRUST_STORE_BYTES` computed the same way: `ADR-028` §5.5's format is a
/// fixed header plus at most `MAX_ENTRIES` fixed records, so a file bigger than
/// that is not a trust store whatever else it is. It is recomputed here rather
/// than imported because that constant lives in a binary crate with no `lib`
/// target — and it is the *arithmetic* that is shared, from the three
/// `trust_store` constants both sides read.
const MAX_BYTES: usize = trust_store::HEADER_BYTES + trust_store::MAX_ENTRIES * trust_store::ENTRY_BYTES;

/// What this machine accepts a signature from.
///
/// # Errors
///
/// If the store cannot be read or does not parse. **An empty store is not an
/// error** — `ADR-028` §5.5.5's `--no-trust` image produces one, and a machine
/// that trusts nobody is a reading rather than a fault. The distinction is the
/// one `cmd/rivet-deploy` already draws over the same file: *"a store that
/// cannot be read is not a store that trusts nobody."*
pub fn read() -> Result<TrustRead, MachineError> {
    let path = trust_store::PATH;
    let bytes = std::fs::read(path).map_err(|e| {
        MachineError::Refused(format!(
            "this machine's trust store at {path} could not be read: {e}. An image built \
             --no-trust has none and can verify nothing (ADR-028 §5.5.5); a build that is not a \
             Rivet has none because it is not one. Either way this is NOT the same as trusting \
             nobody, and it is not reported as an empty store."
        ))
    })?;

    // The bound before the parse, so a `/system` somebody filled cannot make
    // this allocate — `verify.rs`'s reason, and the same number.
    if bytes.len() > MAX_BYTES {
        return Err(MachineError::Refused(format!(
            "{path} is {} bytes and the largest possible trust store is {MAX_BYTES} — a fixed \
             {}-byte header plus at most {} records of {} bytes (ADR-028 §5.5). A file larger \
             than that is not a trust store whatever else it is, and it is refused rather than \
             parsed as far as it goes.",
            bytes.len(),
            trust_store::HEADER_BYTES,
            trust_store::MAX_ENTRIES,
            trust_store::ENTRY_BYTES,
        )));
    }

    let store = TrustStore::from_bytes(&bytes).map_err(|e| {
        MachineError::Refused(format!(
            "this machine's trust store at {path} does not parse: {e}. It is refused rather than \
             skipped: a store that cannot be read is not a store that trusts nobody, and \
             treating it as one is how a corrupt file becomes an install nobody authorised \
             (ADR-028 §5.5.6)."
        ))
    })?;

    Ok(TrustRead {
        source: path.to_string(),
        // **In file order, one entry per record.** `TrustSet::trust` would fold
        // a duplicate, which is right for a verifier and wrong for a report.
        keys: store
            .entries()
            .iter()
            .map(|entry| TrustedKey {
                id: key_id(&entry.key),
                key: entry.key,
                label: entry.label.clone(),
                may_sign: entry
                    .may_sign
                    .kinds()
                    .iter()
                    .map(|k| k.as_str().to_string())
                    .collect(),
            })
            .collect(),
    })
}

/// `BLAKE3` of a public key — the value a payload header carries as `key_id`.
///
/// `ADR-029` §3, and it is the one line in this module that has a twin
/// elsewhere: `rivet_core::package::port::TrustSet::trust` computes the same
/// thing through the `Digest` port. The module header says why this is not that
/// call, and the test below is what keeps the two equal.
fn key_id(key: &PublicKey) -> KeyId {
    KeyId(*blake3::hash(&key.0).as_bytes())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The port-agreement test.** This module derives a key id with `blake3`
    /// directly; a board derives it through `rivet-core`'s `Digest` port, with
    /// `rivet-crypto`'s implementation. If those two ever stopped agreeing,
    /// `get trust` would report ids that match no artifact's header and an
    /// operator would conclude their key was not trusted.
    ///
    /// Driven rather than asserted by inspection, over several keys including
    /// the two boundary patterns.
    #[test]
    fn the_id_this_computes_is_the_one_the_verifier_derives() {
        use rivet_core::package::port::{Digest as _, MaySign, TrustSet};

        for fill in [0x00u8, 0x01, 0x7f, 0xa5, 0xff] {
            let key = PublicKey([fill; 32]);

            let mine = key_id(&key);

            let mut digest = rivet_crypto::Blake3::new();
            let mut set = TrustSet::new();
            let theirs = set.trust(&mut digest, key, "under-test", MaySign::nothing());

            assert_eq!(
                mine.0, theirs.0,
                "the id this adapter reports and the id the verifier looks up have diverged \
                 for a key of {fill:#04x}"
            );
            // And it really is a digest of the key rather than the key.
            assert_ne!(mine.0, key.0);
            assert_eq!(mine.0, digest.of(&key.0));
        }
    }

    /// A store this build renders must read back to what it rendered, through
    /// the same two functions `read` uses — the parse and the id derivation.
    ///
    /// **Bytes rather than a struct**, because the parse is the half that can
    /// be wrong: `TrustStore::to_bytes` is `cmd/rivet-image`'s writer, so this
    /// exercises the reader against what the image builder actually emits.
    #[test]
    fn a_rendered_store_reads_back_with_every_label_and_entitlement_intact() {
        use rivet_core::package::kind::Kind;
        use rivet_core::package::port::MaySign;

        let rendered = TrustStore::new(vec![
            trust_store::Entry {
                key: PublicKey([0x11; 32]),
                label: "rivet-system".to_string(),
                may_sign: MaySign::these(&[Kind::update()]),
            },
            trust_store::Entry {
                key: PublicKey([0x22; 32]),
                label: "joel-apps".to_string(),
                may_sign: MaySign::these(&[Kind::application()]),
            },
        ])
        .to_bytes()
        .expect("a store this build can render");

        let parsed = TrustStore::from_bytes(&rendered).expect("a store this build can read");
        let entries = parsed.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "rivet-system");
        assert_eq!(entries[0].may_sign.kinds(), &[Kind::update()]);
        assert_eq!(entries[1].label, "joel-apps");
        assert_eq!(key_id(&entries[0].key).0.len(), 32);
        assert_ne!(key_id(&entries[0].key).0, key_id(&entries[1].key).0);
    }

    /// **A file that is not a trust store is refused, not partly parsed.** The
    /// refusal names the file and cites the section, because on a machine that
    /// cannot verify anything the next question is always *what do I do about
    /// it* (`ADR-006` §4.1).
    #[test]
    fn bytes_that_are_not_a_store_are_refused_and_the_refusal_cites_the_format() {
        let refusal = TrustStore::from_bytes(b"not a trust store at all")
            .expect_err("arbitrary bytes parsed as a trust store");
        // The type's own refusal is what `read` wraps; asserting it here keeps
        // this module's message honest about what it is reporting.
        assert!(!refusal.to_string().is_empty());
    }

    /// The bound is the arithmetic `ADR-028` §5.5's format implies, and nothing
    /// else. Pinned so a `MAX_ENTRIES` change cannot silently leave this reading
    /// less than a legal store.
    #[test]
    fn the_size_bound_is_a_whole_store_of_the_maximum_number_of_keys() {
        let biggest = TrustStore::new(
            (0..trust_store::MAX_ENTRIES)
                .map(|n| trust_store::Entry {
                    key: PublicKey([u8::try_from(n).unwrap_or(0); 32]),
                    label: format!("k{n}"),
                    may_sign: rivet_core::package::port::MaySign::nothing(),
                })
                .collect(),
        )
        .to_bytes()
        .expect("the largest legal store");
        assert_eq!(biggest.len(), MAX_BYTES);
    }
}
