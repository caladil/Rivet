//! The reader and the verifier — **the security boundary**, `ADR-028` §2's last
//! row.
//!
//! # The order is the design
//!
//! `ADR-029` §5 is not advice about a sensible sequence; it is the property the
//! whole format exists to have. Before a signature is checked this module does
//! **five integer comparisons and a table lookup**, allocates nothing, and
//! bounds no loop by a number an attacker chose:
//!
//! | # | Check | Where |
//! | :-- | :-- | :-- |
//! | 1 | file length ≥ 4096 | [`open`] |
//! | 2 | magic | [`open`] |
//! | 3 | `format_version == 1` | [`open`] |
//! | 4 | `header_bytes == 4096`, **checked, never obeyed** | [`open`] |
//! | 5 | `sig_alg` and `digest_alg` are what this build implements | [`open`] |
//! | 6 | `key_id` is in the trust set | [`open`] |
//! | 7 | the signature covers `[0 .. 4032)` | [`open`] |
//!
//! Only then is the manifest read, and by then its bytes are ours — so
//! `member_count`, `offset` and `stored_len` are **authenticated before they are
//! believed**.
//!
//! [`Package::verify_member`] and [`Package::member_stream`] are steps 8 and 9,
//! and they hand back a [`VerifiedMember`] rather than a `Result<(), _>`. That
//! is deliberate: the property `ADR-029` §9 rests on is *nothing is written
//! before verification*, and a token a caller cannot forge is a stronger way to
//! say it than a comment.
//!
//! # What this module cannot do, and why that is the point
//!
//! It performs **no I/O** — `ADR-014` §3. It never opens the payload, never
//! seeks and never reads. The caller has the file; this module is handed bytes
//! and says whether they are the ones the signature covers. That is what lets
//! `cargo test -p rivet-core` need no filesystem beyond temporary paths
//! (`ADR-014` §3.1) while still exercising every refusal in `ADR-029` §9's
//! table.

use crate::package::error::PackageError;
use crate::package::format::{
    ARCH_BYTES, DIGEST_BYTES, FORMAT_VERSION, HEADER_BYTES, HEADER_RESERVED_BYTES, KIND_BYTES,
    M_COMPRESSION, M_DIGEST, M_NAME, M_NAME_BYTES, M_OFFSET, M_PLAIN_LEN, M_RESERVED_0,
    M_RESERVED_1, M_RESERVED_1_BYTES, M_STORED_LEN, M_TARGET_CLASS, M_VARIANT, MAGIC, MAX_MEMBERS,
    MEMBER_RECORD_BYTES, MEMBER_TABLE_OFFSET, OFF_ARCH, OFF_BUILD_EPOCH, OFF_DIGEST_ALG, OFF_FLAGS,
    OFF_FORMAT_VERSION, OFF_HEADER_BYTES, OFF_HEADER_RESERVED, OFF_KEY_ID, OFF_KIND,
    OFF_MEMBER_COUNT, OFF_PLATFORM, OFF_PRODUCT, OFF_SIG_ALG, OFF_VERSION, PLATFORM_BYTES,
    PRODUCT_BYTES, SIGNATURE_BYTES, SIGNATURE_OFFSET, SIGNED_BYTES, VERSION_BYTES, read_i64,
    read_name, read_text, read_u32, read_u64,
};
use crate::package::kind::Kind;
use crate::package::port::{
    Digest, DigestAlgorithm, KeyId, Signature, SignatureAlgorithm, TrustSet, Verify,
};

/// The value `compression` takes when a member's bytes are stored as they are.
///
/// Legal on a member of any kind. `ADR-029` §5.1 puts compression *inside* the
/// container specifically so a decompressor never sees bytes the signer did not
/// produce.
pub const COMPRESSION_NONE: u8 = 0;

/// The value `compression` takes when a member's bytes are zstd-compressed.
///
/// **`R-177.1`, 2026-09-13: zstd, confirmed.** `ADR-029` §13/R-107 (2026-08-29)
/// keeps every *board-installed* kind uncompressed on purpose — a decompressor
/// is a permanent dependency on every device that boots it, to save transfer
/// time on an operation that happens rarely. That reasoning does not reach a
/// `card` image: it is decompressed exactly once, on the workstation, when
/// provisioning writes it, and no board ever holds one. So this codec is
/// **legal only on a member of a [`crate::package::kind::Kind::card`]
/// payload** — this module's own manifest parser and
/// [`crate::package::write::Builder::add`] both refuse it on any other kind,
/// with [`crate::package::error::PackageError::CompressionNotPermittedForKind`].
///
/// **This crate never decompresses.** It digests and verifies the *stored*
/// (compressed) bytes only — `ADR-029` §5.1's whole point, that the
/// decompressor never sees bytes the signer did not produce, is that it does
/// not run here at all. The zstd codec itself lives in `cmd/cargo-rivet`
/// alone, which is the one binary that ever needs to run it; this constant is
/// the byte value the format recognises, not an implementation.
pub const COMPRESSION_ZSTD: u8 = 1;

