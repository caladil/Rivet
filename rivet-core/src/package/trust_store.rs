//! The file that says what a machine trusts, as bytes.
//!
//! `ADR-028` §5.5.6, ruled 2026-08-24: the trust store is **a file in
//! `/system`**, not constants compiled into the binaries. Security is not what
//! decides that — §5.4 already concedes that an attacker who can write
//! `/system` has replaced the verifier — it is that **a machine should be able
//! to say what it trusts.** A constant in a stripped binary is a fact nobody
//! can ask for.
//!
//! # Why this looks like `ADR-029` §3 rather than like a config file
//!
//! Because it is read by the same kind of code, at the same moment, for the
//! same stakes. `crate::package::format`'s argument applies here unchanged:
//! fixed offsets, no length prefixes to disagree with a buffer, and **every
//! reserved byte refused unless it is zero**. A trust store is the one file on
//! a Rivet whose misparse means an attacker's payload verifies, so it gets
//! the format with no parser to get wrong rather than the one that is pleasant
//! to hand-edit.
//!
//! It is emitted by `cmd/rivet-image` and read by whatever is about to verify
//! something. Nothing hand-edits it, so nothing is owed a text format —
//! `rivet get trust` is how a human asks, and that renders from these bytes.
//!
//! # What is deliberately NOT in a record
//!
//! **The key id.** It is `BLAKE3` of the public key (`ADR-029` §3), so storing
//! it would create a second place for it to be written down and a way for the
//! two to disagree — `ADR-011` in the small, in the file where a disagreement
//! is worst. `Entry::id` derives it.
//!
//! **An expiry.** `ADR-028` §5.5.4 refuses one: an expired trust entry does not
//! fail safe on a headless board, it removes the ability to update the machine
//! that needs updating. Revocation is an A/B image update that ships a store
//! without the key.
//!
//! **A delegation.** §5.6 defers the root key, and records why deferring is
//! safe: a delegation travels as a *payload member*, not as a field here, so
//! adding one later changes this file's meaning not at all.

use crate::package::error::PackageError;
use crate::package::format::{PUBLIC_KEY_BYTES, TextFault, read_text};
use crate::package::kind::Kind;
use crate::package::port::{MaySign, PublicKey};

/// Where the image records what this machine trusts.
///
/// `ADR-028` §5.5.6, ruled 2026-08-24: **a file in `/system`**, not constants
/// compiled into the binaries. `/system` is read-only by format (`ADR-018` §3)
/// and replaced only wholesale by an update that was itself verified, which is
/// what makes §5.5.4's revocation mechanism — ship an image without the key —
/// the same mechanism as everything else.
///
/// It is under `/system` rather than `/system/bin` because it is not a program,
/// and at a fixed path rather than a configured one because a configurable
/// trust anchor is the thing §5.2 exists to forbid.
///
/// **Declared here rather than in the tool that writes it**, since 2026-08-24:
/// `cmd/rivet-image` writes this file and `cmd/rivet-deploy` reads it, and
/// they are binaries that cannot see each other. Two spellings of one path is
/// `ADR-011` §3, and the failure it produces is a machine that says what it
/// trusts in a file nothing consults.
pub const PATH: &str = "/system/trust";

/// Bytes 0..8. Deliberately not the container's magic: a payload and a trust
/// store must never be mistaken for one another, and the first eight bytes are
/// where that is cheapest to establish.
///
/// ~~`*b"FERRIPIT"` — the container's seven bytes plus `T` for trust.~~
/// **`*b"RIVETTRS"` since the rename (`ADR-033`)**, and the distinction it
/// exists for is *stronger* now rather than weaker: `RIVETPKG` and `RIVETTRS`
/// differ in three of eight bytes and both say in plain ASCII which one they
/// are, where the old pair differed in one byte and neither said anything.
pub const MAGIC: [u8; 8] = *b"RIVETTRS";

/// Refused if it is anything else, for `ADR-029` §5 step 3's reason.
pub const FORMAT_VERSION: u32 = 1;

