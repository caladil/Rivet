//! The zip half of an APK: the end-of-central-directory record, the central
//! directory, and the local headers that lead to an entry's data.
//!
//! Only what an APK is: one disk, no Zip64, no encryption, entries stored or
//! deflated. Anything else is refused by name. The central directory is where
//! every size is taken from, because an APK's local headers may defer theirs
//! to a data descriptor.

use super::{ApkError, add, slice, to_usize, u16_at, u32_at};

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const EOCD_LEN: usize = 22;
const CD_SIGNATURE: u32 = 0x0201_4b50;
const CD_FIXED_LEN: usize = 46;
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const LOCAL_FIXED_LEN: usize = 30;
/// The longest a zip comment can be, which bounds the search for the record.
const MAX_COMMENT: usize = 0xffff;

pub(crate) const METHOD_STORED: u16 = 0;
pub(crate) const METHOD_DEFLATED: u16 = 8;

const EOCD: &str = "zip end-of-central-directory record";
const CD: &str = "zip central directory";
const LOCAL: &str = "zip local header";
const DATA: &str = "zip entry data";

/// One central-directory record, as far as this reader needs it.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) name: Vec<u8>,
    pub(crate) flags: u16,
    pub(crate) method: u16,
    /// Kept so a derived test fixture carries the original's; this reader
    /// checks no CRC.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) crc32: u32,
    pub(crate) compressed: u64,
    pub(crate) uncompressed: u64,
    pub(crate) local_offset: u64,
}

/// An opened archive: its central directory, and where that directory starts
/// — which is where the APK Signing Block ends.
pub(crate) struct Archive<'a> {
    data: &'a [u8],
    cd_offset: usize,
    entries: Vec<Entry>,
}

impl<'a> Archive<'a> {
    /// Find the end-of-central-directory record and read the directory it
    /// points at.
    pub(crate) fn open(data: &'a [u8]) -> Result<Self, ApkError> {
        let eocd = find_eocd(data)?;
        let disk = u16_at(data, eocd + 4, EOCD)?;
        let cd_disk = u16_at(data, eocd + 6, EOCD)?;
        let on_disk = u16_at(data, eocd + 8, EOCD)?;
        let total = u16_at(data, eocd + 10, EOCD)?;
        let cd_size = u32_at(data, eocd + 12, EOCD)?;
        let cd_offset = u32_at(data, eocd + 16, EOCD)?;

        if total == 0xffff || on_disk == 0xffff {
            return Err(ApkError::Zip64 { field: "entry count" });
        }
        if cd_size == 0xffff_ffff {
            return Err(ApkError::Zip64 { field: "central directory size" });
        }
        if cd_offset == 0xffff_ffff {
            return Err(ApkError::Zip64 { field: "central directory offset" });
        }
        if disk != 0 || cd_disk != 0 || on_disk != total {
            return Err(ApkError::SpannedArchive);
        }

        let cd_offset = to_usize(u64::from(cd_offset), EOCD, eocd + 16)?;
        let cd_size = to_usize(u64::from(cd_size), EOCD, eocd + 12)?;
        // An APK's central directory ends exactly where the record begins —
        // the v2 signature scheme depends on it — so a gap or an overlap is a
        // file this reader would have to guess about.
        if add(cd_offset, cd_size, EOCD, eocd + 12)? != eocd {
            return Err(ApkError::Malformed {
                structure: EOCD,
                offset: eocd + 12,
                detail: format!(
                    "the central directory is {cd_size} bytes at offset {cd_offset}, which does \
                     not end where this record begins ({eocd})"
                ),
            });
        }
        let directory = slice(data, cd_offset, cd_size, CD)?;

        let mut entries = Vec::with_capacity(usize::from(total).min(directory.len() / CD_FIXED_LEN));
        let mut at = 0usize;
        for _ in 0..total {
            let signature = u32_at(directory, at, CD)?;
            if signature != CD_SIGNATURE {
                return Err(ApkError::Malformed {
                    structure: CD,
                    offset: cd_offset + at,
                    detail: format!("signature 0x{signature:08x} where 0x{CD_SIGNATURE:08x} belongs"),
                });
            }
            let flags = u16_at(directory, at + 8, CD)?;
            let method = u16_at(directory, at + 10, CD)?;
            let crc32 = u32_at(directory, at + 16, CD)?;
            let compressed = u32_at(directory, at + 20, CD)?;
            let uncompressed = u32_at(directory, at + 24, CD)?;
            let name_len = usize::from(u16_at(directory, at + 28, CD)?);
            let extra_len = usize::from(u16_at(directory, at + 30, CD)?);
            let comment_len = usize::from(u16_at(directory, at + 32, CD)?);
            let local_offset = u32_at(directory, at + 42, CD)?;
            if compressed == 0xffff_ffff || uncompressed == 0xffff_ffff || local_offset == 0xffff_ffff {
                return Err(ApkError::Zip64 { field: "central directory entry" });
            }
            let name = slice(directory, at + CD_FIXED_LEN, name_len, CD)?.to_vec();
            entries.push(Entry {
                name,
                flags,
                method,
                crc32,
                compressed: u64::from(compressed),
                uncompressed: u64::from(uncompressed),
                local_offset: u64::from(local_offset),
            });
            at += CD_FIXED_LEN + name_len + extra_len + comment_len;
        }

        Ok(Self { data, cd_offset, entries })
    }

