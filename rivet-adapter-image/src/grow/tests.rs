//! `ADR-018` §10.3's tests, and one of them is worth more than the rest.
//!
//! `AGENTS.md` §4: *"a test that round-trips a reader against its own writer
//! proves the two agree with each other and nothing about the format."* A GPT
//! crosses an implementation boundary in the sharpest way this project has —
//! `sgdisk` writes it, the Linux kernel reads it, and this module rewrites it
//! in between — so the tests that matter are the ones driven from
//! `fixtures/gpt-from-sgdisk.hex`, which is bytes `sgdisk` wrote.
//!
//! The rest of the file is about decisions and orderings, where a fake is the
//! right instrument because the states being described — a card pulled out
//! mid-write — are ones no machine can be asked to produce on demand.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;

use super::*;

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

const FIXTURE: &str = include_str!("../../fixtures/gpt-from-sgdisk.hex");

/// The device the fixture's table came from, in sectors. 2048 MiB.
const IMAGE_LBAS: u64 = 4 * 1024 * 1024 * 1024 / 512 / 2;

/// A 7.42 GiB card, which is the one `ADR-018` §10.1's successor measured:
/// the kernel printed `GPT:4194303 != 15564799` against exactly this device.
const CARD_LBAS: u64 = 15_564_800;

/// `/state` as the image ships it: 3672064..=4192255.
const STATE_FIRST: u64 = 3_672_064;
const STATE_LAST_ON_IMAGE: u64 = 4_192_255;

fn section(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in FIXTURE.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{name}]");
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bytes: Vec<u8> = line
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let text = std::str::from_utf8(pair).expect("hex is ascii");
                u8::from_str_radix(text, 16).expect("hex digit")
            })
            .collect();
        out.extend_from_slice(&bytes);
    }
    assert!(!out.is_empty(), "fixture section [{name}] is missing");
    out
}

/// The full 16 KiB entry array: the fixture's five entries, then zeroes.
///
/// The zeroes are checked rather than assumed — see
/// [`the_entry_array_checksum_sgdisk_recorded_holds_over_our_own_bytes`].
fn entry_array() -> Vec<u8> {
    let mut array = section("entries");
    array.resize(128 * 128, 0);
    array
}

// ---------------------------------------------------------------------------
// A device that can be pulled out of the machine
// ---------------------------------------------------------------------------

/// A whole disk in a `BTreeMap`, sector by sector, with a power cut on a timer.
///
/// Sparse: an unwritten sector reads as zeroes, which is what a card that was
/// never written there gives.
struct FakeDisk {
    lbas: u64,
    sectors: BTreeMap<u64, Vec<u8>>,
    /// Every write, as `(lba, sectors)`, in order.
    writes: Vec<(u64, u64)>,
    /// Every flush, as the number of writes that had happened when it was made.
    flushes: Vec<usize>,
    /// After this many writes, every further write answers `EIO`. The power cut.
    cut_after: Option<usize>,
    /// After this many writes, every further FLUSH answers `EIO` — the write
    /// itself lands, but its durability does not. A different fault from
    /// `cut_after`: the bytes reached the device and the caller cannot tell.
    flush_cut_after: Option<usize>,
    size_error: Option<Errno>,
    read_error: Option<Errno>,
    partition_resizes: Vec<(u32, u64, u64)>,
    partition_resize_error: Option<Errno>,
    filesystem_resizes: Vec<u64>,
    filesystem_resize_error: Option<Errno>,
}

impl FakeDisk {
    /// A card of `lbas` sectors carrying the fixture's image, written at LBA 0.
    ///
    /// The backup GPT is where the *image* put it — at LBA 4194303 — which on
    /// any larger card is nowhere near the end. That is the defect §10 is
    /// about, reproduced rather than described.
    fn written_card(lbas: u64) -> Self {
        let mut disk = Self {
            lbas,
            sectors: BTreeMap::new(),
            writes: Vec::new(),
            flushes: Vec::new(),
            cut_after: None,
            flush_cut_after: None,
            size_error: None,
            read_error: None,
            partition_resizes: Vec::new(),
            partition_resize_error: None,
            filesystem_resizes: Vec::new(),
            filesystem_resize_error: None,
        };
        disk.place(1, &section("primary"));
        disk.place(2, &entry_array());
        disk.place(IMAGE_LBAS - 33, &entry_array());
        disk.place(IMAGE_LBAS - 1, &section("backup"));
        // `mke2fs`'s own superblock, 1024 bytes into partition 5.
        disk.place(STATE_FIRST + 2, &section("ext4"));
        disk
    }

    /// Rewrite `s_blocks_count_lo`, the way the kernel does on an online
    /// resize. The fake's `resize_filesystem` calls this, so a second run of
    /// [`grow_state`] reads back what the first one did.
    fn set_fs_blocks(&mut self, blocks: u64) {
        let mut sb = section("ext4");
        sb.resize(1024, 0);
        let mut current = self.read_state_lbas(2, 2).unwrap_or(sb);
        current[0x04..0x08].copy_from_slice(&(blocks as u32).to_le_bytes());
        current[0x150..0x154].copy_from_slice(&((blocks >> 32) as u32).to_le_bytes());
        self.place(STATE_FIRST + 2, &current);
    }

    /// Write without recording it, for building a starting state.
    fn place(&mut self, lba: u64, bytes: &[u8]) {
        for (n, chunk) in bytes.chunks(LBA_BYTES as usize).enumerate() {
            let mut sector = chunk.to_vec();
            sector.resize(LBA_BYTES as usize, 0);
            self.sectors.insert(lba + n as u64, sector);
        }
    }

    fn sector(&self, lba: u64) -> Vec<u8> {
        self.sectors
            .get(&lba)
            .cloned()
            .unwrap_or_else(|| vec![0u8; LBA_BYTES as usize])
    }

    /// Read the device back the way a kernel would: primary first, backup only
    /// if the primary will not verify.
    fn as_a_reader_sees_it(&mut self) -> Result<Table, Refusal> {
        let primary = read_copy(
            self,
            PRIMARY_HEADER_LBA,
            (Step::ReadPrimaryHeader, Step::ReadPrimaryEntries),
        );
        match primary {
            Ok(Ok(table)) => return Ok(table),
            Ok(Err(_)) => {}
            Err(_) => panic!("the fake cannot fail a read"),
        }
        let last = self.lbas - 1;
        match read_copy(
            self,
            last,
            (Step::ReadBackupHeader, Step::ReadBackupEntries),
        ) {
            Ok(result) => result,
            Err(_) => panic!("the fake cannot fail a read"),
        }
    }

    fn state_entry(&mut self) -> Option<EntryView> {
        let table = self.as_a_reader_sees_it().ok()?;
        table.used().into_iter().find(|e| e.number == STATE_PARTITION)
    }
}

