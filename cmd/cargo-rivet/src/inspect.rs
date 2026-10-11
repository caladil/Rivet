//! `cargo rivet inspect` — verify a `.rvt` and read it back.
//!
//! # This is the round trip, and it is deliberately not the writer's mirror
//!
//! `AGENTS.md` §4: *"a test that round-trips a reader against its own writer
//! proves the two agree with each other and nothing about the format."* True,
//! and the answer here is not to skip the round trip but to make it go through
//! the thing that will do the job on a machine: `rivet_core::package::read`
//! is the **verifier**, the same code a board runs, and `rivet-crypto`'s
//! Ed25519 and BLAKE3 are the real algorithms rather than
//! `package::testing`'s fakes. What this closes is that an artifact this tool
//! writes is one the verifier accepts, which is a different claim from "the
//! writer and the reader in one crate agree".
//!
//! # It reads both kinds this project emits — and it did not, until 2026-08-24
//!
//! This function called `Application::open` **unconditionally**, so a perfectly
//! valid update payload was refused for not being an application — the verifier
//! had already accepted it and the *reporting* layer was the thing that could
//! not read it. `ADR-029` §11's argument is that the container is one and the
//! interpretation is per-kind; this was the one place that had the
//! interpretation hard-coded. It now dispatches on the header's `kind` and
//! refuses an unimplemented one **by name** — `ADR-029` §15.7.
//!
//! # What it does NOT do, and the omission is the whole state of play
//!
//! **It is not an installer and it does not run on a Rivet.** ~~`PLAN.md` §0.8
//! link 6 is the installer and is unwritten~~ — **false since later the same
//! day**: `cmd/rivet-deploy` is link 6 and has installed onto a loop device
//! (`ADR-029` §14.6). Link 7 — a machine being able to be *told about* a service
//! it was not compiled with — is deeper still and is unchanged.
//!
//! **The sentence that has not moved is the one that matters.** Nothing on a
//! board can verify one of these today, because **no root of trust ships in any
//! image** (`ADR-028` §5.2, unbuilt). So the trust set here is one key handed
//! over on the command line, which is the honest shape of a developer checking
//! their own artifact and is not a fleet.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rivet_core::package::Kind;
use rivet_core::package::application::{Application, Manifest, Network};
use rivet_core::package::format::HEADER_BYTES;
use rivet_core::package::port::MaySign;
use rivet_core::package::port::{Digest, PublicKey, TrustSet, Verify};
use rivet_core::package::read::{Member, Package, open};
use rivet_core::package::update::Payload;
use rivet_core::state::Slot;

const CHUNK: usize = 1024 * 1024;

pub struct Inspected {
    pub package: Package,
    /// `(index, name, class, variant, length)` for every member, all verified.
    pub members: Vec<(usize, String, u8, u8, u64)>,
    /// What the `<kind>` token made of the metadata — `ADR-029` §11's
    /// *"the **interpretation** of the metadata, not its shape"*.
    pub details: Details,
}

