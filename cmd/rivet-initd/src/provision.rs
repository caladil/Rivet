//! Item 2: read the provisioning file from the boot partition.
//!
//! `ADR-004` §6 settles the sequence and it settles who mounts:
//!
//! > 1. **PID 1 mounts the boot partition** and reads `rivet.toml`.
//!
//! So the partition is *not* already mounted when this runs — the kernel gave
//! PID 1 a root filesystem and nothing else. `ADR-002` §4 lists the file among
//! the boot partition's contents alongside `config.txt` and `autoboot.txt`.
//!
//! # Where it is mounted, and why not somewhere more obvious
//!
//! `/run/boot-partition`. `ADR-008` §3 says a top-level name must answer "who
//! owns this, and does it survive an update", and §3.1 rejected a top-level
//! `/config` for failing that test — so inventing a root directory here would
//! be the same crack in the same principle. The boot partition is firmware-
//! owned, FAT, and *replaced by an update* (`ADR-018` §5.2 writes the inactive
//! one), which makes it none of `/system`, `/state` or a root of its own. What
//! it actually is at runtime is a transient mount, and `/run` is exactly the
//! layout's name for runtime state that does not survive. It is also not
//! `/boot`: `AGENTS.md` §10.2 reserves "the boot partition" as a term that
//! specifically does **not** mean the Linux `/boot` convention.
//!
//! **The mount is transient in both directions.** It is mounted read-only,
//! read, and unmounted again, so a power cut during the rest of the boot cannot
//! corrupt a FAT filesystem that nothing is using. `rivet-adapter-image`
//! mounts it again, writable, for the one moment it needs to.
//!
//! # What is unproven here
//!
//! The device name `/dev/mmcblk0` is documentation-derived from `ADR-002` §4's
//! `cmdline.txt`, and the `vfat` filesystem type requires `CONFIG_VFAT_FS`,
//! which `ADR-024` §3 is the authority for and which this crate does not check.
//! Neither has been confirmed on the board by this code. `AGENTS.md` §10.5.
//!
//! # Where the firmware's facts went — 2026-08-23
//!
//! `BOOT_DISK`, the two `/proc/device-tree` properties, `parse_dt_u32`,
//! [`boot_partition_device`] and the autoboot partition's mount point were
//! defined here until [`crate::slots`]'s port moved into
//! `rivet-adapter-image`. Two crates now need them, so `ADR-011` puts them in
//! one place and everything here **reads them from that crate** — including this
//! module's own tests, and `crate::state`. There is deliberately no re-export:
//! a second name for the same constant is how the two answers start.

use rivet_adapter_image::{BOOT_FSTYPE, Mounted, boot_partition_device};
use rivet_config::Document;

use crate::kernel::{Errno, Kernel, MountFlags};
use crate::slots::KernelMedium;

/// Where the boot partition is mounted, transiently. See the module docs.
///
/// **Never `rivet_adapter_image::AUTOBOOT_PARTITION_MOUNT`**, and that
/// constant carries why: the two name the same filesystem in slot A and
/// different ones in slot B, and [`mount_boot_filesystem`] treats `EBUSY` as the
/// convergent answer.
pub const BOOT_PARTITION_MOUNT: &str = "/run/boot-partition";

/// `ADR-004` §6 names the file.
pub const PROVISIONING_FILE: &str = "rivet.toml";

/// What the provisioning file turned out to be.
///
/// Four outcomes, not three, because "there is no boot partition on this
/// machine" and "there is one and the file is not on it" are the same to a
/// caller and neither is a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provisioning {
    /// No document was offered. A valid state, not an error: a freshly written
    /// card need not carry one, and `ADR-004` §6 step 3 has the credential
    /// *removed* from the file after the first boot, so its absence is the
    /// steady state of every boot after the first.
    Absent { why: String },
    /// Read and understood.
    Loaded(Document),
    /// It is there and the bytes did not arrive.
    Unreadable { errno: Errno },
    /// It arrived and was refused. `ADR-007` §3.6.5 — Rivet does not accept
    /// an input it intends to discard, so nothing is defaulted around this.
    Refused { detail: String },
}

