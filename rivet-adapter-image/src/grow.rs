//! `ADR-018` §10.3: `/state` grows to fill the device it was written to.
//!
//! `ADR-018` §10.2 builds the card image at [the layout's
//! minimum](crate::grow) — 2048 MiB — rather than at the size of some
//! particular card. Everything past that 2048th mebibyte is unallocated on a
//! card of any other size, and this module is the half of §10 that reclaims it.
//!
//! # What a freshly written card actually looks like
//!
//! Measured on a Raspberry Pi 4 on 2026-08-24, from a 2048 MiB image on a
//! 7.42 GiB card. Two facts, and the kernel volunteered the second one:
//!
//! ```text
//! /state                                    232 MiB used of a 254 MiB partition
//! GPT:Primary header thinks Alt. header is not at the end of the disk
//! GPT:4194303 != 15564799
//! ```
//!
//! The image's **backup GPT sits at the end of the image**, which is nowhere
//! near the end of the card, and partition 5 ends where the image ended. Both
//! are the same repair, which is §10.3's own wording.
//!
//! # Three operations, and only one of them is this crate's own code
//!
//! §10.3 left *"who does it and with what"* unresolved and required
//! `ADR-009` §3.2's survey before the choice. `ADR-018` §10.7 records the
//! survey; the outcome is that **no C mechanism is adopted and no filesystem
//! resizer is written**, because the kernel already contains one:
//!
//! | Step | Mechanism | Where it lives |
//! | :-- | :-- | :-- |
//! | 1. Move the GPT and enlarge partition 5 | ~1.5 KiB of buffer arithmetic and three writes | **this module** |
//! | 2. Tell the kernel the partition changed | `BLKPG_RESIZE_PARTITION` | [`GrowMedium::resize_partition`] |
//! | 3. Grow the `ext4` inside it | `EXT4_IOC_RESIZE_FS` | [`GrowMedium::resize_filesystem`] |
//!
//! Step 3 is the one §10.3 called *"emphatically not"* within reach, and it is
//! not reached — `resize2fs` growing a **mounted** filesystem does nothing but
//! issue that ioctl and let `fs/ext4/resize.c` do the work, journalled. Rivet
//! issues the same ioctl and skips the 400 KiB binary. The consequence is that
//! `/state` must be **mounted** while it happens, which is §10.3's *"unless the
//! online path is used deliberately and knowingly"* — taken deliberately, and
//! `ADR-018` §10.7.2 is the knowing part.
//!
//! # Interrupting it, and the one window that cannot be closed
//!
//! §10.3: *"A card pulled mid-resize must still boot … the failure must land
//! somewhere recoverable, and 'the resize did not happen' is recoverable in a
//! way 'the partition table is half-written' is not."*
//!
//! **The write order is most of the answer. It is not all of it, and the
//! remainder is measured rather than argued.**
//!
//! ## What Linux actually does with a damaged primary GPT
//!
//! This module was first written on the assumption every reader repeats: a GPT
//! has two complete copies, and a reader that rejects the primary falls back to
//! the backup. **Linux does not**, and finding that out cost nothing because a
//! loop device was asked before a card was.
//!
//! Measured on Linux 6.6 (WSL2) on 2026-08-24, against a whole, already-grown
//! Rivet card image on a loop device, one damaged structure at a time:
//!
//! | The primary GPT | `/sys/block/…` partitions | `sgdisk -p` |
//! | :-- | --: | :-- |
//! | intact | **5** | the table |
//! | header zeroed, entry array intact | **0** | falls back to the backup |
//! | entry array corrupted, header intact | **0** | falls back to the backup |
//!
//! `block/partitions/efi.c` says why: `good_agpt` is only computed by following
//! *the primary header's* `alternate_lba`, and the fallback that looks at the
//! last LBA directly is behind `force_gpt`, which is off unless `gpt` is on the
//! kernel command line. **A card whose primary GPT is inconsistent has no
//! partitions at all as far as Linux is concerned, and does not boot** — the
//! same experiment reproduced it from the other direction, as an interrupted
//! rewrite whose `/dev/…p5` never appeared. `ADR-018` §10.7.5 records the
//! question this raises about `cmdline.txt`.
//!
//! ## So the order is chosen to make the inconsistent window as small as a
//! ## window can be
//!
//! | Order | Written | If the power fails **before** this write |
//! | --: | :-- | :-- |
//! | 1 | backup entry array, 32 sectors at the end of the device | the **old** layout: the primary is untouched, valid, and is what every reader uses |
//! | 2 | backup header, the last sector | the **old** layout, for the same reason |
//! | 3 | **only the primary entry-array sectors whose bytes change** — one, in practice | the **old** layout, still |
//! | 4 | primary header, LBA 1 | **nothing**: the primary's array checksum no longer matches its header. This is the window |
//! | — | after 4 | the **new** layout, in both copies |
//!
//! Steps 3 and 4 are one 512-byte sector each, flushed, back to back. Growing
//! `/state` moves eight bytes — partition 5's ending LBA, at offset 552 of the
//! entry array — so step 3 rewrites the second sector of the array and nothing
//! else. **The exposure is the interval between two adjacent single-sector
//! writes**, once in a card's life, and it is the same exposure `sgdisk`,
//! `parted` and `growpart` all carry, because a GPT cannot be committed in
//! fewer than two places.
//!
//! **It is recoverable without re-imaging, and the recovery was measured
//! rather than assumed.** A card pulled inside the window, put in a host and
//! given `sgdisk -e`, came back with **five partitions and a clean `e2fsck`,
//! at the layout it had before the grow** — so it boots, and the next boot
//! grows it. Not the grown layout, and the reason is the same clause as
//! everything else on this page: the stale primary header still points its
//! `alternate_lba` at the image's original backup, and that is the copy a
//! repair follows. Landing on the *old* whole layout is the better of the two
//! outcomes anyway, because it is the one this operation knows how to finish.
//!
//! [`grow_state`] performs the same repair itself (see [`Source::Backup`]) for
//! any machine that can still reach the code.
//!
//! **Everything else is safe by construction.** An interruption after step 4
//! but before the filesystem is grown leaves the GPT right and the `ext4`
//! small, which is the ordinary starting state of the next boot; the next boot
//! finishes the job, because what is assessed is the device rather than a
//! record of what was attempted.
//!
//! # Idempotent because nothing is remembered
//!
//! §10.3: *"There is no flag file to go stale — the condition is observable
//! from the device itself, which is `ADR-011`: one fact, one place, and the
//! place is the hardware."*
//!
//! So [`grow_state`] writes no marker and reads none. It asks two questions of
//! the machine it is standing on:
//!
//! * does partition 5 end where a partition 5 on **this** device should end?
//! * does the `ext4` inside it have as many blocks as that partition holds?
//!
//! When both answers are yes it returns [`Grew::AlreadyFillsTheDevice`] having
//! written nothing, which is every boot after the first and every boot of a
//! card that was written at its own size with `--device-mib`. There is no
//! "first boot" to detect and no state that can be wrong about it.
//!
//! # It refuses a table it does not recognise
//!
//! `AGENTS.md` §5 — *refuse rather than guess*. This module is **not** a
//! general partition editor and will not become one. It rewrites one entry of
//! one table, and it does so only when the table it read is the one
//! `cmd/rivet-image` writes: a GPT whose checksums hold, whose partition 5
//! is in use and labelled `state`, and where nothing else on the device begins
//! after partition 5 does. Anything else is a [`Refusal`], and a refusal writes
//! nothing at all.
//!
//! That matters more here than anywhere else in this repository, because the
//! failure it prevents is not a wrong answer on a console. It is a partition
//! table over somebody's disk.
//!
//! # A failure here degrades the boot; it never rolls it back
//!
//! Same ruling as the `/state` mount and the seeding of the configuration
//! (`ADR-017` §3.1.7, §3.1.9, `ADR-018` §4.1 and §8): **a rollback cannot
//! repair a `/state` that an update never touches.** So [`grow_state`] returns
//! an outcome rather than an error, no path out of it is `Err`, and
//! [`Grew::is_fault`] is for the caller's report and not for its control flow.