impl GrowMedium for FakeDisk {
    fn device_lbas(&mut self) -> Result<u64, Errno> {
        match self.size_error {
            Some(e) => Err(e),
            None => Ok(self.lbas),
        }
    }

    fn read_lbas(&mut self, lba: u64, count: u64) -> Result<Vec<u8>, Errno> {
        if let Some(e) = self.read_error {
            return Err(e);
        }
        let mut out = Vec::new();
        for n in 0..count {
            out.extend_from_slice(&self.sector(lba + n));
        }
        Ok(out)
    }

    fn read_state_lbas(&mut self, lba: u64, count: u64) -> Result<Vec<u8>, Errno> {
        self.read_lbas(STATE_FIRST + lba, count)
    }

    fn write_lbas(&mut self, lba: u64, bytes: &[u8]) -> Result<(), Errno> {
        if self.cut_after.is_some_and(|n| self.writes.len() >= n) {
            return Err(Errno::EIO);
        }
        let count = bytes.len() as u64 / LBA_BYTES;
        self.writes.push((lba, count));
        self.place(lba, bytes);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Errno> {
        self.flushes.push(self.writes.len());
        if self.flush_cut_after.is_some_and(|n| self.writes.len() >= n) {
            return Err(Errno::EIO);
        }
        Ok(())
    }

    fn resize_partition(
        &mut self,
        number: u32,
        start_bytes: u64,
        len_bytes: u64,
    ) -> Result<(), Errno> {
        if let Some(e) = self.partition_resize_error {
            return Err(e);
        }
        self.partition_resizes.push((number, start_bytes, len_bytes));
        Ok(())
    }

    fn resize_filesystem(&mut self, blocks: u64) -> Result<(), Errno> {
        if let Some(e) = self.filesystem_resize_error {
            return Err(e);
        }
        self.filesystem_resizes.push(blocks);
        self.set_fs_blocks(blocks);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The format, against bytes this repository did not write
// ---------------------------------------------------------------------------

/// The CRC catalogue's check value for CRC-32/ISO-HDLC. A wrong polynomial, a
/// wrong initial value or a missing final inversion all fail this.
#[test]
fn crc32_is_the_one_uefi_names() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

/// **The load-bearing test.** It proves, in one assertion, that this file's
/// CRC-32 is right, that the entry array is 128 entries of 128 bytes in that
/// order, and that the fixture's untranscribed tail really is zeroes — because
/// the number it is checked against is the one `sgdisk` wrote into its own
/// header.
#[test]
fn the_entry_array_checksum_sgdisk_recorded_holds_over_our_own_bytes() {
    let header = Header::parse(&section("primary")).expect("sgdisk's header parses");
    assert_eq!(header.entry_count, 128);
    assert_eq!(header.entry_bytes, 128);
    assert_eq!(crc32(&entry_array()), header.entry_array_crc);
}

/// The other checksum, and it is checked by `Header::parse` refusing to return
/// a header whose CRC does not hold.
#[test]
fn both_of_sgdisks_headers_verify_and_describe_the_same_device() {
    let primary = Header::parse(&section("primary")).expect("primary verifies");
    let backup = Header::parse(&section("backup")).expect("backup verifies");
    assert_eq!(primary.my_lba, 1);
    assert_eq!(primary.alternate_lba, IMAGE_LBAS - 1);
    assert_eq!(backup.my_lba, IMAGE_LBAS - 1);
    assert_eq!(backup.alternate_lba, 1);
    assert_eq!(primary.last_usable_lba, backup.last_usable_lba);
    assert_eq!(primary.entry_array_crc, backup.entry_array_crc);
    assert_eq!(backup.entry_array_lba, IMAGE_LBAS - 33);
}

/// The five partitions `ADR-018` §4 specifies, read out of `sgdisk`'s bytes.
#[test]
fn the_fixture_is_adr_018_4s_table_and_partition_5_is_state() {
    let table = fixture_table(IMAGE_LBAS);
    let used = table.used();
    let seen: Vec<_> = used
        .iter()
        .map(|e| (e.number, e.name.as_str(), e.first_lba, e.last_lba))
        .collect();
    assert_eq!(
        seen,
        [
            (1, "boot-a", 2048, 526_335),
            (2, "boot-b", 526_336, 1_050_623),
            (3, "system-a", 1_050_624, 2_361_343),
            (4, "system-b", 2_361_344, 3_672_063),
            (5, "state", STATE_FIRST, STATE_LAST_ON_IMAGE),
        ]
    );
}

/// **The writer, checked against the foreign writer.** Given the table on a
/// device of the size it was built for, this module's rewrite must reproduce
/// `sgdisk`'s own two headers byte for byte — including the disk GUID, the
/// revision and every reserved field it never names. If the rewrite dropped a
/// field, moved one, or computed either checksum differently, these differ.
#[test]
fn rewriting_a_table_on_its_own_device_reproduces_sgdisks_bytes_exactly() {
    let table = fixture_table(IMAGE_LBAS);
    let plan = assess(&table, IMAGE_LBAS).expect("the fixture's own table is acceptable");
    assert!(
        !plan.table_needs_writing,
        "a card written at its own size has nothing to do"
    );
    let bytes = rewrite(&table, &plan);
    assert_eq!(bytes.primary_header, section("primary"), "primary header");
    assert_eq!(bytes.backup_header, section("backup"), "backup header");
    assert_eq!(bytes.entries, entry_array(), "entry array");
}

fn fixture_table(device_lbas: u64) -> Table {
    let mut disk = FakeDisk::written_card(device_lbas);
    disk.as_a_reader_sees_it().expect("the fixture is readable")
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

/// `ADR-018` §10.1's card. The grown partition must end on a 1 MiB boundary,
/// leave room for the backup GPT, and start where it always started.
#[test]
fn a_card_bigger_than_the_image_grows_state_to_the_end_of_it() {
    let table = fixture_table(CARD_LBAS);
    let plan = assess(&table, CARD_LBAS).expect("acceptable");
    assert!(plan.table_needs_writing);
    assert_eq!(plan.state_first_lba, STATE_FIRST, "the start never moves");
    assert_eq!(plan.state_last_lba_now, STATE_LAST_ON_IMAGE);
    assert_eq!(plan.backup_header_lba, CARD_LBAS - 1);
    assert_eq!(plan.backup_entries_lba, CARD_LBAS - 33);
    assert_eq!(plan.last_usable_lba, CARD_LBAS - 34);
    assert_eq!(
        (plan.state_last_lba_new + 1) % ALIGN_LBA,
        0,
        "the end is on a 1 MiB boundary"
    );
    assert!(plan.state_last_lba_new <= plan.last_usable_lba);
    assert!(plan.state_last_lba_new > STATE_LAST_ON_IMAGE);
    // 3672064..=15562751 is 5.67 GiB, which is what a 7.42 GiB card leaves.
    assert_eq!(plan.state_last_lba_new, 15_562_751);
}

/// The escape hatch `ADR-018` §10.4 kept: a card written with `--device-mib`
/// at its own size must be left alone entirely.
#[test]
fn a_card_written_at_its_own_size_is_untouched() {
    let mut disk = FakeDisk::written_card(IMAGE_LBAS);
    let got = grow_state(&mut disk);
    assert_eq!(got, Grew::AlreadyFillsTheDevice);
    assert!(disk.writes.is_empty(), "{:?}", disk.writes);
    assert!(disk.partition_resizes.is_empty());
    assert!(disk.filesystem_resizes.is_empty());
    assert!(!got.is_fault());
}

/// A card a *fraction* bigger than the image has no room worth taking, and the
/// backup GPT is still in the wrong place. The table is rewritten anyway,
/// because the kernel's complaint is about the header and not about the space.
///
/// **32 sectors, and the number is not arbitrary.** The image already ends
/// 2015 sectors short of its own 1 MiB boundary, because the backup GPT has to
/// fit above the last partition; so a card up to 32 sectors larger still aligns
/// down to the same place, and one 33 sectors larger gains a whole megabyte at
/// a stroke. This is the widest card that gains nothing.
#[test]
fn a_card_barely_bigger_moves_the_backup_gpt_without_moving_the_partition() {
    let bigger = IMAGE_LBAS + 32;
    let table = fixture_table(bigger);
    let plan = assess(&table, bigger).expect("acceptable");
    assert!(
        plan.table_needs_writing,
        "the alternate header is not at the end of the disk"
    );
    assert_eq!(
        plan.state_last_lba_new, STATE_LAST_ON_IMAGE,
        "there is no aligned room, so the partition does not move"
    );
    assert_eq!(plan.backup_header_lba, bigger - 1);
}

/// The condition §10.3 asks for, stated as a property: after one run there is
/// nothing left to do, and a second run writes nothing at all.
#[test]
fn a_second_run_writes_nothing() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    let first = grow_state(&mut disk);
    assert!(matches!(first, Grew::Grown { .. }), "{first:?}");
    assert!(!disk.writes.is_empty());

    disk.writes.clear();
    disk.partition_resizes.clear();
    disk.filesystem_resizes.clear();
    let second = grow_state(&mut disk);
    assert_eq!(second, Grew::AlreadyFillsTheDevice, "{}", second.summary());
    assert!(disk.writes.is_empty(), "{:?}", disk.writes);
    assert!(disk.partition_resizes.is_empty());
    assert!(disk.filesystem_resizes.is_empty());
}

/// End to end on the fake, with the numbers a 7.42 GiB card produces.
#[test]
fn the_whole_operation_grows_the_partition_the_kernels_view_and_the_filesystem() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    let got = grow_state(&mut disk);
    let Grew::Grown {
        partition_from_lbas,
        partition_to_lbas,
        fs_from_blocks,
        fs_to_blocks,
        block_bytes,
    } = got
    else {
        panic!("{got:?}");
    };
    assert_eq!(partition_from_lbas, 520_192);
    assert_eq!(partition_to_lbas, 15_562_751 - STATE_FIRST + 1);
    // The fixture's own superblock: 1 KiB blocks, 260096 of them. These are
    // the numbers a real loop device produced on 2026-08-24 — see
    // `ADR-018` §10.7.4.
    assert_eq!(block_bytes, 1024);
    assert_eq!(fs_from_blocks, 260_096);
    assert_eq!(fs_to_blocks, 5_945_344);
    assert_eq!(fs_to_blocks, partition_to_lbas * LBA_BYTES / block_bytes);

    assert_eq!(
        disk.partition_resizes,
        vec![(
            5,
            STATE_FIRST * LBA_BYTES,
            partition_to_lbas * LBA_BYTES
        )],
        "the kernel is told once, with byte offsets"
    );
    assert_eq!(disk.filesystem_resizes, vec![fs_to_blocks]);

    // And the device now reads back as the grown layout, from either copy.
    let state = disk.state_entry().expect("partition 5 is still there");
    assert_eq!(state.first_lba, STATE_FIRST);
    assert_eq!(state.last_lba, 15_562_751);
    assert_eq!(state.name, "state");
}

/// The filesystem is sized from the partition the plan *makes*, not the one
/// that is there. On a fresh card the ext4 exactly fills its 254 MiB, so
/// measuring it against the current partition would find nothing to do and the
/// card would grow its table and stop.
#[test]
fn the_filesystem_is_measured_against_the_partition_it_is_about_to_have() {
    let settled = assess_filesystem(
        FsSize {
            block_bytes: 4096,
            blocks: 65_024,
        },
        266_338_304,
    );
    assert_eq!(settled, FsPlan::AlreadyFills);

    // The same filesystem, measured against the partition the plan makes on a
    // 7.42 GiB card: 3672064..=15562751, which is 6 087 872 512 bytes.
    let grows = assess_filesystem(
        FsSize {
            block_bytes: 4096,
            blocks: 65_024,
        },
        6_087_872_512,
    );
    assert_eq!(
        grows,
        FsPlan::Resize {
            from: 65_024,
            to: 1_486_297,
            block_bytes: 4096
        }
    );
}

/// This module only ever grows. A filesystem already larger than its partition
/// is a fault to report, never a thing to trim.
#[test]
fn a_filesystem_larger_than_its_partition_is_refused_and_never_shrunk() {
    let got = assess_filesystem(
        FsSize {
            block_bytes: 4096,
            blocks: 100_000,
        },
        65_024 * 4096,
    );
    assert_eq!(
        got,
        FsPlan::Refused(FsRefusal::LargerThanItsPartition {
            blocks: 100_000,
            room: 65_024
        })
    );
}

/// `statvfs` on something that is not a filesystem reports zeroes, and a
/// division by that would be the last thing this process ever did.
#[test]
fn a_block_size_ext4_cannot_have_is_refused_rather_than_divided_by() {
    for bytes in [0, 1, 512, 3000, 131_072] {
        let got = assess_filesystem(FsSize { block_bytes: bytes, blocks: 1 }, 1 << 30);
        assert_eq!(got, FsPlan::Refused(FsRefusal::BlockSize { bytes }), "{bytes}");
    }
}

// ---------------------------------------------------------------------------
// Refusals — nothing is written
// ---------------------------------------------------------------------------

/// Every one of these must leave the device untouched. This is the test that
/// stands between a bug in this file and somebody's disk.
#[test]
fn a_table_this_does_not_recognise_is_refused_and_nothing_is_written() {
    struct Case {
        name: &'static str,
        damage: fn(&mut FakeDisk),
        expect: fn(&Refusal) -> bool,
    }

    let cases = [
        Case {
            name: "no GPT at all",
            damage: |d| {
                d.sectors.clear();
            },
            expect: |r| matches!(r, Refusal::NotGpt),
        },
        Case {
            name: "a primary header with one bit flipped, and no usable backup",
            damage: |d| {
                let mut sector = d.sector(1);
                sector[30] ^= 0x01;
                d.place(1, &sector);
                d.sectors.remove(&(IMAGE_LBAS - 1));
            },
            expect: |r| matches!(r, Refusal::HeaderChecksum { .. }),
        },
        Case {
            name: "an entry array that does not match its header",
            damage: |d| {
                let mut sector = d.sector(2);
                sector[64] ^= 0xFF;
                d.place(2, &sector);
                d.sectors.remove(&(IMAGE_LBAS - 1));
                d.sectors.remove(&(IMAGE_LBAS - 33));
            },
            expect: |r| matches!(r, Refusal::EntryArrayChecksum { .. }),
        },
        Case {
            name: "partition 5 labelled something else",
            damage: |d| relabel_state(d, "swap"),
            expect: |r| matches!(r, Refusal::StateLabel { .. }),
        },
        Case {
            name: "partition 5 unused",
            damage: |d| {
                let mut array = entry_array();
                for byte in array.iter_mut().skip(4 * 128).take(16) {
                    *byte = 0;
                }
                reseal(d, &array);
            },
            expect: |r| matches!(r, Refusal::StateUnused),
        },
        Case {
            name: "somebody added a sixth partition after /state",
            damage: |d| {
                let mut array = entry_array();
                let (six, five) = array.split_at_mut(5 * 128);
                five[..128].copy_from_slice(&six[4 * 128..5 * 128]);
                // Move it well past /state's start so the overlap is obvious.
                let base = 5 * 128;
                array[base + 32..base + 40].copy_from_slice(&5_000_000u64.to_le_bytes());
                array[base + 40..base + 48].copy_from_slice(&6_000_000u64.to_le_bytes());
                reseal(d, &array);
            },
            expect: |r| matches!(r, Refusal::SomethingAfterState { number: 6, .. }),
        },
    ];

    for case in cases {
        let mut disk = FakeDisk::written_card(CARD_LBAS);
        (case.damage)(&mut disk);
        disk.writes.clear();
        let got = grow_state(&mut disk);
        let Grew::Refused { why } = &got else {
            panic!("{}: expected a refusal, got {got:?}", case.name);
        };
        assert!((case.expect)(why), "{}: got {why}", case.name);
        assert!(
            disk.writes.is_empty(),
            "{}: a refusal wrote to the device: {:?}",
            case.name,
            disk.writes
        );
        assert!(disk.partition_resizes.is_empty(), "{}", case.name);
        assert!(disk.filesystem_resizes.is_empty(), "{}", case.name);
        assert!(got.is_fault(), "{}", case.name);
        assert!(!got.summary().is_empty(), "{}", case.name);
    }
}

/// Rewrite an entry array onto the fake and fix both headers' checksums, so the
/// damage under test is the only thing wrong with the device.
fn reseal(disk: &mut FakeDisk, array: &[u8]) {
    let crc = crc32(array);
    disk.place(2, array);
    disk.place(IMAGE_LBAS - 33, array);
    for lba in [1, IMAGE_LBAS - 1] {
        let mut sector = disk.sector(lba);
        put_u32(&mut sector, header_at::ENTRY_ARRAY_CRC, crc);
        put_u32(&mut sector, header_at::HEADER_CRC, 0);
        let size = u32_at(&sector, header_at::HEADER_SIZE).unwrap_or(92) as usize;
        let fixed = crc32(&sector[..size]);
        put_u32(&mut sector, header_at::HEADER_CRC, fixed);
        disk.place(lba, &sector);
    }
}

fn relabel_state(disk: &mut FakeDisk, label: &str) {
    let mut array = entry_array();
    let base = 4 * 128 + entry_at::NAME;
    for slot in array[base..base + entry_at::NAME_BYTES].iter_mut() {
        *slot = 0;
    }
    for (n, unit) in label.encode_utf16().enumerate() {
        array[base + n * 2..base + n * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    reseal(disk, &array);
}

// ---------------------------------------------------------------------------
// The write order, and the power cut
// ---------------------------------------------------------------------------

/// The order in the module documentation, asserted so a refactor cannot quietly
/// reorder it — the crash-safety argument is the *order* and nothing else.
///
/// Note the third write: **LBA 3, one sector**, not the 32 sectors of the whole
/// entry array. Partition 5's ending LBA is at byte 552 of the array, which is
/// its second sector, and the interval between that write and the header is the
/// one window in this operation. Everything else in the array is written back
/// unchanged, so it is not written at all.
#[test]
fn the_backup_goes_first_the_header_last_and_only_one_array_sector_moves() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    let _ = grow_state(&mut disk);
    assert_eq!(
        disk.writes,
        vec![
            (CARD_LBAS - 33, 32),
            (CARD_LBAS - 1, 1),
            (3, 1),
            (PRIMARY_HEADER_LBA, 1)
        ],
        "backup entries, backup header, ONE primary entry sector, primary header"
    );
    assert_eq!(
        disk.flushes,
        vec![1, 2, 3, 4],
        "each step is made durable before the next begins"
    );
}

/// Only the sectors that differ, and none when nothing differs.
#[test]
fn only_the_sectors_whose_bytes_change_are_named() {
    let old = entry_array();
    let mut new = old.clone();
    assert!(changed_entry_sectors(&old, &new).is_empty());

    new[4 * 128 + 40] ^= 0xFF; // partition 5's ending LBA
    let changed = changed_entry_sectors(&old, &new);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].0, 1, "the array's second sector");
    assert_eq!(changed[0].1.len(), LBA_BYTES as usize);
}

/// **What a card pulled mid-rewrite actually looks like, stated as the three
/// separate outcomes it has** rather than the one this module was first written
/// to claim.
///
/// The distinction is not academic and was not deduced: a Linux 6.6 kernel
/// creates **no partition devices at all** for a primary GPT whose two
/// checksums disagree with each other, and does not fall back to the backup
/// unless `gpt` is on the kernel command line. So "the primary still verifies"
/// is the same sentence as "the board still boots", and that is what this
/// asserts for every cut but one.
#[test]
fn only_the_last_write_has_a_window_and_every_other_cut_leaves_a_bootable_card() {
    for cut in 0..=4usize {
        let mut disk = FakeDisk::written_card(CARD_LBAS);
        disk.cut_after = Some(cut);
        let got = grow_state(&mut disk);
        if cut < 4 {
            assert!(
                matches!(got, Grew::Failed { errno: Errno::EIO, .. }),
                "cut after {cut}: {got:?}"
            );
        }
        disk.cut_after = None;

        let primary = read_copy(
            &mut disk,
            PRIMARY_HEADER_LBA,
            (Step::ReadPrimaryHeader, Step::ReadPrimaryEntries),
        )
        .unwrap_or_else(|_| panic!("the fake cannot fail a read"));

        if cut == 3 {
            // The window: the array sector landed and the header did not.
            assert!(
                matches!(primary, Err(Refusal::EntryArrayChecksum { .. })),
                "cut after 3 is the window and must look like one: {primary:?}"
            );
            // …and the backup at the end of the device is already whole, which
            // is what makes it recoverable rather than fatal.
            let backup = read_copy(
                &mut disk,
                CARD_LBAS - 1,
                (Step::ReadBackupHeader, Step::ReadBackupEntries),
            )
            .unwrap_or_else(|_| panic!("the fake cannot fail a read"));
            let backup = backup.expect("the backup was written before the window opened");
            let state = backup
                .used()
                .into_iter()
                .find(|e| e.number == STATE_PARTITION)
                .expect("partition 5");
            assert_eq!(state.last_lba, 15_562_751, "and it carries the new layout");
            continue;
        }

        let table = primary
            .unwrap_or_else(|why| panic!("cut after {cut} left an unbootable primary: {why}"));
        let state = table
            .used()
            .into_iter()
            .find(|e| e.number == STATE_PARTITION)
            .expect("partition 5");
        assert_eq!(state.first_lba, STATE_FIRST, "cut after {cut}");
        assert_eq!(state.name, "state", "cut after {cut}");
        let expected = if cut == 4 {
            15_562_751
        } else {
            STATE_LAST_ON_IMAGE
        };
        assert_eq!(
            state.last_lba, expected,
            "cut after {cut}: the primary must carry one whole layout"
        );
    }
}

/// And the next boot finishes the job, from wherever the previous one stopped.
/// This is `ADR-018` §10.3's *"idempotent and self-disarming"* under the worst
/// starting condition it can be given.
#[test]
fn the_boot_after_a_power_cut_finishes_the_grow_from_wherever_it_stopped() {
    for cut in 0..4usize {
        let mut disk = FakeDisk::written_card(CARD_LBAS);
        disk.cut_after = Some(cut);
        let _ = grow_state(&mut disk);

        disk.cut_after = None;
        let second = grow_state(&mut disk);
        assert!(
            matches!(second, Grew::Grown { .. }),
            "cut after {cut}: the next boot got {second:?}"
        );
        let third = grow_state(&mut disk);
        assert_eq!(
            third,
            Grew::AlreadyFillsTheDevice,
            "cut after {cut}: the boot after that"
        );

        let state = disk.state_entry().expect("a table");
        assert_eq!(state.last_lba, 15_562_751, "cut after {cut}");
    }
}

/// An interruption between the table and the filesystem leaves the state the
/// documentation calls recoverable, and the next boot completes it.
#[test]
fn a_table_that_grew_without_its_filesystem_is_finished_by_the_next_boot() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    disk.filesystem_resize_error = Some(Errno::EIO);
    let got = grow_state(&mut disk);
    assert!(
        matches!(
            got,
            Grew::Failed {
                at: Step::ResizeFilesystem,
                errno: Errno::EIO
            }
        ),
        "{got:?}"
    );
    assert!(got.is_fault());
    assert!(
        got.summary().contains("already rewritten"),
        "the report must say the table was touched: {}",
        got.summary()
    );