impl Provisioning {
    /// Whether this outcome permits the boot to be confirmed.
    ///
    /// **Ruled here.** An absent document is a valid state and confirms
    /// normally. A document that exists and could not be applied is not: the
    /// operator asked for something, the machine demonstrably did not do it,
    /// and confirming would make that machine the new default. `ADR-018` §5's
    /// A/B fallback is exactly the remedy for "this image is not what was
    /// wanted", and `ADR-017` §3.1.6 already established that for PID 1 "fatal"
    /// means "never confirm the boot".
    ///
    /// **This is a Stage 0 answer and it gets stronger, not weaker.** ~~Today
    /// the document declares only a schema version, so the cost of refusing it
    /// is small.~~ Corrected 2026-08-23: it declares `version`, `services` and
    /// `network`, and since [`crate::seed`] a refused document is also one that
    /// is never stored — so on a first boot the cost of refusing is already a
    /// machine with no address. `ADR-019` §3 puts wireless credentials and a
    /// fleet endpoint in it later, at which point it is an unreachable machine
    /// outright — the same escalation `ADR-017` §3.1.6 predicts for `/dev/pts`,
    /// arriving by the same route.
    ///
    /// That is the right direction: withholding the confirmation rolls the
    /// image back, and a document the *boot partition* holds is a fault an
    /// update can replace. Compare [`crate::seed`], where the fault is on
    /// `/state` and a rollback cannot reach it (`ADR-018` §8).
    pub fn confirms_boot(&self) -> bool {
        matches!(self, Self::Absent { .. } | Self::Loaded(_))
    }

    /// One line for the console.
    ///
    /// **The interface count is reported and the interfaces are not**, and the
    /// distinction is `ADR-020` §4's: a count cannot carry a credential, and
    /// this file is where a wireless passphrase arrives. What the line is for
    /// is answering, from a photograph of an HDMI console, whether the
    /// `[network]` table the operator wrote was actually read — which is the
    /// first question asked when a board comes up with no address.
    pub fn summary(&self) -> String {
        match self {
            Self::Absent { why } => format!("no provisioning document: {why}"),
            Self::Loaded(doc) => format!(
                "provisioning document read; schema version {}, {} service(s), {} interface(s)",
                doc.version(),
                doc.services().len(),
                doc.network().len()
            ),
            Self::Unreadable { errno } => {
                format!("provisioning document could not be read: {errno}")
            }
            Self::Refused { detail } => format!("provisioning document REFUSED: {detail}"),
        }
    }
}

/// Mount a boot filesystem at a mount point, read-only unless `writable`.
///
/// **The one place a FAT partition reaches `mount(2)` in this crate**, and both
/// of them go through it: the provisioning read below, and the `autoboot.txt`
/// rewrite in `rivet-adapter-image`, which arrives here through
/// [`crate::slots::KernelMedium`]. Stating the flag policy once is the point —
/// `ADR-014` §3 puts the `MS_*` translation in this crate, and a second copy of
/// "which bits a FAT partition is mounted with" is how two answers diverge.
///
/// Returns the raw [`Errno`] rather than prose, because the caller is what words
/// a failure and one of the two callers is behind a port that has no console.
/// On success it says whether this call made the mount or found one in place
/// ([`Mounted`]); a caller unmounts only one it made.
pub fn mount_boot_filesystem(
    k: &mut dyn Kernel,
    device: &str,
    at: &str,
    fstype: &str,
    writable: bool,
) -> Result<Mounted, Errno> {
    k.mkdir_p(at)?;

    // Never executable, never suid, never a device node: it is a FAT filesystem
    // an operator edits from a Windows machine (`ADR-002` §4.1), which is as
    // untrusted as anything on the board gets.
    let mut flags = MountFlags::NOEXEC
        .union(MountFlags::NOSUID)
        .union(MountFlags::NODEV);
    if !writable {
        flags = flags.union(MountFlags::RDONLY);
    }

    match k.mount(device, at, fstype, flags, None) {
        Ok(()) => Ok(Mounted::Made),
        // Already mounted here, which is the convergent answer — the same
        // reasoning as `boot::perform_mount`. Safe only because each caller has
        // its own mount point; see `rivet_adapter_image`'s
        // `AUTOBOOT_PARTITION_MOUNT`.
        //
        // **`EBUSY` alone is not "already mounted here"; the mount table is
        // what makes it true.** The kernel also answers `EBUSY` for this
        // device mounted ELSEWHERE with incompatible flags, and treating that
        // as convergence would read and write the bare mount-point directory
        // on `/run` as though it were the card.
        Err(Errno::EBUSY) if mounted_at(k, at) => Ok(Mounted::Adopted),
        Err(e) => Err(e),
    }
}

