//! `ADR-029` §9.2's install sequence, as a **decision** — steps 4, 5, 7 and 8,
//! with every byte of I/O left to the caller.
//!
//! # Why the plan is here and not in `rivet-core`
//!
//! `AGENTS.md` §3 puts pure domain logic in `rivet-core`, and the first
//! instinct is that "which member goes where, in what order" belongs there. It
//! cannot, and the reason is the same one `rivet_core::package::update`
//! already records against its own `TargetClass`: **the plan's vocabulary is
//! this crate's.** A step names a [`PayloadTarget`], a device node and a
//! [`SlotId`] — and `ADR-014` §3's table forbids core → adapter precisely so
//! that naming one of those from `rivet-core` fails to compile rather than
//! being noticed in review.
//!
//! What *is* in `rivet-core` is everything that can be said without them:
//! `Payload::open` refuses a payload whose members are incoherent or reserved,
//! `Payload::accepts` refuses the wrong product, architecture or platform, and
//! `Package::verify_member` is the only route to a `VerifiedMember`. This module
//! turns a payload that has passed all of that into an ordered list of writes
//! against one slot, and it performs none of them.
//!
//! # The order is a safety property, not a tidiness one
//!
//! `ADR-029` §9 F5 — the one failure the format does **not** make safe — is a
//! power cut while the boot half of slot A is being written, because that is the
//! unjournalled FAT filesystem holding the only record of which slot the
//! firmware boots. §9's second bound is *"order: system member first, boot half
//! last. The dangerous write is the shortest-lived."* [`InstallPlan`] holds the
//! two halves in separate fields for that reason: there is no list a caller
//! could iterate in the wrong order.
//!
//! **It does not close F5** — §9.3 settled on hardware that a Pi with an
//! unparseable `autoboot.txt` boots partition 1, which is the half-written slot
//! — and none of §9.3's three candidate mitigations is ruled. See
//! [`InstallPlan::targets_the_fallback_partition`], which is this crate's way of
//! saying so out loud rather than in a comment.
//!
//! # What it refuses, and why each refusal is not fussiness
//!
//! | Refusal | The failure it prevents |
//! | :-- | :-- |
//! | [`PlanRefusal::UnknownSlot`] | A slot outside `ADR-018` §4's table is a device node this layout does not describe, and the write would land somewhere nobody chose |
//! | [`PlanRefusal::SystemDoesNotFit`] | `ADR-029` F8. A 640 MiB member over a 256 MiB partition writes into the next partition, which is the other slot |
//! | [`PlanRefusal::UnexpectedTargetShape`] | A platform whose `SlotPayload` is not one whole partition plus one directory. This plan does not describe it, and pretending it does would write a filesystem image into a file |

use rivet_core::package::format::DIGEST_BYTES;
use rivet_core::package::read::Member;
use rivet_core::package::update::{BOOT_MEMBER_TEMP_SUFFIX, Payload};
use rivet_core::state::{AUTOBOOT_PARTITION, Slot};

use crate::slots::{PayloadTarget, SlotId, SlotPayload, known_slot};

/// Why a payload could not be turned into a plan for this slot.
///
/// Typed rather than a `String`, for [`crate::slots::ArmRefusal`]'s reason: an
/// operator — or the agent standing in for one (`ADR-006` §4.1) — has to be able
/// to branch on *what kind* of failure it was without reading English. Here the
/// distinction that matters is **whether a different payload would help**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanRefusal {
    /// The firmware named a boot partition `ADR-018` §4's layout does not
    /// contain. Nothing about the payload is wrong; the card is.
    UnknownSlot(String),
    /// `ADR-029` F8 — the system member is larger than the partition it would
    /// be written to. **A different payload would help.**
    SystemDoesNotFit {
        name: String,
        member_bytes: u64,
        partition_bytes: u64,
        device: String,
    },
    /// The platform's [`SlotPayload`] is not the shape this plan describes.
    /// Neither a different payload nor a different card would help; the code
    /// would have to grow.
    UnexpectedTargetShape(String),
}

