//! The partition table, as data.
//!
//! `ADR-018` §4 is the authority for what partitions exist and what is on
//! them. This module is that section expressed so a program can act on it, and
//! it is the **shared contract** the rest of this crate is built against:
//! [`boot`](crate::boot) fills the FAT partitions, [`system`](crate::system)
//! builds the erofs tree, and [`disk`](crate::disk) writes the whole thing.
//!
//! Nothing here performs I/O.
//!
//! **The `dead_code` allow below is temporary and must come out.** It is here
//! only because `boot`, `system` and `disk` are unwritten, so nothing consumes
//! this contract yet. The moment they do, delete it — a standing allow is how a
//! genuinely unused type survives for a year.

#![allow(dead_code)]

/// A slot in the A/B pair.
///
/// `ADR-018` §5 makes the pair the update mechanism: one slot runs while the
/// other is written, and `tryboot` selects between them. Naming the slot is
/// what stops "the other one" being computed in three different places.
///
/// **The four partition numbers moved to `rivet-core` on 2026-08-23**, when
/// `cmd/rivet-initd`'s boot-slot port needed the same mapping to say where a
/// slot's payload goes. This type stays because
/// this module is the authority for the partition *table* and its own
/// vocabulary — [`as_str`](Self::as_str) is the GPT label suffix and means
/// nothing to `rivet-core` — but the numbers are now read from there rather
/// than restated, exactly as [`STATE_PARTITION`] already is. Two copies of the
/// same four numbers in two crates is `PLAN.md` §0.0.2 item 1's failure, and a
/// comment asking for them to be kept in step is not a mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    /// The same slot in `rivet-core`'s vocabulary, which owns the numbers.
    const fn core(self) -> rivet_core::state::Slot {
        match self {
            Self::A => rivet_core::state::Slot::A,
            Self::B => rivet_core::state::Slot::B,
        }
    }

    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }

    /// The 1-based partition number of this slot's boot partition.
    ///
    /// `ADR-018` §4's order is boot-A, boot-B, system-A, system-B, state — and
    /// **the numbers matter beyond convenience**: `autoboot.txt`'s
    /// `boot_partition=` names one of these, and `ADR-018` §5.3's `tryboot`
    /// flag selects between them. `rivet-initd` reads which one it booted from
    /// `/proc/device-tree/chosen/bootloader/partition`.
    pub const fn boot_partition(self) -> u32 {
        self.core().boot_partition()
    }

    /// The 1-based partition number of this slot's `/system` partition.
    pub const fn system_partition(self) -> u32 {
        self.core().system_partition()
    }
}

/// The `/state` partition. One, not per-slot — `ADR-018` §4 is explicit that an
/// update replaces `/system` and **cannot touch `/state`**.
///
/// **Taken from `rivet-core` since 2026-08-23**, when `cmd/rivet-initd`
/// started mounting it and needed the same number to build a device path. This
/// module is still the authority for the partition *table*; the number itself is
/// now one fact in one place (`ADR-011`), which is what the standing
/// path-duplication debt asks for in the general case.
pub const STATE_PARTITION: u32 = rivet_core::state::STATE_PARTITION;

/// `/state`'s GPT label, from `rivet-core` for [`STATE_PARTITION`]'s reason.
///
/// **Taken from there since 2026-08-28** (`PLAN.md` §62), when `cmd/rivet-initd`
/// started *finding* `/state` by this label instead of composing a device path
/// from a disk name it had assumed. That makes the string a run-time contract
/// between the tool that writes the card and the machine that boots on it, and
/// `ADR-011` gives a contract exactly one definition.
pub const STATE_PARTLABEL: &str = rivet_core::state::STATE_PARTLABEL;

/// What a partition is for, and how it must be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionSpec {
    /// 1-based, and load-bearing — see [`Slot::boot_partition`].
    pub number: u32,
    /// The GPT partition label.
    pub label: &'static str,
    /// The filesystem as `mkfs` names it.
    pub fstype: FsType,
    /// Size in mebibytes, or `None` for "the rest of the device".
    pub size_mib: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    /// The Raspberry Pi firmware reads only FAT (`ADR-018` §5), which is why
    /// the A/B pair needs two of them.
    Fat32,
    /// `/system`, read-only **by format** rather than by mount flag
    /// (`ADR-018` §3). That distinction is the point: a mount flag can be
    /// remounted, a format cannot.
    Erofs,
    /// `/state`. `ADR-018` §7 chose `ext4` over `f2fs` deliberately.
    Ext4,
}

