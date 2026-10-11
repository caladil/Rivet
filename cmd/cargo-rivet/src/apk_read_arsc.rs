//! Just enough of `resources.arsc` to resolve a string reference.
//!
//! A resource id is `0xPPTTEEEE`: package, type and entry. The table is a
//! `RES_TABLE_TYPE` chunk holding the global string pool (where every string
//! value lives) and one chunk per package; a package holds, per type, one
//! `RES_TABLE_TYPE_TYPE` chunk per configuration. This reads the entry from
//! the **default** configuration — the one whose every qualifier is unset —
//! which is the value `aapt2 dump badging` prints as `application-label`.

use super::axml::{StringPool, chunk_at};
use super::{ApkError, add, mul, slice, u8_at, u16_at, u32_at};

const TABLE: &str = "resources.arsc resource table";
const POOL: &str = "resources.arsc string pool";

const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_TABLE_TYPE: u16 = 0x0002;
const RES_TABLE_PACKAGE_TYPE: u16 = 0x0200;
const RES_TABLE_TYPE_TYPE: u16 = 0x0201;

/// `ResTable_type` flags.
const FLAG_SPARSE: u8 = 0x01;
const FLAG_OFFSET16: u8 = 0x02;

/// `ResTable_entry` flags.
const ENTRY_FLAG_COMPLEX: u16 = 0x0001;
const ENTRY_FLAG_COMPACT: u16 = 0x0008;

const NO_ENTRY32: u32 = 0xffff_ffff;
const NO_ENTRY16: u16 = 0xffff;

const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;

/// How many references one resolution follows before refusing, so a table
/// whose references form a cycle is refused rather than followed for ever.
const MAX_HOPS: usize = 8;

/// The string resource `id` resolves to in the default configuration,
/// following references to other resources.
pub(super) fn resolve_string(table: &[u8], id: u32) -> Result<String, ApkError> {
    let kind = u16_at(table, 0, TABLE)?;
    if kind != RES_TABLE_TYPE {
        return Err(ApkError::Malformed {
            structure: TABLE,
            offset: 0,
            detail: format!("chunk type 0x{kind:04x} where 0x{RES_TABLE_TYPE:04x} opens the file"),
        });
    }
    let (_, header_size, size) = chunk_at(table, 0, table.len(), TABLE)?;

    let mut pool: Option<StringPool<'_>> = None;
    let mut packages: Vec<(u32, usize)> = Vec::new();
    let mut at = header_size;
    while at < size {
        let (kind, _, chunk_size) = chunk_at(table, at, size, TABLE)?;
        match kind {
            RES_STRING_POOL_TYPE if pool.is_none() => {
                pool = Some(StringPool::parse(slice(table, at, chunk_size, TABLE)?, POOL)?);
            }
            RES_TABLE_PACKAGE_TYPE => packages.push((u32_at(table, at + 8, TABLE)?, at)),
            _ => {}
        }
        at += chunk_size;
    }
    let pool = pool.ok_or_else(|| ApkError::Malformed {
        structure: TABLE,
        offset: header_size,
        detail: "no global string pool".to_string(),
    })?;

    let mut current = id;
    for _ in 0..MAX_HOPS {
        let (data_type, data) = default_value(table, &packages, current)?;
        match data_type {
            TYPE_STRING => return pool.get(data),
            TYPE_REFERENCE => current = data,
            other => {
                return Err(ApkError::UnresolvedReference {
                    resource: id,
                    detail: format!(
                        "0x{current:08x} holds a value of type 0x{other:02x}, not a string"
                    ),
                });
            }
        }
    }
    Err(ApkError::UnresolvedReference {
        resource: id,
        detail: format!("more than {MAX_HOPS} references in a row"),
    })
}