/// Does the kernel's mount table show something mounted at `at`?
///
/// A table that cannot be read answers `false`, which is the safe direction:
/// the caller then reports the `EBUSY` rather than treating an unknown as
/// convergence.
fn mounted_at(k: &mut dyn Kernel, at: &str) -> bool {
    k.read_mounts()
        .is_ok_and(|table| crate::boot::mount_table_has(&table, at))
}

/// Mount the boot partition, read-only unless `writable`.
pub fn mount_boot_partition(k: &mut dyn Kernel, writable: bool) -> Result<String, String> {
    // Reborrowed for the port call and kept afterwards, the same shape
    // `cmd/rivet-sysd/src/clock.rs` uses: which partition the firmware
    // booted is `rivet-adapter-image`'s answer, because it is the same
    // question `BootSlots::running_slot` asks and `ADR-011` allows one answer.
    let (_, device) = {
        let mut m = KernelMedium::new(&mut *k);
        boot_partition_device(&mut m)?
    };
    match mount_boot_filesystem(k, &device, BOOT_PARTITION_MOUNT, BOOT_FSTYPE, writable) {
        Ok(_) => Ok(device),
        Err(e) => Err(format!("cannot mount {device} on {BOOT_PARTITION_MOUNT}: {e}")),
    }
}

/// Unmount it again, reporting but not caring.
pub fn umount_boot_partition(k: &mut dyn Kernel) {
    if let Err(e) = k.umount(BOOT_PARTITION_MOUNT) {
        k.say(&format!("  could not unmount {BOOT_PARTITION_MOUNT}: {e}"));
    }
}

