//! `.rvt` — the signed container. `ADR-029`.
//!
//! **One reader, one writer, one verifier**, with `<kind>` saying how the
//! metadata is to be read. `ADR-029` §11 rules that, and the argument is
//! `ADR-028` §2's last row:
//!
//! > **The decisive argument is `ADR-028` §2's last row again: the verifier is
//! > the most security-sensitive parser on the machine. Three formats means
//! > three verifiers, each with a third of the scrutiny.** One is strictly
//! > better.
//!
//! So an application, an extension and an update payload are all *a signed set
//! of named members plus typed metadata*, and what differs is the
//! **interpretation** of the metadata rather than its shape. `update` is the
//! first case ([`update`]); ~~the other two are `ADR-027` §9 items 2–4 and are
//! the maintainer's to rule. Nothing here presumes them, and [`kind::Kind`] is a
//! validated token rather than an enum precisely so that they stay decidable.~~
//!
//! **`application` is the second, ruled 2026-08-24** — the maintainer's ruling
//! dropping `<platform>` from the name, written into `ADR-027` §9 items 2–4. `ADR-016`'s extension is still
//! unruled and is still not presumed here.
//!
//! **§11 was load-bearing rather than decorative, and adding the second kind is
//! what proved it:** [`read`], [`mod@write`], [`mod@format`], [`port`] and [`kind`]'s
//! validation are byte-for-byte unchanged by it. A kind is a module in this
//! directory and a token in a header; it is not a second verifier.
//!
//! # The shape of this module
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`mod@format`] | `ADR-029` §3's byte layout as constants, and the text rules. Decides nothing |
//! | [`port`] | the cryptography seam — [`port::Digest`], [`port::Verify`], [`port::Sign`], [`port::TrustSet`] |
//! | [`error`] | every refusal, each carrying what was found |
//! | [`kind`] | the `<kind>` token and the `.rvt` suffix |
//! | [`read`] | **the verifier.** `ADR-029` §5's order, then the manifest |
//! | [`verify`] | **the one sequence that uses it** — trust set from the machine's store, header, every member — called by the installer and by the artifact store's door alike (`ADR-052` §7) |
//! | [`mod@write`] | the builder, holding itself to the reader's rules |
//! | [`update`] | what the metadata means for `ADR-029` §8's `update` kind |
//! | [`application`] | what it means for `ADR-015`'s `application` kind, ruled 2026-08-24 |
//! | `testing` | fakes, under `cfg(test)` |
//!
//! # Why it is here rather than in an adapter
//!
//! `ADR-029` §10 rules it, against `rivet-adapter-image`'s row:
//!
//! > It has no dependencies and `unsafe_code = "forbid"`; a verifier needs
//! > crypto. **The header layout, ordering and refusals are pure computation and
//! > belong in `rivet-core`** — which has an empty dependency table by design
//! > — behind a `Verifier`/`Digest` port, with the crypto crate landing at the
//! > composition root exactly as `Kernel` and `BootMedium` already do. This also
//! > makes the property that matters — *nothing is written before verification*
//! > — testable against a fake, which a hard crypto dependency would not.
//!
//! `rivet-core`'s `[dependencies]` table is still empty, and that is what
//! makes `ADR-014` §3's forbidden edges fail to compile rather than fail review.
//! The consequence is that **this module cannot hash and cannot verify** — it
//! can only say what must be hashed, in what order, and what to do when it does
//! not match. `rivet-crypto` supplies the implementations.
//!
//! # What is built, and what is not — `AGENTS.md` §6 and §7
//!
//! Built: the reader, the writer, the verifier, `update`'s interpretation, and
//! `ADR-029` §4's reserved-name refusal.
//!
//! **Not built, and deliberately:**
//!
//! * **Compression.** `ADR-029` §12 item 2 has not ruled a decompressor crate,
//!   and §5.1 puts compression in the one position the design was shaped to
//!   protect. `compression` is read, and any value but `0` is refused.
//! * **Version ordering.** `ADR-028` §6.1 says report a non-newer payload rather
//!   than refuse it. [`read::Header::version`] carries the string; nothing here
//!   orders two of them, because no ADR says what a version string means and a
//!   wrong order would mislabel a downgrade as an upgrade.
//! * **The installer.** `ADR-029` §12 item 4 — *which process installs an
//!   update* — is open, so nothing here writes to a slot. What is here is the
//!   part an installer would need whichever process turns out to own it,
//!   including [`read::VerifiedMember`], which is the token an installer should
//!   be made to hold.
//! * **Key management.** `ADR-015` §4.3 makes the maintainer the gatekeeper and
//!   there is no key story anywhere. [`port::TrustSet`] holds keys and has no
//!   opinion about where they came from.
//!
//! **Nothing in this module has ever run on a board.**

pub mod apk;
pub mod application;
pub mod arch;
// Every installed application's declared closure, read once for the two
// things that must never remove a library an application names: retention
// and `apply uninstall-library`.
pub mod closure;
pub mod error;
pub mod format;
pub mod install;
pub mod key_material;
pub mod kind;
pub mod library;
pub mod port;
pub mod read;
pub mod update;
pub mod verify;
pub mod write;

#[cfg(test)]
pub mod testing;

#[cfg(test)]
mod tests;
pub mod trust_store;

pub use error::PackageError;
pub use kind::{Kind, SUFFIX};
pub use port::{
    Digest, DigestAlgorithm, KeyId, PublicKey, Sign, Signature, SignatureAlgorithm, TrustSet,
    Verify,
};
pub use read::{COMPRESSION_NONE, Header, Member, MemberStream, Package, VerifiedMember, open};
pub use write::{Builder, NewMember};
