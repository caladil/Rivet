//! **Is this whole artifact one this machine would accept?** — one answer,
//! consulted wherever an artifact is admitted.
//!
//! [`crate::package::read`] is the verifier: `ADR-029` §5's seven steps, then
//! the manifest, then a member at a time. What it does not do is *be used*:
//! something has to read the header off a medium, build the trust set out of the
//! machine's own store, hand every member's bytes to its stream in order, and
//! say which failures are the artifact's and which are the medium's. Until
//! 2026-09-12 that sequence was written out inside `cmd/rivet-deploy`, where the
//! installer was the only caller there was.
//!
//! # Why it moved, and why that is not a second verifier
//!
//! **The maintainer's ruling, 2026-09-12** (`.rulings/` R-308): the artifact
//! store holds no unverified object, however the check is shared. An artifact
//! entered the machine's artifact store before anything had checked its
//! signature — the bulk endpoint hashed the bytes and named them, and only the
//! installer, later, refused. So a refused package stayed in the store, and one
//! carrying a high version held a retention slot for as long as nothing higher
//! arrived. The remedy is to refuse at the door (`ADR-052` §7), and the door is
//! `cmd/rivet-mcpd`.
//!
//! That puts a second **consultation** of the verifier in a second process, and
//! `ADR-029` §11's rule — *"three formats means three verifiers, each with a
//! third of the scrutiny"* — is about **implementations**. Two copies of this
//! sequence could disagree: one reads the header and not the members, one maps
//! a short read to a refusal and the other to success, one builds its trust set
//! with a narrowed mask. The weaker copy is the one that eventually gets
//! believed. One copy, called from two places, cannot disagree with itself. So
//! the sequence lives here, and the installer and the store's door both call it.
//!
//! **This is not new in kind.** `cmd/rivet-sysd` has consulted
//! [`crate::package::read`] at every launch since 2026-08-25 (`ADR-015` §4),
//! a second point of consultation of one verifier, in a second process — and
//! `ADR-028` §5's revocation is built on it.
//!
//! # What "verified" means here, and what it deliberately does not
//!
//! [`artifact`] establishes exactly two things, against the trust set it is
//! handed:
//!
//! 1. **The header is signed by a key that set trusts, and that key may sign
//!    this kind** — `ADR-029` §5 steps 1–7 and `ADR-028` §5's entitlement.
//! 2. **Every member's bytes are the ones the signature covers** — §5 step 8,
//!    for every member, with the length checked before the digest is consulted.
//!
//! Together those are *authenticity*: the bytes are ours, all of them. They are
//! properties of the artifact against the keys the machine trusts, and they do
//! not change with anything else about the machine.
//!
//! # And one applicability question: the architecture — [`architecture`]
//!
//! **An artifact this machine cannot execute is garbage too**, under the same
//! ruling (`PLAN-Feature-Set-2026-09-12` §3): it would install and then fail
//! when it ran. So [`architecture`] is asked at both points as well, for every
//! kind, against the token the IMAGE recorded ([`crate::package::arch`]).
//!
//! It belongs beside authenticity rather than with the installer's questions
//! because of where its answer comes from. The trust set and the architecture
//! record are both files in `/system`, fixed for the life of the image, so a
//! verdict on either is as true at the door as at install. **Everything else
//! about applicability is not**: whether a slot will take a system update,
//! whether an application's closure is satisfied, whether a version directory
//! is already occupied, whether the artifact is already what this machine is
//! running — each depends on state that changes between the door and the
//! install (the running slot, what is under `/state/app` and `/state/lib`) or on
//! a boot port only the installer links, and the installer refuses those, in the
//! journal, with nothing written.
//!
//! **A system update's product and platform stay the installer's.** The product
//! is a constant only the installer is built against, and the platform is the
//! boot port's answer.
//!
//! # A verdict is true of the trust store it was reached against
//!
//! [`trust_set`] reads the store each time it is called and never caches, for
//! `ADR-028` §5's reason: revocation is an image update that replaces that
//! file. So an artifact admitted today was verified against today's store, and
//! **the installer asks again** when the install runs — against the store in
//! force then — which is why the installer keeps its check. The store is also
//! not the only way bytes reach `/state`.
//!
//! # No I/O
//!
//! `ADR-014` §3. The caller supplies a [`Source`] and the store's bytes; this
//! module decides.