/// What the signed header says, once the signature has been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    signature_algorithm: SignatureAlgorithm,
    digest_algorithm: DigestAlgorithm,
    key_id: KeyId,
    product: String,
    version: String,
    arch: String,
    platform: Option<String>,
    kind: Kind,
    build_epoch: i64,
}

impl Header {
    #[must_use]
    pub fn signature_algorithm(&self) -> SignatureAlgorithm {
        self.signature_algorithm
    }
    #[must_use]
    pub fn digest_algorithm(&self) -> DigestAlgorithm {
        self.digest_algorithm
    }
    /// Which key signed it. `ADR-028` §5.
    #[must_use]
    pub fn key_id(&self) -> KeyId {
        self.key_id
    }
    #[must_use]
    pub fn product(&self) -> &str {
        &self.product
    }
    /// `ADR-028` §6.1: the manifest carries the version so that a non-newer
    /// payload is **reported**, and enforcement later needs no flag day.
    ///
    /// **This crate does not order versions.** Comparing `0.2.0` against
    /// `0.0.1-g7b84d4b` is a decision about what a version string means, no ADR
    /// makes it, and a wrong ordering would silently mislabel a downgrade as an
    /// upgrade — which is `AGENTS.md` §5's *"a wrong guess does not fail, it
    /// succeeds"* exactly. The string is carried; the comparison is open.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
    #[must_use]
    pub fn arch(&self) -> &str {
        &self.arch
    }
    /// `None` when the field is empty.
    ///
    /// **Empty is legal, and that is how `ADR-027` §9 items 3–4 stay open.** An
    /// application is architecture-shaped rather than firmware-shaped, so it may
    /// turn out not to carry a platform at all; a format that forced one would
    /// have answered the maintainer's question on his behalf.
    #[must_use]
    pub fn platform(&self) -> Option<&str> {
        self.platform.as_deref()
    }
    #[must_use]
    pub fn kind(&self) -> &Kind {
        &self.kind
    }
    /// `ADR-029` §3: *"informational, NEVER compared"*.
    ///
    /// It is here because a build wants a timestamp in a report, and it is never
    /// read by anything that decides. A clock is not a version, and a payload
    /// whose freshness rested on a builder's clock would be `ADR-028` §6's
    /// problem wearing a worse disguise.
    #[must_use]
    pub fn build_epoch(&self) -> i64 {
        self.build_epoch
    }
}

/// One member's signed record. `ADR-029` §3 — 96 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    index: usize,
    name: String,
    digest: [u8; DIGEST_BYTES],
    target_class: u8,
    variant: u8,
    compression: u8,
    offset: u64,
    stored_len: u64,
    plain_len: u64,
}

impl Member {
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Over the **stored** bytes. `ADR-029` §5.1 — if this were over the plain
    /// bytes the decompressor would be upstream of the verifier.
    #[must_use]
    pub fn digest(&self) -> &[u8; DIGEST_BYTES] {
        &self.digest
    }
    /// Byte 64. Kind-defined; for `update` see
    /// [`crate::package::update::TargetClass`].
    #[must_use]
    pub fn target_class(&self) -> u8 {
        self.target_class
    }
    /// Byte 65, which `ADR-029` §3 calls `slot_variance`.
    ///
    /// **Renamed at this layer, and the rename is deliberate rather than
    /// careless.** §11 makes the container general and *slot* is the update
    /// kind's word; a `slot_variance` on an application's member would be
    /// vocabulary from another kind leaking into a shape that is supposed to be
    /// shared. It is the same byte at the same offset, and
    /// [`crate::package::update::SlotVariance`] is what it means for an update.
    ///
    /// Its structural job in the container is exactly one thing: **it is the
    /// second half of a member's identity.** Two members may share a name if
    /// they differ here, which is what lets `ADR-029` §3's payload carry
    /// `cmdline.txt` twice.
    #[must_use]
    pub fn variant(&self) -> u8 {
        self.variant
    }
    #[must_use]
    pub fn compression(&self) -> u8 {
        self.compression
    }
    /// Where the stored bytes begin, from the start of the file.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }
    #[must_use]
    pub fn stored_len(&self) -> u64 {
        self.stored_len
    }
    #[must_use]
    pub fn plain_len(&self) -> u64 {
        self.plain_len
    }
}

