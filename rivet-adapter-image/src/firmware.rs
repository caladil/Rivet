//! What the Raspberry Pi firmware publishes, and where the A/B layout lives on
//! the card.
//!
//! **Moved here from `cmd/rivet-initd/src/provision.rs` on 2026-08-23, bodies
//! unchanged**, when [`crate::slots`] moved into a library crate. Two callers now
//! need these facts — this crate's [`crate::slots::PiFirmware`], and PID 1's
//! provisioning reader — and `ADR-011` says a fact lives in exactly one place.
//! `cmd/rivet-initd` reads them from here rather than keeping a second copy;
//! `ADR-014` §3 permits that edge explicitly, because `cmd/*` may depend on
//! everything.
//!
//! The alternative was leaving them where they were and having this crate spell
//! them again, which is exactly the failure `PLAN.md` §0.0.2 item 1 is the
//! epitaph for: two agents wrote one program's path the same day, each
//! internally consistent and tested, and disagreed.
//!
//! # What is unproven here
//!
//! [`BOOT_DISK`] is documentation-derived from `ADR-002` §4's `cmdline.txt` and
//! **confirmed on hardware 2026-08-23 for `p1` only**. `ADR-026` §4 puts
//! `/proc/device-tree/chosen/bootloader` in the column QEMU does not emulate at
//! all, so [`DT_TRYBOOT`] and [`DT_BOOT_PARTITION`] are read by unit tests
//! against a fake and, for the second of them, by two boots of one board.

use rivet_core::state::{AUTOBOOT_PARTITION, partition_device};

use crate::port::{BootMedium, Mounted};

/// The block device the Raspberry Pi firmware boots from.
///
/// Documentation-derived: `ADR-002` §4's `cmdline.txt` names `/dev/mmcblk0p2`
/// as the root device, so `/dev/mmcblk0` is the disk.
///
/// **Confirmed against hardware 2026-08-23.** On Rivet's first boot on a Pi 4
/// Model B, PID 1 reported
/// `mounted /dev/mmcblk0p1 read-only on /run/boot-partition` and read the
/// provisioning document through it. The inference held.
///
/// It holds for an SD card. A board booting the same image from USB would
/// present `/dev/sda`, and this constant would be wrong — the partition is
/// found by number from the device tree ([`DT_BOOT_PARTITION`]) but the *disk*
/// is still assumed here.
pub const BOOT_DISK: &str = "/dev/mmcblk0";

/// The filesystem both boot partitions carry. `ADR-018` §4.
///
/// **Named rather than left to a kernel guess**, for the reason
/// `rivet_core::state::STATE_FSTYPE` gives about `ext4`: getting a filesystem
/// type wrong is not always a refusal, and a mount that succeeds as the wrong
/// type is worse than one that fails.
pub const BOOT_FSTYPE: &str = "vfat";

/// Where the partition holding the authoritative `autoboot.txt` is mounted.
///
/// **A separate mount point from `cmd/rivet-initd`'s `BOOT_PARTITION_MOUNT` on
/// purpose.** The two name the same filesystem in slot A and different ones in
/// slot B, and [`BootMedium::mount_boot_filesystem`] treats "already mounted
/// here" as the convergent answer. Sharing one path would make that convergence
/// hand back the *booted* partition when this crate asked for the first one —
/// silently writing the commit to the wrong file, which is the whole of the bug
/// this constant exists to fix.
pub const AUTOBOOT_PARTITION_MOUNT: &str = "/run/autoboot-partition";