    disk.filesystem_resize_error = None;
    disk.writes.clear();
    let second = grow_state(&mut disk);
    assert!(matches!(second, Grew::Grown { .. }), "{second:?}");
    assert!(
        disk.writes.is_empty(),
        "the table was already right, so nothing was rewritten: {:?}",
        disk.writes
    );
    assert_eq!(disk.filesystem_resizes.len(), 1);
}

/// The partition grew and the filesystem cannot follow, for a reason that is
/// not an errno. Reported, not fatal, the boot goes on — and **the partition
/// was still enlarged**, because enlarging a partition around bytes nobody
/// understands is safe and enlarging a filesystem is not.
#[test]
fn a_state_that_is_not_ext4_leaves_the_partition_grown_and_says_so() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    let mut sb = disk.read_state_lbas(2, 2).unwrap();
    sb[0x38] = 0;
    sb[0x39] = 0;
    disk.place(STATE_FIRST + 2, &sb);

    let got = grow_state(&mut disk);
    let Grew::PartitionOnly {
        why,
        partition_from_lbas,
        partition_to_lbas,
    } = &got
    else {
        panic!("{got:?}");
    };
    assert!(matches!(why, FsRefusal::NotExt4 { magic: 0 }), "{why}");
    assert!(partition_to_lbas > partition_from_lbas);
    assert!(got.is_fault());
    assert!(got.summary().contains("next boot"), "{}", got.summary());
    assert!(disk.filesystem_resizes.is_empty());
    assert_eq!(disk.partition_resizes.len(), 1);
}

