//! Where the container and the boot port meet.
//!
//! `AGENTS.md` §4: *"Prioritise the tests that tie two modules together. The
//! individually-correct-but-jointly-wrong case is what silently ships."*
//!
//! Two halves of `ADR-029` live in crates that cannot see each other, on purpose:
//!
//! * **`rivet-core`** owns the container and takes the reserved-name list as a
//!   parameter, because `ADR-014` §3 forbids core → adapter.
//! * **`rivet-adapter-image`** owns `BootSlots::reserved_names`, because
//!   `ADR-029` §4 says *"this is the port's job, not the installer's"* — on a
//!   UEFI machine the equivalent object is an EFI variable and is not a file.
//!
//! Each half is individually correct and proved so by its own suite. What
//! neither suite can show is that the port's answer, handed to the container, is
//! the thing that refuses the payload `ADR-029` §4 was written about. This file
//! does, over real Ed25519 signatures.
//!
//! It also drives the sequence `ADR-029` §9.2 steps 4–7 reduce to — *running
//! slot, other slot, that slot's payload target, the class the member names* —
//! and asserts the thing that would destroy a machine if it were ever wrong:
//! **the system member's target is never the running slot's `/system`.**

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use rivet_adapter_image::{
    BootMedium, BootSlots, DT_BOOT_PARTITION, Errno, Mounted, PayloadTarget, PiFirmware, SlotId,
};
use rivet_core::package::error::PackageError;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::MaySign;
use rivet_core::package::port::{Digest, Sign, TrustSet};
use rivet_core::package::read::{Package, open};
use rivet_core::package::update::{Payload, SlotVariance, TargetClass};
use rivet_core::package::write::{Builder, NewMember};
use rivet_core::state::Slot;
use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier, SEED_BYTES};

/// A board that reports it booted partition 2 — slot B — and refuses everything
/// else.
///
/// `rivet-adapter-image`'s own `testio` fake is `cfg(test)`, so it is not
/// reachable from another crate. A handful of methods is cheap, and writing them here
/// keeps this file honest about what it is driving: `PiFirmware`'s real
/// `running_slot`, over the real device-tree property name.
struct BoardRunningSlotB;

impl BootMedium for BoardRunningSlotB {
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        if path == DT_BOOT_PARTITION {
            // The firmware publishes a NUL-terminated string here.
            Ok(b"2\0".to_vec())
        } else {
            Err(Errno::ENOENT)
        }
    }
    fn read_file(&mut self, _path: &str) -> Result<String, Errno> {
        Err(Errno::ENOENT)
    }
    fn write_file(&mut self, _path: &str, _contents: &[u8]) -> Result<(), Errno> {
        // Nothing in this file may write. If a change ever makes it try, the
        // refusal is what says so — `AGENTS.md` §3: stubs refuse.
        Err(Errno::EROFS)
    }
    fn rename(&mut self, _from: &str, _to: &str) -> Result<(), Errno> {
        Err(Errno::EROFS)
    }
    fn sync(&mut self) {}
    fn mount_boot_filesystem(
        &mut self,
        _device: &str,
        _at: &str,
        _fstype: &str,
        _writable: bool,
    ) -> Result<Mounted, Errno> {
        Err(Errno::ENODEV)
    }
    fn unmount(&mut self, _at: &str) {}
    fn reclaimed_orphan(&mut self, _at: &str) {}
    fn lock(&mut self, _path: &str) -> Result<(), Errno> {
        Err(Errno::EAGAIN)
    }
    fn unlock(&mut self, _path: &str) {}
}

fn signer() -> Ed25519Signer {
    let mut digest = Blake3::new();
    let seed: [u8; SEED_BYTES] = digest.of(b"rivet port-agreement suite seed, not a key");
    Ed25519Signer::from_seed(&seed)
}

fn trust() -> TrustSet {
    let mut digest = Blake3::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        signer().public_key(),
        "test-update",
        MaySign::these(&[Kind::update()]),
    );
    trust
}

