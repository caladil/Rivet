//! `[volumes.<name>]` — the storage a machine's applications live on.
//! `ADR-031` §7.1, ruled 2026-08-25.
//!
//! ```toml
//! [volumes.media]
//! guid = "9f2c1e40-7b3a-4c58-9d61-2ab7e0c4f183"
//! ```
//!
//! # One key, and the three that are deliberately absent
//!
//! **There is no `device`, no `path` and no `label`**, and `ADR-031` §7.1 names
//! why: each would be a second authority for *which storage this is*, and it is
//! the one a tired operator reaches for. A device path is not stable across a
//! reboot, a label is writable by anything that can write the filesystem, and
//! either would silently win over the GUID on the day they disagreed.
//!
//! §4 makes the GUID the only identifier: it is in the partition table, it is
//! assigned once when the volume is made, and it survives the device moving to
//! another port, another reader, or another machine.
//!
//! # What refusing an unknown key buys, which is the whole safety argument
//!
//! `ADR-031` §7.2, and it was **measured** rather than asserted: `ADR-018`
//! §5.3.3's deliberate fault put one unknown key in a `rivet.toml` and
//! produced a completely healthy board whose only symptom was a line that did
//! not appear.
//!
//! Two consequences here, and the second is the one that matters:
//!
//! 1. **A machine cannot acquire a volume by a typo.** `[volume.media]` is
//!    refused at parse time and named.
//! 2. **A machine running an older image refuses a document that declares
//!    volumes**, rather than booting healthy with half its applications missing
//!    and saying nothing. Ignoring an unknown table would produce exactly
//!    `ADR-031` §6's failure with no message at all.
//!
//! # This crate performs no I/O and cannot know whether a volume is there
//!
//! `ADR-014` §3.1. A declaration is a statement about what the operator wants
//! attached, not a claim that anything is plugged in — `ADR-031` §7.5 is
//! explicit that declaring a volume does not create one. Whether the GUID
//! resolves is the machine's answer, at the moment it is asked, and
//! `ADR-031` §6 is what happens when it does not.

use crate::error::ConfigError;

/// The name of the built-in volume, which a declaration may not take.
///
/// `/state` is a volume already — `ADR-031` §2.1 makes it *the built-in one*
/// rather than a special case — so a declared `[volumes.state]` would be two
/// answers to one name, which is `ADR-011` §3's failure exactly.
pub const BUILT_IN: &str = "state";

/// The length of the full 36-character GUID form, `ADR-031` §4.1.
pub const GUID_TEXT_LEN: usize = 36;

/// Where the hyphens sit in that form.
const HYPHENS_AT: [usize; 4] = [8, 13, 18, 23];

/// A volume's name: a single path component, and not the built-in one.
///
/// **Checked at construction**, so a `VolumeName` that exists is one a path may
/// be built from. That is the same guarantee `rivet_core::service::ServiceName`
/// gives, and it is here for the sharper version of the same reason: this name
/// becomes a directory under which an application's programs are found and run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VolumeName(String);

