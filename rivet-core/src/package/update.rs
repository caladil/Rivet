//! The `update` kind: what a payload's metadata *means*.
//!
//! `ADR-029` §11 rules one container for three artifact kinds, differing in
//! *"the **interpretation** of the metadata, not its shape"*. This module is one
//! such interpretation, and it is the only one that exists — the application and
//! extension cases are `ADR-027` §9 items 2–4 and are the maintainer's.
//!
//! **Nothing in [`crate::package::read`] or [`crate::package::write`] mentions a
//! slot, a partition or a boot directory.** That separation is the whole of §11
//! made structural: a second kind is a second module here, and the verifier is
//! untouched.
//!
//! # What this module refuses, and why each refusal is not fussiness
//!
//! | Refusal | The failure it prevents |
//! | :-- | :-- |
//! | [`PackageError::NothingToInstall`] | An install that changes nothing and reports success — the direction `ADR-018` §5.1's slot-B commit bug pointed in |
//! | [`PackageError::SystemMemberCount`] | Zero leaves the slot's `/system` untouched under a report that it was replaced; two leaves it ambiguous which one the slot becomes |
//! | [`PackageError::SlotVarianceIncomplete`] | A payload carrying `cmdline.txt` for A only would, installed to B, leave B's kernel command line at whatever was there — `ADR-029` §6.2's *"this artifact plus whatever happened to be there"* |
//! | [`PackageError::ReservedMemberName`] | `ADR-029` §4 — the whole payload, not the member |
//! | [`PackageError::SystemMemberIsSlotVariant`] | A claim the measurement contradicts: the two `/system` filesystems are byte-identical |
//! | [`PackageError::BootMemberNameCollides`] | Two members this container sees as distinct become one file on a boot filesystem that compares names case-insensitively |
//! | [`PackageError::BootMemberNameCollidesWithTemporary`] | A member's name is another member's install-time staging name, so installing the second overwrites the first before it is ever read |

use crate::package::error::PackageError;
use crate::package::kind::Kind;
use crate::package::read::{Member, Package};
use crate::state::Slot;

/// `ADR-027` §3's `<product>` token for a system update payload.
///
/// **It is here because this is a library and every other holder of it is a
/// binary.** `cmd/rivet-deploy`'s own `PRODUCT` recorded the problem in full:
/// the token was spelled separately in `cmd/rivet-image`, in this crate's
/// tests, in `rivet-crypto`'s loopback bench and in that binary, and *"the one
/// place it should live is `rivet_core::package`, which is a library and can
/// be the authority; this crate is a binary and could never have been"*
/// (`ADR-011`; the same standing debt, since 2026-08-23, tracks this shape
/// for four other path constants).
///
/// ~~**Adding it here does not by itself remove a spelling.** … leaving them
/// un-migrated would make this a *fifth* copy rather than the authority.~~
///
/// **THE MIGRATION IS DONE — 2026-08-24.** `cmd/rivet-deploy` re-exports this
/// constant instead of declaring one, and `rivet-crypto`'s `container` and
/// `loopback` tests build their payloads from it. There is now exactly one
/// spelling of the product token in the workspace, which is what `ADR-011`
/// asks for. `cmd/rivet-image`'s `"rivet"` is deliberately untouched: it is
/// a **binary name**, not this token, and merging the two would be the same
/// error in the opposite direction.
pub const PRODUCT: &str = "rivet";

/// The member name a builder gives the `/system` filesystem.
///
/// **Nothing reads it.** `Payload::system_member` finds the member by its
/// [`TargetClass`], never by name, and `rivet_adapter_image::install` copies
/// whatever name it was given into the report — so this is a convention for the
/// benefit of a person running `cargo rivet inspect`, not an interface. It is
/// a constant so that the three places that write a payload agree on the label,
/// which is `ADR-027` §5's *"the name is for humans; the manifest is the
/// authority"* one level down.
pub const SYSTEM_MEMBER_NAME: &str = "system.img";

/// The suffix a boot-directory member's install-time staging file carries
/// while its write is in flight, on a target that stages by write-sync-rename
/// — `rivet_adapter_image::BootHalf::temp_name`.
///
/// **Defined here, once, rather than in the adapter that uses it** (`ADR-011`
/// — the same reasoning [`PRODUCT`]'s doc comment records against a second
/// spelling): the value has to agree with [`check_boot_member_names`] below,
/// which refuses a payload where one member's name is another's staging name
/// under this suffix.
pub const BOOT_MEMBER_TEMP_SUFFIX: &str = ".new";

