//! **Phones in the fleet register** — `ADR-061` §5.
//!
//! ```text
//! <register>/phones/<identity>.toml    one file per phone, filed by its identity
//! ```
//!
//! # A separate accessor, and never a field of `Register`
//!
//! [`crate::Register`] is what the workstation tool, provisioning's staging and
//! the fleet repository's projection read and write, and [`crate::store::save`]
//! rewrites every file it holds. Phones belong to the fleet server's register
//! only (`ADR-061` §5): a workstation register holds none and must never grow a
//! `phones/` directory. Keeping phones out of `Register` means no existing
//! caller's load, render or save can see, write or drop a phone file, so what
//! each of them writes is unchanged by this module's existence.
//!
//! # One file per phone, and why that needs no lock
//!
//! Enroll, rename and revoke each touch exactly one file: a write replaces it
//! atomically through the same durable sequence every other register file
//! takes, and a revoke removes it. A list is a directory read. So two writers of
//! this directory stay safe without coordinating, provided neither keeps a copy.
//!
//! # The identity names a path, so its grammar is checked first
//!
//! An identity is base64url, unpadded, of a SHA-256: 43 characters of
//! `A-Z a-z 0-9 - _`. Nothing in that alphabet is a separator or a dot, so an
//! identity that passes [`is_identity`] can only ever name a file directly
//! inside `phones/`. Every path here is built from a checked identity.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::{durably, io};
use crate::{RECORD_SUFFIX, RegisterError};

/// The directory phone records live in, under the register root.
pub const PHONES_DIR: &str = "phones";

/// How long an identity is: base64url, unpadded, of 32 bytes.
pub const IDENTITY_CHARS: usize = 43;

/// The most characters a label may hold.
pub const LABEL_MAX_CHARS: usize = 64;

/// One phone, as the register records it — `ADR-061` §5's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    /// base64url, unpadded, of the SHA-256 of [`Self::public_key`]. The file
    /// name, and derived by whoever enrolls the phone rather than supplied.
    pub identity: String,
    /// The device model, or a name a person gave it.
    pub label: String,
    /// The DER SubjectPublicKeyInfo of the phone's key.
    pub public_key: Vec<u8>,
    /// When it was enrolled: RFC 3339, UTC, to the second, with a `Z` suffix.
    pub enrolled: String,
}

/// The record as it is written: the key as standard padded base64, because a
/// TOML file holds text.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    identity: String,
    label: String,
    public_key: String,
    enrolled: String,
}

/// What went wrong with a phone record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneError {
    /// The path could not be read or written, and what the system said.
    Io { path: PathBuf, detail: String },
    /// A file under `phones/` is not a phone record this crate can read.
    Malformed { path: PathBuf, detail: String },
    /// A record's `identity` and its file name disagree.
    Misfiled {
        path: PathBuf,
        says: String,
        filed_as: String,
    },
    /// No phone is enrolled under that identity.
    NoSuchPhone { identity: String },
    /// Not 43 characters of the base64url alphabet.
    BadIdentity { identity: String },
    /// Empty, too long, or holding a control character.
    BadLabel { label: String, why: &'static str },
    /// The enrollment time is not RFC 3339 UTC to the second.
    BadEnrolled { enrolled: String },
}

impl core::fmt::Display for PhoneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Malformed { path, detail } => write!(
                f,
                "{} is not a phone record this register can read: {detail}",
                path.display()
            ),
            Self::Misfiled {
                path,
                says,
                filed_as,
            } => write!(
                f,
                "{} says its identity is {says:?} but it is filed as {filed_as:?}. A phone is \
                 filed under its identity, so the two disagreeing means the file was renamed by \
                 hand. Nothing has been changed.",
                path.display()
            ),
            Self::NoSuchPhone { identity } => {
                write!(f, "no phone is enrolled under the identity {identity:?}")
            }
            Self::BadIdentity { identity } => write!(
                f,
                "{identity:?} is not a phone identity: one is {IDENTITY_CHARS} characters of the \
                 base64url alphabet (A-Z, a-z, 0-9, - and _)"
            ),
            Self::BadLabel { label, why } => {
                write!(f, "{label:?} is not a label a phone may carry: {why}")
            }
            Self::BadEnrolled { enrolled } => write!(
                f,
                "{enrolled:?} is not an enrollment time: one is RFC 3339 UTC to the second, such \
                 as 2000-01-01T00:00:00Z"
            ),
        }
    }
}

