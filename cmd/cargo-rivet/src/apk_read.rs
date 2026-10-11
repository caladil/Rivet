//! Reading an Android APK — the four structures `ADR-061` §4 names, read here
//! rather than by Android's own tools.
//!
//! [`read`](crate::apk_read::read) takes an APK's bytes and answers what `apk.json` needs: the
//! package name, `versionCode`, `versionName`, the label, the two SDK levels,
//! the DER of the certificate the APK is currently signed with, and the v3
//! proof-of-rotation lineage when there is one.
//!
//! | structure | module | what is taken from it |
//! | :-- | :-- | :-- |
//! | zip end of central directory, central directory, local headers | `zip` | `AndroidManifest.xml` and `resources.arsc`, inflated |
//! | Android binary XML | `axml` | `<manifest>`, `<uses-sdk>`, `<application>` attributes |
//! | `resources.arsc` | `arsc` | a string reference's value in the default configuration |
//! | APK Signing Block | `signing` | the current signer's certificate, and the rotation lineage |
//!
//! # It reads and does not verify
//!
//! Android verifies an APK's signature when it installs it. What a publish
//! needs is **which key**, and a certificate's SHA-256 answers that without
//! any cryptography — `ADR-061` §4. No digest in the signing block is checked
//! and no CRC-32 in the zip is checked.
//!
//! # Every parse is bounded and refuses rather than guesses
//!
//! Every offset and length read from the file is checked against the buffer
//! it points into before it is used, every count is multiplied with overflow
//! checked, and a structure this reader does not understand — an unknown
//! compression method, a Zip64 archive, a lineage version other than 1 — is
//! an [`ApkError`](crate::apk_read::ApkError) naming the structure and the offset, never a fallback to a
//! different reading.

use sha2::{Digest as _, Sha256};

#[path = "apk_read_arsc.rs"]
mod arsc;
#[path = "apk_read_axml.rs"]
mod axml;
#[path = "apk_read_signing.rs"]
mod signing;
#[path = "apk_read_zip.rs"]
pub(crate) mod zip;

pub use signing::Scheme;

/// The manifest entry every APK carries.
const MANIFEST_ENTRY: &str = "AndroidManifest.xml";

/// The compiled resource table, which a label reference resolves through.
const RESOURCES_ENTRY: &str = "resources.arsc";

/// What [`read`] found in an APK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApkInfo {
    /// `<manifest package=…>`.
    pub application_id: String,
    /// Android's *long* version code: `versionCodeMajor` in the high 32 bits
    /// and `versionCode` in the low 32, which is the number Android compares
    /// when it decides whether an install is an upgrade. Equal to `versionCode`
    /// when the manifest carries no `versionCodeMajor`.
    pub version_code: i64,
    /// `android:versionCodeMajor`, when the manifest carries one.
    pub version_code_major: Option<u32>,
    pub version_name: String,
    /// `<application android:label>`, resolved through the resource table's
    /// default configuration when it is a reference.
    pub label: String,
    pub min_sdk: u32,
    pub target_sdk: u32,
    /// Which signature scheme's signer [`Self::certificate`] was taken from.
    pub scheme: Scheme,
    /// The DER of the certificate the APK is currently signed with.
    pub certificate: Vec<u8>,
    /// The v3 proof-of-rotation lineage, oldest certificate first and ending
    /// at [`Self::certificate`]; empty when the APK carries none.
    pub lineage: Vec<Vec<u8>>,
}

impl ApkInfo {
    /// Lowercase hex SHA-256 of [`Self::certificate`] — `apk.json`'s
    /// `signer_sha256`, and what `apksigner verify --print-certs` prints as
    /// the certificate's SHA-256 digest.
    #[must_use]
    pub fn signer_sha256(&self) -> String {
        sha256_hex(&self.certificate)
    }

    /// [`Self::lineage`]'s certificates, each as [`Self::signer_sha256`]
    /// names one, oldest first.
    #[must_use]
    pub fn lineage_sha256(&self) -> Vec<String> {
        self.lineage.iter().map(|der| sha256_hex(der)).collect()
    }
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::key::hex(&Sha256::digest(bytes))
}

