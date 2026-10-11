//! The APK Signing Block: which certificate an APK is signed with now, and
//! the v3 proof-of-rotation lineage that links it to earlier ones.
//!
//! The block sits immediately before the central directory and ends with its
//! own size and the magic `APK Sig Block 42`. Between its two size fields is a
//! run of ID-value pairs; three IDs carry signers:
//!
//! | ID | scheme |
//! | :-- | :-- |
//! | `0x1b93ad61` | v3.1 — rotation targeted at newer SDKs |
//! | `0xf05368c0` | v3 |
//! | `0x7109871a` | v2 |
//!
//! **The current certificate comes from the newest scheme present**: v3.1,
//! then v3, then v2. A phone on a current SDK verifies with that one, so it is
//! the one a later update must match. Within a v3 or v3.1 block, the signer
//! whose SDK range reaches highest is current.
//!
//! Nothing here verifies a signature or a digest — `ADR-061` §4.

use super::{ApkError, add, slice, sha256_hex, to_usize, u32_at, u64_at};

const BLOCK: &str = "APK Signing Block";
const MAGIC: &[u8; 16] = b"APK Sig Block 42";

const V2_ID: u32 = 0x7109_871a;
const V3_ID: u32 = 0xf053_68c0;
const V31_ID: u32 = 0x1b93_ad61;
/// The v3 signed-data attribute carrying the proof-of-rotation lineage.
const PROOF_OF_ROTATION_ID: u32 = 0x3ba0_6f8c;
/// The one lineage encoding version there is.
const LINEAGE_VERSION: u32 = 1;

/// Which signature scheme a certificate was taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    V2,
    V3,
    V31,
}

impl core::fmt::Display for Scheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::V2 => "v2",
            Self::V3 => "v3",
            Self::V31 => "v3.1",
        })
    }
}

/// The current signer, and its lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Signing {
    pub(super) scheme: Scheme,
    pub(super) certificate: Vec<u8>,
    pub(super) lineage: Vec<Vec<u8>>,
}

/// A cursor over length-prefixed data, every read bounded.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    structure: &'static str,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], structure: &'static str) -> Self {
        Self { data, at: 0, structure }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.at
    }

    fn u32(&mut self) -> Result<u32, ApkError> {
        let value = u32_at(self.data, self.at, self.structure)?;
        self.at += 4;
        Ok(value)
    }

    /// A `u32`-length-prefixed slice.
    fn prefixed(&mut self) -> Result<&'a [u8], ApkError> {
        let len = self.u32()? as usize;
        let bytes = slice(self.data, self.at, len, self.structure)?;
        self.at += len;
        Ok(bytes)
    }
}

/// Read the signing block that ends at `cd_offset`.
pub(super) fn read(data: &[u8], cd_offset: usize) -> Result<Signing, ApkError> {
    let pairs = pairs(data, cd_offset)?;
    let find = |id: u32| -> Result<Option<&[u8]>, ApkError> {
        let mut found = pairs.iter().filter(|(pid, _)| *pid == id);
        let first = found.next().map(|(_, value)| *value);
        if found.next().is_some() {
            return Err(ApkError::Malformed {
                structure: BLOCK,
                offset: cd_offset,
                detail: format!("ID 0x{id:08x} appears twice"),
            });
        }
        Ok(first)
    };

    if let Some(value) = find(V31_ID)? {
        return v3_signer(value, Scheme::V31);
    }
    if let Some(value) = find(V3_ID)? {
        return v3_signer(value, Scheme::V3);
    }
    if let Some(value) = find(V2_ID)? {
        return v2_signer(value);
    }
    Err(ApkError::NoSigner)
}

/// The block's ID-value pairs, or [`ApkError::NoSigningBlock`] when there is
/// no block before the central directory.
fn pairs(data: &[u8], cd_offset: usize) -> Result<Vec<(u32, &[u8])>, ApkError> {
    // The smallest block is its two size fields and the magic.
    if cd_offset < 32 || cd_offset > data.len() {
        return Err(ApkError::NoSigningBlock);
    }
    let footer = cd_offset - 24;
    if slice(data, footer + 8, 16, BLOCK)? != MAGIC {
        return Err(ApkError::NoSigningBlock);
    }
    let size = to_usize(u64_at(data, footer, BLOCK)?, BLOCK, footer)?;
    // `size` counts everything after the leading size field.
    let total = add(size, 8, BLOCK, footer)?;
    if total > cd_offset || size < 24 {
        return Err(ApkError::Malformed {
            structure: BLOCK,
            offset: footer,
            detail: format!("a block of {size} bytes ending at offset {cd_offset}"),
        });
    }
    let start = cd_offset - total;
    let leading = u64_at(data, start, BLOCK)?;
    if leading != size as u64 {
        return Err(ApkError::Malformed {
            structure: BLOCK,
            offset: start,
            detail: format!("the leading size {leading} and the trailing size {size} disagree"),
        });
    }

    let region = &data[start + 8..footer];
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < region.len() {
        let len = to_usize(u64_at(region, at, BLOCK)?, BLOCK, start + 8 + at)?;
        if len < 4 {
            return Err(ApkError::Malformed {
                structure: BLOCK,
                offset: start + 8 + at,
                detail: format!("a pair of {len} bytes, too short to hold its ID"),
            });
        }
        let body = slice(region, at + 8, len, BLOCK)?;
        let id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        out.push((id, &body[4..]));
        at = add(at + 8, len, BLOCK, start + 8 + at)?;
    }
    Ok(out)
}

