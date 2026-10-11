//! Reading this machine's block devices — `ADR-035` phase 4, and
//! `docs/design-candidates/Admin-Surface-Survey.md` §7.3.
//!
//! **`/sys/class/block/*`, and the `uevent` and `stat` files under each.**
//! `crate::journal`'s note carries why that belongs here rather than behind
//! `BootMedium`: no `mount(2)`, no device tree, no pid 1 — and, pointedly, no
//! *mount* at all, which is what separates this from `crate::storage`.
//!
//! # The unit trap in `size`
//!
//! **`/sys/class/block/<name>/size` is in 512-byte sectors ALWAYS**, whatever
//! the device's logical block size is. Multiplying it by the logical block size
//! is the classic error and it inflates a 4 KiB-sector device eightfold. The
//! conversion happens once, in [`bytes_of_sectors`], and there is a test with a
//! real card's sector count in it.
//!
//! The same 512 applies to `stat`'s sector counts, and for the same reason — the
//! kernel's block layer counts in 512-byte units regardless of the hardware.
//!
//! # The number this refuses to invent
//!
//! Survey §7.3 was asked for *"remaining endurance"* and split the answer: a
//! JEDEC 5.0+ eMMC genuinely reports `life_time` and `pre_eol_info`; **a
//! consumer SD card generally does not**, and producing a figure for one would
//! be inventing it. So both are read where the device offers them, carried
//! **verbatim**, and absent where it does not. Nothing here turns a vendor code
//! into a percentage.
//!
//! What *is* always there is the write volume, and §7.3 is right that it is the
//! more useful number anyway: `ADR-018` §7 made the journal-on-disk decisions
//! for SD wear without ever measuring any.

use rivet_core::operation::machine::{BlockDevice, BlockIo, DeviceRead, MachineError, Unmeasured};

/// One symlink per block device, whole disks and partitions alike.
const CLASS_BLOCK: &str = "/sys/class/block";

/// **The block layer's unit, and it is not the device's.** `size` and `stat`
/// both count 512-byte sectors whatever the hardware's logical block size is.
const SECTOR_BYTES: u64 = 512;

/// Every block device this machine has.
///
/// # Errors
///
/// If `/sys/class/block` could not be listed — on Linux a fault, and elsewhere
/// the absence of the whole interface. **A device that could not be read is not
/// an error**: it lands in [`DeviceRead::devices_unreadable`] with the reason.
pub fn read() -> Result<DeviceRead, MachineError> {
    let entries = std::fs::read_dir(CLASS_BLOCK).map_err(|e| {
        MachineError::Refused(format!(
            "{CLASS_BLOCK} could not be listed: {e}. On Linux that is a fault; on any other \
             platform there is no such directory and nothing about this machine's block devices \
             was measured."
        ))
    })?;

    let mut devices = Vec::new();
    let mut devices_unreadable = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(name) = file.to_str() else { continue };
        match device(name) {
            Ok(one) => devices.push(one),
            // **Counted, not dropped**: an answer that omits a device says the
            // machine does not have one, and on an A/B machine the device that
            // went missing is the one somebody is looking for.
            Err(why) => devices_unreadable.push(Unmeasured {
                what: name.to_string(),
                why,
            }),
        }
    }
    // Name order, so two readings of an unchanged machine produce the same
    // bytes — `crate::process`'s rule.
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices_unreadable.sort_by(|a, b| a.what.cmp(&b.what));

    Ok(DeviceRead {
        devices,
        devices_unreadable,
        source: CLASS_BLOCK.to_string(),
    })
}

