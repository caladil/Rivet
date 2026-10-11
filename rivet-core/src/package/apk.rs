//! What the metadata means for the `apk` kind — Gantry, `ADR-061` §3.
//!
//! An `apk` artifact is an Android APK, signed and carried through a fleet
//! repository (`ADR-052` §7) so `gantry` can hand it to a phone —
//! `ADR-061` §3. It has no meaning on a Rivet machine, so this module
//! interprets it and nothing installs it.
//!
//! # The whole of what this kind is — `ADR-029` §11's claim, made a sixth
//! time
//!
//! One token ([`crate::package::kind::Kind::apk`]), one interpretation module
//! (this one) and one installer branch that refuses it. Nothing here asks the
//! verifier, the builder or the container's layout to change.
//!
//! # The header — `ADR-061` §3
//!
//! | field | value |
//! | :-- | :-- |
//! | `platform` | `"android"`, always |
//! | `arch` | `"any"`, always — an APK runs on whatever the phone is |
//!
//! Both are checked in [`Apk::open`], because a header that carries neither
//! is not the artifact `ADR-061` describes, whatever its members hold.
//!
//! # The members — `ADR-061` §3
//!
//! | member | required | stored |
//! | :-- | :-- | :-- |
//! | [`APK_NAME`] | yes | **uncompressed** — an APK is already a zip of \
//!   compressed entries |
//! | [`METADATA_NAME`] | yes | yes |
//! | [`ICON_NAME`] | no | yes |
//!
//! **Any other member name is refused** — `PackageError::ApkForeignMember`.
//! Unlike a library's `soname` list or an application's assets, an apk
//! artifact's members are three fixed names and nothing else, so this module
//! checks names directly rather than a target-class byte a builder chose.
//!
//! `app.apk` being stored rather than compressed is enforced twice: this
//! module never has to check it, because [`crate::package::read::open`]
//! already refuses any non-`card` member whose `compression` is not
//! [`crate::package::read::COMPRESSION_NONE`] — `ADR-029` §13/R-107. An apk
//! artifact asks nothing new of that rule; it is simply a kind the rule
//! already covers.
//!
//! # `apk.json` — `ADR-061` §3
//!
//! Every key is read from the APK at build time, none typed by hand. See
//! [`Metadata`] for the eight keys and [`Metadata::parse`] for what is
//! checked. `rivet-core` has no dependencies (`ADR-014` §3), so this reads
//! JSON with [`crate::json`] — the hand-written reader `ADR-032` §4 already
//! put in this crate for JSON-RPC — rather than adding one.

use crate::json::Json;
use crate::package::error::PackageError;
use crate::package::kind::Kind;
use crate::package::read::{Member, Package, VerifiedMember};

/// The APK itself, byte for byte as the build produced it.
pub const APK_NAME: &str = "app.apk";

/// [`Metadata`]'s canonical bytes.
pub const METADATA_NAME: &str = "apk.json";

/// The launcher icon at 192x192, optional — `ADR-061` §3.
pub const ICON_NAME: &str = "icon.png";

/// Every member name an `apk` artifact may carry. Anything else is
/// [`PackageError::ApkForeignMember`].
const MEMBER_NAMES: [&str; 3] = [APK_NAME, METADATA_NAME, ICON_NAME];

/// The platform every `apk` header must carry — `ADR-061` §3.
const PLATFORM: &str = "android";

/// The arch every `apk` header must carry — `ADR-061` §3.
const ARCH: &str = "any";

/// The longest a string value in [`Metadata`] may be.
///
/// [`crate::package::application::MAX_VALUE`] is this format's existing bound
/// for a manifest string field, and an application label or an
/// `application_id` is exactly that shape of value.
const MAX_VALUE: usize = crate::package::application::MAX_VALUE;

/// The exact length of [`Metadata::signer_sha256`] — 32 bytes of SHA-256, as
/// lowercase hex.
const SIGNER_HEX_LEN: usize = 64;