use crate::port::Errno;

// ---------------------------------------------------------------------------
// The format, as constants
// ---------------------------------------------------------------------------

/// GPT's logical block, and the unit every LBA on this page counts in.
///
/// The same number as `cmd/rivet-image`'s `disk::SECTOR_BYTES`, and that
/// crate's `the_grow_and_the_builder_agree_on_the_geometry` fails if the two
/// ever part company — one fact in one place is `ADR-011`, and a comment asking
/// for two copies to be kept in step is not a mechanism.
pub const LBA_BYTES: u64 = 512;

/// Where the primary header lives. Sector 0 is the protective MBR.
pub const PRIMARY_HEADER_LBA: u64 = 1;

/// `EFI PART`, and the first thing checked.
pub const SIGNATURE: [u8; 8] = *b"EFI PART";

/// The smallest header UEFI defines. Everything this module reads is inside it.
pub const MIN_HEADER_BYTES: u32 = 92;

/// One partition entry, as every generator on this planet writes it.
pub const ENTRY_BYTES: u32 = 128;

/// A cap on the entry array, so a corrupt header cannot ask for a huge read.
/// Eight times what `sgdisk` writes.
pub const MAX_ENTRIES: u32 = 1024;

/// 1 MiB, in LBAs — the alignment `cmd/rivet-image` lays every partition on,
/// and the boundary the grown partition's end is pulled down to.
pub const ALIGN_LBA: u64 = 2048;

/// Below this much new room, nothing is done.
///
/// Not a fudge factor: it is the granularity everything else here works in. A
/// card whose spare space is under one alignment unit has nothing to give, and
/// the arithmetic that decides so must not be able to produce a *shrink*.
pub const MIN_GROWTH_BYTES: u64 = 1024 * 1024;

/// The GPT label `cmd/rivet-image` gives partition 5.
///
/// Checked, rather than trusting the partition number alone. The number says
/// where to look; the label is what says the table is the one this project
/// writes (`AGENTS.md` §5).
pub const STATE_LABEL: &str = "state";

/// The partition this module grows. `ADR-018` §4 — one `/state`, not per-slot.
pub const STATE_PARTITION: u32 = rivet_core::state::STATE_PARTITION;

// ---------------------------------------------------------------------------
// CRC-32, because a GPT is four checksums and nothing else holds it together
// ---------------------------------------------------------------------------

/// CRC-32/ISO-HDLC — the one UEFI specifies for both of a GPT's checksums.
///
/// Bit-at-a-time rather than table-driven. The largest buffer it ever sees is a
/// 16 KiB entry array, twice, once per boot of a machine's life; a 1 KiB table
/// to make that faster would be a bigger thing to get wrong than the loop is.
///
/// **Checked against bytes this project did not write.** The test module
/// verifies it against the check value in the CRC catalogue *and* against the
/// two checksums `sgdisk` recorded in the fixture at `fixtures/`, which is the
/// only test here that proves anything about the format rather than about this
/// file's agreement with itself (`AGENTS.md` §4).
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// Reading a header
// ---------------------------------------------------------------------------

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(slice);
    Some(u32::from_le_bytes(buf))
}

fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    let slice = bytes.get(offset..offset + 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(slice);
    Some(u64::from_le_bytes(buf))
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    if let Some(slot) = bytes.get_mut(offset..offset + 4) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    if let Some(slot) = bytes.get_mut(offset..offset + 8) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

/// Field offsets in the GPT header, from UEFI 2.10 table 5-5.
mod header_at {
    pub const HEADER_SIZE: usize = 12;
    pub const HEADER_CRC: usize = 16;
    pub const MY_LBA: usize = 24;
    pub const ALTERNATE_LBA: usize = 32;
    pub const FIRST_USABLE_LBA: usize = 40;
    pub const LAST_USABLE_LBA: usize = 48;
    pub const ENTRY_ARRAY_LBA: usize = 72;
    pub const ENTRY_COUNT: usize = 80;
    pub const ENTRY_SIZE: usize = 84;
    pub const ENTRY_ARRAY_CRC: usize = 88;
}

/// Field offsets within one partition entry, from UEFI 2.10 table 5-6.
mod entry_at {
    pub const TYPE_GUID: usize = 0;
    pub const FIRST_LBA: usize = 32;
    pub const LAST_LBA: usize = 40;
    pub const NAME: usize = 56;
    pub const NAME_BYTES: usize = 72;
}

/// The header fields this module acts on. Everything else in the 512 bytes is
/// carried across untouched, which is why a rewrite preserves the disk GUID
/// without this type ever naming one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub header_bytes: u32,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub entry_array_lba: u64,
    pub entry_count: u32,
    pub entry_bytes: u32,
    pub entry_array_crc: u32,
}

impl Header {
    /// Bytes of entry array this header claims.
    pub const fn entry_array_bytes(&self) -> u64 {
        self.entry_count as u64 * self.entry_bytes as u64
    }

    /// LBAs of entry array this header claims, rounded up.
    pub const fn entry_array_lbas(&self) -> u64 {
        self.entry_array_bytes().div_ceil(LBA_BYTES)
    }

    /// Parse and **verify** one 512-byte sector.
    ///
    /// The checksum is not optional and there is no lenient mode. A GPT header
    /// whose CRC does not hold is a header this module has no business acting
    /// on, and the whole design of the write order below depends on a reader
    /// rejecting exactly what a reader is specified to reject.
    pub fn parse(sector: &[u8]) -> Result<Self, Refusal> {
        if sector.len() < MIN_HEADER_BYTES as usize {
            return Err(Refusal::ShortRead { bytes: sector.len() });
        }
        if sector.get(..8) != Some(&SIGNATURE[..]) {
            return Err(Refusal::NotGpt);
        }
        let header_bytes = u32_at(sector, header_at::HEADER_SIZE).unwrap_or(0);
        if header_bytes < MIN_HEADER_BYTES || header_bytes as usize > sector.len() {
            return Err(Refusal::HeaderSize {
                bytes: header_bytes,
            });
        }

        // The stored CRC is over the header with its own CRC field zeroed.
        let mut copy = sector[..header_bytes as usize].to_vec();
        let stored = u32_at(&copy, header_at::HEADER_CRC).unwrap_or(0);
        put_u32(&mut copy, header_at::HEADER_CRC, 0);
        let computed = crc32(&copy);
        if stored != computed {
            return Err(Refusal::HeaderChecksum { stored, computed });
        }

        let entry_count = u32_at(sector, header_at::ENTRY_COUNT).unwrap_or(0);
        let entry_bytes = u32_at(sector, header_at::ENTRY_SIZE).unwrap_or(0);
        if entry_bytes < ENTRY_BYTES || !entry_bytes.is_multiple_of(8) {
            return Err(Refusal::EntrySize {
                bytes: entry_bytes,
            });
        }
        if entry_count == 0 || entry_count > MAX_ENTRIES {
            return Err(Refusal::EntryCount {
                count: entry_count,
            });
        }

        Ok(Self {
            header_bytes,
            my_lba: u64_at(sector, header_at::MY_LBA).unwrap_or(0),
            alternate_lba: u64_at(sector, header_at::ALTERNATE_LBA).unwrap_or(0),
            first_usable_lba: u64_at(sector, header_at::FIRST_USABLE_LBA).unwrap_or(0),
            last_usable_lba: u64_at(sector, header_at::LAST_USABLE_LBA).unwrap_or(0),
            entry_array_lba: u64_at(sector, header_at::ENTRY_ARRAY_LBA).unwrap_or(0),
            entry_count,
            entry_bytes,
            entry_array_crc: u32_at(sector, header_at::ENTRY_ARRAY_CRC).unwrap_or(0),
        })
    }
}