impl From<RegisterError> for PhoneError {
    fn from(error: RegisterError) -> Self {
        match error {
            RegisterError::Io { path, detail } => Self::Io { path, detail },
            RegisterError::Malformed { path, detail } => Self::Malformed { path, detail },
            other => Self::Io {
                path: PathBuf::new(),
                detail: other.to_string(),
            },
        }
    }
}

impl Phone {
    /// A phone record, every field checked.
    ///
    /// # Errors
    ///
    /// [`PhoneError::BadIdentity`], [`PhoneError::BadLabel`] or
    /// [`PhoneError::BadEnrolled`], for the first field that fails its rule.
    pub fn new(
        identity: &str,
        label: &str,
        public_key: Vec<u8>,
        enrolled: &str,
    ) -> Result<Self, PhoneError> {
        if !is_identity(identity) {
            return Err(PhoneError::BadIdentity {
                identity: identity.to_owned(),
            });
        }
        check_label(label)?;
        if !is_enrolled_time(enrolled) {
            return Err(PhoneError::BadEnrolled {
                enrolled: enrolled.to_owned(),
            });
        }
        Ok(Self {
            identity: identity.to_owned(),
            label: label.to_owned(),
            public_key,
            enrolled: enrolled.to_owned(),
        })
    }
}

/// Whether `text` is an identity this register files phones under.
#[must_use]
pub fn is_identity(text: &str) -> bool {
    text.len() == IDENTITY_CHARS
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The identity of a key whose DER SubjectPublicKeyInfo hashes to `sha256`:
/// base64url, unpadded.
///
/// The hash is the caller's, so this crate carries no digest; the encoding is
/// here so that every writer of a phone file spells an identity the same way.
#[must_use]
pub fn identity_from_sha256(sha256: &[u8; 32]) -> String {
    rivet_core::base64::encode(sha256)
        .chars()
        .filter(|c| *c != '=')
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            other => other,
        })
        .collect()
}

/// Refuse a label that is empty, longer than [`LABEL_MAX_CHARS`] characters, or
/// holds a control character.
///
/// # Errors
///
/// [`PhoneError::BadLabel`], saying which rule it broke.
pub fn check_label(label: &str) -> Result<(), PhoneError> {
    let why = if label.is_empty() {
        Some("it is empty")
    } else if label.chars().count() > LABEL_MAX_CHARS {
        Some("it is longer than 64 characters")
    } else if label.chars().any(char::is_control) {
        Some("it holds a control character")
    } else {
        None
    };
    why.map_or(Ok(()), |why| {
        Err(PhoneError::BadLabel {
            label: label.to_owned(),
            why,
        })
    })
}

/// Whether `text` is `YYYY-MM-DDTHH:MM:SSZ`.
fn is_enrolled_time(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 20
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'Z',
            _ => c.is_ascii_digit(),
        })
}

/// Where the phone records of the register at `root` are.
#[must_use]
pub fn phones_dir(root: &Path) -> PathBuf {
    root.join(PHONES_DIR)
}

/// Where one phone's record is, or would be. `None` for an identity that is
/// not one, so no unchecked text is ever joined onto a path.
#[must_use]
pub fn record_path(root: &Path, identity: &str) -> Option<PathBuf> {
    is_identity(identity).then(|| phones_dir(root).join(format!("{identity}{RECORD_SUFFIX}")))
}