/// Where an update mounts the boot half of the slot it is **writing**.
///
/// A third mount point, and for the reason [`AUTOBOOT_PARTITION_MOUNT`] is a
/// second one. The three name different filesystems and
/// [`BootMedium::mount_boot_filesystem`] treats *already mounted here* as the
/// convergent answer, so a shared path would hand a caller the wrong partition
/// without failing:
///
/// | Mount point | Whose | Which partition |
/// | :-- | :-- | :-- |
/// | `cmd/rivet-initd`'s `BOOT_PARTITION_MOUNT` | PID 1's provisioning read | the **booted** one |
/// | [`AUTOBOOT_PARTITION_MOUNT`] | the commit and the arming | always `rivet_core::state::AUTOBOOT_PARTITION` |
/// | this | an update's boot-half write | the **inactive** slot's |
///
/// The third is the one where sharing would be worst: when slot A is inactive,
/// all three name `p1`, and an update's writable mount converging onto PID 1's
/// read-only one would fail at the first write with `EROFS` — or, the other way
/// round, would leave the firmware's own filesystem mounted writable for the
/// rest of the boot.
pub const PAYLOAD_BOOT_MOUNT: &str = "/run/update-boot-target";

/// The advisory lock every process holds while it has the partition at
/// [`AUTOBOOT_PARTITION_MOUNT`] mounted — PID 1's commit, an arming, and every
/// reader of the default slot. See [`BootMedium::lock`].
///
/// **Under `/run/rivet`**, the machine's runtime directory on the `/run` tmpfs,
/// which starts empty on every boot: no lock outlives the boot that took it.
/// The directory is created by whichever process takes the lock first, because
/// PID 1's commit can precede the supervisor's first use of it. The file is
/// created mode `0600` by a root process, so an application granted
/// `/run/rivet` that does not run as root cannot open it, and therefore cannot
/// hold it and keep a trial boot from being committed.
pub const AUTOBOOT_LOCK: &str = "/run/rivet/autoboot.lock";

/// The first-boot provisioning document. `ADR-004` §6.
///
/// **Here because `ADR-029` §4 makes it a *reserved name*** — see
/// [`crate::slots::BootSlots::reserved_names`]. Nothing in this crate reads or
/// writes the file; what this crate owns is the fact that an update payload may
/// not carry one.
///
/// **It is spelled twice more in this workspace and this is the third**, which
/// is `ADR-011`'s second-authority problem and is recorded rather than quietly
/// added to: `cmd/rivet-image/src/boot.rs` writes the file when it builds a
/// card, and `cmd/rivet-initd/src/provision.rs` reads it at boot. Both are
/// **binary crates with no lib target**, so neither could ever have been the one
/// place — the same shape as `ADR-014` §2.2's argument for creating this crate.
/// This is a library and can be, so the two should come to read it from here;
/// `cmd/rivet-initd`'s test module carries a drift guard in the meantime,
/// because a guard that fails today is worth more than a rename nobody has time
/// for.
pub const PROVISIONING_FILE: &str = "rivet.toml";

/// Which partition the firmware booted from.
///
/// Confirmed on hardware 2026-08-22 (`ADR-018` §5.3): this property exists and
/// read `1` on a normal boot and on a tryboot boot alike. It is the authority
/// for which of `ADR-018` §4's two boot partitions is the live one — `ADR-011`,
/// the firmware owns that fact and we read it rather than assuming.
pub const DT_BOOT_PARTITION: &str = "/proc/device-tree/chosen/bootloader/partition";

/// The one-shot flag saying this is a trial boot. `ADR-018` §5.3.
pub const DT_TRYBOOT: &str = "/proc/device-tree/chosen/bootloader/tryboot";