/// One partition entry, read for the two things a decision needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryView {
    /// 1-based, as `ADR-018` §4 and `autoboot.txt` both number them.
    pub number: u32,
    pub first_lba: u64,
    pub last_lba: u64,
    pub name: String,
}

impl EntryView {
    /// `true` when the type GUID is all zeroes, which is UEFI's "unused".
    fn unused(entry: &[u8]) -> bool {
        entry
            .get(entry_at::TYPE_GUID..entry_at::TYPE_GUID + 16)
            .is_none_or(|guid| guid.iter().all(|b| *b == 0))
    }

    /// The label, decoded from UTF-16LE and stopping at the first NUL.
    ///
    /// Lossy on purpose. The label is compared against one ASCII word and
    /// otherwise only reported, so an unpaired surrogate written by some other
    /// tool must produce a refusal a human can read rather than an error path
    /// of its own.
    fn label(entry: &[u8]) -> String {
        let Some(raw) = entry.get(entry_at::NAME..entry_at::NAME + entry_at::NAME_BYTES) else {
            return String::new();
        };
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }
}

/// A whole table as it was read, and which of the two copies answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub source: Source,
    pub header: Header,
    /// The header sector, byte for byte. A rewrite patches a copy of this, so
    /// every field this module does not understand survives untouched.
    pub header_sector: Vec<u8>,
    /// The entry array, byte for byte, `entry_count * entry_bytes` long.
    pub entries: Vec<u8>,
}

/// Which copy of the GPT was readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The header at LBA 1. The ordinary case.
    Primary,
    /// The header at the last LBA, because the primary would not verify.
    ///
    /// **This is the recovery half of the write order.** An interruption
    /// between steps 3 and 4 of the module documentation's table leaves exactly
    /// this state, and the repair is to write both copies again — which is what
    /// the plan does anyway.
    Backup,
}

impl Table {
    /// The `n`-th entry's bytes, 1-based like the partition number.
    fn entry(&self, number: u32) -> Option<&[u8]> {
        let index = (number as usize).checked_sub(1)?;
        let width = self.header.entry_bytes as usize;
        self.entries.get(index * width..index * width + width)
    }

