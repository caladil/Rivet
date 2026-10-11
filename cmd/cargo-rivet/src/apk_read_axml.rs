//! Android binary XML (`AXML`) — the compiled form of `AndroidManifest.xml`,
//! and the string pool it shares with `resources.arsc`.
//!
//! A file is a chunk of type `RES_XML_TYPE` holding a string pool, a resource
//! map, and a flat run of start-namespace, start-element, end-element and
//! end-namespace chunks. Every chunk opens with the same eight bytes: a
//! 16-bit type, a 16-bit header size and a 32-bit total size.
//!
//! Attributes are matched by **namespace URI and name**, which is how
//! Android's own lightweight manifest parser asks for `versionCode` and the
//! rest; the resource map is not consulted.

use super::{ApkError, add, mul, slice, u8_at, u16_at, u32_at};

const XML: &str = "AndroidManifest.xml binary XML";
const POOL: &str = "string pool";

const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_XML_TYPE: u16 = 0x0003;
const RES_XML_START_ELEMENT_TYPE: u16 = 0x0102;
const RES_XML_END_ELEMENT_TYPE: u16 = 0x0103;

/// The string pool's flag for UTF-8 strings; otherwise they are UTF-16.
const UTF8_FLAG: u32 = 1 << 8;
/// "No string" in any string-index field.
const NO_INDEX: u32 = 0xffff_ffff;

const ANDROID_NS: &str = "http://schemas.android.com/apk/res/android";

// `Res_value` data types this reader interprets.
const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;

/// A typed attribute value, as far as a manifest needs one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Value {
    Str(String),
    Int(u32),
    /// A resource id, to be resolved through `resources.arsc`.
    Reference(u32),
    /// Any other `Res_value` type, kept only so a refusal can name it.
    Other { data_type: u8, data: u32 },
}

impl core::fmt::Display for Value {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Str(text) => write!(f, "the string {text:?}"),
            Self::Int(n) => write!(f, "the integer {n}"),
            Self::Reference(id) => write!(f, "a reference to resource 0x{id:08x}"),
            Self::Other { data_type, data } => {
                write!(f, "a value of type 0x{data_type:02x} (data 0x{data:08x})")
            }
        }
    }
}

/// What the manifest says, before any reference is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Manifest {
    pub(super) package: String,
    pub(super) version_code: u32,
    pub(super) version_code_major: Option<u32>,
    pub(super) version_name: Value,
    pub(super) min_sdk: Option<Value>,
    pub(super) target_sdk: Option<Value>,
    pub(super) label: Value,
}

/// A string pool chunk, decoded lazily: a string is read only when it is
/// asked for, so a malformed string nothing uses refuses nothing.
pub(super) struct StringPool<'a> {
    chunk: &'a [u8],
    count: usize,
    utf8: bool,
    offsets_at: usize,
    strings_at: usize,
    structure: &'static str,
}

impl<'a> StringPool<'a> {
    /// `chunk` is the whole chunk, header included.
    pub(super) fn parse(chunk: &'a [u8], structure: &'static str) -> Result<Self, ApkError> {
        let header_size = usize::from(u16_at(chunk, 2, structure)?);
        if header_size < 28 {
            return Err(ApkError::Malformed {
                structure,
                offset: 2,
                detail: format!("a string pool header of {header_size} bytes, where 28 is the least"),
            });
        }
        let count = u32_at(chunk, 8, structure)? as usize;
        let flags = u32_at(chunk, 16, structure)?;
        let strings_at = u32_at(chunk, 20, structure)? as usize;
        // The offset table must fit the chunk before anything indexes it.
        slice(chunk, header_size, mul(count, 4, structure, 8)?, structure)?;
        Ok(Self {
            chunk,
            count,
            utf8: flags & UTF8_FLAG != 0,
            offsets_at: header_size,
            strings_at,
            structure,
        })
    }

    /// The string at `index`.
    pub(super) fn get(&self, index: u32) -> Result<String, ApkError> {
        let structure = self.structure;
        let i = index as usize;
        if i >= self.count {
            return Err(ApkError::Malformed {
                structure,
                offset: self.offsets_at,
                detail: format!("string index {index} in a pool of {}", self.count),
            });
        }
        let offset = u32_at(self.chunk, self.offsets_at + 4 * i, structure)? as usize;
        let at = add(self.strings_at, offset, structure, self.offsets_at + 4 * i)?;
        if self.utf8 {
            // UTF-16 length, then UTF-8 length, each one or two bytes.
            let (_, after_utf16) = utf8_length(self.chunk, at, structure)?;
            let (len, start) = utf8_length(self.chunk, after_utf16, structure)?;
            let bytes = slice(self.chunk, start, len, structure)?;
            String::from_utf8(bytes.to_vec()).map_err(|e| ApkError::Malformed {
                structure,
                offset: start,
                detail: format!("string {index} is not UTF-8: {e}"),
            })
        } else {
            let first = u16_at(self.chunk, at, structure)?;
            let (len, start) = if first & 0x8000 != 0 {
                let second = u16_at(self.chunk, at + 2, structure)?;
                ((usize::from(first & 0x7fff) << 16) | usize::from(second), at + 4)
            } else {
                (usize::from(first), at + 2)
            };
            let bytes = slice(self.chunk, start, mul(len, 2, structure, at)?, structure)?;
            let units: Vec<u16> =
                bytes.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect();
            String::from_utf16(&units).map_err(|e| ApkError::Malformed {
                structure,
                offset: start,
                detail: format!("string {index} is not UTF-16: {e}"),
            })
        }
    }
}