/// A verified package: the header is signed by a trusted key and the manifest is
/// coherent. **No member's content has been checked yet** — that is
/// [`Package::verify_member`], and it must be done before any of it is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    header: Header,
    members: Vec<Member>,
    file_len: u64,
}

impl Package {
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }
    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }
    #[must_use]
    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    /// Members with this name, in file order. Plural because
    /// [`Member::variant`] may distinguish two.
    pub fn members_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Member> {
        self.members.iter().filter(move |m| m.name == name)
    }

    /// Refuse unless this is the kind the caller expected.
    pub fn require_kind(&self, expected: &Kind) -> Result<(), PackageError> {
        if self.header.kind == *expected {
            Ok(())
        } else {
            Err(PackageError::WrongKind {
                expected: expected.as_str().to_string(),
                found: self.header.kind.as_str().to_string(),
            })
        }
    }

    /// `ADR-029` §5 step 8 for one member, in one shot.
    ///
    /// For anything that will not fit in memory — the `/system` member is
    /// 640 MiB — use [`Self::member_stream`].
    pub fn verify_member<'a>(
        &'a self,
        index: usize,
        digest: &mut dyn Digest,
        stored: &[u8],
    ) -> Result<VerifiedMember<'a>, PackageError> {
        let mut stream = self.member_stream(index, digest)?;
        stream.write(stored)?;
        stream.finish()
    }

    /// `ADR-029` §5 step 8, a chunk at a time.
    ///
    /// The [`Digest`] is **borrowed by the stream** rather than passed to each
    /// call, so two members cannot be interleaved into one accumulator. That
    /// mistake would produce a digest neither member has and would surface as a
    /// refusal rather than as a wrong answer — but the type makes it
    /// unreachable, which is cheaper than the test that would have found it.
    pub fn member_stream<'a, 'd>(
        &'a self,
        index: usize,
        digest: &'d mut dyn Digest,
    ) -> Result<MemberStream<'a, 'd>, PackageError> {
        let member = self.members.get(index).ok_or(PackageError::NoSuchMember {
            index,
            member_count: self.members.len(),
        })?;
        digest.start();
        Ok(MemberStream {
            member,
            digest,
            seen: 0,
        })
    }
}

/// One member's bytes, being digested. See [`Package::member_stream`].
pub struct MemberStream<'a, 'd> {
    member: &'a Member,
    digest: &'d mut dyn Digest,
    seen: u64,
}

impl<'a> MemberStream<'a, '_> {
    /// The record this stream is checking against.
    #[must_use]
    pub fn member(&self) -> &'a Member {
        self.member
    }

    /// Absorb the next chunk, refusing to take more than the record says exists.
    ///
    /// **The refusal is not politeness.** A caller reading from a source that
    /// lies about its length — a fleet server, a pipe — would otherwise hash
    /// past the member and into whatever followed.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), PackageError> {
        let offered = self.seen.saturating_add(bytes.len() as u64);
        if offered > self.member.stored_len {
            return Err(PackageError::MemberOverrun {
                index: self.member.index,
                stored_len: self.member.stored_len,
                offered,
            });
        }
        self.digest.write(bytes);
        self.seen = offered;
        Ok(())
    }

    /// Finish: the length must match and the digest must match.
    pub fn finish(self) -> Result<VerifiedMember<'a>, PackageError> {
        if self.seen != self.member.stored_len {
            return Err(PackageError::MemberShort {
                index: self.member.index,
                stored_len: self.member.stored_len,
                seen: self.seen,
            });
        }
        let found = self.digest.finish();
        if found != self.member.digest {
            return Err(PackageError::MemberDigestMismatch {
                index: self.member.index,
                name: self.member.name.clone(),
            });
        }
        Ok(VerifiedMember {
            member: self.member,
        })
    }
}

/// Proof that one member's stored bytes are the ones the signature covers.
///
/// **This type is the point.** `ADR-029` §5 step 8 ends *"refuse on mismatch,
/// having written nothing further"*, and §9's whole failure table rests on
/// nothing being written before that. A caller cannot construct one of these,
/// so an installer that takes a `VerifiedMember` where it would have taken a
/// `&Member` cannot be called with unverified bytes by mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedMember<'a> {
    member: &'a Member,
}