    /// Every entry that is in use.
    pub fn used(&self) -> Vec<EntryView> {
        (1..=self.header.entry_count)
            .filter_map(|number| {
                let entry = self.entry(number)?;
                if EntryView::unused(entry) {
                    return None;
                }
                Some(EntryView {
                    number,
                    first_lba: u64_at(entry, entry_at::FIRST_LBA)?,
                    last_lba: u64_at(entry, entry_at::LAST_LBA)?,
                    name: EntryView::label(entry),
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// Why this module declined to touch the device. **Nothing is written when one
/// of these is produced**, including the cases that look like damage: repairing
/// a table this module does not recognise is not a thing it is allowed to
/// attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The device gave back fewer bytes than a header occupies.
    ShortRead { bytes: usize },
    /// No `EFI PART` at LBA 1 or at the last LBA. An MBR-partitioned disk, a
    /// disk with no table at all, or the wrong device.
    NotGpt,
    /// A header size UEFI does not permit.
    HeaderSize { bytes: u32 },
    /// Neither copy's header checksum held.
    HeaderChecksum { stored: u32, computed: u32 },
    /// An entry size smaller than UEFI's minimum, or not a multiple of 8.
    EntrySize { bytes: u32 },
    /// An entry count of zero, or more than [`MAX_ENTRIES`].
    EntryCount { count: u32 },
    /// The entry array does not match the checksum its own header records.
    EntryArrayChecksum { stored: u32, computed: u32 },
    /// Partition 5's entry is unused. Not a Rivet card.
    StateUnused,
    /// Partition 5 exists and is labelled something else.
    StateLabel { got: String },
    /// Some other partition starts after `/state` does, so growing `/state`
    /// would run over it. A Rivet card has nothing there; a repartitioned one
    /// might.
    SomethingAfterState { number: u32, first_lba: u64 },
    /// The device is too small to hold the table that is on it — which means
    /// the device shrank, or this is not the device the table came from.
    DeviceTooSmall { device_lbas: u64, needs_lbas: u64 },
    /// Recovering from the backup, the primary array's fixed location —
    /// immediately after the primary header — would still overrun the first
    /// usable sector this table itself claims. A table this module can rebuild
    /// a primary for has room between its header and its first partition; one
    /// that does not is not a table this module recognises.
    PrimaryEntriesOverrunFirstUsable {
        entries_end_lba: u64,
        first_usable_lba: u64,
    },
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortRead { bytes } => {
                write!(f, "the device returned {bytes} bytes where a GPT header needs {MIN_HEADER_BYTES}")
            }
            Self::NotGpt => write!(f, "neither LBA 1 nor the last LBA carries a GPT signature"),
            Self::HeaderSize { bytes } => write!(f, "the GPT header claims to be {bytes} bytes"),
            Self::HeaderChecksum { stored, computed } => write!(
                f,
                "the GPT header checksum does not hold: it records {stored:#010x}, the bytes give {computed:#010x}"
            ),
            Self::EntrySize { bytes } => {
                write!(f, "a partition entry of {bytes} bytes is not one this reads")
            }
            Self::EntryCount { count } => {
                write!(f, "a table of {count} entries is outside 1..={MAX_ENTRIES}")
            }
            Self::EntryArrayChecksum { stored, computed } => write!(
                f,
                "the partition entries do not match their header: it records {stored:#010x}, the bytes give {computed:#010x}"
            ),
            Self::StateUnused => write!(
                f,
                "partition {STATE_PARTITION} is unused, so this is not a card this project wrote"
            ),
            Self::StateLabel { got } => write!(
                f,
                "partition {STATE_PARTITION} is labelled {got:?} rather than {STATE_LABEL:?}"
            ),
            Self::SomethingAfterState { number, first_lba } => write!(
                f,
                "partition {number} starts at LBA {first_lba}, after /state does, so /state cannot be extended"
            ),
            Self::DeviceTooSmall {
                device_lbas,
                needs_lbas,
            } => write!(
                f,
                "the device is {device_lbas} sectors and the table on it needs {needs_lbas}"
            ),
            Self::PrimaryEntriesOverrunFirstUsable {
                entries_end_lba,
                first_usable_lba,
            } => {
                let primary_entries_lba = PRIMARY_HEADER_LBA + 1;
                write!(
                    f,
                    "recovering the primary array at LBA {primary_entries_lba} would end at {entries_end_lba}, at or past first_usable_lba {first_usable_lba}"
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// Everything the rewrite needs, decided from the table and the device size
/// alone. Pure: no I/O produced it and none is needed to check it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub device_lbas: u64,
    /// Where the backup header must end up: the very last LBA.
    pub backup_header_lba: u64,
    /// Where the backup entry array must end up: immediately before it.
    pub backup_entries_lba: u64,
    /// Where the primary entry array belongs: never moved when the table came
    /// from the primary, and rebuilt at its fixed location — immediately after
    /// the primary header — when the table came from the backup instead.
    pub primary_entries_lba: u64,
    /// The new `last_usable_lba` for both headers.
    pub last_usable_lba: u64,
    pub state_first_lba: u64,
    pub state_last_lba_now: u64,
    /// Aligned down to [`ALIGN_LBA`], exactly as `cmd/rivet-image` places the
    /// same partition's end when it builds at a card's size.
    pub state_last_lba_new: u64,
    /// `false` when both headers already say what they should and partition 5
    /// already ends where it should. Then nothing is written.
    pub table_needs_writing: bool,
}

impl Plan {
    /// Byte offset of `/state`, for `BLKPG`.
    pub const fn state_start_bytes(&self) -> u64 {
        self.state_first_lba * LBA_BYTES
    }

    /// Byte length `/state` will have, for `BLKPG` and for sizing the `ext4`.
    pub const fn state_bytes(&self) -> u64 {
        (self.state_last_lba_new - self.state_first_lba + 1) * LBA_BYTES
    }

    pub const fn state_lbas_now(&self) -> u64 {
        self.state_last_lba_now - self.state_first_lba + 1
    }

    pub const fn state_lbas_new(&self) -> u64 {
        self.state_last_lba_new - self.state_first_lba + 1
    }
}

/// The last LBA a partition may occupy on a device of `device_lbas` sectors
/// whose GPT entry array is `entry_array_lbas` long.
///
/// One header at the last sector, its entry array immediately before it.
pub const fn last_usable(device_lbas: u64, entry_array_lbas: u64) -> u64 {
    device_lbas - 2 - entry_array_lbas
}

/// `last_usable_lba` pulled down to the previous [`ALIGN_LBA`] boundary.
///
/// **This is `cmd/rivet-image`'s rule, restated so that both paths produce
/// the same card.** That crate's `disk::plan` places the remainder partition's
/// end the same way when it builds at a card's size, and its
/// `the_grow_and_the_builder_place_the_last_partition_identically` fails if the
/// two ever part company. It is not cosmetic: it is the only reason
/// `ADR-018` §10.4's `--device-mib` escape hatch and §10.3's grow can be said
/// to reach the same place.
pub const fn aligned_end(last_usable_lba: u64) -> u64 {
    // Saturating, because a device too small to hold one alignment unit would
    // otherwise underflow to `u64::MAX` and describe a partition off the end of
    // it. `assess` refuses the zero this produces.
    ((last_usable_lba + 1) / ALIGN_LBA * ALIGN_LBA).saturating_sub(1)
}

/// Decide what to do, from a table and a device size. **Pure.**
///
/// The alignment rule is `cmd/rivet-image`'s, restated: the last partition's
/// end is pulled down to a 1 MiB boundary so that a grown card's partition 5
/// ends exactly where a card built at that size would have put it. That is not
/// cosmetic — it is what makes the two paths produce the same card, which is
/// the only reason `--device-mib` can stay as §10.4's escape hatch.
pub fn assess(table: &Table, device_lbas: u64) -> Result<Plan, Refusal> {
    let entry_lbas = table.header.entry_array_lbas();
    // One header at the last LBA, its entry array immediately before it — and
    // saturating, because `first_usable_lba` came off a disk.
    let needs_lbas = entry_lbas
        .saturating_add(2)
        .saturating_add(table.header.first_usable_lba);
    if device_lbas < needs_lbas {
        return Err(Refusal::DeviceTooSmall {
            device_lbas,
            needs_lbas,
        });
    }
    let backup_header_lba = device_lbas - 1;
    let backup_entries_lba = backup_header_lba - entry_lbas;
    let last_usable_lba = last_usable(device_lbas, entry_lbas);

    // **The primary array's own location.** Ordinarily this is simply what the
    // primary's own header already says. When the table was read from the
    // BACKUP instead, that field names the *backup's* array — its own neighbour
    // some 33 sectors from the end of the device — and using it here would have
    // both headers point at the same, single copy of the array rather than
    // rebuilding an independent primary one. The primary array's location is
    // never a fact this module reads off a disk: it is always the primary
    // header's own neighbour, one sector after it.
    let primary_entries_lba = match table.source {
        Source::Primary => table.header.entry_array_lba,
        Source::Backup => PRIMARY_HEADER_LBA + 1,
    };
    if table.source == Source::Backup {
        let entries_end_lba = primary_entries_lba.saturating_add(entry_lbas);
        if entries_end_lba > table.header.first_usable_lba {
            return Err(Refusal::PrimaryEntriesOverrunFirstUsable {
                entries_end_lba,
                first_usable_lba: table.header.first_usable_lba,
            });
        }
    }

    let used = table.used();
    let state = used
        .iter()
        .find(|e| e.number == STATE_PARTITION)
        .ok_or(Refusal::StateUnused)?;
    if state.name != STATE_LABEL {
        return Err(Refusal::StateLabel {
            got: state.name.clone(),
        });
    }
    if let Some(after) = used
        .iter()
        .find(|e| e.number != STATE_PARTITION && e.first_lba > state.first_lba)
    {
        return Err(Refusal::SomethingAfterState {
            number: after.number,
            first_lba: after.first_lba,
        });
    }

    // **The partition that is already there has to FIT on this device**, and
    // this is the check that was missing.
    //
    // `needs_lbas` above asks only whether the device can hold the TABLE — two
    // headers, an entry array and the first usable sector, which is 68 sectors
    // and is true of anything. It says nothing about partition 5, and partition
    // 5 is the one that can be too big: a card imaged from a larger one, or a
    // "16 GB" card replaced by another vendor's "16 GB" card with fewer
    // sectors, arrives here with `state.last_lba` ABOVE `last_usable_lba`.
    //
    // `.max(state.last_lba)` below then keeps it there and the plan is written
    // out with a header whose `last_usable_lba` is BELOW the last partition it
    // describes — a GPT every tool refuses and `sgdisk` calls corrupt, produced
    // by a step whose whole purpose is to leave a card bootable.
    //
    // **The guard that was here could not fire.** It asked whether
    // `state_last_lba_new < state.first_lba`, and `state_last_lba_new` is a
    // `max` with `state.last_lba` in it, so on any entry where the last sector
    // is at or after the first — which is every entry a GPT can express — the
    // answer is always no. It read as a bounds check and was dead code.
    if state.last_lba > last_usable_lba {
        return Err(Refusal::DeviceTooSmall {
            device_lbas,
            // What the device would have to be for the table ON it to be
            // describable: partition 5's last sector, plus its own entry array
            // and header above it. `DeviceTooSmall`'s sentence — "the device is
            // N sectors and the table on it needs M" — is exactly this.
            needs_lbas: state
                .last_lba
                .saturating_add(2)
                .saturating_add(entry_lbas),
        });
    }

    // Align down, exactly as `cmd/rivet-image`'s `plan` does. Never below
    // where the partition already ends: this only ever grows.
    let state_last_lba_new = aligned_end(last_usable_lba).max(state.last_lba);
    if state_last_lba_new < state.first_lba {
        // Reachable only through an entry whose last sector is before its
        // first, which no card this project writes carries and which the check
        // above does not cover.
        return Err(Refusal::DeviceTooSmall {
            device_lbas,
            needs_lbas,
        });
    }

    let grows = (state_last_lba_new - state.last_lba) * LBA_BYTES >= MIN_GROWTH_BYTES;
    let headers_are_right = table.source == Source::Primary
        && table.header.my_lba == PRIMARY_HEADER_LBA
        && table.header.alternate_lba == backup_header_lba
        && table.header.last_usable_lba == last_usable_lba;

    Ok(Plan {
        device_lbas,
        backup_header_lba,
        backup_entries_lba,
        primary_entries_lba,
        last_usable_lba,
        state_first_lba: state.first_lba,
        state_last_lba_now: state.last_lba,
        state_last_lba_new,
        table_needs_writing: grows || !headers_are_right,
    })
}

/// The blobs the rewrite writes, in the order it writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub entries: Vec<u8>,
    pub backup_header: Vec<u8>,
    pub primary_header: Vec<u8>,
}

/// The sectors of the primary entry array whose bytes actually change, as
/// `(offset in sectors from the array's start, the sector)`.
///
/// **This is the window-narrowing step, not an optimisation.** Between the
/// array changing and the header committing, Linux sees no partitions at all
/// (see the module documentation), so the fewer sectors that have to land in
/// that interval, the smaller it is. Growing `/state` changes eight bytes and
/// therefore one sector; a table this module has decided to rewrite for some
/// other reason may change none, and then step 3 writes nothing.
pub fn changed_entry_sectors(old: &[u8], new: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let width = LBA_BYTES as usize;
    new.chunks(width)
        .enumerate()
        .filter_map(|(n, chunk)| {
            let before = old.get(n * width..n * width + chunk.len());
            if before == Some(chunk) {
                return None;
            }
            let mut sector = chunk.to_vec();
            sector.resize(width, 0);
            Some((n as u64, sector))
        })
        .collect()
}

/// Build the new bytes. **Pure**, and the only place a GPT is composed.
pub fn rewrite(table: &Table, plan: &Plan) -> Rewrite {
    let mut entries = table.entries.clone();
    let width = table.header.entry_bytes as usize;
    let offset = (STATE_PARTITION as usize - 1) * width + entry_at::LAST_LBA;
    put_u64(&mut entries, offset, plan.state_last_lba_new);
    let entries_crc = crc32(&entries);

    let header = |my_lba: u64, alternate_lba: u64, entry_array_lba: u64| -> Vec<u8> {
        let mut sector = table.header_sector.clone();
        put_u64(&mut sector, header_at::MY_LBA, my_lba);
        put_u64(&mut sector, header_at::ALTERNATE_LBA, alternate_lba);
        put_u64(&mut sector, header_at::LAST_USABLE_LBA, plan.last_usable_lba);
        put_u64(&mut sector, header_at::ENTRY_ARRAY_LBA, entry_array_lba);
        put_u32(&mut sector, header_at::ENTRY_ARRAY_CRC, entries_crc);
        put_u32(&mut sector, header_at::HEADER_CRC, 0);
        let size = table.header.header_bytes as usize;
        let crc = sector.get(..size).map_or(0, crc32);
        put_u32(&mut sector, header_at::HEADER_CRC, crc);
        sector
    };

    Rewrite {
        backup_header: header(
            plan.backup_header_lba,
            PRIMARY_HEADER_LBA,
            plan.backup_entries_lba,
        ),
        primary_header: header(
            PRIMARY_HEADER_LBA,
            plan.backup_header_lba,
            plan.primary_entries_lba,
        ),
        entries,
    }
}

// ---------------------------------------------------------------------------
// The filesystem half
// ---------------------------------------------------------------------------

/// Where an `ext4` superblock sits, measured from the start of the filesystem.
pub const SUPERBLOCK_OFFSET_BYTES: u64 = 1024;

/// `s_magic`. A filesystem that does not carry it is not one this grows.
pub const EXT4_MAGIC: u16 = 0xEF53;

/// `INCOMPAT_64BIT`, which is what makes `s_blocks_count_hi` meaningful.
const INCOMPAT_64BIT: u32 = 0x0080;

/// Field offsets inside the `ext4` superblock.
mod super_at {
    pub const BLOCKS_COUNT_LO: usize = 0x04;
    pub const LOG_BLOCK_SIZE: usize = 0x18;
    pub const MAGIC: usize = 0x38;
    pub const FEATURE_INCOMPAT: usize = 0x60;
    pub const BLOCKS_COUNT_HI: usize = 0x150;
}

/// How big the `ext4` on `/state` is, **as its own superblock records it**.
///
/// # Why not `statvfs(3)`, which was tried first and is wrong here
///
/// Measured on a loop device on 2026-08-24: a 254 MiB `/state` whose superblock
/// says **260096** blocks is reported by `statvfs` as **237607**. `ext4_statfs`
/// subtracts the filesystem's own overhead, so `f_blocks` is what a *user* can
/// have and not what the filesystem *is*.
///
/// Using it made the second run of [`grow_state`] issue a second resize for a
/// filesystem that was already the right size — an operation that is idempotent
/// in the kernel and so did no damage, and which would have gone on doing
/// nothing visible on every boot for ever. `ADR-018` §10.3 asks for a mechanism
/// that *"does nothing otherwise"*, and the difference between "does nothing"
/// and "does nothing observable" is the difference between the two readings.
///
/// The superblock is authoritative, is 1024 bytes into the partition, and is
/// read over the same seam as the GPT (`AGENTS.md` §5 — the authority for a
/// fact is asked for it directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsSize {
    pub block_bytes: u64,
    /// `s_blocks_count`, both halves.
    pub blocks: u64,
}

/// Read [`FsSize`] out of an `ext4` superblock. **Pure.**
pub fn parse_superblock(sb: &[u8]) -> Result<FsSize, FsRefusal> {
    let magic = sb
        .get(super_at::MAGIC..super_at::MAGIC + 2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .unwrap_or(0);
    if magic != EXT4_MAGIC {
        return Err(FsRefusal::NotExt4 { magic });
    }
    let log = u32_at(sb, super_at::LOG_BLOCK_SIZE).unwrap_or(u32::MAX);
    if log > 6 {
        return Err(FsRefusal::BlockSize { bytes: 0 });
    }
    let block_bytes = 1024u64 << log;
    let incompat = u32_at(sb, super_at::FEATURE_INCOMPAT).unwrap_or(0);
    let lo = u64::from(u32_at(sb, super_at::BLOCKS_COUNT_LO).unwrap_or(0));
    let hi = if incompat & INCOMPAT_64BIT != 0 {
        u64::from(u32_at(sb, super_at::BLOCKS_COUNT_HI).unwrap_or(0))
    } else {
        0
    };
    Ok(FsSize {
        block_bytes,
        blocks: lo | (hi << 32),
    })
}

/// What to do about the filesystem, once the partition is the right size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsPlan {
    /// It already spans its partition, to within one alignment unit.
    AlreadyFills,
    Resize { from: u64, to: u64, block_bytes: u64 },
    Refused(FsRefusal),
}

/// Why the filesystem half declined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsRefusal {
    /// No `0xEF53` where a superblock should be. The partition holds something
    /// that is not an `ext4`, and enlarging the *partition* around it is still
    /// safe — enlarging the filesystem is not, so it is not attempted.
    NotExt4 { magic: u16 },
    /// A block size `ext4` cannot have. A partition full of zeroes reads as
    /// zero, and that must not become a division.
    BlockSize { bytes: u64 },
    /// The filesystem claims more blocks than its partition holds. **Never
    /// shrunk** — this module only ever grows, and a filesystem that is already
    /// too big is a fault to report rather than a thing to trim.
    LargerThanItsPartition { blocks: u64, room: u64 },
}

impl core::fmt::Display for FsRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotExt4 { magic } => write!(
                f,
                "partition {STATE_PARTITION} carries {magic:#06x} where an ext4 superblock's magic should be"
            ),
            Self::BlockSize { bytes } => write!(
                f,
                "/state reports a block size of {bytes} bytes, which is not one ext4 has"
            ),
            Self::LargerThanItsPartition { blocks, room } => write!(
                f,
                "/state already claims {blocks} blocks and its partition holds {room}; this only ever grows"
            ),
        }
    }
}

/// Decide the filesystem half. **Pure.**
pub fn assess_filesystem(size: FsSize, partition_bytes: u64) -> FsPlan {
    // 1 KiB to 64 KiB, powers of two — every block size ext4 permits.
    let plausible = matches!(size.block_bytes, 1024 | 2048 | 4096 | 8192 | 16384 | 32768 | 65536);
    if !plausible {
        return FsPlan::Refused(FsRefusal::BlockSize {
            bytes: size.block_bytes,
        });
    }
    let room = partition_bytes / size.block_bytes;
    if size.blocks > room {
        return FsPlan::Refused(FsRefusal::LargerThanItsPartition {
            blocks: size.blocks,
            room,
        });
    }
    if (room - size.blocks) * size.block_bytes < MIN_GROWTH_BYTES {
        return FsPlan::AlreadyFills;
    }
    FsPlan::Resize {
        from: size.blocks,
        to: room,
        block_bytes: size.block_bytes,
    }
}

// ---------------------------------------------------------------------------
// The seam
// ---------------------------------------------------------------------------

/// Everything this module asks of the machine, and nothing else.
///
/// Object-safe, like [`crate::port::BootMedium`], and for the same reason: the
/// decisions above it compile once and the tests drive the code a board runs.
///
/// **This crate ships no implementor**, which is the same absence
/// `crate::port` and `rivet-adapter-system` both record and for the same
/// reason: `ioctl(2)` has no `std` equivalent, so a real one would mean taking
/// `libc` here and standing a second syscall seam beside the one the calling
/// process already owns — `ADR-011`'s second authority, over syscalls. The
/// implementor belongs at the composition root of whichever process calls
/// [`grow_state`]; `ADR-018` §10.7.3 names it and says why it is not PID 1.
pub trait GrowMedium {
    /// The size of the **whole disk**, in 512-byte sectors.
    ///
    /// Not of the partition. `ADR-018` §10.1's whole subject is the difference
    /// between the two.
    fn device_lbas(&mut self) -> Result<u64, Errno>;

