//! The writer.
//!
//! # Why it lives beside the reader rather than in the build tool
//!
//! `ADR-011`: one authority per fact. The fact here is *where the signature
//! starts and what it covers*, and a writer in `cmd/rivet-image` would be a
//! second spelling of `4032` maintained by a different person from the one who
//! maintains the verifier. `cmd/rivet-image`'s tests already carry the scar of
//! the general case — they ran a **verbatim copy** of PID 1's `autoboot.txt`
//! parser behind a text-equality drift guard, until `ADR-014` §2.2 made the real
//! one depend-able.
//!
//! # It refuses everything the reader refuses
//!
//! A builder that can emit a header its own verifier rejects will eventually
//! ship one, and the failure lands on a board rather than on a build machine. So
//! [`Builder::add`] applies the reader's name rules, the reader's duplicate rule
//! and the reader's compression rule at the point a member is added — where the
//! diagnostic can still name the file on the build host's disk.
//!
//! # It does not hold a payload in memory
//!
//! [`Builder::seal`] returns the 4096-byte header and nothing else; the caller
//! writes it and then streams each member's stored bytes in order. The largest
//! member is a whole `/system` filesystem, and a writer that needed it resident
//! would work on the development host and fail on the smallest machine anyone
//! tried it on.
//!
//! [`Builder::seal_in_memory`] exists for the small cases and for the test
//! suite, and it **re-digests every member it is given** rather than trusting
//! the records — so a caller that hands it the wrong bytes is told, instead of
//! producing a package that fails on the board.

use crate::package::error::PackageError;
use crate::package::format::{
    ARCH_BYTES, DIGEST_BYTES, FORMAT_VERSION, HEADER_BYTES, KIND_BYTES, M_COMPRESSION, M_DIGEST,
    M_NAME, M_NAME_BYTES, M_OFFSET, M_PLAIN_LEN, M_STORED_LEN, M_TARGET_CLASS, M_VARIANT, MAGIC,
    MAX_MEMBERS, MEMBER_RECORD_BYTES, MEMBER_TABLE_OFFSET, OFF_ARCH, OFF_BUILD_EPOCH,
    OFF_DIGEST_ALG, OFF_FORMAT_VERSION, OFF_HEADER_BYTES, OFF_KEY_ID, OFF_KIND, OFF_MEMBER_COUNT,
    OFF_PLATFORM, OFF_PRODUCT, OFF_SIG_ALG, OFF_VERSION, PLATFORM_BYTES, PRODUCT_BYTES,
    SIGNATURE_BYTES, SIGNATURE_OFFSET, SIGNED_BYTES, VERSION_BYTES, read_name, write_text,
};
use crate::package::kind::Kind;
use crate::package::port::{Digest, Sign};
use crate::package::read::{COMPRESSION_NONE, COMPRESSION_ZSTD};

/// A member about to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMember {
    name: String,
    target_class: u8,
    variant: u8,
    compression: u8,
    digest: [u8; DIGEST_BYTES],
    stored_len: u64,
    plain_len: u64,
    offset: u64,
}

impl NewMember {
    /// Compute the digest here, over the bytes that will be **stored**.
    ///
    /// `ADR-029` §5.1 — the digest is over the stored bytes precisely so that a
    /// decompressor only ever sees bytes the signer produced. Taking the slice
    /// and hashing it here means a caller cannot get that backwards by passing a
    /// digest of the plain form.
    pub fn stored(
        name: &str,
        target_class: u8,
        variant: u8,
        bytes: &[u8],
        digest: &mut dyn Digest,
    ) -> Result<Self, PackageError> {
        let len = bytes.len() as u64;
        Self::streamed(
            name,
            target_class,
            variant,
            COMPRESSION_NONE,
            digest.of(bytes),
            len,
            len,
        )
    }