/// Byte 64 of a member record, for an `update`.
///
/// `ADR-029` §3: *"**`target_class` is `PayloadTarget`** — deliberately the same
/// two cases in the same order."*
///
/// **It cannot literally *be* `PayloadTarget`**, which lives in
/// `rivet-adapter-image`, because `ADR-014` §3's table forbids core → adapter
/// and that edge is what makes every other forbidden edge fail to compile. So it
/// is the same two cases in the same order, and the drift guard lives in
/// `rivet-adapter-image`, which can see both — the same shape as the
/// `autoboot.txt` parser's, and for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetClass {
    /// `PayloadTarget::WholePartition` — the `/system` filesystem, replaced
    /// whole because `ADR-018` §3 makes it read-only by format and there is
    /// nothing on it to preserve.
    WholePartition = 0,
    /// `PayloadTarget::DirectoryOnPartition` — the boot half. Files on a FAT
    /// filesystem the firmware also reads `config.txt`, `cmdline.txt` and the
    /// device trees from (`ADR-002` §4).
    BootDirectory = 1,
}

impl TargetClass {
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::WholePartition),
            1 => Some(Self::BootDirectory),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Byte 65 of a member record, for an `update` — `ADR-029` §3's
/// `slot_variance`.
///
/// > **`slot_variance` exists because exactly one boot file differs between
/// > slots**, and that was measured rather than assumed: extracting `p1` and
/// > `p2` from a built image and diffing them, **`cmdline.txt` is the only file
/// > that differs** (`root=PARTLABEL=system-a` versus `-b`).
///
/// So a payload carries `cmdline.txt` twice, 136 bytes of duplication, rather
/// than generating it on the device — which would stand a second authority over
/// the kernel command line beside `cmd/rivet-image`'s (`ADR-011`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotVariance {
    /// Identical whichever slot it is installed into. Everything but
    /// `cmdline.txt`.
    Invariant = 0,
    /// Install only when the target is slot A.
    SlotA = 1,
    /// Install only when the target is slot B.
    SlotB = 2,
}

impl SlotVariance {
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Invariant),
            1 => Some(Self::SlotA),
            2 => Some(Self::SlotB),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    /// Which slot this is for, or `None` for [`Self::Invariant`].
    #[must_use]
    pub fn slot(self) -> Option<Slot> {
        match self {
            Self::Invariant => None,
            Self::SlotA => Some(Slot::A),
            Self::SlotB => Some(Slot::B),
        }
    }

    #[must_use]
    pub fn of(slot: Slot) -> Self {
        match slot {
            Slot::A => Self::SlotA,
            Slot::B => Self::SlotB,
        }
    }

    /// Does a member with this variance belong in `slot`?
    #[must_use]
    pub fn applies_to(self, slot: Slot) -> bool {
        match self.slot() {
            None => true,
            Some(theirs) => theirs == slot,
        }
    }
}

/// One boot-directory member's name and place in the container, independent
/// of whether it has been read back from a signed [`Package`] or is still
/// being assembled by something that has not signed anything yet.
///
/// **Exists so [`check_boot_member_names`] is one function rather than two.**
/// [`Payload::open`] has [`Member`]s; a builder that has not sealed a header
/// yet has whatever it collects its members into — and a build-time copy of
/// the read-time check is exactly the second answer `ADR-011` exists to rule
/// out.
#[derive(Debug, Clone, Copy)]
pub struct BootMemberIdentity<'a> {
    pub index: usize,
    pub name: &'a str,
    pub target_class: u8,
    pub variant: u8,
}

