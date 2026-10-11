//! **Which slot this machine is running**, resolved the way a running
//! supervisor can afford to ask: through sysfs when it is readable, and
//! through `/proc/self/mountinfo` alone when it is not.
//!
//! `rivet_core::state::Slot::from_mountinfo` treats the root device's minor as
//! its partition number, which holds only for a driver's *first* disk —
//! `sdb3` is minor 19, not 3, because `sdb`'s own base minor is 16
//! (`Documentation/admin-guide/devices.txt`). `rivet-core` performs no I/O
//! (`ADR-014` §3), so it cannot ask the kernel what a device's minor actually
//! means; this module is the platform adapter that can.

use rivet_core::state::{self, Slot};

use crate::kernel::{Kernel, MOUNTINFO_PATH};

/// Which slot this machine is running, or `None` when that cannot be said.
///
/// Prefers the root device's own sysfs `partition` attribute — the number the
/// kernel itself assigned when it parsed the partition table — over
/// `/proc/self/mountinfo`'s minor. Falls back to
/// [`Slot::from_mountinfo`]'s mountinfo-only reading whenever sysfs cannot be
/// read or its contents do not parse as a partition number, so a machine
/// whose sysfs is unreadable answers exactly as it always has.
///
/// `None` either way is ordinary rather than a fault: a development host and
/// the emulator both have no `/system` line for the kernel to have made a
/// sysfs entry for at all (`ADR-026` §4).
pub fn running_slot(k: &mut dyn Kernel) -> Option<Slot> {
    let text = k.read_file(MOUNTINFO_PATH).ok()?;
    let from_sysfs = state::root_device(&text).and_then(|(major, minor)| {
        let attribute = k
            .read_file(&format!("/sys/dev/block/{major}:{minor}/partition"))
            .ok()?;
        Slot::from_system_partition(attribute.trim().parse().ok()?)
    });
    from_sysfs.or_else(|| Slot::from_mountinfo(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    fn mountinfo_for(device: &str) -> String {
        format!("20 1 {device} / / rw,relatime - ext4 /dev/root rw\n")
    }

    fn set_file(k: &mut FakeKernel, path: &str, contents: &str) {
        k.files_mut()
            .insert(path.to_string(), Script::always(Ok(contents.to_string())));
    }

    /// **`sdb`'s own sysfs partition number wins over its minor.** `sdb3` is
    /// device 8:19, and sysfs at that address says `3` — the same board `sda3`
    /// would report, which is the whole point: two disks on the same layout
    /// must answer the same slot.
    #[test]
    fn sysfs_partition_3_at_sdb3s_own_device_gives_slot_a() {
        let mut k = FakeKernel::new();
        set_file(&mut k, MOUNTINFO_PATH, &mountinfo_for("8:19"));
        set_file(&mut k, "/sys/dev/block/8:19/partition", "3\n");
        assert_eq!(running_slot(&mut k), Some(Slot::A));
    }

    /// The same proof at `sdb4` (8:20), which must answer B.
    #[test]
    fn sysfs_partition_4_at_sdb4s_own_device_gives_slot_b() {
        let mut k = FakeKernel::new();
        set_file(&mut k, MOUNTINFO_PATH, &mountinfo_for("8:20"));
        set_file(&mut k, "/sys/dev/block/8:20/partition", "4\n");
        assert_eq!(running_slot(&mut k), Some(Slot::B));
    }

    /// Without a sysfs answer, `sdb3` falls back to the mountinfo-only
    /// reading — which is `from_system_partition(19)`, i.e. `None`. This is
    /// **today's behaviour**, kept rather than improved on, because a wrong
    /// guess is worse than an honest unknown (`AGENTS.md` §5).
    #[test]
    fn a_missing_sysfs_entry_falls_back_to_the_minor_reading() {
        let mut k = FakeKernel::new();
        set_file(&mut k, MOUNTINFO_PATH, &mountinfo_for("8:19"));
        // No entry at all for /sys/dev/block/8:19/partition: ENOENT.
        assert_eq!(running_slot(&mut k), None, "the minor, 19, is not a slot");
    }

    /// An unparseable sysfs answer falls back the same way a missing one does.
    #[test]
    fn an_unparseable_sysfs_entry_falls_back_to_the_minor_reading() {
        let mut k = FakeKernel::new();
        set_file(&mut k, MOUNTINFO_PATH, &mountinfo_for("8:19"));
        set_file(&mut k, "/sys/dev/block/8:19/partition", "not-a-number\n");
        assert_eq!(running_slot(&mut k), None);
    }

    /// The fallback still answers correctly on a driver's *first* disk, where
    /// the minor really is the partition number — a machine with no sysfs at
    /// all must behave exactly as it did before this module existed.
    #[test]
    fn the_fallback_still_resolves_a_first_disk_from_its_minor_alone() {
        let mut k = FakeKernel::new();
        set_file(&mut k, MOUNTINFO_PATH, &mountinfo_for("179:3"));
        assert_eq!(running_slot(&mut k), Some(Slot::A));
    }

    /// A machine with no readable mountinfo at all resolves nothing, from
    /// either source — there is no root device to ask sysfs about.
    #[test]
    fn no_mountinfo_at_all_resolves_nothing() {
        let mut k = FakeKernel::new();
        assert_eq!(running_slot(&mut k), None);
    }
}