impl FsType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fat32 => "vfat",
            Self::Erofs => "erofs",
            Self::Ext4 => "ext4",
        }
    }
}

/// Default sizes, chosen for the **8 GB spare card** rather than the 128 GB
/// working one, because the small card is the constraint and a layout that fits
/// it fits both. On the 8 GB card this leaves `/state` roughly 5.5 GB.
///
/// `ADR-018` §9 item 2 leaves sizing open and warns that a fixed size has to be
/// guessed years in advance. These are that guess, so they are named constants
/// with the reasoning attached rather than literals buried in a function:
///
/// * **Boot, 256 MiB each.** The Raspberry Pi firmware plus a kernel and device
///   trees is well under 100 MiB today. The slack is for a future kernel and an
///   initramfs, neither of which exists yet.
/// * **System, 640 MiB each.** Nothing measured yet — Stage 0's image is two
///   static binaries and a directory tree. `PLAN.md` §6 tracks image size as a
///   number to be measured, and this should be revisited the moment there is
///   one.
/// * **State takes the remainder**, per `ADR-018` §4's ruling that a fixed size
///   would be a guess and that a reserved-space guarantee inside one partition
///   is the adjustable answer.
pub const BOOT_SIZE_MIB: u64 = 256;
pub const SYSTEM_SIZE_MIB: u64 = 640;

/// What `/system` is formatted as **today**, which is not what `ADR-018` §3
/// says it should be.
///
/// **Ruled 2026-08-22, as a stopgap with a retirement condition.** `ADR-018` §3
/// makes `/system` **erofs — read-only by *format*** rather than by mount flag,
/// and that distinction is the whole point: a mount flag can be remounted, a
/// format cannot.
///
/// **The stock Raspberry Pi kernel cannot mount it.** `ADR-024` §8.1 recorded
/// `EROFS_FS=m`, and a module cannot be loaded from the filesystem it is needed
/// to mount. Confirmed a second, independent way by decompressing
/// `tools/qemu/kernel8.img` and listing the filesystem names the kernel
/// registers: `cgroup2 devtmpfs ext2 ext4 msdos proc sysfs tmpfs vfat` — no
/// erofs. (Careful: the image *does* contain 38 occurrences of `EROFS`, which
/// are the **errno** for "read-only file system" in error paths, not the
/// filesystem. Reading those as support is the obvious way to get this wrong.)
///
/// So the first cards ship `ext4` mounted read-only — read-only **by flag**,
/// which `ADR-018` §3 explicitly calls the weaker property. That is accepted
/// deliberately in order to reach a first boot and prove PID 1, the mount plan,
/// reaping, the supervisor and `tryboot` on real hardware, none of which
/// depends on the root filesystem's format.
///
/// **The retirement condition, stated now so it is testable later:** when
/// `ADR-024` §8's custom kernel ships with `EROFS_FS=y`, this becomes
/// [`FsType::Erofs`] and this constant is deleted. It does **not** survive on
/// the grounds that ext4 works — the reason erofs was chosen is in `ADR-018` §3
/// and has not changed.
pub const SYSTEM_FSTYPE: FsType = FsType::Ext4;

/// The whole table, in on-disk order.
pub fn partition_table() -> Vec<PartitionSpec> {
    vec![
        PartitionSpec {
            number: 1,
            label: "boot-a",
            fstype: FsType::Fat32,
            size_mib: Some(BOOT_SIZE_MIB),
        },
        PartitionSpec {
            number: 2,
            label: "boot-b",
            fstype: FsType::Fat32,
            size_mib: Some(BOOT_SIZE_MIB),
        },
        PartitionSpec {
            number: 3,
            label: "system-a",
            fstype: SYSTEM_FSTYPE,
            size_mib: Some(SYSTEM_SIZE_MIB),
        },
        PartitionSpec {
            number: 4,
            label: "system-b",
            fstype: SYSTEM_FSTYPE,
            size_mib: Some(SYSTEM_SIZE_MIB),
        },
        PartitionSpec {
            // The label, from rivet-core rather than spelled here, for the
            // reason the number already is: cmd/rivet-initd looks the running
            // machine's /state up BY this string (PLAN.md §62), so a second
            // literal here is a card written with one name and read for
            // another. ADR-011.
            number: STATE_PARTITION,
            label: STATE_PARTLABEL,
            fstype: FsType::Ext4,
            size_mib: None,
        },
    ]
}