/// One variant per kind this tool can read back.
///
/// **There is no fallback arm**, and that is the point: a kind this build does
/// not implement is a refusal naming the token, never a package printed as a
/// bag of members. `ADR-029` §13.2 item 6 is the same posture in the reader —
/// *"a closed enum would make 'a kind I do not implement' indistinguishable
/// from a malformed field"*.
pub enum Details {
    Application {
        manifest: Manifest,
        program: (usize, String, u64),
        /// **Who this application will be on a machine** — `ADR-015` §5.1.1.
        ///
        /// Added 2026-08-29, Pile B item 2. `PLAN.md` §102.6's cold agent needed
        /// it, could not get it from this tool, and inferred it. Inferring it is
        /// not hard — the uid is a pure function of the name and the function is
        /// in `rivet-core` — but *"the board explained almost everything it was
        /// asked; the failures were on the workstation side"* is the finding,
        /// and a host tool that describes a package without saying who it runs
        /// as is the workstation side of exactly that.
        ///
        /// `None` when the header's `product` is not a name a machine could make
        /// a service of. That is not a hypothetical for a **reader**: every name
        /// in a package is chosen by whoever built it (`ADR-015` §7.1.1), the
        /// container's own text rules are looser than `ServiceName`'s, and the
        /// refusal happens at install rather than here. Reporting a number for a
        /// name no machine will accept would be worse than reporting none.
        identity: Option<Identity>,
    },
    /// **Shared Rivets, 2026-08-31.** A library artifact, read back.
    ///
    /// The field that matters is [`Self::Library::reference`], and it is why
    /// this variant exists rather than the members being listed: the reference
    /// is the thing an application declares, the directory the library installs
    /// to, and the identity — and anybody holding a `.rvt` should be able to ask
    /// it for one rather than keep the line `cargo rivet library` printed.
    Library {
        manifest: rivet_core::package::library::Manifest,
        /// `(index, name, length)` per shared object.
        objects: Vec<(usize, String, u64)>,
        /// `<name>-<version>-<digest>`, recomputed from the header that just
        /// verified — never read out of the file name.
        reference: String,
    },
    Update {
        /// The `/system` member's index — `Payload::system_member`'s answer,
        /// found by target class rather than by name.
        system: usize,
        /// The boot-half members that would be written to each slot, in file
        /// order. `Payload::boot_members_for`, which is what
        /// `rivet_adapter_image::install` calls on a machine.
        slot_a: Vec<String>,
        slot_b: Vec<String>,
    },
    /// **`R-177`, 2026-09-13.** A card image, wrapped as one opaque member.
    /// There is no manifest and nothing here interprets its bytes — the
    /// container's own [`Member`] fields are the whole of what this kind has
    /// to say, which is why there is no `rivet_core::package::card` module at
    /// all: everything reported below is already generic across every kind.
    Card {
        /// Bytes actually stored in the `.rvt` — the compressed size when
        /// `compression` is `COMPRESSION_ZSTD`, the same as `plain_len`
        /// otherwise.
        stored_len: u64,
        /// The image's own uncompressed size — what provisioning will write to
        /// a device once it is materialised back.
        plain_len: u64,
        /// `rivet_core::package::read::COMPRESSION_NONE` or `..._ZSTD`.
        compression: u8,
    },
    /// **Gantry, `ADR-061` §3.** An APK, read back — held by a fleet server
    /// for a phone, never installed on a Rivet machine.
    ///
    /// `metadata` is boxed: `Metadata` is the largest thing this enum would
    /// otherwise carry apart from `Application`'s manifest, and a small
    /// variant next to a 500-byte one is exactly what
    /// `clippy::large_enum_variant` is for.
    Apk {
        metadata: Box<rivet_core::package::apk::Metadata>,
        apk: (usize, String, u64),
        icon: Option<(usize, String, u64)>,
    },
}

/// **The account an application runs as, and the one directory it can write.**
/// `ADR-015` §5.1.1 and §3.2A.
///
/// Every field is **derived** rather than read out of the package: the uid is
/// `rivet_core::service::app_identity` of the name, and the data directory is
/// `rivet_core::service::data_dir` of the same name. Nothing here is a second
/// answer — the board computes these with the same two functions, which is what
/// makes reporting them here worth anything (`ADR-011`).
pub struct Identity {
    /// The name the machine will know it by — the header's `product`, once it
    /// has been through `ServiceName`.
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// `/state/app/<name>/data` — the ONLY path this application can write.
    ///
    /// It is here because of `PLAN.md` §102.6's *"single worst finding"*:
    /// `ADR-015` §3.2 stated the writable directory as `/state/app/<name>` and
    /// §5.1.1 — 400 lines later in the same document — had `/state/app/<name>/data`
    /// all along. An agent wrote code against the first, deployed it, and got
    /// `EACCES` on the board. The remedy that survives is not a corrected
    /// sentence but a **tool that prints the path**, computed by the same
    /// function the machine creates it with.
    pub data_dir: String,
    /// The mode that directory is created with — `0700`, so the number an
    /// operator sees here is the number `ls -l` will show.
    pub data_dir_mode: u32,
}