pub const HEADER_BYTES: usize = 32;
pub const ENTRY_BYTES: usize = 128;

pub const OFF_MAGIC: usize = 0;
pub const OFF_FORMAT_VERSION: usize = 8;
pub const OFF_ENTRY_COUNT: usize = 12;
pub const OFF_HEADER_RESERVED: usize = 16;
pub const HEADER_RESERVED_BYTES: usize = HEADER_BYTES - OFF_HEADER_RESERVED;

pub const E_OFF_KEY: usize = 0;
pub const E_OFF_LABEL: usize = 32;
pub const E_OFF_KINDS: usize = 48;
pub const E_OFF_RESERVED: usize = 112;

pub const LABEL_BYTES: usize = 16;
pub const KIND_SLOT_BYTES: usize = 16;

/// How many kinds one key may be entitled to sign.
///
/// Four slots of 16 bytes. `ADR-029` has three kinds today and `Kind` is open
/// text rather than an enum, so this is a limit rather than an enumeration —
/// and a limit is what a fixed-size record needs. A key wanting a fifth is a
/// format decision, which is the right size of decision for it to be.
pub const MAX_KINDS: usize = 4;

/// A store may not carry more keys than this.
///
/// **A bound rather than a guess.** The file is read into memory by a verifier
/// with no allocator worth trusting an attacker with, and 64 keys is far past
/// anything `ADR-028` §5.6's two-key design implies. What it stops is a store
/// whose header claims four billion entries.
pub const MAX_ENTRIES: usize = 64;

const _: () = assert!(E_OFF_KINDS + MAX_KINDS * KIND_SLOT_BYTES == E_OFF_RESERVED);
const _: () = assert!(E_OFF_LABEL + LABEL_BYTES == E_OFF_KINDS);
const _: () = assert!(E_OFF_KEY + PUBLIC_KEY_BYTES == E_OFF_LABEL);

/// One key, and the limits of trusting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: PublicKey,
    pub label: String,
    pub may_sign: MaySign,
}

/// The whole file, parsed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustStore {
    entries: Vec<Entry>,
}

impl TrustStore {
    #[must_use]
    pub fn new(entries: Vec<Entry>) -> Self {
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Render the file.
    ///
    /// # Errors
    ///
    /// If a label or a kind will not fit its field, or there are too many of
    /// either — refused with the limit named, never truncated, because a
    /// truncated label names a key that does not exist and a dropped kind
    /// silently widens or narrows what a key may sign.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PackageError> {
        if self.entries.len() > MAX_ENTRIES {
            return Err(PackageError::TooManyMembers {
                found: u32::try_from(self.entries.len()).unwrap_or(u32::MAX),
            });
        }
        let mut out = vec![0u8; HEADER_BYTES + self.entries.len() * ENTRY_BYTES];
        out[OFF_MAGIC..OFF_MAGIC + 8].copy_from_slice(&MAGIC);
        out[OFF_FORMAT_VERSION..OFF_FORMAT_VERSION + 4]
            .copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        let count = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
        out[OFF_ENTRY_COUNT..OFF_ENTRY_COUNT + 4].copy_from_slice(&count.to_le_bytes());

        for (n, entry) in self.entries.iter().enumerate() {
            let at = HEADER_BYTES + n * ENTRY_BYTES;
            out[at + E_OFF_KEY..at + E_OFF_KEY + PUBLIC_KEY_BYTES].copy_from_slice(&entry.key.0);
            text_into(
                &mut out[at + E_OFF_LABEL..at + E_OFF_LABEL + LABEL_BYTES],
                &entry.label,
                "label",
            )?;
            let kinds = entry.may_sign.kinds();
            if kinds.len() > MAX_KINDS {
                return Err(PackageError::TextTooLong {
                    field: "may-sign",
                    len: kinds.len(),
                    capacity: MAX_KINDS,
                });
            }
            for (slot, kind) in kinds.iter().enumerate() {
                let from = at + E_OFF_KINDS + slot * KIND_SLOT_BYTES;
                text_into(
                    &mut out[from..from + KIND_SLOT_BYTES],
                    kind.as_str(),
                    "may-sign",
                )?;
            }
        }
        Ok(out)
    }

