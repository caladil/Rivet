//! The byte layout, and nothing that decides anything.
//!
//! `ADR-029` §3's diagram, transcribed as constants so that the reader and the
//! writer cannot disagree about an offset. Every number below appears exactly
//! once in this workspace — `ADR-011`, applied to a file format: a second
//! spelling of `4032` is a second authority over where the signature is.
//!
//! # Endianness — decided here, because `ADR-029` does not say
//!
//! **Little-endian**, for every integer field. `ADR-029` §3 gives widths and
//! offsets and never names a byte order; both architectures Rivet targets
//! (`aarch64` per `ADR-003` §4, and `x86_64` for `PLAN.md` §9.2's x86 target) are
//! little-endian, so big-endian would mean every machine that reads a payload
//! byte-swapping every field for no reader's benefit. Recorded rather than
//! assumed, because "obvious" is how a format grows two interpretations.
//!
//! # Reserved bytes must be zero, and that is strictness bought for free
//!
//! Every reserved byte in the header and in a member record is refused unless
//! it is zero. The usual objection — that this forecloses adding a field later
//! — does not apply: `ADR-029` §5 step 3 already refuses any `format_version`
//! that is not `1`, so a new field costs a version bump whatever this file
//! does. What it buys is that there is no place in a signed header to carry a
//! byte no reader looks at.

/// Bytes 0..8. `ADR-029` §3.
///
/// ~~`*b"FERRIPI\x1a"`. The `\x1A` is DOS end-of-file, the same trick PNG uses:
/// a payload catted to a terminal stops rather than scrolling, and a text-mode
/// transfer that mangles line endings is visible in the first eight bytes.~~
///
/// **Changed 2026-08-25 with the rename to Rivet** — `ADR-033`. `RIVET` is
/// three bytes shorter than `FERRIPI`, so the trailing `\x1A` no longer filled
/// the field, and the maintainer ruled all eight bytes printable rather than
/// name-plus-padding: **a hex dump says what the file is, in full.**
///
/// **What that trades away, stated because it was a real property:** a payload
/// `cat`ed to a terminal now scrolls 683 MB instead of stopping at byte eight.
/// The text-mode-transfer check is unaffected — the eight bytes are still the
/// first thing a reader compares, and a mangled transfer still shows up there.
///
/// **No compatibility branch is owed.** The old magic identified a format this
/// magic does not claim to read: a `FERRIPI` payload fails at byte zero with
/// the refusal `ADR-029` §5 step 1 already produces, which is the correct
/// answer rather than a missing feature.
pub const MAGIC: [u8; 8] = *b"RIVETPKG";

/// `ADR-029` §3. Refused if it is anything else — §5 step 3.
///
/// **2 since 2026-08-24**, and version 1 is refused rather than supported.
/// `product` grew from 16 bytes to 32, which moved every field after it and the
/// member table with them, so a version-1 header parsed by this reader would
/// find `version` where `product` ends.
///
/// **Nothing had ever verified an `.rvt` on a Rivet when this changed** — no
/// image carried a trust store (`ADR-028` §5.7) — so there were zero deployed
/// artifacts and no compatibility branch is owed to anybody. That was the whole
/// argument for doing it now rather than later; see `PLAN.md` §0.4.5.
pub const FORMAT_VERSION: u32 = 2;

/// The whole header. **Checked against this constant and never used to size a
/// read** — `ADR-029` §5 step 4.
pub const HEADER_BYTES: usize = 4096;

/// What the signature covers: `[0 .. 4032)`. `ADR-029` §3.
pub const SIGNED_BYTES: usize = 4032;

/// Where the signature itself sits.
pub const SIGNATURE_OFFSET: usize = SIGNED_BYTES;

/// Ed25519's signature size, and the size this format has room for.
///
/// `ADR-029` §7.1 leaves `sig_alg` open so a post-quantum scheme is "a field
/// value rather than a redesign". That is true of the *algorithm identifier*
/// and not of the *size*: a scheme whose signature is not 64 bytes needs
/// [`FORMAT_VERSION`] 2, because there are only 64 bytes here. Stated so nobody
/// discovers it.
pub const SIGNATURE_BYTES: usize = 64;

/// Both the digest size and the key-id size. `ADR-029` §3, §7.2 (BLAKE3-256).
pub const DIGEST_BYTES: usize = 32;

/// Ed25519's raw public key — `ADR-029` §7.1's "32 raw bytes … has no parser to
/// get wrong".
pub const PUBLIC_KEY_BYTES: usize = 32;

/// Where the member table starts. `ADR-029` §3.
pub const MEMBER_TABLE_OFFSET: usize = 184;