    /// Read `count` sectors from the whole disk, starting at `lba`.
    fn read_lbas(&mut self, lba: u64, count: u64) -> Result<Vec<u8>, Errno>;

    /// Write `bytes` to the whole disk at `lba`. Always a whole number of
    /// sectors.
    fn write_lbas(&mut self, lba: u64, bytes: &[u8]) -> Result<(), Errno>;

    /// Read `count` sectors from **the `/state` partition**, `lba` counted from
    /// the start of that partition.
    ///
    /// # Why this is not [`read_lbas`](Self::read_lbas) with an offset added
    ///
    /// Because it gave the wrong answer, measured on a loop device on
    /// 2026-08-24. `/state` is **mounted** while this runs, and a mounted
    /// filesystem's writes go through the *partition* block device's page
    /// cache. The whole-disk device has a different one. After a successful
    /// online resize, `dumpe2fs` on the partition reported 5945344 blocks and a
    /// read of the same bytes through the whole disk still reported 260096 —
    /// so the run after it decided the filesystem still needed growing.
    ///
    /// It was harmless in that instance, because `EXT4_IOC_RESIZE_FS` to the
    /// size a filesystem already has is a no-op in the kernel, and because a
    /// reboot empties both caches. It was still a component reading a stale
    /// copy of a fact and acting on it, which is `AGENTS.md` §5's *"the kernel
    /// is the authority for system state; never cache its verdicts"* arriving
    /// by the back door. Two devices, two answers, and the stale one is the one
    /// that got used.
    ///
    /// So the filesystem is read through the device the filesystem is on.
    fn read_state_lbas(&mut self, lba: u64, count: u64) -> Result<Vec<u8>, Errno>;