    /// Parse the file.
    ///
    /// # Errors
    ///
    /// Anything that is not exactly a well-formed store: wrong magic, an
    /// unimplemented version, a length that does not match the count, a
    /// reserved byte that is not zero, or a label or kind that is not clean
    /// text. **None of these is repaired.**
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() < HEADER_BYTES {
            return Err(PackageError::TooShortForHeader {
                file_len: bytes.len() as u64,
            });
        }
        if bytes[OFF_MAGIC..OFF_MAGIC + 8] != MAGIC {
            return Err(PackageError::NotAPackage {
                found: [
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ],
            });
        }
        let version = read_u32(bytes, OFF_FORMAT_VERSION);
        if version != FORMAT_VERSION {
            return Err(PackageError::UnsupportedFormatVersion { found: version });
        }
        if !bytes[OFF_HEADER_RESERVED..HEADER_BYTES]
            .iter()
            .all(|b| *b == 0)
        {
            return Err(PackageError::ReservedBytesNotZero {
                field: "trust store header",
            });
        }
        let count = read_u32(bytes, OFF_ENTRY_COUNT) as usize;
        if count > MAX_ENTRIES {
            return Err(PackageError::TooManyMembers {
                found: u32::try_from(count).unwrap_or(u32::MAX),
            });
        }
        // **The length must match exactly.** A store with trailing bytes is a
        // store somebody appended to, and a verifier that ignored them would be
        // trusting a file it did not read all of.
        let expected = HEADER_BYTES + count * ENTRY_BYTES;
        if bytes.len() != expected {
            return Err(PackageError::TrailingBytes {
                members_end: expected as u64,
                file_len: bytes.len() as u64,
            });
        }

        let mut entries = Vec::with_capacity(count);
        for n in 0..count {
            let at = HEADER_BYTES + n * ENTRY_BYTES;
            if !bytes[at + E_OFF_RESERVED..at + ENTRY_BYTES]
                .iter()
                .all(|b| *b == 0)
            {
                return Err(PackageError::ReservedBytesNotZero {
                    field: "trust entry",
                });
            }
            let mut key = [0u8; PUBLIC_KEY_BYTES];
            key.copy_from_slice(&bytes[at + E_OFF_KEY..at + E_OFF_KEY + PUBLIC_KEY_BYTES]);

            let label = required_text(
                &bytes[at + E_OFF_LABEL..at + E_OFF_LABEL + LABEL_BYTES],
                "label",
            )?;

            let mut kinds = Vec::new();
            for slot in 0..MAX_KINDS {
                let from = at + E_OFF_KINDS + slot * KIND_SLOT_BYTES;
                let field = &bytes[from..from + KIND_SLOT_BYTES];
                match read_text(field) {
                    Ok(Some(text)) => kinds.push(Kind::parse(text)?),
                    // An empty slot is how "fewer than four" is written, and
                    // trailing empties are the only place one may appear —
                    // a gap would make the count ambiguous.
                    Ok(None) => {
                        let rest_from = from + KIND_SLOT_BYTES;
                        let rest_to = at + E_OFF_RESERVED;
                        if !bytes[rest_from..rest_to].iter().all(|b| *b == 0) {
                            return Err(PackageError::MalformedText {
                                field: "may-sign",
                                fault: TextFault::Empty,
                            });
                        }
                        break;
                    }
                    Err(fault) => {
                        return Err(PackageError::MalformedText {
                            field: "may-sign",
                            fault,
                        });
                    }
                }
            }
            entries.push(Entry {
                key: PublicKey(key),
                label,
                may_sign: MaySign::these(&kinds),
            });
        }
        Ok(Self { entries })
    }
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(raw)
}