/// Item 2, end to end.
pub fn read_provisioning(k: &mut dyn Kernel) -> Provisioning {
    k.say("rivet-initd: reading the provisioning file from the boot partition");

    let device = match mount_boot_partition(k, false) {
        Ok(device) => device,
        Err(why) => {
            // No boot partition reachable is not a refusal: under an emulator
            // there is none at all (`ADR-026` §4 — the VideoCore boot chain is
            // the board's alone), and on a board that has one this says which
            // step failed.
            let outcome = Provisioning::Absent { why };
            k.say(&format!("  {}", outcome.summary()));
            return outcome;
        }
    };
    k.say(&format!(
        "  mounted {device} read-only on {BOOT_PARTITION_MOUNT}"
    ));

    let path = format!("{BOOT_PARTITION_MOUNT}/{PROVISIONING_FILE}");
    let outcome = match k.read_file(&path) {
        Err(Errno::ENOENT) => Provisioning::Absent {
            why: format!("{PROVISIONING_FILE} is not on the boot partition"),
        },
        Err(errno) => Provisioning::Unreadable { errno },
        Ok(text) => match Document::parse(&text) {
            Ok(doc) => Provisioning::Loaded(doc),
            // `ConfigError`'s own design keeps the offending *value* out of the
            // message and carries a line and column instead, precisely because
            // this file holds a wireless passphrase (`ADR-004` §6). Printing it
            // to the console is therefore safe, and printing the text we just
            // read would not be.
            Err(e) => Provisioning::Refused {
                detail: e.to_string(),
            },
        },
    };

    umount_boot_partition(k);
    k.say(&format!("  {}", outcome.summary()));
    if !outcome.confirms_boot() {
        k.say("  this boot will NOT be confirmed: the operator asked for something");
        k.say("  the machine could not apply (ADR-017 §3.1.6, ADR-018 §5).");
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_adapter_image::DT_BOOT_PARTITION;

    /// A passphrase-shaped value, so the console can be checked for leakage.
    /// `AGENTS.md` §8.3 — this is a test fixture, not a real secret.
    const NOT_A_REAL_SECRET: &str = "hunter2-correct-horse-battery";

    /// **A drift guard, added 2026-08-24 with `ADR-029` §4.**
    ///
    /// This file's name is now load-bearing in a third place:
    /// `rivet_adapter_image::firmware::PROVISIONING_FILE` is what
    /// `BootSlots::reserved_names` refuses an update payload for carrying, and
    /// the reason is that PID 1 *mutates* this file (`ADR-004` §6) and that
    /// `ADR-018` §8 makes it the recovery path for a damaged `/state`. An update
    /// restoring the blank template destroys both.
    ///
    /// `ADR-011` says one authority per fact, and the honest fix is for this
    /// constant to be deleted and read from the library — which this crate
    /// already depends on. Until somebody does that, a rename here that missed
    /// the library would leave the payload check reserving a name nothing uses,
    /// and this test is what fails instead.
    #[test]
    fn the_provisioning_file_pid_1_reads_is_the_one_an_update_payload_may_not_carry() {
        assert_eq!(
            PROVISIONING_FILE,
            rivet_adapter_image::firmware::PROVISIONING_FILE
        );
    }

    fn with_boot_partition(k: &mut FakeKernel, file: Script<Result<String, Errno>>) {
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(b"1".to_vec())),
        );
        k.files_mut()
            .insert(format!("{BOOT_PARTITION_MOUNT}/{PROVISIONING_FILE}"), file);
    }

    // -- locating the partition ----------------------------------------------
    //
    // `parse_dt_u32` and `boot_partition_device` moved to
    // `rivet-adapter-image`'s `firmware` module on 2026-08-23 and their tests
    // moved with them. What is tested here is what is still decided here: the
    // mount, its flags, and that the two reach each other through
    // `crate::slots::KernelMedium`.

    /// The refusal, reached through PID 1's own kernel rather than through the
    /// image crate's fake. Under QEMU there is no device tree at all
    /// (`ADR-026` §4), and assuming `p1` would read a stale provisioning file.
    #[test]
    fn no_device_tree_means_no_boot_partition_rather_than_a_guess_at_p1() {
        let mut k = FakeKernel::new();
        let got = mount_boot_partition(&mut k, false);
        assert!(got.is_err(), "must refuse, got {got:?}");
        assert!(k.mount_calls.is_empty());
    }

    #[test]
    fn the_boot_partition_is_mounted_read_only_and_never_executable() {
        let mut k = FakeKernel::new();
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(b"1".to_vec())),
        );
        let mounted = mount_boot_partition(&mut k, false);
        assert_eq!(mounted, Ok("/dev/mmcblk0p1".to_string()));
        let call = k.mount_calls.first();
        assert!(call.is_some_and(|c| c.source == "/dev/mmcblk0p1"
            && c.target == BOOT_PARTITION_MOUNT
            && c.fstype == "vfat"
            && c.flags.contains(MountFlags::RDONLY)
            && c.flags.contains(MountFlags::NOEXEC)
            && c.flags.contains(MountFlags::NOSUID)
            && c.flags.contains(MountFlags::NODEV)));
    }

    #[test]
    fn a_writable_mount_drops_only_the_read_only_bit() {
        let mut k = FakeKernel::new();
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(b"1".to_vec())),
        );
        let _ = mount_boot_partition(&mut k, true);
        let call = k.mount_calls.first();
        assert!(call.is_some_and(|c| !c.flags.contains(MountFlags::RDONLY)
            && c.flags.contains(MountFlags::NOEXEC)
            && c.flags.contains(MountFlags::NOSUID)));
    }

    #[test]
    fn a_boot_partition_already_mounted_converges_rather_than_failing() {
        let mut k = FakeKernel::new();
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(b"1".to_vec())),
        );
        k.mount_results = Script::always(Err(Errno::EBUSY));
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(format!("/dev/mmcblk0p1 {BOOT_PARTITION_MOUNT} vfat ro 0 0\n"))),
        );
        assert!(mount_boot_partition(&mut k, false).is_ok());
        assert_eq!(
            mount_boot_filesystem(&mut k, "/dev/mmcblk0p1", BOOT_PARTITION_MOUNT, BOOT_FSTYPE, false),
            Ok(Mounted::Adopted)
        );
    }

    /// **`EBUSY` with nothing mounted at the mount point is not convergence.**
    /// The kernel says `EBUSY` for this device mounted elsewhere with
    /// incompatible flags too, and treating that as "already mounted here"
    /// would read the bare directory on `/run` as though it were the card.
    #[test]
    fn busy_with_nothing_mounted_at_the_mount_point_is_refused() {
        let mut k = FakeKernel::new();
        k.mount_results = Script::always(Err(Errno::EBUSY));
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok(format!(
                "/dev/mmcblk0p1 {BOOT_PARTITION_MOUNT} vfat ro 0 0\n\
                 /dev/mmcblk0p1 /run/boot-partitionx vfat ro 0 0\n"
            ))),
        );
        assert_eq!(
            mount_boot_filesystem(&mut k, "/dev/mmcblk0p1", "/run/elsewhere", BOOT_FSTYPE, true),
            Err(Errno::EBUSY)
        );
        // An unreadable mount table is not evidence either.
        let mut blind = FakeKernel::new();
        blind.mount_results = Script::always(Err(Errno::EBUSY));
        assert_eq!(
            mount_boot_filesystem(&mut blind, "/dev/mmcblk0p1", "/run/x", BOOT_FSTYPE, true),
            Err(Errno::EBUSY)
        );
    }

    // -- the three outcomes the file itself can have -------------------------

    #[test]
    fn an_absent_file_is_not_a_failure_and_still_confirms_the_boot() {
        let mut k = FakeKernel::new();
        k.blobs_mut().insert(
            DT_BOOT_PARTITION.to_string(),
            Script::always(Ok(b"1".to_vec())),
        );
        // The provisioning path is simply not in the fake, so it reads ENOENT.
        let got = read_provisioning(&mut k);
        assert!(matches!(got, Provisioning::Absent { .. }), "{got:?}");
        assert!(got.confirms_boot());
        assert!(k.umount_calls.contains(&BOOT_PARTITION_MOUNT.to_string()));
    }

    #[test]
    fn an_unreadable_file_is_distinct_from_an_absent_one_and_blocks_confirmation() {
        let mut k = FakeKernel::new();
        with_boot_partition(&mut k, Script::always(Err(Errno::EIO)));
        let got = read_provisioning(&mut k);
        assert_eq!(got, Provisioning::Unreadable { errno: Errno::EIO });
        assert!(!got.confirms_boot());
    }

    #[test]
    fn a_malformed_document_is_refused_legibly_and_blocks_confirmation() {
        let mut k = FakeKernel::new();
        with_boot_partition(&mut k, Script::always(Ok("verison = 1\n".to_string())));
        let got = read_provisioning(&mut k);
        // The misspelled key is named, which is what makes it fixable.
        assert!(
            matches!(&got, Provisioning::Refused { detail } if detail.contains("verison")),
            "expected a refusal naming the key, got {got:?}"
        );
        assert!(!got.confirms_boot());
        assert!(
            k.lines.iter().any(|l| l.contains("REFUSED")),
            "the console must carry the refusal: {:?}",
            k.lines
        );
        assert!(k.lines.iter().any(|l| l.contains("NOT be confirmed")));
    }

    /// The zero-scrub consequence, as a test rather than as a comment. The
    /// provisioning file holds a wireless passphrase (`ADR-004` §6), and a
    /// console is a log.
    #[test]
    fn a_refusal_never_puts_the_documents_own_values_on_the_console() {
        let mut k = FakeKernel::new();
        with_boot_partition(
            &mut k,
            Script::always(Ok(format!("psk = \"{NOT_A_REAL_SECRET}\"\n"))),
        );
        let got = read_provisioning(&mut k);
        assert!(matches!(got, Provisioning::Refused { .. }), "{got:?}");
        let console = k.lines.join("\n");
        assert!(
            !console.contains(NOT_A_REAL_SECRET),
            "the console quoted the document's value: {console}"
        );
        assert!(
            console.contains("psk"),
            "but it must still name the key: {console}"
        );
    }

    #[test]
    fn a_valid_document_loads_and_reports_its_schema_version() {
        let mut k = FakeKernel::new();
        with_boot_partition(&mut k, Script::always(Ok("version = 1\n".to_string())));
        let got = read_provisioning(&mut k);
        assert_eq!(got, Provisioning::Loaded(Document::default()));
        assert!(got.confirms_boot());
        assert!(k.lines.iter().any(|l| l.contains("schema version 1")));
    }

    /// `ADR-004` §6 is the whole reason the provisioning file exists: a
    /// headless board with no wired link cannot be told anything after the
    /// fact. Since 2026-08-23 the thing it is told includes the `[network]`
    /// table, so the console has to show that the table arrived — otherwise the
    /// only evidence a board with no address gives is silence.
    ///
    /// **What this test does not prove.** ~~The document is read and reported
    /// and then dropped. `ADR-014` §2's `rivet-store` does not exist, so
    /// nothing writes these values anywhere `cmd/rivet-netd` can read them
    /// back from~~ — corrected 2026-08-23: [`crate::seed`] now writes the
    /// document to `/state/config` on a first boot (`ADR-017` §3.1.9), and this
    /// test still proves nothing about it, because this module ends at the
    /// reporting. The reader was the remaining gap — `cmd/rivet-netd` needed
    /// a `--document` path from whoever started it — and it closed the same
    /// day: `cmd/rivet-sysd`'s `netd` service definition points it at
    /// `ConfigStore::machine_path()`.
    #[test]
    fn a_provisioned_network_is_reported_by_count_rather_than_by_value() {
        let mut k = FakeKernel::new();
        with_boot_partition(
            &mut k,
            Script::always(Ok("version = 1\n\
                               [network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n\
                               [network.wlan0]\nmethod = \"dhcp\"\n"
                .to_string())),
        );
        let got = read_provisioning(&mut k);
        assert!(matches!(&got, Provisioning::Loaded(_)), "{got:?}");
        assert!(got.confirms_boot());
        let console = k.lines.join("\n");
        assert!(console.contains("2 interface(s)"), "{console}");
        // ...and by count only. The console is a log (`ADR-020` §4).
        assert!(!console.contains("192.168.1.50"), "{console}");
    }

    /// A `[network]` table that contradicts itself does not confirm the boot,
    /// for the reason `confirms_boot` states: the operator asked for something
    /// and the machine demonstrably did not do it.
    #[test]
    fn a_contradictory_network_table_is_refused_and_blocks_confirmation() {
        let mut k = FakeKernel::new();
        with_boot_partition(
            &mut k,
            Script::always(Ok(
                "version = 1\n[network.eth0]\nmethod = \"dhcp\"\naddress = \"10.0.0.2/24\"\n"
                    .to_string(),
            )),
        );
        let got = read_provisioning(&mut k);
        assert!(
            matches!(&got, Provisioning::Refused { detail } if detail.contains("network.eth0.address")),
            "expected a refusal naming the key, got {got:?}"
        );
        assert!(!got.confirms_boot());
    }

    #[test]
    fn the_partition_is_unmounted_whatever_the_outcome() {
        for file in [
            Script::always(Ok("version = 1\n".to_string())),
            Script::always(Ok("nonsense".to_string())),
            Script::always(Err(Errno::EIO)),
            Script::always(Err(Errno::ENOENT)),
        ] {
            let mut k = FakeKernel::new();
            with_boot_partition(&mut k, file);
            let _ = read_provisioning(&mut k);
            assert_eq!(k.umount_calls, vec![BOOT_PARTITION_MOUNT.to_string()]);
        }
    }

    #[test]
    fn a_failed_unmount_is_reported_and_survived() {
        let mut k = FakeKernel::new();
        with_boot_partition(&mut k, Script::always(Ok("version = 1\n".to_string())));
        k.umount_results = Script::always(Err(Errno::EBUSY));
        let got = read_provisioning(&mut k);
        assert!(matches!(got, Provisioning::Loaded(_)));
        assert!(k.lines.iter().any(|l| l.contains("could not unmount")));
    }
}