impl core::fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownSlot(why) => f.write_str(why),
            Self::SystemDoesNotFit {
                name,
                member_bytes,
                partition_bytes,
                device,
            } => write!(
                f,
                "the payload's `{name}` is {member_bytes} bytes and {device} is \
                 {partition_bytes}; refusing before anything is written, because a write \
                 that ran off the end of this partition would land in the next one — \
                 which on ADR-018 §4's layout is the other slot"
            ),
            Self::UnexpectedTargetShape(why) => write!(
                f,
                "this platform's slot payload is not one whole partition plus one boot \
                 directory ({why}); ADR-029 §9.2's install sequence does not describe it"
            ),
        }
    }
}

/// The `/system` write. `ADR-029` §9.2 step 7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemWrite {
    /// The member's index in the package, which is what
    /// `Package::member_stream` takes.
    pub member_index: usize,
    pub name: String,
    /// The block device the member's stored bytes replace, whole.
    pub device: String,
    pub bytes: u64,
    /// Over the **stored** bytes — the same digest `ADR-029` §9.2 step 9 reads
    /// the written partition back and compares against.
    pub digest: [u8; DIGEST_BYTES],
}

/// One file of the boot half. `ADR-029` §9.2 step 8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootWrite {
    pub member_index: usize,
    /// A flat name — `rivet_core::package::format::read_name` has already
    /// refused a path separator, a `.` and a `..`, so joining it onto a
    /// directory cannot traverse.
    pub name: String,
    pub bytes: u64,
    pub digest: [u8; DIGEST_BYTES],
}

/// The boot half's target, and the files that go into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootHalf {
    pub device: String,
    pub fstype: &'static str,
    /// The directory within that filesystem. `/` on the Pi; the boot-slot
    /// port expects `/EFI/rivet/a/` on a UEFI machine.
    pub path: String,
    pub files: Vec<BootWrite>,
}

impl BootHalf {
    /// Where `file` lands, given the mount point the caller used.
    ///
    /// The join is here rather than at the caller because [`Self::path`] may or
    /// may not be `/`, and a second implementation of that arithmetic is a
    /// second answer (`ADR-011`).
    #[must_use]
    pub fn path_of(&self, mount: &str, name: &str) -> String {
        let mount = mount.trim_end_matches('/');
        let inner = self.path.trim_matches('/');
        if inner.is_empty() {
            format!("{mount}/{name}")
        } else {
            format!("{mount}/{inner}/{name}")
        }
    }

    /// The name a write-sync-rename uses on the way in.
    ///
    /// The same pattern — and the same reason — as
    /// [`crate::slots::AUTOBOOT_TEMP`]: on a FAT filesystem with no journal, a
    /// half-written file under the firmware's own name is a board that does not
    /// boot.
    ///
    /// Built from `rivet_core::package::update::BOOT_MEMBER_TEMP_SUFFIX`
    /// rather than its own `".new"`, so this and the container's own refusal
    /// of a payload that collides with it cannot drift apart (`ADR-011`).
    #[must_use]
    pub fn temp_name(name: &str) -> String {
        format!("{name}{BOOT_MEMBER_TEMP_SUFFIX}")
    }
}

/// What to do with one boot-half file, once the target's digest is known.
///
/// `ADR-029` §6: *"the installer compares each boot-directory member's digest
/// against what is on the target and skips the identical ones"*, and §9 makes
/// that a **safety** property rather than an optimisation — the usual update
/// never opens the FAT filesystem for writing at all, which is the main defence
/// against F5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootFileAction {
    /// The target already holds exactly these bytes.
    Skip,
    /// It does not, or there is nothing there to compare against.
    Write,
}