/// Read a `/proc/device-tree` property as an unsigned number.
///
/// A device-tree property is bytes. The firmware may publish a number as a
/// big-endian cell or as a NUL-terminated ASCII string, and `ADR-018` §5.3
/// recorded the value only as it appeared under `cat` — which cannot tell the
/// two apart. So both are accepted **and nothing else is**: an unrecognised
/// encoding is refused with its length, per `AGENTS.md` §3, rather than being
/// coerced into a number that would silently select the wrong partition.
///
/// The refusal names the byte length and the bytes in hex. That is safe to
/// print: these properties are partition numbers and flags published by the
/// firmware, never operator content — the zero-scrub rule (`AGENTS.md` §8.3)
/// binds the provisioning file, which is never parsed by this function.
pub fn parse_dt_u32(bytes: &[u8]) -> Result<u32, String> {
    // A big-endian cell: exactly four bytes.
    if bytes.len() == 4
        && let Some(cell) = bytes.first_chunk::<4>()
    {
        return Ok(u32::from_be_bytes(*cell));
    }
    // A string, with or without the terminating NUL.
    let text = bytes.strip_suffix(b"\0").unwrap_or(bytes);
    if !text.is_empty()
        && text.len() <= 10
        && text.iter().all(u8::is_ascii_digit)
        && let Ok(s) = core::str::from_utf8(text)
        && let Ok(n) = s.parse::<u32>()
    {
        return Ok(n);
    }
    let hex: String = bytes
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    Err(format!(
        "{} byte(s) [{hex}] is neither a big-endian cell nor an ASCII number",
        bytes.len()
    ))
}

/// The device node for the boot partition the firmware actually booted.
///
/// Refuses rather than assuming `p1`. `ADR-018` §4 has two boot partitions and
/// picking the wrong one would read a stale provisioning file, or — worse, in
/// [`crate::slots`] — commit the wrong slot.
pub fn boot_partition_device(m: &mut dyn BootMedium) -> Result<(u32, String), String> {
    let bytes = m
        .read_property(DT_BOOT_PARTITION)
        .map_err(|e| format!("cannot read {DT_BOOT_PARTITION}: {e}"))?;
    let n = parse_dt_u32(&bytes).map_err(|why| format!("{DT_BOOT_PARTITION}: {why}"))?;
    Ok((n, partition_device(BOOT_DISK, n)))
}

/// The device node for the partition whose `autoboot.txt` the firmware reads.
///
/// **Not the booted partition.** `rivet_core::state::AUTOBOOT_PARTITION`
/// carries the hardware evidence and what believing otherwise would have cost.
/// Takes no [`BootMedium`] because nothing is read to determine it — that is the
/// point of the fact.
pub fn autoboot_partition_device() -> String {
    partition_device(BOOT_DISK, AUTOBOOT_PARTITION)
}

/// The partition holding the authoritative `autoboot.txt`, as one call to
/// [`mount_autoboot_partition`] left it.
///
/// **Always a mount this caller made**, because [`mount_autoboot_partition`]
/// reclaims any mount it finds — so [`umount_autoboot_partition`] always
/// unmounts it.
///
/// **Not `Clone` and not `Copy`**: the release consumes it, so one mount is
/// released once.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub struct AutobootMount {
    /// The device mounted, so a caller reporting what happened names what the
    /// system actually did rather than what it asked for (`AGENTS.md` §5).
    pub device: String,
}

/// Mount the partition holding the authoritative `autoboot.txt`, reclaiming
/// an orphaned mount found at [`AUTOBOOT_PARTITION_MOUNT`].
///
/// **The caller must already hold [`AUTOBOOT_LOCK`]** — see
/// [`BootMedium::lock`]; `crate::slots` takes it around every mount it makes.
/// That is what makes the reclaim safe: every process that mounts at this
/// mount point holds the lock from its mount to its unmount, so a mount found
/// there by a lock-holder belongs to nobody still using it — a process that
/// died, or whose unmount failed. It is unmounted, reported through
/// [`BootMedium::reclaimed_orphan`], and the partition mounted afresh with the
/// flags this caller asked for. Adopting it instead would read and write
/// through whatever flags the dead process chose, and a leaked read-only
/// mount would then refuse every commit for the rest of the boot.
///
/// A mount that is still there after the unmount — the unmount failed — is a
/// refusal: it cannot be reclaimed, and writing through it is what the reclaim
/// exists to avoid.
pub fn mount_autoboot_partition(
    m: &mut dyn BootMedium,
    writable: bool,
) -> Result<AutobootMount, String> {
    let device = autoboot_partition_device();
    let refused = |e: crate::port::Errno| format!("cannot mount {device} on {AUTOBOOT_PARTITION_MOUNT}: {e}");
    match m
        .mount_boot_filesystem(&device, AUTOBOOT_PARTITION_MOUNT, BOOT_FSTYPE, writable)
        .map_err(refused)?
    {
        Mounted::Made => Ok(AutobootMount { device }),
        Mounted::Adopted => {
            m.unmount(AUTOBOOT_PARTITION_MOUNT);
            match m
                .mount_boot_filesystem(&device, AUTOBOOT_PARTITION_MOUNT, BOOT_FSTYPE, writable)
                .map_err(refused)?
            {
                Mounted::Made => {
                    m.reclaimed_orphan(AUTOBOOT_PARTITION_MOUNT);
                    Ok(AutobootMount { device })
                }
                Mounted::Adopted => Err(format!(
                    "an orphaned mount at {AUTOBOOT_PARTITION_MOUNT} could not be unmounted, so \
                     {device} was not mounted afresh and nothing was read or written through it"
                )),
            }
        }
    }
}