/// Verify the header, then **every member**, then read the kind's metadata.
///
/// `ADR-029` §9.2 step 6 is *"every member before any byte was written"*, and
/// the same order is right here for a different reason: a tool that printed a
/// manifest from a package whose binary had not been checked would be showing an
/// operator a permission request that does not belong to the bytes it names.
///
/// `reserved` is `ADR-029` §4's list, from the caller's `BootSlots` — an update
/// payload carrying one of those names is refused here exactly as it would be
/// on the machine, which is the whole reason this goes through `Payload::open`
/// rather than reading the member table itself.
pub fn inspect(
    path: &Path,
    trusted: PublicKey,
    reserved: &[&str],
    digest: &mut dyn Digest,
    verify: &dyn Verify,
) -> Result<Inspected, String> {
    let mut file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("cannot stat {}: {e}", path.display()))?
        .len();

    let mut head = vec![0u8; HEADER_BYTES];
    // `read_exact` rather than `read`: a short header is `ADR-029` F1 and must
    // be a refusal, not a header padded with whatever was in the buffer.
    match file.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(format!(
                "{} is {file_len} bytes and a package header is {HEADER_BYTES}; there is nothing \
                 to verify",
                path.display()
            ));
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    }

    // **Both kinds here, deliberately, and it is not a weakening.**
    //
    // `ADR-028` §5.5's entitlement is a rule about what a *machine* will
    // install. This is a host-side inspector whose job is to report what a file
    // is, and refusing to look at an artifact because of its kind would answer
    // a question nobody asked. The check that matters still happens where it
    // matters — on the Rivet, in `cmd/rivet-deploy`, which trusts a key for
    // `update` alone.
    let mut trust = TrustSet::new();
    trust.trust(
        digest,
        trusted,
        "inspected",
        // **Every kind this tool can read**, and `library` joined them on
        // 2026-08-31, `card` on 2026-09-13 (`R-177`). The entitlement here is
        // not a policy — a BOARD's is, and this one exists only so the
        // verifier has something to check against while reading a file the
        // caller already has. A kind missing from this list refuses with an
        // entitlement error, which reads as a fact about the artifact and is a
        // fact about this line.
        MaySign::these(&[
            Kind::update(),
            Kind::application(),
            Kind::library(),
            Kind::card(),
            Kind::apk(),
        ]),
    );

    let package =
        open(&head, file_len, digest, verify, &trust).map_err(|e| format!("refused: {e}"))?;

    // **Which kind this is decides which rules apply, and it is decided before
    // anything is interpreted.** Until 2026-08-24 this function called
    // `Application::open` unconditionally, so a perfectly valid update payload
    // was refused for not being an application — the verifier had already
    // accepted it and the *reporting* layer was the thing that could not read
    // it. `ADR-029` §11's whole argument is that the container is one and the
    // interpretation is per-kind; this is that, in the one place that had it
    // hard-coded.
    let kind = package.header().kind().to_string();

    // Every member, whatever the kind. Order matters for the same reason as on
    // the machine: nothing is reported from bytes whose digest has not matched.
    //
    // **A `card`'s one member is verified and never retained** — Wave B item
    // 5. It can be a whole 2 GiB `.img`, nothing here ever interprets its
    // bytes (`Details::Card` carries only lengths already in the header), and
    // `member_bytes` holding it resident would double the peak this process
    // needs for no reader. Every other kind is unchanged: its manifest (or, for
    // `update`, nothing at all) is what the branches below actually read back
    // out of `member_bytes`.
    let is_card = kind == Kind::card().as_str();
    let mut member_bytes: Vec<Vec<u8>> = Vec::new();
    let mut members = Vec::new();
    for member in package.members() {
        if is_card {
            verify_member_discarding(&mut file, &package, member, digest)
                .map_err(|e| format!("refused: {e}"))?;
        } else {
            let bytes = verify_member(&mut file, &package, member, digest)
                .map_err(|e| format!("refused: {e}"))?;
            member_bytes.push(bytes);
        }
        members.push((
            member.index(),
            member.name().to_string(),
            member.target_class(),
            member.variant(),
            member.stored_len(),
        ));
    }

    let details = if kind == Kind::application().as_str() {
        let application = Application::open(&package).map_err(|e| format!("refused: {e}"))?;
        let at = application.manifest_member().index();
        let manifest_bytes = member_bytes
            .get(at)
            .cloned()
            .ok_or("the manifest member is not in the file")?;
        // Re-verified rather than remembered: the token from the loop above is
        // not in scope here, and re-taking it costs a hash of bytes that are
        // already resident. `read_manifest` will not parse without it.
        let verified = package
            .verify_member(at, digest, &manifest_bytes)
            .map_err(|e| format!("refused: {e}"))?;
        let manifest = application
            .read_manifest(verified, &manifest_bytes)
            .map_err(|e| format!("refused: {e}"))?;
        Details::Application {
            manifest,
            program: (
                application.program_member().index(),
                application.program_member().name().to_string(),
                application.program_member().stored_len(),
            ),
            // **The name comes from the verified header**, exactly as it does in
            // `rivet_core::package::install::plan` — an installer has no
            // directory to observe and neither has a host tool, so the header's
            // `product` is the one authority for what this will be called
            // (`ADR-015` §7.1.1).
            identity: identity_of(package.header().product()),
        }
    } else if kind == Kind::library().as_str() {
        let library =
            rivet_core::package::library::Library::open(&package).map_err(|e| format!("refused: {e}"))?;
        let at = library.manifest_member().index();
        let manifest_bytes = member_bytes
            .get(at)
            .cloned()
            .ok_or("the manifest member is not in the file")?;
        let manifest = rivet_core::package::library::Manifest::parse(&manifest_bytes)
            .map_err(|fault| format!("refused: the library manifest cannot be read: {fault}"))?;
        // Recomputed from the head that verified, exactly as the board does.
        // Reading it off the file name would be this tool trusting a string
        // somebody could rename (`ADR-027` §5).
        let identity = rivet_core::package::library::identity(&head, digest)
            .ok_or("the header is too short to have an identity")?;
        let reference = rivet_core::package::library::Reference::compose(
            package.header().product(),
            package.header().version(),
            &identity,
        )
        .map_err(|fault| format!("this artifact cannot be referenced: {fault}"))?;
        Details::Library {
            manifest,
            objects: library
                .objects()
                .iter()
                .map(|m| (m.index(), m.name().to_string(), m.stored_len()))
                .collect(),
            reference: reference.to_string(),
        }
    } else if kind == Kind::update().as_str() {
        let payload = Payload::open(&package, reserved).map_err(|e| format!("refused: {e}"))?;
        Details::Update {
            system: payload.system_member().index(),
            slot_a: payload
                .boot_members_for(Slot::A)
                .map(|m| m.name().to_string())
                .collect(),
            slot_b: payload
                .boot_members_for(Slot::B)
                .map(|m| m.name().to_string())
                .collect(),
        }
    } else if kind == Kind::card().as_str() {
        // **One opaque member, and nothing to open.** Unlike the three kinds
        // above, `card` has no manifest and no per-kind reader — the whole of
        // what there is to report is already in the verified header and the
        // member table, generic across every kind (`rivet_core::package::read`).
        let member = package
            .members()
            .first()
            .ok_or("a card artifact with no member — nothing to write to a device")?;
        Details::Card {
            stored_len: member.stored_len(),
            plain_len: member.plain_len(),
            compression: member.compression(),
        }
    } else if kind == Kind::apk().as_str() {
        let apk = rivet_core::package::apk::Apk::open(&package).map_err(|e| format!("refused: {e}"))?;
        let at = apk.metadata_member().index();
        let metadata_bytes = member_bytes
            .get(at)
            .cloned()
            .ok_or("the apk.json member is not in the file")?;
        let verified = package
            .verify_member(at, digest, &metadata_bytes)
            .map_err(|e| format!("refused: {e}"))?;
        let metadata = apk
            .read_metadata(verified, &metadata_bytes)
            .map_err(|e| format!("refused: {e}"))?;
        Details::Apk {
            metadata: Box::new(metadata),
            apk: (
                apk.apk_member().index(),
                apk.apk_member().name().to_string(),
                apk.apk_member().stored_len(),
            ),
            icon: apk
                .icon_member()
                .map(|m| (m.index(), m.name().to_string(), m.stored_len())),
        }
    } else {
        return Err(format!(
            "refused: this artifact's <kind> is {kind:?}, and this build implements \
             {:?}, {:?}, {:?}, {:?} and {:?}. ADR-016's extension is unruled (ADR-027 §9). \
             The container verified; nothing here knows what its metadata MEANS.",
            Kind::update().as_str(),
            Kind::application().as_str(),
            Kind::library().as_str(),
            Kind::card().as_str(),
            Kind::apk().as_str(),
        ));
    };

    Ok(Inspected {
        package,
        members,
        details,
    })
}