/// [`BootFileAction`], from the two digests.
///
/// `found` is `None` when the file is not on the target at all — which is a
/// write, not a skip. A reader that treated "I could not read it" as "it
/// matches" would silently leave the old kernel in place under a report that a
/// new one had been installed, and `AGENTS.md` §5's *"a wrong guess does not
/// fail, it succeeds"* is exactly that failure.
#[must_use]
pub fn action_for(want: &[u8; DIGEST_BYTES], found: Option<&[u8; DIGEST_BYTES]>) -> BootFileAction {
    match found {
        Some(have) if have == want => BootFileAction::Skip,
        _ => BootFileAction::Write,
    }
}

/// One payload, one slot, as an ordered set of writes.
///
/// Holding one means the slot is in `ADR-018` §4's layout, the platform's
/// targets are the two kinds this sequence describes, and the system member
/// fits. It does **not** mean any member's content has been verified — that is
/// `Package::verify_member`, and `ADR-029` §9.2 step 6 requires **every** member
/// to pass it before step 7 writes the first byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    slot: SlotId,
    system: SystemWrite,
    boot: BootHalf,
}

impl InstallPlan {
    /// `ADR-029` §9.2 steps 4, 5 and 8's member selection.
    ///
    /// `partition_bytes` is the size of the system partition, read by the caller
    /// from the block device — the one number here that is I/O, taken as an
    /// argument so this function is not.
    pub fn for_slot(
        payload: &Payload<'_>,
        slot: SlotId,
        target: &SlotPayload,
        partition_bytes: u64,
    ) -> Result<Self, PlanRefusal> {
        let named = known_slot(slot).map_err(PlanRefusal::UnknownSlot)?;

        let PayloadTarget::WholePartition { device } = &target.system else {
            return Err(PlanRefusal::UnexpectedTargetShape(
                "its `/system` half is not a whole partition".to_string(),
            ));
        };
        let PayloadTarget::DirectoryOnPartition {
            device: boot_device,
            fstype,
            path,
        } = &target.kernel
        else {
            return Err(PlanRefusal::UnexpectedTargetShape(
                "its boot half is not a directory on a partition".to_string(),
            ));
        };

        let member = payload.system_member();
        if member.stored_len() > partition_bytes {
            return Err(PlanRefusal::SystemDoesNotFit {
                name: member.name().to_string(),
                member_bytes: member.stored_len(),
                partition_bytes,
                device: device.clone(),
            });
        }

        Ok(Self {
            slot,
            system: SystemWrite {
                member_index: member.index(),
                name: member.name().to_string(),
                device: device.clone(),
                bytes: member.stored_len(),
                digest: *member.digest(),
            },
            boot: BootHalf {
                device: boot_device.clone(),
                fstype,
                path: path.clone(),
                files: payload.boot_members_for(named).map(write_for).collect(),
            },
        })
    }

    #[must_use]
    pub fn slot(&self) -> SlotId {
        self.slot
    }

    /// `ADR-029` §9.2 step 7. **Performed first**, and the field is separate
    /// from [`Self::boot`] so that no caller can iterate the two in the other
    /// order — see this module's documentation.
    #[must_use]
    pub fn system(&self) -> &SystemWrite {
        &self.system
    }

    /// `ADR-029` §9.2 step 8. **Performed last.**
    #[must_use]
    pub fn boot(&self) -> &BootHalf {
        &self.boot
    }

    /// Every member index this plan touches, in the order the plan writes them.
    ///
    /// `ADR-029` §9.2 step 6 verifies **every member of the payload** before
    /// step 7, not merely the ones this plan writes — so this is not the list to
    /// verify. It is what a report names, and what a test asserts an order over.
    #[must_use]
    pub fn member_indices(&self) -> Vec<usize> {
        let mut all = vec![self.system.member_index];
        all.extend(self.boot.files.iter().map(|f| f.member_index));
        all
    }