/// A [`Package`] read as an `apk`.
///
/// Holding one means the kind is `apk`, the header's `platform` is
/// `"android"` and its `arch` is `"any"`, and every member is one of
/// [`APK_NAME`], [`METADATA_NAME`] or an optional [`ICON_NAME`]. It does
/// **not** mean any member's content has been checked — that is
/// [`Package::verify_member`], per member, at the point it is about to be
/// used, exactly as for every other kind.
#[derive(Debug, PartialEq, Eq)]
pub struct Apk<'a> {
    package: &'a Package,
    apk_member: &'a Member,
    metadata_member: &'a Member,
    icon_member: Option<&'a Member>,
}

impl<'a> Apk<'a> {
    pub fn open(package: &'a Package) -> Result<Self, PackageError> {
        package.require_kind(&Kind::apk())?;

        match package.header().platform() {
            Some(PLATFORM) => {}
            other => {
                return Err(PackageError::ApkWrongPlatform {
                    found: other.map(ToString::to_string),
                });
            }
        }
        if package.header().arch() != ARCH {
            return Err(PackageError::ApkWrongArch {
                found: package.header().arch().to_string(),
            });
        }

        let mut apk_member: Option<&Member> = None;
        let mut metadata_member: Option<&Member> = None;
        let mut icon_member: Option<&Member> = None;

        for member in package.members() {
            if !MEMBER_NAMES.contains(&member.name()) {
                return Err(PackageError::ApkForeignMember {
                    name: member.name().to_string(),
                });
            }
            let (slot, static_name): (&mut Option<&Member>, &'static str) = match member.name() {
                APK_NAME => (&mut apk_member, APK_NAME),
                METADATA_NAME => (&mut metadata_member, METADATA_NAME),
                _ => (&mut icon_member, ICON_NAME),
            };
            if slot.is_some() {
                return Err(PackageError::ApkMemberCount {
                    name: static_name,
                    found: 2,
                });
            }
            *slot = Some(member);
        }

        let apk_member = apk_member.ok_or(PackageError::ApkMemberCount {
            name: APK_NAME,
            found: 0,
        })?;
        let metadata_member = metadata_member.ok_or(PackageError::ApkMemberCount {
            name: METADATA_NAME,
            found: 0,
        })?;

        Ok(Self {
            package,
            apk_member,
            metadata_member,
            icon_member,
        })
    }

    #[must_use]
    pub fn package(&self) -> &'a Package {
        self.package
    }

    #[must_use]
    pub fn apk_member(&self) -> &'a Member {
        self.apk_member
    }

    #[must_use]
    pub fn metadata_member(&self) -> &'a Member {
        self.metadata_member
    }

    #[must_use]
    pub fn icon_member(&self) -> Option<&'a Member> {
        self.icon_member
    }

    /// Parse `apk.json` — **and only from bytes that have been verified.**
    ///
    /// The same discipline [`crate::package::application::Application::read_manifest`]
    /// and [`crate::package::library::Library`]'s manifest reading hold to:
    /// the [`VerifiedMember`] cannot be constructed outside
    /// [`crate::package::read::MemberStream::finish`], so a caller cannot read
    /// `apk.json` before its digest has checked out.
    pub fn read_metadata(
        &self,
        verified: VerifiedMember<'_>,
        bytes: &[u8],
    ) -> Result<Metadata, PackageError> {
        if verified.member().index() != self.metadata_member.index() {
            return Err(PackageError::WrongMemberVerified {
                expected: self.metadata_member.index(),
                found: verified.member().index(),
            });
        }
        Metadata::parse(bytes).map_err(|fault| PackageError::MalformedApkMetadata { fault })
    }
}

/// `apk.json`'s typed metadata — `ADR-061` §3, every key read from the APK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    application_id: String,
    version_code: i64,
    version_name: String,
    label: String,
    min_sdk: i64,
    target_sdk: i64,
    signer_sha256: String,
    development: bool,
}

/// Why `apk.json` could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataFault {
    NotText,
    NotJson,
    NotObject,
    UnknownKey { key: String },
    MissingKey { key: &'static str },
    BadValue { key: &'static str },
}

impl core::fmt::Display for MetadataFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotText => write!(f, "it is not printable UTF-8 text"),
            Self::NotJson => write!(f, "it is not a JSON document"),
            Self::NotObject => write!(f, "the document is not a JSON object"),
            Self::UnknownKey { key } => write!(f, "{key:?} is a key this build does not define"),
            Self::MissingKey { key } => write!(f, "{key:?} is required and is absent"),
            Self::BadValue { key } => write!(f, "{key:?} does not hold a value it may hold"),
        }
    }
}