/// The identity a package's `product` will have on a machine, or `None` if that
/// product is not a name a machine will accept.
///
/// **`BUILT_IN_VOLUME`, and it is stated rather than hidden.** `ADR-031` §8 item
/// 1 made the application root a function of a volume root and §11 leaves which
/// volume an install lands on unruled, so `cmd/rivet-deploy` passes the built-in
/// one at its only call site. A host tool reporting a path must report the one
/// that will actually be created, so it passes the same thing — and when that
/// stops being the only answer, this is one of the places that has to learn it.
fn identity_of(product: &str) -> Option<Identity> {
    let name = rivet_core::service::ServiceName::new(product)?;
    let id = rivet_core::service::app_identity(&name);
    Some(Identity {
        data_dir: rivet_core::service::data_dir(rivet_core::service::BUILT_IN_VOLUME, &name),
        data_dir_mode: rivet_core::service::DATA_DIR_MODE,
        name: name.as_str().to_string(),
        uid: id.uid,
        gid: id.gid,
    })
}

/// Read one member out of the file and hand it back only if its digest matched.
fn verify_member(
    file: &mut File,
    package: &Package,
    member: &Member,
    digest: &mut dyn Digest,
) -> Result<Vec<u8>, rivet_core::package::PackageError> {
    file.seek(SeekFrom::Start(member.offset())).map_err(|_| {
        rivet_core::package::PackageError::MemberOverflowsFile {
            index: member.index(),
            offset: member.offset(),
            stored_len: member.stored_len(),
            file_len: package.file_len(),
        }
    })?;

    let mut stream = package.member_stream(member.index(), digest)?;
    let mut kept = Vec::new();
    let mut left = member.stored_len();
    let mut buffer = vec![0u8; CHUNK];
    while left > 0 {
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let read = file.read(&mut buffer[..want]).unwrap_or(0);
        if read == 0 {
            // The header says these bytes exist and the file ended. `finish`
            // below is what turns that into `MemberShort`, which names both
            // numbers; breaking here rather than erroring keeps one refusal for
            // one fault.
            break;
        }
        stream.write(&buffer[..read])?;
        kept.extend_from_slice(&buffer[..read]);
        left -= read as u64;
    }
    stream.finish()?;
    Ok(kept)
}