/// `ADR-029` §4's platform is FAT, whose default `check=n` comparison folds
/// ASCII case. This container's own rules compare exact bytes, so two members
/// it considers distinct can still become one file on the boot partition —
/// at the same final name, or at one member's final name and another
/// member's [`BOOT_MEMBER_TEMP_SUFFIX`]-suffixed staging name.
///
/// Checked **per slot**, which is [`SlotVariance::applies_to`]'s own
/// boundary: an `Invariant` member and a `SlotA` member of the same name
/// never both reach slot B, but they do both reach slot A. An invariant
/// member sharing a name with a slot-variant one is caught here with no
/// special case, the same as two same-slot members whose names differ only
/// in case.
///
/// # Errors
///
/// [`PackageError::BootMemberNameCollides`] or
/// [`PackageError::BootMemberNameCollidesWithTemporary`], each naming both
/// members.
pub fn check_boot_member_names(members: &[BootMemberIdentity<'_>]) -> Result<(), PackageError> {
    for slot in [Slot::A, Slot::B] {
        let applicable: Vec<&BootMemberIdentity<'_>> = members
            .iter()
            .filter(|m| {
                m.target_class == TargetClass::BootDirectory.as_byte()
                    && SlotVariance::from_byte(m.variant).is_some_and(|v| v.applies_to(slot))
            })
            .collect();

        for (position, member) in applicable.iter().enumerate() {
            for other in applicable.iter().skip(position + 1) {
                if member.name.eq_ignore_ascii_case(other.name) {
                    return Err(PackageError::BootMemberNameCollides {
                        index: member.index,
                        name: member.name.to_string(),
                        other_index: other.index,
                        other_name: other.name.to_string(),
                    });
                }
            }
            let temp = format!("{}{BOOT_MEMBER_TEMP_SUFFIX}", member.name);
            if let Some(collides) = applicable
                .iter()
                .find(|other| other.name.eq_ignore_ascii_case(&temp))
            {
                return Err(PackageError::BootMemberNameCollidesWithTemporary {
                    index: collides.index,
                    name: collides.name.to_string(),
                    other_index: member.index,
                    other_name: member.name.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// A [`Package`] read as an update payload.
///
/// Holding one means the kind is `update`, every member's metadata is a value
/// this build defines, no member is named something the platform reserves, and
/// the payload is complete for **both** slots. It does **not** mean any member's
/// content has been checked — that is [`Package::verify_member`], per member, at
/// the point it is about to be used.
#[derive(Debug)]
pub struct Payload<'a> {
    package: &'a Package,
    system: &'a Member,
}

impl<'a> Payload<'a> {
    /// Read `package` as an update payload.
    ///
    /// `reserved` is what the platform reserves on a boot-directory target —
    /// `ADR-029` §4, and the port's answer rather than this crate's. On the
    /// Raspberry Pi it is `autoboot.txt` and `rivet.toml`; on a UEFI machine
    /// the equivalent object is an EFI variable and is not a file at all, which
    /// is exactly why the list is a parameter.
    ///
    /// **Passing an empty list is legal and is how a test drives the
    /// unprotected case.** It is not how a machine calls this: an installer that
    /// passed `&[]` would be re-introducing §4's bug, and the only defence
    /// against that is that the caller has a `BootSlots` in its hand already.
    pub fn open(package: &'a Package, reserved: &[&str]) -> Result<Self, PackageError> {
        package.require_kind(&Kind::update())?;
        if package.members().is_empty() {
            return Err(PackageError::NothingToInstall);
        }

        let mut system: Option<&Member> = None;
        let mut system_count = 0usize;
        for member in package.members() {
            let index = member.index();
            let class = TargetClass::from_byte(member.target_class()).ok_or(
                PackageError::UnknownTargetClass {
                    index,
                    found: member.target_class(),
                },
            )?;
            let variance =
                SlotVariance::from_byte(member.variant()).ok_or(PackageError::UnknownVariant {
                    index,
                    found: member.variant(),
                })?;
            match class {
                TargetClass::WholePartition => {
                    if variance != SlotVariance::Invariant {
                        return Err(PackageError::SystemMemberIsSlotVariant {
                            index,
                            variant: member.variant(),
                        });
                    }
                    system_count += 1;
                    system = Some(member);
                }
                TargetClass::BootDirectory => {
                    // Case-insensitive: `ADR-029` §4's platform is FAT, whose
                    // default `check=n` comparison folds ASCII case, so
                    // `RIVET.TOML` reaches the same file `rivet.toml` does.
                    if reserved.iter().any(|r| r.eq_ignore_ascii_case(member.name())) {
                        return Err(PackageError::ReservedMemberName {
                            index,
                            name: member.name().to_string(),
                        });
                    }
                }
            }
        }

        if system_count != 1 {
            return Err(PackageError::SystemMemberCount {
                found: system_count,
            });
        }

        // A slot-variant name must be present for both slots. Otherwise a
        // payload that installs cleanly into A silently omits a file in B, and
        // the omission is invisible until the machine boots the wrong root.
        for member in package.members() {
            let Some(theirs) =
                SlotVariance::from_byte(member.variant()).and_then(SlotVariance::slot)
            else {
                continue;
            };
            let wanted = SlotVariance::of(theirs.other());
            let present = package
                .members_named(member.name())
                .any(|other| other.variant() == wanted.as_byte());
            if !present {
                return Err(PackageError::SlotVarianceIncomplete {
                    name: member.name().to_string(),
                    missing: match theirs.other() {
                        Slot::A => "slot A",
                        Slot::B => "slot B",
                    },
                });
            }
        }

        let identities: Vec<BootMemberIdentity<'_>> = package
            .members()
            .iter()
            .map(|m| BootMemberIdentity {
                index: m.index(),
                name: m.name(),
                target_class: m.target_class(),
                variant: m.variant(),
            })
            .collect();
        check_boot_member_names(&identities)?;

        let system = system.ok_or(PackageError::SystemMemberCount { found: 0 })?;
        Ok(Self { package, system })
    }

    #[must_use]
    pub fn package(&self) -> &'a Package {
        self.package
    }

    /// The `/system` member. Exactly one, by [`Self::open`]'s check.
    ///
    /// A reference rather than an index, so there is no lookup here that could
    /// fail and no fallback to reason about. A verifier that can abort is a
    /// denial of service reachable from somebody else's bytes, and the cheapest
    /// way to have no `unwrap` is to have nothing to unwrap.
    #[must_use]
    pub fn system_member(&self) -> &'a Member {
        self.system
    }

    /// The boot-half members that belong in `slot`, in file order.
    ///
    /// `ADR-029` §9.2 step 8 writes these, *"skipping any whose digest already
    /// matches the target"* — which is what makes the usual update never open
    /// the FAT filesystem at all, and is a **safety** property rather than an
    /// optimisation (§9, F5).
    pub fn boot_members_for(&self, slot: Slot) -> impl Iterator<Item = &'a Member> {
        let members: &'a [Member] = self.package.members();
        members.iter().filter(move |m| {
            m.target_class() == TargetClass::BootDirectory.as_byte()
                && SlotVariance::from_byte(m.variant()).is_some_and(|v| v.applies_to(slot))
        })
    }

    /// `ADR-029` §9.2 step 2 — refuse on `product` / `arch` / `platform`
    /// mismatch, before any write. `ADR-029` F6.
    ///
    /// `platform` is compared only when the payload carries one. A payload with
    /// no platform is not refused here, because whether a given kind carries one
    /// is `ADR-027` §9 items 3–4 and is open; for `update` the builder always
    /// writes one, so the case does not arise in practice and is not made an
    /// error on the strength of a guess.
    pub fn accepts(&self, product: &str, arch: &str, platform: &str) -> Result<(), PackageError> {
        let header = self.package.header();
        if header.product() != product {
            return Err(PackageError::WrongKind {
                expected: format!("a payload for product {product:?}"),
                found: format!("one for {:?}", header.product()),
            });
        }
        accepts_arch(self.package, arch)?;
        if let Some(theirs) = header.platform()
            && theirs != platform
        {
            return Err(PackageError::WrongKind {
                expected: format!("a payload for platform {platform:?}"),
                found: format!("one for {theirs:?}"),
            });
        }
        Ok(())
    }
}

/// **The architecture half of [`Payload::accepts`], without opening a
/// [`Payload`].**
///
/// Opening one needs the boot port's reserved names, which only the installer
/// links; the artifact store's door has no boot port and still has to refuse a
/// system update built for another architecture
/// (`crate::package::verify::architecture`). So the comparison is here, once,
/// and [`Payload::accepts`] calls it rather than spelling it a second time.
///
/// # Errors
///
/// [`PackageError::WrongKind`] naming both architectures.
pub fn accepts_arch(package: &Package, arch: &str) -> Result<(), PackageError> {
    let found = package.header().arch();
    if found == arch {
        return Ok(());
    }
    Err(PackageError::WrongKind {
        expected: format!("a payload for arch {arch:?}"),
        found: format!("one for {found:?}"),
    })
}

#[cfg(test)]
mod constant_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{PRODUCT, SYSTEM_MEMBER_NAME};
    use crate::package::format::{M_NAME_BYTES, PRODUCT_BYTES, read_name, write_text};

    /// Both constants have to survive the fixed-width field they are written
    /// into, and neither is checked anywhere else until a build fails.
    #[test]
    fn both_constants_survive_the_fields_they_are_written_into() {
        let mut product = [0u8; PRODUCT_BYTES];
        write_text(&mut product, PRODUCT).expect("the product token is writable");

        let mut name = [0u8; M_NAME_BYTES];
        write_text(&mut name, SYSTEM_MEMBER_NAME).expect("the member name is writable");
        assert_eq!(
            read_name(&name).expect("and readable back"),
            SYSTEM_MEMBER_NAME
        );
    }
}