/// Release what [`mount_autoboot_partition`] returned.
///
/// The reporting of a failed unmount is [`BootMedium::unmount`]'s — see that
/// method for why the outcome belongs to the implementor rather than to this
/// call.
pub fn umount_autoboot_partition(m: &mut dyn BootMedium, mount: AutobootMount) {
    let AutobootMount { device: _ } = mount;
    m.unmount(AUTOBOOT_PARTITION_MOUNT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Errno;
    use crate::testio::{FakeMedium, Script};

    // -- device-tree property decoding ---------------------------------------
    //
    // Moved from `cmd/rivet-initd/src/provision.rs` with the function they
    // exercise; bodies unchanged.

    #[test]
    fn a_big_endian_cell_decodes() {
        assert_eq!(parse_dt_u32(&[0, 0, 0, 1]), Ok(1));
        assert_eq!(parse_dt_u32(&[0, 0, 0, 0]), Ok(0));
        assert_eq!(parse_dt_u32(&[0, 0, 1, 0]), Ok(256));
    }

    #[test]
    fn an_ascii_number_decodes_with_or_without_its_nul() {
        assert_eq!(parse_dt_u32(b"1"), Ok(1));
        assert_eq!(parse_dt_u32(b"1\0"), Ok(1));
        assert_eq!(parse_dt_u32(b"0\0"), Ok(0));
        assert_eq!(parse_dt_u32(b"12"), Ok(12));
    }

    #[test]
    fn an_unrecognised_encoding_is_refused_rather_than_coerced() {
        // Choosing the wrong boot partition from a misread property would
        // commit the wrong slot in `crate::slots`.
        for bytes in [
            b"".as_slice(),
            b"x".as_slice(),
            b"1 2".as_slice(),
            &[1, 2, 3],
        ] {
            let got = parse_dt_u32(bytes);
            assert!(got.is_err(), "{bytes:?} must be refused, got {got:?}");
        }
    }

    #[test]
    fn a_refusal_names_the_length_so_the_encoding_can_be_identified_on_the_board() {
        let err = parse_dt_u32(&[1, 2, 3]);
        assert!(
            err.is_err_and(|e| e.contains("3 byte")),
            "the length must be reported"
        );
    }

    #[test]
    fn four_ascii_digits_decode_as_a_number_not_as_a_cell() {
        // "1234" is four bytes, which is also a cell's width. The cell reading
        // wins by length, which is a real ambiguity and is recorded as a test
        // rather than left to be discovered.
        assert_eq!(parse_dt_u32(b"1234"), Ok(u32::from_be_bytes(*b"1234")));
    }

    // -- locating the partition ----------------------------------------------

    #[test]
    fn the_device_comes_from_the_firmware_rather_than_from_an_assumption() {
        let mut m = FakeMedium::new();
        m.properties_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(vec![0, 0, 0, 2])),
        );
        assert_eq!(
            boot_partition_device(&mut m),
            Ok((2, "/dev/mmcblk0p2".to_string()))
        );
    }

    #[test]
    fn no_device_tree_means_no_boot_partition_rather_than_a_guess_at_p1() {
        let mut m = FakeMedium::new();
        let got = boot_partition_device(&mut m);
        assert!(got.is_err(), "must refuse, got {got:?}");
        assert!(m.mount_calls.is_empty());
    }

    /// The one that the hardware settled: the file the firmware reads is on
    /// `ADR-018` §4's *first* boot partition, whatever booted.
    #[test]
    fn the_autoboot_partition_is_the_first_one_and_not_the_booted_one() {
        assert_eq!(autoboot_partition_device(), "/dev/mmcblk0p1");
        assert_eq!(
            autoboot_partition_device(),
            partition_device(BOOT_DISK, AUTOBOOT_PARTITION)
        );
    }

    /// The two mount points must never be the same string. If they were, a
    /// convergent "already mounted" would hand back the booted partition when
    /// this crate asked for the first one — the bug of 2026-08-23, restored.
    #[test]
    fn the_autoboot_mount_point_is_not_the_one_the_provisioning_read_uses() {
        assert_ne!(AUTOBOOT_PARTITION_MOUNT, "/run/boot-partition");
    }

    #[test]
    fn mounting_the_autoboot_partition_names_the_first_partition_and_its_filesystem() {
        let mut m = FakeMedium::new();
        assert_eq!(
            mount_autoboot_partition(&mut m, true),
            Ok(AutobootMount {
                device: "/dev/mmcblk0p1".to_string(),
            })
        );
        let call = m.mount_calls.first();
        assert!(call.is_some_and(|c| c.device == "/dev/mmcblk0p1"
            && c.at == AUTOBOOT_PARTITION_MOUNT
            && c.fstype == BOOT_FSTYPE
            && c.writable));
    }

    /// **A mount found in place is an orphan, and is reclaimed**: unmounted,
    /// reported, and the partition mounted afresh with the flags asked for.
    #[test]
    fn a_mount_found_in_place_is_reclaimed_and_mounted_afresh() {
        let mut m = FakeMedium::new();
        m.mount_results = Script::sequence(vec![Ok(Mounted::Adopted)], Ok(Mounted::Made));
        let got = mount_autoboot_partition(&mut m, true);
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(m.mount_calls.len(), 2);
        assert!(m.mount_calls.iter().all(|c| c.writable));
        assert_eq!(
            m.order,
            vec![
                format!("mount {AUTOBOOT_PARTITION_MOUNT}"),
                format!("unmount {AUTOBOOT_PARTITION_MOUNT}"),
                format!("mount {AUTOBOOT_PARTITION_MOUNT}"),
            ]
        );
        assert_eq!(m.reclaimed, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);
        if let Ok(mount) = got {
            umount_autoboot_partition(&mut m, mount);
        }
        assert_eq!(m.unmount_calls.len(), 2, "the fresh mount is released too");
    }

    /// An orphan that will not unmount is a refusal, never a mount written
    /// through with somebody else's flags.
    #[test]
    fn an_orphan_that_will_not_unmount_is_refused() {
        let mut m = FakeMedium::new();
        m.mount_results = Script::always(Ok(Mounted::Adopted));
        let got = mount_autoboot_partition(&mut m, true);
        assert!(
            got.as_ref().is_err_and(|why| why.contains("could not be unmounted")),
            "{got:?}"
        );
        assert!(m.reclaimed.is_empty());
    }

    #[test]
    fn a_partition_that_will_not_mount_reports_the_device_and_the_reason() {
        let mut m = FakeMedium::new();
        m.mount_results = Script::always(Err(Errno::ENODEV));
        let got = mount_autoboot_partition(&mut m, true);
        assert!(got.is_err_and(|e| e.contains("/dev/mmcblk0p1") && e.contains("ENODEV")));
    }
}