fn required_text(field: &[u8], name: &'static str) -> Result<String, PackageError> {
    match read_text(field) {
        Ok(Some(text)) => Ok(text.to_string()),
        Ok(None) => Err(PackageError::MissingText { field: name }),
        Err(fault) => Err(PackageError::MalformedText { field: name, fault }),
    }
}

fn text_into(field: &mut [u8], text: &str, name: &'static str) -> Result<(), PackageError> {
    if text.is_empty() {
        return Err(PackageError::MissingText { field: name });
    }
    if text.len() > field.len() {
        return Err(PackageError::TextTooLong {
            field: name,
            len: text.len(),
            capacity: field.len(),
        });
    }
    field[..text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn key(fill: u8) -> PublicKey {
        PublicKey([fill; PUBLIC_KEY_BYTES])
    }

    fn store() -> TrustStore {
        TrustStore::new(vec![
            Entry {
                key: key(0x11),
                label: "rivet-system".to_string(),
                may_sign: MaySign::these(&[Kind::update(), Kind::parse("extension").unwrap()]),
            },
            Entry {
                key: key(0x22),
                label: "joel-apps".to_string(),
                may_sign: MaySign::these(&[Kind::application()]),
            },
        ])
    }

    /// The two-key arrangement `ADR-028` §5.5 exists for, through the bytes.
    #[test]
    fn the_two_key_arrangement_survives_a_round_trip() {
        let bytes = store().to_bytes().unwrap();
        assert_eq!(bytes.len(), HEADER_BYTES + 2 * ENTRY_BYTES);
        let back = TrustStore::from_bytes(&bytes).unwrap();
        assert_eq!(back, store());

        let system = &back.entries()[0];
        let apps = &back.entries()[1];
        assert!(system.may_sign.permits(&Kind::update()));
        assert!(!system.may_sign.permits(&Kind::application()));
        // The property the whole design is for.
        assert!(!apps.may_sign.permits(&Kind::update()));
        assert!(apps.may_sign.permits(&Kind::application()));
    }

    /// A store with no keys is a *legal file*, and reading one back must not be
    /// how a machine discovers it cannot verify anything.
    ///
    /// `ADR-028` §5.5.5 puts that refusal at **build** time, in
    /// `cmd/rivet-image`, where a human can still act on it.
    #[test]
    fn an_empty_store_round_trips_rather_than_being_refused_here() {
        let empty = TrustStore::new(Vec::new());
        let bytes = empty.to_bytes().unwrap();
        assert_eq!(bytes.len(), HEADER_BYTES);
        assert_eq!(TrustStore::from_bytes(&bytes).unwrap(), empty);
        assert!(TrustStore::from_bytes(&bytes).unwrap().is_empty());
    }

    #[test]
    fn a_payload_is_not_a_trust_store_and_the_magic_says_so_first() {
        let mut bytes = store().to_bytes().unwrap();
        // The container's own constant rather than a second spelling of it —
        // `ADR-011`. The rename (`ADR-033`) is what made that matter: a literal
        // here went stale the moment the magic changed, and a mechanical
        // find-and-replace turned it into six bytes where eight were needed.
        bytes[..8].copy_from_slice(&crate::package::format::MAGIC);
        assert!(matches!(
            TrustStore::from_bytes(&bytes),
            Err(PackageError::NotAPackage { .. })
        ));
    }

    #[test]
    fn an_unimplemented_version_is_refused_rather_than_guessed_at() {
        for version in [0u32, 2, 0xffff_ffff] {
            let mut bytes = store().to_bytes().unwrap();
            bytes[OFF_FORMAT_VERSION..OFF_FORMAT_VERSION + 4]
                .copy_from_slice(&version.to_le_bytes());
            assert_eq!(
                TrustStore::from_bytes(&bytes),
                Err(PackageError::UnsupportedFormatVersion { found: version })
            );
        }
    }

    /// **Appending to a trust store must not go unnoticed.**
    ///
    /// A verifier that read `count` entries and ignored the rest would be
    /// trusting a file it had not read all of.
    #[test]
    fn a_store_with_bytes_after_the_last_entry_is_refused() {
        let mut bytes = store().to_bytes().unwrap();
        bytes.push(0);
        assert!(matches!(
            TrustStore::from_bytes(&bytes),
            Err(PackageError::TrailingBytes { .. })
        ));
    }

    /// A count that does not match the file is refused, in both directions.
    #[test]
    fn a_count_that_disagrees_with_the_length_is_refused() {
        for claimed in [0u32, 1, 3, 64] {
            let mut bytes = store().to_bytes().unwrap();
            bytes[OFF_ENTRY_COUNT..OFF_ENTRY_COUNT + 4].copy_from_slice(&claimed.to_le_bytes());
            assert!(
                TrustStore::from_bytes(&bytes).is_err(),
                "a store of two entries claiming {claimed} was accepted"
            );
        }
    }

    #[test]
    fn a_count_past_the_maximum_is_refused_before_anything_is_allocated() {
        let mut bytes = store().to_bytes().unwrap();
        bytes[OFF_ENTRY_COUNT..OFF_ENTRY_COUNT + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            TrustStore::from_bytes(&bytes),
            Err(PackageError::TooManyMembers { .. })
        ));
    }

    #[test]
    fn a_reserved_byte_that_is_not_zero_is_refused_in_the_header_and_in_an_entry() {
        let mut header = store().to_bytes().unwrap();
        header[OFF_HEADER_RESERVED] = 1;
        assert!(matches!(
            TrustStore::from_bytes(&header),
            Err(PackageError::ReservedBytesNotZero { .. })
        ));

        let mut entry = store().to_bytes().unwrap();
        entry[HEADER_BYTES + E_OFF_RESERVED] = 1;
        assert!(matches!(
            TrustStore::from_bytes(&entry),
            Err(PackageError::ReservedBytesNotZero { .. })
        ));
    }

    /// **A kind written after an empty slot is refused rather than read.**
    ///
    /// Trailing empties are how "fewer than four" is spelled. A gap would make
    /// the entitlement ambiguous, and an ambiguous entitlement resolved
    /// generously is how a key quietly gains a kind.
    #[test]
    fn a_kind_hidden_after_an_empty_slot_is_refused() {
        let mut bytes = store().to_bytes().unwrap();
        // Entry 1 is `joel-apps` with one kind; write another into slot 2,
        // leaving slot 1 empty.
        let at = HEADER_BYTES + ENTRY_BYTES + E_OFF_KINDS + 2 * KIND_SLOT_BYTES;
        bytes[at..at + 6].copy_from_slice(b"update");
        assert!(matches!(
            TrustStore::from_bytes(&bytes),
            Err(PackageError::MalformedText { .. })
        ));
    }

    #[test]
    fn a_label_that_will_not_fit_is_refused_with_the_limit_named() {
        let long = TrustStore::new(vec![Entry {
            key: key(1),
            label: "a-label-far-too-long-for-the-field".to_string(),
            may_sign: MaySign::these(&[Kind::update()]),
        }]);
        assert!(matches!(
            long.to_bytes(),
            Err(PackageError::TextTooLong {
                field: "label",
                capacity: LABEL_BYTES,
                ..
            })
        ));
    }

    /// A key entitled to nothing is representable, because that is what an
    /// entry becomes on its way out.
    #[test]
    fn a_key_entitled_to_nothing_round_trips() {
        let none = TrustStore::new(vec![Entry {
            key: key(3),
            label: "revoked".to_string(),
            may_sign: MaySign::nothing(),
        }]);
        let back = TrustStore::from_bytes(&none.to_bytes().unwrap()).unwrap();
        assert_eq!(back, none);
        assert!(!back.entries()[0].may_sign.permits(&Kind::update()));
    }
}