    /// For a member too large to hold — the caller streamed it past a
    /// [`Digest`] and has the result.
    pub fn streamed(
        name: &str,
        target_class: u8,
        variant: u8,
        compression: u8,
        digest: [u8; DIGEST_BYTES],
        stored_len: u64,
        plain_len: u64,
    ) -> Result<Self, PackageError> {
        if name.len() > M_NAME_BYTES {
            return Err(PackageError::TextTooLong {
                field: "member name",
                len: name.len(),
                capacity: M_NAME_BYTES,
            });
        }
        // The reader's own rule, applied here: no traversal, no control bytes,
        // no `.`/`..`, nothing that would not survive the round trip.
        let mut field = [0u8; M_NAME_BYTES];
        field[..name.len()].copy_from_slice(name.as_bytes());
        let round_tripped =
            read_name(&field).map_err(|fault| PackageError::MalformedMemberName {
                index: usize::MAX,
                fault,
            })?;
        if round_tripped != name {
            return Err(PackageError::MalformedMemberName {
                index: usize::MAX,
                fault: crate::package::format::TextFault::TrailingBytesAfterNul,
            });
        }
        // **The kind gate lives in [`Builder::add`], not here.** This
        // constructor has no `Kind` to compare against — a `Builder` does —
        // so it recognises the byte value and leaves whether THIS payload may
        // carry it to the caller that knows the kind.
        match compression {
            COMPRESSION_NONE | COMPRESSION_ZSTD => {}
            other => {
                return Err(PackageError::UnsupportedCompression {
                    index: usize::MAX,
                    found: other,
                });
            }
        }
        // Only a plainly-stored member must match its own length twice — see
        // `read::parse_manifest`'s identical rule, which this holds itself to
        // for `AGENTS.md` §5's reason: a builder that could emit what its own
        // verifier refuses is a builder that ships one.
        if compression == COMPRESSION_NONE && stored_len != plain_len {
            return Err(PackageError::LengthsDisagree {
                index: usize::MAX,
                stored_len,
                plain_len,
            });
        }
        Ok(Self {
            name: name.to_string(),
            target_class,
            variant,
            compression,
            digest,
            stored_len,
            plain_len,
            offset: 0,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn stored_len(&self) -> u64 {
        self.stored_len
    }
    /// Where this member will land, assigned by [`Builder::add`].
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

/// Assembles one `.rvt`.
#[derive(Debug, Clone)]
pub struct Builder {
    product: String,
    version: String,
    arch: String,
    platform: Option<String>,
    kind: Kind,
    build_epoch: i64,
    members: Vec<NewMember>,
    reserved: Vec<String>,
    next_offset: u64,
}

impl Builder {
    /// `platform` is `None` for an artifact that does not carry one.
    ///
    /// **That option exists because `ADR-027` §9 items 3–4 are open** — whether
    /// applications and extensions carry `<platform>` is the maintainer's, and a
    /// builder that required one would have answered it.
    pub fn new(
        product: &str,
        version: &str,
        arch: &str,
        platform: Option<&str>,
        kind: Kind,
    ) -> Result<Self, PackageError> {
        check_text("product", product, PRODUCT_BYTES, false)?;
        check_text("version", version, VERSION_BYTES, false)?;
        check_text("arch", arch, ARCH_BYTES, false)?;
        if let Some(platform) = platform {
            check_text("platform", platform, PLATFORM_BYTES, true)?;
        }
        check_text("kind", kind.as_str(), KIND_BYTES, false)?;
        Ok(Self {
            product: product.to_string(),
            version: version.to_string(),
            arch: arch.to_string(),
            platform: platform.map(ToString::to_string),
            kind,
            build_epoch: 0,
            members: Vec::new(),
            reserved: Vec::new(),
            next_offset: HEADER_BYTES as u64,
        })
    }

    /// `ADR-029` §3: *"informational, NEVER compared"*.
    #[must_use]
    pub fn build_epoch(mut self, epoch: i64) -> Self {
        self.build_epoch = epoch;
        self
    }

    /// Names this build refuses to carry — `ADR-029` §4.
    ///
    /// **The list is not this crate's to know**, which is why it is supplied.
    /// §4 is explicit: *"This is the port's job, not the installer's. `BootSlots`
    /// gains an operation naming what a platform reserves on a payload target —
    /// on the Pi that is a pair of filenames; on a UEFI machine the equivalent
    /// object is an EFI variable and is not a file at all."* `rivet-core` may
    /// not name `BootSlots` (`ADR-014` §3 forbids core → adapter), so the
    /// composition root asks the port and passes the answer in.
    #[must_use]
    pub fn reserving(mut self, names: &[&str]) -> Self {
        self.reserved = names.iter().map(|n| (*n).to_string()).collect();
        self
    }

    /// Add a member, assigning it the next offset.
    ///
    /// Returns the offset, so a streaming writer can assert it wrote where the
    /// header says it did.
    pub fn add(&mut self, mut member: NewMember) -> Result<u64, PackageError> {
        let index = self.members.len();
        if index >= MAX_MEMBERS {
            return Err(PackageError::TooManyMembers {
                found: index as u32 + 1,
            });
        }
        // Case-insensitive: a target whose reserved names are files on a FAT
        // filesystem (`ADR-029` §4) compares names by ASCII case fold, so a
        // caller reserving `rivet.toml` is also reserving `RIVET.TOML`.
        if self
            .reserved
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&member.name))
        {
            return Err(PackageError::ReservedMemberName {
                index,
                name: member.name.clone(),
            });
        }
        // **Only a `card` payload may compress a member** — `R-177.1`. Checked
        // here, where `self.kind` is known, rather than in `NewMember::streamed`,
        // which has no `Kind` to compare against. `read::parse_manifest` makes
        // the identical check on the way back in, so a builder cannot emit what
        // its own reader refuses.
        if member.compression != COMPRESSION_NONE && self.kind != Kind::card() {
            return Err(PackageError::CompressionNotPermittedForKind {
                index,
                kind: self.kind.as_str().to_string(),
            });
        }
        if self
            .members
            .iter()
            .any(|m| m.name == member.name && m.variant == member.variant)
        {
            return Err(PackageError::DuplicateMember {
                index,
                name: member.name.clone(),
                variant: member.variant,
            });
        }
        member.offset = self.next_offset;
        self.next_offset = self
            .next_offset
            .checked_add(member.stored_len)
            .ok_or(PackageError::LengthOverflow)?;
        let offset = member.offset;
        self.members.push(member);
        Ok(offset)
    }

    #[must_use]
    pub fn members(&self) -> &[NewMember] {
        &self.members
    }

    /// The size of the finished file: the header plus every member.
    #[must_use]
    pub fn total_len(&self) -> u64 {
        self.next_offset
    }

    /// Compose and sign the header.
    ///
    /// `sig_alg` and `digest_alg` are taken from the implementors that produced
    /// them, never from a parameter — so a header cannot claim an algorithm
    /// other than the one that actually signed and hashed it.
    pub fn seal(
        &self,
        digest: &mut dyn Digest,
        signer: &mut dyn Sign,
    ) -> Result<[u8; HEADER_BYTES], PackageError> {
        let mut head = [0u8; HEADER_BYTES];
        head[..8].copy_from_slice(&MAGIC);
        head[OFF_FORMAT_VERSION..OFF_FORMAT_VERSION + 4]
            .copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        head[OFF_HEADER_BYTES..OFF_HEADER_BYTES + 4]
            .copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
        head[OFF_SIG_ALG..OFF_SIG_ALG + 4].copy_from_slice(&signer.algorithm().0.to_le_bytes());
        head[OFF_DIGEST_ALG..OFF_DIGEST_ALG + 4]
            .copy_from_slice(&digest.algorithm().0.to_le_bytes());

        let key = signer.public_key();
        let key_id = digest.of(&key.0);
        head[OFF_KEY_ID..OFF_KEY_ID + DIGEST_BYTES].copy_from_slice(&key_id);

        head[OFF_MEMBER_COUNT..OFF_MEMBER_COUNT + 4]
            .copy_from_slice(&(self.members.len() as u32).to_le_bytes());
        // `flags` stays zero: this format version defines none, and the reader
        // refuses any bit that is set.

        text_into(
            &mut head[OFF_PRODUCT..OFF_PRODUCT + PRODUCT_BYTES],
            &self.product,
            "product",
        )?;
        text_into(
            &mut head[OFF_VERSION..OFF_VERSION + VERSION_BYTES],
            &self.version,
            "version",
        )?;
        text_into(
            &mut head[OFF_ARCH..OFF_ARCH + ARCH_BYTES],
            &self.arch,
            "arch",
        )?;
        if let Some(platform) = &self.platform {
            text_into(
                &mut head[OFF_PLATFORM..OFF_PLATFORM + PLATFORM_BYTES],
                platform,
                "platform",
            )?;
        }
        text_into(
            &mut head[OFF_KIND..OFF_KIND + KIND_BYTES],
            self.kind.as_str(),
            "kind",
        )?;
        head[OFF_BUILD_EPOCH..OFF_BUILD_EPOCH + 8].copy_from_slice(&self.build_epoch.to_le_bytes());

        for (index, member) in self.members.iter().enumerate() {
            let at = MEMBER_TABLE_OFFSET + index * MEMBER_RECORD_BYTES;
            let record = &mut head[at..at + MEMBER_RECORD_BYTES];
            record[M_NAME..M_NAME + member.name.len()].copy_from_slice(member.name.as_bytes());
            record[M_DIGEST..M_DIGEST + DIGEST_BYTES].copy_from_slice(&member.digest);
            record[M_TARGET_CLASS] = member.target_class;
            record[M_VARIANT] = member.variant;
            record[M_COMPRESSION] = member.compression;
            record[M_OFFSET..M_OFFSET + 8].copy_from_slice(&member.offset.to_le_bytes());
            record[M_STORED_LEN..M_STORED_LEN + 8]
                .copy_from_slice(&member.stored_len.to_le_bytes());
            record[M_PLAIN_LEN..M_PLAIN_LEN + 8].copy_from_slice(&member.plain_len.to_le_bytes());
        }

        let signature = signer.sign(&head[..SIGNED_BYTES]);
        head[SIGNATURE_OFFSET..SIGNATURE_OFFSET + SIGNATURE_BYTES].copy_from_slice(&signature.0);
        Ok(head)
    }

    /// The whole file, for the cases that fit in memory and for the tests.
    ///
    /// Re-digests every member rather than trusting the records it was built
    /// with. `AGENTS.md` §4's *"individually-correct-but-jointly-wrong"* case is
    /// exactly a builder whose header and whose bytes were assembled from
    /// different sources.
    pub fn seal_in_memory(
        &self,
        digest: &mut dyn Digest,
        signer: &mut dyn Sign,
        member_bytes: &[&[u8]],
    ) -> Result<Vec<u8>, PackageError> {
        if member_bytes.len() != self.members.len() {
            return Err(PackageError::MemberBytesDisagree {
                index: self.members.len().min(member_bytes.len()),
            });
        }
        for (index, (member, bytes)) in self.members.iter().zip(member_bytes).enumerate() {
            if bytes.len() as u64 != member.stored_len || digest.of(bytes) != member.digest {
                return Err(PackageError::MemberBytesDisagree { index });
            }
        }
        let head = self.seal(digest, signer)?;
        let mut out = Vec::with_capacity(self.total_len() as usize);
        out.extend_from_slice(&head);
        for bytes in member_bytes {
            out.extend_from_slice(bytes);
        }
        Ok(out)
    }
}

fn check_text(
    field: &'static str,
    text: &str,
    capacity: usize,
    may_be_empty: bool,
) -> Result<(), PackageError> {
    if text.len() > capacity {
        return Err(PackageError::TextTooLong {
            field,
            len: text.len(),
            capacity,
        });
    }
    if text.is_empty() && !may_be_empty {
        return Err(PackageError::MissingText { field });
    }
    let mut scratch = vec![0u8; capacity];
    write_text(&mut scratch, text).map_err(|fault| PackageError::MalformedText { field, fault })
}

fn text_into(field: &mut [u8], text: &str, name: &'static str) -> Result<(), PackageError> {
    write_text(field, text).map_err(|fault| PackageError::MalformedText { field: name, fault })
}