fn read(bytes: &[u8]) -> Result<Package, PackageError> {
    let mut digest = Blake3::new();
    open(
        bytes,
        bytes.len() as u64,
        &mut digest,
        &Ed25519Verifier::new(),
        &trust(),
    )
}

/// Build a payload from `(name, class, variant, bytes)`, without consulting any
/// reserved list — which is how a builder that had not asked the port would.
fn build(members: &[(&str, u8, u8, Vec<u8>)]) -> Vec<u8> {
    let mut digest = Blake3::new();
    let mut signer = signer();
    let mut builder =
        Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update()).unwrap();
    for (name, class, variant, bytes) in members {
        builder
            .add(NewMember::stored(name, *class, *variant, bytes, &mut digest).unwrap())
            .unwrap();
    }
    let slices: Vec<&[u8]> = members.iter().map(|(_, _, _, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .unwrap()
}

fn good_members() -> Vec<(&'static str, u8, u8, Vec<u8>)> {
    vec![
        ("system.img", 0, 0, vec![0x21u8; 2048]),
        ("kernel8.img", 1, 0, vec![0x11u8; 512]),
        (
            "cmdline.txt",
            1,
            SlotVariance::SlotA.as_byte(),
            b"root=PARTLABEL=system-a\n".to_vec(),
        ),
        (
            "cmdline.txt",
            1,
            SlotVariance::SlotB.as_byte(),
            b"root=PARTLABEL=system-b\n".to_vec(),
        ),
    ]
}

/// The list the port answers with, for the slot an update on this board would
/// write.
fn reserved_for_inactive_boot_half() -> &'static [&'static str] {
    let mut medium = BoardRunningSlotB;
    let running = PiFirmware
        .running_slot(&mut medium)
        .expect("the fake reports p2");
    let target = PiFirmware.other_slot(running).expect("a known slot");
    let payload = PiFirmware.payload(target).expect("a known slot");
    PiFirmware.reserved_names(&payload.kernel)
}

// ---------------------------------------------------------------------------

/// `ADR-029` §4, end to end and across three crates: the port names the file,
/// the container refuses the payload, and the payload is correctly signed the
/// whole time.
#[test]
fn the_ports_reserved_list_is_what_refuses_the_payload_adr_029_4_is_about() {
    let reserved = reserved_for_inactive_boot_half();
    assert!(
        !reserved.is_empty(),
        "the port reserved nothing on a boot target"
    );

    for name in reserved {
        let mut members = good_members();
        members.push((name, 1, 0, b"[all]\nboot_partition=1\n".to_vec()));
        let bytes = build(&members);

        // It verifies. Nothing is wrong with its signature or its structure.
        let package = read(&bytes).expect("a correctly signed payload");

        // And it is refused, whole, on the port's answer alone.
        let outcome = Payload::open(&package, reserved).unwrap_err();
        assert!(
            matches!(outcome, PackageError::ReservedMemberName { name: ref n, .. } if n == name),
            "{name} was not refused: {outcome:?}"
        );

        // The same bytes, on a platform that reserves nothing, are accepted —
        // which is what makes this the *port's* refusal rather than the
        // container's.
        assert!(Payload::open(&package, &[]).is_ok());
    }
}

/// The refusal reaches the *builder* too, so a payload carrying a reserved name
/// cannot be produced by a tool that asked the port.
#[test]
fn a_builder_that_asked_the_port_cannot_produce_such_a_payload() {
    let reserved = reserved_for_inactive_boot_half();
    let mut digest = Blake3::new();
    for name in reserved {
        let mut builder = Builder::new("rivet", "0.2.0", "aarch64", Some("rpi4"), Kind::update())
            .unwrap()
            .reserving(reserved);
        let member = NewMember::stored(name, 1, 0, b"anything", &mut digest).unwrap();
        assert!(matches!(
            builder.add(member),
            Err(PackageError::ReservedMemberName { .. })
        ));
    }
}