/// One block device out of its `/sys/class/block` directory.
///
/// `uevent` is the one that must be there: a directory under
/// `/sys/class/block` without it is not a block device.
fn device(name: &str) -> Result<BlockDevice, String> {
    let dir = format!("{CLASS_BLOCK}/{name}");
    let text = |leaf: &str| -> Option<String> {
        std::fs::read_to_string(format!("{dir}/{leaf}"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let number = |leaf: &str| -> Option<u64> { text(leaf)?.parse().ok() };

    let uevent = std::fs::read_to_string(format!("{dir}/uevent"))
        .map_err(|e| format!("{dir}/uevent could not be read: {e}, so this is not a device"))?;
    let key = |name: &str| uevent_key(&uevent, name);

    Ok(BlockDevice {
        name: name.to_string(),
        kind: key("DEVTYPE"),
        node: key("DEVNAME"),
        // **The GPT label**, which is what `root=PARTLABEL=` names and what a
        // running machine finds `/state` by (`rivet_core::state`). On a
        // partition it is the most useful single string there is.
        partition_label: key("PARTNAME"),
        partition_number: number("partition").and_then(|n| u32::try_from(n).ok()),
        parent: parent_of(&dir),
        size_bytes: number("size").map(bytes_of_sectors),
        read_only: number("ro").map(|n| n != 0),
        removable: number("removable").map(|n| n != 0),
        model: text("device/model").or_else(|| text("device/name")),
        io: io(&text_at(&dir, "stat")),
        // Verbatim, per survey §7.3. An eMMC states these; an SD card does not,
        // and a percentage computed here would be the invented endurance figure
        // that section refuses.
        life_time: text("device/life_time"),
        pre_eol_info: text("device/pre_eol_info"),
    })
}

/// One `KEY=value` line out of a `uevent`.
///
/// **The `=` is what makes a key a key**, which is `crate::memory`'s colon in
/// another spelling: without it `DEVTYPE` would answer for `DEVTYPEX`.
fn uevent_key(uevent: &str, key: &str) -> Option<String> {
    uevent
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name == key).then_some(value)
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The whole disk a partition belongs to, or `None` for a whole disk.
///
/// A partition's sysfs directory is a child of its disk's, so the parent is the
/// name of the directory above — read from the resolved path rather than
/// guessed by stripping a `pN` suffix from the name, which is wrong for
/// `mmcblk0p3` in one direction and for a disk called `nvme0n1` in the other.
fn parent_of(dir: &str) -> Option<String> {
    let resolved = std::fs::canonicalize(dir).ok()?;
    let parent = resolved.parent()?;
    // A whole disk's parent is the bus or the block class, not another device.
    parent
        .join("dev")
        .exists()
        .then(|| parent.file_name()?.to_str().map(str::to_string))?
}

/// A whole file, or an empty string.
fn text_at(dir: &str, leaf: &str) -> String {
    std::fs::read_to_string(format!("{dir}/{leaf}")).unwrap_or_default()
}

/// A `/sys/class/block/<name>/stat` line as reads and writes.
///
/// Eleven or more whitespace-separated numbers; the four this build reports are
/// fields 1, 3, 5 and 7 — reads completed, sectors read, writes completed,
/// sectors written.
fn io(stat: &str) -> Option<BlockIo> {
    let column: Vec<u64> = stat
        .split_whitespace()
        .map(|word| word.parse().unwrap_or(0))
        .collect();
    // **All four or nothing.** A partial set invites a reader to compare a
    // present write count against an absent read count.
    if column.len() < 7 {
        return None;
    }
    Some(BlockIo {
        reads_completed: column[0],
        read_bytes: bytes_of_sectors(column[2]),
        writes_completed: column[4],
        written_bytes: bytes_of_sectors(column[6]),
    })
}

/// Sectors as bytes.
///
/// **512 always** — see this module's documentation. Saturating rather than
/// wrapping: a nonsense sector count should produce an implausibly large number
/// rather than a small plausible one.
const fn bytes_of_sectors(sectors: u64) -> u64 {
    sectors.saturating_mul(SECTOR_BYTES)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **512, not the logical block size.** A 32 GB card reports 62 333 952
    /// sectors; multiplying by a 4096 logical block size would report 255 GB.
    #[test]
    fn a_sector_count_becomes_bytes_at_five_hundred_and_twelve() {
        assert_eq!(bytes_of_sectors(62_333_952), 62_333_952 * 512);
        assert_eq!(bytes_of_sectors(0), 0);
        assert_eq!(
            bytes_of_sectors(u64::MAX),
            u64::MAX,
            "a nonsense count saturates rather than wrapping to something small"
        );
    }

    /// A `uevent` as the kernel writes one for a partition.
    #[test]
    fn a_uevent_yields_its_keys_and_nothing_else() {
        let uevent = "MAJOR=179\nMINOR=3\nDEVNAME=mmcblk0p3\nDEVTYPE=partition\n\
                      DISKSEQ=1\nPARTN=3\nPARTNAME=system-a\n";
        assert_eq!(uevent_key(uevent, "DEVTYPE").as_deref(), Some("partition"));
        assert_eq!(uevent_key(uevent, "DEVNAME").as_deref(), Some("mmcblk0p3"));
        assert_eq!(uevent_key(uevent, "PARTNAME").as_deref(), Some("system-a"));
        assert_eq!(uevent_key(uevent, "NOSUCHKEY"), None);
    }

    /// **A key is a whole key.** `DEVTYPE` must not answer for `DEVTYPEX`, and
    /// a prefix match is how `crate::network`'s nameserver defect happened.
    #[test]
    fn a_prefix_is_not_a_uevent_key() {
        let uevent = "DEVTYPEX=nonsense\nDEVTYPE=disk\n";
        assert_eq!(uevent_key(uevent, "DEVTYPE").as_deref(), Some("disk"));
        assert_eq!(uevent_key(uevent, "DEV"), None);
    }

    /// The four counters, from a real-shaped `stat` line.
    #[test]
    fn a_stat_line_yields_reads_and_writes_in_bytes() {
        let stat = "   12345      678  1234567    45678     9012      345  7654321    23456 \
                    0    12345    67890";
        let io = io(stat).expect("eleven fields is a stat line");
        assert_eq!(io.reads_completed, 12_345);
        assert_eq!(io.read_bytes, 1_234_567 * 512);
        assert_eq!(io.writes_completed, 9_012);
        assert_eq!(io.written_bytes, 7_654_321 * 512);
    }

    /// **A short `stat` yields nothing rather than four zeroes**, which would
    /// say this device has carried nothing.
    #[test]
    fn a_short_stat_line_is_no_counters_rather_than_zero_counters() {
        assert_eq!(io(""), None);
        assert_eq!(io("1 2 3"), None);
    }

    /// **The read either answers or refuses, and it never invents a device.**
    /// Runs on whatever host the suite is on — Linux answers, Windows refuses.
    #[test]
    fn a_reading_on_this_host_is_either_an_answer_or_a_named_refusal() {
        match read() {
            Ok(reading) => {
                assert_eq!(reading.source, CLASS_BLOCK);
                let mut names: Vec<&str> =
                    reading.devices.iter().map(|d| d.name.as_str()).collect();
                let sorted = {
                    let mut copy = names.clone();
                    copy.sort_unstable();
                    copy
                };
                names.dedup();
                assert_eq!(names.len(), reading.devices.len(), "a device is listed twice");
                assert_eq!(
                    sorted,
                    reading
                        .devices
                        .iter()
                        .map(|d| d.name.as_str())
                        .collect::<Vec<_>>(),
                    "the devices are not in name order"
                );
            }
            Err(refusal) => assert!(refusal.why().contains(CLASS_BLOCK), "{}", refusal.why()),
        }
    }
}