    /// Make everything written so far durable, and **do not return until it
    /// is**.
    ///
    /// Unlike [`crate::port::BootMedium::sync`] this can fail, and the failure
    /// matters: the write order in this module's documentation is a claim about
    /// what reaches the medium in what order, and a flush that silently did
    /// nothing would turn that claim into a hope. A Pi has no battery.
    fn flush(&mut self) -> Result<(), Errno>;

    /// Tell the running kernel that partition `number` is now `len_bytes` long.
    ///
    /// `BLKPG_RESIZE_PARTITION` on the whole-disk device. **Not `BLKRRPART`**:
    /// re-reading the whole table needs every partition on the disk to be
    /// unused, and `/system` is mounted from this one. `start_bytes` is passed
    /// so the kernel can refuse a resize whose start does not match the
    /// partition it already has, which is a check worth having.
    fn resize_partition(
        &mut self,
        number: u32,
        start_bytes: u64,
        len_bytes: u64,
    ) -> Result<(), Errno>;

    /// Grow the mounted `/state` to `blocks` filesystem blocks.
    ///
    /// `EXT4_IOC_RESIZE_FS`. The kernel does the work — `fs/ext4/resize.c`,
    /// journalled — and this is the entirety of what `resize2fs` does when the
    /// filesystem it is given is mounted.
    ///
    /// # The implementor must issue it against a file INSIDE the filesystem
    ///
    /// **Not against the partition device**, and this is stated on the port
    /// rather than left to each implementor because the first one got it wrong
    /// and only a board could say so. An `ioctl` is dispatched through the open
    /// file's own `f_op`: a block device's is the block layer's, which has
    /// never heard of ext4, so the request never arrives and the answer is
    /// `EINVAL`. `resize2fs` opens the **mount point**.
    ///
    /// It is the one method here whose descriptor is not a device, which is
    /// exactly why it is easy to get wrong: every other call on this port is a
    /// read or a write of sectors.
    fn resize_filesystem(&mut self, blocks: u64) -> Result<(), Errno>;
}

// ---------------------------------------------------------------------------
// The outcome
// ---------------------------------------------------------------------------

/// Which step a failure happened at, so a console line can say where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    ReadDeviceSize,
    ReadPrimaryHeader,
    ReadPrimaryEntries,
    ReadBackupHeader,
    ReadBackupEntries,
    WriteBackupEntries,
    WriteBackupHeader,
    WritePrimaryEntries,
    WritePrimaryHeader,
    ResizePartition,
    ReadSuperblock,
    ResizeFilesystem,
}