/// The bytes a phone's file holds.
///
/// # Errors
///
/// [`PhoneError::Malformed`] if the record will not render, which can only
/// happen if a field stops being representable in TOML.
pub fn render(phone: &Phone, path: &Path) -> Result<String, PhoneError> {
    let record = Record {
        identity: phone.identity.clone(),
        label: phone.label.clone(),
        public_key: rivet_core::base64::encode(&phone.public_key),
        enrolled: phone.enrolled.clone(),
    };
    toml_edit::ser::to_string_pretty(&record).map_err(|e| PhoneError::Malformed {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

/// Read one phone record's text, refusing one not filed under its identity.
///
/// # Errors
///
/// [`PhoneError::Malformed`] for text that is not a record or a field that
/// fails its rule, and [`PhoneError::Misfiled`] when the identity and the file
/// name disagree.
pub fn read_record(path: &Path, text: &str) -> Result<Phone, PhoneError> {
    let malformed = |detail: String| PhoneError::Malformed {
        path: path.to_path_buf(),
        detail,
    };
    let record: Record = toml_edit::de::from_str(text).map_err(|e| malformed(e.to_string()))?;
    let filed_as = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if record.identity != filed_as {
        return Err(PhoneError::Misfiled {
            path: path.to_path_buf(),
            says: record.identity,
            filed_as,
        });
    }
    let public_key = rivet_core::base64::decode(&record.public_key)
        .ok_or_else(|| malformed("public_key is not standard padded base64".to_owned()))?;
    Phone::new(&record.identity, &record.label, public_key, &record.enrolled)
        .map_err(|e| malformed(e.to_string()))
}

/// Every phone the register at `root` holds, sorted by identity.
///
/// **A register with no `phones/` directory is a register with no phones**, so
/// every register written before phones existed reads unchanged.
///
/// # Errors
///
/// [`PhoneError::Io`] for a directory or file that exists and will not be read,
/// and [`read_record`]'s refusals for a file that is not a phone record. A file
/// not ending in `.toml` is left alone.
pub fn list(root: &Path) -> Result<Vec<Phone>, PhoneError> {
    let dir = phones_dir(root);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(&dir, &e).into()),
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| PhoneError::from(io(&dir, &e)))?;
        let path = entry.path();
        if path.extension().is_some_and(|e| {
            RECORD_SUFFIX
                .strip_prefix('.')
                .is_some_and(|want| e.eq_ignore_ascii_case(want))
        }) {
            paths.push(path);
        }
    }
    let mut phones = Vec::with_capacity(paths.len());
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|e| PhoneError::from(io(&path, &e)))?;
        phones.push(read_record(&path, &text)?);
    }
    phones.sort_by(|a, b| a.identity.cmp(&b.identity));
    Ok(phones)
}

/// The phone enrolled under `identity`.
///
/// # Errors
///
/// [`PhoneError::BadIdentity`] for an identity that is not one,
/// [`PhoneError::NoSuchPhone`] when none is enrolled under it, and
/// [`read_record`]'s refusals.
pub fn read(root: &Path, identity: &str) -> Result<Phone, PhoneError> {
    let path = record_path(root, identity).ok_or_else(|| PhoneError::BadIdentity {
        identity: identity.to_owned(),
    })?;
    match fs::read_to_string(&path) {
        Ok(text) => read_record(&path, &text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(PhoneError::NoSuchPhone {
            identity: identity.to_owned(),
        }),
        Err(e) => Err(io(&path, &e).into()),
    }
}

/// Write one phone's record, creating `phones/` if it is absent and replacing
/// any record already filed under that identity, atomically.
///
/// # Errors
///
/// [`Phone::new`]'s refusals for a record whose fields break a rule, and
/// [`PhoneError::Io`] for anything the filesystem refuses.
pub fn write(root: &Path, phone: &Phone) -> Result<PathBuf, PhoneError> {
    let checked = Phone::new(
        &phone.identity,
        &phone.label,
        phone.public_key.clone(),
        &phone.enrolled,
    )?;
    let path = record_path(root, &checked.identity).ok_or_else(|| PhoneError::BadIdentity {
        identity: checked.identity.clone(),
    })?;
    let text = render(&checked, &path)?;
    let dir = phones_dir(root);
    fs::create_dir_all(&dir).map_err(|e| PhoneError::from(io(&dir, &e)))?;
    durably(&path, text.as_bytes())?;
    Ok(path)
}

/// Remove the phone enrolled under `identity`.
///
/// **A phone that is not there is a named error**, unlike removing a device:
/// the caller that asked to revoke it holds a view of the list that is stale,
/// and saying so is how it learns.
///
/// # Errors
///
/// [`PhoneError::BadIdentity`], [`PhoneError::NoSuchPhone`], or
/// [`PhoneError::Io`] for a file that exists and will not be removed.
pub fn remove(root: &Path, identity: &str) -> Result<(), PhoneError> {
    let path = record_path(root, identity).ok_or_else(|| PhoneError::BadIdentity {
        identity: identity.to_owned(),
    })?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(PhoneError::NoSuchPhone {
            identity: identity.to_owned(),
        }),
        Err(e) => Err(io(&path, &e).into()),
    }
}

#[cfg(test)]
#[path = "phones_tests.rs"]
mod tests;