use crate::package::apk::Apk;
use crate::package::application::Application;
use crate::package::error::PackageError;
use crate::package::format::HEADER_BYTES;
use crate::package::kind::Kind;
use crate::package::library::Library;
use crate::package::update;
use crate::package::port::{Digest, TrustSet, Verify};
use crate::package::read::{Package, VerifiedMember, open};
use crate::package::trust_store::{self, TrustStore};

/// Where an artifact's bytes are read from.
///
/// **A port with one method**, because the two callers read from different
/// worlds: `cmd/rivet-deploy` through its `Kernel`, which a test replaces with a
/// map, and `cmd/rivet-mcpd` from a file it has just written with `std::fs`.
pub trait Source {
    /// What a failed read reports.
    type Error;

    /// Feed `len` bytes starting at `offset` to `sink`, in order, a chunk at a
    /// time.
    ///
    /// `sink` returns `false` to stop early, which is how a member whose stream
    /// has already refused stops a 640 MiB read it no longer wants; stopping
    /// early is `Ok`. **Fewer than `len` bytes is an error, not a short answer**:
    /// the length came from a signed header, so a source that cannot supply it
    /// is truncated.
    ///
    /// # Errors
    ///
    /// When the medium cannot supply the bytes.
    fn read_range(
        &mut self,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), Self::Error>;
}

/// What was being read when a [`Source`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// The 4 KiB signed header.
    Header,
    /// The member at this index.
    Member(usize),
}

/// Why an artifact is not accepted.
///
/// **Two kinds, because they mean opposite things to whoever reads them.** A
/// [`Fault::Artifact`] is the artifact's own and a different artifact would
/// help; a [`Fault::Medium`] is the machine's, and the same artifact on a
/// working medium might verify. A caller that discards anything on refusal must
/// discard only on the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault<E> {
    /// The artifact does not verify.
    Artifact(PackageError),
    /// Its bytes could not be read.
    Medium { reading: Reading, error: E },
}

/// Why a machine's trust store yields no trust set.
///
/// **Three situations, not one**, which `ADR-028` §5 separated because
/// *"no key"* was three situations wearing one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustRefusal {
    /// The file could not be read — which is what a `--no-trust` card looks
    /// like, and also what a damaged one does.
    Unreadable(String),
    /// The file was read and does not parse.
    Corrupt(PackageError),
    /// The file parses and holds no key.
    Empty,
}

impl core::fmt::Display for TrustRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(
                f,
                "cannot read this machine's trust store at {}: {why}. An image built --no-trust \
                 has none, and verifies nothing (ADR-028 §5)",
                trust_store::PATH
            ),
            Self::Corrupt(why) => write!(
                f,
                "this machine's trust store at {} does not parse: {why}. It is refused rather \
                 than skipped: a store that cannot be read is not a store that trusts nobody, \
                 and treating it as one is how a corrupt file becomes an install nobody \
                 authorised",
                trust_store::PATH
            ),
            Self::Empty => write!(
                f,
                "this machine's trust store at {} is empty, so it trusts no signing key and can \
                 verify nothing (ADR-028 §5). This is what a card built --no-trust looks \
                 like from here",
                trust_store::PATH
            ),
        }
    }
}

/// The machine's trust set, from the bytes of its trust store.
///
/// `stored` is the result of reading [`trust_store::PATH`] — or wherever a
/// caller's test keeps one — **at the moment of use**. Each entry keeps its own
/// label and its own mask, never a narrowed one, so a key the store holds for
/// applications is refused for an update by the same rule everywhere.
///
/// # Errors
///
/// [`TrustRefusal`], for a store that is unreadable, corrupt or empty. An empty
/// store is refused rather than returned as an empty set: an empty set refuses
/// everything too, and would do so with a sentence about an untrusted key rather
/// than about the machine.
pub fn trust_set<E: core::fmt::Display>(
    stored: Result<Vec<u8>, E>,
    digest: &mut dyn Digest,
) -> Result<TrustSet, TrustRefusal> {
    let bytes = stored.map_err(|e| TrustRefusal::Unreadable(e.to_string()))?;
    let store = TrustStore::from_bytes(&bytes).map_err(TrustRefusal::Corrupt)?;
    if store.is_empty() {
        return Err(TrustRefusal::Empty);
    }
    let mut trust = TrustSet::new();
    for entry in store.entries() {
        trust.trust(digest, entry.key, &entry.label, entry.may_sign.clone());
    }
    Ok(trust)
}