/// A UTF-8 pool length: one byte, or two when the first's high bit is set.
/// Returns the length and the offset just past it.
fn utf8_length(chunk: &[u8], at: usize, structure: &'static str) -> Result<(usize, usize), ApkError> {
    let first = u8_at(chunk, at, structure)?;
    if first & 0x80 != 0 {
        let second = u8_at(chunk, at + 1, structure)?;
        Ok(((usize::from(first & 0x7f) << 8) | usize::from(second), at + 2))
    } else {
        Ok((usize::from(first), at + 1))
    }
}

/// One chunk's header: its type, header size and total size, checked against
/// the region it must fit in.
pub(super) fn chunk_at(
    data: &[u8],
    at: usize,
    end: usize,
    structure: &'static str,
) -> Result<(u16, usize, usize), ApkError> {
    let kind = u16_at(data, at, structure)?;
    let header_size = usize::from(u16_at(data, at + 2, structure)?);
    let size = u32_at(data, at + 4, structure)? as usize;
    if size < 8 || header_size < 8 || header_size > size {
        return Err(ApkError::Malformed {
            structure,
            offset: at,
            detail: format!(
                "a chunk of type 0x{kind:04x} with header {header_size} bytes and size {size}"
            ),
        });
    }
    let chunk_end = add(at, size, structure, at + 4)?;
    if chunk_end > end {
        return Err(ApkError::Truncated {
            structure,
            offset: at,
            needed: size,
            available: end.saturating_sub(at),
        });
    }
    Ok((kind, header_size, size))
}

/// One attribute of a start element.
struct Attribute {
    namespace: Option<String>,
    name: String,
    raw: u32,
    data_type: u8,
    data: u32,
}

/// Read `<manifest>`, its `<uses-sdk>` and its `<application>`.
pub(super) fn read_manifest(data: &[u8]) -> Result<Manifest, ApkError> {
    let kind = u16_at(data, 0, XML)?;
    if kind != RES_XML_TYPE {
        return Err(ApkError::Malformed {
            structure: XML,
            offset: 0,
            detail: format!("chunk type 0x{kind:04x} where 0x{RES_XML_TYPE:04x} opens the file"),
        });
    }
    let (_, header_size, size) = chunk_at(data, 0, data.len(), XML)?;

    let mut pool: Option<StringPool<'_>> = None;
    let mut depth = 0usize;
    let mut manifest_attributes: Option<Vec<Attribute>> = None;
    let mut uses_sdk: Option<Vec<Attribute>> = None;
    let mut application: Option<Vec<Attribute>> = None;

    let mut at = header_size;
    while at < size {
        let (kind, chunk_header, chunk_size) = chunk_at(data, at, size, XML)?;
        let chunk = slice(data, at, chunk_size, XML)?;
        match kind {
            RES_STRING_POOL_TYPE => {
                if pool.is_some() {
                    return Err(ApkError::Malformed {
                        structure: XML,
                        offset: at,
                        detail: "a second string pool".to_string(),
                    });
                }
                pool = Some(StringPool::parse(chunk, POOL)?);
            }
            RES_XML_START_ELEMENT_TYPE => {
                let pool = pool.as_ref().ok_or_else(|| ApkError::Malformed {
                    structure: XML,
                    offset: at,
                    detail: "an element before the string pool".to_string(),
                })?;
                depth += 1;
                let name = pool.get(u32_at(chunk, chunk_header + 4, XML)?)?;
                let wanted = match (depth, name.as_str()) {
                    (1, "manifest") => Some(&mut manifest_attributes),
                    (1, other) => {
                        return Err(ApkError::Malformed {
                            structure: XML,
                            offset: at,
                            detail: format!("the root element is <{other}>, not <manifest>"),
                        });
                    }
                    (2, "uses-sdk") => Some(&mut uses_sdk),
                    (2, "application") => Some(&mut application),
                    _ => None,
                };
                if let Some(slot) = wanted {
                    if slot.is_some() {
                        return Err(ApkError::DuplicateElement {
                            element: match depth {
                                1 => "manifest",
                                _ if name == "uses-sdk" => "uses-sdk",
                                _ => "application",
                            },
                        });
                    }
                    *slot = Some(attributes(chunk, chunk_header, at, pool)?);
                }
            }
            RES_XML_END_ELEMENT_TYPE => {
                depth = depth.checked_sub(1).ok_or_else(|| ApkError::Malformed {
                    structure: XML,
                    offset: at,
                    detail: "an end element with no element open".to_string(),
                })?;
            }
            // Namespaces, the resource map, CDATA: nothing a manifest's
            // identity needs.
            _ => {}
        }
        at += chunk_size;
    }

    let manifest = manifest_attributes.ok_or(ApkError::MissingElement { element: "manifest" })?;
    let application = application.ok_or(ApkError::MissingElement { element: "application" })?;

    let package = match find(&manifest, None, "package") {
        Some(attribute) => match value(attribute, pool.as_ref())? {
            Value::Str(text) if !text.is_empty() => text,
            other => {
                return Err(ApkError::BadAttribute {
                    element: "manifest",
                    attribute: "package",
                    detail: format!("{other}, where the package name is needed"),
                });
            }
        },
        None => return Err(ApkError::MissingAttribute { element: "manifest", attribute: "package" }),
    };

    let version_code = int_attribute(&manifest, "manifest", "versionCode", pool.as_ref())?
        .ok_or(ApkError::MissingAttribute { element: "manifest", attribute: "android:versionCode" })?;
    let version_code_major =
        int_attribute(&manifest, "manifest", "versionCodeMajor", pool.as_ref())?;
    let version_name = match find(&manifest, Some(ANDROID_NS), "versionName") {
        Some(attribute) => value(attribute, pool.as_ref())?,
        None => {
            return Err(ApkError::MissingAttribute {
                element: "manifest",
                attribute: "android:versionName",
            });
        }
    };

    let (min_sdk, target_sdk) = match &uses_sdk {
        Some(attributes) => (
            find(attributes, Some(ANDROID_NS), "minSdkVersion")
                .map(|a| value(a, pool.as_ref()))
                .transpose()?,
            find(attributes, Some(ANDROID_NS), "targetSdkVersion")
                .map(|a| value(a, pool.as_ref()))
                .transpose()?,
        ),
        None => (None, None),
    };

    let label = match find(&application, Some(ANDROID_NS), "label") {
        Some(attribute) => value(attribute, pool.as_ref())?,
        None => {
            return Err(ApkError::MissingAttribute {
                element: "application",
                attribute: "android:label",
            });
        }
    };

    Ok(Manifest {
        package,
        version_code,
        version_code_major,
        version_name,
        min_sdk,
        target_sdk,
        label,
    })
}

