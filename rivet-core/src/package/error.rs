//! Every way a `.rvt` can be refused.
//!
//! # Why this is not a variant of [`crate::CoreError`]
//!
//! `CoreError` documents itself as *"every way an operation in this crate can
//! fail"* and its variants are an operator's mistakes — a bad interface name, a
//! gateway off-link. These are a *file's* faults, most of them reached while
//! parsing bytes somebody else wrote, and `ADR-006` §6.4's closed-vocabulary
//! rule is the argument for keeping them apart rather than for merging them:
//! folding twenty-odd parser refusals into the enum an operator's errors live in
//! would leave neither set meaning anything.
//!
//! # Every variant carries what was found
//!
//! `AGENTS.md` §5 — *report what the system said*. A verifier that says "bad
//! payload" sends its reader to a hex dump; one that says which member, at which
//! offset, claiming which length, does not. The one place a value is
//! deliberately withheld is that no error here carries member **content**.

use crate::package::format::TextFault;
use crate::package::port::{DigestAlgorithm, KeyId, SignatureAlgorithm};

/// A `.rvt` this build will not read, or will not write.
///
/// **Closed and exhaustive on purpose**, unlike [`crate::CoreError`]: a caller
/// installing an update should be made to think about a new refusal rather than
/// have it fall into a catch-all arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    // --- ADR-029 §5 steps 1-7, in order. Everything here is decided before a
    //     single byte of the manifest is believed. -----------------------------
    /// Step 1. The file is shorter than the fixed header.
    TooShortForHeader {
        file_len: u64,
    },
    /// The caller handed over fewer than `HEADER_BYTES` to read from. Not an
    /// attack — a caller's bug, named as one so it is not mistaken for a
    /// truncated payload.
    ShortHeaderBuffer {
        given: usize,
    },
    /// Step 2. Byte 0..8 is not [`crate::package::format::MAGIC`].
    NotAPackage {
        found: [u8; 8],
    },
    /// Step 3.
    UnsupportedFormatVersion {
        found: u32,
    },
    /// Step 4. `header_bytes` is checked against the constant and is never used
    /// to size a read, so this is a refusal rather than a smaller parse.
    HeaderBytesWrong {
        found: u32,
    },
    /// Step 5.
    UnimplementedSignatureAlgorithm {
        found: SignatureAlgorithm,
        implemented: SignatureAlgorithm,
    },
    /// Step 5.
    UnimplementedDigestAlgorithm {
        found: DigestAlgorithm,
        implemented: DigestAlgorithm,
    },
    /// Step 6. The header names a key this machine does not trust.
    /// The signature is **valid** and the key is **trusted**, and the artifact
    /// is refused anyway: this key may not sign this kind.
    ///
    /// `ADR-028` §5.5, ruled 2026-08-24. Signing and being entitled to sign are
    /// different questions (§4), and this is the one that separates a key held
    /// for Rivet's own images from a key held for the applications somebody
    /// writes. Without it, the machine that signs a hobby application can ship
    /// an operating system to every board in the fleet.
    ///
    /// The `label` is carried so the refusal can name the key in the terms its
    /// holder knows it by. It decides nothing.
    KindNotPermitted {
        key_id: KeyId,
        label: String,
        kind: String,
        may_sign: Vec<String>,
    },

    /// The package is not an application, and an application is what an install
    /// plan is for. `crate::package::install`.
    ///
    /// Reached only by a caller that asked the wrong thing of a valid package —
    /// a system update handed to the application installer, or the reverse — so
    /// the message names both what it is and what was wanted rather than
    /// implying the artifact is bad.
    WrongKindForInstall {
        kind: String,
        wanted: String,
    },

    /// The header's `product` is not a usable application name.
    ///
    /// A name is a directory, a cgroup and a socket (`ADR-017` §3.1.3), so
    /// `rivet_core::service::ServiceName` is stricter than a filesystem is —
    /// and that strictness is what stops a header field becoming a path.
    UnusableName {
        product: String,
    },

    /// The header's `product` names a service `cmd/rivet-image` already bakes
    /// into every card, whatever add-ons it carries.
    ///
    /// `PLAN-Feature-Set-2026-09-12` §3, `.rulings/` R-156 R10: an
    /// installed application under one of [`crate::service::IMAGE_SERVICE_NAMES`]
    /// is refused every boot by `cmd/rivet-sysd`'s own discovery and never
    /// started — a shadow that, once written, `Supervisor::begin_uninstall`
    /// cannot name to remove, because it finds the image's service of that name
    /// first. This is the door on the way in, checked at the same point a wrong
    /// architecture is (`crate::package::verify::architecture`), so the shadow
    /// is never created a second time.
    NamesAnImageService {
        name: String,
    },

    /// The header's `product` is a name this project's own machinery already
    /// means something else by — `.rulings/` R-210 D21.
    ///
    /// Wider than [`Self::NamesAnImageService`], which is about a service the
    /// image runs: this is an account or group in the image's own
    /// `/etc` tables (`crate::etc::SYSTEM_IDENTITIES`,
    /// `crate::etc::SYSTEM_GROUPS`), a word the uninstall grammar reads as an
    /// instruction rather than a name, or the deploy, supervisor, kernel-log
    /// or ingest service name. Every one of those is a name `getpwnam`, the
    /// uninstall parser, or a journal reader already resolves to something
    /// else, so an application under it would be indistinguishable from that
    /// other thing rather than merely refused every boot.
    NamesAReservedName {
        name: String,
    },

    /// The header's `version` cannot be a directory name under the application's
    /// root.
    ///
    /// Either it is not one safe path component, or it is one of the names that
    /// already mean something there — `current`, `data` — which a version
    /// directory would replace rather than sit beside.
    UnusableVersion {
        version: String,
    },

    /// A member's name would put the file somewhere other than the version
    /// directory.
    ///
    /// **Every member name is attacker-chosen**, and this is the refusal that
    /// says so. It fires *after* the signature verified, which is the point
    /// rather than a redundancy: a signed artifact is one somebody vouched for,
    /// not one that is well-formed.
    UnsafeMemberName {
        name: String,
    },

    /// An asset's name equals a name the installer writes into the same
    /// directory on its own — `crate::package::install::
    /// RESERVED_APPLICATION_ASSET_NAMES` for an application,
    /// `crate::package::install::RESERVED_LIBRARY_ASSET_NAMES` for a library.
    ///
    /// **Different from [`Self::UnsafeMemberName`]**, which is about a name
    /// that already means something to this module's own directory layout —
    /// `current`, `data`, a traversal shape. This one is a perfectly safe
    /// component that collides only because the installer happens to write a
    /// file of exactly that name beside it: the retained header, the
    /// manifest, the recorded size, or the directory a program or every
    /// object lands under.
    ReservedAssetName {
        name: String,
    },

    UntrustedKey {
        key_id: KeyId,
    },
    /// Step 7. The bytes are not the ones that key signed.
    ///
    /// **`ADR-029` F2: nothing has been written, and nothing will be.**
    BadSignature {
        key_id: KeyId,
    },

    // --- The manifest. Authenticated by the time any of this can happen, so a
    //     refusal here means a *builder* produced something incoherent. ---------
    /// More members than [`crate::package::format::MAX_MEMBERS`].
    TooManyMembers {
        found: u32,
    },
    /// `flags` carries a bit this build does not define. No flag is defined at
    /// `format_version` 1, so any non-zero value is refused.
    UnknownFlags {
        flags: u32,
    },
    /// A reserved byte that is not zero.
    ReservedBytesNotZero {
        field: &'static str,
    },
    /// One of the fixed-width text fields.
    MalformedText {
        field: &'static str,
        fault: TextFault,
    },
    /// A field that may not be empty, and is.
    MissingText {
        field: &'static str,
    },
    /// A member name.
    MalformedMemberName {
        index: usize,
        fault: TextFault,
    },
    /// Two members share a name **and** a variant byte. Sharing a name alone is
    /// legal — `ADR-029` §3's `cmdline.txt` appears twice, once per slot.
    DuplicateMember {
        index: usize,
        name: String,
        variant: u8,
    },
    /// A member does not start where the previous one ended.
    ///
    /// The members are contiguous from `HEADER_BYTES` on, which is what leaves
    /// no gap in the file that no digest covers.
    MemberNotContiguous {
        index: usize,
        expected_offset: u64,
        found_offset: u64,
    },
    /// A member claims a length the file does not have — `ADR-029` F1.
    MemberOverflowsFile {
        index: usize,
        offset: u64,
        stored_len: u64,
        file_len: u64,
    },
    /// The file is longer than the last member's end. Refused rather than
    /// ignored: bytes no digest covers have no business being in a signed
    /// artifact.
    TrailingBytes {
        members_end: u64,
        file_len: u64,
    },
    /// `compression` names a codec this build does not implement at all.
    /// `R-177.1` (2026-09-13) ruled the one codec this format has —
    /// [`crate::package::read::COMPRESSION_ZSTD`] — so this build implements
    /// exactly two values, `0` (stored) and `1` (zstd), and refuses every
    /// other rather than guessing which one was meant.
    UnsupportedCompression {
        index: usize,
        found: u8,
    },
    /// The codec is one this build implements, but not for this payload's
    /// `<kind>`. **`R-177.1`: only a `card` payload's member may compress.**
    /// `ADR-029` §13/R-107 keeps every kind a board installs uncompressed on
    /// purpose — a decompressor would then be a permanent dependency on every
    /// device that boots it — and a card image is the one kind no board ever
    /// installs, so it alone may spend the trade the other way.
    CompressionNotPermittedForKind {
        index: usize,
        kind: String,
    },
    /// `compression = none` and the stored and plain lengths disagree.
    LengthsDisagree {
        index: usize,
        stored_len: u64,
        plain_len: u64,
    },

    // --- Member content. ADR-029 §5 steps 8-9. -------------------------------
    /// More bytes were offered for a member than its record says it has.
    MemberOverrun {
        index: usize,
        stored_len: u64,
        offered: u64,
    },
    /// Fewer bytes arrived than the record says.
    MemberShort {
        index: usize,
        stored_len: u64,
        seen: u64,
    },
    /// The bytes are not the ones the signed header names.
    ///
    /// **`ADR-029` F3: whatever has already been written is left inert, not
    /// wiped.**
    MemberDigestMismatch {
        index: usize,
        name: String,
    },
    /// There is no member with that index.
    NoSuchMember {
        index: usize,
        member_count: usize,
    },

    // --- Interpretation of a kind's metadata. --------------------------------
    /// The payload is not the kind the caller asked for.
    WrongKind {
        expected: String,
        found: String,
    },
    /// `ADR-029` §4. **The whole payload is refused, not the member** — silently
    /// skipping would let a future builder believe it had shipped something it
    /// had not.
    ReservedMemberName {
        index: usize,
        name: String,
    },
    /// `target_class` is not a value this kind defines.
    UnknownTargetClass {
        index: usize,
        found: u8,
    },
    /// The variant byte is not a value this kind defines.
    UnknownVariant {
        index: usize,
        found: u8,
    },
    /// An `update` with nothing in it. A payload that writes nothing and reports
    /// success is the silent-success direction `ADR-018` §5.1's slot-B commit bug
    /// pointed in.
    NothingToInstall,
    /// An `update` must carry exactly one whole-partition member — the `/system`
    /// filesystem. Zero would leave the slot's system untouched while reporting
    /// an install; two would make it ambiguous which one the slot becomes.
    SystemMemberCount {
        found: usize,
    },
    /// A member is slot-variant in one direction only, so installing to the
    /// other slot would silently omit a file the payload was built to carry.
    SlotVarianceIncomplete {
        name: String,
        missing: &'static str,
    },
    /// A whole-partition member that claims to differ between slots. `/system`
    /// is byte-identical in both (`ADR-029` §3, measured), and a payload saying
    /// otherwise is one this build does not understand.
    SystemMemberIsSlotVariant {
        index: usize,
        variant: u8,
    },
    /// Two boot-directory members reach one boot filesystem under the same
    /// name, once ASCII case is folded — the comparison the target's FAT
    /// filesystem actually makes (`ADR-029` §4). Checked per slot, so an
    /// `Invariant` member colliding with a slot-variant one of the same name
    /// is this, with no separate variant of its own.
    BootMemberNameCollides {
        index: usize,
        name: String,
        other_index: usize,
        other_name: String,
    },
    /// A boot-directory member's name matches another member's install-time
    /// staging name, once ASCII case is folded. The second member's write
    /// would land on the first member's already-installed file.
    BootMemberNameCollidesWithTemporary {
        index: usize,
        name: String,
        other_index: usize,
        other_name: String,
    },

    // --- The `application` kind. The maintainer's 2026-08-24 ruling, `ADR-027` §9. ----
    /// The header carries a `platform` and this kind does not have one.
    ///
    /// Ruled 2026-08-24: an application is architecture-shaped rather than
    /// firmware-shaped, so it carries no `<platform>` at all. Refused rather
    /// than ignored — a builder that stamped one had a different idea of the
    /// format, and the field is inside the signature, so accepting it would
    /// make the ruling advisory.
    UnexpectedPlatform {
        found: String,
    },
    /// An application must carry exactly one manifest. Zero is an artifact whose
    /// permission request is missing (`ADR-015` §3.3); two is two requests with
    /// no rule for choosing.
    ManifestMemberCount {
        found: usize,
    },
    /// The manifest is present and is not member 0. An installer reads what an
    /// application *is* before it streams a binary that may be megabytes.
    ManifestNotFirst {
        index: usize,
    },
    /// The manifest member is not named
    /// [`crate::package::application::MANIFEST_NAME`].
    ManifestMisnamed {
        name: String,
    },
    /// An application must carry exactly one program. `ADR-015` §3: *"a package
    /// is **one** static binary"*, and there is no `exec` field to disambiguate
    /// two.
    ProgramMemberCount {
        found: usize,
    },
    /// A [`crate::package::read::VerifiedMember`] for one member was offered
    /// where another member's was wanted. A caller's bug, named as one — the
    /// token proves *some* member's bytes were checked, and this is the check
    /// that it was the right one.
    WrongMemberVerified {
        expected: usize,
        found: usize,
    },
    /// The manifest's bytes are not a manifest this build reads.
    MalformedManifest {
        fault: crate::package::application::ManifestFault,
    },

    // --- Shared Rivets. `crate::package::library`, 2026-08-31. ---------------
    /// A `library` artifact carrying no object, or more than
    /// [`crate::package::library::MAX_OBJECTS`] of them.
    ///
    /// A separate variant from [`Self::ProgramMemberCount`] rather than a reuse
    /// of it: an application carries **exactly one** program and a library
    /// carries **one or more** objects, so a shared variant would have to say
    /// "wrong number" without being able to say what the right number is.
    ObjectMemberCount {
        found: usize,
    },
    /// A library manifest's bytes are not a library manifest this build reads.
    MalformedLibraryManifest {
        fault: crate::package::library::ManifestFault,
    },
    /// An artifact declares a closure this machine cannot satisfy.
    ///
    /// **Refused at install, never at start** — the design's own rule
    /// (`Shared-Libraries-and-the-Build-Target.md` §4.4): *"the installer
    /// refuses an application whose closure is not satisfied, naming what is
    /// missing — refusal at install, never a loader error at start."*
    ///
    /// It carries the missing references rather than a count, because the whole
    /// value of refusing here instead of there is that the operator is told
    /// **which** library to install (`AGENTS.md` §5).
    ClosureNotSatisfied {
        missing: Vec<String>,
    },

    // --- The `apk` kind. `ADR-061` §3. ---------------------------------------
    /// A member whose name is none of `app.apk`, `apk.json` or `icon.png` —
    /// `ADR-061` §3 refuses everything else, by name.
    ApkForeignMember {
        name: String,
    },
    /// A required or singular member of an `apk` artifact does not appear
    /// exactly the number of times it must: `app.apk` and `apk.json` exactly
    /// once each, `icon.png` at most once.
    ApkMemberCount {
        name: &'static str,
        found: usize,
    },
    /// The header's `platform` is not `"android"` — `ADR-061` §3.
    ApkWrongPlatform {
        found: Option<String>,
    },
    /// The header's `arch` is not `"any"` — `ADR-061` §3.
    ApkWrongArch {
        found: String,
    },
    /// `apk.json`'s bytes are not the metadata this build reads — `ADR-061`
    /// §3.2.
    MalformedApkMetadata {
        fault: crate::package::apk::MetadataFault,
    },

    // --- The writer. ---------------------------------------------------------
    /// A text field is longer than its fixed width.
    TextTooLong {
        field: &'static str,
        len: usize,
        capacity: usize,
    },
    /// The total of the header and every member overflows a `u64`.
    LengthOverflow,
    /// The bytes handed to an in-memory write do not match the records.
    MemberBytesDisagree {
        index: usize,
    },
}