/// The smallest device this layout fits on, in mebibytes.
///
/// Two boot partitions, two system partitions, and enough left that `/state`
/// is not a rounding error. A card smaller than this is refused rather than
/// partially written — `AGENTS.md` §3.
pub const fn minimum_device_mib() -> u64 {
    2 * BOOT_SIZE_MIB + 2 * SYSTEM_SIZE_MIB + 256
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `rivet_core::state::AUTOBOOT_PARTITION` is the firmware's fact — the
    /// partition whose `autoboot.txt` it reads, proven on hardware 2026-08-23.
    /// This table is what actually gets written to a card. If the two ever
    /// disagree, `cmd/rivet-initd` commits into a file that is not slot A's
    /// boot partition, which is the failure that correction was for.
    #[test]
    fn the_partition_the_firmware_reads_autoboot_from_is_slot_as_boot_partition() {
        assert_eq!(
            Slot::A.boot_partition(),
            rivet_core::state::AUTOBOOT_PARTITION
        );
    }

    /// **The label is a run-time contract, not a cosmetic.** `cmd/rivet-initd`
    /// finds `/state` by looking for this exact string on a partition
    /// (`PLAN.md` §62), so a card written with a different one is a machine
    /// that boots and cannot find its own storage. Asserted against the value
    /// that actually reaches the table rather than against the constant that
    /// fed it — comparing a constant with itself catches nothing.
    #[test]
    fn the_state_partitions_label_is_the_one_a_running_machine_looks_for() {
        let state: Vec<_> = partition_table()
            .into_iter()
            .filter(|s| s.number == STATE_PARTITION)
            .collect();
        assert_eq!(state.len(), 1, "one partition is /state, no more and no fewer");
        assert_eq!(state[0].label, rivet_core::state::STATE_PARTLABEL);
        assert_eq!(state[0].label, "state");
    }

    #[test]
    fn the_table_is_adr_018_4s_five_partitions_in_its_order() {
        let t = partition_table();
        let got: Vec<_> = t.iter().map(|p| (p.number, p.label, p.fstype)).collect();
        assert_eq!(
            got,
            [
                (1, "boot-a", FsType::Fat32),
                (2, "boot-b", FsType::Fat32),
                (3, "system-a", SYSTEM_FSTYPE),
                (4, "system-b", SYSTEM_FSTYPE),
                (5, "state", FsType::Ext4),
            ]
        );
    }

    /// The numbers are not decoration: `autoboot.txt` names them and the
    /// firmware reports which one it used.
    #[test]
    fn slot_partition_numbers_match_the_table() {
        assert_eq!(Slot::A.boot_partition(), 1);
        assert_eq!(Slot::B.boot_partition(), 2);
        assert_eq!(Slot::A.system_partition(), 3);
        assert_eq!(Slot::B.system_partition(), 4);
    }

    /// This crate's `Slot` and `rivet-core`'s must name the same partitions,
    /// in both directions. They are one mapping since 2026-08-23 and this is
    /// what keeps that true if either grows a variant.
    #[test]
    fn this_slot_and_rivet_cores_are_the_same_mapping() {
        for (mine, theirs) in [
            (Slot::A, rivet_core::state::Slot::A),
            (Slot::B, rivet_core::state::Slot::B),
        ] {
            assert_eq!(mine.boot_partition(), theirs.boot_partition());
            assert_eq!(mine.system_partition(), theirs.system_partition());
            assert_eq!(
                mine.other().boot_partition(),
                theirs.other().boot_partition()
            );
            assert_eq!(
                rivet_core::state::Slot::from_boot_partition(mine.boot_partition()),
                Some(theirs)
            );
        }
    }

    #[test]
    fn a_slot_has_exactly_one_other_and_it_is_not_itself() {
        assert_eq!(Slot::A.other(), Slot::B);
        assert_eq!(Slot::B.other(), Slot::A);
        assert_ne!(Slot::A.other(), Slot::A);
    }

    #[test]
    fn only_state_takes_the_remainder_of_the_device() {
        let sized: Vec<_> = partition_table()
            .into_iter()
            .filter(|p| p.size_mib.is_none())
            .map(|p| p.number)
            .collect();
        assert_eq!(sized, [STATE_PARTITION]);
    }

    #[test]
    fn the_minimum_device_holds_every_fixed_partition_with_room_for_state() {
        let fixed: u64 = partition_table().iter().filter_map(|p| p.size_mib).sum();
        assert!(minimum_device_mib() > fixed);
    }
}