impl<'a> VerifiedMember<'a> {
    #[must_use]
    pub fn member(self) -> &'a Member {
        self.member
    }
}

/// `ADR-029` §5 steps 1–7, then the manifest.
///
/// `head` is the first [`HEADER_BYTES`] of the file and `file_len` is the whole
/// file's length. Nothing else is read, and nothing is read *from* — this crate
/// performs no I/O (`ADR-014` §3).
///
/// # The five comparisons, and what happens if one is skipped
///
/// Each of steps 1–6 is cheap and each one removes a class of input from the
/// signature check's reach. The one that is easiest to get wrong is step 4:
/// `header_bytes` is compared against the constant and **never used to size a
/// read**, so a header declaring itself 4 GB long is a refusal rather than an
/// allocation.
/// Re-open the header of a package whose **file is gone** — `ADR-015` §4.2's
/// launch-time verification.
///
/// # Why this exists
///
/// An installed application is an unpacked tree: a program, a manifest, and
/// since 2026-08-25 the 4 KiB signed header beside them. The `.rvt` itself is
/// not kept — for a system update it is 683 MB — so [`open`]'s `file_len`
/// argument has nothing to supply it.
///
/// **The length is recomputed from the member table**, which is inside
/// `[0 .. SIGNED_BYTES)` and therefore covered by the signature. So
/// [`PackageError::TooShortForHeader`] and [`PackageError::TrailingBytes`]
/// become tautological on this path, deliberately: they exist to catch a
/// **truncated transfer**, and at launch there is no transfer. What is being
/// checked here is the signature, against the trust store **as it is now** —
/// which is the whole point, because `ADR-028` §5.5.4 makes revocation an
/// image update and an image update changes exactly that store.
///
/// The table is read before the signature is checked, which is safe for the
/// same reason every other field is: nothing downstream of a failed signature
/// is used. A forged header can make this compute any length it likes and then
/// fails to verify.
pub fn open_installed(
    head: &[u8],
    digest: &mut dyn Digest,
    verify: &dyn Verify,
    trust: &TrustSet,
) -> Result<Package, PackageError> {
    let file_len = implied_file_len(head).ok_or(PackageError::TooShortForHeader {
        file_len: head.len() as u64,
    })?;
    open(head, file_len, digest, verify, trust)
}

/// Where the last member ends, read straight out of the header's own table.
///
/// `None` when the buffer is not a whole header — the caller turns that into
/// the same refusal a short file gets. Saturating throughout: these are bytes
/// off a disk and the arithmetic must not be the thing that panics.
fn implied_file_len(head: &[u8]) -> Option<u64> {
    if head.len() < HEADER_BYTES {
        return None;
    }
    let count = (read_u32(head, OFF_MEMBER_COUNT)? as usize).min(MAX_MEMBERS);
    let mut end = HEADER_BYTES as u64;
    for index in 0..count {
        let at = MEMBER_TABLE_OFFSET + index * MEMBER_RECORD_BYTES;
        let offset = read_u64(head, at + M_OFFSET)?;
        let stored = read_u64(head, at + M_STORED_LEN)?;
        end = end.max(offset.saturating_add(stored));
    }
    Some(end)
}