/// Like [`verify_member`], but never accumulates the member's bytes.
///
/// Used for a `card`'s one member, which nothing here interprets — Wave B
/// item 5. It streams the same `CHUNK`-sized buffer [`verify_member`] does, so
/// this function's own peak is one chunk rather than the whole member; the
/// difference from [`verify_member`] is only that the chunk is never appended
/// anywhere once its bytes have been fed to the digest.
fn verify_member_discarding(
    file: &mut File,
    package: &Package,
    member: &Member,
    digest: &mut dyn Digest,
) -> Result<(), rivet_core::package::PackageError> {
    file.seek(SeekFrom::Start(member.offset())).map_err(|_| {
        rivet_core::package::PackageError::MemberOverflowsFile {
            index: member.index(),
            offset: member.offset(),
            stored_len: member.stored_len(),
            file_len: package.file_len(),
        }
    })?;

    let mut stream = package.member_stream(member.index(), digest)?;
    let mut left = member.stored_len();
    let mut buffer = vec![0u8; CHUNK];
    while left > 0 {
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let read = file.read(&mut buffer[..want]).unwrap_or(0);
        if read == 0 {
            break;
        }
        stream.write(&buffer[..read])?;
        left -= read as u64;
    }
    stream.finish()?;
    Ok(())
}

/// Extract one verified member to a path.
pub fn extract(
    path: &Path,
    package: &Package,
    index: usize,
    digest: &mut dyn Digest,
    to: &Path,
) -> Result<u64, String> {
    let mut file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let member = package
        .members()
        .get(index)
        .ok_or_else(|| format!("there is no member {index}"))?;
    let bytes =
        verify_member(&mut file, package, member, digest).map_err(|e| format!("refused: {e}"))?;
    if let Some(parent) = to.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(to, &bytes).map_err(|e| format!("cannot write {}: {e}", to.display()))?;
    Ok(bytes.len() as u64)
}

/// `ADR-015` §5.4's plan, as far as anything can honestly render it today.
pub fn requests(manifest: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    // **Where it binds, from the function `rivet-sysd` binds with** — `ADR-046`
    // §7. A proxied application's ordinary port says `127.0.0.1`, which is the
    // one fact about it an author reading this is most likely to have missed.
    for socket in manifest.listen() {
        out.push(match socket.bind_address(manifest.proxied()) {
            Some(address) => format!("bind {} on {address}", socket.render()),
            None => format!("bind {}", socket.render()),
        });
    }
    match manifest.network() {
        Network::None => {}
        Network::Egress => out.push("network egress".to_string()),
        Network::Group(group) => out.push(format!("network group:{group}")),
    }
    for endpoint in manifest.provides() {
        out.push(format!("provide {}:{}", endpoint.owner(), endpoint.name()));
    }
    for endpoint in manifest.connects() {
        out.push(format!("connect {}:{}", endpoint.owner(), endpoint.name()));
    }
    if let Some(memory) = manifest.memory() {
        out.push(format!("memory {}", memory.as_str()));
    }
    out
}
