//! **Reading the GPT off a card image** — enough of it to find a partition by
//! its label, and no more.
//!
//! # Why this exists rather than a crate
//!
//! The first version of `cargo rivet provision` read an **MBR** and indexed
//! partitions 1 and 2. Rivet card images are **GPT** — `sgdisk -p` on one
//! reports `Disk identifier (GUID): FEEDFACE-…` and a 128-entry table — so what
//! it actually read was the *protective* MBR: a single entry of type `0xEE`
//! covering the whole disk. It then seeked to byte 512, which is the GPT header
//! rather than a FAT boot sector, and looked for slot B in an entry that is
//! always empty. The code could not work on any image this project produces.
//!
//! `tools/update/build.sh` says it in its own header — *"`sgdisk -p` reads the
//! GPT out of the image file"* — and asserts the partitions against their GPT
//! labels rather than trusting their order.
//!
//! Parsing what we need is about a hundred lines, so this replaces the
//! `mbrman` dependency rather than swapping it for a GPT one.
//!
//! # What it deliberately does NOT do
//!
//! **It does not write.** Nothing here can damage a table. `cmd/rivet-image`
//! owns creating one (`disk.rs`), and a second writer would be a second
//! statement of `ADR-018` §4's layout.
//!
//! **It does not validate the CRCs.** A card image this tool is handed came out
//! of `tools/card/build.sh` minutes earlier; a corrupt table shows up as a
//! partition whose label does not match, which is the check that matters here
//! and is made below. Verifying a CRC would be a claim about media integrity
//! that this tool is in no position to make.
//!
//! **It does not assume the order.** `ADR-018` §4 fixes p1 boot-a, p2 boot-b,
//! p3 system-a, p4 system-b, p5 state — and this looks partitions up **by
//! label** anyway, for `tools/update/build.sh`'s stated reason: an image built
//! by a future layout would otherwise be silently mis-read.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// The sector size every Rivet image is built at — `cmd/rivet-image`'s
/// `disk::SECTOR_BYTES`. Spelled again here because this crate does not depend
/// on that one, and a card whose sectors were a different size is not a card
/// this project made.
pub const SECTOR_BYTES: u64 = 512;

/// The signature at the start of the GPT header, `"EFI PART"`.
const SIGNATURE: &[u8; 8] = b"EFI PART";

/// One partition, as the table states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// The GPT partition name, UTF-16LE in the table, decoded here.
    ///
    /// This is what `ADR-018` §4 calls a partlabel and what the kernel exposes
    /// as `PARTLABEL=` — the string `cmdline.txt` names to find its root.
    pub label: String,
    /// First LBA, inclusive.
    pub first_lba: u64,
    /// Last LBA, **inclusive**, as GPT and `sgdisk` both mean it.
    pub last_lba: u64,
}

impl Partition {
    /// Where this partition starts, in bytes.
    #[must_use]
    pub const fn offset_bytes(&self) -> u64 {
        self.first_lba * SECTOR_BYTES
    }

    /// How long it is, in bytes. Inclusive `last_lba`, hence the `+ 1`.
    #[must_use]
    pub const fn len_bytes(&self) -> u64 {
        (self.last_lba - self.first_lba + 1) * SECTOR_BYTES
    }
}

/// Read the primary GPT and return every partition that has one.
///
/// # Errors
///
/// A file that cannot be read, or one whose header is not a GPT — which is the
/// answer an operator most needs, because it is what an MBR image, a raw
/// filesystem, or a truncated download all look like.
pub fn read(image: &mut File) -> Result<Vec<Partition>, String> {
    // The header is LBA 1. LBA 0 is the protective MBR, which is exactly what
    // the first version of this tool mistook for the real table.
    let mut header = [0u8; 92];
    image
        .seek(SeekFrom::Start(SECTOR_BYTES))
        .map_err(|e| format!("cannot seek to the GPT header: {e}"))?;
    image
        .read_exact(&mut header)
        .map_err(|e| format!("cannot read the GPT header: {e}"))?;

    if &header[0..8] != SIGNATURE {
        return Err(
            "this file has no GPT header at LBA 1, so it is not a card image this project \
             built. An MBR-partitioned image, a bare filesystem and a truncated download all \
             look like this"
                .to_string(),
        );
    }

    let entry_lba = u64::from_le_bytes(take8(&header, 72)?);
    let count = u32::from_le_bytes(take4(&header, 80)?);
    let entry_size = u32::from_le_bytes(take4(&header, 84)?);

    // **Bounds before allocation.** These three numbers come out of a file, and
    // a corrupt or hostile one naming four billion entries of four billion
    // bytes must be refused rather than multiplied.
    if !(128..=4096).contains(&entry_size) {
        return Err(format!(
            "the GPT says each partition entry is {entry_size} bytes; the specification's \
             minimum is 128 and this refuses anything above 4096 rather than trusting it"
        ));
    }
    if count > 1024 {
        return Err(format!(
            "the GPT claims {count} partition entries, which no card this project builds has; \
             refusing rather than reading that much"
        ));
    }

    let mut out = Vec::new();
    let mut entry = vec![0u8; entry_size as usize];
    image
        .seek(SeekFrom::Start(entry_lba * SECTOR_BYTES))
        .map_err(|e| format!("cannot seek to the partition table: {e}"))?;

    for _ in 0..count {
        image
            .read_exact(&mut entry)
            .map_err(|e| format!("cannot read a partition entry: {e}"))?;

        // An all-zero type GUID is an unused slot. GPT tables are almost
        // entirely unused slots — 128 entries for five partitions — so this is
        // the common case rather than an error.
        if entry[0..16].iter().all(|b| *b == 0) {
            continue;
        }

        let first_lba = u64::from_le_bytes(take8(&entry, 32)?);
        let last_lba = u64::from_le_bytes(take8(&entry, 40)?);
        if last_lba < first_lba {
            return Err(format!(
                "a partition entry says it ends at LBA {last_lba} and starts at {first_lba}"
            ));
        }

        // The name is 72 bytes of UTF-16LE, NUL-padded.
        let mut units = Vec::new();
        for pair in entry[56..128].as_chunks::<2>().0 {
            let unit = u16::from_le_bytes(*pair);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let label = String::from_utf16_lossy(&units);

        out.push(Partition {
            label,
            first_lba,
            last_lba,
        });
    }

    Ok(out)
}

/// Find one partition by its label, or say what was there instead.
///
/// **The refusal lists what it found**, because the failure this guards against
/// is being handed the wrong file — and a reader who is told *"no partition
/// named boot-a"* beside *"this image has: EFI, rootfs"* knows immediately that
/// they pointed at somebody else's card.
///
/// # Errors
///
/// When no partition carries the label.
pub fn by_label<'a>(parts: &'a [Partition], label: &str) -> Result<&'a Partition, String> {
    parts.iter().find(|p| p.label == label).ok_or_else(|| {
        let found: Vec<&str> = parts.iter().map(|p| p.label.as_str()).collect();
        format!(
            "this image has no partition labelled {label:?}. It has: {}",
            if found.is_empty() {
                "nothing".to_string()
            } else {
                found.join(", ")
            }
        )
    })
}

fn take8(buf: &[u8], at: usize) -> Result<[u8; 8], String> {
    buf.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| format!("the GPT structure is shorter than {} bytes", at + 8))
}

fn take4(buf: &[u8], at: usize) -> Result<[u8; 4], String> {
    buf.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| format!("the GPT structure is shorter than {} bytes", at + 4))
}

#[cfg(test)]
#[path = "gpt_tests.rs"]
mod tests;