    /// **`ADR-029` §9.3's F5, named at the point it applies.**
    ///
    /// True when this plan's boot half is the partition the Pi firmware falls
    /// back to when `autoboot.txt` will not parse — settled on hardware
    /// 2026-08-23, and the answer was the unfavourable one: *"the firmware obeys
    /// `autoboot.txt` when it can, and falls back to partition 1 when it
    /// cannot."*
    ///
    /// So an update targeting that partition writes `kernel8.img` beside the
    /// only record of which slot the machine boots, on an unjournalled FAT
    /// filesystem, and a torn write there does not leave the machine on the slot
    /// it was running — it boots the half-written one.
    ///
    /// **None of §9.3's three mitigations is ruled**, so this returns a fact
    /// rather than refusing: refusing would be taking the second of them
    /// ("refuse the ordering") by implementation, which `AGENTS.md` §1 reserves
    /// to the maintainer. What a caller must do with it is *say so* — see
    /// `cmd/rivet-deploy`, which emits it as a `warn` record naming the
    /// section.
    ///
    /// It is computed from `rivet_core::state::AUTOBOOT_PARTITION` rather than
    /// from the literal `1`, so a layout change moves both together.
    #[must_use]
    pub fn targets_the_fallback_partition(&self) -> bool {
        self.slot.0 == AUTOBOOT_PARTITION
    }
}

fn write_for(member: &Member) -> BootWrite {
    BootWrite {
        member_index: member.index(),
        name: member.name().to_string(),
        bytes: member.stored_len(),
        digest: *member.digest(),
    }
}

/// The slot an update writes, given the one the firmware says it booted.
///
/// A free function rather than a method, because it is the pair
/// `running_slot` → `other_slot` that `ADR-029` §9.2 step 4 names and because
/// `BootSlots::other_slot` is the only thing entitled to answer it — *"no code
/// above this port may do arithmetic on the number"* ([`SlotId`]).
pub fn target_slot(
    slots: &dyn crate::slots::BootSlots,
    m: &mut dyn crate::port::BootMedium,
) -> Result<SlotId, String> {
    let running = slots.running_slot(m)?;
    slots.other_slot(running)
}