/// Why an APK could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApkError {
    /// A structure runs past the end of the buffer that holds it.
    Truncated {
        structure: &'static str,
        offset: usize,
        needed: usize,
        available: usize,
    },
    /// A value this format does not allow where it was found.
    Malformed {
        structure: &'static str,
        offset: usize,
        detail: String,
    },
    /// No end-of-central-directory record: not a zip, or cut short.
    NoEndOfCentralDirectory,
    /// A Zip64 marker. An APK is never 4 GiB, so this reader does not read one.
    Zip64 { field: &'static str },
    /// An archive that spans disks.
    SpannedArchive,
    /// An entry with the encryption flag set.
    Encrypted { entry: String },
    /// A compression method other than stored (0) or deflated (8).
    UnknownCompression { entry: String, method: u16 },
    /// A deflated entry that does not inflate.
    Inflate { entry: String, detail: String },
    /// An entry that inflated to a different length than the directory says.
    InflatedSize {
        entry: String,
        declared: u64,
        actual: u64,
    },
    /// An entry the APK must carry and does not.
    MissingEntry { name: &'static str },
    /// An entry named twice in the central directory.
    DuplicateEntry { name: &'static str },
    /// An element the manifest must carry and does not.
    MissingElement { element: &'static str },
    /// An element this reader takes once, found twice.
    DuplicateElement { element: &'static str },
    /// An attribute the manifest must carry and does not.
    MissingAttribute {
        element: &'static str,
        attribute: &'static str,
    },
    /// An attribute whose value is not one this reader can use.
    BadAttribute {
        element: &'static str,
        attribute: &'static str,
        detail: String,
    },
    /// A resource reference that does not resolve to a string.
    UnresolvedReference { resource: u32, detail: String },
    /// No APK Signing Block before the central directory: an unsigned APK, or
    /// one signed with the v1 (JAR) scheme alone, which carries no signing
    /// block for this reader to take a certificate from.
    NoSigningBlock,
    /// A signing block with no v2, v3 or v3.1 signature in it.
    NoSigner,
    /// A signature block whose signers do not name one current certificate.
    SignerCount { scheme: Scheme, count: usize },
    /// A rotation lineage that does not end at the certificate the APK is
    /// signed with.
    LineageDoesNotEndAtSigner {
        signer_sha256: String,
        last_sha256: String,
    },
}

impl core::fmt::Display for ApkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { structure, offset, needed, available } => write!(
                f,
                "the {structure} is cut short: {needed} bytes are needed at offset {offset} and \
                 {available} are there"
            ),
            Self::Malformed { structure, offset, detail } => {
                write!(f, "the {structure} is malformed at offset {offset}: {detail}")
            }
            Self::NoEndOfCentralDirectory => write!(
                f,
                "there is no zip end-of-central-directory record: this is not a zip, or it is cut \
                 short"
            ),
            Self::Zip64 { field } => write!(
                f,
                "the zip's {field} carries a Zip64 marker, and an APK is never large enough to \
                 need one; this reader does not read Zip64"
            ),
            Self::SpannedArchive => write!(f, "the zip spans more than one disk"),
            Self::Encrypted { entry } => write!(f, "{entry} is encrypted"),
            Self::UnknownCompression { entry, method } => write!(
                f,
                "{entry} uses compression method {method}; an APK's entries are stored (0) or \
                 deflated (8)"
            ),
            Self::Inflate { entry, detail } => write!(f, "{entry} does not inflate: {detail}"),
            Self::InflatedSize { entry, declared, actual } => write!(
                f,
                "{entry} inflated to {actual} bytes and the central directory says {declared}"
            ),
            Self::MissingEntry { name } => write!(f, "the APK has no {name}"),
            Self::DuplicateEntry { name } => {
                write!(f, "the APK names {name} twice, and Android refuses an APK that does")
            }
            Self::MissingElement { element } => {
                write!(f, "AndroidManifest.xml has no <{element}> element")
            }
            Self::DuplicateElement { element } => {
                write!(f, "AndroidManifest.xml has more than one <{element}> element")
            }
            Self::MissingAttribute { element, attribute } => {
                write!(f, "<{element}> in AndroidManifest.xml has no {attribute}")
            }
            Self::BadAttribute { element, attribute, detail } => {
                write!(f, "<{element} {attribute}> in AndroidManifest.xml is {detail}")
            }
            Self::UnresolvedReference { resource, detail } => {
                write!(f, "resource 0x{resource:08x} does not resolve to a string: {detail}")
            }
            Self::NoSigningBlock => write!(
                f,
                "the APK has no APK Signing Block before its central directory — it is unsigned, \
                 or signed with the v1 (JAR) scheme alone, and the v2/v3 signer is what names its \
                 key"
            ),
            Self::NoSigner => {
                write!(f, "the APK Signing Block holds no v2, v3 or v3.1 signature")
            }
            Self::SignerCount { scheme, count } => write!(
                f,
                "the {scheme} signature names {count} signers where one current signer is needed"
            ),
            Self::LineageDoesNotEndAtSigner { signer_sha256, last_sha256 } => write!(
                f,
                "the rotation lineage ends at certificate {last_sha256}, and the APK is signed \
                 with {signer_sha256}"
            ),
        }
    }
}

/// Read what `apk.json` needs out of an APK's bytes — `ADR-061` §4.
///
/// # Errors
///
/// Any [`ApkError`]: a structure that is cut short or malformed, an entry
/// or attribute the APK must carry and does not, or a signature this reader
/// cannot name one current certificate from.
pub fn read(bytes: &[u8]) -> Result<ApkInfo, ApkError> {
    let archive = zip::Archive::open(bytes)?;
    let manifest_bytes = archive.read(MANIFEST_ENTRY)?;
    let manifest = axml::read_manifest(&manifest_bytes)?;

    // The resource table is read only if something needs resolving through
    // it, so an APK whose label is a literal string needs none.
    let mut resources: Option<Vec<u8>> = None;
    let mut resolve = |element: &'static str,
                       attribute: &'static str,
                       value: &axml::Value|
     -> Result<String, ApkError> {
        match value {
            axml::Value::Str(text) => Ok(text.clone()),
            axml::Value::Reference(id) => {
                if resources.is_none() {
                    resources = Some(archive.read(RESOURCES_ENTRY)?);
                }
                let table = resources.as_deref().unwrap_or_default();
                arsc::resolve_string(table, *id)
            }
            other => Err(ApkError::BadAttribute {
                element,
                attribute,
                detail: format!("{other}, where a string or a string reference is needed"),
            }),
        }
    };

    let version_name = resolve("manifest", "android:versionName", &manifest.version_name)?;
    let label = resolve("application", "android:label", &manifest.label)?;

    let min_sdk = match &manifest.min_sdk {
        Some(value) => sdk_level("android:minSdkVersion", value)?,
        // Android's own default for an absent `minSdkVersion`.
        None => 1,
    };
    let target_sdk = match &manifest.target_sdk {
        Some(value) => sdk_level("android:targetSdkVersion", value)?,
        // Android's own default: an absent `targetSdkVersion` is the minimum.
        None => min_sdk,
    };

    let version_code = (i64::from(manifest.version_code_major.unwrap_or(0)) << 32)
        | i64::from(manifest.version_code);

    let signing = signing::read(bytes, archive.central_directory_offset())?;

    Ok(ApkInfo {
        application_id: manifest.package,
        version_code,
        version_code_major: manifest.version_code_major,
        version_name,
        label,
        min_sdk,
        target_sdk,
        scheme: signing.scheme,
        certificate: signing.certificate,
        lineage: signing.lineage,
    })
}

/// An SDK level: an integer, or a decimal string of one. A codename (a
/// preview SDK's `minSdkVersion="Tiramisu"`) is refused — it names no number
/// a phone can compare.
fn sdk_level(attribute: &'static str, value: &axml::Value) -> Result<u32, ApkError> {
    match value {
        axml::Value::Int(n) => Ok(*n),
        axml::Value::Str(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().map_err(|_| ApkError::BadAttribute {
                element: "uses-sdk",
                attribute,
                detail: format!("{text:?}, which is not a 32-bit number"),
            })
        }
        other => Err(ApkError::BadAttribute {
            element: "uses-sdk",
            attribute,
            detail: format!("{other}, where an SDK level number is needed"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Bounded reads, shared by every structure above.
// ---------------------------------------------------------------------------

/// `len` bytes of `data` at `offset`, or [`ApkError::Truncated`] naming
/// `structure`.
fn slice<'a>(
    data: &'a [u8],
    offset: usize,
    len: usize,
    structure: &'static str,
) -> Result<&'a [u8], ApkError> {
    offset
        .checked_add(len)
        .and_then(|end| data.get(offset..end))
        .ok_or(ApkError::Truncated {
            structure,
            offset,
            needed: len,
            available: data.len().saturating_sub(offset),
        })
}

fn u8_at(data: &[u8], offset: usize, structure: &'static str) -> Result<u8, ApkError> {
    Ok(slice(data, offset, 1, structure)?[0])
}

fn u16_at(data: &[u8], offset: usize, structure: &'static str) -> Result<u16, ApkError> {
    let b = slice(data, offset, 2, structure)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(data: &[u8], offset: usize, structure: &'static str) -> Result<u32, ApkError> {
    let b = slice(data, offset, 4, structure)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(data: &[u8], offset: usize, structure: &'static str) -> Result<u64, ApkError> {
    let b = slice(data, offset, 8, structure)?;
    Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// A length read from the file, as a `usize`, or [`ApkError::Malformed`].
fn to_usize(value: u64, structure: &'static str, offset: usize) -> Result<usize, ApkError> {
    usize::try_from(value).map_err(|_| ApkError::Malformed {
        structure,
        offset,
        detail: format!("{value} does not fit this machine's address space"),
    })
}

/// `a + b`, or [`ApkError::Malformed`] naming the overflow.
fn add(a: usize, b: usize, structure: &'static str, offset: usize) -> Result<usize, ApkError> {
    a.checked_add(b).ok_or_else(|| ApkError::Malformed {
        structure,
        offset,
        detail: format!("{a} + {b} overflows"),
    })
}

/// `a * b`, or [`ApkError::Malformed`] naming the overflow.
fn mul(a: usize, b: usize, structure: &'static str, offset: usize) -> Result<usize, ApkError> {
    a.checked_mul(b).ok_or_else(|| ApkError::Malformed {
        structure,
        offset,
        detail: format!("{a} * {b} overflows"),
    })
}

#[cfg(test)]
#[path = "apk_read_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "apk_read_fixture_tests.rs"]
mod fixture_tests;