/// The superblock is the authority for the filesystem's size, and **not
/// `statvfs`**. Measured on a loop device: a 254 MiB `/state` whose superblock
/// says 260096 blocks is reported by `statvfs` as 237607, because `ext4_statfs`
/// subtracts its own overhead. Reading the smaller number made the second run
/// of a completed grow issue another resize for ever.
#[test]
fn the_size_of_the_filesystem_comes_from_its_own_superblock() {
    let got = parse_superblock(&section("ext4")).expect("mke2fs's superblock parses");
    assert_eq!(
        got,
        FsSize {
            block_bytes: 1024,
            blocks: 260_096
        }
    );
    // …and 260096 blocks of 1024 bytes is exactly partition 5 of the image.
    assert_eq!(
        got.blocks * got.block_bytes,
        (STATE_LAST_ON_IMAGE - STATE_FIRST + 1) * LBA_BYTES
    );
}

/// Anything that is not `0xEF53` is refused rather than sized.
#[test]
fn something_that_is_not_an_ext4_superblock_is_refused() {
    assert_eq!(
        parse_superblock(&[0u8; 1024]),
        Err(FsRefusal::NotExt4 { magic: 0 })
    );
    assert_eq!(parse_superblock(&[]), Err(FsRefusal::NotExt4 { magic: 0 }));

    let mut sb = section("ext4");
    sb.resize(1024, 0);
    sb[super_at::LOG_BLOCK_SIZE] = 9;
    assert_eq!(
        parse_superblock(&sb),
        Err(FsRefusal::BlockSize { bytes: 0 })
    );
}