/// Which of `ADR-018` §4's two slots a [`SlotId`] is, or a refusal naming why
/// not. Re-exported here because a caller planning an install needs the name for
/// a report and must not derive it by arithmetic.
pub fn slot_named(slot: SlotId) -> Result<Slot, String> {
    known_slot(slot)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::package::kind::Kind;
    use rivet_core::package::port::{
        Digest, DigestAlgorithm, PublicKey, Sign, Signature, SignatureAlgorithm, TrustSet, Verify,
    };
    use rivet_core::package::read::{Package, open};
    use rivet_core::package::update::{SlotVariance, TargetClass};
    use rivet_core::package::write::{Builder, NewMember};

    use crate::slots::{BootSlots as _, PiFirmware};

    // ---------------------------------------------------------------------
    // Fakes.
    //
    // `rivet_core::package::testing` holds exactly these three and is
    // `#[cfg(test)]`, so it is unreachable from another crate — a `cfg(test)`
    // module is compiled only for its own crate's test binary. Taking
    // `rivet-crypto` as a dev-dependency instead would close a cycle, because
    // `rivet-crypto` already dev-depends on this crate for
    // `tests/port_agreement.rs`, and cargo refuses one.
    //
    // They are small on purpose and they prove nothing about cryptography. What
    // they buy is that the thing this module owns — which member goes where, in
    // what order — is tested over a package that was really written and really
    // read back, rather than over a hand-built `Payload`.
    // ---------------------------------------------------------------------

    /// A mixing function, not a hash. It must depend on **every** input byte and
    /// on their order, or the slot-variance test below would pass for the wrong
    /// reason.
    struct ToyDigest {
        state: [u8; DIGEST_BYTES],
        n: u64,
    }

    impl ToyDigest {
        fn new() -> Self {
            Self {
                state: [0; DIGEST_BYTES],
                n: 0,
            }
        }
    }

    impl Digest for ToyDigest {
        fn algorithm(&self) -> DigestAlgorithm {
            DigestAlgorithm::BLAKE3
        }
        fn start(&mut self) {
            self.state = [0; DIGEST_BYTES];
            self.n = 0;
        }
        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                let at = (self.n % DIGEST_BYTES as u64) as usize;
                self.state[at] = self.state[at]
                    .rotate_left(3)
                    .wrapping_add(*byte)
                    .wrapping_add(self.n as u8);
                self.n = self.n.wrapping_add(1);
            }
        }
        fn finish(&mut self) -> [u8; DIGEST_BYTES] {
            let mut out = self.state;
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = byte.wrapping_add((self.n as u8).wrapping_mul(i as u8 | 1));
            }
            out
        }
    }

    /// Signature = the digest of the message, keyed by the public key's first
    /// byte and repeated to fill the field. Enough for a writer and a reader to
    /// agree; nothing more is claimed.
    fn toy_signature(key: u8, message: &[u8]) -> Signature {
        let mut digest = ToyDigest::new();
        digest.start();
        digest.write(&[key]);
        digest.write(message);
        let half = digest.finish();
        let mut out = [0u8; 64];
        out[..DIGEST_BYTES].copy_from_slice(&half);
        out[DIGEST_BYTES..].copy_from_slice(&half);
        Signature(out)
    }

    struct ToySigner(u8);

    impl Sign for ToySigner {
        fn algorithm(&self) -> SignatureAlgorithm {
            SignatureAlgorithm::ED25519
        }
        fn public_key(&self) -> PublicKey {
            PublicKey([self.0; 32])
        }
        fn sign(&mut self, message: &[u8]) -> Signature {
            toy_signature(self.0, message)
        }
    }

    struct ToyVerifier;

    impl Verify for ToyVerifier {
        fn algorithm(&self) -> SignatureAlgorithm {
            SignatureAlgorithm::ED25519
        }
        fn verify(&self, key: &PublicKey, message: &[u8], signature: &Signature) -> bool {
            toy_signature(key.0[0], message).0 == signature.0
        }
    }

    const KEY: u8 = 0x5a;
    const SYSTEM_BYTES: u64 = 4096;

    fn boot_target() -> PayloadTarget {
        PayloadTarget::DirectoryOnPartition {
            device: "/dev/mmcblk0p1".to_string(),
            fstype: "vfat",
            path: "/".to_string(),
        }
    }

    /// A payload with one system member, four invariant boot files and
    /// `cmdline.txt` for both slots — `ADR-029` §3's shape.
    fn payload_bytes() -> Vec<u8> {
        let mut digest = ToyDigest::new();
        let mut builder = Builder::new("rivet", "0.0.2", "aarch64", Some("rpi4"), Kind::update())
            .expect("the header fields fit")
            .reserving(PiFirmware.reserved_names(&boot_target()));

        let mut bytes: Vec<Vec<u8>> = Vec::new();
        let add = |builder: &mut Builder,
                   bytes: &mut Vec<Vec<u8>>,
                   digest: &mut ToyDigest,
                   name: &str,
                   class: TargetClass,
                   variance: SlotVariance,
                   content: Vec<u8>| {
            builder
                .add(
                    NewMember::stored(name, class.as_byte(), variance.as_byte(), &content, digest)
                        .expect("a well-formed member"),
                )
                .expect("a member the builder accepts");
            bytes.push(content);
        };

        add(
            &mut builder,
            &mut bytes,
            &mut digest,
            "system.img",
            TargetClass::WholePartition,
            SlotVariance::Invariant,
            vec![0xa5u8; SYSTEM_BYTES as usize],
        );
        for name in ["kernel8.img", "start4.elf", "config.txt", "tryboot.txt"] {
            add(
                &mut builder,
                &mut bytes,
                &mut digest,
                name,
                TargetClass::BootDirectory,
                SlotVariance::Invariant,
                name.as_bytes().to_vec(),
            );
        }
        for (variance, root) in [
            (SlotVariance::SlotA, "root=PARTLABEL=system-a"),
            (SlotVariance::SlotB, "root=PARTLABEL=system-b"),
        ] {
            add(
                &mut builder,
                &mut bytes,
                &mut digest,
                "cmdline.txt",
                TargetClass::BootDirectory,
                variance,
                root.as_bytes().to_vec(),
            );
        }

        let borrowed: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
        let mut signer = ToySigner(KEY);
        builder
            .seal_in_memory(&mut digest, &mut signer, &borrowed)
            .expect("a sealed payload")
    }

    fn read(bytes: &[u8]) -> Package {
        let mut digest = ToyDigest::new();
        let mut trust = TrustSet::new();
        trust.trust(
            &mut digest,
            ToySigner(KEY).public_key(),
            "test-update",
            rivet_core::package::port::MaySign::these(&[rivet_core::package::Kind::update()]),
        );
        open(bytes, bytes.len() as u64, &mut digest, &ToyVerifier, &trust)
            .expect("the payload verifies")
    }

    fn plan_for(slot: SlotId, partition_bytes: u64) -> Result<InstallPlan, PlanRefusal> {
        let bytes = payload_bytes();
        let package = read(&bytes);
        // A slot outside the layout has no `SlotPayload` at all, so the case is
        // driven with a stand-in target rather than being unreachable — the
        // refusal under test is the plan's, not the firmware's.
        let target = PiFirmware.payload(slot).unwrap_or(SlotPayload {
            kernel: boot_target(),
            system: PayloadTarget::WholePartition {
                device: "/dev/mmcblk0p9".to_string(),
            },
        });
        let payload = Payload::open(&package, PiFirmware.reserved_names(&target.kernel))
            .expect("a well-formed update payload");
        InstallPlan::for_slot(&payload, slot, &target, partition_bytes)
    }

    #[test]
    fn a_plan_writes_the_system_member_to_the_targets_own_partition() {
        let plan = plan_for(SlotId(2), 1 << 30).expect("a plan");
        assert_eq!(plan.system().device, "/dev/mmcblk0p4");
        assert_eq!(plan.system().name, "system.img");
        assert_eq!(plan.system().bytes, SYSTEM_BYTES);
        assert_eq!(plan.boot().device, "/dev/mmcblk0p2");
        assert_eq!(plan.boot().fstype, "vfat");
        assert_eq!(plan.slot(), SlotId(2));
    }

    /// The whole of `ADR-029` §3's slot variance, at the place it decides
    /// something: slot B's boot half carries B's `cmdline.txt` and not A's.
    #[test]
    fn the_boot_half_carries_this_slots_cmdline_and_not_the_others() {
        let b = plan_for(SlotId(2), 1 << 30).expect("a plan");
        let names: Vec<&str> = b.boot().files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "kernel8.img",
                "start4.elf",
                "config.txt",
                "tryboot.txt",
                "cmdline.txt"
            ]
        );
        let a = plan_for(SlotId(1), 1 << 30).expect("a plan");
        let cmdline = |p: &InstallPlan| {
            p.boot()
                .files
                .iter()
                .find(|f| f.name == "cmdline.txt")
                .map(|f| f.digest)
                .expect("the slot-variant member")
        };
        assert_ne!(
            cmdline(&a),
            cmdline(&b),
            "both slots were given the same cmdline.txt, which is the one file \
             ADR-029 §3 measured as differing between them"
        );
    }

    /// `ADR-029` F8. The number is the partition's, read from the device by the
    /// caller; a member that does not fit is refused **before** anything is
    /// opened for writing.
    #[test]
    fn a_system_member_larger_than_the_partition_is_refused_before_any_write() {
        let refusal = plan_for(SlotId(2), SYSTEM_BYTES - 1).expect_err("refused");
        assert!(
            matches!(refusal, PlanRefusal::SystemDoesNotFit { .. }),
            "{refusal:?}"
        );
        assert!(refusal.to_string().contains("/dev/mmcblk0p4"), "{refusal}");
        // Exactly the partition's size is not too large.
        assert!(plan_for(SlotId(2), SYSTEM_BYTES).is_ok());
    }

    #[test]
    fn a_slot_outside_the_layout_is_refused() {
        let refusal = plan_for(SlotId(9), 1 << 30).expect_err("refused");
        assert!(
            matches!(refusal, PlanRefusal::UnknownSlot(_)),
            "{refusal:?}"
        );
        assert!(refusal.to_string().contains("partition 9"), "{refusal}");
    }

    /// The order is `ADR-029` §9's second bound on F5 and there must be no way
    /// to get it wrong: the system member is first in every listing this type
    /// produces.
    #[test]
    fn the_system_member_comes_before_every_boot_file() {
        let plan = plan_for(SlotId(2), 1 << 30).expect("a plan");
        let order = plan.member_indices();
        assert_eq!(order.first(), Some(&plan.system().member_index));
        assert_eq!(order.len(), 1 + plan.boot().files.len());
    }

    /// `ADR-029` §9.3, and the reason it is a fact rather than a refusal: the
    /// mitigation is unruled, so the plan reports and does not decide.
    #[test]
    fn a_plan_targeting_slot_a_says_it_is_the_partition_the_firmware_falls_back_to() {
        assert!(
            plan_for(SlotId(1), 1 << 30)
                .expect("a plan")
                .targets_the_fallback_partition()
        );
        assert!(
            !plan_for(SlotId(2), 1 << 30)
                .expect("a plan")
                .targets_the_fallback_partition()
        );
    }

    #[test]
    fn an_identical_file_is_skipped_and_an_absent_one_is_written() {
        let want = [7u8; DIGEST_BYTES];
        let other = [8u8; DIGEST_BYTES];
        assert_eq!(action_for(&want, Some(&want)), BootFileAction::Skip);
        assert_eq!(action_for(&want, Some(&other)), BootFileAction::Write);
        assert_eq!(action_for(&want, None), BootFileAction::Write);
    }

    #[test]
    fn a_boot_file_path_joins_the_mount_the_directory_and_the_name() {
        let root = BootHalf {
            device: "/dev/mmcblk0p2".to_string(),
            fstype: "vfat",
            path: "/".to_string(),
            files: Vec::new(),
        };
        assert_eq!(
            root.path_of("/run/update-boot/", "kernel8.img"),
            "/run/update-boot/kernel8.img"
        );
        let nested = BootHalf {
            path: "/EFI/rivet/a/".to_string(),
            ..root
        };
        assert_eq!(
            nested.path_of("/mnt", "kernel8.img"),
            "/mnt/EFI/rivet/a/kernel8.img"
        );
        assert_eq!(BootHalf::temp_name("kernel8.img"), "kernel8.img.new");
    }

    /// A payload whose halves are the other way round is not this sequence's,
    /// and pretending otherwise would write a filesystem image into a file.
    #[test]
    fn a_platform_whose_targets_are_not_this_shape_is_refused_rather_than_guessed_at() {
        let bytes = payload_bytes();
        let package = read(&bytes);
        let real = PiFirmware.payload(SlotId(2)).expect("a slot");
        let payload = Payload::open(&package, PiFirmware.reserved_names(&real.kernel))
            .expect("a well-formed payload");
        let swapped = SlotPayload {
            kernel: real.system.clone(),
            system: real.kernel.clone(),
        };
        let refusal =
            InstallPlan::for_slot(&payload, SlotId(2), &swapped, 1 << 30).expect_err("refused");
        assert!(
            matches!(refusal, PlanRefusal::UnexpectedTargetShape(_)),
            "{refusal:?}"
        );
    }

    /// `ADR-029` §9.2 step 4, driven through the port rather than by arithmetic
    /// on the slot number.
    #[test]
    fn the_target_slot_is_the_one_the_machine_is_not_running() {
        let mut medium = crate::testio::FakeMedium::new();
        medium.properties_mut().insert(
            crate::firmware::DT_BOOT_PARTITION.to_string(),
            crate::testio::Script::always(Ok(vec![0, 0, 0, 2])),
        );
        assert_eq!(target_slot(&PiFirmware, &mut medium), Ok(SlotId(1)));
        assert_eq!(slot_named(SlotId(1)), Ok(Slot::A));
    }
}
