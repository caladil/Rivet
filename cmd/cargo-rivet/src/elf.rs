//! Just enough ELF to answer three questions about a program or a shared
//! object, and nothing more.
//!
//! | Question | Answered by |
//! | :-- | :-- |
//! | is this dynamically linked, and by which loader? | `PT_INTERP` |
//! | what does it need at run time? | `DT_NEEDED` |
//! | what does it call itself? | `DT_SONAME` |
//!
//! # Why this is here rather than a crate
//!
//! `ADR-009` §3's ladder prefers an existing Rust crate, and `goblin` or
//! `object` would answer these three questions and several hundred more. The
//! reason not to is `ADR-009` §5's gate applied to a **host** tool: this is the
//! binary that holds the signing key (`cmd/cargo-rivet/Cargo.toml`), and every
//! dependency it gains is one more thing with the opportunity to read a seed.
//! Two hundred lines that read six fields is the cheaper side of that trade.
//!
//! # What it deliberately does not do
//!
//! **It does not validate an ELF.** A malformed file gets [`ElfFault`] and the
//! answer is *"this tool could not read it"*, never *"this file is bad"* — the
//! kernel and the loader are the authorities on whether a binary runs, and a
//! second opinion from a build tool would be a second authority (`ADR-011`).
//!
//! **It reads aarch64 little-endian 64-bit and refuses everything else by
//! name.** `cargo rivet` cross-compiles to one target class (`ADR-027` §3.1),
//! and a parser that guessed at a big-endian 32-bit file would be guessing on
//! behalf of a platform this project does not have.
//!
//! **That sentence was half true until 2026-09-03** and is worth leaving as a
//! warning about the other kind of documentation drift — the kind where the
//! prose is right about the intent and the code never caught up. The class and
//! the endianness were checked; `e_machine` was not, so a host build was read,
//! signed and packaged as aarch64. See [`ElfFault::NotAarch64`].

/// Why a file could not be read as the ELF this tool expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElfFault {
    /// Fewer bytes than the smallest ELF header.
    TooShort,
    /// No `\x7fELF`.
    NotElf,
    /// Not 64-bit, or not little-endian.
    NotSixtyFourLittleEndian,
    /// `e_machine` is not `EM_AARCH64`, carrying whatever it actually is.
    ///
    /// **The refusal this module's own header claimed and did not make**, until
    /// 2026-09-03: it said it *"reads aarch64 little-endian 64-bit and refuses
    /// everything else by name"*, and it checked the class and the endianness
    /// and never the architecture (`PLAN.md` §123, Wave 4.2). An x86-64 static
    /// binary is 64-bit and little-endian, so it read cleanly, was signed, and
    /// was packaged under `ADR-027` §3.1's aarch64 name. The first thing that
    /// would have noticed is the board, at `execve`, with `ENOEXEC` — after a
    /// card had been written.
    NotAarch64 { found: u16 },
    /// A header field points outside the file.
    OutOfBounds { what: &'static str },
    /// A `DT_STRTAB` address that no `PT_LOAD` segment covers, so a string
    /// index cannot be turned into a file offset.
    UnmappedStringTable,
    /// A string in the dynamic string table that is not NUL-terminated inside
    /// it, or is not UTF-8.
    BadString,
}

impl core::fmt::Display for ElfFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "it is shorter than an ELF header"),
            Self::NotElf => write!(f, "it does not begin with the ELF magic"),
            Self::NotSixtyFourLittleEndian => write!(
                f,
                "it is not a 64-bit little-endian ELF, which is the only shape this tool reads"
            ),
            Self::NotAarch64 { found } => write!(
                f,
                "it is built for machine type 0x{found:x} and a Rivet runs aarch64 (0x{:x}). \
                 A cargo build for the host produces exactly this: 64-bit, little-endian, and \
                 for the wrong processor. Build with --target aarch64-unknown-linux-musl",
                EM_AARCH64
            ),
            Self::OutOfBounds { what } => write!(f, "its {what} points outside the file"),
            Self::UnmappedStringTable => write!(
                f,
                "its dynamic string table is at an address no loadable segment covers"
            ),
            Self::BadString => write!(f, "a name in its dynamic string table is unreadable"),
        }
    }
}