/// A start element's attributes, read from its `ResXMLTree_attrExt`.
fn attributes(
    chunk: &[u8],
    header: usize,
    at: usize,
    pool: &StringPool<'_>,
) -> Result<Vec<Attribute>, ApkError> {
    let start = u16_at(chunk, header + 8, XML)?;
    let each = usize::from(u16_at(chunk, header + 10, XML)?);
    let count = usize::from(u16_at(chunk, header + 12, XML)?);
    if each < 20 {
        return Err(ApkError::Malformed {
            structure: XML,
            offset: at + header + 10,
            detail: format!("attributes of {each} bytes, where 20 is the least"),
        });
    }
    let first = header + usize::from(start);
    slice(chunk, first, mul(count, each, XML, at)?, XML)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let a = first + i * each;
        let namespace = match u32_at(chunk, a, XML)? {
            NO_INDEX => None,
            index => Some(pool.get(index)?),
        };
        out.push(Attribute {
            namespace,
            name: pool.get(u32_at(chunk, a + 4, XML)?)?,
            raw: u32_at(chunk, a + 8, XML)?,
            data_type: u8_at(chunk, a + 15, XML)?,
            data: u32_at(chunk, a + 16, XML)?,
        });
    }
    Ok(out)
}

fn find<'a>(attributes: &'a [Attribute], namespace: Option<&str>, name: &str) -> Option<&'a Attribute> {
    attributes.iter().find(|a| a.namespace.as_deref() == namespace && a.name == name)
}

fn value(attribute: &Attribute, pool: Option<&StringPool<'_>>) -> Result<Value, ApkError> {
    Ok(match attribute.data_type {
        TYPE_STRING => {
            let index = if attribute.raw == NO_INDEX { attribute.data } else { attribute.raw };
            let pool = pool.ok_or_else(|| ApkError::Malformed {
                structure: XML,
                offset: 0,
                detail: "a string attribute and no string pool".to_string(),
            })?;
            Value::Str(pool.get(index)?)
        }
        TYPE_INT_DEC | TYPE_INT_HEX => Value::Int(attribute.data),
        TYPE_REFERENCE => Value::Reference(attribute.data),
        data_type => Value::Other { data_type, data: attribute.data },
    })
}

/// An `android:` integer attribute of `<element>`, if present.
fn int_attribute(
    attributes: &[Attribute],
    element: &'static str,
    name: &'static str,
    pool: Option<&StringPool<'_>>,
) -> Result<Option<u32>, ApkError> {
    let Some(attribute) = find(attributes, Some(ANDROID_NS), name) else {
        return Ok(None);
    };
    match value(attribute, pool)? {
        Value::Int(n) => Ok(Some(n)),
        other => Err(ApkError::BadAttribute {
            element,
            attribute: match name {
                "versionCode" => "android:versionCode",
                _ => "android:versionCodeMajor",
            },
            detail: format!("{other}, where an integer is needed"),
        }),
    }
}