impl Step {
    pub const fn described(self) -> &'static str {
        match self {
            Self::ReadDeviceSize => "reading the size of the device",
            Self::ReadPrimaryHeader => "reading the GPT header",
            Self::ReadPrimaryEntries => "reading the partition entries",
            Self::ReadBackupHeader => "reading the backup GPT header",
            Self::ReadBackupEntries => "reading the backup partition entries",
            Self::WriteBackupEntries => "writing the backup partition entries",
            Self::WriteBackupHeader => "writing the backup GPT header",
            Self::WritePrimaryEntries => "writing the partition entries",
            Self::WritePrimaryHeader => "writing the GPT header",
            Self::ResizePartition => "telling the kernel the partition grew",
            Self::ReadSuperblock => "reading the ext4 superblock on /state",
            Self::ResizeFilesystem => "growing the filesystem",
        }
    }

    /// `true` once a write to EITHER copy has been issued, so the caller's
    /// report can say whether anything may have reached the device at all — as
    /// opposed to a failure during a read, before any write was attempted.
    ///
    /// This alone does not say which copy: [`Self::is_a_backup_only_write`]
    /// narrows that, because a failure while only the backup may have changed
    /// must not be reported the same way as one after the primary's own header
    /// has committed.
    pub const fn is_after_a_write(self) -> bool {
        matches!(
            self,
            Self::WriteBackupEntries
                | Self::WriteBackupHeader
                | Self::WritePrimaryEntries
                | Self::WritePrimaryHeader
                | Self::ResizePartition
                | Self::ResizeFilesystem
        )
    }

    /// `true` for a failure that can only have reached the BACKUP copy — the
    /// copy nothing reads while the primary still verifies (see the module
    /// documentation's write order). The primary is untouched at these two
    /// steps, so a failure here is not "the table was already rewritten".
    pub const fn is_a_backup_only_write(self) -> bool {
        matches!(self, Self::WriteBackupEntries | Self::WriteBackupHeader)
    }
}

/// What [`grow_state`] did. **There is no `Err`** — `ADR-018` §4.1 and §8
/// degrade the boot rather than roll it back, so every path out of the
/// operation is one of these and the caller reports rather than branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grew {
    /// Nothing was written, because nothing needed to be. Every boot after the
    /// first, and every boot of a card written with `--device-mib`.
    AlreadyFillsTheDevice,
    /// Both halves done.
    Grown {
        partition_from_lbas: u64,
        partition_to_lbas: u64,
        fs_from_blocks: u64,
        fs_to_blocks: u64,
        block_bytes: u64,
    },
    /// The partition table now reaches the end of the device and the
    /// filesystem did not follow. **Recoverable and self-correcting**: the next
    /// boot assesses the device again and finds the partition already right and
    /// the filesystem still small.
    PartitionOnly {
        partition_from_lbas: u64,
        partition_to_lbas: u64,
        why: FsRefusal,
    },
    /// The table is not one this module will touch. Nothing was written.
    Refused { why: Refusal },
    /// The machine refused an operation. `at` says which,
    /// [`Step::is_after_a_write`] says whether anything reached the device at
    /// all, and [`Step::is_a_backup_only_write`] narrows that to whether only
    /// the copy nothing boots from may have changed.
    Failed { at: Step, errno: Errno },
}

impl Grew {
    /// `true` when the operator should look at the card. **Not** a reason to
    /// withhold the boot confirmation.
    pub const fn is_fault(&self) -> bool {
        matches!(
            self,
            Self::Refused { .. } | Self::Failed { .. } | Self::PartitionOnly { .. }
        )
    }

    /// One line, for a console or a journal entry.
    pub fn summary(&self) -> String {
        match self {
            Self::AlreadyFillsTheDevice => {
                "/state already fills the device; nothing was written".to_string()
            }
            Self::Grown {
                partition_from_lbas,
                partition_to_lbas,
                fs_from_blocks,
                fs_to_blocks,
                block_bytes,
            } => format!(
                "/state grew from {} MiB to {} MiB ({} to {} blocks of {block_bytes} bytes)",
                mib(partition_from_lbas * LBA_BYTES),
                mib(partition_to_lbas * LBA_BYTES),
                fs_from_blocks,
                fs_to_blocks,
            ),
            Self::PartitionOnly {
                partition_from_lbas,
                partition_to_lbas,
                why,
            } if partition_from_lbas == partition_to_lbas => format!(
                "partition {STATE_PARTITION} already fills the device and the filesystem on it was left alone: {why}. The next boot will look again"
            ),
            Self::PartitionOnly {
                partition_from_lbas,
                partition_to_lbas,
                why,
            } => format!(
                "partition {STATE_PARTITION} grew from {} MiB to {} MiB and the filesystem did not follow: {why}. The next boot will look again",
                mib(partition_from_lbas * LBA_BYTES),
                mib(partition_to_lbas * LBA_BYTES),
            ),
            Self::Refused { why } => {
                format!("/state was NOT grown and nothing was written: {why}")
            }
            Self::Failed { at, errno } => format!(
                "/state was NOT grown: {} failed with {errno}{}",
                at.described(),
                if at.is_a_backup_only_write() {
                    ". The BACKUP copy of the partition table may have changed; the copy a boot reads was not touched"
                } else if at.is_after_a_write() {
                    ". The partition table was already rewritten; the next boot continues from there"
                } else {
                    ". Nothing was written"
                }
            ),
        }
    }
}

fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

// ---------------------------------------------------------------------------
// The operation
// ---------------------------------------------------------------------------

/// Internal early-exit, so the body below reads as the sequence it is.
enum Stop {
    Refused(Refusal),
    Failed(Step, Errno),
}

impl From<Stop> for Grew {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Refused(why) => Self::Refused { why },
            Stop::Failed(at, errno) => Self::Failed { at, errno },
        }
    }
}

fn read_copy(
    m: &mut dyn GrowMedium,
    header_lba: u64,
    steps: (Step, Step),
) -> Result<Result<Table, Refusal>, Stop> {
    let sector = m
        .read_lbas(header_lba, 1)
        .map_err(|e| Stop::Failed(steps.0, e))?;
    let header = match Header::parse(&sector) {
        Ok(h) => h,
        Err(why) => return Ok(Err(why)),
    };
    let entries = m
        .read_lbas(header.entry_array_lba, header.entry_array_lbas())
        .map_err(|e| Stop::Failed(steps.1, e))?;
    let wanted = header.entry_array_bytes() as usize;
    if entries.len() < wanted {
        return Ok(Err(Refusal::ShortRead {
            bytes: entries.len(),
        }));
    }
    let entries = entries[..wanted].to_vec();
    let computed = crc32(&entries);
    if computed != header.entry_array_crc {
        return Ok(Err(Refusal::EntryArrayChecksum {
            stored: header.entry_array_crc,
            computed,
        }));
    }
    Ok(Ok(Table {
        source: if header_lba == PRIMARY_HEADER_LBA {
            Source::Primary
        } else {
            Source::Backup
        },
        header,
        header_sector: sector,
        entries,
    }))
}