pub fn open(
    head: &[u8],
    file_len: u64,
    digest: &mut dyn Digest,
    verify: &dyn Verify,
    trust: &TrustSet,
) -> Result<Package, PackageError> {
    // 1. The file is at least a header.
    if file_len < HEADER_BYTES as u64 {
        return Err(PackageError::TooShortForHeader { file_len });
    }
    // And the caller actually gave us one. A distinct refusal, because a short
    // buffer is a bug here and a short file is a truncated transfer (F1).
    if head.len() < HEADER_BYTES {
        return Err(PackageError::ShortHeaderBuffer { given: head.len() });
    }
    let head = &head[..HEADER_BYTES];

    // 2. Magic.
    let mut magic = [0u8; 8];
    magic.copy_from_slice(&head[..8]);
    if magic != MAGIC {
        return Err(PackageError::NotAPackage { found: magic });
    }

    // 3. Format version.
    let format_version = read_u32(head, OFF_FORMAT_VERSION).unwrap_or_default();
    if format_version != FORMAT_VERSION {
        return Err(PackageError::UnsupportedFormatVersion {
            found: format_version,
        });
    }

    // 4. Header size — checked against the constant, never obeyed.
    let declared = read_u32(head, OFF_HEADER_BYTES).unwrap_or_default();
    if declared != HEADER_BYTES as u32 {
        return Err(PackageError::HeaderBytesWrong { found: declared });
    }

    // 5. Algorithms this build implements.
    let sig_alg = SignatureAlgorithm(read_u32(head, OFF_SIG_ALG).unwrap_or_default());
    if sig_alg != verify.algorithm() {
        return Err(PackageError::UnimplementedSignatureAlgorithm {
            found: sig_alg,
            implemented: verify.algorithm(),
        });
    }
    let digest_alg = DigestAlgorithm(read_u32(head, OFF_DIGEST_ALG).unwrap_or_default());
    if digest_alg != digest.algorithm() {
        return Err(PackageError::UnimplementedDigestAlgorithm {
            found: digest_alg,
            implemented: digest.algorithm(),
        });
    }

    // 6. The key is one this machine trusts.
    let mut id = [0u8; DIGEST_BYTES];
    id.copy_from_slice(&head[OFF_KEY_ID..OFF_KEY_ID + DIGEST_BYTES]);
    let key_id = KeyId(id);
    let entry = trust
        .find(&key_id)
        .ok_or(PackageError::UntrustedKey { key_id })?;

    // 7. The signature covers [0 .. 4032).
    let mut raw = [0u8; SIGNATURE_BYTES];
    raw.copy_from_slice(&head[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_BYTES]);
    if !verify.verify(entry.key(), &head[..SIGNED_BYTES], &Signature(raw)) {
        return Err(PackageError::BadSignature { key_id });
    }

    // --- Everything below this line is authenticated. -----------------------
    let package = parse_manifest(head, file_len, sig_alg, digest_alg, key_id)?;

    // 8. The key is entitled to sign THIS KIND of artifact — `ADR-028` §5.5.
    //
    // **After step 7 and not before, and the order is the whole point.** `kind`
    // lives at `OFF_KIND`, well below `UNAUTHENTICATED_BOUNDARY`, so until the
    // signature verifies it is a claim an attacker wrote. Checking a mask
    // against an unauthenticated kind would let a payload nominate the
    // permission it wanted to be judged by.
    //
    // What this stops is the case the two keys exist for: a key held to sign
    // applications offering a system image. The signature is *valid* — the
    // holder really did sign it — and it is refused anyway, because signing and
    // being entitled to sign are different questions (`ADR-028` §4).
    if !entry.may_sign().permits(package.header().kind()) {
        return Err(PackageError::KindNotPermitted {
            key_id,
            label: entry.label().to_string(),
            kind: package.header().kind().as_str().to_string(),
            may_sign: entry
                .may_sign()
                .kinds()
                .iter()
                .map(|k| k.as_str().to_string())
                .collect(),
        });
    }

    Ok(package)
}