/// A v2 block: exactly one signer, whose first certificate is current.
fn v2_signer(value: &[u8]) -> Result<Signing, ApkError> {
    const V2: &str = "v2 signature";
    let mut outer = Reader::new(value, V2);
    let mut signers = Reader::new(outer.prefixed()?, V2);
    let mut certificates = Vec::new();
    while signers.remaining() > 0 {
        let mut signer = Reader::new(signers.prefixed()?, V2);
        let mut signed = Reader::new(signer.prefixed()?, V2);
        let _digests = signed.prefixed()?;
        certificates.push(first_certificate(signed.prefixed()?, V2)?);
    }
    if certificates.len() != 1 {
        return Err(ApkError::SignerCount { scheme: Scheme::V2, count: certificates.len() });
    }
    Ok(Signing {
        scheme: Scheme::V2,
        certificate: certificates.remove(0),
        lineage: Vec::new(),
    })
}

/// One signer of a v3 or v3.1 block.
struct V3Signer {
    /// (max SDK, min SDK): the order in which "reaches highest" is decided.
    sdk: (u32, u32),
    certificate: Vec<u8>,
    lineage: Vec<Vec<u8>>,
}

/// A v3 or v3.1 block: the signer whose SDK range reaches highest.
fn v3_signer(value: &[u8], scheme: Scheme) -> Result<Signing, ApkError> {
    let structure = match scheme {
        Scheme::V31 => "v3.1 signature",
        _ => "v3 signature",
    };
    let mut outer = Reader::new(value, structure);
    let mut signers = Reader::new(outer.prefixed()?, structure);
    let mut found: Vec<V3Signer> = Vec::new();
    while signers.remaining() > 0 {
        let mut signer = Reader::new(signers.prefixed()?, structure);
        let signed_bytes = signer.prefixed()?;
        let min_sdk = signer.u32()?;
        let max_sdk = signer.u32()?;

        let mut signed = Reader::new(signed_bytes, structure);
        let _digests = signed.prefixed()?;
        let certificate = first_certificate(signed.prefixed()?, structure)?;
        let _min = signed.u32()?;
        let _max = signed.u32()?;
        let mut lineage = Vec::new();
        let mut attributes = Reader::new(signed.prefixed()?, structure);
        while attributes.remaining() > 0 {
            let mut attribute = Reader::new(attributes.prefixed()?, structure);
            if attribute.u32()? == PROOF_OF_ROTATION_ID {
                lineage = read_lineage(&attribute.data[attribute.at..])?;
            }
        }
        found.push(V3Signer { sdk: (max_sdk, min_sdk), certificate, lineage });
    }

    let count = found.len();
    found.sort_by_key(|signer| core::cmp::Reverse(signer.sdk));
    let mut found = found.into_iter();
    let Some(V3Signer { sdk, certificate, lineage }) = found.next() else {
        return Err(ApkError::SignerCount { scheme, count });
    };
    // Two signers for the same range name no one current certificate.
    if found.next().is_some_and(|next| next.sdk == sdk) {
        return Err(ApkError::SignerCount { scheme, count });
    }

    if let Some(last) = lineage.last()
        && *last != certificate
    {
        return Err(ApkError::LineageDoesNotEndAtSigner {
            signer_sha256: sha256_hex(&certificate),
            last_sha256: sha256_hex(last),
        });
    }
    Ok(Signing { scheme, certificate, lineage })
}

/// The first certificate in a length-prefixed sequence of them — the signer's
/// own; any after it are its chain.
fn first_certificate(certificates: &[u8], structure: &'static str) -> Result<Vec<u8>, ApkError> {
    let mut reader = Reader::new(certificates, structure);
    if reader.remaining() == 0 {
        return Err(ApkError::Malformed {
            structure,
            offset: 0,
            detail: "a signer with no certificate".to_string(),
        });
    }
    Ok(reader.prefixed()?.to_vec())
}

/// The proof-of-rotation attribute: a version, then length-prefixed nodes,
/// each holding length-prefixed signed data whose first field is the node's
/// length-prefixed certificate. Oldest first.
fn read_lineage(value: &[u8]) -> Result<Vec<Vec<u8>>, ApkError> {
    const LINEAGE: &str = "v3 proof-of-rotation lineage";
    let mut reader = Reader::new(value, LINEAGE);
    let version = reader.u32()?;
    if version != LINEAGE_VERSION {
        return Err(ApkError::Malformed {
            structure: LINEAGE,
            offset: 0,
            detail: format!("lineage version {version}, where {LINEAGE_VERSION} is the one there is"),
        });
    }
    let mut certificates = Vec::new();
    while reader.remaining() > 0 {
        let mut node = Reader::new(reader.prefixed()?, LINEAGE);
        let mut signed = Reader::new(node.prefixed()?, LINEAGE);
        certificates.push(signed.prefixed()?.to_vec());
        let _signature_algorithm = signed.u32()?;
        let _flags = node.u32()?;
        let _parent_algorithm = node.u32()?;
        let _signature = node.prefixed()?;
    }
    if certificates.is_empty() {
        return Err(ApkError::Malformed {
            structure: LINEAGE,
            offset: 4,
            detail: "a lineage with no certificate in it".to_string(),
        });
    }
    Ok(certificates)
}