/// **`ADR-029` §9.2 steps 4–7, reduced to the one thing that must never be
/// wrong.** A board running slot B, a payload whose system member is a whole
/// partition, and the device that member's target class resolves to.
///
/// If this ever names `p4` the update destroys the machine it is updating — and
/// it would do so with every other test in this repository still green, because
/// each half is individually right.
#[test]
fn the_system_member_lands_on_the_inactive_slot_and_never_on_the_running_one() {
    let bytes = build(&good_members());
    let package = read(&bytes).unwrap();
    let reserved = reserved_for_inactive_boot_half();
    let payload = Payload::open(&package, reserved).unwrap();

    let mut medium = BoardRunningSlotB;
    let running = PiFirmware.running_slot(&mut medium).unwrap();
    assert_eq!(running, SlotId(2));
    let target = PiFirmware.other_slot(running).unwrap();
    assert_eq!(target, SlotId(1));

    let targets = PiFirmware.payload(target).unwrap();
    let running_targets = PiFirmware.payload(running).unwrap();

    let class = TargetClass::from_byte(payload.system_member().target_class())
        .expect("the payload declares a class this kind defines");
    assert_eq!(class, TargetClass::WholePartition);

    let PayloadTarget::WholePartition { device } = targets.target_for(class) else {
        panic!("the system member's target must be a whole partition");
    };
    assert_eq!(device, "/dev/mmcblk0p3");

    let PayloadTarget::WholePartition {
        device: running_device,
    } = running_targets.target_for(class)
    else {
        panic!("the running slot's system target must be a whole partition");
    };
    assert_ne!(
        device, running_device,
        "the update would write the system partition this machine is running from"
    );
}

/// The boot half of the target slot is a directory on the *target's* FAT
/// partition, and the members that go into it are the ones the target slot's
/// variance selects.
#[test]
fn the_boot_half_goes_to_the_target_slots_partition_with_the_target_slots_command_line() {
    let bytes = build(&good_members());
    let package = read(&bytes).unwrap();
    let payload = Payload::open(&package, reserved_for_inactive_boot_half()).unwrap();

    let mut medium = BoardRunningSlotB;
    let running = PiFirmware.running_slot(&mut medium).unwrap();
    let target = PiFirmware.other_slot(running).unwrap();
    let targets = PiFirmware.payload(target).unwrap();

    let PayloadTarget::DirectoryOnPartition { device, fstype, .. } =
        targets.target_for(TargetClass::BootDirectory)
    else {
        panic!("the Pi's boot half is a directory on a partition");
    };
    assert_eq!(device, "/dev/mmcblk0p1");
    assert_eq!(*fstype, "vfat");

    // Slot A is the target, so slot A's command line is what goes there.
    let chosen: Vec<&str> = payload
        .boot_members_for(Slot::A)
        .map(|m| m.name())
        .collect();
    assert_eq!(chosen, vec!["kernel8.img", "cmdline.txt"]);
    let cmdline = payload
        .boot_members_for(Slot::A)
        .find(|m| m.name() == "cmdline.txt")
        .unwrap();
    let start = cmdline.offset() as usize;
    let text =
        String::from_utf8(bytes[start..start + cmdline.stored_len() as usize].to_vec()).unwrap();
    assert!(text.contains("system-a"), "{text:?}");
}

/// Nothing in this file mounted, wrote or renamed anything. The fake refuses
/// every one of those, so a change that made the read path touch the medium
/// would fail here rather than on a board.
#[test]
fn reading_a_payloads_targets_touches_nothing_writable() {
    let mut medium = BoardRunningSlotB;
    assert!(PiFirmware.running_slot(&mut medium).is_ok());
    assert_eq!(medium.write_file("/anything", b""), Err(Errno::EROFS));
    assert_eq!(
        medium.mount_boot_filesystem("/dev/mmcblk0p1", "/mnt", "vfat", true),
        Err(Errno::ENODEV)
    );
}