/// What this tool reads out of a binary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dynamic {
    /// `PT_INTERP` — the loader the kernel will run. `None` means the file is
    /// statically linked, which is every Rivet binary before 2026-08-31.
    pub interpreter: Option<String>,
    /// `DT_NEEDED`, in order. The SONAMEs the loader will look for.
    pub needed: Vec<String>,
    /// `DT_SONAME` — what this object calls itself. Present on a shared object
    /// built with `-Wl,-soname`, absent otherwise, and its absence is the thing
    /// `cargo rivet library` refuses on.
    pub soname: Option<String>,
}

impl Dynamic {
    /// A binary with no interpreter needs nothing at run time, whatever else it
    /// says.
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.interpreter.is_none()
    }
}

/// `EM_AARCH64` — the one `e_machine` a Rivet runs (`ADR-002`).
const EM_AARCH64: u16 = 183;

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;

const DT_NULL: i64 = 0;
const DT_NEEDED: i64 = 1;
const DT_STRTAB: i64 = 5;
const DT_STRSZ: i64 = 10;
const DT_SONAME: i64 = 14;

/// Read [`Dynamic`] out of the bytes of a file.
pub fn read(bytes: &[u8]) -> Result<Dynamic, ElfFault> {
    if bytes.len() < 64 {
        return Err(ElfFault::TooShort);
    }
    if &bytes[0..4] != b"\x7fELF" {
        return Err(ElfFault::NotElf);
    }
    // EI_CLASS = 2 (64-bit), EI_DATA = 1 (little-endian).
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err(ElfFault::NotSixtyFourLittleEndian);
    }
    // `e_machine`, at offset 18. See [`ElfFault::NotAarch64`] for why this was
    // missing and what it cost to leave out.
    let machine = u16_at(bytes, 18)?;
    if machine != EM_AARCH64 {
        return Err(ElfFault::NotAarch64 { found: machine });
    }

    let e_phoff = u64_at(bytes, 32)? as usize;
    let e_phentsize = u16_at(bytes, 54)? as usize;
    let e_phnum = u16_at(bytes, 56)? as usize;

    // A program header table that does not fit is refused rather than
    // truncated: a partial read here would answer "needs nothing" for a file
    // that needs plenty, which is the direction that fails silently.
    let table_len = e_phentsize
        .checked_mul(e_phnum)
        .ok_or(ElfFault::OutOfBounds {
            what: "program header table",
        })?;
    // The entry-size floor is checked only when there are entries. A file with
    // `e_phnum == 0` legitimately carries `e_phentsize == 0`, and refusing it
    // would refuse a relocatable object rather than answering the honest
    // "nothing here needs anything".
    if e_phnum > 0 && e_phentsize < 56 {
        return Err(ElfFault::OutOfBounds {
            what: "program header table",
        });
    }
    if e_phoff.checked_add(table_len).is_none_or(|end| end > bytes.len()) {
        return Err(ElfFault::OutOfBounds {
            what: "program header table",
        });
    }

    let mut out = Dynamic::default();
    let mut loads: Vec<(u64, u64, u64)> = Vec::new(); // (vaddr, memsz, offset)
    let mut dynamic: Option<(usize, usize)> = None; // (offset, filesz)

    for index in 0..e_phnum {
        let at = e_phoff + index * e_phentsize;
        let p_type = u32_at(bytes, at)?;
        let p_offset = u64_at(bytes, at + 8)? as usize;
        let p_vaddr = u64_at(bytes, at + 16)?;
        let p_filesz = u64_at(bytes, at + 32)? as usize;
        let p_memsz = u64_at(bytes, at + 40)?;

        match p_type {
            PT_LOAD => loads.push((p_vaddr, p_memsz, p_offset as u64)),
            PT_DYNAMIC => dynamic = Some((p_offset, p_filesz)),
            PT_INTERP => {
                let end = p_offset
                    .checked_add(p_filesz)
                    .ok_or(ElfFault::OutOfBounds { what: "PT_INTERP" })?;
                let slice = bytes
                    .get(p_offset..end)
                    .ok_or(ElfFault::OutOfBounds { what: "PT_INTERP" })?;
                let text = slice.split(|b| *b == 0).next().unwrap_or(&[]);
                out.interpreter = Some(
                    core::str::from_utf8(text)
                        .map_err(|_| ElfFault::BadString)?
                        .to_string(),
                );
            }
            _ => {}
        }
    }

    let Some((dyn_offset, dyn_filesz)) = dynamic else {
        // No PT_DYNAMIC. A fully static binary, and there is nothing more to
        // say about it.
        return Ok(out);
    };

    // Pass one: find the string table, because every name below is an index
    // into it and the entries may appear in any order.
    let mut strtab_vaddr: Option<u64> = None;
    let mut strsz: u64 = 0;
    let entries = dyn_filesz / 16;
    for index in 0..entries {
        let at = dyn_offset + index * 16;
        let tag = i64_at(bytes, at)?;
        let value = u64_at(bytes, at + 8)?;
        match tag {
            DT_NULL => break,
            DT_STRTAB => strtab_vaddr = Some(value),
            DT_STRSZ => strsz = value,
            _ => {}
        }
    }
    let Some(strtab_vaddr) = strtab_vaddr else {
        return Ok(out);
    };

    // A dynamic-section address is a VIRTUAL address, and the file offset it
    // corresponds to is only recoverable through the PT_LOAD that covers it.
    // Getting this wrong reads plausible-looking garbage from the wrong part of
    // the file rather than failing, which is why it is a named error.
    let strtab_offset = loads
        .iter()
        .find(|(vaddr, memsz, _)| strtab_vaddr >= *vaddr && strtab_vaddr < vaddr + memsz)
        .map(|(vaddr, _, offset)| offset + (strtab_vaddr - vaddr))
        .ok_or(ElfFault::UnmappedStringTable)? as usize;

    let strtab_end = strtab_offset
        .checked_add(strsz as usize)
        .filter(|end| *end <= bytes.len())
        .ok_or(ElfFault::OutOfBounds {
            what: "dynamic string table",
        })?;
    let strtab = &bytes[strtab_offset..strtab_end];

    // Pass two: the names.
    for index in 0..entries {
        let at = dyn_offset + index * 16;
        let tag = i64_at(bytes, at)?;
        let value = u64_at(bytes, at + 8)?;
        match tag {
            DT_NULL => break,
            DT_NEEDED => out.needed.push(string_at(strtab, value as usize)?),
            DT_SONAME => out.soname = Some(string_at(strtab, value as usize)?),
            _ => {}
        }
    }

    Ok(out)
}