impl core::fmt::Display for PackageError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShortForHeader { file_len } => write!(
                f,
                "this is {file_len} bytes and a package header is {}; there is nothing to verify",
                crate::package::format::HEADER_BYTES
            ),
            Self::ShortHeaderBuffer { given } => write!(
                f,
                "the caller offered {given} bytes of header, which is fewer than {} — a bug in the \
                 caller rather than a fault in the package",
                crate::package::format::HEADER_BYTES
            ),
            Self::NotAPackage { found } => {
                write!(f, "this does not begin with the package magic; it begins ")?;
                for byte in found {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
            Self::UnsupportedFormatVersion { found } => write!(
                f,
                "this package says format version {found} and this build implements {}",
                crate::package::format::FORMAT_VERSION
            ),
            Self::HeaderBytesWrong { found } => write!(
                f,
                "the header declares itself {found} bytes and this format's header is {} — the \
                 field is checked, never obeyed (ADR-029 §5 step 4)",
                crate::package::format::HEADER_BYTES
            ),
            Self::UnimplementedSignatureAlgorithm { found, implemented } => write!(
                f,
                "this package is signed with {found} and this build verifies {implemented}"
            ),
            Self::UnimplementedDigestAlgorithm { found, implemented } => write!(
                f,
                "this package is digested with {found} and this build computes {implemented}"
            ),
            Self::KindNotPermitted {
                key_id,
                label,
                kind,
                may_sign,
            } => {
                let permitted = if may_sign.is_empty() {
                    "nothing at all".to_string()
                } else {
                    may_sign.join(", ")
                };
                write!(
                    f,
                    "REFUSED this payload is a `{kind}` signed by the key labelled `{label}` ({key_id}), and that key is trusted to sign {permitted}. The signature is valid and the key is one this machine trusts — what it is not is entitled to sign this. ADR-028 §5.5: Rivet's own images and the applications you write are signed by different keys ON PURPOSE, so that a machine holding an application key cannot ship an operating system. If this is your artifact, sign it with the key that may sign a `{kind}`; if it is not, somebody has offered you one thing dressed as another."
                )
            }

            Self::WrongKindForInstall { kind, wanted } => write!(
                f,
                "REFUSED this package is a `{kind}` and installing one needs a `{wanted}`. Both are valid artifacts in the same container (ADR-029 §11) and they are installed by different code, because a `{wanted}` becomes files under an application's root and an update becomes 640 MiB written onto a slot."
            ),
            Self::UnusableName { product } => write!(
                f,
                "REFUSED the header names this application {product:?}, which cannot be an application name. A name is a directory, a cgroup and a socket (ADR-017 §3.1.3), so it must begin with a letter or a digit and hold only letters, digits, `-`, `_` and `.`, and be at most {} characters. The container's own header is looser than that on purpose — it carries text, and what a name may be is this system's question rather than the format's. Rebuild the package with a name that is one.",
                crate::service::ServiceName::MAX_LEN
            ),
            Self::NamesAnImageService { name } => write!(
                f,
                "REFUSED the header names this application {name:?}, and this machine's image \
                 already defines a service under that name. One name cannot have two definitions \
                 (ADR-011): an installed application here would be refused every boot and never \
                 started, so it is refused now instead, before anything is written. Rebuild the \
                 package under a different name."
            ),
            Self::NamesAReservedName { name } => write!(
                f,
                "REFUSED the header names this application {name:?}, and this project's own \
                 machinery already means something else by that name — an account or group this \
                 image defines, a word the uninstall grammar reads as an instruction, or the \
                 deploy, supervisor, kernel-log or ingest service name. One name cannot have two \
                 meanings (ADR-011). Rebuild the package under a different name."
            ),
            Self::UnusableVersion { version } => write!(
                f,
                "REFUSED the header gives the version as {version:?}, which cannot be a directory name under this application's root. It must be one path component, and it may not be `current`, `data` or `.current.new` — those already exist there, and a version directory would replace the link that selects a version, or the directory holding everything this application has ever written."
            ),
            Self::UnsafeMemberName { name } => write!(
                f,
                "REFUSED this package contains a member named {name:?}, which already means something under an application's root and so cannot also be a file installed into it. The signature is valid and the key is trusted; what the package is not is well-formed. Traversal shapes — a `/`, a `..` — never reach here: the container refuses those when the package is written."
            ),
            Self::ReservedAssetName { name } => write!(
                f,
                "REFUSED this package offers an asset named {name:?}, and the installer writes a file of exactly that name into the same directory on its own. The asset would not sit beside the file it is named for — it would land on top of it. Rename the asset and rebuild the package."
            ),

            Self::UntrustedKey { key_id } => write!(
                f,
                "this package is signed by key {key_id}, which is not in this machine's trust set"
            ),
            Self::BadSignature { key_id } => write!(
                f,
                "the signature by key {key_id} does not cover these bytes; nothing has been written"
            ),
            Self::TooManyMembers { found } => write!(
                f,
                "the header names {found} members and the format allows {}",
                crate::package::format::MAX_MEMBERS
            ),
            Self::UnknownFlags { flags } => {
                write!(
                    f,
                    "the header carries flags 0x{flags:08x} and this format version defines none"
                )
            }
            Self::ReservedBytesNotZero { field } => {
                write!(f, "the reserved bytes at {field} are not zero")
            }
            Self::MalformedText { field, fault } => {
                write!(f, "the {field} field is unusable: {fault}")
            }
            Self::MissingText { field } => write!(f, "the {field} field is empty and may not be"),
            Self::MalformedMemberName { index, fault } => {
                write!(f, "member {index} has an unusable name: {fault}")
            }
            Self::DuplicateMember {
                index,
                name,
                variant,
            } => write!(
                f,
                "member {index} repeats the name {name:?} with variant {variant}, which an earlier \
                 member already used"
            ),
            Self::MemberNotContiguous {
                index,
                expected_offset,
                found_offset,
            } => write!(
                f,
                "member {index} starts at {found_offset} and the previous member ends at \
                 {expected_offset}; members are contiguous so that no byte of the file is outside \
                 every digest"
            ),
            Self::MemberOverflowsFile {
                index,
                offset,
                stored_len,
                file_len,
            } => write!(
                f,
                "member {index} claims {stored_len} bytes at {offset} and the file is {file_len} \
                 bytes; it is truncated"
            ),
            Self::TrailingBytes {
                members_end,
                file_len,
            } => write!(
                f,
                "the last member ends at {members_end} and the file is {file_len} bytes; the \
                 remainder is covered by no digest"
            ),
            Self::UnsupportedCompression { index, found } => write!(
                f,
                "member {index} is compressed with codec {found}; this build implements codec 0 \
                 (stored) and codec 1 (zstd, R-177.1) and refuses any other rather than guessing \
                 which one was meant"
            ),
            Self::CompressionNotPermittedForKind { index, kind } => write!(
                f,
                "member {index} is compressed and this is a {kind:?} payload; only a `card` \
                 payload's member may be compressed (ADR-029 §13/R-107, R-177.1) because a card \
                 image is the one kind no board ever installs — every kind a board installs stays \
                 uncompressed on purpose, so a decompressor never becomes a permanent dependency \
                 on the device"
            ),
            Self::LengthsDisagree {
                index,
                stored_len,
                plain_len,
            } => write!(
                f,
                "member {index} is stored uncompressed, is {stored_len} bytes stored and claims \
                 {plain_len} bytes plain"
            ),
            Self::MemberOverrun {
                index,
                stored_len,
                offered,
            } => write!(
                f,
                "member {index} is {stored_len} bytes and {offered} were offered"
            ),
            Self::MemberShort {
                index,
                stored_len,
                seen,
            } => write!(
                f,
                "member {index} is {stored_len} bytes and only {seen} arrived"
            ),
            Self::MemberDigestMismatch { index, name } => write!(
                f,
                "member {index} ({name:?}) does not match the digest the signed header holds for it"
            ),
            Self::NoSuchMember {
                index,
                member_count,
            } => {
                write!(
                    f,
                    "there is no member {index}; this package has {member_count}"
                )
            }
            Self::WrongKind { expected, found } => {
                write!(
                    f,
                    "this is a {found:?} package and a {expected:?} one was expected"
                )
            }
            Self::ReservedMemberName { index, name } => write!(
                f,
                "member {index} is named {name:?}, which this platform reserves on that target; \
                 the whole payload is refused rather than the member skipped (ADR-029 §4)"
            ),
            Self::UnknownTargetClass { index, found } => {
                write!(
                    f,
                    "member {index} has target class {found}, which this kind does not define"
                )
            }
            Self::UnknownVariant { index, found } => {
                write!(
                    f,
                    "member {index} has variant {found}, which this kind does not define"
                )
            }
            Self::NothingToInstall => {
                write!(
                    f,
                    "this update payload has no members, so installing it would change nothing"
                )
            }
            Self::SystemMemberCount { found } => write!(
                f,
                "an update payload carries exactly one whole-partition member and this one carries \
                 {found}"
            ),
            Self::SlotVarianceIncomplete { name, missing } => write!(
                f,
                "{name:?} is carried for one slot only; the {missing} copy is absent, so installing \
                 to that slot would silently omit it"
            ),
            Self::SystemMemberIsSlotVariant { index, variant } => write!(
                f,
                "member {index} is a whole partition and claims variant {variant}; the /system \
                 filesystem is identical in both slots"
            ),
            Self::BootMemberNameCollides {
                index,
                name,
                other_index,
                other_name,
            } => write!(
                f,
                "member {index} ({name:?}) and member {other_index} ({other_name:?}) reach one \
                 boot filesystem under the same name once ASCII case is folded, which is how \
                 the target compares them (ADR-029 §4)"
            ),
            Self::BootMemberNameCollidesWithTemporary {
                index,
                name,
                other_index,
                other_name,
            } => write!(
                f,
                "member {index} ({name:?}) is named the same, once ASCII case is folded, as the \
                 staging file member {other_index} ({other_name:?}) installs through; the \
                 second write would land on the first member's file"
            ),
            Self::UnexpectedPlatform { found } => write!(
                f,
                "this application names platform {found:?} and an application carries none; it is \
                 architecture-shaped rather than firmware-shaped (ADR-027 §9 item 3)"
            ),
            Self::ManifestMemberCount { found } => write!(
                f,
                "an application carries exactly one manifest and this one carries {found}"
            ),
            Self::ManifestNotFirst { index } => write!(
                f,
                "the manifest is member {index} and must be member 0, so that what an application \
                 is can be read before its binary is streamed"
            ),
            Self::ManifestMisnamed { name } => write!(
                f,
                "the manifest member is named {name:?} and must be named {:?}",
                crate::package::application::MANIFEST_NAME
            ),
            Self::ProgramMemberCount { found } => write!(
                f,
                "an application is one static binary and this one carries {found}"
            ),
            Self::WrongMemberVerified { expected, found } => write!(
                f,
                "member {found} was verified and member {expected} was the one being read"
            ),
            Self::MalformedManifest { fault } => {
                write!(f, "the manifest cannot be read: {fault}")
            }
            Self::ObjectMemberCount { found } => write!(
                f,
                "a library carries one or more shared objects and this one carries {found}"
            ),
            Self::MalformedLibraryManifest { fault } => {
                write!(f, "the library manifest cannot be read: {fault}")
            }
            Self::ClosureNotSatisfied { missing } => write!(
                f,
                "this machine does not hold {} of the libraries this artifact declares: {}",
                missing.len(),
                missing.join(", ")
            ),
            Self::ApkForeignMember { name } => write!(
                f,
                "an apk artifact holds a member named {name:?}; the only legal members are app.apk, \
                 apk.json and icon.png (ADR-061 §3)"
            ),
            Self::ApkMemberCount { name, found } => write!(
                f,
                "an apk artifact must carry a member named {name:?} and this one carries {found}"
            ),
            Self::ApkWrongPlatform { found } => write!(
                f,
                "an apk artifact's platform must be \"android\" and this one carries {found:?} \
                 (ADR-061 §3)"
            ),
            Self::ApkWrongArch { found } => write!(
                f,
                "an apk artifact's arch must be \"any\" and this one carries {found:?} (ADR-061 §3)"
            ),
            Self::MalformedApkMetadata { fault } => write!(f, "apk.json cannot be read: {fault}"),
            Self::TextTooLong {
                field,
                len,
                capacity,
            } => {
                write!(
                    f,
                    "the {field} field holds {capacity} bytes and {len} were offered"
                )
            }
            Self::LengthOverflow => {
                write!(
                    f,
                    "the header and its members do not fit in a 64-bit length"
                )
            }
            Self::MemberBytesDisagree { index } => write!(
                f,
                "the bytes offered for member {index} are not the ones its record describes"
            ),
        }
    }
}