/// The manifest, parsed only after `ADR-029` §5 step 7 has passed.
fn parse_manifest(
    head: &[u8],
    file_len: u64,
    signature_algorithm: SignatureAlgorithm,
    digest_algorithm: DigestAlgorithm,
    key_id: KeyId,
) -> Result<Package, PackageError> {
    let member_count = read_u32(head, OFF_MEMBER_COUNT).unwrap_or_default();
    if member_count as usize > MAX_MEMBERS {
        return Err(PackageError::TooManyMembers {
            found: member_count,
        });
    }
    let flags = read_u32(head, OFF_FLAGS).unwrap_or_default();
    if flags != 0 {
        return Err(PackageError::UnknownFlags { flags });
    }
    if head[OFF_HEADER_RESERVED..OFF_HEADER_RESERVED + HEADER_RESERVED_BYTES]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(PackageError::ReservedBytesNotZero {
            field: "the end of the header",
        });
    }
    // Every member record above `member_count` must be zero, for the reason the
    // reserved bytes must be: a signed header with no room for bytes nobody
    // reads.
    let used = MEMBER_TABLE_OFFSET + member_count as usize * MEMBER_RECORD_BYTES;
    if head[used..OFF_HEADER_RESERVED].iter().any(|b| *b != 0) {
        return Err(PackageError::ReservedBytesNotZero {
            field: "the unused member records",
        });
    }

    let product = required_text(head, OFF_PRODUCT, PRODUCT_BYTES, "product")?;
    let version = required_text(head, OFF_VERSION, VERSION_BYTES, "version")?;
    let arch = required_text(head, OFF_ARCH, ARCH_BYTES, "arch")?;
    let platform = optional_text(head, OFF_PLATFORM, PLATFORM_BYTES, "platform")?;
    let kind_text = required_text(head, OFF_KIND, KIND_BYTES, "kind")?;
    let kind = Kind::parse(&kind_text)?;
    let build_epoch = read_i64(head, OFF_BUILD_EPOCH).unwrap_or_default();

    let mut members: Vec<Member> = Vec::with_capacity(member_count as usize);
    let mut expected_offset = HEADER_BYTES as u64;
    for index in 0..member_count as usize {
        let at = MEMBER_TABLE_OFFSET + index * MEMBER_RECORD_BYTES;
        let record = &head[at..at + MEMBER_RECORD_BYTES];

        if record[M_RESERVED_0] != 0
            || record[M_RESERVED_1..M_RESERVED_1 + M_RESERVED_1_BYTES]
                .iter()
                .any(|b| *b != 0)
        {
            return Err(PackageError::ReservedBytesNotZero {
                field: "a member record",
            });
        }

        let name = read_name(&record[M_NAME..M_NAME + M_NAME_BYTES])
            .map_err(|fault| PackageError::MalformedMemberName { index, fault })?
            .to_string();
        let mut member_digest = [0u8; DIGEST_BYTES];
        member_digest.copy_from_slice(&record[M_DIGEST..M_DIGEST + DIGEST_BYTES]);

        let compression = record[M_COMPRESSION];
        match compression {
            COMPRESSION_NONE => {}
            // `kind` is already authenticated here — it was read from the same
            // signed header, above this loop — so this compares one already-
            // trusted field against another rather than judging an attacker's
            // claim by itself.
            COMPRESSION_ZSTD if kind == Kind::card() => {}
            COMPRESSION_ZSTD => {
                return Err(PackageError::CompressionNotPermittedForKind {
                    index,
                    kind: kind.as_str().to_string(),
                });
            }
            other => {
                return Err(PackageError::UnsupportedCompression { index, found: other });
            }
        }
        let variant = record[M_VARIANT];
        let offset = read_u64(record, M_OFFSET).unwrap_or_default();
        let stored_len = read_u64(record, M_STORED_LEN).unwrap_or_default();
        let plain_len = read_u64(record, M_PLAIN_LEN).unwrap_or_default();

        // **Only a member stored plain must match its own length twice.** A
        // compressed member's whole reason for compressing is that the two
        // differ; `R-177.1` widens what this format's members may be without
        // touching what an uncompressed one still promises.
        if compression == COMPRESSION_NONE && stored_len != plain_len {
            return Err(PackageError::LengthsDisagree {
                index,
                stored_len,
                plain_len,
            });
        }
        if offset != expected_offset {
            return Err(PackageError::MemberNotContiguous {
                index,
                expected_offset,
                found_offset: offset,
            });
        }
        let end = offset
            .checked_add(stored_len)
            .ok_or(PackageError::LengthOverflow)?;
        if end > file_len {
            return Err(PackageError::MemberOverflowsFile {
                index,
                offset,
                stored_len,
                file_len,
            });
        }
        if members
            .iter()
            .any(|m: &Member| m.name == name && m.variant == variant)
        {
            return Err(PackageError::DuplicateMember {
                index,
                name,
                variant,
            });
        }
        expected_offset = end;
        members.push(Member {
            index,
            name,
            digest: member_digest,
            target_class: record[M_TARGET_CLASS],
            variant,
            compression,
            offset,
            stored_len,
            plain_len,
        });
    }

    if expected_offset != file_len {
        return Err(PackageError::TrailingBytes {
            members_end: expected_offset,
            file_len,
        });
    }

    Ok(Package {
        header: Header {
            signature_algorithm,
            digest_algorithm,
            key_id,
            product,
            version,
            arch,
            platform,
            kind,
            build_epoch,
        },
        members,
        file_len,
    })
}

fn required_text(
    head: &[u8],
    at: usize,
    len: usize,
    field: &'static str,
) -> Result<String, PackageError> {
    match read_text(&head[at..at + len]) {
        Ok(Some(text)) => Ok(text.to_string()),
        Ok(None) => Err(PackageError::MissingText { field }),
        Err(fault) => Err(PackageError::MalformedText { field, fault }),
    }
}

fn optional_text(
    head: &[u8],
    at: usize,
    len: usize,
    field: &'static str,
) -> Result<Option<String>, PackageError> {
    match read_text(&head[at..at + len]) {
        Ok(text) => Ok(text.map(ToString::to_string)),
        Err(fault) => Err(PackageError::MalformedText { field, fault }),
    }
}