fn string_at(strtab: &[u8], index: usize) -> Result<String, ElfFault> {
    let rest = strtab.get(index..).ok_or(ElfFault::BadString)?;
    let end = rest.iter().position(|b| *b == 0).ok_or(ElfFault::BadString)?;
    Ok(core::str::from_utf8(&rest[..end])
        .map_err(|_| ElfFault::BadString)?
        .to_string())
}

/// `SHN_XINDEX` — the escape hatch for a file with too many sections to count
/// in `e_shnum`, or a `.shstrtab` index too large for `e_shstrndx`.
const SHN_XINDEX: u16 = 0xffff;

/// The DWARF a program carries, as `(section name, bytes)` for every
/// `.debug_*` section.
///
/// # Why this exists, and what it replaces
///
/// `main::debug_note` used to answer this by searching the file's bytes for
/// the literal `.debug_info`, and its own comment defended that: a search *"can
/// be wrong in one direction only"*, and the cost of a false positive is one
/// sentence.
///
/// **The false positive turned out to be every stripped Rust binary.**
/// Measured 2026-09-04 on `examples/probes/slow-stop-probe`, built `--release`
/// with `strip = "symbols"`: `readelf -S` shows nineteen sections and **not one
/// `.debug_*` among them**, the cross toolchain's `strip` removes nothing
/// further because there is nothing to remove — and the literal `.debug_info`
/// appears in the file twice. It is in `.rodata`, because `std` links the DWARF
/// reader its own backtraces use and that carries the section names as data.
///
/// So the note fired on a binary with no debug information at all, advising a
/// `strip` that would do nothing. That is worse than one wasted sentence: an
/// author who strips, sees the same message, and strips again learns that the
/// warning cannot be acted on, and the next one they ignore is a real one.
///
/// Reading the section header table is thirty lines and cannot be wrong in
/// either direction, which is `AGENTS.md` §3's ranking applied to a check
/// rather than to a boundary.
///
/// # Errors
///
/// [`ElfFault`], and **an unreadable section table is an error rather than an
/// empty answer**. A file with no section headers at all is `Ok(vec![])` — that
/// is a real reading and a legitimate shape — but a table that points outside
/// the file, or a `.shstrtab` index that names no section, means this function
/// does not know, and the two must not arrive at the caller looking alike.
pub fn debug_sections(bytes: &[u8]) -> Result<Vec<(String, u64)>, ElfFault> {
    const WHAT: &str = "section header table";

    let e_shoff = u64_at(bytes, 40)? as usize;
    let e_shentsize = u16_at(bytes, 58)? as usize;
    let e_shnum = u16_at(bytes, 60)? as usize;
    let e_shstrndx = u16_at(bytes, 62)?;

    // A fully stripped file can legitimately have none. `sstrip` and some
    // linkers produce exactly this, and it is not a fault.
    if e_shoff == 0 || e_shnum == 0 {
        return Ok(Vec::new());
    }
    if e_shentsize < 64 {
        return Err(ElfFault::OutOfBounds { what: WHAT });
    }
    // **Not supported, and refused rather than guessed.** A file with 0xff00 or
    // more sections puts the real count in section 0 and the string-table index
    // in its `sh_link`. Nothing this tool packages is remotely near that, and a
    // parser that half-implemented the escape would be wrong on the one file
    // that ever hit it.
    if e_shstrndx == SHN_XINDEX {
        return Err(ElfFault::OutOfBounds { what: WHAT });
    }
    let table_len = e_shentsize
        .checked_mul(e_shnum)
        .ok_or(ElfFault::OutOfBounds { what: WHAT })?;
    if e_shoff.checked_add(table_len).is_none_or(|end| end > bytes.len()) {
        return Err(ElfFault::OutOfBounds { what: WHAT });
    }
    if usize::from(e_shstrndx) >= e_shnum {
        return Err(ElfFault::OutOfBounds { what: WHAT });
    }

    // The section name string table, found through its own section header.
    let strtab_header = e_shoff + usize::from(e_shstrndx) * e_shentsize;
    let strtab_offset = u64_at(bytes, strtab_header + 24)? as usize;
    let strtab_size = u64_at(bytes, strtab_header + 32)? as usize;
    let strtab = strtab_offset
        .checked_add(strtab_size)
        .and_then(|end| bytes.get(strtab_offset..end))
        .ok_or(ElfFault::OutOfBounds { what: ".shstrtab" })?;

    let mut found = Vec::new();
    for index in 0..e_shnum {
        let at = e_shoff + index * e_shentsize;
        let sh_name = u32_at(bytes, at)? as usize;
        let sh_size = u64_at(bytes, at + 32)?;
        let name = string_at(strtab, sh_name)?;
        if name.starts_with(".debug_") || name.starts_with(".zdebug_") {
            found.push((name, sh_size));
        }
    }
    Ok(found)
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, ElfFault> {
    let slice: [u8; 2] = bytes
        .get(at..at + 2)
        .ok_or(ElfFault::OutOfBounds { what: "header" })?
        .try_into()
        .map_err(|_| ElfFault::OutOfBounds { what: "header" })?;
    Ok(u16::from_le_bytes(slice))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, ElfFault> {
    let slice: [u8; 4] = bytes
        .get(at..at + 4)
        .ok_or(ElfFault::OutOfBounds { what: "header" })?
        .try_into()
        .map_err(|_| ElfFault::OutOfBounds { what: "header" })?;
    Ok(u32::from_le_bytes(slice))
}

fn u64_at(bytes: &[u8], at: usize) -> Result<u64, ElfFault> {
    let slice: [u8; 8] = bytes
        .get(at..at + 8)
        .ok_or(ElfFault::OutOfBounds { what: "header" })?
        .try_into()
        .map_err(|_| ElfFault::OutOfBounds { what: "header" })?;
    Ok(u64::from_le_bytes(slice))
}

fn i64_at(bytes: &[u8], at: usize) -> Result<i64, ElfFault> {
    u64_at(bytes, at).map(|v| v as i64)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn something_that_is_not_an_elf_is_refused_by_name() {
        assert_eq!(read(&[0u8; 8]), Err(ElfFault::TooShort));
        assert_eq!(read(&[0u8; 128]), Err(ElfFault::NotElf));
    }

    #[test]
    fn a_thirty_two_bit_elf_is_refused_rather_than_misread() {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 1; // ELFCLASS32
        bytes[5] = 1;
        assert_eq!(read(&bytes), Err(ElfFault::NotSixtyFourLittleEndian));
    }

    #[test]
    fn a_big_endian_elf_is_refused_rather_than_misread() {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 2; // ELFDATA2MSB
        assert_eq!(read(&bytes), Err(ElfFault::NotSixtyFourLittleEndian));
    }

    /// A 64-bit little-endian ELF with no program headers at all: legal to
    /// read, and it says the file needs nothing. That is the answer a static
    /// binary must get, and getting it wrong in the other direction — claiming
    /// a dependency that is not there — would refuse a correct application.
    #[test]
    fn an_elf_with_no_program_headers_is_static_and_needs_nothing() {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
        let dynamic = read(&bytes).unwrap();
        assert!(dynamic.is_static());
        assert!(dynamic.needed.is_empty());
        assert_eq!(dynamic.soname, None);
    }

    /// **A host build is refused, and this is the shape a host build has.**
    ///
    /// `PLAN.md` §123, Wave 4.2. `cargo build` without `--target` produces a
    /// 64-bit little-endian ELF for x86-64, which passed every check this
    /// module made — so it was read, signed and packaged under `ADR-027`
    /// §3.1's aarch64 name, and the first thing to notice would have been the
    /// board answering `ENOEXEC` after a card was written.
    ///
    /// x86-64 and RISC-V, because the refusal has to be about what the file IS
    /// rather than about one architecture somebody remembered.
    #[test]
    fn an_elf_for_the_wrong_processor_is_refused_by_name() {
        const EM_X86_64: u16 = 62;
        const EM_RISCV: u16 = 243;
        for machine in [EM_X86_64, EM_RISCV] {
            let mut bytes = vec![0u8; 128];
            bytes[0..4].copy_from_slice(b"\x7fELF");
            bytes[4] = 2;
            bytes[5] = 1;
            bytes[18..20].copy_from_slice(&machine.to_le_bytes());
            assert_eq!(read(&bytes), Err(ElfFault::NotAarch64 { found: machine }));
        }
    }

    /// The refusal says what to do about it, because the person meeting it has
    /// just run a build that succeeded.
    #[test]
    fn the_wrong_processor_refusal_names_the_flag_that_fixes_it() {
        let said = ElfFault::NotAarch64 { found: 62 }.to_string();
        assert!(said.contains("aarch64-unknown-linux-musl"), "{said}");
        assert!(said.contains("0x3e"), "the machine type it found is not named: {said}");
    }
}