/// One member record. `ADR-029` §3.
pub const MEMBER_RECORD_BYTES: usize = 96;

/// `ADR-029` §3: `member_count u32, <= 40`.
pub const MAX_MEMBERS: usize = 40;

// --- header field offsets, in the order ADR-029 §3 lists them ---------------

pub const OFF_MAGIC: usize = 0;
pub const OFF_FORMAT_VERSION: usize = 8;
pub const OFF_HEADER_BYTES: usize = 12;
pub const OFF_SIG_ALG: usize = 16;
pub const OFF_DIGEST_ALG: usize = 20;
pub const OFF_KEY_ID: usize = 24;

/// **Nothing at or below this offset is parsed until `ADR-029` §5 step 7.**
pub const UNAUTHENTICATED_BOUNDARY: usize = 56;

pub const OFF_MEMBER_COUNT: usize = 56;
pub const OFF_FLAGS: usize = 60;
pub const OFF_PRODUCT: usize = 64;
pub const OFF_VERSION: usize = 96;
pub const OFF_ARCH: usize = 128;
pub const OFF_PLATFORM: usize = 144;
pub const OFF_KIND: usize = 160;
pub const OFF_BUILD_EPOCH: usize = 176;

/// 32 bytes since 2026-08-24, up from 16.
///
/// **16 could not hold `ADR-015` §5.4's own example.** `metrics-collector` is
/// 17 characters, and `cmd/cargo-rivet` refused to package it — correctly,
/// since truncating would silently rename somebody's application, but the
/// refusal was the format's fault rather than the name's.
pub const PRODUCT_BYTES: usize = 32;
pub const VERSION_BYTES: usize = 32;
pub const ARCH_BYTES: usize = 16;
pub const PLATFORM_BYTES: usize = 16;
pub const KIND_BYTES: usize = 16;

/// The gap between the last possible member record and the signature.
///
/// `184 + 40 * 96 = 4024`, and the signature starts at `4032` — so **8 bytes**,
/// which must be zero like every other reserved byte.
///
/// **It was 24 before `product` grew on 2026-08-24, and that is worth saying
/// out loud: this header is now nearly full.** It is not a cliff, because
/// [`HEADER_BYTES`] is itself a field at offset 12 — a future version may
/// declare a larger header and a reader can tell which it is holding. That
/// escape hatch is why spending 16 of the 24 here was affordable. The next
/// field that does not fit in 8 bytes is a header-size decision, not a
/// squeeze.
pub const OFF_HEADER_RESERVED: usize = MEMBER_TABLE_OFFSET + MAX_MEMBERS * MEMBER_RECORD_BYTES;
pub const HEADER_RESERVED_BYTES: usize = SIGNED_BYTES - OFF_HEADER_RESERVED;

// --- member record field offsets, relative to the record --------------------

pub const M_NAME: usize = 0;
pub const M_NAME_BYTES: usize = 32;
pub const M_DIGEST: usize = 32;
pub const M_TARGET_CLASS: usize = 64;
pub const M_VARIANT: usize = 65;
pub const M_COMPRESSION: usize = 66;
pub const M_RESERVED_0: usize = 67;
pub const M_OFFSET: usize = 68;
pub const M_STORED_LEN: usize = 76;
pub const M_PLAIN_LEN: usize = 84;
pub const M_RESERVED_1: usize = 92;
pub const M_RESERVED_1_BYTES: usize = 4;

// `ADR-029` §3's arithmetic, checked at compile time rather than believed. A
// member table that overran the signature would be a format whose last member
// record and whose signature were the same bytes.
const _: () = assert!(OFF_HEADER_RESERVED == 4024);
const _: () = assert!(HEADER_RESERVED_BYTES == 8);
const _: () = assert!(SIGNATURE_OFFSET + SIGNATURE_BYTES == HEADER_BYTES);
const _: () = assert!(M_RESERVED_1 + M_RESERVED_1_BYTES == MEMBER_RECORD_BYTES);
const _: () = assert!(OFF_BUILD_EPOCH + 8 == MEMBER_TABLE_OFFSET);
const _: () = assert!(UNAUTHENTICATED_BOUNDARY == OFF_KEY_ID + DIGEST_BYTES);