/// A filesystem past 4 TiB carries its block count in two halves, and the top
/// one is only meaningful with `INCOMPAT_64BIT` — which `mke2fs` set on the
/// fixture. Reading it unconditionally would multiply a card's size by four
/// billion on a filesystem that did not have the feature.
#[test]
fn the_high_half_of_the_block_count_is_read_only_when_the_feature_says_so() {
    let mut sb = section("ext4");
    sb.resize(1024, 0);
    sb[super_at::BLOCKS_COUNT_HI..super_at::BLOCKS_COUNT_HI + 4]
        .copy_from_slice(&7u32.to_le_bytes());
    assert_eq!(
        parse_superblock(&sb).map(|s| s.blocks),
        Ok(260_096 | (7u64 << 32)),
        "the fixture has INCOMPAT_64BIT, so the high half counts"
    );

    sb[super_at::FEATURE_INCOMPAT..super_at::FEATURE_INCOMPAT + 4]
        .copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        parse_superblock(&sb).map(|s| s.blocks),
        Ok(260_096),
        "without it, the high half is not a number"
    );
}

// ---------------------------------------------------------------------------
// Every failure is survived
// ---------------------------------------------------------------------------

/// The standing rule for anything on a boot path: reach the end of the
/// function whatever the machine does. No `Err` escapes, nothing panics, and
/// the outcome always has something to say.
#[test]
fn every_failure_the_machine_can_produce_is_survived_and_reported() {
    /// One thing the machine can refuse to do, and what it is called.
    type Breakage = (&'static str, fn(&mut FakeDisk));

    let breakages: Vec<Breakage> = vec![
        ("the size of the device", |d| {
            d.size_error = Some(Errno::ENODEV)
        }),
        ("the partition resize", |d| {
            d.partition_resize_error = Some(Errno::EBUSY)
        }),
        ("every read", |d| d.read_error = Some(Errno::EIO)),
        ("the filesystem resize", |d| {
            d.filesystem_resize_error = Some(Errno::ENOSPC)
        }),
        ("every write", |d| d.cut_after = Some(0)),
        ("all of it at once", |d| {
            d.size_error = Some(Errno::EIO);
            d.partition_resize_error = Some(Errno::EIO);
            d.read_error = Some(Errno::EIO);
            d.filesystem_resize_error = Some(Errno::EIO);
            d.cut_after = Some(0);
        }),
    ];
    for (name, break_it) in breakages {
        let mut disk = FakeDisk::written_card(CARD_LBAS);
        break_it(&mut disk);
        let got = grow_state(&mut disk);
        assert!(got.is_fault(), "{name}: {got:?}");
        assert!(!got.summary().is_empty(), "{name}");
    }
}

/// A failure before anything is written must say so, and one after must say the
/// opposite. The operator's next move differs.
#[test]
fn the_report_says_whether_the_partition_table_was_touched() {
    let mut early = FakeDisk::written_card(CARD_LBAS);
    early.size_error = Some(Errno::ENODEV);
    assert!(
        grow_state(&mut early).summary().contains("Nothing was written"),
        "{}",
        grow_state(&mut early).summary()
    );

    let mut late = FakeDisk::written_card(CARD_LBAS);
    late.partition_resize_error = Some(Errno::EBUSY);
    assert!(
        grow_state(&mut late).summary().contains("already rewritten"),
        "{}",
        grow_state(&mut late).summary()
    );
}

/// A flush that fails right after the backup entries landed must not claim
/// nothing was written: the entries are on the device and only their
/// durability is in doubt. It must also not claim the PRIMARY was rewritten —
/// the window between the two copies has not even opened yet.
#[test]
fn a_flush_failure_after_the_backup_entries_land_does_not_claim_nothing_was_written() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    disk.flush_cut_after = Some(1); // the very first flush, right after write 1
    let got = grow_state(&mut disk);
    assert_eq!(
        got,
        Grew::Failed {
            at: Step::WriteBackupEntries,
            errno: Errno::EIO
        }
    );
    assert_eq!(
        disk.writes,
        vec![(CARD_LBAS - 33, 32)],
        "the write itself reached the fake before the flush was asked to fail"
    );
    let summary = got.summary();
    assert!(!summary.contains("Nothing was written"), "{summary}");
    assert!(!summary.contains("already rewritten"), "{summary}");
    assert!(
        summary.contains("BACKUP") && summary.contains("may have changed"),
        "{summary}"
    );
}