/// `ADR-018` §10.3, end to end.
///
/// Reads the table, decides, writes both copies of the GPT in the order the
/// module documentation sets out, tells the kernel, and issues the `ext4`
/// resize. Returns what happened; never fails, never panics, never rolls a boot
/// back.
pub fn grow_state(m: &mut dyn GrowMedium) -> Grew {
    match run(m) {
        Ok(grew) => grew,
        Err(stop) => stop.into(),
    }
}

fn run(m: &mut dyn GrowMedium) -> Result<Grew, Stop> {
    let device_lbas = m
        .device_lbas()
        .map_err(|e| Stop::Failed(Step::ReadDeviceSize, e))?;

    // The primary first, exactly as a kernel does; the backup only when the
    // primary will not verify, which is the state an interruption between
    // steps 3 and 4 of the write order leaves behind.
    let primary = read_copy(
        m,
        PRIMARY_HEADER_LBA,
        (Step::ReadPrimaryHeader, Step::ReadPrimaryEntries),
    )?;
    let table = match primary {
        Ok(table) => table,
        Err(primary_refusal) => {
            let backup = read_copy(
                m,
                device_lbas.saturating_sub(1),
                (Step::ReadBackupHeader, Step::ReadBackupEntries),
            )?;
            match backup {
                Ok(table) => table,
                // The primary's complaint is the one reported: it is the copy a
                // reader prefers, and it is the one a human will look at.
                Err(_) => return Err(Stop::Refused(primary_refusal)),
            }
        }
    };

    let plan = assess(&table, device_lbas).map_err(Stop::Refused)?;

    // The superblock is 1024 bytes into the partition, and is read through the
    // partition's own device — see `GrowMedium::read_state_lbas`. Read
    // **before** anything is written: the decision is made once, from the state
    // the boot found.
    let superblock = m
        .read_state_lbas(
            SUPERBLOCK_OFFSET_BYTES / LBA_BYTES,
            SUPERBLOCK_OFFSET_BYTES / LBA_BYTES,
        )
        .map_err(|e| Stop::Failed(Step::ReadSuperblock, e))?;

    // Measured against the partition the plan *will* make, not the one that is
    // there — on a fresh card the ext4 exactly fills its 254 MiB partition and
    // would look settled against it.
    let measured = parse_superblock(&superblock);
    let fs = match &measured {
        Ok(size) => assess_filesystem(*size, plan.state_bytes()),
        Err(why) => FsPlan::Refused(why.clone()),
    };
    let measured = measured.unwrap_or(FsSize {
        block_bytes: 0,
        blocks: 0,
    });

    if !plan.table_needs_writing {
        match &fs {
            FsPlan::AlreadyFills => return Ok(Grew::AlreadyFillsTheDevice),
            FsPlan::Refused(why) => {
                return Ok(Grew::PartitionOnly {
                    partition_from_lbas: plan.state_lbas_new(),
                    partition_to_lbas: plan.state_lbas_new(),
                    why: why.clone(),
                });
            }
            // The table is right and the filesystem is behind it, which is what
            // an interruption between the two halves leaves. Carry on.
            FsPlan::Resize { .. } => {}
        }
    }

    if plan.table_needs_writing {
        let bytes = rewrite(&table, &plan);

        // 1 and 2: the whole backup, at the end of the device. Nothing reads it
        // while the primary is valid, so this cannot be seen going wrong — and
        // it is complete before the window in 3..4 opens, which is what makes
        // that window recoverable.
        m.write_lbas(plan.backup_entries_lba, &bytes.entries)
            .map_err(|e| Stop::Failed(Step::WriteBackupEntries, e))?;
        m.flush()
            .map_err(|e| Stop::Failed(Step::WriteBackupEntries, e))?;
        m.write_lbas(plan.backup_header_lba, &bytes.backup_header)
            .map_err(|e| Stop::Failed(Step::WriteBackupHeader, e))?;
        m.flush()
            .map_err(|e| Stop::Failed(Step::WriteBackupHeader, e))?;

        // 3: the window opens. When the table came from the PRIMARY, only the
        // sectors that differ are written, so the window is as narrow as it can
        // be made. When it came from the BACKUP, there is no prior primary
        // array at `plan.primary_entries_lba` to diff against — what is there
        // now is either the backup's own array at a different address entirely,
        // or nothing recognisable — so the whole array is written, rebuilding an
        // independent primary copy rather than pointing the primary header at
        // the one copy the backup already carries.
        match table.source {
            Source::Primary => {
                for (offset, sector) in changed_entry_sectors(&table.entries, &bytes.entries) {
                    m.write_lbas(plan.primary_entries_lba + offset, &sector)
                        .map_err(|e| Stop::Failed(Step::WritePrimaryEntries, e))?;
                }
            }
            Source::Backup => {
                m.write_lbas(plan.primary_entries_lba, &bytes.entries)
                    .map_err(|e| Stop::Failed(Step::WritePrimaryEntries, e))?;
            }
        }
        m.flush()
            .map_err(|e| Stop::Failed(Step::WritePrimaryEntries, e))?;

        // 4: the window closes. The header is the commit record, so it is what
        // lands last — and a torn write of it is the same state as a cut before
        // it, rather than a fifth thing to reason about.
        m.write_lbas(PRIMARY_HEADER_LBA, &bytes.primary_header)
            .map_err(|e| Stop::Failed(Step::WritePrimaryHeader, e))?;
        m.flush()
            .map_err(|e| Stop::Failed(Step::WritePrimaryHeader, e))?;
    }

    // Idempotent in the kernel: setting a partition to the size it already has
    // is accepted and does nothing. Issued whenever anything is left to do, so
    // that a boot which crashed after the table was written still converges the
    // kernel's own view before the filesystem is grown into it.
    m.resize_partition(
        STATE_PARTITION,
        plan.state_start_bytes(),
        plan.state_bytes(),
    )
    .map_err(|e| Stop::Failed(Step::ResizePartition, e))?;

    match fs {
        // The table moved and the filesystem was already right for it — a card
        // whose `/state` was grown by hand, or one written with `--device-mib`
        // and then read on a slightly different device.
        FsPlan::AlreadyFills => Ok(Grew::Grown {
            partition_from_lbas: plan.state_lbas_now(),
            partition_to_lbas: plan.state_lbas_new(),
            fs_from_blocks: measured.blocks,
            fs_to_blocks: measured.blocks,
            block_bytes: measured.block_bytes,
        }),
        FsPlan::Refused(why) => Ok(Grew::PartitionOnly {
            partition_from_lbas: plan.state_lbas_now(),
            partition_to_lbas: plan.state_lbas_new(),
            why,
        }),
        FsPlan::Resize {
            from,
            to,
            block_bytes,
        } => {
            m.resize_filesystem(to)
                .map_err(|e| Stop::Failed(Step::ResizeFilesystem, e))?;
            Ok(Grew::Grown {
                partition_from_lbas: plan.state_lbas_now(),
                partition_to_lbas: plan.state_lbas_new(),
                fs_from_blocks: from,
                fs_to_blocks: to,
                block_bytes,
            })
        }
    }
}

#[cfg(test)]
mod tests;