/// Ways a fixed-width text field, or a member name, can be unusable.
///
/// Every variant carries what was found, per `AGENTS.md` §5 — the offending
/// byte is what the *builder* said, and a diagnostic that does not quote it
/// sends the reader back to a hex dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFault {
    /// Every byte is NUL, and no field this format has may be nameless. The one
    /// deliberate exception is `platform` — see [`read_text`]'s caller.
    Empty,
    /// Outside `0x21 ..= 0x7e`. Space is excluded along with the control codes:
    /// these values go into filenames and into `ADR-027` §3's grammar, where a
    /// space is a quoting problem waiting for a shell.
    NotPrintableAscii { byte: u8 },
    /// A non-NUL byte after the first NUL. NUL-padded means padded, and bytes
    /// hiding behind a terminator are bytes no reader looks at.
    TrailingBytesAfterNul,
    /// `/` or `\`. Member names are flat; a name that can traverse is the
    /// oldest archive bug there is.
    PathSeparator { byte: u8 },
    /// `.` or `..` exactly. Neither names a member, and both name a directory
    /// to anything that later joins the name onto a path.
    DotOrDotDot,
}

impl core::fmt::Display for TextFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "it is empty"),
            Self::NotPrintableAscii { byte } => {
                write!(f, "byte 0x{byte:02x} is not printable ASCII")
            }
            Self::TrailingBytesAfterNul => write!(f, "there are bytes after the NUL padding"),
            Self::PathSeparator { byte } => {
                write!(f, "it contains a path separator (0x{byte:02x})")
            }
            Self::DotOrDotDot => write!(f, "`.` and `..` are not names"),
        }
    }
}

/// Read one NUL-padded ASCII field.
///
/// **Refuses rather than truncating at the first thing it does not like**
/// (`AGENTS.md` §5). A lenient reader here would let two implementations
/// disagree about what a header says while both reporting success, which is the
/// failure mode a signed format is supposed to make impossible.
///
/// `Ok(None)` means the field is entirely NUL. Whether that is legal is the
/// caller's — `platform` may be empty (`ADR-027` §9 items 3–4 are open, and an
/// application may turn out not to carry one), and nothing else may be.
pub fn read_text(field: &[u8]) -> Result<Option<&str>, TextFault> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    if field[end..].iter().any(|b| *b != 0) {
        return Err(TextFault::TrailingBytesAfterNul);
    }
    if end == 0 {
        return Ok(None);
    }
    for byte in &field[..end] {
        if !(0x21..=0x7e).contains(byte) {
            return Err(TextFault::NotPrintableAscii { byte: *byte });
        }
    }
    // Every byte is in 0x21..=0x7e, which is ASCII, which is UTF-8.
    match core::str::from_utf8(&field[..end]) {
        Ok(text) => Ok(Some(text)),
        // Unreachable given the loop above; refusing is still the right answer
        // and costs nothing, where a panic in a verifier would be a denial of
        // service reachable from an attacker's bytes.
        Err(_) => Err(TextFault::NotPrintableAscii { byte: field[0] }),
    }
}

/// [`read_text`], plus the two rules that apply to a member name and to nothing
/// else.
pub fn read_name(field: &[u8]) -> Result<&str, TextFault> {
    let text = read_text(field)?.ok_or(TextFault::Empty)?;
    for byte in text.as_bytes() {
        if *byte == b'/' || *byte == b'\\' {
            return Err(TextFault::PathSeparator { byte: *byte });
        }
    }
    if text == "." || text == ".." {
        return Err(TextFault::DotOrDotDot);
    }
    Ok(text)
}