/// The same call-site failure, but the write call itself is the one that
/// fails rather than the flush after it — a different point of uncertainty
/// with the same consequence, so it must read the same way.
#[test]
fn a_write_failure_at_the_backup_entries_step_does_not_claim_nothing_was_written() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    disk.cut_after = Some(0); // the very first write call
    let got = grow_state(&mut disk);
    assert_eq!(
        got,
        Grew::Failed {
            at: Step::WriteBackupEntries,
            errno: Errno::EIO
        }
    );
    assert!(disk.writes.is_empty(), "the fake reports the call itself failing");
    let summary = got.summary();
    assert!(!summary.contains("Nothing was written"), "{summary}");
    assert!(
        summary.contains("BACKUP") && summary.contains("may have changed"),
        "{summary}"
    );
}

/// The backup HEADER step gets the same conservative sentence as the backup
/// ENTRIES step, and neither is "already rewritten" — that sentence is true
/// only once the primary's own header has committed, which is steps away.
#[test]
fn a_failure_writing_the_backup_header_does_not_claim_the_primary_was_rewritten() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    disk.cut_after = Some(1); // the backup entries already landed; the header write fails
    let got = grow_state(&mut disk);
    assert_eq!(
        got,
        Grew::Failed {
            at: Step::WriteBackupHeader,
            errno: Errno::EIO
        }
    );
    assert_eq!(disk.writes, vec![(CARD_LBAS - 33, 32)]);
    let summary = got.summary();
    assert!(!summary.contains("Nothing was written"), "{summary}");
    assert!(!summary.contains("already rewritten"), "{summary}");
    assert!(
        summary.contains("BACKUP") && summary.contains("may have changed"),
        "{summary}"
    );
}

/// A failure at the PRIMARY header — the commit record, the last of the four
/// writes — must keep today's sentence: the table really was already
/// rewritten by the time this step is reached, and the next boot continues
/// from there.
#[test]
fn a_failure_writing_the_primary_header_still_says_the_table_was_already_rewritten() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    disk.cut_after = Some(3); // backup entries, backup header, and the one
    // changed primary-entry sector have all landed; the primary header fails.
    let got = grow_state(&mut disk);
    assert_eq!(
        got,
        Grew::Failed {
            at: Step::WritePrimaryHeader,
            errno: Errno::EIO
        }
    );
    let summary = got.summary();
    assert!(!summary.contains("Nothing was written"), "{summary}");
    assert!(summary.contains("already rewritten"), "{summary}");
}