impl Metadata {
    #[must_use]
    pub fn application_id(&self) -> &str {
        &self.application_id
    }
    #[must_use]
    pub fn version_code(&self) -> i64 {
        self.version_code
    }
    #[must_use]
    pub fn version_name(&self) -> &str {
        &self.version_name
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
    #[must_use]
    pub fn min_sdk(&self) -> i64 {
        self.min_sdk
    }
    #[must_use]
    pub fn target_sdk(&self) -> i64 {
        self.target_sdk
    }
    #[must_use]
    pub fn signer_sha256(&self) -> &str {
        &self.signer_sha256
    }
    #[must_use]
    pub fn development(&self) -> bool {
        self.development
    }

    /// Read `apk.json`'s bytes back — `ADR-061` §3's eight keys, every one
    /// required and none inferred.
    ///
    /// **Every top-level key is checked against the closed set below** —
    /// `ADR-006` §6's closed-vocabulary rule, applied to the one manifest
    /// in this crate that is JSON rather than this format's usual key/value
    /// text. A key this build does not define is refused rather than
    /// ignored, so a typo in a future writer is found here instead of
    /// silently carrying no weight.
    pub fn parse(bytes: &[u8]) -> Result<Self, MetadataFault> {
        let text = core::str::from_utf8(bytes).map_err(|_| MetadataFault::NotText)?;
        let json = crate::json::parse(text).map_err(|_| MetadataFault::NotJson)?;
        let Json::Object(fields) = &json else {
            return Err(MetadataFault::NotObject);
        };
        for (key, _) in fields {
            if !KEYS.contains(&key.as_str()) {
                return Err(MetadataFault::UnknownKey { key: key.clone() });
            }
        }

        let application_id = string_field(&json, "application_id")?;
        let version_code = nonneg_int_field(&json, "version_code")?;
        let version_name = string_field(&json, "version_name")?;
        let label = string_field(&json, "label")?;
        let min_sdk = nonneg_int_field(&json, "min_sdk")?;
        let target_sdk = nonneg_int_field(&json, "target_sdk")?;
        let signer_sha256 = string_field(&json, "signer_sha256")?;
        if !is_lowercase_hex(&signer_sha256, SIGNER_HEX_LEN) {
            return Err(MetadataFault::BadValue {
                key: "signer_sha256",
            });
        }
        let development = bool_field(&json, "development")?;

        Ok(Self {
            application_id,
            version_code,
            version_name,
            label,
            min_sdk,
            target_sdk,
            signer_sha256,
            development,
        })
    }
}

/// Every key [`Metadata::parse`] reads. `ADR-061` §3's table, in the order
/// it lists them.
const KEYS: [&str; 8] = [
    "application_id",
    "version_code",
    "version_name",
    "label",
    "min_sdk",
    "target_sdk",
    "signer_sha256",
    "development",
];

fn string_field(json: &Json, key: &'static str) -> Result<String, MetadataFault> {
    match json.get(key) {
        None => Err(MetadataFault::MissingKey { key }),
        Some(value) => match value.as_str() {
            Some(text) if !text.is_empty() && text.len() <= MAX_VALUE => Ok(text.to_string()),
            _ => Err(MetadataFault::BadValue { key }),
        },
    }
}

fn nonneg_int_field(json: &Json, key: &'static str) -> Result<i64, MetadataFault> {
    match json.get(key) {
        None => Err(MetadataFault::MissingKey { key }),
        Some(value) => match value.as_integer() {
            Some(n) if n >= 0 => Ok(n),
            _ => Err(MetadataFault::BadValue { key }),
        },
    }
}

fn bool_field(json: &Json, key: &'static str) -> Result<bool, MetadataFault> {
    match json.get(key) {
        None => Err(MetadataFault::MissingKey { key }),
        Some(value) => value.as_bool().ok_or(MetadataFault::BadValue { key }),
    }
}

fn is_lowercase_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests;