/// Write a NUL-padded ASCII field, refusing anything [`read_text`] would refuse.
///
/// The writer holds itself to the reader's rules on purpose: a builder that can
/// emit a header its own verifier rejects is a builder that ships one.
pub fn write_text(field: &mut [u8], text: &str) -> Result<(), TextFault> {
    if text.len() > field.len() {
        // Not a `TextFault` — length is the caller's problem and the caller
        // knows the capacity. Reported by the writer, which has both numbers.
        return Err(TextFault::TrailingBytesAfterNul);
    }
    for byte in text.as_bytes() {
        if !(0x21..=0x7e).contains(byte) {
            return Err(TextFault::NotPrintableAscii { byte: *byte });
        }
    }
    field.fill(0);
    field[..text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

/// Little-endian `u32` at `at`. Panic-free by construction: every call site
/// passes an offset this module owns, and the slice is [`HEADER_BYTES`] long.
pub fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

/// Little-endian `u64` at `at`.
pub fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    bytes
        .get(at..at + 8)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

/// Little-endian `i64` at `at`.
pub fn read_i64(bytes: &[u8], at: usize) -> Option<i64> {
    bytes
        .get(at..at + 8)?
        .try_into()
        .ok()
        .map(i64::from_le_bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_layout_is_adr_029_3s() {
        // Read off the ADR's diagram by hand rather than derived from the
        // constants, so that a typo in a constant fails here instead of
        // agreeing with itself.
        assert_eq!(MAGIC, *b"RIVETPKG");
        assert!(
            MAGIC.iter().all(u8::is_ascii_graphic),
            "ADR-033 ruled all eight bytes printable — a hex dump says what the file is"
        );
        assert_eq!((OFF_FORMAT_VERSION, OFF_HEADER_BYTES), (8, 12));
        assert_eq!((OFF_SIG_ALG, OFF_DIGEST_ALG, OFF_KEY_ID), (16, 20, 24));
        assert_eq!((OFF_MEMBER_COUNT, OFF_FLAGS), (56, 60));
        assert_eq!((OFF_PRODUCT, OFF_VERSION, OFF_ARCH), (64, 96, 128));
        assert_eq!((OFF_PLATFORM, OFF_KIND, OFF_BUILD_EPOCH), (144, 160, 176));
        assert_eq!(MEMBER_TABLE_OFFSET, 184);
        assert_eq!(MEMBER_RECORD_BYTES, 96);
        assert_eq!(SIGNATURE_OFFSET, 4032);
        assert_eq!(HEADER_BYTES, 4096);
        assert_eq!(MAX_MEMBERS, 40);
    }

    #[test]
    fn a_text_field_is_nul_padded_and_read_back_whole() {
        let mut field = [0xffu8; 16];
        write_text(&mut field, "rivet").unwrap();
        assert_eq!(&field[..5], b"rivet");
        assert!(field[5..].iter().all(|b| *b == 0));
        assert_eq!(read_text(&field), Ok(Some("rivet")));
    }

    #[test]
    fn a_full_width_field_needs_no_terminator() {
        let mut field = [0u8; 16];
        write_text(&mut field, "0123456789abcdef").unwrap();
        assert_eq!(read_text(&field), Ok(Some("0123456789abcdef")));
    }

    #[test]
    fn bytes_hiding_behind_the_nul_padding_are_refused() {
        let mut field = [0u8; 16];
        field[..3].copy_from_slice(b"upd");
        field[9] = b'!';
        assert_eq!(read_text(&field), Err(TextFault::TrailingBytesAfterNul));
    }

    #[test]
    fn an_all_nul_field_reads_as_absent_rather_than_as_an_error() {
        // `platform` depends on this: ADR-027 §9 items 3-4 are open, so the
        // format must be able to say "no platform" without a reader guessing.
        assert_eq!(read_text(&[0u8; 16]), Ok(None));
        assert_eq!(read_name(&[0u8; 32]), Err(TextFault::Empty));
    }

    #[test]
    fn a_name_may_not_traverse_and_may_not_be_a_directory() {
        let name = |text: &str| -> Result<String, TextFault> {
            let mut field = [0u8; 32];
            field[..text.len()].copy_from_slice(text.as_bytes());
            read_name(&field).map(ToString::to_string)
        };
        assert_eq!(name("kernel8.img").as_deref(), Ok("kernel8.img"));
        assert_eq!(
            name("../etc/shadow"),
            Err(TextFault::PathSeparator { byte: b'/' })
        );
        assert_eq!(name("a\\b"), Err(TextFault::PathSeparator { byte: b'\\' }));
        assert_eq!(name(".."), Err(TextFault::DotOrDotDot));
        assert_eq!(name("."), Err(TextFault::DotOrDotDot));
        // `..foo` traverses nothing and is a legal, if odd, name.
        assert_eq!(name("..foo").as_deref(), Ok("..foo"));
    }

    #[test]
    fn control_bytes_and_spaces_are_not_names() {
        let mut field = [0u8; 32];
        field[..3].copy_from_slice(b"a b");
        assert_eq!(
            read_name(&field),
            Err(TextFault::NotPrintableAscii { byte: b' ' })
        );
        let mut field = [0u8; 32];
        field[..3].copy_from_slice(b"a\x07b");
        assert_eq!(
            read_name(&field),
            Err(TextFault::NotPrintableAscii { byte: 7 })
        );
        // High-bit bytes are the ones that would otherwise reach a UTF-8
        // decoder, and they never do.
        let mut field = [0u8; 32];
        field[..2].copy_from_slice(&[0xc3, 0xa9]);
        assert_eq!(
            read_name(&field),
            Err(TextFault::NotPrintableAscii { byte: 0xc3 })
        );
    }

    #[test]
    fn the_integer_readers_refuse_a_short_slice_rather_than_indexing_past_it() {
        assert_eq!(read_u32(&[1, 2, 3], 0), None);
        assert_eq!(read_u64(&[0u8; 7], 0), None);
        assert_eq!(read_u32(&[1, 0, 0, 0], 0), Some(1));
        assert_eq!(read_u64(&[1, 0, 0, 0, 0, 0, 0, 0], 0), Some(1));
        assert_eq!(read_i64(&[0xff; 8], 0), Some(-1));
    }
}