impl std::error::Error for PackageError {}

#[cfg(test)]
mod message_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::package::format::TextFault;

    /// Every variant, with a sample of each. **Exhaustive, and
    /// [`the_run_of_spaces_guard_covers_every_variant_there_is`] is what makes
    /// that true rather than aspirational.**
    fn every() -> Vec<PackageError> {
        vec![
            PackageError::TooShortForHeader { file_len: 3 },
            PackageError::NotAPackage { found: [0; 8] },
            PackageError::UnsupportedFormatVersion { found: 9 },
            PackageError::UntrustedKey {
                key_id: KeyId([0; 32]),
            },
            PackageError::BadSignature {
                key_id: KeyId([0; 32]),
            },
            PackageError::KindNotPermitted {
                key_id: KeyId([0; 32]),
                label: "joel-apps".to_string(),
                kind: "update".to_string(),
                may_sign: vec!["application".to_string()],
            },
            PackageError::WrongKindForInstall {
                kind: "update".to_string(),
                wanted: "application".to_string(),
            },
            PackageError::UnusableName {
                product: "../evil".to_string(),
            },
            PackageError::NamesAnImageService {
                name: "rivet-launcher".to_string(),
            },
            PackageError::NamesAReservedName {
                name: "root".to_string(),
            },
            PackageError::UnusableVersion {
                version: "current".to_string(),
            },
            PackageError::UnsafeMemberName {
                name: "data".to_string(),
            },
            PackageError::ReservedAssetName {
                name: "package-header".to_string(),
            },
            PackageError::ShortHeaderBuffer { given: 2 },
            PackageError::HeaderBytesWrong { found: 2 },
            PackageError::UnimplementedSignatureAlgorithm {
                found: SignatureAlgorithm::ED25519,
                implemented: SignatureAlgorithm::ED25519,
            },
            PackageError::UnimplementedDigestAlgorithm {
                found: DigestAlgorithm::BLAKE3,
                implemented: DigestAlgorithm::BLAKE3,
            },
            PackageError::UnknownFlags { flags: 2 },
            PackageError::MalformedMemberName {
                index: 2,
                fault: TextFault::DotOrDotDot,
            },
            PackageError::DuplicateMember {
                index: 2,
                name: "banner.txt".to_string(),
                variant: 9,
            },
            PackageError::MemberNotContiguous {
                index: 2,
                expected_offset: 2,
                found_offset: 2,
            },
            PackageError::MemberOverflowsFile {
                index: 2,
                offset: 2,
                stored_len: 2,
                file_len: 2,
            },
            PackageError::TrailingBytes {
                members_end: 2,
                file_len: 2,
            },
            PackageError::UnsupportedCompression { index: 2, found: 9 },
            PackageError::CompressionNotPermittedForKind {
                index: 2,
                kind: "update".to_string(),
            },
            PackageError::LengthsDisagree {
                index: 2,
                stored_len: 2,
                plain_len: 2,
            },
            PackageError::MemberOverrun {
                index: 2,
                stored_len: 2,
                offered: 2,
            },
            PackageError::MemberShort {
                index: 2,
                stored_len: 2,
                seen: 2,
            },
            PackageError::MemberDigestMismatch {
                index: 2,
                name: "banner.txt".to_string(),
            },
            PackageError::NoSuchMember {
                index: 2,
                member_count: 2,
            },
            PackageError::ReservedMemberName {
                index: 2,
                name: "banner.txt".to_string(),
            },
            PackageError::UnknownTargetClass { index: 2, found: 9 },
            PackageError::UnknownVariant { index: 2, found: 9 },
            PackageError::SystemMemberCount { found: 2 },
            PackageError::SlotVarianceIncomplete {
                name: "banner.txt".to_string(),
                missing: "system",
            },
            PackageError::SystemMemberIsSlotVariant {
                index: 2,
                variant: 9,
            },
            PackageError::BootMemberNameCollides {
                index: 2,
                name: "kernel8.img".to_string(),
                other_index: 3,
                other_name: "KERNEL8.IMG".to_string(),
            },
            PackageError::BootMemberNameCollidesWithTemporary {
                index: 2,
                name: "kernel8.img.new".to_string(),
                other_index: 3,
                other_name: "kernel8.img".to_string(),
            },
            PackageError::UnexpectedPlatform {
                found: "rpi5".to_string(),
            },
            PackageError::ManifestMemberCount { found: 2 },
            PackageError::ManifestNotFirst { index: 2 },
            PackageError::ManifestMisnamed {
                name: "banner.txt".to_string(),
            },
            PackageError::ProgramMemberCount { found: 2 },
            PackageError::WrongMemberVerified {
                expected: 2,
                found: 2,
            },
            PackageError::MalformedManifest {
                fault: crate::package::application::ManifestFault::MissingKey { key: "kind" },
            },
            PackageError::ObjectMemberCount { found: 0 },
            PackageError::MalformedLibraryManifest {
                fault: crate::package::library::ManifestFault::MissingKey { key: "soname" },
            },
            PackageError::ClosureNotSatisfied {
                missing: vec!["libgreet-1.0.0-".to_string() + &"ab".repeat(32)],
            },
            PackageError::ApkForeignMember {
                name: "readme.txt".to_string(),
            },
            PackageError::ApkMemberCount {
                name: "app.apk",
                found: 0,
            },
            PackageError::ApkWrongPlatform {
                found: Some("linux".to_string()),
            },
            PackageError::ApkWrongArch {
                found: "aarch64".to_string(),
            },
            PackageError::MalformedApkMetadata {
                fault: crate::package::apk::MetadataFault::MissingKey { key: "application_id" },
            },
            PackageError::MemberBytesDisagree { index: 2 },
            PackageError::WrongKind {
                expected: "update".to_string(),
                found: "application".to_string(),
            },
            PackageError::NothingToInstall,
            PackageError::LengthOverflow,
            PackageError::TooManyMembers { found: 99 },
            PackageError::ReservedBytesNotZero { field: "header" },
            PackageError::MissingText { field: "product" },
            PackageError::MalformedText {
                field: "product",
                fault: TextFault::Empty,
            },
            PackageError::TextTooLong {
                field: "product",
                len: 33,
                capacity: 32,
            },
        ]
    }

    /// **No refusal may contain a run of spaces**, which is a formatting defect
    /// rather than a style preference.
    ///
    /// Added 2026-08-24 after five messages shipped with them, including both of
    /// that day's most important refusals. The cause is worth naming because it
    /// is invisible in review: a `\` line-continuation inside a string literal
    /// strips the newline **and** the following indentation, so when the
    /// backslash is lost the indentation is left behind as ordinary text and
    /// the message reads "signing              key".
    ///
    /// **This paragraph blamed `cargo fmt` until 2026-09-02, and that was
    /// wrong.** It said the literal was reflowed onto one line by the
    /// formatter. Tested directly against rustfmt 1.9.0 with this repository's
    /// configuration — there is no `rustfmt.toml`, so `format_strings` is false
    /// — and the continuation survived untouched. The real mechanism is
    /// anything that processes escapes while WRITING the file, which here has
    /// meant agent tooling. Naming the wrong cause is worse than naming none:
    /// it sends the next reader to configure a formatter that was innocent.
    ///
    /// These are the sentences an operator reads at the worst moment they will
    /// have with this software, so they are checked like anything else.
    ///
    /// **This guard covers one enum in one crate**, which is why sixty-five of
    /// these were found alive in twenty-nine files on 2026-09-02, none of them
    /// reachable from `every()`. `tools/source/prose.py` is the repo-wide
    /// version and is the one that would have caught them.
    #[test]
    fn no_refusal_carries_a_run_of_spaces() {
        for error in every() {
            let said = error.to_string();
            assert!(
                !said.contains("  "),
                "this refusal carries a run of spaces, so a line continuation survived into the literal: {said:?}"
            );
        }
    }

    /// **The name of a variant, by an exhaustive match.**
    ///
    /// This exists to fail to compile. Adding a variant to [`PackageError`]
    /// stops this building until somebody adds it here, and
    /// [`the_run_of_spaces_guard_covers_every_variant_there_is`] then fails
    /// until it is added to `every()` as well.
    ///
    /// **Written because the guard silently covered less than it claimed.** On
    /// 2026-08-24 four variants were added with `\` line continuations in their
    /// messages — the exact defect `no_refusal_carries_a_run_of_spaces` was
    /// written that morning to catch — and it did not catch them, because its
    /// fixture is a hand-written list and nobody had added them to it. A guard
    /// that has to be remembered is a guard that will be forgotten, which is
    /// `AGENTS.md` §7 wearing a different hat.
    fn name_of(error: &PackageError) -> &'static str {
        match error {
            PackageError::TooShortForHeader { .. } => "TooShortForHeader",
            PackageError::ShortHeaderBuffer { .. } => "ShortHeaderBuffer",
            PackageError::NotAPackage { .. } => "NotAPackage",
            PackageError::UnsupportedFormatVersion { .. } => "UnsupportedFormatVersion",
            PackageError::HeaderBytesWrong { .. } => "HeaderBytesWrong",
            PackageError::UnimplementedSignatureAlgorithm { .. } => {
                "UnimplementedSignatureAlgorithm"
            }
            PackageError::UnimplementedDigestAlgorithm { .. } => "UnimplementedDigestAlgorithm",
            PackageError::KindNotPermitted { .. } => "KindNotPermitted",
            PackageError::WrongKindForInstall { .. } => "WrongKindForInstall",
            PackageError::UnusableName { .. } => "UnusableName",
            PackageError::NamesAnImageService { .. } => "NamesAnImageService",
            PackageError::NamesAReservedName { .. } => "NamesAReservedName",
            PackageError::UnusableVersion { .. } => "UnusableVersion",
            PackageError::UnsafeMemberName { .. } => "UnsafeMemberName",
            PackageError::ReservedAssetName { .. } => "ReservedAssetName",
            PackageError::UntrustedKey { .. } => "UntrustedKey",
            PackageError::BadSignature { .. } => "BadSignature",
            PackageError::TooManyMembers { .. } => "TooManyMembers",
            PackageError::UnknownFlags { .. } => "UnknownFlags",
            PackageError::ReservedBytesNotZero { .. } => "ReservedBytesNotZero",
            PackageError::MalformedText { .. } => "MalformedText",
            PackageError::MissingText { .. } => "MissingText",
            PackageError::MalformedMemberName { .. } => "MalformedMemberName",
            PackageError::DuplicateMember { .. } => "DuplicateMember",
            PackageError::MemberNotContiguous { .. } => "MemberNotContiguous",
            PackageError::MemberOverflowsFile { .. } => "MemberOverflowsFile",
            PackageError::TrailingBytes { .. } => "TrailingBytes",
            PackageError::UnsupportedCompression { .. } => "UnsupportedCompression",
            PackageError::CompressionNotPermittedForKind { .. } => "CompressionNotPermittedForKind",
            PackageError::LengthsDisagree { .. } => "LengthsDisagree",
            PackageError::MemberOverrun { .. } => "MemberOverrun",
            PackageError::MemberShort { .. } => "MemberShort",
            PackageError::MemberDigestMismatch { .. } => "MemberDigestMismatch",
            PackageError::NoSuchMember { .. } => "NoSuchMember",
            PackageError::WrongKind { .. } => "WrongKind",
            PackageError::ReservedMemberName { .. } => "ReservedMemberName",
            PackageError::UnknownTargetClass { .. } => "UnknownTargetClass",
            PackageError::UnknownVariant { .. } => "UnknownVariant",
            PackageError::NothingToInstall { .. } => "NothingToInstall",
            PackageError::SystemMemberCount { .. } => "SystemMemberCount",
            PackageError::SlotVarianceIncomplete { .. } => "SlotVarianceIncomplete",
            PackageError::SystemMemberIsSlotVariant { .. } => "SystemMemberIsSlotVariant",
            PackageError::BootMemberNameCollides { .. } => "BootMemberNameCollides",
            PackageError::BootMemberNameCollidesWithTemporary { .. } => {
                "BootMemberNameCollidesWithTemporary"
            }
            PackageError::UnexpectedPlatform { .. } => "UnexpectedPlatform",
            PackageError::ManifestMemberCount { .. } => "ManifestMemberCount",
            PackageError::ManifestNotFirst { .. } => "ManifestNotFirst",
            PackageError::ManifestMisnamed { .. } => "ManifestMisnamed",
            PackageError::ProgramMemberCount { .. } => "ProgramMemberCount",
            PackageError::WrongMemberVerified { .. } => "WrongMemberVerified",
            PackageError::MalformedManifest { .. } => "MalformedManifest",
            PackageError::ObjectMemberCount { .. } => "ObjectMemberCount",
            PackageError::MalformedLibraryManifest { .. } => "MalformedLibraryManifest",
            PackageError::ClosureNotSatisfied { .. } => "ClosureNotSatisfied",
            PackageError::ApkForeignMember { .. } => "ApkForeignMember",
            PackageError::ApkMemberCount { .. } => "ApkMemberCount",
            PackageError::ApkWrongPlatform { .. } => "ApkWrongPlatform",
            PackageError::ApkWrongArch { .. } => "ApkWrongArch",
            PackageError::MalformedApkMetadata { .. } => "MalformedApkMetadata",
            PackageError::TextTooLong { .. } => "TextTooLong",
            PackageError::LengthOverflow { .. } => "LengthOverflow",
            PackageError::MemberBytesDisagree { .. } => "MemberBytesDisagree",
        }
    }

    /// Every variant's name. Kept beside [`name_of`], which the compiler forces
    /// somebody to visit at the same moment.
    const EVERY_VARIANT: &[&str] = &[
        "TooShortForHeader",
        "ShortHeaderBuffer",
        "NotAPackage",
        "UnsupportedFormatVersion",
        "HeaderBytesWrong",
        "UnimplementedSignatureAlgorithm",
        "UnimplementedDigestAlgorithm",
        "KindNotPermitted",
        "WrongKindForInstall",
        "UnusableName",
        "NamesAnImageService",
        "NamesAReservedName",
        "UnusableVersion",
        "UnsafeMemberName",
        "ReservedAssetName",
        "UntrustedKey",
        "BadSignature",
        "TooManyMembers",
        "UnknownFlags",
        "ReservedBytesNotZero",
        "MalformedText",
        "MissingText",
        "MalformedMemberName",
        "DuplicateMember",
        "MemberNotContiguous",
        "MemberOverflowsFile",
        "TrailingBytes",
        "UnsupportedCompression",
        "CompressionNotPermittedForKind",
        "LengthsDisagree",
        "MemberOverrun",
        "MemberShort",
        "MemberDigestMismatch",
        "NoSuchMember",
        "WrongKind",
        "ReservedMemberName",
        "UnknownTargetClass",
        "UnknownVariant",
        "NothingToInstall",
        "SystemMemberCount",
        "SlotVarianceIncomplete",
        "SystemMemberIsSlotVariant",
        "BootMemberNameCollides",
        "BootMemberNameCollidesWithTemporary",
        "UnexpectedPlatform",
        "ManifestMemberCount",
        "ManifestNotFirst",
        "ManifestMisnamed",
        "ProgramMemberCount",
        "WrongMemberVerified",
        "MalformedManifest",
        "ObjectMemberCount",
        "MalformedLibraryManifest",
        "ClosureNotSatisfied",
        "ApkForeignMember",
        "ApkMemberCount",
        "ApkWrongPlatform",
        "ApkWrongArch",
        "MalformedApkMetadata",
        "TextTooLong",
        "LengthOverflow",
        "MemberBytesDisagree",
    ];

    /// The fixture covers every variant this enum has — not most of them, and
    /// not the ones somebody happened to remember.
    #[test]
    fn the_run_of_spaces_guard_covers_every_variant_there_is() {
        let covered: Vec<&str> = every().iter().map(name_of).collect();
        for name in EVERY_VARIANT {
            assert!(
                covered.contains(name),
                "PackageError::{name} is not in the refusal fixture, so nothing checks its message. Add a sample to `every()`."
            );
        }
    }
}