/// A header that verified, and the exact bytes it verified over.
///
/// **The bytes are kept** because two installs need them after the fact:
/// `ADR-015` §4 retains an application's signed header beside it, and a
/// library's directory is named by a digest of it. Handing back the buffer the
/// signature was checked over means neither re-reads a header that might not be
/// the one that verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    package: Package,
    head: Vec<u8>,
}

impl Opened {
    #[must_use]
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// The [`HEADER_BYTES`] the signature was checked over.
    #[must_use]
    pub fn head(&self) -> &[u8] {
        &self.head
    }
}

/// `ADR-029` §5 steps 1–7, read off a [`Source`] — **no member is checked**.
///
/// For a caller that has cheap refusals to make before the expensive pass: the
/// installer asks whether a slot will take an update before it reads 640 MiB.
/// Anything that must not act on an unverified member follows this with
/// [`members`]; [`artifact`] is both.
///
/// # Errors
///
/// [`Fault::Artifact`] for anything `read::open` refuses, [`Fault::Medium`] when
/// the header cannot be read.
pub fn header<S: Source + ?Sized>(
    source: &mut S,
    file_len: u64,
    digest: &mut dyn Digest,
    verify: &dyn Verify,
    trust: &TrustSet,
) -> Result<Opened, Fault<S::Error>> {
    let mut head = vec![0u8; HEADER_BYTES];
    // A file shorter than a header is not read at all: `open`'s step 1 refuses
    // it on `file_len` alone, and asking a medium for 4 KiB it does not have
    // would turn the artifact's fault into the medium's.
    if file_len >= HEADER_BYTES as u64 {
        let mut at = 0usize;
        source
            .read_range(0, HEADER_BYTES as u64, &mut |chunk| {
                let end = (at + chunk.len()).min(head.len());
                head[at..end].copy_from_slice(&chunk[..end - at]);
                at = end;
                at < head.len()
            })
            .map_err(|error| Fault::Medium {
                reading: Reading::Header,
                error,
            })?;
    }
    let package = open(&head, file_len, digest, verify, trust).map_err(Fault::Artifact)?;
    Ok(Opened { package, head })
}

/// `ADR-029` §5 step 8 for one member, streamed off a [`Source`].
///
/// # Errors
///
/// [`Fault::Artifact`] when the member is absent from the table, overruns or falls short
/// of its record, or does not match its digest; [`Fault::Medium`] when its bytes
/// cannot be read.
pub fn member<'p, S: Source + ?Sized>(
    source: &mut S,
    package: &'p Package,
    index: usize,
    digest: &mut dyn Digest,
) -> Result<VerifiedMember<'p>, Fault<S::Error>> {
    let record = package
        .members()
        .get(index)
        .ok_or(Fault::Artifact(PackageError::NoSuchMember {
            index,
            member_count: package.members().len(),
        }))?;
    let (offset, len) = (record.offset(), record.stored_len());
    let mut stream = package
        .member_stream(index, digest)
        .map_err(Fault::Artifact)?;
    let mut refused: Option<PackageError> = None;
    source
        .read_range(offset, len, &mut |chunk| match stream.write(chunk) {
            Ok(()) => true,
            Err(e) => {
                refused = Some(e);
                false
            }
        })
        .map_err(|error| Fault::Medium {
            reading: Reading::Member(index),
            error,
        })?;
    if let Some(e) = refused {
        return Err(Fault::Artifact(e));
    }
    stream.finish().map_err(Fault::Artifact)
}

/// Every member, in file order, **before any of them is handed back**.
///
/// # Errors
///
/// The first member that fails, as [`member`] reports it.
pub fn members<'p, S: Source + ?Sized>(
    source: &mut S,
    package: &'p Package,
    digest: &mut dyn Digest,
) -> Result<Vec<VerifiedMember<'p>>, Fault<S::Error>> {
    let mut verified = Vec::with_capacity(package.members().len());
    for index in 0..package.members().len() {
        verified.push(member(source, package, index, digest)?);
    }
    Ok(verified)
}

/// **The whole artifact**: [`header`], then [`members`].
///
/// What the artifact store's door asks. On `Ok` the signature is by a key
/// `trust` holds, that key may sign this kind, and every byte of every member is
/// covered by it — so nothing in the file is unaccounted for, because
/// `read::open` has already refused a file with bytes after the last member.
///
/// # Errors
///
/// As [`header`] and [`member`].
pub fn artifact<S: Source + ?Sized>(
    source: &mut S,
    file_len: u64,
    digest: &mut dyn Digest,
    verify: &dyn Verify,
    trust: &TrustSet,
) -> Result<Opened, Fault<S::Error>> {
    let opened = header(source, file_len, digest, verify, trust)?;
    members(source, &opened.package, digest)?;
    Ok(opened)
}