/// Every step must be able to name itself, or a console line reads
/// `... failed with EIO` and says nothing about where.
#[test]
fn every_step_describes_itself() {
    for step in [
        Step::ReadDeviceSize,
        Step::ReadPrimaryHeader,
        Step::ReadPrimaryEntries,
        Step::ReadBackupHeader,
        Step::ReadBackupEntries,
        Step::WriteBackupEntries,
        Step::WriteBackupHeader,
        Step::WritePrimaryEntries,
        Step::WritePrimaryHeader,
        Step::ResizePartition,
        Step::ReadSuperblock,
        Step::ResizeFilesystem,
    ] {
        assert!(!step.described().is_empty(), "{step:?}");
    }
    assert!(!Step::ReadDeviceSize.is_after_a_write(), "a read has not written anything");
    assert!(Step::WriteBackupEntries.is_after_a_write(), "the first write has landed");
    assert!(Step::ResizeFilesystem.is_after_a_write());

    assert!(Step::WriteBackupEntries.is_a_backup_only_write());
    assert!(Step::WriteBackupHeader.is_a_backup_only_write());
    assert!(!Step::WritePrimaryEntries.is_a_backup_only_write());
    assert!(!Step::WritePrimaryHeader.is_a_backup_only_write());
    assert!(!Step::ResizePartition.is_a_backup_only_write());
    assert!(!Step::ReadDeviceSize.is_a_backup_only_write());
}

// ---------------------------------------------------------------------------
// Recovery from the backup copy
// ---------------------------------------------------------------------------

/// The primary is unreadable and the backup is the new layout — which is
/// exactly what an interruption between writes three and four leaves. The
/// backup answers, and the repair rewrites both copies.
#[test]
fn a_broken_primary_is_repaired_from_the_backup_rather_than_refused() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    let _ = grow_state(&mut disk); // put a good, grown table on both copies

    let mut sector = disk.sector(1);
    sector[16] ^= 0xFF; // its own checksum field
    disk.place(1, &sector);
    disk.writes.clear();
    disk.filesystem_resizes.clear();

    let got = grow_state(&mut disk);
    assert!(!matches!(got, Grew::Refused { .. }), "{got:?}");
    assert!(
        !disk.writes.is_empty(),
        "the primary had to be written back: {:?}",
        disk.writes
    );
    let table = disk.as_a_reader_sees_it().expect("a table");
    assert_eq!(table.source, Source::Primary, "the primary verifies again");
}

// ---------------------------------------------------------------------------
// Recovery rebuilds an independent primary array, not a pointer at the backup
// ---------------------------------------------------------------------------

/// Flip the primary header's own checksum field, so `Header::parse` refuses it
/// while its array is untouched.
fn corrupt_primary_header(disk: &mut FakeDisk) {
    let mut sector = disk.sector(PRIMARY_HEADER_LBA);
    sector[16] ^= 0xFF; // the header's own CRC field
    disk.place(PRIMARY_HEADER_LBA, &sector);
}

/// Flip a byte inside the primary array, so it no longer matches the checksum
/// its own — untouched — header records.
fn corrupt_primary_array(disk: &mut FakeDisk) {
    let mut sector = disk.sector(2);
    sector[0] ^= 0xFF;
    disk.place(2, &sector);
}

/// Both at once: neither half of the primary is trustworthy.
fn corrupt_primary_header_and_array(disk: &mut FakeDisk) {
    corrupt_primary_header(disk);
    corrupt_primary_array(disk);
}

/// Read BOTH copies independently — not only ask whether the reader that
/// prefers the primary is satisfied — and check the placement, distinctness
/// and content invariants an independent recovery must hold.
fn assert_both_copies_are_independent_and_valid(disk: &mut FakeDisk) {
    let lbas = disk.lbas;
    let primary = read_copy(
        disk,
        PRIMARY_HEADER_LBA,
        (Step::ReadPrimaryHeader, Step::ReadPrimaryEntries),
    )
    .unwrap_or_else(|_| panic!("the fake cannot fail a read"))
    .expect("the primary header and array both verify, independently, after recovery");
    assert_eq!(primary.source, Source::Primary);
    assert_eq!(
        primary.header.entry_array_lba, 2,
        "the primary array belongs at LBA 2, immediately after its header"
    );
    let primary_entries_end = primary.header.entry_array_lba + primary.header.entry_array_lbas();
    assert!(
        primary_entries_end <= primary.header.first_usable_lba,
        "the primary array must end before first_usable_lba: {primary_entries_end} > {}",
        primary.header.first_usable_lba
    );

    let backup = read_copy(
        disk,
        lbas - 1,
        (Step::ReadBackupHeader, Step::ReadBackupEntries),
    )
    .unwrap_or_else(|_| panic!("the fake cannot fail a read"))
    .expect("the backup header and array both verify, independently, after recovery");
    assert_eq!(backup.source, Source::Backup);
    assert!(
        backup.header.entry_array_lba > primary.header.last_usable_lba,
        "the backup array must lie after last_usable_lba: {} <= {}",
        backup.header.entry_array_lba,
        primary.header.last_usable_lba
    );

    assert_ne!(
        primary.header.entry_array_lba, backup.header.entry_array_lba,
        "the two arrays must be at distinct locations"
    );
    assert_eq!(
        primary.entries, backup.entries,
        "the two arrays must hold identical bytes"
    );

    let last_usable_lba = last_usable(lbas, primary.header.entry_array_lbas());
    assert_eq!(primary.header.last_usable_lba, last_usable_lba);
    let primary_state = primary
        .used()
        .into_iter()
        .find(|e| e.number == STATE_PARTITION)
        .expect("partition 5 is in use");
    let backup_state = backup
        .used()
        .into_iter()
        .find(|e| e.number == STATE_PARTITION)
        .expect("partition 5 is in use");
    assert_eq!(primary_state.first_lba, backup_state.first_lba, "partition 5's extent");
    assert_eq!(primary_state.last_lba, backup_state.last_lba, "partition 5's extent");
    assert_eq!(
        primary_state.last_lba,
        aligned_end(last_usable_lba),
        "partition 5 must fill the device up to alignment"
    );
}