/// The `Res_value` (type, data) of resource `id` in the default configuration.
fn default_value(table: &[u8], packages: &[(u32, usize)], id: u32) -> Result<(u8, u32), ApkError> {
    let package_id = id >> 24;
    let type_id = ((id >> 16) & 0xff) as u8;
    let entry = (id & 0xffff) as usize;
    let unresolved = |detail: String| ApkError::UnresolvedReference { resource: id, detail };

    let &(_, package_at) = packages
        .iter()
        .find(|(pid, _)| *pid == package_id)
        .ok_or_else(|| unresolved(format!("no package 0x{package_id:02x} in the table")))?;
    let (_, package_header, package_size) = chunk_at(table, package_at, table.len(), TABLE)?;
    let package_end = package_at + package_size;

    let mut at = add(package_at, package_header, TABLE, package_at)?;
    while at < package_end {
        let (kind, header, chunk_size) = chunk_at(table, at, package_end, TABLE)?;
        if kind == RES_TABLE_TYPE_TYPE
            && u8_at(table, at + 8, TABLE)? == type_id
            && is_default_config(table, at, header)?
            && let Some(value) = entry_value(table, at, header, chunk_size, entry)?
        {
            return Ok(value);
        }
        at += chunk_size;
    }
    Err(unresolved("no value in the default configuration".to_string()))
}

/// Whether a type chunk's `ResTable_config` sets no qualifier at all.
fn is_default_config(table: &[u8], at: usize, header: usize) -> Result<bool, ApkError> {
    let config_at = at + 20;
    let config_size = u32_at(table, config_at, TABLE)? as usize;
    if config_size < 4 || 20 + config_size > header {
        return Err(ApkError::Malformed {
            structure: TABLE,
            offset: config_at,
            detail: format!("a configuration of {config_size} bytes in a header of {header}"),
        });
    }
    let qualifiers = slice(table, config_at + 4, config_size - 4, TABLE)?;
    Ok(qualifiers.iter().all(|b| *b == 0))
}

/// Entry `entry` of the type chunk at `at`: its (type, data), or `None` when
/// this configuration does not define it.
fn entry_value(
    table: &[u8],
    at: usize,
    header: usize,
    chunk_size: usize,
    entry: usize,
) -> Result<Option<(u8, u32)>, ApkError> {
    let chunk = slice(table, at, chunk_size, TABLE)?;
    let flags = u8_at(chunk, 9, TABLE)?;
    let count = u32_at(chunk, 12, TABLE)? as usize;
    let entries_at = u32_at(chunk, 16, TABLE)? as usize;

    let offset = if flags & FLAG_SPARSE != 0 {
        // (index, offset / 4) pairs, one per entry present.
        let pairs = slice(chunk, header, mul(count, 4, TABLE, at + 12)?, TABLE)?;
        let found = pairs.as_chunks::<4>().0.iter().find(|pair| {
            usize::from(u16::from_le_bytes([pair[0], pair[1]])) == entry
        });
        match found {
            Some(pair) => usize::from(u16::from_le_bytes([pair[2], pair[3]])) * 4,
            None => return Ok(None),
        }
    } else {
        if entry >= count {
            return Ok(None);
        }
        if flags & FLAG_OFFSET16 != 0 {
            match u16_at(chunk, header + 2 * entry, TABLE)? {
                NO_ENTRY16 => return Ok(None),
                offset => usize::from(offset) * 4,
            }
        } else {
            match u32_at(chunk, header + 4 * entry, TABLE)? {
                NO_ENTRY32 => return Ok(None),
                offset => offset as usize,
            }
        }
    };

    let entry_at = add(entries_at, offset, TABLE, at + 16)?;
    let entry_flags = u16_at(chunk, entry_at + 2, TABLE)?;
    if entry_flags & ENTRY_FLAG_COMPACT != 0 {
        // A compact entry carries its data type in the flags' high byte and
        // its data where a key index would otherwise be.
        let data_type = (entry_flags >> 8) as u8;
        return Ok(Some((data_type, u32_at(chunk, entry_at + 4, TABLE)?)));
    }
    if entry_flags & ENTRY_FLAG_COMPLEX != 0 {
        return Err(ApkError::Malformed {
            structure: TABLE,
            offset: at + entry_at,
            detail: "a bag (style, array or plural) where a single value is needed".to_string(),
        });
    }
    let entry_size = usize::from(u16_at(chunk, entry_at, TABLE)?);
    let value_at = add(entry_at, entry_size, TABLE, at + entry_at)?;
    let data_type = u8_at(chunk, value_at + 3, TABLE)?;
    let data = u32_at(chunk, value_at + 4, TABLE)?;
    Ok(Some((data_type, data)))
}