    /// Where the central directory starts — the end of the APK Signing Block.
    pub(crate) fn central_directory_offset(&self) -> usize {
        self.cd_offset
    }

    /// The one record named `name`.
    pub(crate) fn find(&self, name: &'static str) -> Result<&Entry, ApkError> {
        let mut found = self.entries.iter().filter(|e| e.name == name.as_bytes());
        let first = found.next().ok_or(ApkError::MissingEntry { name })?;
        if found.next().is_some() {
            return Err(ApkError::DuplicateEntry { name });
        }
        Ok(first)
    }

    /// An entry's bytes exactly as they lie in the file — compressed if it is
    /// compressed.
    pub(crate) fn raw(&self, entry: &Entry) -> Result<&'a [u8], ApkError> {
        let local = to_usize(entry.local_offset, LOCAL, 0)?;
        let signature = u32_at(self.data, local, LOCAL)?;
        if signature != LOCAL_SIGNATURE {
            return Err(ApkError::Malformed {
                structure: LOCAL,
                offset: local,
                detail: format!("signature 0x{signature:08x} where 0x{LOCAL_SIGNATURE:08x} belongs"),
            });
        }
        let name_len = usize::from(u16_at(self.data, local + 26, LOCAL)?);
        let extra_len = usize::from(u16_at(self.data, local + 28, LOCAL)?);
        let start = add(local, LOCAL_FIXED_LEN + name_len + extra_len, LOCAL, local)?;
        let len = to_usize(entry.compressed, DATA, start)?;
        // An entry's data precedes the central directory; one that runs into
        // it (or into the signing block before it) is not an entry's data.
        let end = add(start, len, DATA, start)?;
        if end > self.cd_offset {
            return Err(ApkError::Truncated {
                structure: DATA,
                offset: start,
                needed: len,
                available: self.cd_offset.saturating_sub(start),
            });
        }
        slice(self.data, start, len, DATA)
    }

    /// The named entry's bytes, inflated if it is deflated.
    pub(crate) fn read(&self, name: &'static str) -> Result<Vec<u8>, ApkError> {
        let entry = self.find(name)?;
        if entry.flags & 1 != 0 {
            return Err(ApkError::Encrypted { entry: name.to_string() });
        }
        let raw = self.raw(entry)?;
        match entry.method {
            METHOD_STORED => {
                if entry.compressed != entry.uncompressed {
                    return Err(ApkError::InflatedSize {
                        entry: name.to_string(),
                        declared: entry.uncompressed,
                        actual: entry.compressed,
                    });
                }
                Ok(raw.to_vec())
            }
            METHOD_DEFLATED => {
                let limit = to_usize(entry.uncompressed, CD, 0)?;
                let inflated = miniz_oxide::inflate::decompress_to_vec_with_limit(raw, limit)
                    .map_err(|e| ApkError::Inflate {
                        entry: name.to_string(),
                        detail: format!("{:?}", e.status),
                    })?;
                if inflated.len() as u64 != entry.uncompressed {
                    return Err(ApkError::InflatedSize {
                        entry: name.to_string(),
                        declared: entry.uncompressed,
                        actual: inflated.len() as u64,
                    });
                }
                Ok(inflated)
            }
            method => Err(ApkError::UnknownCompression { entry: name.to_string(), method }),
        }
    }
}

/// The end-of-central-directory record: the last one whose comment length
/// reaches exactly to the end of the file.
fn find_eocd(data: &[u8]) -> Result<usize, ApkError> {
    if data.len() < EOCD_LEN {
        return Err(ApkError::NoEndOfCentralDirectory);
    }
    let last = data.len() - EOCD_LEN;
    let first = last.saturating_sub(MAX_COMMENT);
    for at in (first..=last).rev() {
        if u32_at(data, at, EOCD)? == EOCD_SIGNATURE {
            let comment = usize::from(u16_at(data, at + 20, EOCD)?);
            if at + EOCD_LEN + comment == data.len() {
                return Ok(at);
            }
        }
    }
    Err(ApkError::NoEndOfCentralDirectory)
}