/// **Can this machine execute what `package` carries?** — its header's `<arch>`
/// against `arch`, the token [`crate::package::arch::read`] took from the image.
///
/// **One question for every kind, answered by each kind's own check**:
/// `Application::accepts` and `Library::accepts`, which existed with no caller
/// until 2026-09-12, and `update::accepts_arch`, which `Payload::accepts` also
/// calls. A kind this build does not interpret is compared on the header field
/// directly — every `ADR-029` §3 header carries one — rather than admitted
/// because nothing here knows its name.
///
/// Opening an application or a library is part of asking, so an artifact of
/// that kind whose structure its own reader refuses is refused here with that
/// reader's reason. That is not a new refusal: the installer opens it the same
/// way before it writes anything.
///
/// # Errors
///
/// [`PackageError::WrongKind`] naming both architectures, or whatever the kind's
/// reader refuses the artifact for.
pub fn architecture(package: &Package, arch: &str) -> Result<(), PackageError> {
    let kind = package.header().kind();
    if *kind == Kind::application() {
        return Application::open(package)?.accepts(arch);
    }
    if *kind == Kind::library() {
        return Library::open(package)?.accepts(arch);
    }
    // **`ADR-061` §3: an apk's `arch` is always `"any"`, and architecture is
    // not this machine's question to ask of it** — it runs on whatever phone
    // `gantry` hands it to, never on this machine. `Apk::open` already checks
    // the header carries `"any"`; there is nothing here to compare it against.
    if *kind == Kind::apk() {
        return Apk::open(package).map(|_| ());
    }
    update::accepts_arch(package, arch)
}

/// **Is this an application whose name an image service already holds?**
/// `PLAN-Feature-Set-2026-09-12` §3, `.rulings/` R-156 R10.
///
/// Checked at the same point [`architecture`] is, by the same two callers —
/// `cmd/rivet-mcpd`'s door and, as defence in depth, `cmd/rivet-deploy`'s
/// installer — so an application named like [`crate::service::NETD_SERVICE_NAME`]
/// or any other of [`crate::service::IMAGE_SERVICE_NAMES`] is refused before it
/// is ever written, rather than written and then refused every boot with no
/// way for an uninstall to name it (`cmd/rivet-sysd`'s
/// `Supervisor::begin_uninstall` finds the image's service of that name first).
///
/// Only an application's header names a service at all — a system update and a
/// library make no claim of that shape — so every other kind answers `Ok(())`
/// unconditionally, exactly as [`architecture`] defers to each kind's own
/// reader.
///
/// # Errors
///
/// [`PackageError::NamesAnImageService`], naming the clash.
pub fn not_an_image_service(package: &Package) -> Result<(), PackageError> {
    if *package.header().kind() != Kind::application() {
        return Ok(());
    }
    let name = package.header().product();
    if crate::service::IMAGE_SERVICE_NAMES.contains(&name)
        || crate::service::is_network_mechanism_service(name)
    {
        return Err(PackageError::NamesAnImageService {
            name: name.to_string(),
        });
    }
    Ok(())
}

/// **Is this an application whose name this project's own machinery already
/// means something else by?** `.rulings/` R-210 D21.
///
/// Checked beside [`not_an_image_service`], at the same two doors: `cmd/rivet-mcpd`'s
/// install door, and `cmd/rivet-deploy`'s installer as defence in depth. See
/// [`crate::service::reserved_application_names`] for what the two checks
/// answer differently and why a name needs only one of them to be refused.
///
/// Only an application's header names anything a person or a script could
/// collide with — a system update and a library carry no name of that
/// shape — so every other kind answers `Ok(())` unconditionally, exactly as
/// [`not_an_image_service`] does.
///
/// # Errors
///
/// [`PackageError::NamesAReservedName`], naming the clash.
pub fn not_a_reserved_name(package: &Package) -> Result<(), PackageError> {
    if *package.header().kind() != Kind::application() {
        return Ok(());
    }
    let name = package.header().product();
    if crate::service::is_reserved_application_name(name) {
        return Err(PackageError::NamesAReservedName {
            name: name.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