impl VolumeName {
    /// Check a name, or say which rule it broke.
    ///
    /// The rules are `ADR-031` §7.1's, and the last is the one worth stating:
    /// a name that is well-formed but already *means* something is refused too,
    /// which is `ADR-015` §7.1.1's clause applied one level up.
    pub fn new(text: &str) -> Result<Self, VolumeFault> {
        if text.is_empty() {
            return Err(VolumeFault::EmptyName);
        }
        if text == BUILT_IN {
            return Err(VolumeFault::BuiltInName);
        }
        // A single component: no separator, no traversal, no leading dot. The
        // first two are what stop a name reaching outside its parent; the third
        // is what stops one hiding from an `ls`.
        if text.contains('/') || text.contains('\\') {
            return Err(VolumeFault::NotOneComponent {
                name: text.to_string(),
            });
        }
        if text == "." || text == ".." || text.starts_with('.') {
            return Err(VolumeFault::NotOneComponent {
                name: text.to_string(),
            });
        }
        if !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(VolumeFault::UnusableName {
                name: text.to_string(),
            });
        }
        if !text.as_bytes()[0].is_ascii_alphanumeric() {
            return Err(VolumeFault::UnusableName {
                name: text.to_string(),
            });
        }
        Ok(Self(text.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for VolumeName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A partition GUID, in `ADR-031` §4.1's full 36-character form.
///
/// **Held as the text the operator wrote, lowercased**, rather than as sixteen
/// bytes. This crate's job is to say whether the document is well-formed; the
/// byte order a GPT stores it in is `rivet-adapter-image`'s business and
/// `ADR-031` §4.1 warns that extending the existing reader *literally* is the
/// trap — the first three fields are little-endian and the last two are not.
/// Keeping the text here means this crate cannot get that wrong, because it
/// never does the conversion.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Guid(String);

impl Guid {
    /// The whole rule, for a diagnostic to quote without quoting the input.
    ///
    /// **The sentence `rivet get schema` publishes and the sentence a
    /// refusal quotes are this one constant.** A schema describing a value
    /// differently from the error the operator gets is exactly the "trusted
    /// and wrong" document `ADR-006` §7 refuses.
    pub const RULE: &'static str =
        "a GUID is the full 36-character form with hyphens, as `blkid` and `sgdisk` print it:  \
         eight hex digits, a hyphen, three groups of four, a hyphen, then twelve";

    /// Check the form, or say what is wrong with it.
    pub fn new(text: &str) -> Result<Self, VolumeFault> {
        if text.len() != GUID_TEXT_LEN {
            return Err(VolumeFault::GuidLength {
                guid: text.to_string(),
                len: text.len(),
            });
        }
        for (at, byte) in text.bytes().enumerate() {
            let expected_hyphen = HYPHENS_AT.contains(&at);
            let ok = if expected_hyphen {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            };
            if !ok {
                return Err(VolumeFault::GuidShape {
                    guid: text.to_string(),
                    at,
                });
            }
        }
        // **Lowercased once, here.** A GUID is compared against one read off a
        // partition table, and two spellings of one identifier is `ADR-011`'s
        // failure in miniature — the comparison would depend on which end
        // happened to normalise.
        Ok(Self(text.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Guid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One declared volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    name: VolumeName,
    guid: Guid,
}

impl Volume {
    /// Every key a volume entry may declare. One list, read by the parser and
    /// by the refusal it produces (`ADR-011`).
    pub const DECLARED_KEYS: &'static [&'static str] = &["guid"];

    pub(crate) const fn new(name: VolumeName, guid: Guid) -> Self {
        Self { name, guid }
    }

    #[must_use]
    pub const fn name(&self) -> &VolumeName {
        &self.name
    }

    /// The only identifier. See this module's documentation for the three keys
    /// that deliberately do not exist beside it.
    #[must_use]
    pub const fn guid(&self) -> &Guid {
        &self.guid
    }
}

/// Why a volume declaration is not one.
///
/// Closed, per `ADR-006` §6.4, and each variant names *a distinct thing the
/// operator got wrong* rather than a place in this file where a check happened.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VolumeFault {
    EmptyName,
    /// The name is `state`, which is the built-in volume's.
    BuiltInName,
    /// A separator, a traversal, or a leading dot.
    NotOneComponent { name: String },
    /// Well-formed as a string and not as a name.
    UnusableName { name: String },
    GuidLength { guid: String, len: usize },
    GuidShape { guid: String, at: usize },
    /// Two declarations naming one GUID. Refused because the machine would
    /// then have two names for one device and `ADR-031` §4 makes the GUID the
    /// identity.
    DuplicateGuid { guid: String, names: (String, String) },
}

impl VolumeFault {
    /// One sentence an operator can act on.
    pub fn why(&self) -> String {
        match self {
            Self::EmptyName => {
                "a volume's name is the table's own key and this one is empty".to_string()
            }
            Self::BuiltInName => format!(
                "`{BUILT_IN}` is the built-in volume (ADR-031 §2.1) and cannot be declared \
                 again — a second thing called `{BUILT_IN}` would be two answers to one name. \
                 /state needs no declaration; it is the volume a Rivet always has"
            ),
            Self::NotOneComponent { name } => format!(
                "{name:?} is not a single path component. A volume's name becomes a directory \
                 under which programs are found and run, so a separator, a `..` or a leading \
                 dot is refused rather than sanitised"
            ),
            Self::UnusableName { name } => format!(
                "{name:?} is not a usable volume name: letters, digits, `-` and `_`, starting \
                 with a letter or a digit"
            ),
            Self::GuidLength { guid, len } => format!(
                "{guid:?} is {len} characters and a GUID is {GUID_TEXT_LEN} — the full form \
                 with hyphens, as `blkid` and `sgdisk` print it (ADR-031 §4.1)"
            ),
            Self::GuidShape { guid, at } => format!(
                "{guid:?} is not a GUID: byte {at} is wrong. The form is eight hex digits, a \
                 hyphen, three groups of four, a hyphen, then twelve"
            ),
            Self::DuplicateGuid { guid, names } => format!(
                "volumes {:?} and {:?} both declare {guid:?}. A GUID is the identity of one \
                 device (ADR-031 §4), so two names for it would make which one holds an \
                 application depend on the order they were read in",
                names.0, names.1
            ),
        }
    }

    /// Whether this is a complaint about the table's **name** or about a
    /// **value** inside it.
    ///
    /// **`ConfigError` is a closed vocabulary** (`ADR-006` §6.4) and this
    /// module does not grow it: a volume fault is one of the two mistakes the
    /// document can already make, and saying which lets an operator look in the
    /// right place. A name fault points at `[volumes.<name>]`; a value fault
    /// points at the `guid` line under it.
    pub const fn is_about_the_name(&self) -> bool {
        matches!(
            self,
            Self::EmptyName
                | Self::BuiltInName
                | Self::NotOneComponent { .. }
                | Self::UnusableName { .. }
                | Self::DuplicateGuid { .. }
        )
    }

    /// The same fault as this crate's error type, at `path`.
    ///
    /// **Always [`ConfigError::InvalidValue`]**, whichever half is wrong.
    /// `ConfigError::InvalidKey` would be the tidier name for a bad table key,
    /// and its `rule` is a `&'static str` while a fault carries a *built*
    /// sentence naming the offending text — so using it would mean discarding
    /// the half of the message an operator acts on. The variant's name is worth
    /// less than that (`AGENTS.md` §5), and [`Self::is_about_the_name`] is how
    /// the caller points `path` at the right line either way.
    pub fn into_error(self, path: String, at: Option<crate::error::Location>) -> ConfigError {
        ConfigError::InvalidValue {
            path,
            at,
            detail: self.why(),
        }
    }
}

/// Check a whole set of declarations for the faults no single one can have.
///
/// **Today that is exactly one: two names for one GUID.** A per-entry check
/// cannot see it, and it is the one that would otherwise be discovered on a
/// board as *"applications appear under whichever name was read first"*.
pub fn check_together(volumes: &[Volume]) -> Result<(), VolumeFault> {
    for (at, one) in volumes.iter().enumerate() {
        for other in volumes.iter().skip(at + 1) {
            if one.guid() == other.guid() {
                return Err(VolumeFault::DuplicateGuid {
                    guid: one.guid().to_string(),
                    names: (one.name().to_string(), other.name().to_string()),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    const GOOD: &str = "9f2c1e40-7b3a-4c58-9d61-2ab7e0c4f183";

    fn volume(name: &str, guid: &str) -> Volume {
        Volume::new(
            VolumeName::new(name).expect("a usable name"),
            Guid::new(guid).expect("a usable guid"),
        )
    }

    #[test]
    fn the_shape_from_the_adr_is_accepted() {
        let v = volume("media", GOOD);
        assert_eq!(v.name().as_str(), "media");
        assert_eq!(v.guid().as_str(), GOOD);
    }

    /// **`state` is refused**, and the refusal explains rather than just
    /// naming the rule — an operator who declared it was reaching for something
    /// they already have.
    #[test]
    fn the_built_in_volume_cannot_be_declared_again() {
        let fault = VolumeName::new(BUILT_IN).expect_err("state is the built-in one");
        assert_eq!(fault, VolumeFault::BuiltInName);
        assert!(fault.why().contains("needs no declaration"), "{}", fault.why());
    }

    /// A name becomes a directory holding programs, so anything that is not one
    /// component is refused at construction.
    #[test]
    fn a_name_that_is_not_one_component_is_refused() {
        for hostile in ["..", ".", "a/b", "a\\b", "/abs", ".hidden", ""] {
            assert!(
                VolumeName::new(hostile).is_err(),
                "{hostile:?} was accepted as a volume name"
            );
        }
    }

    #[test]
    fn a_name_outside_the_alphabet_is_refused() {
        for hostile in ["me dia", "média", "-lead", "_lead", "a:b"] {
            assert!(VolumeName::new(hostile).is_err(), "{hostile:?}");
        }
        for fine in ["media", "big-disk", "vol_2", "a"] {
            assert!(VolumeName::new(fine).is_ok(), "{fine:?}");
        }
    }

    /// The GUID is checked for **form**, and the refusal says where.
    #[test]
    fn a_guid_of_the_wrong_shape_is_refused_and_says_where() {
        let short = Guid::new("9f2c1e40").expect_err("too short");
        assert!(matches!(short, VolumeFault::GuidLength { .. }));
        // Right length, hyphen in the wrong place.
        let wrong = Guid::new("9f2c1e4-07b3a-4c58-9d61-2ab7e0c4f183").expect_err("malformed");
        assert!(matches!(wrong, VolumeFault::GuidShape { .. }));
        // Right length and shape, not hex.
        let nonhex = Guid::new("9f2c1e4g-7b3a-4c58-9d61-2ab7e0c4f183").expect_err("not hex");
        assert!(matches!(nonhex, VolumeFault::GuidShape { at: 7, .. }));
    }

    /// **One spelling.** A GUID read off a partition table is compared against
    /// this, and two cases of one identifier is `ADR-011`'s failure.
    #[test]
    fn a_guid_is_lowercased_so_two_spellings_cannot_disagree() {
        let upper = Guid::new(&GOOD.to_ascii_uppercase()).expect("upper case is a GUID");
        let lower = Guid::new(GOOD).expect("lower case is one too");
        assert_eq!(upper, lower);
        assert_eq!(upper.as_str(), GOOD);
    }

    /// Two names for one device is refused, and no single entry could catch it.
    #[test]
    fn two_volumes_naming_one_guid_are_refused() {
        let fault = check_together(&[volume("media", GOOD), volume("backup", GOOD)])
            .expect_err("one GUID, two names");
        assert!(matches!(fault, VolumeFault::DuplicateGuid { .. }));
        assert!(fault.why().contains("media"), "{}", fault.why());
        assert!(fault.why().contains("backup"), "{}", fault.why());
    }

    #[test]
    fn distinct_volumes_are_fine_together() {
        let other = "0123abcd-4567-89ef-0123-456789abcdef";
        assert!(check_together(&[volume("media", GOOD), volume("backup", other)]).is_ok());
    }

    /// Every fault says something, and none of them is empty — `AGENTS.md` §5.
    #[test]
    fn every_fault_explains_itself() {
        let faults = [
            VolumeFault::EmptyName,
            VolumeFault::BuiltInName,
            VolumeFault::NotOneComponent {
                name: "a/b".to_string(),
            },
            VolumeFault::UnusableName {
                name: "a b".to_string(),
            },
            VolumeFault::GuidLength {
                guid: "x".to_string(),
                len: 1,
            },
            VolumeFault::GuidShape {
                guid: "x".to_string(),
                at: 0,
            },
            VolumeFault::DuplicateGuid {
                guid: GOOD.to_string(),
                names: ("a".to_string(), "b".to_string()),
            },
        ];
        for fault in faults {
            assert!(!fault.why().trim().is_empty(), "{fault:?} says nothing");
        }
    }

    /// The declared key list is what the parser and the refusal both read.
    #[test]
    fn there_is_exactly_one_declared_key_and_it_is_guid() {
        assert_eq!(Volume::DECLARED_KEYS, &["guid"]);
        // ADR-031 §7.1: adding one of these would be the second authority the
        // whole design refuses, so their absence is asserted rather than
        // assumed.
        for forbidden in ["device", "path", "label"] {
            assert!(
                !Volume::DECLARED_KEYS.contains(&forbidden),
                "{forbidden} is a key and ADR-031 §7.1 says it must not be"
            );
        }
    }
}