/// **The bug, reproduced and fixed**: recovering from the backup must rebuild
/// an independent primary array at its own fixed location rather than leave
/// the primary header pointing at the backup's copy. Damages the primary's
/// header, its array, and both together; recovers each against a device that
/// still needs to grow and one that already matches the image's own size; and
/// checks both copies independently rather than only asking whether *a* read
/// of the primary is satisfied — which is exactly the check the old code
/// passed while sharing one array between the two headers.
#[test]
fn recovery_from_the_backup_rebuilds_an_independent_primary_array() {
    type Damage = fn(&mut FakeDisk);
    let damages: [(&str, Damage); 3] = [
        ("header", corrupt_primary_header),
        ("array", corrupt_primary_array),
        ("both", corrupt_primary_header_and_array),
    ];
    for (name, damage) in damages {
        // Unchanged: the device already matches the image's own size, so the
        // fixture's own backup already sits exactly where this device size
        // expects it — no prior successful grow is needed to place it there.
        {
            let mut disk = FakeDisk::written_card(IMAGE_LBAS);
            damage(&mut disk);
            let got = grow_state(&mut disk);
            assert!(!got.is_fault(), "{name}, unchanged device: {got:?}");
            assert_both_copies_are_independent_and_valid(&mut disk);
        }

        // Grown: a first successful grow relocates the backup to this
        // device's own last LBA, and recovery must rebuild the primary at
        // THAT geometry.
        {
            let mut disk = FakeDisk::written_card(CARD_LBAS);
            let first = grow_state(&mut disk);
            assert!(!first.is_fault(), "{name}, grown device, setup: {first:?}");
            damage(&mut disk);
            disk.writes.clear();
            disk.filesystem_resizes.clear();
            let got = grow_state(&mut disk);
            assert!(!got.is_fault(), "{name}, grown device: {got:?}");
            assert_both_copies_are_independent_and_valid(&mut disk);
        }
    }
}

/// The two copies are not merely both readable after a recovery — they are
/// not, as the bug made them, the SAME BYTES AT THE SAME ADDRESS.
/// Corrupting the just-written backup must leave the primary alone.
#[test]
fn after_recovery_corrupting_the_backup_leaves_the_primary_alone() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    let first = grow_state(&mut disk);
    assert!(!first.is_fault(), "{first:?}");
    corrupt_primary_header(&mut disk);
    disk.writes.clear();
    disk.filesystem_resizes.clear();
    let recovered = grow_state(&mut disk);
    assert!(!recovered.is_fault(), "{recovered:?}");

    // The backup's ARRAY, not its header: this is the actual bytes the bug
    // shared with the primary by pointing both headers at one copy. Damaging
    // only it must not touch what a primary-only read sees — corrupting the
    // header instead would not have caught the bug, since each header always
    // had its own distinct bytes even when the two arrays were one.
    let backup_entries_lba = disk.lbas - 33;
    let mut sector = disk.sector(backup_entries_lba);
    sector[0] ^= 0xFF;
    disk.place(backup_entries_lba, &sector);

    let primary_only = read_copy(
        &mut disk,
        PRIMARY_HEADER_LBA,
        (Step::ReadPrimaryHeader, Step::ReadPrimaryEntries),
    )
    .unwrap_or_else(|_| panic!("the fake cannot fail a read"));
    assert!(
        primary_only.is_ok(),
        "corrupting the backup array must not touch what the primary reads: {primary_only:?}"
    );
}

/// When neither copy verifies, the *primary's* complaint is the one reported —
/// it is the copy a reader prefers and the one a human will look at first.
#[test]
fn when_neither_copy_verifies_the_primarys_complaint_is_the_one_reported() {
    let mut disk = FakeDisk::written_card(CARD_LBAS);
    disk.sectors.clear();
    let mut sector = vec![0u8; LBA_BYTES as usize];
    sector[..8].copy_from_slice(&SIGNATURE);
    sector[12..16].copy_from_slice(&92u32.to_le_bytes());
    disk.place(1, &sector);

    let got = grow_state(&mut disk);
    let Grew::Refused { why } = &got else {
        panic!("{got:?}");
    };
    assert!(
        matches!(why, Refusal::HeaderChecksum { .. }),
        "the primary's own complaint, not the backup's NotGpt: {why}"
    );
}

/// A device too small to hold the table that is on it is not a device to write
/// a table to.
#[test]
fn a_device_smaller_than_its_own_table_is_refused() {
    let table = fixture_table(IMAGE_LBAS);
    let got = assess(&table, 40);
    assert!(matches!(got, Err(Refusal::DeviceTooSmall { .. })), "{got:?}");
}

/// **A card SMALLER than the image is refused rather than described wrongly.**
///
/// The realistic case, and it needs no damaged table: two cards sold as the same
/// size do not have the same number of sectors, so an image built at 4 GiB and
/// written to a card a megabyte short arrives here with partition 5 ending above
/// anything this device can address.
///
/// Before this was checked, `assess` took `aligned_end(last_usable).max(state
/// .last_lba)` and kept the partition where it was — then `rewrite` composed a
/// header saying `last_usable_lba = 4192222` over an entry array saying
/// partition 5 ends at 4192255, and wrote both copies. That is a GPT whose two
/// halves contradict each other, produced by the one operation on a Rivet whose
/// entire job is to leave a card bootable, on the first boot of a new card.
///
/// The guard that was supposed to catch it compared the new end against the
/// partition's own FIRST sector, which a `max` including its last sector can
/// never fall below — so it never ran. `assert!(...is_err())` here is worth
/// little on its own; what makes this a regression test is that it fails with
/// the check removed, and it does.
#[test]
fn a_card_smaller_than_the_image_is_refused_rather_than_given_a_contradictory_table() {
    // One MiB short of the image the fixture was built for.
    let short = IMAGE_LBAS - 2048;
    let table = fixture_table(short);
    let got = assess(&table, short);
    let Err(Refusal::DeviceTooSmall {
        device_lbas,
        needs_lbas,
    }) = got
    else {
        panic!("a card that cannot hold partition 5 must be refused, got {got:?}");
    };
    assert_eq!(device_lbas, short);
    assert!(
        needs_lbas > device_lbas,
        "the refusal has to name a size larger than the device or it explains nothing: \
         {needs_lbas} vs {device_lbas}"
    );
    // Partition 5's own end, plus its entry array and the backup header above
    // it. 4192255 + 2 + 32.
    assert_eq!(needs_lbas, STATE_LAST_ON_IMAGE + 34);

    // And the boundary: exactly large enough for partition 5's last sector,
    // its entry array and the backup header is ACCEPTED, because at that size
    // `last_usable_lba` lands exactly on the partition's end.
    let exact = STATE_LAST_ON_IMAGE + 34;
    let table = fixture_table(exact);
    let plan = assess(&table, exact).expect("the smallest device that can hold this table");
    assert_eq!(plan.last_usable_lba, STATE_LAST_ON_IMAGE);
    assert_eq!(
        plan.state_last_lba_new, STATE_LAST_ON_IMAGE,
        "there is no room to grow into and none is claimed"
    );
    assert!(plan.state_last_lba_new <= plan.last_usable_lba);
}
